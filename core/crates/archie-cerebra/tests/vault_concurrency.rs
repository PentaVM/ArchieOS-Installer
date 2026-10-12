use archie_cerebra::vault_io::{open_vault, persist_vault};
use archie_types::PrivacyGrade;
use archie_vault::{EntryClass, VaultRef};
use std::{path::PathBuf, process::{Command, Stdio}, time::Duration};

#[test]
fn concurrent_processes_preserve_each_others_vault_entries() {
    let dir = std::env::temp_dir().join(format!("archie-wanda-concurrency-{}", std::process::id()));
    if dir.exists() { std::fs::remove_dir_all(&dir).unwrap(); }
    std::fs::create_dir_all(&dir).unwrap();
    { let vault = open_vault(&dir).unwrap(); persist_vault(&dir, &vault).unwrap(); }
    let mut children = Vec::new();
    for worker in 0..6 {
        children.push(Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "vault_writer_process", "--nocapture"])
            .env("ARCHIE_WANDA_TEST_VAULT", &dir)
            .env("ARCHIE_WANDA_TEST_WORKER", worker.to_string())
            .stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap());
    }
    let outputs: Vec<_> = children.into_iter().map(|child| child.wait_with_output().unwrap()).collect();
    assert!(outputs.iter().all(|o| o.status.success()), "a writer lost its transaction: {:?}", outputs.iter().map(|o| (&o.status, String::from_utf8_lossy(&o.stderr))).collect::<Vec<_>>());
    let reopened = open_vault(&dir).unwrap();
    assert_eq!(reopened.sealed_entries().len(), 24, "concurrent load-modify-save must retain all updates");
    drop(reopened);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "subprocess fixture, invoked by the concurrency test"]
fn vault_writer_process() {
    let dir = PathBuf::from(std::env::var_os("ARCHIE_WANDA_TEST_VAULT").expect("parent test directory"));
    let worker = std::env::var("ARCHIE_WANDA_TEST_WORKER").unwrap();
    for item in 0..4 {
        let mut vault = open_vault(&dir).unwrap();
        std::thread::sleep(Duration::from_millis(25));
        let reference = VaultRef::parse(&format!("vault:personal/owner/worker-{worker}/item-{item}")).unwrap();
        vault.store("concurrency-test", reference, EntryClass::PrivateFact, PrivacyGrade::Restricted, "fixture fact", 1_000, None).unwrap();
        persist_vault(&dir, &vault).unwrap();
    }
}

#[test]
fn a_key_from_another_directory_cannot_overwrite_a_vault() {
    let root = std::env::temp_dir().join(format!("archie-wanda-bound-vault-{}", std::process::id()));
    if root.exists() { std::fs::remove_dir_all(&root).unwrap(); }
    let a = root.join("a"); let b = root.join("b");
    let vault_a = open_vault(&a).unwrap();
    { let vault_b = open_vault(&b).unwrap(); persist_vault(&b, &vault_b).unwrap(); }
    assert!(persist_vault(&b, &vault_a).is_err(), "persistence must be bound to the opened vault directory");
    drop(vault_a);
    std::fs::remove_dir_all(&root).unwrap();
}
