//! PC control over SSH: Windows, Linux and macOS workstations reached through
//! the system `ssh` binary, reusing the router's [`SshConfig`]/[`SshChannel`]
//! for local argv construction (absolute configured binary, BatchMode, key
//! auth only, host-key policy) and [`CommandRunner`] for the only place
//! processes exist. There is never a shell on this side; every argument is a
//! separate local argv element.
//!
//! Remote commands come exclusively from static per-OS tables. ssh joins the
//! remote argv with spaces and the *remote* shell re-parses it, so table
//! entries carry their own quoting where an argument contains spaces
//! (single quotes on POSIX targets, `-EncodedCommand` base64 on Windows
//! PowerShell, which needs no quoting at all). The only dynamic values that
//! ever reach a remote command are a volume percent (contract-validated
//! `u8`, rendered as digits), a `loginctl` session id (digits, parsed from
//! the host's own `list-sessions`), and app launch tokens from operator
//! configuration (whitelisted charset, no quotes or expansion characters).
//!
//! The capability probe ([`probe_host`]) is authoritative: it detects the OS
//! (`uname -s`, falling back to `cmd.exe /c ver`), then checks which of the
//! per-OS tools exist (`where.exe` on Windows, `command -v` elsewhere, a
//! `test -x` for macOS's CGSession). Anything the probe did not confirm is
//! [`AdapterError::Unsupported`] and is never attempted.
//!
//! Per-OS honesty notes, deliberately part of the module contract:
//! - **Windows**: lock via `rundll32.exe user32.dll,LockWorkStation`;
//!   lock-state read-back via a PowerShell `LogonUI` process check (the
//!   secure-attention screen owns a LogonUI process; when it runs, the
//!   session is locked). **Unlock is `Unsupported`**: Windows exposes no
//!   remote unlock, and this adapter never simulates input. Volume set via
//!   nircmd when present (PowerShell audio endpoint otherwise), read-back
//!   always via the PowerShell `IAudioEndpointVolume` script — a set without
//!   a read-back is not offered. Foreground app via PowerShell
//!   `GetForegroundWindow` + process name.
//! - **Linux**: lock *and* unlock via `loginctl lock-session` /
//!   `unlock-session` (unlock works where the session's own policy allows
//!   it; if the host refuses, the command fails honestly as `Unavailable`).
//!   Lock state is the `LockedHint` of every listed session: locked only
//!   when *all* sessions report it, unknown when any session cannot say.
//!   Volume via `pactl` (preferred) or `amixer`, with parsed read-backs.
//!   Foreground app via `xdotool getactivewindow getwindowclassname`.
//!   App launch via `systemd-run --user` so the process lands in the user
//!   session, from the configured allowlist only.
//! - **macOS**: lock-only via `CGSession -suspend` (fast user switching to
//!   the login window — a real lock). The postcondition is measurable:
//!   after a lock, the frontmost process reported by `osascript` (System
//!   Events) is `loginwindow`, so `session_locked` is `Some(true)` exactly
//!   when the frontmost process is `loginwindow`. Unlock is `Unsupported`:
//!   CGSession cannot bring a session back. `pmset displaysleepnow` is
//!   display sleep, not a lock, and is never used. Volume via `osascript`
//!   with `get volume settings` read-back. App launch via `open -a`.
//! - `awake` means "the host answered ssh" — the OS is up. Display sleep is
//!   not distinguished; a host that did not answer yields `Unavailable`,
//!   never an invented state.
//! - **Shutdown/reboot** have no `Command` in the contract, so they are
//!   adapter-level operations ([`HostSsh::shutdown`], [`HostSsh::reboot`]),
//!   gated on both the probe finding the OS's shutdown tool and the
//!   operator setting `allow_power` in configuration, and intended to be
//!   wrapped by policy/approval above. Their only honest postcondition is
//!   that the host stops answering; there is no state read-back, and the
//!   engine's Intent pipeline never carries them.
//!
//! **Tested vs. untested.** Parsers, argv construction, probe logic, allowlist
//! and gating behaviour are exercised against scripted fixtures with a fake
//! `CommandRunner` (`tests/adapters_host.rs`); fixtures are recorded shapes,
//! reported as fixtures. Nothing here has run against a real Windows, Linux
//! or macOS host: exact `pactl`/`osascript`/`loginctl` output flavours vary,
//! and parsers return "unknown" rather than guessing when a shape differs.

