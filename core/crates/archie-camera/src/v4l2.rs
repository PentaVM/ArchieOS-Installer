//! Video4Linux2 capture, straight from the kernel (`uvcvideo` and any other V4L2 driver).
//!
//! The structures and ioctl numbers are the kernel's ABI (`linux/videodev2.h`) for 64-bit
//! Linux; tests check their sizes and the numbers against the header's values. Capture is
//! memory-mapped streaming: the driver fills `buffers` frames ahead, the reader dequeues one,
//! copies its bytes out and queues it again, so the camera never waits on the reader.

use crate::{Error, Format, Frame, Request};
use std::ffi::CStr;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const MEMORY_MMAP: u32 = 1;
const FIELD_NONE: u32 = 1;
const CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
const CAP_STREAMING: u32 = 0x0400_0000;
const CAP_DEVICE_CAPS: u32 = 0x8000_0000;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Capability {
    pub driver: [u8; 16],
    pub card: [u8; 32],
    pub bus_info: [u8; 32],
    pub version: u32,
    pub capabilities: u32,
    pub device_caps: u32,
    pub reserved: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FmtDesc {
    pub index: u32,
    pub kind: u32,
    pub flags: u32,
    pub description: [u8; 32],
    pub pixelformat: u32,
    pub mbus_code: u32,
    pub reserved: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PixFormat {
    pub width: u32,
    pub height: u32,
    pub pixelformat: u32,
    pub field: u32,
    pub bytesperline: u32,
    pub sizeimage: u32,
    pub colorspace: u32,
    pub private: u32,
    pub flags: u32,
    pub ycbcr_enc: u32,
    pub quantization: u32,
    pub xfer_func: u32,
}

/// `struct v4l2_format`: the union holds pointers, so it is 8-byte aligned.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FormatAbi {
    pub kind: u32,
    pub fmt: FormatUnion,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union FormatUnion {
    pub pix: PixFormat,
    pub raw: [u8; 200],
    _align: [u64; 25],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RequestBuffers {
    pub count: u32,
    pub kind: u32,
    pub memory: u32,
    pub capabilities: u32,
    pub flags: u8,
    pub reserved: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Timecode {
    pub kind: u32,
    pub flags: u32,
    pub frames: u8,
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    pub userbits: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union BufferM {
    pub offset: u32,
    pub userptr: u64,
    pub fd: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Buffer {
    pub index: u32,
    pub kind: u32,
    pub bytesused: u32,
    pub flags: u32,
    pub field: u32,
    pub timestamp: libc::timeval,
    pub timecode: Timecode,
    pub sequence: u32,
    pub memory: u32,
    pub m: BufferM,
    pub length: u32,
    pub reserved2: u32,
    pub request_fd: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Fract {
    pub numerator: u32,
    pub denominator: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CaptureParm {
    pub capability: u32,
    pub capturemode: u32,
    pub timeperframe: Fract,
    pub extendedmode: u32,
    pub readbuffers: u32,
    pub reserved: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union ParmUnion {
    pub capture: CaptureParm,
    pub raw: [u8; 200],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StreamParm {
    pub kind: u32,
    pub parm: ParmUnion,
}

const fn ioc(dir: u32, nr: u32, size: usize) -> libc::c_ulong {
    ((dir << 30) | ((size as u32) << 16) | ((b'V' as u32) << 8) | nr) as libc::c_ulong
}
const READ: u32 = 2;
const WRITE: u32 = 1;
pub const VIDIOC_QUERYCAP: libc::c_ulong = ioc(READ, 0, std::mem::size_of::<Capability>());
pub const VIDIOC_ENUM_FMT: libc::c_ulong = ioc(READ | WRITE, 2, std::mem::size_of::<FmtDesc>());
pub const VIDIOC_S_FMT: libc::c_ulong = ioc(READ | WRITE, 5, std::mem::size_of::<FormatAbi>());
pub const VIDIOC_REQBUFS: libc::c_ulong =
    ioc(READ | WRITE, 8, std::mem::size_of::<RequestBuffers>());
pub const VIDIOC_QUERYBUF: libc::c_ulong = ioc(READ | WRITE, 9, std::mem::size_of::<Buffer>());
pub const VIDIOC_QBUF: libc::c_ulong = ioc(READ | WRITE, 15, std::mem::size_of::<Buffer>());
pub const VIDIOC_DQBUF: libc::c_ulong = ioc(READ | WRITE, 17, std::mem::size_of::<Buffer>());
pub const VIDIOC_STREAMON: libc::c_ulong = ioc(WRITE, 18, std::mem::size_of::<libc::c_int>());
pub const VIDIOC_STREAMOFF: libc::c_ulong = ioc(WRITE, 19, std::mem::size_of::<libc::c_int>());
pub const VIDIOC_S_PARM: libc::c_ulong = ioc(READ | WRITE, 22, std::mem::size_of::<StreamParm>());

fn zeroed<T: Copy>() -> T {
    // SAFETY: every type used here is a plain C struct or union of integers and arrays,
    // for which all-zero bytes is a valid value (it is what the kernel ABI expects too).
    unsafe { std::mem::zeroed() }
}

fn ioctl<T>(
    file: &File,
    call: &'static str,
    request: libc::c_ulong,
    arg: &mut T,
) -> Result<(), Error> {
    loop {
        // SAFETY: `arg` is a live, exclusively borrowed value of the structure the request
        // number encodes (its size is part of the number), and the descriptor is open.
        let result = unsafe { libc::ioctl(file.as_raw_fd(), request as _, arg as *mut T) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(Error::Ioctl {
                call,
                source: error,
            });
        }
    }
}

fn text(bytes: &[u8]) -> String {
    CStr::from_bytes_until_nul(bytes)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| String::from_utf8_lossy(bytes).into_owned())
}

/// A capture device as the kernel describes it.
#[derive(Debug, Clone)]
pub struct Device {
    pub path: PathBuf,
    /// The camera's name (`card`), e.g. "Elgato Facecam MK.2".
    pub name: String,
    pub driver: String,
    pub bus: String,
    pub formats: Vec<Format>,
}

fn open(path: &Path) -> Result<File, Error> {
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)?)
}

/// Describes `path` if it is a streaming capture device.
pub fn describe(path: &Path) -> Result<Device, Error> {
    let file = open(path)?;
    let mut cap: Capability = zeroed();
    ioctl(&file, "VIDIOC_QUERYCAP", VIDIOC_QUERYCAP, &mut cap)?;
    let caps = if cap.capabilities & CAP_DEVICE_CAPS != 0 {
        cap.device_caps
    } else {
        cap.capabilities
    };
    if caps & CAP_VIDEO_CAPTURE == 0 || caps & CAP_STREAMING == 0 {
        return Err(Error::NotCapture(path.display().to_string()));
    }
    let mut formats = Vec::new();
    for index in 0..64 {
        let mut desc: FmtDesc = zeroed();
        desc.index = index;
        desc.kind = BUF_TYPE_VIDEO_CAPTURE;
        if ioctl(&file, "VIDIOC_ENUM_FMT", VIDIOC_ENUM_FMT, &mut desc).is_err() {
            break;
        }
        if let Some(format) = Format::from_fourcc(desc.pixelformat) {
            if !formats.contains(&format) {
                formats.push(format);
            }
        }
    }
    Ok(Device {
        path: path.to_path_buf(),
        name: text(&cap.card),
        driver: text(&cap.driver),
        bus: text(&cap.bus_info),
        formats,
    })
}

/// Every `/dev/video*` that is a streaming capture device (a camera usually also exposes a
/// metadata node, which is skipped).
pub fn list() -> Vec<Device> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir("/dev")
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("video"))
                })
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    paths.iter().filter_map(|p| describe(p).ok()).collect()
}

struct Mapped {
    ptr: *mut libc::c_void,
    length: usize,
}

/// A running capture. Dropping it stops the stream and unmaps its buffers.
pub struct Stream {
    file: File,
    buffers: Vec<Mapped>,
    pub width: u32,
    pub height: u32,
    pub format: Format,
    pub device: Device,
}

// SAFETY: the mapped buffers belong to this stream alone and are only read through `&mut self`.
unsafe impl Send for Stream {}

impl Stream {
    /// Opens `device` and starts streaming the first requested format it offers.
    pub fn open(device: &Device, request: &Request) -> Result<Self, Error> {
        let format = request
            .formats
            .iter()
            .copied()
            .find(|f| device.formats.contains(f))
            .ok_or_else(|| {
                Error::Unsupported(format!(
                    "{} offers {:?}, none of {:?}",
                    device.name, device.formats, request.formats
                ))
            })?;
        let file = open(&device.path)?;

        let mut fmt: FormatAbi = zeroed();
        fmt.kind = BUF_TYPE_VIDEO_CAPTURE;
        fmt.fmt.pix.width = request.width;
        fmt.fmt.pix.height = request.height;
        fmt.fmt.pix.pixelformat = format.fourcc();
        fmt.fmt.pix.field = FIELD_NONE;
        ioctl(&file, "VIDIOC_S_FMT", VIDIOC_S_FMT, &mut fmt)?;
        // SAFETY: the driver filled `pix` back with what it agreed to.
        let pix = unsafe { fmt.fmt.pix };
        let agreed = Format::from_fourcc(pix.pixelformat)
            .ok_or_else(|| Error::Unsupported("the driver chose another format".into()))?;

        // The rate is a request; a driver that cannot set it keeps its own.
        let mut parm: StreamParm = zeroed();
        parm.kind = BUF_TYPE_VIDEO_CAPTURE;
        parm.parm.capture.timeperframe = Fract {
            numerator: 1,
            denominator: request.fps.max(1),
        };
        let _ = ioctl(&file, "VIDIOC_S_PARM", VIDIOC_S_PARM, &mut parm);

        let mut req: RequestBuffers = zeroed();
        req.count = request.buffers.clamp(2, 8);
        req.kind = BUF_TYPE_VIDEO_CAPTURE;
        req.memory = MEMORY_MMAP;
        ioctl(&file, "VIDIOC_REQBUFS", VIDIOC_REQBUFS, &mut req)?;
        let mut stream = Self {
            file,
            buffers: Vec::with_capacity(req.count as usize),
            width: pix.width,
            height: pix.height,
            format: agreed,
            device: device.clone(),
        };
        for index in 0..req.count {
            let mut buf: Buffer = zeroed();
            buf.index = index;
            buf.kind = BUF_TYPE_VIDEO_CAPTURE;
            buf.memory = MEMORY_MMAP;
            ioctl(&stream.file, "VIDIOC_QUERYBUF", VIDIOC_QUERYBUF, &mut buf)?;
            let length = buf.length as usize;
            // SAFETY: maps the buffer the driver just described, shared and read-only, at the
            // offset it gave; the mapping is released in Drop.
            let ptr = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    length,
                    libc::PROT_READ,
                    libc::MAP_SHARED,
                    stream.file.as_raw_fd(),
                    buf.m.offset as libc::off_t,
                )
            };
            if ptr == libc::MAP_FAILED {
                return Err(Error::Io(std::io::Error::last_os_error()));
            }
            stream.buffers.push(Mapped { ptr, length });
            ioctl(&stream.file, "VIDIOC_QBUF", VIDIOC_QBUF, &mut buf)?;
        }
        let mut kind: libc::c_int = BUF_TYPE_VIDEO_CAPTURE as libc::c_int;
        ioctl(&stream.file, "VIDIOC_STREAMON", VIDIOC_STREAMON, &mut kind)?;
        Ok(stream)
    }

    /// The next frame the driver finished (blocks until there is one).
    pub fn next_frame(&mut self) -> Result<Frame, Error> {
        let mut buf: Buffer = zeroed();
        buf.kind = BUF_TYPE_VIDEO_CAPTURE;
        buf.memory = MEMORY_MMAP;
        ioctl(&self.file, "VIDIOC_DQBUF", VIDIOC_DQBUF, &mut buf)?;
        let mapped = self
            .buffers
            .get(buf.index as usize)
            .ok_or_else(|| Error::Frame("the driver returned an unknown buffer".into()))?;
        let used = (buf.bytesused as usize).min(mapped.length);
        // SAFETY: the buffer is dequeued, so the driver is not writing it; `used` is within
        // the mapping's length.
        let data = unsafe { std::slice::from_raw_parts(mapped.ptr as *const u8, used) }.to_vec();
        ioctl(&self.file, "VIDIOC_QBUF", VIDIOC_QBUF, &mut buf)?;
        Ok(Frame {
            width: self.width,
            height: self.height,
            format: self.format,
            data,
            sequence: buf.sequence,
            timestamp_us: (buf.timestamp.tv_sec as u64) * 1_000_000 + buf.timestamp.tv_usec as u64,
        })
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let mut kind: libc::c_int = BUF_TYPE_VIDEO_CAPTURE as libc::c_int;
        let _ = ioctl(&self.file, "VIDIOC_STREAMOFF", VIDIOC_STREAMOFF, &mut kind);
        for mapped in &self.buffers {
            // SAFETY: each mapping was created in `open` with this length and is unmapped once.
            unsafe {
                libc::munmap(mapped.ptr, mapped.length);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_abi_matches_videodev2_on_64_bit_linux() {
        assert_eq!(std::mem::size_of::<Capability>(), 104);
        assert_eq!(std::mem::size_of::<FmtDesc>(), 64);
        assert_eq!(std::mem::size_of::<FormatAbi>(), 208);
        assert_eq!(std::mem::size_of::<RequestBuffers>(), 20);
        assert_eq!(std::mem::size_of::<Buffer>(), 88);
        assert_eq!(std::mem::size_of::<StreamParm>(), 204);
        // The numbers videodev2.h gives.
        assert_eq!(VIDIOC_QUERYCAP, 0x8068_5600);
        assert_eq!(VIDIOC_ENUM_FMT, 0xc040_5602);
        assert_eq!(VIDIOC_S_FMT, 0xc0d0_5605);
        assert_eq!(VIDIOC_REQBUFS, 0xc014_5608);
        assert_eq!(VIDIOC_QUERYBUF, 0xc058_5609);
        assert_eq!(VIDIOC_QBUF, 0xc058_560f);
        assert_eq!(VIDIOC_DQBUF, 0xc058_5611);
        assert_eq!(VIDIOC_STREAMON, 0x4004_5612);
        assert_eq!(VIDIOC_STREAMOFF, 0x4004_5613);
        assert_eq!(VIDIOC_S_PARM, 0xc0cc_5616);
    }

    #[test]
    fn fourccs_are_the_kernels() {
        assert_eq!(Format::Mjpeg.fourcc(), 0x4750_4a4d);
        assert_eq!(Format::Yuyv.fourcc(), 0x5659_5559);
        assert_eq!(Format::from_fourcc(0x4745_504a), Some(Format::Mjpeg)); // "JPEG"
    }

    #[test]
    fn a_missing_device_is_an_error_and_listing_never_fails() {
        assert!(describe(Path::new("/dev/archie-no-such-camera")).is_err());
        let _ = list();
    }
}
