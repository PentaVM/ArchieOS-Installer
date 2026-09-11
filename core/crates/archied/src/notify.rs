//! The systemd notification protocol, implemented directly.
//!
//! # The problem this fixes
//!
//! `archied` was `Type=simple`, so systemd considered it started the instant it was
//! `exec`'d — before identity, state, fabric or topology existed. Everything ordered
//! `After=archied.service` therefore raced it, including `archie-displayd`, which would
//! connect to a control socket that had not been bound yet and report the daemon
//! unreachable on a perfectly healthy boot.
//!
//! `Type=notify` moves readiness from "the process exists" to "the process said so", and
//! the only honest place to say so is the moment the boot report reaches ARCHIE READY.
//!
//! # Why not link libsystemd
//!
//! The protocol is a datagram of `KEY=VALUE` lines to an `AF_UNIX` socket whose path is
//! in `$NOTIFY_SOCKET`. That is perhaps forty lines. Linking libsystemd to get them
//! would put a C dependency into the trusted build and make the daemon harder to build
//! on a machine without systemd headers, in exchange for nothing.
//!
//! # The watchdog rule that matters
//!
//! `WATCHDOG=1` must be sent from whatever loop actually does the daemon's work. A timer
//! task that pings on its own proves only that the timer task is alive — which is
//! precisely the failure a watchdog exists to catch. Here the ping is emitted from the
//! same `select!` that serves the fabric and the control endpoint, so a wedged main loop
//! stops petting the dog and systemd restarts the daemon.

use std::time::Duration;

/// The environment a notifier was built from.
///
/// Captured once at startup rather than read per-call, because systemd unsets these for
/// child processes and re-reading later would silently stop the notifications.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NotifyEnvironment {
    /// `$NOTIFY_SOCKET`. Absent outside systemd.
    pub socket: Option<String>,
    /// `$WATCHDOG_USEC`, the interval systemd will kill us after.
    pub watchdog_usec: Option<u64>,
    /// `$WATCHDOG_PID`. When set, only that pid may pet the watchdog.
    pub watchdog_pid: Option<u32>,
}

impl NotifyEnvironment {
    /// Read from the process environment.
    pub fn from_env() -> Self {
        Self {
            socket: std::env::var("NOTIFY_SOCKET")
                .ok()
                .filter(|s| !s.is_empty()),
            watchdog_usec: std::env::var("WATCHDOG_USEC")
                .ok()
                .and_then(|v| v.parse().ok()),
            watchdog_pid: std::env::var("WATCHDOG_PID")
                .ok()
                .and_then(|v| v.parse().ok()),
        }
    }

    /// How often to send `WATCHDOG=1`.
    ///
    /// systemd's documented contract is to ping at *half* the configured interval, so a
    /// single delayed ping does not kill a healthy daemon. Returning `None` means the
    /// watchdog is disabled and no timer should be armed at all.
    ///
    /// `WATCHDOG_PID` is honoured: systemd sets it so that a forked child does not keep
    /// petting a watchdog on behalf of a parent that has died.
    pub fn watchdog_interval(&self, our_pid: u32) -> Option<Duration> {
        let usec = self.watchdog_usec?;
        if usec == 0 {
            return None;
        }
        if let Some(pid) = self.watchdog_pid {
            if pid != our_pid {
                return None;
            }
        }
        // Guard against a pathologically small interval turning into a busy loop.
        Some(Duration::from_micros((usec / 2).max(1_000)))
    }

    /// Whether systemd is listening at all.
    pub fn is_supervised(&self) -> bool {
        self.socket.is_some()
    }
}

/// One notification to systemd.
///
/// A small closed set rather than free-form strings: every message the daemon can send
/// is visible here, and a typo cannot become a silently ignored notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notification {
    /// Startup finished. Sent exactly once, at ARCHIE READY.
    Ready { status: String },
    /// A one-line status for `systemctl status`.
    Status(String),
    /// Still alive. Sent from the main loop.
    Watchdog,
    /// Shutting down, so systemd does not treat the exit as a failure to be restarted.
    Stopping { status: String },
}

