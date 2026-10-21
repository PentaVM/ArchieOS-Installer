//! Frames over loopback, for a VM whose kernel has no camera driver.
//!
//! Wire: each frame is a `u32` big-endian length then a JPEG, nothing else. The host that owns
//! the camera connects and sends; the receiver keeps one sender at a time (a new connection
//! replaces the old) and hands frames out in order. It binds to loopback only.

use crate::{Error, Format, Frame};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Largest frame accepted (a 4K MJPEG frame is well under this).
pub const MAX_FRAME: usize = 8 * 1024 * 1024;

/// Writes one frame.
pub fn write_frame(stream: &mut impl Write, jpeg: &[u8]) -> Result<(), Error> {
    if jpeg.is_empty() || jpeg.len() > MAX_FRAME {
        return Err(Error::Frame(format!("{} bytes", jpeg.len())));
    }
    stream.write_all(&(jpeg.len() as u32).to_be_bytes())?;
    stream.write_all(jpeg)?;
    Ok(())
}

/// Reads one frame; `None` when the sender closed between frames.
pub fn read_frame(stream: &mut impl Read) -> Result<Option<Vec<u8>>, Error> {
    let mut header = [0u8; 4];
    match stream.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(Error::Frame(format!("declared {length} bytes")));
    }
    let mut jpeg = vec![0u8; length];
    stream.read_exact(&mut jpeg)?;
    if !jpeg.starts_with(&[0xFF, 0xD8]) {
        return Err(Error::Frame("not a JPEG".into()));
    }
    Ok(Some(jpeg))
}

/// The JPEG's own size, from its first start-of-frame marker.
pub fn jpeg_size(jpeg: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    while at + 9 < jpeg.len() {
        if jpeg[at] != 0xFF {
            return None;
        }
        let marker = jpeg[at + 1];
        let length = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            let height = u16::from_be_bytes([jpeg[at + 5], jpeg[at + 6]]);
            let width = u16::from_be_bytes([jpeg[at + 7], jpeg[at + 8]]);
            return Some((u32::from(width), u32::from(height)));
        }
        at += 2 + length;
    }
    None
}

/// The VM's end: frames from whichever sender is connected.
pub struct Receiver {
    listener: TcpListener,
    stream: Option<TcpStream>,
    sequence: u32,
    started: Instant,
}

impl Receiver {
    /// Listens on `address`, which must be loopback.
    pub fn bind(address: SocketAddr) -> Result<Self, Error> {
        if !address.ip().is_loopback() {
            return Err(Error::Unsupported(
                "the bridge binds to loopback only".into(),
            ));
        }
        Ok(Self {
            listener: TcpListener::bind(address)?,
            stream: None,
            sequence: 0,
            started: Instant::now(),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, Error> {
        Ok(self.listener.local_addr()?)
    }

    /// The next frame, waiting for a sender if none is connected.
    pub fn next_frame(&mut self) -> Result<Frame, Error> {
        loop {
            if self.stream.is_none() {
                let (stream, _) = self.listener.accept()?;
                stream.set_nodelay(true)?;
                stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                self.stream = Some(stream);
            }
            let stream = self.stream.as_mut().expect("connected above");
            match read_frame(stream) {
                Ok(Some(jpeg)) => {
                    let (width, height) = jpeg_size(&jpeg).unwrap_or((0, 0));
                    self.sequence = self.sequence.wrapping_add(1);
                    return Ok(Frame {
                        width,
                        height,
                        format: Format::Mjpeg,
                        data: jpeg,
                        sequence: self.sequence,
                        timestamp_us: self.started.elapsed().as_micros() as u64,
                    });
                }
                // The sender left or sent garbage: wait for the next one.
                Ok(None) | Err(_) => self.stream = None,
            }
        }
    }
}

/// The host's end: connects (again, when the receiver restarts) and sends.
pub struct Sender {
    address: SocketAddr,
    stream: Option<TcpStream>,
}

impl Sender {
    pub fn new(address: SocketAddr) -> Self {
        Self {
            address,
            stream: None,
        }
    }

    /// Sends one JPEG; reconnects once if the receiver went away.
    pub fn send(&mut self, jpeg: &[u8]) -> Result<(), Error> {
        for _ in 0..2 {
            if self.stream.is_none() {
                let stream = TcpStream::connect_timeout(&self.address, Duration::from_secs(2))?;
                stream.set_nodelay(true)?;
                self.stream = Some(stream);
            }
            match write_frame(self.stream.as_mut().expect("connected above"), jpeg) {
                Ok(()) => return Ok(()),
                Err(_) => self.stream = None,
            }
        }
        Err(Error::Frame("the receiver is not there".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal baseline JPEG header: SOI, SOF0 for 640x480, EOI.
    fn tiny_jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        j.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08]);
        j.extend_from_slice(&height.to_be_bytes());
        j.extend_from_slice(&width.to_be_bytes());
        j.extend_from_slice(&[0x01, 0x01, 0x11, 0x00, 0xFF, 0xD9]);
        j
    }

    #[test]
    fn frames_round_trip_and_bad_ones_are_refused() {
        let jpeg = tiny_jpeg(640, 480);
        assert_eq!(jpeg_size(&jpeg), Some((640, 480)));
        let mut wire = Vec::new();
        write_frame(&mut wire, &jpeg).unwrap();
        write_frame(&mut wire, &jpeg).unwrap();
        let mut reader = wire.as_slice();
        assert_eq!(read_frame(&mut reader).unwrap().unwrap(), jpeg);
        assert_eq!(read_frame(&mut reader).unwrap().unwrap(), jpeg);
        assert!(read_frame(&mut reader).unwrap().is_none());
        let huge = ((MAX_FRAME + 1) as u32).to_be_bytes();
        assert!(read_frame(&mut huge.as_slice()).is_err());
        let mut not_jpeg = Vec::new();
        write_frame(&mut not_jpeg, b"hello").unwrap();
        assert!(read_frame(&mut not_jpeg.as_slice()).is_err());
        assert!(write_frame(&mut Vec::new(), &[]).is_err());
    }

    #[test]
    fn a_sender_reaches_the_receiver_and_a_new_one_replaces_it() {
        let mut receiver = Receiver::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = receiver.local_addr().unwrap();
        assert!(Receiver::bind("0.0.0.0:0".parse().unwrap()).is_err());
        let first = std::thread::spawn(move || {
            let mut sender = Sender::new(address);
            sender.send(&tiny_jpeg(1280, 720)).unwrap();
        });
        let frame = receiver.next_frame().unwrap();
        first.join().unwrap();
        assert_eq!(
            (frame.width, frame.height, frame.format),
            (1280, 720, Format::Mjpeg)
        );
        let second = std::thread::spawn(move || {
            let mut sender = Sender::new(address);
            sender.send(&tiny_jpeg(320, 240)).unwrap();
        });
        let frame = receiver.next_frame().unwrap();
        second.join().unwrap();
        assert_eq!((frame.width, frame.height), (320, 240));
        assert_eq!(frame.sequence, 2);
    }
}
