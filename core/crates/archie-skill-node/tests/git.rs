//! The real git backend against a local bare repository: two clones, as two people's nodes.

use archie_skill_node::hub::{GitRepo, Hub, HubError};
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "archie-skill-node-git-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A shared repository with one commit, and a clone of it for `who`.
fn clone_for(root: &Path, who: &str) -> GitRepo {
    let dir = root.join(who);
    git(root, &["clone", "--quiet", "shared.git", who]);
    git(&dir, &["config", "user.name", who]);
    git(
        &dir,
        &["config", "user.email", &format!("{who}@example.invalid")],
    );
    GitRepo::new(&dir, "main")
}

#[test]
fn two_clones_commit_as_their_owners_and_the_slower_one_is_told_to_read_again() {
    let root = scratch("race");
    git(
        &root,
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch=main",
            "shared.git",
        ],
    );
    let seed = root.join("seed");
    git(&root, &["clone", "--quiet", "shared.git", "seed"]);
    git(&seed, &["config", "user.name", "seed"]);
    git(&seed, &["config", "user.email", "seed@example.invalid"]);
    std::fs::write(seed.join("README.md"), "skills\n").unwrap();
    git(&seed, &["add", "README.md"]);
    git(&seed, &["commit", "--quiet", "-m", "start"]);
    git(&seed, &["push", "--quiet", "origin", "HEAD:main"]);

    let nelson = clone_for(&root, "nelson");
    let xavi = clone_for(&root, "xavi");
    assert_eq!(nelson.read("board.json"), Err(HubError::NotFound));
    assert_eq!(nelson.read("../outside"), Err(HubError::NotFound));

    let start = nelson.head().unwrap();
    assert_eq!(xavi.head().unwrap(), start);
    let first = nelson
        .commit(
            "nelson claims csgo",
            &[
                ("board.json".into(), b"{\"v\":1}".to_vec()),
                ("skills/csgo/a.skill".into(), vec![1, 2, 3]),
            ],
            &start,
        )
        .unwrap();
    // Xavi still holds the old head: the remote refuses, and nothing of his lands.
    assert_eq!(
        xavi.commit(
            "xavi claims csgo",
            &[("board.json".into(), b"{\"v\":2}".to_vec())],
            &start
        ),
        Err(HubError::Moved)
    );
    let head = xavi.head().unwrap();
    assert_eq!(head, first);
    assert_eq!(xavi.read("board.json").unwrap(), b"{\"v\":1}");
    assert_eq!(xavi.read("skills/csgo/a.skill").unwrap(), vec![1, 2, 3]);
    xavi.commit(
        "xavi wants kdenlive",
        &[("board.json".into(), b"{\"v\":3}".to_vec())],
        &head,
    )
    .unwrap();
    assert_eq!(
        nelson.read("board.json").unwrap(),
        b"{\"v\":1}",
        "until he fetches"
    );
    nelson.head().unwrap();
    assert_eq!(nelson.read("board.json").unwrap(), b"{\"v\":3}");

    // The history is the credit: each commit carries the name of the person whose node made it.
    let log = Command::new("git")
        .arg("-C")
        .arg(root.join("shared.git"))
        .args(["log", "--format=%an: %s", "main"])
        .output()
        .unwrap();
    let lines: Vec<String> = String::from_utf8_lossy(&log.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        lines,
        vec![
            "xavi: xavi wants kdenlive",
            "nelson: nelson claims csgo",
            "seed: start"
        ]
    );
    assert!(nelson
        .commit("x", &[("../escape".into(), vec![0])], &first)
        .is_err());
    let _ = std::fs::remove_dir_all(root);
}
