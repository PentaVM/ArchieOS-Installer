//! One night's cycle: pause, train, delete, resume.
//!
//! The order is fixed and two steps are unconditional. Whatever happens between pausing
//! and the end — no windows, a trainer that fails, a person who comes back, an error
//! reading a file — the day's windows are deleted and the paused jobs are resumed. A
//! marker on disk says a cycle is under way, so a supervisor that was killed in the
//! middle does both at its next start instead of leaving a GPU idle and a day's windows
//! on disk.

use crate::activity::Activity;
use crate::buffer::{shred_dir, DayBuffer};
use crate::gate::{Conditions, Gate, Wait};
use crate::jobs::{load_jobs, Job, ResumeList};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Where things are and how long to wait: the supervisor's configuration.
#[derive(Debug, Clone)]
pub struct Plan {
    pub gate: Gate,
    /// The files that register long jobs.
    pub jobs_dir: PathBuf,
    pub resume_list: PathBuf,
    /// Exists while a cycle is under way.
    pub marker: PathBuf,
    /// The day's sealed windows.
    pub buffer_dir: PathBuf,
    /// Where the windows are opened for the trainer, for the length of the cycle only.
    /// Meant to be memory-backed (under /run).
    pub work_dir: PathBuf,
    pub activity_file: PathBuf,
    /// The trainer, as a program and its arguments. `{work}` and `{activity}` in an
    /// argument become the work directory and the activity file. Empty trains nothing.
    pub trainer: Vec<String>,
    /// How long a trainer may take to leave after it was told a person is back.
    pub grace_seconds: u64,
    /// How long a resumed job has to show progress before it is reported as stuck.
    pub resume_wait_seconds: u64,
    /// Windows older than this are deleted even if no cycle ran: a week of nights on
    /// battery must not become a week of retention.
    pub retention_seconds: u64,
}

/// A started trainer.
pub trait Running {
    /// Its exit code once it has ended.
    fn finished(&mut self) -> Option<i32>;
    fn kill(&mut self);
}

/// The machine, as the cycle needs it. The real one reads sensors and runs programs; the
/// tests' one is scripted.
pub trait World {
    fn conditions(&mut self) -> Conditions;
    /// The readings for the activity file, with the idle time the conditions gave.
    fn activity(&mut self, idle_seconds: u64) -> Activity;
    /// Runs a program to its end and returns its exit code.
    fn run(&mut self, command: &[String]) -> Result<i32, String>;
    fn start(&mut self, command: &[String]) -> Result<Box<dyn Running>, String>;
    fn now(&mut self) -> SystemTime;
    fn wait(&mut self, seconds: u64);
}

/// How the training step ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrainerEnd {
    /// There was nothing to learn from, or no trainer is configured.
    Skipped(String),
    Finished {
        exit_code: i32,
    },
    /// A person came back (or the power went) and the trainer left by itself.
    Interrupted,
    /// It had not left after the grace period.
    Killed,
    Failed(String),
}

/// What one cycle did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub paused: Vec<String>,
    /// Jobs whose checkpoint command failed. They stay on the resume list.
    pub not_paused: Vec<(String, String)>,
    pub windows: usize,
    pub trainer: TrainerEnd,
    pub windows_deleted: bool,
    pub resumed: Vec<String>,
    /// Jobs that were told to resume and showed no progress in time.
    pub stuck: Vec<String>,
    pub errors: Vec<String>,
}

pub struct Cycle<'a, W: World> {
    plan: &'a Plan,
    world: &'a mut W,
}

impl<'a, W: World> Cycle<'a, W> {
    pub fn new(plan: &'a Plan, world: &'a mut W) -> Self {
        Self { plan, world }
    }

