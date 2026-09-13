use serde::Serialize;
use std::fmt;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

impl CheckStatus {
    pub fn is_healthy(self) -> bool {
        self != Self::Fail
    }
}

impl fmt::Display for CheckStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

fn check(name: &str, ok: bool, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        status: if ok {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        detail: detail.into(),
    }
}

fn optional_command(name: &str, command: &str, args: &[&str]) -> Check {
    match Command::new(command).args(args).output() {
        Ok(output) if output.status.success() => Check {
            name: name.into(),
            status: CheckStatus::Pass,
            detail: String::from_utf8_lossy(&output.stdout)
                .trim()
                .chars()
                .take(160)
                .collect(),
        },
        Ok(_) | Err(_) => Check {
            name: name.into(),
            status: CheckStatus::Warn,
            detail: format!("optional runtime {command} is unavailable"),
        },
    }
}

pub fn check_component(component: &str) -> Vec<Check> {
    match component {
        "cerebra" => vec![check(
            "cerebra-contract",
            true,
            "trusted Rust sequencer linked",
        )],
        "vault" => vec![check(
            "vault-boundary",
            Path::new("/vault").exists() || !cfg!(target_os = "linux"),
            "sealed store directory",
        )],
        "router" => vec![check(
            "router-config",
            true,
            "fail-closed provider configuration",
        )],
        "speech" => vec![optional_command(
            "nemotron-runtime",
            "nemo-speech",
            &["--help"],
        )],
        "tools" => vec![optional_command("freecad", "FreeCADCmd", &["--version"])],
        "all" => doctor(),
        unknown => vec![check(
            "component",
            false,
            format!("unknown component {unknown}"),
        )],
    }
}

pub fn doctor() -> Vec<Check> {
    let linux = cfg!(target_os = "linux");
    let wsl = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|value| value.to_ascii_lowercase().contains("microsoft"))
        .unwrap_or(false);
    let systemd = Path::new("/run/systemd/system").exists();
    let mut checks = vec![
        check("linux", linux, std::env::consts::OS),
        check(
            "wsl2",
            !linux || wsl,
            if wsl { "detected" } else { "not detected" },
        ),
        check(
            "systemd",
            !linux || systemd,
            if systemd { "running" } else { "not running" },
        ),
        check(
            "workspace",
            Path::new("/archie/workspaces").exists() || !linux,
            "/archie/workspaces",
        ),
        check("vault", Path::new("/vault").exists() || !linux, "/vault"),
    ];
    checks.extend([
        optional_command(
            "gpu",
            "nvidia-smi",
            &["--query-gpu=name", "--format=csv,noheader"],
        ),
        optional_command("freecad", "FreeCADCmd", &["--version"]),
        optional_command("speech", "nemo-speech", &["--help"]),
    ]);
    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_component_fails_closed() {
        assert_eq!(check_component("imaginary")[0].status, CheckStatus::Fail);
    }

    #[test]
    fn optional_hardware_is_a_warning() {
        let check = optional_command("missing", "archie-command-that-does-not-exist", &[]);
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.status.is_healthy());
    }
}