use crate::{
    adapters::{
        process::CommandRunner,
        router::ssh::{SshChannel, SshConfig},
    },
    contracts::{validate_id, Command, CommandKind, Observation, ObservationOrigin, ResourceState},
    engine::{AdapterAck, AdapterError, EdgeAdapter},
    registry::ResourceDescriptor,
};
use archie_types::Timestamp;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

/// The most login sessions whose lock state is read one by one.
const MAX_LOCK_SESSIONS: usize = 8;

/// Bound on one remote argv element. Static PowerShell `-EncodedCommand`
/// scripts are the largest entries (a few KB of base64); dynamic values are
/// digits or allowlisted tokens and stay tiny.
const MAX_REMOTE_ARG: usize = 4096;

/// The workstation operating system, as detected by the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostOs {
    Windows,
    Linux,
    MacOs,
}

/// What the host can do about session locking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockSupport {
    /// Locking is not offered (tool or read-back missing).
    #[default]
    None,
    /// Lock with a measurable postcondition; unlock honestly Unsupported
    /// (Windows, macOS: the OS exposes no remote unlock).
    LockOnly,
    /// Lock and unlock with a measured postcondition (Linux loginctl, where
    /// the session's policy permits).
    LockUnlock,
}

/// The probed volume tool. `Nircmd` still reads back through PowerShell;
/// every other tool reads back through itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeTool {
    Nircmd,
    PowerShell,
    Pactl,
    Amixer,
    Osascript,
}

/// What [`probe_host`] found. Persistable, so configuration can record the
/// discovered shape and re-probe on schedule. A capability that did not
/// answer is absent — nothing here assumes a tool exists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapabilities {
    /// ssh connected and answered an OS-detection command.
    pub ssh: bool,
    pub os: Option<HostOs>,
    pub lock: LockSupport,
    /// The volume tool the probe found, or None: VolumeSet is then
    /// Unsupported and never attempted.
    pub volume: Option<VolumeTool>,
    /// A foreground-app read exists (PowerShell / xdotool / osascript).
    pub foreground: bool,
    /// AppLaunch is offered: the launcher exists AND the foreground
    /// read-back that verifies it exists.
    pub app_launch: bool,
    /// The OS's shutdown tool answered the probe. Power actions additionally
    /// require `allow_power` in configuration.
    pub power: bool,
}
impl HostCapabilities {
    /// The contract capabilities a resource descriptor should declare for a
    /// host with this probe result — nothing more.
    pub fn command_kinds(&self) -> Vec<CommandKind> {
        let mut kinds = vec![];
        if self.lock != LockSupport::None {
            kinds.push(CommandKind::SessionLock);
        }
        if self.volume.is_some() {
            kinds.push(CommandKind::VolumeSet);
        }
        if self.app_launch {
            kinds.push(CommandKind::AppLaunch);
        }
        kinds
    }
}

/// Per-resource wiring as written in operator configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostTarget {
    pub ssh: SshConfig,
    /// `app_id` → OS launch token (a path for `start`/`systemd-run`, an
    /// application name for `open -a`). The `app_id` is also the identity
    /// the OS's foreground read must report for the launch to verify, so
    /// configure it as what the OS reports (a process name on Windows, a
    /// window class on Linux, an application name on macOS).
    pub apps: BTreeMap<String, String>,
    /// Master gate for shutdown/reboot, which live outside the Intent
    /// pipeline. Both this flag AND a probed shutdown tool are required.
    pub allow_power: bool,
}

/// A launch token is an argument inside a quoted remote command: no quotes,
/// no expansion characters (`%` for cmd, `$`/backtick for POSIX shells), no
/// control bytes. Letters, digits, spaces and path punctuation are fine.
fn valid_launch_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 256
        && token.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b' ' | b'.' | b'_' | b'-' | b'/' | b'\\' | b':' | b'(' | b')' | b'+' | b'@'
                )
        })
}

// ---------------------------------------------------------------------------
// PowerShell helpers: `-EncodedCommand` takes base64(UTF-16LE(script)), which
// needs no shell quoting on any sshd — the payload is [A-Za-z0-9+/=] only.
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding. Used for PowerShell `-EncodedCommand`.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() / 3 * 4 + 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// base64(UTF-16LE(script)) — the value of powershell.exe -EncodedCommand.
pub fn ps_encoded_command(script: &str) -> String {
    let utf16le: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    base64_encode(&utf16le)
}

