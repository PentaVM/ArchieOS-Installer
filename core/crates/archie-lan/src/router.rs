//! Sentry's view through the owner's router: its DHCP leases and its neighbour (ARP)
//! table, read over SSH with a key that can do nothing else.
//!
//! The router is the one device that sees every other: a switch hides the rest of the
//! network from the operator's machine, but every device asks the router for an address
//! and talks through it. Enrolment is done once, by the owner:
//!
//! 1. `detect` looks at the default gateway only (its SSH banner and its web page) to say
//!    what it runs: OpenWrt, Asuswrt, a FreeBSD firewall (pfSense, OPNsense) or another
//!    Linux.
//! 2. `enroll` makes an ed25519 key for Sentry and the one `authorized_keys` line the
//!    owner adds on the router. The line pins the key to a fixed read-only command
//!    (`command="…"`) with no port forwarding, agent, X11 or terminal: whoever holds the
//!    key can read the lease and neighbour tables and nothing more. With `--install`
//!    on OpenWrt or Linux, Sentry adds the line itself over one interactive `ssh` session
//!    in which the owner types the router's password; the password never passes through
//!    Sentry and is not kept.
//! 3. `pull` (and `live`, every few minutes, once enrolled) runs that command and feeds
//!    the leases and neighbours to the audit. The router's host key is pinned at
//!    enrolment (`router_known_hosts`).
//!
//! Nothing here changes the router. Changes (blocking a device, SQM) are proposals that
//! go through Policy and the owner, as `docs/SENTRY-ROADMAP.md` says.

use crate::leases::Lease;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Between the lease table and the neighbour table in the command's output.
pub const SEPARATOR: &str = "@@ARCHIE-NEIGHBOURS@@";
/// The most of a router's answer that is read.
pub const MAX_OUTPUT: usize = 1024 * 1024;
/// How long one read of the router may take, connection included.
pub const PULL_TIMEOUT: Duration = Duration::from_secs(20);
/// The files kept in the state directory.
pub const CONFIG: &str = "router.json";
pub const KEY: &str = "router_ed25519";
pub const KNOWN_HOSTS: &str = "router_known_hosts";

/// What the router runs, as far as the lease and neighbour tables go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// dnsmasq at `/tmp/dhcp.leases`, Dropbear's `/etc/dropbear/authorized_keys`.
    Openwrt,
    /// dnsmasq at `/var/lib/misc/dnsmasq.leases`; keys go in through the web interface.
    Asuswrt,
    /// pfSense or OPNsense: ISC dhcpd (or OPNsense's dnsmasq), BSD `arp`.
    BsdFirewall,
    /// Any other Linux router running dnsmasq.
    Linux,
}

impl Kind {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "openwrt" => Some(Self::Openwrt),
            "asuswrt" => Some(Self::Asuswrt),
            "bsd-firewall" | "pfsense" | "opnsense" => Some(Self::BsdFirewall),
            "linux" => Some(Self::Linux),
            _ => None,
        }
    }

    /// The one command the key may run: print the lease table, the separator, then the
    /// neighbour table. Read-only, on every kind.
    pub fn read_command(self) -> String {
        let (leases, neighbours) = match self {
            Self::Openwrt => ("cat /tmp/dhcp.leases", "cat /proc/net/arp"),
            Self::Asuswrt => ("cat /var/lib/misc/dnsmasq.leases", "cat /proc/net/arp"),
            Self::BsdFirewall => (
                "cat /var/dhcpd/var/db/dhcpd.leases 2>/dev/null || cat /var/db/dnsmasq.leases",
                "arp -an",
            ),
            Self::Linux => (
                "cat /var/lib/misc/dnsmasq.leases 2>/dev/null || cat /tmp/dhcp.leases",
                "cat /proc/net/arp",
            ),
        };
        format!("{leases}; echo {SEPARATOR}; {neighbours}")
    }

    /// Where the owner puts the key line, said plainly.
    pub fn where_the_key_goes(self) -> &'static str {
        match self {
            Self::Openwrt => {
                "OpenWrt: append it to /etc/dropbear/authorized_keys (LuCI: System → \
                 Administration → SSH-Keys), or run `enroll --install`"
            }
            Self::Asuswrt => {
                "Asuswrt: Administration → System → Enable SSH, and paste it into \
                 Authorized Keys (the router rewrites its files from that setting)"
            }
            Self::BsdFirewall => {
                "pfSense / OPNsense: System → Access → Users → (an admin user) → \
                 Authorized keys, then enable SSH under System → Settings → Administration \
                 (the firewall rewrites ~/.ssh from its configuration)"
            }
            Self::Linux => "Linux: append it to ~/.ssh/authorized_keys of the router's admin user, or run `enroll --install`",
        }
    }

    /// Whether `enroll --install` can add the line over SSH and expect it to stay.
    pub fn installs_over_ssh(self) -> bool {
        matches!(self, Self::Openwrt | Self::Linux)
    }

    fn authorized_keys_path(self) -> &'static str {
        match self {
            Self::Openwrt => "/etc/dropbear/authorized_keys",
            _ => "$HOME/.ssh/authorized_keys",
        }
    }
}

