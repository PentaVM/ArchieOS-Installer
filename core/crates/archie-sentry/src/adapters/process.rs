//! Process execution for adapters that speak through local CLI tools
//! (ssh, adb). The binary path is always absolute and configured; there is
//! never a shell, so an argument is data and can never become syntax.

use crate::engine::AdapterError;
use std::{
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// What a finished tool said, bounded on both channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// A runner is injected so tests never touch a real binary, and so the real
/// one is the only place processes exist.
pub trait CommandRunner: Send + Sync {
    fn run(&self, argv: &[String], timeout: Duration) -> Result<CommandOutput, AdapterError>;
}

/// The real runner. Configuration hands over an absolute tool path; anything
/// else is refused before a process exists.
pub struct ProcessRunner {
    max_output: usize,
}
impl Default for ProcessRunner {
    fn default() -> Self {
        Self {
            max_output: 262_144,
        }
    }
}
impl ProcessRunner {
    /// The tool path must be absolute and an existing file. No PATH search,
    /// no relative resolution, no surprise binary.
    pub fn tool(path: &str) -> Result<PathBuf, AdapterError> {
        let path = PathBuf::from(path);
        if !path.is_absolute() || !path.is_file() {
            return Err(AdapterError::Protocol);
        }
        Ok(path)
    }
}
impl CommandRunner for ProcessRunner {
    fn run(&self, argv: &[String], timeout: Duration) -> Result<CommandOutput, AdapterError> {
        let (program, args) = argv.split_first().ok_or(AdapterError::Protocol)?;
        Self::tool(program)?;
        let mut child = Command::new(program)
            .args(args)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| AdapterError::Unavailable)?;
        let cap = self.max_output;
        let mut out_pipe = child.stdout.take().ok_or(AdapterError::Unavailable)?;
        let mut err_pipe = child.stderr.take().ok_or(AdapterError::Unavailable)?;
        let reader = move |pipe: &mut dyn Read| -> Vec<u8> {
            let mut buf = Vec::new();
            // Read at most cap + 1 so an unbounded stream is noticed, then stop.
            let _ = pipe.take(cap as u64 + 1).read_to_end(&mut buf);
            buf.truncate(cap);
            buf
        };
        let out_handle = thread::spawn(move || reader(&mut out_pipe));
        let err_handle = thread::spawn(move || reader(&mut err_pipe));
        let started = Instant::now();
        let status = loop {
            if let Ok(Some(status)) = child.try_wait() {
                break status.code().unwrap_or(-1);
            }
            if started.elapsed() > timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AdapterError::Unavailable);
            }
            thread::sleep(Duration::from_millis(10));
        };
        let stdout = out_handle.join().unwrap_or_default();
        let stderr = err_handle.join().unwrap_or_default();
        Ok(CommandOutput {
            status,
            stdout,
            stderr,
        })
    }
}

/// Runs blocking adapter work (subprocesses waited on with `try_wait`) on Tokio's
/// blocking pool. Called inline from an `async fn`, that work would hold an async
/// worker for its whole length and finish on the first poll, so the engine's
/// per-intent deadline could never fire. Here the engine's timeout drops this future
/// and returns on time; the runner's own timeout still kills the process.
pub(crate) async fn off_runtime<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, AdapterError> + Send + 'static,
) -> Result<T, AdapterError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| AdapterError::Unavailable)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_or_missing_tools_never_spawn() {
        assert!(ProcessRunner::tool("ssh").is_err());
        assert!(ProcessRunner::tool("bin/ssh").is_err());
        assert!(ProcessRunner::tool("./adb.exe").is_err());
        assert!(ProcessRunner::tool("C:/definitely/not/here.exe").is_err());
        // The binary running this test is absolute and exists.
        let this = std::env::current_exe().unwrap();
        assert!(ProcessRunner::tool(this.to_str().unwrap()).is_ok());
    }

    #[test]
    fn an_empty_argv_is_refused() {
        let runner = ProcessRunner::default();
        assert!(runner.run(&[], Duration::from_millis(50)).is_err());
        assert!(runner
            .run(&["relative".into()], Duration::from_millis(50))
            .is_err());
    }
}
