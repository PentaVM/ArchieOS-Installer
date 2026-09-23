use crate::{OrbModel, Signal, SignalError};
use std::path::{Path, PathBuf};

/// Best-effort one-way producer. A missing observatory never delays speech or Stage.
#[cfg(unix)]
pub struct SignalPublisher {
    socket: std::os::unix::net::UnixDatagram,
    path: PathBuf,
}

#[cfg(unix)]
impl SignalPublisher {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, SignalError> {
        use std::os::unix::net::UnixDatagram;
        let socket = UnixDatagram::unbound().map_err(|_| SignalError::Unavailable)?;
        socket
            .set_nonblocking(true)
            .map_err(|_| SignalError::Unavailable)?;
        Ok(Self {
            socket,
            path: path.into(),
        })
    }

    /// Sends one bounded datagram and returns immediately if the socket is full.
    pub fn publish(&self, signal: &Signal) -> Result<(), SignalError> {
        let bytes = signal.encode()?;
        self.socket
            .send_to(&bytes, &self.path)
            .map(|_| ())
            .map_err(|_| SignalError::Unavailable)
    }
}

#[cfg(not(unix))]
pub struct SignalPublisher;

#[cfg(not(unix))]
impl SignalPublisher {
    pub fn new(_path: impl Into<PathBuf>) -> Result<Self, SignalError> {
        Ok(Self)
    }
    pub fn publish(&self, _signal: &Signal) -> Result<(), SignalError> {
        Err(SignalError::Unavailable)
    }
}

/// The display-owned, bounded local event receiver. It only updates an in-memory model.
#[cfg(unix)]
pub struct SignalReceiver {
    socket: tokio::net::UnixDatagram,
    path: PathBuf,
}

#[cfg(unix)]
impl SignalReceiver {
    pub async fn bind(path: impl AsRef<Path>) -> std::io::Result<Self> {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        let path = path.as_ref().to_path_buf();
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            if !metadata.file_type().is_socket() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "TFOG endpoint exists and is not a socket",
                ));
            }
            std::fs::remove_file(&path)?;
        }
        let socket = tokio::net::UnixDatagram::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660))?;
        Ok(Self { socket, path })
    }

    pub async fn receive(&self, model: &tokio::sync::Mutex<OrbModel>) -> std::io::Result<()> {
        let mut buffer = vec![0_u8; crate::signal::MAX_SIGNAL_BYTES];
        loop {
            let (size, _) = self.socket.recv_from(&mut buffer).await?;
            if let Ok(signal) = Signal::decode(&buffer[..size]) {
                let mut model = model.lock().await;
                let _ = model.apply(signal);
            }
        }
    }
}

#[cfg(unix)]
impl Drop for SignalReceiver {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(not(unix))]
pub struct SignalReceiver;

#[cfg(not(unix))]
impl SignalReceiver {
    pub async fn bind(_path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self)
    }
    pub async fn receive(&self, _model: &tokio::sync::Mutex<OrbModel>) -> std::io::Result<()> {
        Ok(())
    }
}