/// What the gateway shows of itself, and what that says it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Detected {
    pub address: IpAddr,
    pub ssh_banner: Option<String>,
    /// The marker found in its web page or headers, if one was.
    pub web: Option<String>,
    pub kind: Option<Kind>,
}

/// What a banner and a web page say the router runs.
pub fn classify(ssh_banner: Option<&str>, web: Option<&str>) -> (Option<String>, Option<Kind>) {
    let page = web.unwrap_or("").to_ascii_lowercase();
    for (marker, kind) in [
        ("luci", Kind::Openwrt),
        ("openwrt", Kind::Openwrt),
        ("opnsense", Kind::BsdFirewall),
        ("pfsense", Kind::BsdFirewall),
        ("asuswrt", Kind::Asuswrt),
        ("asus wireless router", Kind::Asuswrt),
    ] {
        if page.contains(marker) {
            return (Some(marker.to_string()), Some(kind));
        }
    }
    let banner = ssh_banner.unwrap_or("").to_ascii_lowercase();
    let kind = if banner.contains("freebsd") {
        Some(Kind::BsdFirewall)
    } else if banner.contains("dropbear") {
        Some(Kind::Openwrt)
    } else if banner.starts_with("ssh-") {
        Some(Kind::Linux)
    } else {
        None
    };
    (None, kind)
}

/// Looks at the gateway, and only the gateway: its SSH banner on port 22 and its web
/// page on port 80. Neither logs in.
pub fn detect(address: IpAddr) -> Detected {
    let ssh_banner = banner(address);
    let web = crate::http::LanUrl::parse(&format!("http://{address}/"))
        .and_then(|url| crate::http::get(&url).ok())
        .map(|raw| String::from_utf8_lossy(&raw[..raw.len().min(64 * 1024)]).into_owned());
    let (marker, kind) = classify(ssh_banner.as_deref(), web.as_deref());
    Detected {
        address,
        ssh_banner,
        web: marker,
        kind,
    }
}

fn banner(address: IpAddr) -> Option<String> {
    let mut stream =
        TcpStream::connect_timeout(&SocketAddr::new(address, 22), Duration::from_secs(2)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut buffer = [0u8; 255];
    let read = stream.read(&mut buffer).ok()?;
    let line = String::from_utf8_lossy(&buffer[..read]);
    let line = line.lines().next()?.trim();
    (line.starts_with("SSH-") && line.chars().all(|c| !c.is_control())).then(|| line.to_string())
}

/// How Sentry reaches the router, kept in the state directory as `router.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub address: IpAddr,
    #[serde(default = "ssh_port")]
    pub port: u16,
    pub user: String,
    pub kind: Kind,
}

fn ssh_port() -> u16 {
    22
}

/// The `authorized_keys` line for Sentry's key: it may run the read command only.
pub fn authorized_line(kind: Kind, public_key: &str) -> String {
    format!(
        "command=\"{}\",no-port-forwarding,no-agent-forwarding,no-X11-forwarding,no-pty {}",
        kind.read_command(),
        public_key.trim()
    )
}

/// The `ssh` arguments that read the router: the key only, no agent and no user
/// configuration, never a password prompt, the host key pinned at enrolment.
pub fn read_arguments(config: &Config, directory: &Path) -> Vec<String> {
    // The enrolled key runs its pinned command whatever is asked; asking for the same
    // command keeps the two in step.
    command_arguments(config, directory, &config.kind.read_command())
}

fn command_arguments(config: &Config, directory: &Path, command: &str) -> Vec<String> {
    let mut arguments = common_arguments(config, directory, true);
    arguments.push(format!("{}@{}", config.user, config.address));
    arguments.push(command.to_string());
    arguments
}

/// The `ssh` arguments that add the key line once, in an interactive session where the
/// owner types the router's password. The line comes on standard input.
pub fn install_arguments(config: &Config, directory: &Path) -> Vec<String> {
    let mut arguments = common_arguments(config, directory, false);
    arguments.push(format!("{}@{}", config.user, config.address));
    let path = config.kind.authorized_keys_path();
    arguments.push(format!(
        "umask 077; read -r line; f=\"{path}\"; mkdir -p \"$(dirname \"$f\")\"; \
         grep -qxF \"$line\" \"$f\" 2>/dev/null || echo \"$line\" >> \"$f\""
    ));
    arguments
}