/// C# interop for the master audio endpoint. Interface method order is the
/// COM vtable order; `_N` entries stand in for methods we never call. No
/// single quotes, so the PowerShell wrapper can single-quote it whole.
const PS_AUDIO_CS: &str = concat!(
    "using System;using System.Runtime.InteropServices;",
    "[ComImport,Guid(\"BCDE0395-E52F-467C-8E3D-C4579291692E\")]public class MMDeviceEnumeratorComObject{}",
    "[Guid(\"A95664D2-9614-4F35-A746-DE8DB63617E6\"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]",
    "public interface IMMDeviceEnumerator{int _1();int GetDefaultAudioEndpoint(int flow,int role,out IMMDevice d);}",
    "[Guid(\"D666063F-1587-4E43-81F1-B948E807363F\"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]",
    "public interface IMMDevice{int Activate(ref Guid iid,int ctx,IntPtr p,[MarshalAs(UnmanagedType.IUnknown)]out object o);}",
    "[Guid(\"5CDF2C82-841E-4546-9722-0CF74078229A\"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]",
    "public interface IAudioEndpointVolume{int _1();int _2();int GetChannelCount(out int n);int _3();int _4();int _5();int _6();",
    "int SetMasterVolumeLevel(float l,Guid g);int SetMasterVolumeLevelScalar(float l,Guid g);int _7();int GetMasterVolumeLevelScalar(out float l);}",
    "public static class ArchieAudio{static IAudioEndpointVolume Ep(){var e=(IMMDeviceEnumerator)new MMDeviceEnumeratorComObject();",
    "IMMDevice d;e.GetDefaultAudioEndpoint(0,0,out d);var iid=typeof(IAudioEndpointVolume).GUID;object o;",
    "d.Activate(ref iid,1,IntPtr.Zero,out o);return (IAudioEndpointVolume)o;}",
    "public static int Get(){float v;Ep().GetMasterVolumeLevelScalar(out v);return (int)Math.Round(v*100f);}",
    "public static void Set(int p){Ep().SetMasterVolumeLevelScalar(p/100f,Guid.Empty);}}"
);

/// Set the master endpoint volume, percent 0-100.
pub fn ps_audio_set(percent: u8) -> String {
    ps_encoded_command(&format!(
        "Add-Type -TypeDefinition '{PS_AUDIO_CS}';[ArchieAudio]::Set({percent})"
    ))
}
/// Print the master endpoint volume as an integer percent.
pub fn ps_audio_get() -> String {
    ps_encoded_command(&format!(
        "Add-Type -TypeDefinition '{PS_AUDIO_CS}';[ArchieAudio]::Get()"
    ))
}
/// Print the process name owning the foreground window.
pub fn ps_foreground() -> String {
    ps_encoded_command(concat!(
        "Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;",
        "public static class FgWin{[DllImport(\"user32.dll\")]public static extern IntPtr GetForegroundWindow();",
        "[DllImport(\"user32.dll\")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);}';",
        "$h=[FgWin]::GetForegroundWindow();$p=0;",
        "[void][FgWin]::GetWindowThreadProcessId($h,[ref]$p);",
        "(Get-Process -Id $p).ProcessName"
    ))
}
/// Print True when a LogonUI process exists (the lock screen owns one).
pub fn ps_lock_state() -> String {
    ps_encoded_command("(Get-Process LogonUI -ErrorAction SilentlyContinue) -ne $null")
}

/// macOS CGSession path, single-quoted for the remote shell (the real path
/// contains a space). Quoting lives in the table, nowhere else.
const CGSESSION: &str =
    "'/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession'";
/// macOS frontmost-process query, single-quoted; the script itself uses only
/// double quotes, which the remote shell leaves untouched inside it.
const MACOS_FRONTMOST: &str =
    "'tell application \"System Events\" to get name of first application process whose frontmost is true'";

// ---------------------------------------------------------------------------
// Parsers. Every one returns None for an unrecognized shape — an honest
// "the host could not say", never a guess.
// ---------------------------------------------------------------------------

/// PowerShell boolean output: exactly `True` or `False`.
pub fn parse_ps_bool(output: &str) -> Option<bool> {
    match output.trim() {
        "True" => Some(true),
        "False" => Some(false),
        _ => None,
    }
}

/// PowerShell integer-percent output (`[ArchieAudio]::Get()`), 0..=100.
/// Louder-than-100 (over-amplification) is unrepresentable in the contract
/// and reported as unknown, never clamped.
pub fn parse_ps_volume(output: &str) -> Option<u8> {
    let value: u32 = output.trim().parse().ok()?;
    (value <= 100).then_some(value as u8)
}

