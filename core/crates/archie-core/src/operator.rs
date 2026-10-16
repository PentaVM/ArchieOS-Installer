//! Authentication comes from the Linux kernel, never from a request's JSON.
pub use crate::operator_session::serve_operator_connection;
use archie_sentry::engine::EngineError;

#[derive(Debug, Clone)]
pub struct KernelOperator {
    uid: u32,
    pid: u32,
    start_ticks: u64,
    terminal: i64,
    session: i64,
    group: i64,
}
impl KernelOperator {
    #[cfg(target_os = "linux")]
    pub fn from_stream(
        stream: &crate::endpoint::ControlStream,
        allowed: &[u32],
    ) -> Result<Self, EngineError> {
        let credentials = stream.peer_cred().map_err(|_| EngineError::Authority)?;
        let uid = credentials.uid();
        if !allowed.contains(&uid) {
            return Err(EngineError::Authority);
        }
        let pid = u32::try_from(credentials.pid().ok_or(EngineError::Authority)?)
            .map_err(|_| EngineError::Authority)?;
        let identity = process_identity(pid, uid)?;
        Ok(Self {
            uid,
            pid,
            start_ticks: identity.start,
            terminal: identity.tty,
            session: identity.session,
            group: identity.group,
        })
    }
    #[cfg(not(target_os = "linux"))]
    pub fn from_stream(_: &crate::endpoint::ControlStream, _: &[u32]) -> Result<Self, EngineError> {
        Err(EngineError::Authority)
    }
    pub fn uid(&self) -> u32 {
        self.uid
    }
    pub fn pid(&self) -> u32 {
        self.pid
    }
    pub fn check_present(&self) -> Result<(), EngineError> {
        let identity = process_identity(self.pid, self.uid)?;
        if identity.start != self.start_ticks
            || identity.tty == 0
            || identity.tty != self.terminal
            || identity.session != self.session
            || identity.group != self.group
            || identity.foreground <= 0
            || identity.foreground != self.group
        {
            return Err(EngineError::Authority);
        }
        Ok(())
    }
}
struct ProcessIdentity {
    start: u64,
    tty: i64,
    session: i64,
    group: i64,
    foreground: i64,
}
#[cfg(target_os = "linux")]
fn process_identity(pid: u32, uid: u32) -> Result<ProcessIdentity, EngineError> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .map_err(|_| EngineError::Authority)?;
    let effective_uid = status
        .lines()
        .find_map(|line| {
            line.strip_prefix("Uid:")
                .and_then(|v| v.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u32>().ok())
        })
        .ok_or(EngineError::Authority)?;
    if effective_uid != uid {
        return Err(EngineError::Authority);
    }
    let stat =
        std::fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|_| EngineError::Authority)?;
    let (_, fields) = stat.rsplit_once(") ").ok_or(EngineError::Authority)?;
    let fields: Vec<_> = fields.split_whitespace().collect();
    let at = |index| fields.get(index).copied().ok_or(EngineError::Authority);
    if matches!(at(0)?, "Z" | "X") {
        return Err(EngineError::Authority);
    }
    let start = at(19)?.parse::<u64>().map_err(|_| EngineError::Authority)?;
    let tty = at(4)?.parse::<i64>().map_err(|_| EngineError::Authority)?;
    let group = at(2)?.parse::<i64>().map_err(|_| EngineError::Authority)?;
    let foreground = at(5)?.parse::<i64>().map_err(|_| EngineError::Authority)?;
    let session = at(3)?.parse::<i64>().map_err(|_| EngineError::Authority)?;
    Ok(ProcessIdentity {
        start,
        tty,
        session,
        group,
        foreground,
    })
}
#[cfg(not(target_os = "linux"))]
fn process_identity(_: u32, _: u32) -> Result<ProcessIdentity, EngineError> {
    Err(EngineError::Authority)
}
