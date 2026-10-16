//! `archie-rem` — the REM supervisor.
//!
//! ```text
//! archie-rem status [--config FILE]   what the machine is doing, and whether REM would start
//! archie-rem once   [--config FILE]   finish any cycle cut short, then run one if it may start
//! archie-rem watch  [--config FILE]   the service: the above, every minute
//! ```
//!
//! The configuration is `/etc/archie/rem.toml`. Reports go to standard output as one JSON
//! object per line.

use archie_rem::cycle::{Cycle, Plan, Report, Running, TrainerEnd, World};
use archie_rem::gate::{minute_of, Conditions, Gate, NightWindow};
use archie_rem::Activity;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_CONFIG: &str = "/etc/archie/rem.toml";

/// `rem.toml`. Every field has a default, so an empty file is a supervisor that pauses
/// and resumes jobs at night and trains nothing.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    night_start: String,
    night_end: String,
    /// Minutes the machine must have been untouched.
    idle_minutes: u64,
    jobs_dir: PathBuf,
    state_dir: PathBuf,
    /// Under /run, so the opened windows and the activity file never reach a disk.
    run_dir: PathBuf,
    /// A file whose modification time is the last time a person touched the machine. The
    /// Stage rewrites it on a person's input. While nothing writes it, REM never starts.
    presence_file: PathBuf,
    /// Exists while something is waiting for IRIS to answer.
    inference_flag: PathBuf,
    trainer: Vec<String>,
    grace_seconds: u64,
    resume_wait_seconds: u64,
    retention_hours: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            night_start: "02:00".into(),
            night_end: "06:00".into(),
            idle_minutes: 30,
            jobs_dir: "/etc/archie/rem/jobs".into(),
            state_dir: "/var/lib/archie/rem".into(),
            run_dir: "/run/archie/rem".into(),
            presence_file: "/run/archie/person-input".into(),
            inference_flag: "/run/archie/iris-pending".into(),
            trainer: Vec::new(),
            grace_seconds: 60,
            resume_wait_seconds: 300,
            retention_hours: 26,
        }
    }
}

impl Config {
    fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn plan(&self) -> Result<Plan, String> {
        let window = NightWindow::parse(&self.night_start, &self.night_end)
            .ok_or("night_start and night_end are times of day, such as \"02:00\"")?;
        Ok(Plan {
            gate: Gate {
                window,
                idle_seconds: self.idle_minutes * 60,
            },
            jobs_dir: self.jobs_dir.clone(),
            resume_list: self.state_dir.join("resume.json"),
            marker: self.state_dir.join("cycle"),
            buffer_dir: self.state_dir.join("day"),
            work_dir: self.run_dir.join("work"),
            activity_file: self.run_dir.join("activity.json"),
            trainer: self.trainer.clone(),
            grace_seconds: self.grace_seconds,
            resume_wait_seconds: self.resume_wait_seconds,
            retention_seconds: self.retention_hours * 3600,
        })
    }
}

/// The machine itself.
struct Machine {
    presence_file: PathBuf,
    inference_flag: PathBuf,
    /// The last reading of /proc/stat: (busy, total) jiffies.
    cpu_before: Option<(u64, u64)>,
}

impl Machine {
    /// Seconds since a person's input, from the presence file's age. Unknown is zero.
    fn idle_seconds(&self) -> u64 {
        std::fs::metadata(&self.presence_file)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|touched| SystemTime::now().duration_since(touched).ok())
            .map_or(0, |age| age.as_secs())
    }

    /// Percent of CPU time spent busy since the last call.
    fn cpu_percent(&mut self) -> u8 {
        let Some(now) = cpu_jiffies() else { return 0 };
        let before = self.cpu_before.replace(now);
        match before {
            Some((busy, total)) if now.1 > total => {
                (100 * now.0.saturating_sub(busy) / (now.1 - total)).min(100) as u8
            }
            _ => 0,
        }
    }
}

/// (busy, total) from the first line of /proc/stat.
fn cpu_jiffies() -> Option<(u64, u64)> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let fields: Vec<u64> = stat
        .lines()
        .next()?
        .split_whitespace()
        .skip(1)
        .filter_map(|f| f.parse().ok())
        .collect();
    let total: u64 = fields.iter().sum();
    // idle and iowait are the fourth and fifth fields.
    let idle = fields.get(3)? + fields.get(4).copied().unwrap_or(0);
    Some((total - idle, total))
}

fn ram_used_bytes() -> u64 {
    let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") else {
        return 0;
    };
    let kilobytes = |name: &str| {
        meminfo
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
    };
    kilobytes("MemTotal:").saturating_sub(kilobytes("MemAvailable:")) * 1024
}

/// (percent busy, bytes of memory used) of the first NVIDIA GPU; zeros without one.
fn gpu() -> (u8, u64) {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=utilization.gpu,memory.used",
            "--format=csv,noheader,nounits",
        ])
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else { return (0, 0) };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut fields = text
        .lines()
        .next()
        .unwrap_or("")
        .split(',')
        .map(|f| f.trim().parse::<u64>().unwrap_or(0));
    let percent = fields.next().unwrap_or(0).min(100) as u8;
    (percent, fields.next().unwrap_or(0) * 1024 * 1024)
}

/// Whether the machine runs from the wall. A machine with no battery does.
fn on_mains() -> bool {
    let Ok(supplies) = std::fs::read_dir("/sys/class/power_supply") else {
        return true;
    };
    let mut battery = false;
    for supply in supplies.flatten() {
        let read =
            |name: &str| std::fs::read_to_string(supply.path().join(name)).unwrap_or_default();
        match read("type").trim() {
            "Mains" | "USB" if read("online").trim() == "1" => return true,
            "Battery" => battery = true,
            _ => {}
        }
    }
    !battery
}