/// A single-line process/application name that satisfies the contract's
/// identifier rules; anything else (spaces, punctuation, extra lines' worth
/// of noise in the first line) is not evidence.
pub fn parse_process_name(output: &str) -> Option<String> {
    let name = output.lines().next()?.trim();
    validate_id(name).ok()?;
    Some(name.to_string())
}

/// Session ids from `loginctl list-sessions --no-legend`: the first field of
/// each line, digits only.
pub fn parse_session_ids(list: &str) -> Vec<String> {
    list.lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|field| {
            !field.is_empty() && field.len() <= 16 && field.bytes().all(|b| b.is_ascii_digit())
        })
        .map(String::from)
        .collect()
}

/// `loginctl show-session <id> -p LockedHint` output.
pub fn parse_locked_hint(output: &str) -> Option<bool> {
    match output.trim() {
        "LockedHint=yes" => Some(true),
        "LockedHint=no" => Some(false),
        _ => None,
    }
}

/// `pactl get-sink-volume @DEFAULT_SINK@`: the first `/ NN% /` field.
/// Values above 100 are real (over-amplification) but unrepresentable in the
/// contract, so they read as unknown.
pub fn parse_pactl_volume(output: &str) -> Option<u8> {
    let field = output.split('/').nth(1)?;
    let percent: u32 = field.trim().strip_suffix('%')?.trim().parse().ok()?;
    (percent <= 100).then_some(percent as u8)
}

/// `amixer sget Master`: the first `[NN%]` playback percentage. Capture
/// groups' `[on]`/`[off]` switches contain no `%`, so the first bracket with
/// a percent is the volume.
pub fn parse_amixer_volume(output: &str) -> Option<u8> {
    let start = output.find('[')? + 1;
    let end = output[start..].find('%')? + start;
    if !output[end..].starts_with("%]") {
        return None;
    }
    let percent: u32 = output[start..end].parse().ok()?;
    (percent <= 100).then_some(percent as u8)
}

/// `osascript -e 'get volume settings'`: the `output volume:NN` field. When
/// no output device exists the value is `missing value`, which is unknown.
pub fn parse_osascript_volume(output: &str) -> Option<u8> {
    for field in output.trim().split(',') {
        if let Some(rest) = field.trim().strip_prefix("output volume:") {
            let percent: u32 = rest.trim().parse().ok()?;
            return (percent <= 100).then_some(percent as u8);
        }
    }
    None
}

/// Windows percent 0-100 → nircmd's 0-65535 scale, rounded to nearest.
pub fn nircmd_volume(percent: u8) -> u32 {
    (u32::from(percent) * 65535 + 50) / 100
}

// ---------------------------------------------------------------------------
// The channel: router ssh plumbing (config, local argv, key policy) plus
// host-shaped validation (larger static scripts, same "no shell here" rule).
// ---------------------------------------------------------------------------

/// A configured SSH channel to a workstation, over an injected runner.
pub struct HostChannel {
    ssh: SshChannel,
    runner: Arc<dyn CommandRunner>,
}
impl std::fmt::Debug for HostChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostChannel")
            .field("user", &self.ssh.config().user)
            .field("host", &self.ssh.config().host)
            .field("port", &self.ssh.config().port)
            .finish_non_exhaustive()
    }
}
impl HostChannel {
    pub fn new(config: SshConfig, runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            ssh: SshChannel::new(config, Arc::clone(&runner)),
            runner,
        }
    }
    pub fn config(&self) -> &SshConfig {
        self.ssh.config()
    }
    /// The exact local argv for a remote command (see [`SshChannel::argv`]).
    pub fn argv(&self, remote: &[&str]) -> Vec<String> {
        self.ssh.argv(remote)
    }
    fn timeout(&self) -> Duration {
        // Connect timeout plus margin: PowerShell compiles the Add-Type
        // interop on first run, which can take seconds on a busy host.
        Duration::from_secs(u64::from(self.ssh.config().connect_timeout_secs) + 20)
    }
    /// Remote argv hygiene: non-empty, bounded, no control bytes. Table
    /// entries carry their own quoting; dynamic values are digits or
    /// allowlisted tokens, validated before they get here.
    fn check(remote: &[&str]) -> Result<(), AdapterError> {
        if remote.is_empty()
            || remote.iter().any(|arg| {
                arg.is_empty()
                    || arg.len() > MAX_REMOTE_ARG
                    || arg.bytes().any(|b| b < 0x20 || b == 0x7f)
            })
        {
            return Err(AdapterError::Protocol);
        }
        Ok(())
    }
    fn finish(output: crate::adapters::process::CommandOutput) -> Result<String, AdapterError> {
        if output.status != 0 {
            // stderr is captured, bounded by the runner, and logged locally
            // for the operator; it never enters receipts or observations.
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            if !stderr.is_empty() {
                eprintln!(
                    "archie-sentry host ssh: exit {}: {}",
                    output.status,
                    // By characters: a byte cut could split one, and that panics.
                    stderr.chars().take(200).collect::<String>()
                );
            }
            return Err(AdapterError::Unavailable);
        }
        String::from_utf8(output.stdout).map_err(|_| AdapterError::Protocol)
    }
    async fn run_async(&self, remote: &[&str]) -> Result<String, AdapterError> {
        Self::check(remote)?;
        let argv = self.ssh.argv(remote);
        let runner = Arc::clone(&self.runner);
        let timeout = self.timeout();
        tokio::task::spawn_blocking(move || Self::finish(runner.run(&argv, timeout)?))
            .await
            .map_err(|_| AdapterError::Unavailable)?
    }
}