impl Notification {
    /// Encode to the wire format: newline-separated `KEY=VALUE`.
    ///
    /// Newlines inside a status would split it into bogus keys, so they are collapsed to
    /// spaces rather than escaped — systemd has no escaping for this and a mangled
    /// message is worse than a flattened one.
    pub fn encode(&self) -> String {
        match self {
            Self::Ready { status } => {
                format!("READY=1\nSTATUS={}\n", sanitize(status))
            }
            Self::Status(status) => format!("STATUS={}\n", sanitize(status)),
            Self::Watchdog => "WATCHDOG=1\n".to_string(),
            Self::Stopping { status } => {
                format!("STOPPING=1\nSTATUS={}\n", sanitize(status))
            }
        }
    }
}

/// Flatten anything that would break the line-oriented format.
fn sanitize(status: &str) -> String {
    let flattened: String = status
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    // systemd truncates long status strings anyway; doing it here keeps the datagram
    // well under any socket buffer limit.
    let trimmed = flattened.trim();
    if trimmed.len() > 256 {
        trimmed.chars().take(253).collect::<String>() + "..."
    } else {
        trimmed.to_string()
    }
}

/// Sends notifications to systemd, or does nothing when unsupervised.
#[derive(Debug, Clone)]
pub struct Notifier {
    environment: NotifyEnvironment,
}

impl Notifier {
    pub fn new(environment: NotifyEnvironment) -> Self {
        Self { environment }
    }

    pub fn from_env() -> Self {
        Self::new(NotifyEnvironment::from_env())
    }

    pub fn environment(&self) -> &NotifyEnvironment {
        &self.environment
    }

    /// Send a notification.
    ///
    /// Running outside systemd is the ordinary development case, so an absent
    /// `$NOTIFY_SOCKET` is a silent no-op rather than an error. A *failed send* to a
    /// socket that does exist is different and is reported, because it means the daemon
    /// believes it is supervised and is wrong.
    pub fn notify(&self, notification: &Notification) -> std::io::Result<()> {
        let Some(socket) = &self.environment.socket else {
            return Ok(());
        };
        send(socket, notification.encode().as_bytes())
    }

    /// Announce readiness. Call once, when the boot report actually says READY.
    pub fn ready(&self, status: impl Into<String>) -> std::io::Result<()> {
        self.notify(&Notification::Ready {
            status: status.into(),
        })
    }

    pub fn status(&self, status: impl Into<String>) -> std::io::Result<()> {
        self.notify(&Notification::Status(status.into()))
    }

    /// Pet the watchdog. Call from the main loop, never from a dedicated timer task.
    pub fn watchdog(&self) -> std::io::Result<()> {
        self.notify(&Notification::Watchdog)
    }

    pub fn stopping(&self, status: impl Into<String>) -> std::io::Result<()> {
        self.notify(&Notification::Stopping {
            status: status.into(),
        })
    }

    /// The watchdog ping interval for this process, if any.
    pub fn watchdog_interval(&self) -> Option<Duration> {
        self.environment.watchdog_interval(std::process::id())
    }
}