    /// What to do at start-up, before anything else: finish a cycle that was cut short,
    /// and delete windows that outlived their day. `None` when there was nothing to do.
    pub fn recover(&mut self) -> Option<Report> {
        if self.plan.marker.exists() {
            let mut report = empty_report(TrainerEnd::Skipped("a cycle was cut short".into()));
            self.finish(&mut report);
            return Some(report);
        }
        let outlived = DayBuffer::started(&self.plan.buffer_dir)
            .and_then(|made| self.world.now().duration_since(made).ok())
            .is_some_and(|age| age.as_secs() > self.plan.retention_seconds);
        if outlived {
            let mut report = empty_report(TrainerEnd::Skipped(
                "the day's windows outlived their retention".into(),
            ));
            self.delete(&mut report);
            return Some(report);
        }
        None
    }

    /// One cycle, if the conditions allow it now.
    pub fn run(&mut self) -> Result<Report, Wait> {
        let entered = self.world.conditions();
        self.plan.gate.may_start(&entered)?;
        let mut report = empty_report(TrainerEnd::Skipped("not reached".into()));
        self.pause(&mut report);
        report.trainer = self.train(entered.idle_seconds, &mut report);
        self.finish(&mut report);
        Ok(report)
    }

    fn pause(&mut self, report: &mut Report) {
        let jobs = match load_jobs(&self.plan.jobs_dir) {
            Ok(jobs) => jobs,
            Err(error) => {
                report.errors.push(error.to_string());
                Vec::new()
            }
        };
        // On disk before the first job is touched: this is what a crash recovers from.
        let listed = ResumeList::at(&self.plan.resume_list).write(&jobs);
        let marked = write_marker(&self.plan.marker);
        if let Err(error) = listed.map_err(|e| e.to_string()).and(marked) {
            report.errors.push(format!("{error}; no job was paused"));
            return;
        }
        for job in &jobs {
            match self.world.run(&job.checkpoint) {
                Ok(0) => report.paused.push(job.id.clone()),
                Ok(code) => report
                    .not_paused
                    .push((job.id.clone(), format!("its checkpoint exited with {code}"))),
                Err(error) => report.not_paused.push((job.id.clone(), error)),
            }
        }
    }

    fn train(&mut self, idle_when_started: u64, report: &mut Report) -> TrainerEnd {
        if self.plan.trainer.is_empty() {
            return TrainerEnd::Skipped("no trainer is configured".into());
        }
        if !DayBuffer::exists(&self.plan.buffer_dir) {
            return TrainerEnd::Skipped("no windows were recorded today".into());
        }
        match self.open_windows() {
            Ok(0) => return TrainerEnd::Skipped("no windows were recorded today".into()),
            Ok(count) => report.windows = count,
            Err(error) => return TrainerEnd::Failed(error),
        }
        let command: Vec<String> = self
            .plan
            .trainer
            .iter()
            .map(|argument| {
                argument
                    .replace("{work}", &self.plan.work_dir.display().to_string())
                    .replace("{activity}", &self.plan.activity_file.display().to_string())
            })
            .collect();
        // The trainer refuses to start without a fresh activity file.
        self.report_activity(idle_when_started);
        let mut trainer = match self.world.start(&command) {
            Ok(trainer) => trainer,
            Err(error) => return TrainerEnd::Failed(error),
        };
        let mut asked_to_stop_for: Option<u64> = None;
        loop {
            if let Some(exit_code) = trainer.finished() {
                return match asked_to_stop_for {
                    Some(_) => TrainerEnd::Interrupted,
                    None => TrainerEnd::Finished { exit_code },
                };
            }
            let now = self.world.conditions();
            // The file is how the trainer learns the person is back: it reads the idle
            // time at every step and leaves on its own.
            self.report_activity(now.idle_seconds);
            if asked_to_stop_for.is_none() && self.plan.gate.must_stop(&now, idle_when_started) {
                asked_to_stop_for = Some(0);
            }
            if let Some(waited) = asked_to_stop_for.as_mut() {
                if *waited >= self.plan.grace_seconds {
                    trainer.kill();
                    return TrainerEnd::Killed;
                }
                *waited += 1;
            }
            self.world.wait(1);
        }
    }