// ---------------------------------------------------------------------------
// The probe: OS detection, then per-OS tool discovery. Read-only: the probe
// never actuates — `where.exe`, `command -v` and `test -x` only look.
// ---------------------------------------------------------------------------

async fn has_tool(channel: &HostChannel, os: HostOs, tool: &str) -> bool {
    let argv: &[&str] = match os {
        HostOs::Windows => &["where.exe", tool],
        _ => &["command", "-v", tool],
    };
    channel.run_async(argv).await.is_ok()
}

/// Probe one host. `ssh: false` (with everything else absent) means the host
/// did not answer or its OS was not recognized — the honest result.
pub async fn probe_host(config: SshConfig, runner: Arc<dyn CommandRunner>) -> HostCapabilities {
    let channel = HostChannel::new(config, runner);
    let mut caps = HostCapabilities::default();
    let os = match channel.run_async(&["uname", "-s"]).await {
        Ok(output) if output.trim() == "Linux" => HostOs::Linux,
        Ok(output) if output.trim() == "Darwin" => HostOs::MacOs,
        _ => match channel.run_async(&["cmd.exe", "/c", "ver"]).await {
            Ok(output) if output.contains("Windows") => HostOs::Windows,
            _ => return caps,
        },
    };
    caps.ssh = true;
    caps.os = Some(os);
    match os {
        HostOs::Windows => {
            let rundll32 = has_tool(&channel, os, "rundll32.exe").await;
            let powershell = has_tool(&channel, os, "powershell.exe").await;
            let nircmd = has_tool(&channel, os, "nircmd.exe").await;
            // Lock needs the action (rundll32) AND its read-back (PowerShell
            // LogonUI check); one without the other is not offered.
            caps.lock = if rundll32 && powershell {
                LockSupport::LockOnly
            } else {
                LockSupport::None
            };
            // Volume set prefers nircmd; read-back is always PowerShell, so
            // PowerShell alone is sufficient and necessary.
            caps.volume = if powershell {
                Some(if nircmd {
                    VolumeTool::Nircmd
                } else {
                    VolumeTool::PowerShell
                })
            } else {
                None
            };
            caps.foreground = powershell;
            // `start` is a cmd builtin; what gates AppLaunch is the
            // PowerShell foreground read-back that verifies it.
            caps.app_launch = powershell;
            caps.power = has_tool(&channel, os, "shutdown.exe").await;
        }
        HostOs::Linux => {
            let loginctl = has_tool(&channel, os, "loginctl").await;
            let pactl = has_tool(&channel, os, "pactl").await;
            let amixer = has_tool(&channel, os, "amixer").await;
            let systemd_run = has_tool(&channel, os, "systemd-run").await;
            let xdotool = has_tool(&channel, os, "xdotool").await;
            caps.lock = if loginctl {
                LockSupport::LockUnlock
            } else {
                LockSupport::None
            };
            caps.volume = if pactl {
                Some(VolumeTool::Pactl)
            } else if amixer {
                Some(VolumeTool::Amixer)
            } else {
                None
            };
            caps.foreground = xdotool;
            caps.app_launch = systemd_run && xdotool;
            caps.power = has_tool(&channel, os, "shutdown").await;
        }
        HostOs::MacOs => {
            let osascript = has_tool(&channel, os, "osascript").await;
            let open = has_tool(&channel, os, "open").await;
            // test -x with the table's pre-quoted path: CGSession exists on
            // every macOS, but honesty costs one round trip.
            let cgsession = channel.run_async(&["test", "-x", CGSESSION]).await.is_ok();
            caps.lock = if cgsession && osascript {
                LockSupport::LockOnly
            } else {
                LockSupport::None
            };
            caps.volume = if osascript {
                Some(VolumeTool::Osascript)
            } else {
                None
            };
            caps.foreground = osascript;
            caps.app_launch = open && osascript;
            caps.power = has_tool(&channel, os, "shutdown").await;
        }
    }
    caps
}

