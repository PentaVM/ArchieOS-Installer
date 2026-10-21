//! # archie-camera
//!
//! Cameras as ArchieOS sees them. One frame type and two ways to get frames:
//!
//! - [`v4l2`] (Linux): straight from the kernel. An installed ArchieOS runs Arch's stock
//!   `linux` kernel, whose `uvcvideo` driver exposes every USB camera (an Elgato Facecam, a
//!   laptop's webcam) as `/dev/videoN`. Frames are read by memory-mapped streaming, the way
//!   the kernel hands them over, with no copy through a capture library. A camera that sends
//!   MJPEG is passed on as the JPEG it sent: nothing is decoded or re-encoded on the way to
//!   IRIS.
//! - [`bridge`] (any OS): frames over loopback, `u32` big-endian length then the frame. For a
//!   VM whose kernel has no camera driver (WSL's Microsoft kernel has no `uvcvideo`), the host
//!   that owns the camera sends them; the VM receives them as if they came from a device.
//!
//! Frames never touch the disk and are never logged.

pub mod bridge;
pub mod convert;
#[cfg(target_os = "linux")]
pub mod v4l2;

/// How a frame's bytes are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A complete JPEG image, as most UVC cameras send at high frame rates.
    Mjpeg,
    /// Packed 4:2:2, `Y0 U Y1 V`, two bytes per pixel.
    Yuyv,
    /// Packed 8-bit RGB, three bytes per pixel.
    Rgb24,
}

impl Format {
    /// The V4L2 fourcc of the format.
    pub fn fourcc(self) -> u32 {
        let code = match self {
            Self::Mjpeg => *b"MJPG",
            Self::Yuyv => *b"YUYV",
            Self::Rgb24 => *b"RGB3",
        };
        u32::from_le_bytes(code)
    }

    pub fn from_fourcc(fourcc: u32) -> Option<Self> {
        match &fourcc.to_le_bytes() {
            b"MJPG" | b"JPEG" => Some(Self::Mjpeg),
            b"YUYV" => Some(Self::Yuyv),
            b"RGB3" => Some(Self::Rgb24),
            _ => None,
        }
    }
}

/// One frame as the camera delivered it.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    pub data: Vec<u8>,
    /// The device's own frame counter (gaps mean the reader fell behind).
    pub sequence: u32,
    /// Monotonic microseconds when the device finished the frame, as the kernel stamped it.
    pub timestamp_us: u64,
}

impl Frame {
    /// The frame as packed RGB, for formats that are not compressed.
    pub fn to_rgb(&self) -> Result<Vec<u8>, Error> {
        match self.format {
            Format::Rgb24 => Ok(self.data.clone()),
            Format::Yuyv => convert::yuyv_to_rgb(&self.data, self.width, self.height),
            Format::Mjpeg => Err(Error::Unsupported(
                "MJPEG frames are passed on as JPEG; decode them where they are used".into(),
            )),
        }
    }
}

/// What to ask a camera for. The device may round size and rate to what it supports;
/// the stream reports what it agreed to.
#[derive(Debug, Clone)]
pub struct Request {
    pub width: u32,
    pub height: u32,
    /// Formats in order of preference.
    pub formats: Vec<Format>,
    pub fps: u32,
    /// Buffers the kernel fills ahead of the reader (2..=8).
    pub buffers: u32,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            formats: vec![Format::Mjpeg, Format::Yuyv],
            fps: 60,
            buffers: 4,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{call} failed: {source}")]
    Ioctl {
        call: &'static str,
        source: std::io::Error,
    },
    #[error("not a capture device: {0}")]
    NotCapture(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("frame refused: {0}")]
    Frame(String),
}