    fn report_activity(&mut self, idle_seconds: u64) {
        let _ = self
            .world
            .activity(idle_seconds)
            .write(&self.plan.activity_file);
    }

    /// Opens the day's buffer into the work directory as the trainer reads it, and says how
    /// many windows there were. A name's `+` are the folders the file goes back into
    /// (`windows+photopea+run-7+000001.npz`), and every run's receipts are joined into one
    /// `receipts.jsonl`.
    fn open_windows(&mut self) -> Result<usize, String> {
        let buffer = DayBuffer::open(&self.plan.buffer_dir).map_err(|e| e.to_string())?;
        let names = buffer.names().map_err(|e| e.to_string())?;
        let work = &self.plan.work_dir;
        let failed = |path: &Path, e: std::io::Error| format!("{}: {e}", path.display());
        let mut windows = 0;
        let mut receipts = Vec::new();
        for name in &names {
            let plain = buffer.unseal(name).map_err(|e| e.to_string())?;
            if name.starts_with("receipts+") {
                receipts.extend_from_slice(&plain);
                continue;
            }
            let path = name
                .split('+')
                .fold(work.clone(), |path, part| path.join(part));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| failed(parent, e))?;
            }
            std::fs::write(&path, plain).map_err(|e| failed(&path, e))?;
            windows += usize::from(name.starts_with("windows+"));
        }
        if windows > 0 {
            let path = work.join("receipts.jsonl");
            std::fs::write(&path, receipts).map_err(|e| failed(&path, e))?;
        }
        Ok(windows)
    }

    /// The two steps that always happen, then the marker goes.
    fn finish(&mut self, report: &mut Report) {
        self.delete(report);
        self.resume(report);
        if report.stuck.is_empty() {
            let _ = std::fs::remove_file(&self.plan.marker);
        }
    }

    fn delete(&mut self, report: &mut Report) {
        let opened = remove_dir(&self.plan.work_dir);
        let sealed = shred_dir(&self.plan.buffer_dir).map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&self.plan.activity_file);
        report.windows_deleted = opened.is_ok() && sealed.is_ok();
        report.errors.extend(opened.err());
        report.errors.extend(sealed.err());
    }

    fn resume(&mut self, report: &mut Report) {
        let list = ResumeList::at(&self.plan.resume_list);
        let jobs = match list.read() {
            Ok(jobs) => jobs,
            Err(error) => {
                report.errors.push(error.to_string());
                return;
            }
        };
        let mut waiting: Vec<(Job, Option<SystemTime>)> = Vec::new();
        for job in jobs {
            let before = job.beat();
            match self.world.run(&job.resume) {
                Ok(0) => waiting.push((job, before)),
                Ok(code) => {
                    report
                        .errors
                        .push(format!("{}: its resume exited with {code}", job.id));
                    report.stuck.push(job.id);
                }
                Err(error) => {
                    report.errors.push(format!("{}: {error}", job.id));
                    report.stuck.push(job.id);
                }
            }
        }
        let mut waited = 0;
        loop {
            let (moving, still): (Vec<_>, Vec<_>) = waiting
                .into_iter()
                .partition(|(job, before)| job.moved_from(*before));
            report
                .resumed
                .extend(moving.into_iter().map(|(job, _)| job.id));
            waiting = still;
            if waiting.is_empty() || waited >= self.plan.resume_wait_seconds {
                break;
            }
            self.world.wait(5);
            waited += 5;
        }
        report
            .stuck
            .extend(waiting.iter().map(|(job, _)| job.id.clone()));
        // A stuck job stays listed, so the next start tries it again.
        let left: Vec<Job> = waiting.into_iter().map(|(job, _)| job).collect();
        let kept = if left.is_empty() {
            list.clear()
        } else {
            list.write(&left)
        };
        report.errors.extend(kept.err().map(|e| e.to_string()));
    }
}

fn empty_report(trainer: TrainerEnd) -> Report {
    Report {
        paused: Vec::new(),
        not_paused: Vec::new(),
        windows: 0,
        trainer,
        windows_deleted: false,
        resumed: Vec::new(),
        stuck: Vec::new(),
        errors: Vec::new(),
    }
}

