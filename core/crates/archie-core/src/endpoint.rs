//! Binding and connecting to the daemon's control endpoint.
//!
//! # Platform split
//!
//! | Platform | Endpoint | Authorisation |
//! |---|---|---|
//! | Linux (production) | Unix socket `/run/archie/archied.sock` | filesystem permissions |
//! | Other (development) | loopback TCP, port published in `archied.endpoint` | loopback only |
//!
//! The Unix socket is the real design. Filesystem permissions *are* the access control:
//! the socket is created 0660 and owned by the `archie` group, so group membership is
//! the check, enforced by the kernel rather than by anything ArchieOS has to get right.
//!
//! The TCP fallback exists so the control plane can be developed on a non-Linux host.
//! It binds `127.0.0.1` only and is not intended for production.

use crate::control::{ControlError, Result};
use archie_state::StatePaths;
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
pub type ControlStream = tokio::net::UnixStream;
#[cfg(not(unix))]
pub type ControlStream = tokio::net::TcpStream;

#[cfg(unix)]
type PlatformListener = tokio::net::UnixListener;
#[cfg(not(unix))]
type PlatformListener = tokio::net::TcpListener;

/// A bound endpoint plus the exclusive per-node process lock.
///
/// Keeping the lock inside the listener makes its lifetime impossible to shorten by
/// accident: the node is exclusively owned for exactly as long as it can accept
/// control connections.
#[derive(Debug)]
pub struct ControlListener {
    inner: PlatformListener,
    _instance_lock: File,
}

const LIVE_ENDPOINT_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

fn acquire_instance_lock(paths: &StatePaths) -> Result<File> {
    let path = paths.control_lock_file();
    let lock = OpenOptions::new()
        .create(true)
        // Explicitly not truncating. This is an advisory lock file acquired below with
        // `try_lock_exclusive`, and emptying a file another process may hold a lock on
        // would be the bug — so the intent is stated rather than left to the default.
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)?;
    lock.try_lock_exclusive()
        .map_err(|error| ControlError::AlreadyRunning(format!("{}: {error}", path.display())))?;
    Ok(lock)
}

async fn endpoint_is_live(paths: &StatePaths) -> bool {
    tokio::time::timeout(LIVE_ENDPOINT_PROBE_TIMEOUT, connect(paths))
        .await
        .is_ok_and(|result| result.is_ok())
}

/// Bind the control endpoint and publish how to reach it.
#[cfg(unix)]
pub async fn bind(paths: &StatePaths) -> Result<ControlListener> {
    let socket = paths.control_socket();
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Check before taking the new lock so an older daemon version, which does not
    // hold archied.lock, is still protected from being displaced during upgrades.
    if endpoint_is_live(paths).await {
        return Err(ControlError::AlreadyRunning(socket.display().to_string()));
    }
    let instance_lock = acquire_instance_lock(paths)?;
    // Close the race between the liveness probe and lock acquisition.
    if endpoint_is_live(paths).await {
        return Err(ControlError::AlreadyRunning(socket.display().to_string()));
    }

    // Only the lock owner may remove a stale socket left by a crashed daemon.
    if socket.exists() {
        std::fs::remove_file(&socket)?;
    }

    let listener = PlatformListener::bind(&socket)?;
    restrict_socket_permissions(&socket)?;
    std::fs::write(
        paths.control_endpoint_file(),
        format!("unix:{}\n", socket.display()),
    )?;
    Ok(ControlListener {
        inner: listener,
        _instance_lock: instance_lock,
    })
}

#[cfg(not(unix))]
pub async fn bind(paths: &StatePaths) -> Result<ControlListener> {
    if let Some(parent) = paths.control_endpoint_file().parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Protect a daemon built before the lock existed, then acquire the authoritative
    // per-root lock and probe once more to close the race.
    if endpoint_is_live(paths).await {
        return Err(ControlError::AlreadyRunning(
            paths.control_endpoint_file().display().to_string(),
        ));
    }
    let instance_lock = acquire_instance_lock(paths)?;
    if endpoint_is_live(paths).await {
        return Err(ControlError::AlreadyRunning(
            paths.control_endpoint_file().display().to_string(),
        ));
    }

    // Loopback only: this path is a development convenience, never a network service.
    let listener = PlatformListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    std::fs::write(paths.control_endpoint_file(), format!("tcp:{addr}\n"))?;
    Ok(ControlListener {
        inner: listener,
        _instance_lock: instance_lock,
    })
}

/// Connect to a running daemon.
#[cfg(unix)]
pub async fn connect(paths: &StatePaths) -> Result<ControlStream> {
    let socket = paths.control_socket();
    ControlStream::connect(&socket).await.map_err(|e| {
        ControlError::NotRunning(format!(
            "{}: {e}. Is archied running? Try: systemctl status archied",
            socket.display()
        ))
    })
}

#[cfg(not(unix))]
pub async fn connect(paths: &StatePaths) -> Result<ControlStream> {
    let endpoint_file = paths.control_endpoint_file();
    let published = std::fs::read_to_string(&endpoint_file).map_err(|e| {
        ControlError::NotRunning(format!(
            "cannot read {}: {e}. Is archied running?",
            endpoint_file.display()
        ))
    })?;
    let addr = published.trim().strip_prefix("tcp:").ok_or_else(|| {
        ControlError::NotRunning(format!("malformed endpoint file: {published:?}"))
    })?;
    ControlStream::connect(addr)
        .await
        .map_err(|e| ControlError::NotRunning(format!("{addr}: {e}. Is archied running?")))
}