// ---------------------------------------------------------------------------
// The adapter.
// ---------------------------------------------------------------------------

/// One paired workstation, driven through the tools the probe confirmed.
pub struct HostSsh {
    edge: String,
    adapter: String,
    resource: String,
    channel: HostChannel,
    capabilities: HostCapabilities,
    apps: BTreeMap<String, String>,
    allow_power: bool,
    observations: u64,
}
impl std::fmt::Debug for HostSsh {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostSsh")
            .field("edge", &self.edge)
            .field("adapter", &self.adapter)
            .field("resource", &self.resource)
            .field("capabilities", &self.capabilities)
            .finish_non_exhaustive()
    }
}
impl HostSsh {
    pub fn new(
        edge: &str,
        adapter: &str,
        resource: &str,
        target: HostTarget,
        capabilities: HostCapabilities,
        runner: Arc<dyn CommandRunner>,
    ) -> Result<Self, AdapterError> {
        validate_id(edge).map_err(|_| AdapterError::Protocol)?;
        validate_id(adapter).map_err(|_| AdapterError::Protocol)?;
        validate_id(resource).map_err(|_| AdapterError::Protocol)?;
        if target.apps.len() > 64 {
            return Err(AdapterError::Protocol);
        }
        for (app_id, token) in &target.apps {
            validate_id(app_id).map_err(|_| AdapterError::Protocol)?;
            if !valid_launch_token(token) {
                return Err(AdapterError::Protocol);
            }
        }
        Ok(Self {
            edge: edge.into(),
            adapter: adapter.into(),
            resource: resource.into(),
            channel: HostChannel::new(target.ssh, runner),
            capabilities,
            apps: target.apps,
            allow_power: target.allow_power,
            observations: 0,
        })
    }
    /// What the probe found; for inspection and honest CLI reporting.
    pub fn capabilities(&self) -> &HostCapabilities {
        &self.capabilities
    }
    /// The channel, for the probe and for operators scripting reads.
    pub fn channel(&self) -> &HostChannel {
        &self.channel
    }
    fn os(&self) -> Result<HostOs, AdapterError> {
        self.capabilities.os.ok_or(AdapterError::Unsupported)
    }
    fn resource_check(&self, resource: &ResourceDescriptor) -> Result<(), AdapterError> {
        if !resource.paired
            || resource.adapter_id != self.adapter
            || resource.edge_id != self.edge
            || resource.resource_id != self.resource
        {
            return Err(AdapterError::Unsupported);
        }
        Ok(())
    }
    /// The volume read-back for the probed tool, as a percent 0-100.
    async fn volume_percent(&self, tool: VolumeTool) -> Result<Option<u8>, AdapterError> {
        let output = match tool {
            VolumeTool::Pactl => {
                self.channel
                    .run_async(&["pactl", "get-sink-volume", "@DEFAULT_SINK@"])
                    .await?
            }
            VolumeTool::Amixer => {
                self.channel
                    .run_async(&["amixer", "sget", "Master"])
                    .await?
            }
            VolumeTool::Osascript => {
                self.channel
                    .run_async(&["osascript", "-e", "'get volume settings'"])
                    .await?
            }
            // nircmd cannot print the level; PowerShell is the read-back for
            // both Windows set paths (the probe guarantees it exists).
            VolumeTool::Nircmd | VolumeTool::PowerShell => {
                let encoded = ps_audio_get();
                self.channel
                    .run_async(&["powershell.exe", "-NoProfile", "-EncodedCommand", &encoded])
                    .await?
            }
        };
        Ok(match tool {
            VolumeTool::Pactl => parse_pactl_volume(&output),
            VolumeTool::Amixer => parse_amixer_volume(&output),
            VolumeTool::Osascript => parse_osascript_volume(&output),
            VolumeTool::Nircmd | VolumeTool::PowerShell => parse_ps_volume(&output),
        })
    }
    /// The foreground application identity, or None when the host could not
    /// say / the name is not a contract identifier.
    async fn foreground_app(&self) -> Result<Option<String>, AdapterError> {
        let output = match self.os()? {
            HostOs::Windows => {
                let encoded = ps_foreground();
                self.channel
                    .run_async(&["powershell.exe", "-NoProfile", "-EncodedCommand", &encoded])
                    .await?
            }
            HostOs::Linux => {
                self.channel
                    .run_async(&["xdotool", "getactivewindow", "getwindowclassname"])
                    .await?
            }
            HostOs::MacOs => {
                self.channel
                    .run_async(&["osascript", "-e", MACOS_FRONTMOST])
                    .await?
            }
        };
        Ok(parse_process_name(&output))
    }
    /// The lock state per OS, or None when the host could not say.
    /// Windows: a LogonUI process means the lock screen. Linux: every listed
    /// session's LockedHint (locked only when all agree; unknown when any
    /// session cannot say). macOS: the frontmost process being loginwindow
    /// means the console sits at the login window — locked or logged out.
    async fn session_locked(&self) -> Result<Option<bool>, AdapterError> {
        match self.os()? {
            HostOs::Windows => {
                let encoded = ps_lock_state();
                let output = self
                    .channel
                    .run_async(&["powershell.exe", "-NoProfile", "-EncodedCommand", &encoded])
                    .await?;
                Ok(parse_ps_bool(&output))
            }
            HostOs::Linux => {
                let list = self
                    .channel
                    .run_async(&["loginctl", "list-sessions", "--no-legend"])
                    .await?;
                let ids = parse_session_ids(&list);
                // One SSH round trip per session: past a handful, the answer would come
                // long after the intent's deadline, so it is not knowable here.
                if ids.is_empty() || ids.len() > MAX_LOCK_SESSIONS {
                    return Ok(None);
                }
                let mut all_locked = true;
                for id in &ids {
                    let output = self
                        .channel
                        .run_async(&["loginctl", "show-session", id, "-p", "LockedHint"])
                        .await?;
                    match parse_locked_hint(&output) {
                        Some(true) => {}
                        Some(false) => all_locked = false,
                        None => return Ok(None),
                    }
                }
                Ok(Some(all_locked))
            }
            HostOs::MacOs => Ok(self
                .foreground_app()
                .await?
                .map(|frontmost| frontmost == "loginwindow")),
        }
    }
    async fn apply_lock(&self, locked: bool) -> Result<(), AdapterError> {
        let argv: Vec<String> = match (self.os()?, locked) {
            (HostOs::Windows, true) => {
                vec!["rundll32.exe".into(), "user32.dll,LockWorkStation".into()]
            }
            // Windows and macOS expose no remote unlock; saying so beats
            // pretending (no simulated input, ever).
            (HostOs::Windows, false) | (HostOs::MacOs, false) => {
                return Err(AdapterError::Unsupported)
            }
            (HostOs::Linux, true) => vec!["loginctl".into(), "lock-session".into()],
            (HostOs::Linux, false) => {
                if self.capabilities.lock != LockSupport::LockUnlock {
                    return Err(AdapterError::Unsupported);
                }
                vec!["loginctl".into(), "unlock-session".into()]
            }
            (HostOs::MacOs, true) => vec![CGSESSION.into(), "-suspend".into()],
        };
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.channel.run_async(&refs).await?;
        Ok(())
    }
    async fn apply_volume(&self, percent: u8) -> Result<(), AdapterError> {
        let tool = self.capabilities.volume.ok_or(AdapterError::Unsupported)?;
        let argv: Vec<String> = match tool {
            VolumeTool::Nircmd => vec![
                "nircmd.exe".into(),
                "setsysvolume".into(),
                nircmd_volume(percent).to_string(),
            ],
            VolumeTool::PowerShell => {
                vec![
                    "powershell.exe".into(),
                    "-NoProfile".into(),
                    "-EncodedCommand".into(),
                    ps_audio_set(percent),
                ]
            }
            VolumeTool::Pactl => vec![
                "pactl".into(),
                "set-sink-volume".into(),
                "@DEFAULT_SINK@".into(),
                format!("{percent}%"),
            ],
            VolumeTool::Amixer => vec![
                "amixer".into(),
                "sset".into(),
                "Master".into(),
                format!("{percent}%"),
            ],
            VolumeTool::Osascript => vec![
                "osascript".into(),
                "-e".into(),
                format!("'set volume output volume {percent}'"),
            ],
        };
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.channel.run_async(&refs).await?;
        Ok(())
    }
    async fn apply_launch(&self, app_id: &str) -> Result<(), AdapterError> {
        let token = self.apps.get(app_id).ok_or(AdapterError::Unsupported)?;
        // Quoting lives here and nowhere else: cmd gets double quotes, POSIX
        // shells single quotes; the token's charset (checked at construction)
        // excludes every character that could escape them.
        let argv: Vec<String> = match self.os()? {
            HostOs::Windows => vec![
                "cmd.exe".into(),
                "/c".into(),
                "start".into(),
                "\"\"".into(),
                format!("\"{token}\""),
            ],
            HostOs::Linux => vec!["systemd-run".into(), "--user".into(), format!("'{token}'")],
            HostOs::MacOs => vec!["open".into(), "-a".into(), format!("'{token}'")],
        };
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.channel.run_async(&refs).await?;
        Ok(())
    }
    /// Power off the host. Outside the Intent pipeline: the contract has no
    /// shutdown command, so this is an adapter-level operation for policy to
    /// wrap. Requires `allow_power` in configuration AND a probed shutdown
    /// tool. The only honest postcondition is that the host stops answering.
    pub async fn shutdown(&self) -> Result<(), AdapterError> {
        self.power(&["-h", "now"], &["/s", "/t", "0"]).await
    }
    /// Reboot the host; same gating and honesty as [`HostSsh::shutdown`].
    pub async fn reboot(&self) -> Result<(), AdapterError> {
        self.power(&["-r", "now"], &["/r", "/t", "0"]).await
    }
    async fn power(&self, posix: &[&str], windows: &[&str]) -> Result<(), AdapterError> {
        if !self.allow_power || !self.capabilities.power {
            return Err(AdapterError::Unsupported);
        }
        let argv: Vec<&str> = match self.os()? {
            HostOs::Windows => {
                let mut v = vec!["shutdown.exe"];
                v.extend_from_slice(windows);
                v
            }
            _ => {
                let mut v = vec!["shutdown"];
                v.extend_from_slice(posix);
                v
            }
        };
        self.channel.run_async(&argv).await?;
        Ok(())
    }
}