fn write_marker(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, b"a REM cycle is under way\n")
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn remove_dir(dir: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", dir.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::NightWindow;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    /// A machine whose night is scripted: what the sensors say at each look, and a
    /// trainer that ends after a number of looks, or never.
    struct Night {
        root: PathBuf,
        looks: Vec<Conditions>,
        looked: usize,
        trainer_ends_after: Option<usize>,
        log: Rc<RefCell<Vec<String>>>,
        heartbeats_move: bool,
    }

    struct Trainer {
        polls: usize,
        ends_after: Option<usize>,
        log: Rc<RefCell<Vec<String>>>,
    }

    impl Running for Trainer {
        fn finished(&mut self) -> Option<i32> {
            self.polls += 1;
            self.ends_after
                .filter(|after| self.polls > *after)
                .map(|_| 0)
        }
        fn kill(&mut self) {
            self.log.borrow_mut().push("kill trainer".into());
        }
    }

    impl World for Night {
        fn conditions(&mut self) -> Conditions {
            let now = self.looks[self.looked.min(self.looks.len() - 1)];
            self.looked += 1;
            now
        }
        fn activity(&mut self, idle_seconds: u64) -> Activity {
            Activity {
                updated_unix: 1,
                cpu_percent: 5,
                ram_bytes: 1,
                vram_bytes: 1,
                gpu_percent: 5,
                idle_seconds,
                inference_pending: false,
            }
        }
        fn run(&mut self, command: &[String]) -> Result<i32, String> {
            let line = command.join(" ");
            // What must already be true when the first job is paused.
            if line.starts_with("pause") {
                assert!(
                    self.root.join("state/resume.json").exists(),
                    "the resume list is written first"
                );
                assert!(self.root.join("state/cycle").exists());
            }
            if let Some(id) = line.strip_prefix("resume ") {
                if self.heartbeats_move {
                    std::fs::write(self.root.join(format!("{id}.heartbeat")), "{}").unwrap();
                }
            }
            self.log.borrow_mut().push(line);
            Ok(0)
        }
        fn start(&mut self, command: &[String]) -> Result<Box<dyn Running>, String> {
            let work = self.root.join("work");
            let opened = std::fs::read_dir(work.join("windows/photopea/run-7"))
                .unwrap()
                .count();
            assert!(work.join("records/photopea/run-7/000001.json").exists());
            let receipts = std::fs::read_to_string(work.join("receipts.jsonl")).unwrap();
            assert_eq!(
                receipts.lines().count(),
                2,
                "every run's receipts, in one file"
            );
            self.log.borrow_mut().push(format!(
                "train {} with {opened} window(s)",
                command.join(" ")
            ));
            assert!(
                self.root.join("run/activity.json").exists(),
                "the trainer needs a fresh activity file"
            );
            Ok(Box::new(Trainer {
                polls: 0,
                ends_after: self.trainer_ends_after,
                log: self.log.clone(),
            }))
        }
        fn now(&mut self) -> SystemTime {
            // Earlier than any file the test writes, whatever the filesystem's clock grain.
            SystemTime::now() - Duration::from_secs(30)
        }
        fn wait(&mut self, _seconds: u64) {}
    }

    const AWAY: Conditions = Conditions {
        local_minute: 180,
        on_mains: true,
        idle_seconds: 4000,
    };
    const BACK: Conditions = Conditions {
        local_minute: 181,
        on_mains: true,
        idle_seconds: 3,
    };

    fn night(
        name: &str,
        looks: Vec<Conditions>,
        trainer_ends_after: Option<usize>,
    ) -> (Plan, Night, Rc<RefCell<Vec<String>>>) {
        let root =
            std::env::temp_dir().join(format!("archie-rem-cycle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("jobs")).unwrap();
        for id in ["render", "trainer"] {
            std::fs::write(
                root.join(format!("jobs/{id}.toml")),
                format!(
                    "id = \"{id}\"\ncheckpoint = [\"pause\", \"{id}\"]\nresume = [\"resume\", \"{id}\"]\n\
                     heartbeat = '{}'\nmax_age_seconds = 600\n",
                    root.join(format!("{id}.heartbeat")).display()
                ),
            )
            .unwrap();
        }
        let buffer = DayBuffer::open(root.join("day")).unwrap();
        for sample in ["000000", "000001"] {
            buffer
                .seal(&format!("windows+photopea+run-7+{sample}.npz"), b"a window")
                .unwrap();
            buffer
                .seal(&format!("records+photopea+run-7+{sample}.json"), b"{}")
                .unwrap();
        }
        buffer
            .seal("receipts+photopea+run-7.jsonl", b"{\"receipt_id\":\"a\"}\n")
            .unwrap();
        buffer
            .seal("receipts+photopea+run-8.jsonl", b"{\"receipt_id\":\"b\"}\n")
            .unwrap();
        let plan = Plan {
            gate: Gate {
                window: NightWindow {
                    start_minute: 120,
                    end_minute: 360,
                },
                idle_seconds: 1800,
            },
            jobs_dir: root.join("jobs"),
            resume_list: root.join("state/resume.json"),
            marker: root.join("state/cycle"),
            buffer_dir: root.join("day"),
            work_dir: root.join("work"),
            activity_file: root.join("run/activity.json"),
            trainer: vec!["trainer".into(), "--windows-root".into(), "{work}".into()],
            grace_seconds: 3,
            resume_wait_seconds: 10,
            retention_seconds: 26 * 3600,
        };
        let log = Rc::new(RefCell::new(Vec::new()));
        let world = Night {
            root,
            looks,
            looked: 0,
            trainer_ends_after,
            log: log.clone(),
            heartbeats_move: true,
        };
        (plan, world, log)
    }

    #[test]
    fn a_quiet_night_pauses_trains_deletes_and_resumes_in_that_order() {
        let (plan, mut world, log) = night("quiet", vec![AWAY], Some(2));
        let report = Cycle::new(&plan, &mut world).run().unwrap();
        assert_eq!(report.paused, vec!["render", "trainer"]);
        assert_eq!(report.windows, 2);
        assert_eq!(report.trainer, TrainerEnd::Finished { exit_code: 0 });
        assert!(report.windows_deleted);
        assert_eq!(report.resumed, vec!["render", "trainer"]);
        assert!(
            report.stuck.is_empty() && report.errors.is_empty(),
            "{report:?}"
        );

        let log = log.borrow();
        assert_eq!(log[0], "pause render");
        assert_eq!(log[1], "pause trainer");
        assert!(
            log[2].starts_with("train trainer --windows-root")
                && log[2].ends_with("with 2 window(s)"),
            "{}",
            log[2]
        );
        assert_eq!(&log[3..], ["resume render", "resume trainer"]);

        assert!(
            !plan.buffer_dir.exists(),
            "the day's key and windows are gone"
        );
        assert!(!plan.work_dir.exists(), "and so are the opened copies");
        assert!(!plan.marker.exists() && !plan.resume_list.exists());
    }

    #[test]
    fn when_the_person_comes_back_the_trainer_is_told_then_killed_and_the_day_is_still_deleted() {
        let (plan, mut world, log) = night("back", vec![AWAY, AWAY, BACK], None);
        let report = Cycle::new(&plan, &mut world).run().unwrap();
        assert_eq!(report.trainer, TrainerEnd::Killed);
        assert!(log.borrow().contains(&"kill trainer".to_string()));
        assert!(report.windows_deleted && !plan.buffer_dir.exists());
        assert_eq!(report.resumed, vec!["render", "trainer"]);
    }

    #[test]
    fn a_trainer_that_leaves_when_told_was_interrupted_not_killed() {
        let (plan, mut world, log) = night("leaves", vec![AWAY, BACK], Some(3));
        let report = Cycle::new(&plan, &mut world).run().unwrap();
        assert_eq!(report.trainer, TrainerEnd::Interrupted);
        assert!(!log.borrow().contains(&"kill trainer".to_string()));
        assert!(report.windows_deleted);
    }

    #[test]
    fn by_day_nothing_is_paused_and_nothing_is_deleted() {
        let noon = Conditions {
            local_minute: 720,
            on_mains: true,
            idle_seconds: 9000,
        };
        let (plan, mut world, log) = night("day", vec![noon], Some(1));
        assert_eq!(
            Cycle::new(&plan, &mut world).run(),
            Err(Wait::OutsideNightWindow)
        );
        assert!(log.borrow().is_empty());
        assert!(DayBuffer::exists(&plan.buffer_dir));
    }

    #[test]
    fn a_supervisor_killed_mid_cycle_deletes_and_resumes_at_its_next_start() {
        let (plan, mut world, log) = night("crash", vec![AWAY], Some(1));
        let jobs = load_jobs(&plan.jobs_dir).unwrap();
        ResumeList::at(&plan.resume_list).write(&jobs).unwrap();
        write_marker(&plan.marker).unwrap();
        std::fs::create_dir_all(&plan.work_dir).unwrap();
        std::fs::write(
            plan.work_dir.join("photopea.run1.s0.npz"),
            b"an opened window",
        )
        .unwrap();

        let report = Cycle::new(&plan, &mut world)
            .recover()
            .expect("a cut-short cycle is finished");
        assert!(report.windows_deleted);
        assert!(!plan.buffer_dir.exists() && !plan.work_dir.exists());
        assert_eq!(report.resumed, vec!["render", "trainer"]);
        assert_eq!(*log.borrow(), ["resume render", "resume trainer"]);
        assert!(
            Cycle::new(&plan, &mut world).recover().is_none(),
            "nothing is left to recover"
        );
    }

    #[test]
    fn a_job_that_does_not_move_is_reported_and_stays_listed() {
        let (plan, mut world, _log) = night("stuck", vec![AWAY], Some(1));
        world.heartbeats_move = false;
        let report = Cycle::new(&plan, &mut world).run().unwrap();
        assert!(report.resumed.is_empty());
        assert_eq!(report.stuck, vec!["render", "trainer"]);
        assert_eq!(ResumeList::at(&plan.resume_list).read().unwrap().len(), 2);
        assert!(plan.marker.exists(), "the next start tries again");
        assert!(
            report.windows_deleted,
            "a stuck job does not keep the day's windows"
        );
    }

    #[test]
    fn windows_that_outlived_their_day_are_deleted_without_a_cycle() {
        let (mut plan, mut world, log) = night("old", vec![AWAY], Some(1));
        assert!(
            Cycle::new(&plan, &mut world).recover().is_none(),
            "today's windows are kept for tonight"
        );
        plan.retention_seconds = 0;
        std::thread::sleep(Duration::from_millis(20));
        struct Later<'a>(&'a mut Night);
        impl World for Later<'_> {
            fn conditions(&mut self) -> Conditions {
                self.0.conditions()
            }
            fn activity(&mut self, idle: u64) -> Activity {
                self.0.activity(idle)
            }
            fn run(&mut self, command: &[String]) -> Result<i32, String> {
                self.0.run(command)
            }
            fn start(&mut self, command: &[String]) -> Result<Box<dyn Running>, String> {
                self.0.start(command)
            }
            fn now(&mut self) -> SystemTime {
                SystemTime::now() + Duration::from_secs(3600)
            }
            fn wait(&mut self, seconds: u64) {
                self.0.wait(seconds)
            }
        }
        let mut later = Later(&mut world);
        let report = Cycle::new(&plan, &mut later)
            .recover()
            .expect("old windows are deleted");
        assert!(report.windows_deleted && !plan.buffer_dir.exists());
        assert!(log.borrow().is_empty(), "no job was touched");
    }
}
