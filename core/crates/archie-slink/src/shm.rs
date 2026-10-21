//! A link between processes: one sealed `memfd` under a seqlock, and an `eventfd`.
//!
//! The writer owns a fixed-size shared region: a control block and two payload slots. It
//! writes the next message into the slot readers are not pointed at, under that slot's
//! sequence (odd while writing), then points readers at it. A reader copies the current
//! slot and keeps the copy only if the slot's sequence did not move meanwhile, so a torn
//! copy is never returned, and with two slots a reader has a whole write period to copy
//! before the writer comes back to its slot. Neither side waits for the other. The writer
//! then bumps an `eventfd` so a reader can sleep until something changes.
//!
//! The region's size is sealed (`F_SEAL_SHRINK | F_SEAL_GROW`) before it is shared, and a
//! reader refuses one that is not, so no writer can shrink it under a reader's mapping. A
//! reader maps it read-only and treats every byte as untrusted: the message must verify
//! (link, limits, digest, grade), and a slot that never settles is reported, not waited on.
//! Both descriptors travel over a Unix socket (`SCM_RIGHTS`).

use crate::{Error, Message, Payload};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{fence, AtomicU64, Ordering};

const MAGIC: u64 = u64::from_le_bytes(*b"SLINK\x00\x00\x02");
/// Bytes before the payload slots: the control block, padded to two cache lines.
const CONTROL: usize = 128;
/// Reads that see the slot move before one gives up.
const ATTEMPTS: usize = 10_000;

#[repr(C)]
struct Slot {
    sequence: AtomicU64,
    revision: AtomicU64,
    length: AtomicU64,
}

#[repr(C)]
struct Control {
    magic: u64,
    /// Bytes per slot.
    capacity: u64,
    /// The slot readers should copy: 0 or 1.
    current: AtomicU64,
    slots: [Slot; 2],
}

fn os_error(what: &str) -> Error {
    Error::Shared(format!("{what}: {}", std::io::Error::last_os_error()))
}

/// A shared mapping, unmapped on drop.
struct Mapping {
    pointer: *mut u8,
    size: usize,
}

// The mapping is plain shared memory; every access goes through the seqlock's atomics or
// volatile copies.
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    fn new(fd: RawFd, size: usize, writable: bool) -> Result<Self, Error> {
        let protection = if writable {
            libc::PROT_READ | libc::PROT_WRITE
        } else {
            libc::PROT_READ
        };
        // SAFETY: a fresh shared mapping of `size` bytes of `fd`; checked for failure.
        let pointer = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                protection,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if pointer == libc::MAP_FAILED {
            return Err(os_error("mmap"));
        }
        Ok(Self {
            pointer: pointer.cast(),
            size,
        })
    }

    fn control(&self) -> &Control {
        // SAFETY: the mapping is at least `CONTROL` bytes and page-aligned.
        unsafe { &*(self.pointer as *const Control) }
    }

    /// The payload of `slot` (0 or 1), `capacity` bytes.
    fn payload(&self, slot: usize, capacity: usize) -> *mut u8 {
        // SAFETY: within the mapping, which is `CONTROL + 2 * capacity` bytes.
        unsafe { self.pointer.add(CONTROL + slot * capacity) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: unmapping what `mmap` returned, once.
        unsafe {
            libc::munmap(self.pointer.cast(), self.size);
        }
    }
}

/// The producer's end.
pub struct ShmWriter {
    memfd: OwnedFd,
    event: OwnedFd,
    map: Mapping,
    capacity: usize,
}