#[async_trait]
impl EdgeAdapter for HostSsh {
    fn edge_id(&self) -> &str {
        &self.edge
    }
    fn adapter_id(&self) -> &str {
        &self.adapter
    }
    /// The host's own answers through the probed tools. A field whose tool
    /// was not probed, or whose output did not parse, is None — unknown
    /// verifies nothing. A host that does not answer ssh is `Unavailable`,
    /// not an invented `reachable: false`.
    async fn observe(
        &mut self,
        resource: &ResourceDescriptor,
    ) -> Result<Observation, AdapterError> {
        self.resource_check(resource)?;
        let session_locked = if self.capabilities.lock != LockSupport::None {
            self.session_locked().await?
        } else {
            None
        };
        let volume_percent = match self.capabilities.volume {
            Some(tool) => self.volume_percent(tool).await?,
            None => None,
        };
        let foreground_app = if self.capabilities.foreground {
            self.foreground_app().await?
        } else {
            None
        };
        let state = ResourceState::Host {
            reachable: true, // the host answered ssh
            awake: Some(true),
            session_locked,
            volume_percent,
            foreground_app,
        };
        let now = Timestamp::now();
        self.observations = self.observations.saturating_add(1);
        // The counter keeps evidence identity honest when nothing changed.
        let report = serde_json::to_vec(&(&state, now.0, self.observations))
            .map_err(|_| AdapterError::Protocol)?;
        Ok(Observation {
            resource_id: resource.resource_id.clone(),
            adapter_id: self.adapter.clone(),
            evidence_id: format!("host-{:x}", Sha256::digest(report)),
            observed_at: now,
            origin: ObservationOrigin::Measurement,
            state,
        })
    }
    async fn apply(
        &mut self,
        resource: &ResourceDescriptor,
        command: &Command,
    ) -> Result<AdapterAck, AdapterError> {
        command.validate().map_err(|_| AdapterError::Protocol)?;
        self.resource_check(resource)?;
        if !resource.capabilities.contains(&command.capability()) {
            return Err(AdapterError::Unsupported);
        }
        match command {
            Command::SessionLock { locked } => {
                if self.capabilities.lock == LockSupport::None {
                    return Err(AdapterError::Unsupported);
                }
                self.apply_lock(*locked).await?;
                Ok(AdapterAck { accepted: true })
            }
            Command::VolumeSet { percent } => {
                self.apply_volume(*percent).await?;
                Ok(AdapterAck { accepted: true })
            }
            Command::AppLaunch { app_id } => {
                if !self.capabilities.app_launch {
                    return Err(AdapterError::Unsupported);
                }
                self.apply_launch(app_id).await?;
                Ok(AdapterAck { accepted: true })
            }
            _ => Err(AdapterError::Unsupported),
        }
    }
}