fn common_arguments(config: &Config, directory: &Path, batch: bool) -> Vec<String> {
    let mut arguments: Vec<String> = vec![
        "-F".into(),
        "/dev/null".into(),
        "-p".into(),
        config.port.to_string(),
        "-o".into(),
        format!(
            "UserKnownHostsFile={}",
            directory.join(KNOWN_HOSTS).display()
        ),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        "ConnectTimeout=5".into(),
        "-o".into(),
        "ForwardAgent=no".into(),
        "-T".into(),
    ];
    if batch {
        arguments.extend([
            "-i".into(),
            directory.join(KEY).display().to_string(),
            "-o".into(),
            "IdentitiesOnly=yes".into(),
            "-o".into(),
            "IdentityAgent=none".into(),
            "-o".into(),
            "BatchMode=yes".into(),
        ]);
    }
    arguments
}

/// What one read of the router gave.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tables {
    pub leases: Vec<Lease>,
    pub neighbours: Vec<(String, Ipv4Addr)>,
}

/// Takes the command's output apart.
pub fn parse(output: &str, now_s: u64) -> Tables {
    let (leases, neighbours) = output.split_once(SEPARATOR).unwrap_or((output, ""));
    Tables {
        leases: crate::leases::parse(leases, now_s),
        neighbours: neighbours_of(neighbours),
    }
}

/// The complete entries of a neighbour table: Linux's `/proc/net/arp`, or BSD's
/// `arp -an`.
pub fn neighbours_of(text: &str) -> Vec<(String, Ipv4Addr)> {
    text.lines()
        .take(crate::leases::MAX_LEASES)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            match fields.as_slice() {
                // 192.168.1.5  0x1  0x2  aa:bb:cc:dd:ee:ff  *  br-lan
                [address, _, flags, mac, ..] if flags.starts_with("0x") => {
                    let complete = u32::from_str_radix(flags.trim_start_matches("0x"), 16)
                        .is_ok_and(|flags| flags & 0x2 != 0);
                    complete
                        .then(|| Some((crate::capture::mac(mac)?, address.parse().ok()?)))
                        .flatten()
                }
                // ? (192.168.1.5) at aa:bb:cc:dd:ee:ff on igb1 expires in 1199 seconds [ethernet]
                [_, address, "at", mac, ..] => Some((
                    crate::capture::mac(mac)?,
                    address
                        .trim_start_matches('(')
                        .trim_end_matches(')')
                        .parse()
                        .ok()?,
                )),
                _ => None,
            }
        })
        .collect()
}

/// Runs `ssh` with these arguments, kills it after `timeout`, and returns what it
/// printed (at most `MAX_OUTPUT`).
pub fn run_ssh(arguments: &[String], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new("ssh")
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!("ssh could not be started ({e}); it comes with the openssh package")
        })?;
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = (&mut stdout)
            .take(MAX_OUTPUT as u64)
            .read_to_end(&mut output);
        output
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "the router did not answer within {} s",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let output = reader.join().unwrap_or_default();
    if !status.success() {
        let mut error = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = (&mut stderr).take(4096).read_to_string(&mut error);
        }
        return Err(format!(
            "ssh to the router failed ({status}): {}",
            error.lines().last().unwrap_or("").trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

/// Reads the router once with the enrolled key.
pub fn pull(config: &Config, directory: &Path, now_s: u64) -> Result<Tables, String> {
    let output = run_ssh(&read_arguments(config, directory), PULL_TIMEOUT)?;
    Ok(parse(&output, now_s))
}

/// Makes Sentry's key if there is none yet, and returns its public half.
pub fn ensure_key(directory: &Path) -> Result<String, String> {
    let key: PathBuf = directory.join(KEY);
    if !key.exists() {
        let status = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "archie-sentry", "-f"])
            .arg(&key)
            .stdin(Stdio::null())
            .status()
            .map_err(|e| {
                format!("ssh-keygen could not be started ({e}); it comes with the openssh package")
            })?;
        if !status.success() {
            return Err(format!("ssh-keygen ended with {status}"));
        }
    }
    let public = std::fs::read_to_string(key.with_extension("pub"))
        .map_err(|e| format!("{}.pub: {e}", key.display()))?;
    let public = public.trim();
    let sane = public.starts_with("ssh-ed25519 ")
        && public.len() < 512
        && public.chars().all(|c| c.is_ascii_graphic() || c == ' ');
    if sane {
        Ok(public.to_string())
    } else {
        Err(format!(
            "{}.pub is not an ed25519 public key",
            key.display()
        ))
    }
}