impl ShmWriter {
    /// A region for messages of up to `capacity` bytes, named `name` in `/proc` listings.
    pub fn create(name: &str, capacity: usize) -> Result<Self, Error> {
        let name = std::ffi::CString::new(format!("slink:{name}"))
            .map_err(|_| Error::Shared("the name holds a NUL".into()))?;
        // SAFETY: a valid C string; the result is checked and owned below.
        let raw = unsafe {
            libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING)
        };
        if raw < 0 {
            return Err(os_error("memfd_create"));
        }
        // SAFETY: `raw` is a descriptor we just opened and nothing else owns.
        let memfd = unsafe { OwnedFd::from_raw_fd(raw) };
        let size = CONTROL + 2 * capacity;
        // SAFETY: plain calls on our own descriptor.
        unsafe {
            if libc::ftruncate(memfd.as_raw_fd(), size as libc::off_t) != 0 {
                return Err(os_error("ftruncate"));
            }
            let seals = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;
            if libc::fcntl(memfd.as_raw_fd(), libc::F_ADD_SEALS, seals) != 0 {
                return Err(os_error("F_ADD_SEALS"));
            }
        }
        // SAFETY: a fresh eventfd, checked and owned below.
        let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if raw < 0 {
            return Err(os_error("eventfd"));
        }
        // SAFETY: as above.
        let event = unsafe { OwnedFd::from_raw_fd(raw) };
        let map = Mapping::new(memfd.as_raw_fd(), size, true)?;
        // SAFETY: we own the only writable mapping; the control block is ours to set up.
        unsafe {
            let control = map.pointer as *mut Control;
            std::ptr::write_volatile(&mut (*control).magic, MAGIC);
            std::ptr::write_volatile(&mut (*control).capacity, capacity as u64);
        }
        Ok(Self {
            memfd,
            event,
            map,
            capacity,
        })
    }

    /// Replaces the region's content with `bytes` at `revision` and wakes readers.
    pub fn write(&mut self, revision: u64, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > self.capacity {
            return Err(Error::Shared(format!(
                "{} bytes for a {}-byte region",
                bytes.len(),
                self.capacity
            )));
        }
        let control = self.map.control();
        let target = 1 - (control.current.load(Ordering::Relaxed) as usize & 1);
        let slot = &control.slots[target];
        let sequence = slot.sequence.load(Ordering::Relaxed);
        slot.sequence
            .store(sequence.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        let payload = self.map.payload(target, self.capacity);
        for (offset, byte) in bytes.iter().enumerate() {
            // SAFETY: `offset < capacity`, inside the slot.
            unsafe { std::ptr::write_volatile(payload.add(offset), *byte) };
        }
        slot.revision.store(revision, Ordering::Relaxed);
        slot.length.store(bytes.len() as u64, Ordering::Relaxed);
        slot.sequence
            .store(sequence.wrapping_add(2), Ordering::Release);
        control.current.store(target as u64, Ordering::Release);
        let one: u64 = 1;
        // SAFETY: writing 8 bytes to our eventfd; a full counter only means readers lag.
        unsafe {
            libc::write(
                self.event.as_raw_fd(),
                (&one as *const u64).cast(),
                std::mem::size_of::<u64>(),
            );
        }
        Ok(())
    }

    pub fn publish<P: Payload>(&mut self, message: &Message<P>) -> Result<(), Error> {
        let bytes = serde_json::to_vec(message)?;
        self.write(message.header.revision, &bytes)
    }

    /// Sends both descriptors to the process at the other end of `socket`.
    pub fn send_to(&self, socket: &UnixStream) -> Result<(), Error> {
        send_fds(socket, &[self.memfd.as_raw_fd(), self.event.as_raw_fd()])
    }
}

/// A consumer's end.
pub struct ShmReader {
    event: OwnedFd,
    map: Mapping,
    capacity: usize,
}

impl ShmReader {
    /// Maps a region a writer shared. Refuses one whose size is not sealed or whose
    /// control block does not match its size.
    pub fn open(memfd: OwnedFd, event: OwnedFd) -> Result<Self, Error> {
        // SAFETY: plain queries on a descriptor we own.
        let (seals, size) = unsafe {
            let seals = libc::fcntl(memfd.as_raw_fd(), libc::F_GET_SEALS);
            let mut stat: libc::stat = std::mem::zeroed();
            if libc::fstat(memfd.as_raw_fd(), &mut stat) != 0 {
                return Err(os_error("fstat"));
            }
            (seals, stat.st_size as usize)
        };
        let needed = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW;
        if seals < 0 || seals & needed != needed {
            return Err(Error::Shared("the region's size is not sealed".into()));
        }
        if size < CONTROL {
            return Err(Error::Shared("the region is too small".into()));
        }
        let map = Mapping::new(memfd.as_raw_fd(), size, false)?;
        let control = map.control();
        // SAFETY: reading the two plain fields the writer set before sharing.
        let (magic, capacity) = unsafe {
            (
                std::ptr::read_volatile(&control.magic),
                std::ptr::read_volatile(&control.capacity) as usize,
            )
        };
        if magic != MAGIC || CONTROL + 2 * capacity != size {
            return Err(Error::Shared("not an S-LINK region".into()));
        }
        Ok(Self {
            event,
            map,
            capacity,
        })
    }

