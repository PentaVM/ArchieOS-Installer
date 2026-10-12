//! The long-jobs reader. `status.json` is written with the REAL
//! `archie_bridges::jobs::Status`, so the bridge is held to what `archie-bridges` emits.

use archie_bridges::jobs::{State, Status};
use archie_shell_bridge::jobs::{read, Jobs, MAX_JOBS, MAX_STATUS_BYTES};
use serde_json::json;
use std::path::{Path, PathBuf};

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("archie-shell-jobs-{tag}-{}", std::process::id()))
        .join("motor-bracket");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn status(id: &str, state: State, done: u64, total: u64, started: u64, updated: u64) -> Status {
    Status {
        id: id.into(),
        bridge: "freecad".into(),
        kind: "sweep".into(),
        state,
        done,
        total,
        started_ms: started,
        updated_ms: updated,
        latest: Some(json!({"private_path": "/home/someone/secret.step"})),
        error: None,
    }
}

fn write_job(ws: &Path, status: &Status) {
    let dir = ws.join(".archie").join("jobs").join(&status.id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("spec.json"), b"{}").unwrap();
    std::fs::write(dir.join("status.json"), serde_json::to_vec(status).unwrap()).unwrap();
}

fn unwrap_ok(jobs: Jobs) -> (Vec<archie_shell_bridge::Job>, usize) {
    match jobs {
        Jobs::Ok { jobs, skipped, .. } => (jobs, skipped),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn no_workspace_given_is_not_configured() {
    assert_eq!(read(&[], 0), Jobs::NotConfigured);
}

#[test]
fn a_job_written_by_the_real_type_is_read_back_without_its_free_form_result() {
    let ws = workspace("ok");
    write_job(&ws, &status("job-1", State::Running, 31, 50, 1_000, 5_000));
    let jobs = read(std::slice::from_ref(&ws), 9_000);
    let line = serde_json::to_string(&jobs).unwrap();
    assert!(
        !line.contains("private_path"),
        "`latest` must not reach the shell: {line}"
    );
    let (jobs, skipped) = unwrap_ok(jobs);
    assert_eq!(skipped, 0);
    let job = &jobs[0];
    assert_eq!((job.id.as_str(), job.done, job.total), ("job-1", 31, 50));
    assert_eq!(job.state, archie_shell_bridge::jobs::State::Running);
    assert_eq!(job.idle_ms, 4_000);
    assert_eq!(job.workspace, "motor-bracket");
}

#[test]
fn only_the_workspace_folder_name_is_sent_never_its_path() {
    let ws = workspace("label");
    write_job(&ws, &status("j", State::Done, 1, 1, 1, 2));
    let line = serde_json::to_string(&read(std::slice::from_ref(&ws), 3)).unwrap();
    let parent = ws
        .parent()
        .unwrap()
        .display()
        .to_string()
        .replace('\\', "\\\\");
    assert!(!line.contains(&parent), "a full path leaked: {line}");
}

#[test]
fn jobs_come_newest_first_and_every_state_round_trips() {
    let ws = workspace("order");
    write_job(&ws, &status("a", State::Done, 5, 5, 100, 200));
    write_job(&ws, &status("b", State::Failed, 1, 5, 300, 400));
    write_job(&ws, &status("c", State::Cancelled, 2, 5, 200, 250));
    write_job(&ws, &status("d", State::Starting, 0, 5, 50, 50));
    let (jobs, _) = unwrap_ok(read(std::slice::from_ref(&ws), 1_000));
    let order: Vec<&str> = jobs.iter().map(|j| j.id.as_str()).collect();
    assert_eq!(order, ["b", "c", "a", "d"]);
}

#[test]
fn a_workspace_that_never_ran_a_job_is_empty_not_an_error() {
    let ws = workspace("none");
    let (jobs, skipped) = unwrap_ok(read(std::slice::from_ref(&ws), 0));
    assert!(jobs.is_empty() && skipped == 0);
}

#[test]
fn a_clock_that_runs_backwards_gives_zero_idle_not_a_wrap() {
    let ws = workspace("skew");
    write_job(&ws, &status("j", State::Running, 1, 2, 10, 9_999_999));
    let (jobs, _) = unwrap_ok(read(std::slice::from_ref(&ws), 100));
    assert_eq!(jobs[0].idle_ms, 0);
}

#[test]
fn damaged_or_oversized_status_files_are_counted_not_hidden() {
    let ws = workspace("bad");
    write_job(&ws, &status("good", State::Running, 1, 2, 1, 2));
    let junk = ws.join(".archie/jobs/junk");
    std::fs::create_dir_all(&junk).unwrap();
    std::fs::write(junk.join("status.json"), b"not json").unwrap();
    let big = ws.join(".archie/jobs/big");
    std::fs::create_dir_all(&big).unwrap();
    std::fs::write(
        big.join("status.json"),
        vec![b' '; (MAX_STATUS_BYTES + 5) as usize],
    )
    .unwrap();
    let (jobs, skipped) = unwrap_ok(read(std::slice::from_ref(&ws), 10));
    assert_eq!(jobs.len(), 1);
    assert_eq!(skipped, 2, "the shell is told something was not readable");
}

#[test]
fn a_long_error_is_truncated() {
    let ws = workspace("err");
    let mut failed = status("j", State::Failed, 1, 2, 1, 2);
    failed.error = Some("x".repeat(5_000));
    write_job(&ws, &failed);
    let (jobs, _) = unwrap_ok(read(std::slice::from_ref(&ws), 10));
    assert_eq!(jobs[0].error.as_ref().unwrap().chars().count(), 300);
}

#[test]
fn more_jobs_than_the_cap_are_counted_but_not_all_sent() {
    let ws = workspace("cap");
    for i in 0..(MAX_JOBS as u64 + 3) {
        write_job(&ws, &status(&format!("j{i:03}"), State::Done, 1, 1, i, i));
    }
    let (jobs, skipped) = unwrap_ok(read(std::slice::from_ref(&ws), 1_000));
    assert_eq!(jobs.len(), MAX_JOBS);
    assert_eq!(skipped, 3);
    assert_eq!(
        jobs[0].id,
        format!("j{:03}", MAX_JOBS + 2),
        "the newest are kept"
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_job_folder_is_never_followed() {
    let ws = workspace("link");
    let outside = std::env::temp_dir().join(format!("archie-shell-outside-{}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        outside.join("status.json"),
        serde_json::to_vec(&status("evil", State::Running, 1, 2, 1, 2)).unwrap(),
    )
    .unwrap();
    let jobs_dir = ws.join(".archie/jobs");
    std::fs::create_dir_all(&jobs_dir).unwrap();
    std::os::unix::fs::symlink(&outside, jobs_dir.join("evil")).unwrap();
    let (jobs, _) = unwrap_ok(read(std::slice::from_ref(&ws), 10));
    assert!(jobs.is_empty());
}

#[test]
fn a_frame_carries_the_jobs_state_in_its_json_line() {
    use archie_shell_bridge::{poll_with, Client, Sources};
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let ws = workspace("frame");
    write_job(&ws, &status("job-9", State::Running, 3, 9, 1, 2));
    let frame = poll_with(
        &Client::new(addr).unwrap(),
        &Sources {
            lan_file: None,
            workspaces: vec![ws],
            ..Default::default()
        },
    );
    let value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&frame).unwrap()).unwrap();
    assert_eq!(value["jobs"]["state"], "ok");
    assert_eq!(value["jobs"]["jobs"][0]["id"], "job-9");
    assert_eq!(value["lan"]["state"], "not_configured");
}

#[test]
fn a_workspaces_root_is_listed_and_an_empty_one_is_an_empty_answer() {
    use archie_shell_bridge::jobs::read_sources;
    let ws = workspace("root");
    let root = ws.parent().unwrap().to_path_buf();
    write_job(&ws, &status("from-root", State::Running, 1, 2, 1, 2));
    let (jobs, _) = unwrap_ok(read_sources(&[], Some(&root), 10));
    assert_eq!(jobs[0].id, "from-root");

    let empty = std::env::temp_dir().join(format!(
        "archie-shell-jobs-emptyroot-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&empty).unwrap();
    let (jobs, skipped) = unwrap_ok(read_sources(&[], Some(&empty), 10));
    assert!(jobs.is_empty() && skipped == 0);
}

#[test]
fn a_root_that_cannot_be_listed_is_reported_not_silently_empty() {
    use archie_shell_bridge::jobs::read_sources;
    let gone =
        std::env::temp_dir().join(format!("archie-shell-jobs-noroot-{}", std::process::id()));
    match read_sources(&[], Some(&gone), 10) {
        Jobs::Ok {
            jobs, unreadable, ..
        } => {
            assert!(jobs.is_empty());
            assert_eq!(unreadable.len(), 1, "the missing root is named");
        }
        other => panic!("got {other:?}"),
    }
}