/// Minutes after local midnight, from `date`, which knows the machine's time zone.
fn local_minute() -> Option<u16> {
    let output = Command::new("date").arg("+%H:%M").output().ok()?;
    minute_of(String::from_utf8_lossy(&output.stdout).trim())
}

struct Process(Child);

impl Running for Process {
    fn finished(&mut self) -> Option<i32> {
        match self.0.try_wait() {
            Ok(Some(status)) => Some(status.code().unwrap_or(-1)),
            Ok(None) => None,
            Err(_) => Some(-1),
        }
    }

    fn kill(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn command(words: &[String]) -> Result<Command, String> {
    let (program, arguments) = words.split_first().ok_or("an empty command")?;
    let mut command = Command::new(program);
    command.args(arguments).stdin(Stdio::null());
    Ok(command)
}

impl World for Machine {
    fn conditions(&mut self) -> Conditions {
        Conditions {
            // An unreadable clock is noon: outside any night.
            local_minute: local_minute().unwrap_or(12 * 60),
            on_mains: on_mains(),
            idle_seconds: self.idle_seconds(),
        }
    }

    fn activity(&mut self, idle_seconds: u64) -> Activity {
        let (gpu_percent, vram_bytes) = gpu();
        Activity {
            updated_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            cpu_percent: self.cpu_percent(),
            ram_bytes: ram_used_bytes(),
            vram_bytes,
            gpu_percent,
            idle_seconds,
            inference_pending: self.inference_flag.exists(),
        }
    }

    fn run(&mut self, words: &[String]) -> Result<i32, String> {
        let status = command(words)?
            .status()
            .map_err(|e| format!("{}: {e}", words[0]))?;
        Ok(status.code().unwrap_or(-1))
    }

    fn start(&mut self, words: &[String]) -> Result<Box<dyn Running>, String> {
        let child = command(words)?
            .spawn()
            .map_err(|e| format!("{}: {e}", words[0]))?;
        Ok(Box::new(Process(child)))
    }

    fn now(&mut self) -> SystemTime {
        SystemTime::now()
    }

    fn wait(&mut self, seconds: u64) {
        std::thread::sleep(Duration::from_secs(seconds));
    }
}

fn print(event: &str, report: &Report) {
    let trainer = match &report.trainer {
        TrainerEnd::Skipped(why) => serde_json::json!({"skipped": why}),
        TrainerEnd::Finished { exit_code } => serde_json::json!({"finished": exit_code}),
        TrainerEnd::Interrupted => serde_json::json!("interrupted"),
        TrainerEnd::Killed => serde_json::json!("killed"),
        TrainerEnd::Failed(why) => serde_json::json!({"failed": why}),
    };
    println!(
        "{}",
        serde_json::json!({
            "event": event, "paused": report.paused, "not_paused": report.not_paused, "windows": report.windows,
            "trainer": trainer, "windows_deleted": report.windows_deleted, "resumed": report.resumed,
            "stuck": report.stuck, "errors": report.errors,
        })
    );
}

/// Finishes what was cut short, then runs a cycle if one may start. Whether the node is
/// left in order: no stuck job and nothing undeleted.
fn attempt(plan: &Plan, machine: &mut Machine, say_waiting: bool) -> bool {
    let mut in_order = true;
    if let Some(report) = Cycle::new(plan, machine).recover() {
        in_order &= report.stuck.is_empty() && report.windows_deleted;
        print("rem_recovered", &report);
    }
    match Cycle::new(plan, machine).run() {
        Ok(report) => {
            in_order &= report.stuck.is_empty() && report.windows_deleted;
            print("rem_cycle", &report);
        }
        Err(wait) if say_waiting => println!(
            "{}",
            serde_json::json!({"event": "rem_waiting", "why": wait.to_string()})
        ),
        Err(_) => {}
    }
    in_order
}

fn main() -> ExitCode {
    let mut arguments: Vec<String> = std::env::args().skip(1).collect();
    let config_path = match arguments.iter().position(|a| a == "--config") {
        Some(at) if at + 1 < arguments.len() => {
            arguments.remove(at);
            PathBuf::from(arguments.remove(at))
        }
        _ => PathBuf::from(DEFAULT_CONFIG),
    };
    let prepared =
        Config::load(&config_path).and_then(|config| config.plan().map(|plan| (config, plan)));
    let (config, plan) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            eprintln!("archie-rem: {error}");
            return ExitCode::from(2);
        }
    };
    let mut machine = Machine {
        presence_file: config.presence_file.clone(),
        inference_flag: config.inference_flag.clone(),
        cpu_before: None,
    };
    match arguments.first().map(String::as_str) {
        Some("status") => {
            let now = machine.conditions();
            let verdict = plan.gate.may_start(&now);
            println!(
                "{}",
                serde_json::json!({
                    "local_minute": now.local_minute, "on_mains": now.on_mains, "idle_seconds": now.idle_seconds,
                    "would_start": verdict.is_ok(), "why_not": verdict.err().map(|w| w.to_string()),
                    "cycle_cut_short": plan.marker.exists(), "trainer_configured": !plan.trainer.is_empty(),
                })
            );
            ExitCode::SUCCESS
        }
        Some("once") => {
            if attempt(&plan, &mut machine, true) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Some("watch") => loop {
            attempt(&plan, &mut machine, false);
            std::thread::sleep(Duration::from_secs(60));
        },
        _ => {
            eprintln!("usage: archie-rem status|once|watch [--config FILE]");
            ExitCode::from(2)
        }
    }
}