    /// Receives a writer's descriptors from `socket` and maps the region.
    pub fn receive_from(socket: &UnixStream) -> Result<Self, Error> {
        let mut fds = receive_fds(socket, 2)?;
        if fds.len() != 2 {
            return Err(Error::Shared(format!(
                "{} descriptors, expected 2",
                fds.len()
            )));
        }
        let event = fds.pop().expect("two descriptors");
        let memfd = fds.pop().expect("two descriptors");
        Self::open(memfd, event)
    }

    /// The latest bytes and their revision; `None` before the first write.
    pub fn read_raw(&self) -> Result<Option<(u64, Vec<u8>)>, Error> {
        let control = self.map.control();
        for attempt in 0..ATTEMPTS {
            if attempt % 64 == 63 {
                std::thread::yield_now();
            }
            let current = control.current.load(Ordering::Acquire) as usize & 1;
            let slot = &control.slots[current];
            let before = slot.sequence.load(Ordering::Acquire);
            if before % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            if before == 0 {
                return Ok(None);
            }
            let length = slot.length.load(Ordering::Relaxed) as usize;
            let revision = slot.revision.load(Ordering::Relaxed);
            if length > self.capacity {
                std::hint::spin_loop();
                continue;
            }
            let payload = self.map.payload(current, self.capacity);
            let mut bytes = vec![0u8; length];
            for (offset, byte) in bytes.iter_mut().enumerate() {
                // SAFETY: `offset < length <= capacity`, inside the slot.
                *byte = unsafe { std::ptr::read_volatile(payload.add(offset)) };
            }
            fence(Ordering::Acquire);
            if slot.sequence.load(Ordering::Relaxed) == before {
                return Ok(Some((revision, bytes)));
            }
        }
        Err(Error::Shared("the writer never settled".into()))
    }

    /// The latest message, verified.
    pub fn latest<P: Payload>(&self) -> Result<Option<Message<P>>, Error> {
        let Some((revision, bytes)) = self.read_raw()? else {
            return Ok(None);
        };
        let message = Message::<P>::from_slice(&bytes)?;
        if message.header.revision != revision {
            return Err(Error::Shared(
                "the revision does not match the message".into(),
            ));
        }
        Ok(Some(message))
    }