/// Accept one connection.
pub async fn accept(listener: &ControlListener) -> Result<ControlStream> {
    let (stream, _addr) = listener.inner.accept().await?;
    Ok(stream)
}

/// Remove the endpoint files on shutdown, so a stale socket does not outlive the daemon.
pub fn cleanup(paths: &StatePaths) {
    let _ = std::fs::remove_file(paths.control_endpoint_file());
    #[cfg(unix)]
    let _ = std::fs::remove_file(paths.control_socket());
    let _ = std::fs::remove_file(paths.control_lock_file());
}

/// Restrict the socket so only the owner and the archie group may connect.
#[cfg(unix)]
fn restrict_socket_permissions(socket: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    // 0660: owner and group read/write, nothing for others. This is the access control
    // for the entire control plane.
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o660))?;
    Ok(())
}

#[cfg(not(unix))]
#[allow(dead_code)]
fn restrict_socket_permissions(_socket: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{request_over, serve_connection, ControlRequest, ControlResponse};

    fn temp_paths(tag: &str) -> StatePaths {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("archie-endpoint-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = StatePaths::rooted(&root);
        paths.ensure_directories().unwrap();
        paths
    }

    #[tokio::test]
    async fn a_client_reaches_the_daemon_over_the_real_endpoint() {
        let paths = temp_paths("roundtrip");
        let listener = bind(&paths).await.unwrap();

        tokio::spawn(async move {
            let stream = accept(&listener).await.unwrap();
            serve_connection(stream, |request| async move {
                ControlResponse::Pong {
                    node_id: request.name().to_string(),
                    version: "0.1.0".into(),
                }
            })
            .await
            .ok();
        });

        let mut client = connect(&paths).await.unwrap();
        let response = request_over(&mut client, &ControlRequest::Status)
            .await
            .unwrap();
        match response {
            ControlResponse::Pong { node_id, .. } => assert_eq!(node_id, "status"),
            other => panic!("unexpected {other:?}"),
        }

        cleanup(&paths);
    }

    #[tokio::test]
    async fn binding_publishes_an_endpoint_file() {
        let paths = temp_paths("published");
        let _listener = bind(&paths).await.unwrap();
        let published = std::fs::read_to_string(paths.control_endpoint_file()).unwrap();
        assert!(
            published.starts_with("unix:") || published.starts_with("tcp:"),
            "endpoint file must name its transport, got {published:?}"
        );
        cleanup(&paths);
    }

    #[tokio::test]
    async fn connecting_with_no_daemon_reports_that_clearly() {
        let paths = temp_paths("nodaemon");
        let err = connect(&paths).await.unwrap_err();
        // The operator's first question is "is it running?", so the error must answer it.
        assert!(matches!(err, ControlError::NotRunning(_)), "got {err:?}");
        assert!(err.to_string().contains("archied"));
    }

    #[tokio::test]
    async fn cleanup_removes_the_endpoint_files() {
        let paths = temp_paths("cleanup");
        let listener = bind(&paths).await.unwrap();
        assert!(paths.control_endpoint_file().exists());
        drop(listener);
        cleanup(&paths);
        assert!(!paths.control_endpoint_file().exists());
        assert!(!paths.control_lock_file().exists());
    }

    #[tokio::test]
    async fn a_second_daemon_cannot_take_over_the_same_node() {
        let paths = temp_paths("exclusive");
        let listener = bind(&paths).await.unwrap();
        let err = bind(&paths).await.unwrap_err();
        assert!(
            matches!(err, ControlError::AlreadyRunning(_)),
            "got {err:?}"
        );
        drop(listener);
        cleanup(&paths);
    }

    #[tokio::test]
    async fn a_stale_lock_file_does_not_block_restart() {
        let paths = temp_paths("stale-lock");
        std::fs::write(paths.control_lock_file(), "stale\n").unwrap();
        let listener = bind(&paths).await.unwrap();
        drop(listener);
        cleanup(&paths);
    }

    #[cfg(not(unix))]
    #[tokio::test]
    async fn a_stale_tcp_endpoint_is_replaced_on_restart() {
        let paths = temp_paths("stale-tcp");
        std::fs::write(paths.control_endpoint_file(), "tcp:127.0.0.1:1\n").unwrap();
        let listener = bind(&paths).await.unwrap();
        let published = std::fs::read_to_string(paths.control_endpoint_file()).unwrap();
        assert_ne!(published.trim(), "tcp:127.0.0.1:1");
        drop(listener);
        cleanup(&paths);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_stale_socket_does_not_block_a_restart() {
        let paths = temp_paths("stale");
        let listener = bind(&paths).await.unwrap();
        drop(listener); // simulate a crash: the socket file survives

        assert!(paths.control_socket().exists());
        // A restart must succeed rather than failing on EADDRINUSE.
        let _listener = bind(&paths).await.unwrap();
        cleanup(&paths);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_control_socket_is_not_world_accessible() {
        use std::os::unix::fs::PermissionsExt;
        let paths = temp_paths("perms");
        let _listener = bind(&paths).await.unwrap();
        let mode = std::fs::metadata(paths.control_socket())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode & 0o007,
            0,
            "the control socket must not be accessible to other users"
        );
        cleanup(&paths);
    }
}