#[cfg(unix)]
fn send(socket: &str, payload: &[u8]) -> std::io::Result<()> {
    use std::os::unix::net::UnixDatagram;

    let sender = UnixDatagram::unbound()?;
    // A leading '@' means the abstract namespace, whose addresses begin with a NUL byte.
    // `send_to` reads that as a path with an interior NUL and refuses it, so the abstract
    // form goes through Linux's own extension trait. The abstract namespace is a Linux
    // facility, not a POSIX one: on any other unix there is nothing to fall back to, and
    // saying so is better than silently sending nowhere.
    match socket.strip_prefix('@') {
        Some(name) => {
            #[cfg(target_os = "linux")]
            {
                use std::os::linux::net::SocketAddrExt;
                let addr = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
                sender.send_to_addr(payload, &addr)?;
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = name;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "abstract unix sockets are a Linux facility",
                ));
            }
        }
        None => {
            sender.send_to(payload, socket)?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn send(_socket: &str, _payload: &[u8]) -> std::io::Result<()> {
    // Windows is a development host. systemd is not present, so `$NOTIFY_SOCKET` is
    // never set and this is unreachable in practice; returning Ok keeps the caller
    // free of platform branches.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_carries_both_the_flag_and_a_status() {
        let encoded = Notification::Ready {
            status: "ARCHIE READY, 11 modules, 0 peers".into(),
        }
        .encode();
        assert!(encoded.starts_with("READY=1\n"));
        assert!(encoded.contains("STATUS=ARCHIE READY, 11 modules, 0 peers"));
        assert!(encoded.ends_with('\n'));
    }

    #[test]
    fn the_watchdog_message_is_exactly_what_systemd_expects() {
        assert_eq!(Notification::Watchdog.encode(), "WATCHDOG=1\n");
    }

    #[test]
    fn stopping_tells_systemd_the_exit_is_deliberate() {
        // Without this, a clean shutdown can be treated as a failure and restarted.
        let encoded = Notification::Stopping {
            status: "shutting down".into(),
        }
        .encode();
        assert!(encoded.starts_with("STOPPING=1\n"));
    }

    #[test]
    fn a_newline_in_a_status_cannot_forge_another_key() {
        // The format is line-oriented, so an embedded newline in a status would be read
        // as a new KEY=VALUE. A hostile hostname or a multi-line error must not be able
        // to inject READY=1.
        let encoded = Notification::Status("line one\nREADY=1\nline two".into()).encode();
        assert_eq!(
            encoded.matches('\n').count(),
            1,
            "the message must remain a single line: {encoded:?}"
        );
        assert!(!encoded.contains("\nREADY=1"));
        assert!(encoded.starts_with("STATUS=line one READY=1 line two"));
    }

    #[test]
    fn a_carriage_return_is_flattened_too() {
        let encoded = Notification::Status("a\r\nb".into()).encode();
        assert_eq!(encoded.matches('\n').count(), 1);
    }

    #[test]
    fn an_overlong_status_is_truncated_rather_than_sent_whole() {
        let encoded = Notification::Status("x".repeat(4000)).encode();
        assert!(encoded.len() < 300, "got {} bytes", encoded.len());
        assert!(encoded.contains("..."));
    }

    // ────────────────────────────────────────────────────────────────── watchdog

    fn environment(usec: Option<u64>, pid: Option<u32>) -> NotifyEnvironment {
        NotifyEnvironment {
            socket: Some("/run/systemd/notify".into()),
            watchdog_usec: usec,
            watchdog_pid: pid,
        }
    }

    #[test]
    fn the_ping_interval_is_half_the_configured_timeout() {
        // systemd's documented contract: pinging at the full interval means one delayed
        // ping kills a healthy daemon.
        let env = environment(Some(30_000_000), None);
        assert_eq!(env.watchdog_interval(1234), Some(Duration::from_secs(15)));
    }

    #[test]
    fn no_watchdog_configured_means_no_timer_is_armed() {
        assert_eq!(environment(None, None).watchdog_interval(1), None);
        assert_eq!(environment(Some(0), None).watchdog_interval(1), None);
    }

    #[test]
    fn only_the_designated_process_pets_the_watchdog() {
        // systemd sets WATCHDOG_PID so a forked child cannot keep a dead parent's
        // watchdog alive.
        let env = environment(Some(30_000_000), Some(1234));
        assert!(env.watchdog_interval(1234).is_some());
        assert_eq!(
            env.watchdog_interval(5678),
            None,
            "a process that is not the designated one must not pet the watchdog"
        );
    }

    #[test]
    fn a_tiny_interval_cannot_become_a_busy_loop() {
        let env = environment(Some(10), None);
        assert_eq!(env.watchdog_interval(1), Some(Duration::from_micros(1_000)));
    }

    // ───────────────────────────────────────────────────────────────── notifier

    #[test]
    fn running_outside_systemd_is_a_silent_no_op() {
        // The ordinary development case. It must not error and must not be mistaken for
        // supervision.
        let notifier = Notifier::new(NotifyEnvironment::default());
        assert!(!notifier.environment().is_supervised());
        assert!(notifier.ready("up").is_ok());
        assert!(notifier.watchdog().is_ok());
        assert!(notifier.status("x").is_ok());
        assert!(notifier.stopping("bye").is_ok());
        assert_eq!(notifier.watchdog_interval(), None);
    }

    #[test]
    fn a_notifier_reports_whether_it_is_supervised() {
        assert!(Notifier::new(environment(None, None))
            .environment()
            .is_supervised());
        assert!(!Notifier::new(NotifyEnvironment::default())
            .environment()
            .is_supervised());
    }

    #[test]
    fn an_empty_notify_socket_is_treated_as_absent() {
        // systemd never sets it empty, but an operator writing a unit by hand might.
        let env = NotifyEnvironment {
            socket: Some(String::new()).filter(|s| !s.is_empty()),
            ..Default::default()
        };
        assert!(!env.is_supervised());
    }
}