    /// Sleeps until the writer writes or `timeout_ms` passes; true when it wrote.
    pub fn wait(&self, timeout_ms: i32) -> Result<bool, Error> {
        let mut poll = libc::pollfd {
            fd: self.event.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one pollfd on our descriptor.
        let ready = unsafe { libc::poll(&mut poll, 1, timeout_ms) };
        if ready < 0 {
            return Err(os_error("poll"));
        }
        if ready == 0 {
            return Ok(false);
        }
        let mut count: u64 = 0;
        // SAFETY: reading the 8-byte counter resets it; EAGAIN only means another reader
        // took it first.
        unsafe {
            libc::read(
                self.event.as_raw_fd(),
                (&mut count as *mut u64).cast(),
                std::mem::size_of::<u64>(),
            );
        }
        Ok(true)
    }
}

/// A link exported to other processes: a writer, and a Unix socket at `path` (owner-only)
/// that hands each process that connects the region's descriptors.
pub struct Exporter {
    writer: std::sync::Mutex<ShmWriter>,
    path: std::path::PathBuf,
}

impl Exporter {
    /// Creates the region for link `name` and listens at `path` (replacing a stale socket).
    pub fn serve(
        path: &std::path::Path,
        name: &str,
        capacity: usize,
    ) -> Result<std::sync::Arc<Self>, Error> {
        use std::os::unix::fs::PermissionsExt;
        let writer = ShmWriter::create(name, capacity)?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        let listener = std::os::unix::net::UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let exporter = std::sync::Arc::new(Self {
            writer: std::sync::Mutex::new(writer),
            path: path.to_owned(),
        });
        let serving = std::sync::Arc::downgrade(&exporter);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Some(exporter) = serving.upgrade() else {
                    return;
                };
                let Ok(stream) = stream else {
                    continue;
                };
                let sent = match exporter.writer.lock() {
                    Ok(writer) => writer.send_to(&stream),
                    Err(_) => return,
                };
                drop(sent);
            }
        });
        Ok(exporter)
    }

    pub fn publish<P: Payload>(&self, message: &Message<P>) -> Result<(), Error> {
        self.writer
            .lock()
            .map_err(|_| Error::Shared("the writer panicked".into()))?
            .publish(message)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// Connects to an exported link at `path` and maps its region.
pub fn connect(path: &std::path::Path) -> Result<ShmReader, Error> {
    ShmReader::receive_from(&UnixStream::connect(path)?)
}

/// Sends `fds` with one byte of data over `socket`.
fn send_fds(socket: &UnixStream, fds: &[RawFd]) -> Result<(), Error> {
    let data = [0u8; 1];
    let mut io = libc::iovec {
        iov_base: data.as_ptr() as *mut libc::c_void,
        iov_len: 1,
    };
    let payload = std::mem::size_of_val(fds) as u32;
    // SAFETY: CMSG_SPACE is a size computation.
    let space = unsafe { libc::CMSG_SPACE(payload) } as usize;
    let mut buffer = vec![0u8; space];
    // SAFETY: a msghdr pointing at our iovec and control buffer, filled with the CMSG
    // macros over a buffer of exactly `space` bytes.
    unsafe {
        let mut header: libc::msghdr = std::mem::zeroed();
        header.msg_iov = &mut io;
        header.msg_iovlen = 1;
        header.msg_control = buffer.as_mut_ptr().cast();
        header.msg_controllen = space as _;
        let cmsg = libc::CMSG_FIRSTHDR(&header);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(payload) as _;
        std::ptr::copy_nonoverlapping(
            fds.as_ptr().cast::<u8>(),
            libc::CMSG_DATA(cmsg),
            payload as usize,
        );
        if libc::sendmsg(socket.as_raw_fd(), &header, libc::MSG_NOSIGNAL) < 0 {
            return Err(os_error("sendmsg"));
        }
    }
    Ok(())
}

/// Receives up to `count` descriptors sent with [`send_fds`].
fn receive_fds(socket: &UnixStream, count: usize) -> Result<Vec<OwnedFd>, Error> {
    let mut data = [0u8; 1];
    let mut io = libc::iovec {
        iov_base: data.as_mut_ptr().cast(),
        iov_len: 1,
    };
    let payload = (count * std::mem::size_of::<RawFd>()) as u32;
    // SAFETY: CMSG_SPACE is a size computation.
    let space = unsafe { libc::CMSG_SPACE(payload) } as usize;
    let mut buffer = vec![0u8; space];
    let mut out = Vec::new();
    // SAFETY: as in `send_fds`; descriptors received are taken into `OwnedFd` once each.
    unsafe {
        let mut header: libc::msghdr = std::mem::zeroed();
        header.msg_iov = &mut io;
        header.msg_iovlen = 1;
        header.msg_control = buffer.as_mut_ptr().cast();
        header.msg_controllen = space as _;
        if libc::recvmsg(socket.as_raw_fd(), &mut header, libc::MSG_CMSG_CLOEXEC) < 0 {
            return Err(os_error("recvmsg"));
        }
        let mut cmsg = libc::CMSG_FIRSTHDR(&header);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let bytes = (*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                let received = bytes / std::mem::size_of::<RawFd>();
                let data = libc::CMSG_DATA(cmsg) as *const RawFd;
                for index in 0..received {
                    out.push(OwnedFd::from_raw_fd(std::ptr::read_unaligned(
                        data.add(index),
                    )));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&header, cmsg);
        }
        if header.msg_flags & libc::MSG_CTRUNC != 0 {
            return Err(Error::Shared("descriptors were truncated".into()));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::{LlmPartial, SocialIntent};

    fn partial(text: &str) -> LlmPartial {
        LlmPartial {
            conversation: "c1".into(),
            text: text.into(),
            done: false,
        }
    }

    fn pair() -> (ShmWriter, ShmReader) {
        let writer = ShmWriter::create("llm.partial", 4096).unwrap();
        let (a, b) = UnixStream::pair().unwrap();
        writer.send_to(&a).unwrap();
        let reader = ShmReader::receive_from(&b).unwrap();
        (writer, reader)
    }

    #[test]
    fn a_reader_in_another_mapping_sees_each_latest_message() {
        let (mut writer, reader) = pair();
        assert!(reader.latest::<LlmPartial>().unwrap().is_none());
        assert!(!reader.wait(0).unwrap());
        for (revision, text) in [(1, "Hola"), (2, "Hola, ¿cómo"), (3, "Hola, ¿cómo estás?")] {
            writer
                .publish(&Message::new(revision, 0, partial(text)).unwrap())
                .unwrap();
        }
        assert!(reader.wait(100).unwrap());
        let latest = reader.latest::<LlmPartial>().unwrap().unwrap();
        assert_eq!(latest.header.revision, 3);
        assert_eq!(latest.payload.text, "Hola, ¿cómo estás?");
        assert!(
            reader.latest::<SocialIntent>().is_err(),
            "another link's payload"
        );
    }

    #[test]
    fn a_reader_racing_the_writer_never_sees_a_torn_message() {
        let (mut writer, reader) = pair();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = stop.clone();
        let reading = std::thread::spawn(move || {
            let mut seen = 0u64;
            let mut last = 0;
            while !done.load(Ordering::Relaxed) {
                if let Some(message) = reader.latest::<LlmPartial>().unwrap() {
                    assert!(message.header.revision >= last);
                    assert_eq!(
                        message.payload.text.len() as u64,
                        message.header.revision % 300
                    );
                    last = message.header.revision;
                    seen += 1;
                }
            }
            seen
        });
        for revision in 1..=20_000u64 {
            let text = "x".repeat((revision % 300) as usize);
            writer
                .publish(&Message::new(revision, 0, partial(&text)).unwrap())
                .unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        assert!(reading.join().unwrap() > 0);
    }

    #[test]
    fn oversized_writes_and_unsealed_regions_are_refused() {
        let (mut writer, _) = pair();
        assert!(writer.write(1, &vec![0u8; 4097]).is_err());
        // A memfd without seals, offered as a region.
        let name = std::ffi::CString::new("plain").unwrap();
        let raw = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        assert!(raw >= 0);
        let memfd = unsafe { OwnedFd::from_raw_fd(raw) };
        unsafe { libc::ftruncate(memfd.as_raw_fd(), 4096) };
        let event = unsafe { OwnedFd::from_raw_fd(libc::eventfd(0, libc::EFD_CLOEXEC)) };
        assert!(matches!(
            ShmReader::open(memfd, event),
            Err(Error::Shared(reason)) if reason.contains("sealed")
        ));
    }

    #[test]
    fn an_exported_link_reaches_a_process_that_connects() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("slink-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("llm.partial.sock");
        let exporter = Exporter::serve(&path, "llm.partial", 4096).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let reader = connect(&path).unwrap();
        exporter
            .publish(&Message::new(7, 0, partial("listo")).unwrap())
            .unwrap();
        let message = reader.latest::<LlmPartial>().unwrap().unwrap();
        assert_eq!(
            (message.header.revision, message.payload.text.as_str()),
            (7, "listo")
        );
        // A second consumer sees the same region.
        let other = connect(exporter.path()).unwrap();
        assert_eq!(other.latest::<LlmPartial>().unwrap().unwrap(), message);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
