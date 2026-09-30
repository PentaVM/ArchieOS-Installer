use archie_skill_node::board::Board;
use archie_skill_node::hub::{Hub, HubError};
use archie_skill_node::mcp;
use archie_skill_node::node::{sha256_hex, Job, NodeConfig};
use archie_skill_node::tools::{Node, BOARD_PATH};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The shared repository, in memory: files, a head that moves with every commit, the
/// authors, and (on demand) another node committing just before this one.
#[derive(Default)]
struct Repo {
    files: BTreeMap<String, Vec<u8>>,
    head: u64,
    authors: Vec<String>,
    race_once: bool,
}

#[derive(Clone)]
struct FakeHub {
    repo: Rc<RefCell<Repo>>,
    author: String,
}

impl Hub for FakeHub {
    fn head(&self) -> Result<String, HubError> {
        Ok(self.repo.borrow().head.to_string())
    }
    fn read(&self, path: &str) -> Result<Vec<u8>, HubError> {
        self.repo.borrow().files.get(path).cloned().ok_or(HubError::NotFound)
    }
    fn commit(&self, _summary: &str, files: &[(String, Vec<u8>)], parent: &str) -> Result<String, HubError> {
        let mut repo = self.repo.borrow_mut();
        if repo.race_once {
            repo.race_once = false;
            repo.head += 1; // someone else committed first
        }
        if parent != repo.head.to_string() {
            return Err(HubError::Moved);
        }
        for (path, bytes) in files {
            repo.files.insert(path.clone(), bytes.clone());
        }
        repo.head += 1;
        repo.authors.push(self.author.clone());
        Ok(repo.head.to_string())
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("archie-skill-node-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn node(repo: &Rc<RefCell<Repo>>, who: &str, work: &Path) -> Node<FakeHub> {
    let config = NodeConfig {
        contributor: who.into(), gpu: "RTX 3060 Ti 8GB".into(), python: "python".into(), foundry: work.join("iris"),
        weights: work.join("base.pt"), skills_dir: work.join("skills"), work_dir: work.to_path_buf(),
        hub_repo: "PentaVM/iris-skills".into(), hub_token_env: "ARCHIE_TEST_NO_TOKEN".into(), base: "b".repeat(16),
    };
    Node { config, hub: FakeHub { repo: repo.clone(), author: who.into() }, clock: || 1_000 }
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

fn board(repo: &Rc<RefCell<Repo>>) -> Board {
    serde_json::from_slice(&repo.borrow().files[BOARD_PATH]).unwrap()
}

/// A job as the trainer leaves it: its record, its report and (for training) its package.
fn finished_job(node: &Node<FakeHub>, id: &str, kind: &str, candidate: &str, report: Value) {
    let job = Job { id: id.into(), kind: kind.into(), skill: "csgo".into(), source: "csgo".into(),
                    candidate: candidate.into(), pid: 0, started: 1 };
    let dir = job.dir(&node.config);
    std::fs::create_dir_all(dir.join("skills")).unwrap();
    std::fs::write(dir.join("job.json"), serde_json::to_vec(&job).unwrap()).unwrap();
    std::fs::write(dir.join("report.json"), report.to_string()).unwrap();
    if kind == "train" {
        std::fs::write(dir.join("skills").join("csgo.skill"), b"adapter bytes").unwrap();
    }
}

#[test]
fn two_people_train_and_check_each_other_and_the_board_credits_both() {
    let repo = Rc::new(RefCell::new(Repo::default()));
    let (nelson_dir, xavi_dir) = (scratch("nelson"), scratch("xavi"));
    let mut nelson = node(&repo, "nelson", &nelson_dir);
    let mut xavi = node(&repo, "xavi", &xavi_dir);

    nelson.call("skill_claim", &args(json!({"skill": "csgo", "source": "csgo", "hours": 2}))).unwrap();
    let refused = xavi.call("skill_claim", &args(json!({"skill": "csgo", "source": "csgo"}))).unwrap_err();
    assert!(refused.contains("skill_claimed_by:nelson"), "{refused}");

    // Nelson's own sealed check turned it down: nothing is uploaded.
    finished_job(&nelson, "1-train-csgo", "train", "", json!({"promoted": false, "reason": "no_gain", "base": "b".repeat(16)}));
    assert!(nelson.call("skill_publish", &args(json!({"job": "1-train-csgo"}))).unwrap_err().contains("no_gain"));

    finished_job(&nelson, "2-train-csgo", "train", "", json!({"promoted": true, "reason": "gain:0.1", "base": "b".repeat(16),
        "before": {"key_f1": 0.6}, "after": {"key_f1": 0.7}}));
    let published = nelson.call("skill_publish", &args(json!({"job": "2-train-csgo"}))).unwrap();
    let path = published["published"].as_str().unwrap().to_string();
    assert!(path.starts_with("skills/csgo/") && path.contains("nelson"));
    assert_eq!(repo.borrow().files[&path], b"adapter bytes");
    let after_publish = board(&repo);
    assert!(after_publish.skills["csgo"].claim.is_none());
    assert_eq!(after_publish.skills["csgo"].candidates[0].sha256, sha256_hex(b"adapter bytes"));
    assert_eq!(after_publish.base, "b".repeat(16));

    // Xavi's work list starts with checking it; Nelson's does not.
    let xavi_board = xavi.call("skill_board", &Map::new()).unwrap();
    assert_eq!(xavi_board["needs"][0]["kind"], "check");
    let nelson_board = nelson.call("skill_board", &Map::new()).unwrap();
    assert!(nelson_board["needs"].as_array().unwrap().iter().all(|need| need["kind"] != "check"));

    // Nelson cannot record a check of his own package.
    finished_job(&nelson, "3-verify-csgo", "verify", &path, json!({"verified": true, "reason": "gain", "base": "b".repeat(16)}));
    assert!(nelson.call("skill_record_check", &args(json!({"job": "3-verify-csgo"}))).is_err());

    finished_job(&xavi, "4-verify-csgo", "verify", &path, json!({"verified": true, "reason": "gain:0.08", "base": "b".repeat(16)}));
    xavi.call("skill_record_check", &args(json!({"job": "4-verify-csgo"}))).unwrap();
    let done = board(&repo);
    let current = done.skills["csgo"].current.as_ref().unwrap();
    assert_eq!((current.trained_by.as_str(), current.verified_by.as_str()), ("nelson", "xavi"));
    assert_eq!((done.credits["nelson"].promoted, done.credits["xavi"].verified), (1, 1));
    // Each change was committed by the node that made it.
    assert_eq!(repo.borrow().authors, vec!["nelson", "nelson", "xavi"]);

    // Installing puts the package where IRIS serving reads it, if its digest matches.
    xavi.call("skill_install", &args(json!({"skill": "csgo"}))).unwrap();
    assert_eq!(std::fs::read(xavi_dir.join("skills").join("csgo.skill")).unwrap(), b"adapter bytes");
    let _ = (std::fs::remove_dir_all(nelson_dir), std::fs::remove_dir_all(xavi_dir));
}

#[test]
fn a_change_made_while_someone_else_committed_is_redone_on_the_new_board() {
    let repo = Rc::new(RefCell::new(Repo::default()));
    let dir = scratch("race");
    let mut sheriff = node(&repo, "sheriff", &dir);
    sheriff.call("skill_want", &args(json!({"skill": "rocket-league", "why": "Nelson plays it", "source": "recorded"}))).unwrap();
    repo.borrow_mut().race_once = true;
    sheriff.call("skill_want", &args(json!({"skill": "kdenlive", "why": "video editing"}))).unwrap();
    let wanted: Vec<String> = board(&repo).wanted.into_iter().map(|w| w.skill).collect();
    assert_eq!(wanted, vec!["rocket-league", "kdenlive"], "nothing was lost to the race");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_protocol_lists_the_tools_and_answers_calls_and_errors() {
    let repo = Rc::new(RefCell::new(Repo::default()));
    let dir = scratch("mcp");
    let mut node = node(&repo, "nelson", &dir);
    let reply: Value = serde_json::from_str(&mcp::handle(&mut node,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#).unwrap()).unwrap();
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
    assert!(mcp::handle(&mut node, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    let list: Value = serde_json::from_str(&mcp::handle(&mut node, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap()).unwrap();
    let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"skill_board") && names.contains(&"skill_check") && names.len() == 11);
    let call: Value = serde_json::from_str(&mcp::handle(&mut node,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"skill_train","arguments":{"skill":"csgo"}}}"#).unwrap()).unwrap();
    assert_eq!(call["result"]["isError"], true, "training an unclaimed skill is refused");
    let bad: Value = serde_json::from_str(&mcp::handle(&mut node,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"rm_rf","arguments":{}}}"#).unwrap()).unwrap();
    assert_eq!(bad["error"]["code"], -32602);
    let status: Value = serde_json::from_str(&mcp::handle(&mut node,
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"node_status","arguments":{}}}"#).unwrap()).unwrap();
    let text = status["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("\"hub_token_set\": false"), "the token's presence is said, never the token");
    let _ = std::fs::remove_dir_all(dir);
}
