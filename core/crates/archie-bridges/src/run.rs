//! Running an application's program: a fixed binary, arguments built by the bridge, no
//! shell, a time limit, and a cap on what it may say.
//!
//! The environment is emptied and refilled with what a desktop program needs to find its
//! own configuration (`HOME`, `XDG_*`, locale), plus the variables the bridge names. A
//! program never inherits a secret that happened to be in the server's environment.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The most a program's stdout or stderr is kept, each.
pub const MAX_OUTPUT: u64 = 1024 * 1024;

const PASSED: [&str; 10] = [
    "PATH",
    "HOME",
    "LANG",
    "LC_ALL",
    "TZ",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_DATA_DIRS",
];

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(
        "{program} could not start: {detail}. Is it installed? The bridge's manifest says how."
    )]
    Start { program: String, detail: String },
    #[error("{program} ran longer than {limit:?} and was stopped")]
    TooLong { program: String, limit: Duration },
    #[error("{program}: {detail}")]
    Wait { program: String, detail: String },
}

/// What a program did.
#[derive(Debug, Clone)]
pub struct Ran {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub seconds: f64,
}

impl Ran {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// The last lines the program wrote to stderr, then stdout: what a planner needs to
    /// see when it failed.
    pub fn said(&self) -> String {
        let text = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        lines[lines.len().saturating_sub(12)..].join("\n")
    }
}

pub struct Run {
    program: PathBuf,
    args: Vec<std::ffi::OsString>,
    cwd: Option<PathBuf>,
    env: Vec<(String, std::ffi::OsString)>,
    input: Option<Vec<u8>>,
    limit: Duration,
}

impl Run {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            input: None,
            limit: Duration::from_secs(120),
        }
    }

    pub fn arg(mut self, arg: impl AsRef<std::ffi::OsStr>) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|a| a.as_ref().to_os_string()));
        self
    }

    pub fn cwd(mut self, dir: &Path) -> Self {
        self.cwd = Some(dir.to_path_buf());
        self
    }

    pub fn env(mut self, key: &str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        self.env
            .push((key.to_string(), value.as_ref().to_os_string()));
        self
    }

    /// Bytes written to the program's standard input, which is closed after them.
    pub fn input(mut self, bytes: Vec<u8>) -> Self {
        self.input = Some(bytes);
        self
    }

    pub fn limit(mut self, limit: Duration) -> Self {
        self.limit = limit;
        self
    }

    pub fn run(self) -> Result<Ran, RunError> {
        let name = self.program.display().to_string();
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .env_clear()
            .stdin(if self.input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in PASSED {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        // Windows programs need these to start at all.
        #[cfg(windows)]
        for key in [
            "SYSTEMROOT",
            "WINDIR",
            "TEMP",
            "TMP",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        for (key, value) in &self.env {
            command.env(key, value);
        }
        if let Some(dir) = &self.cwd {
            command.current_dir(dir);
        }
        let started = Instant::now();
        let mut child = command.spawn().map_err(|e| RunError::Start {
            program: name.clone(),
            detail: e.to_string(),
        })?;
        if let (Some(bytes), Some(mut stdin)) = (self.input, child.stdin.take()) {
            // Written from its own thread, so a program that answers before reading all of
            // its input cannot deadlock against us.
            std::thread::spawn(move || {
                use std::io::Write;
                let _ = stdin.write_all(&bytes);
            });
        }
        // Both pipes are drained as the program runs, so a chatty one cannot stall.
        let drain = |pipe: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                if let Some(pipe) = pipe {
                    let mut pipe = pipe.take(MAX_OUTPUT);
                    let _ = pipe.read_to_end(&mut bytes);
                    // Whatever is past the cap is read and dropped.
                    let _ = std::io::copy(&mut pipe.into_inner(), &mut std::io::sink());
                }
                String::from_utf8_lossy(&bytes).into_owned()
            })
        };
        let stdout = drain(
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        let stderr = drain(
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(e) => {
                    let _ = child.kill();
                    return Err(RunError::Wait {
                        program: name,
                        detail: e.to_string(),
                    });
                }
            }
            if started.elapsed() > self.limit {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::TooLong {
                    program: name,
                    limit: self.limit,
                });
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        Ok(Ran {
            code: status.code(),
            stdout: stdout.join().unwrap_or_default(),
            stderr: stderr.join().unwrap_or_default(),
            seconds: started.elapsed().as_secs_f64(),
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_missing_program_says_how_to_get_it() {
        let error = Run::new("/nonexistent/kicad-cli")
            .run()
            .unwrap_err()
            .to_string();
        assert!(error.contains("manifest"), "{error}");
    }

    #[test]
    fn the_environment_is_not_inherited() {
        std::env::set_var("ARCHIE_TEST_SECRET", "hunter2");
        let ran = Run::new("/bin/sh").args(["-c", "env"]).run().unwrap();
        assert!(ran.success());
        assert!(!ran.stdout.contains("hunter2"));
        let ran = Run::new("/bin/sh")
            .args(["-c", "echo $ARCHIE_JOB"])
            .env("ARCHIE_JOB", "named")
            .run()
            .unwrap();
        assert_eq!(ran.stdout.trim(), "named");
    }

    #[test]
    fn a_program_past_its_limit_is_stopped() {
        let error = Run::new("/bin/sh")
            .args(["-c", "sleep 5"])
            .limit(Duration::from_millis(200))
            .run()
            .unwrap_err();
        assert!(matches!(error, RunError::TooLong { .. }));
    }
}