/// Adds the key line on the router in an interactive session: `ssh` asks the owner for the
/// router's password on the terminal; Sentry never sees it.
pub fn install(config: &Config, directory: &Path, line: &str) -> Result<(), String> {
    let mut child = Command::new("ssh")
        .args(install_arguments(config, directory))
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ssh could not be started ({e})"))?;
    if let Some(mut stdin) = child.stdin.take() {
        writeln!(stdin, "{line}").map_err(|e| e.to_string())?;
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("adding the key on the router failed ({status})"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_router_is_told_by_its_page_then_its_banner() {
        let (marker, kind) = classify(
            Some("SSH-2.0-dropbear_2022.83"),
            Some("<title>OpenWrt - LuCI</title>"),
        );
        assert_eq!(
            (marker.as_deref(), kind),
            (Some("luci"), Some(Kind::Openwrt))
        );
        let pf = classify(
            Some("SSH-2.0-OpenSSH_9.7 FreeBSD-20240318"),
            Some("HTTP/1.1 301"),
        );
        assert_eq!(pf, (None, Some(Kind::BsdFirewall)));
        let opn = classify(None, Some("<title>Login | OPNsense</title>"));
        assert_eq!(opn.1, Some(Kind::BsdFirewall));
        assert_eq!(classify(None, Some("ASUSWRT")).1, Some(Kind::Asuswrt));
        assert_eq!(
            classify(Some("SSH-2.0-OpenSSH_9.6"), None).1,
            Some(Kind::Linux)
        );
        assert_eq!(classify(None, None), (None, None));
    }

    #[test]
    fn the_key_line_pins_a_read_only_command() {
        let line = authorized_line(Kind::Openwrt, "ssh-ed25519 AAAAC3Nza archie-sentry\n");
        assert!(line.starts_with(
            "command=\"cat /tmp/dhcp.leases; echo @@ARCHIE-NEIGHBOURS@@; cat /proc/net/arp\","
        ));
        assert!(line.contains("no-port-forwarding,no-agent-forwarding,no-X11-forwarding,no-pty "));
        assert!(line.ends_with("ssh-ed25519 AAAAC3Nza archie-sentry"));
        for kind in [Kind::Openwrt, Kind::Asuswrt, Kind::BsdFirewall, Kind::Linux] {
            let command = kind.read_command();
            assert!(!command.contains('"'), "{command}");
            for word in ["rm ", "> ", "uci", "reboot", "kill"] {
                assert!(
                    !command.contains(word),
                    "{command} writes or changes something"
                );
            }
        }
    }

    #[test]
    fn reading_never_prompts_and_pins_the_host() {
        let config = Config {
            address: "192.168.1.1".parse().unwrap(),
            port: 22,
            user: "root".into(),
            kind: Kind::Openwrt,
        };
        let arguments = read_arguments(&config, Path::new("/var/lib/archie/netwatch"));
        let joined = arguments.join(" ");
        for wanted in [
            "-F /dev/null",
            "BatchMode=yes",
            "IdentitiesOnly=yes",
            "IdentityAgent=none",
            "StrictHostKeyChecking=accept-new",
            "UserKnownHostsFile=/var/lib/archie/netwatch/router_known_hosts",
            "-i /var/lib/archie/netwatch/router_ed25519",
            "root@192.168.1.1",
        ] {
            assert!(joined.contains(wanted), "{wanted} missing from {joined}");
        }
        let install = install_arguments(&config, Path::new("/s")).join(" ");
        assert!(
            !install.contains("BatchMode"),
            "the owner types the password once"
        );
        assert!(install.contains("/etc/dropbear/authorized_keys"));
    }

    #[test]
    fn the_neighbour_tables_of_linux_and_bsd_are_read() {
        let linux = "\
IP address       HW type     Flags       HW address            Mask     Device
192.168.1.84     0x1         0x2         54:4e:f0:07:3d:e8     *        br-lan
192.168.1.90     0x1         0x0         00:00:00:00:00:00     *        br-lan
192.168.1.61     0x1         0x2         00:12:12:00:00:02     *        br-lan
";
        let read = neighbours_of(linux);
        assert_eq!(
            read,
            [
                ("54:4e:f0:07:3d:e8".into(), "192.168.1.84".parse().unwrap()),
                ("00:12:12:00:00:02".into(), "192.168.1.61".parse().unwrap()),
            ]
        );
        let bsd = "\
? (192.168.1.10) at 00:11:22:33:44:55 on igb1 expires in 1180 seconds [ethernet]
? (192.168.1.11) at (incomplete) on igb1 expired [ethernet]
";
        assert_eq!(
            neighbours_of(bsd),
            [("00:11:22:33:44:55".into(), "192.168.1.10".parse().unwrap())]
        );
        let whole = format!("1791750000 54:4e:f0:07:3d:e8 192.168.1.84 tv *\n{SEPARATOR}\n{linux}");
        let read = parse(&whole, 0);
        assert_eq!((read.leases.len(), read.neighbours.len()), (1, 2));
    }
}
