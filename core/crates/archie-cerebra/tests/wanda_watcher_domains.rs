use archie_cerebra::{vault_io, wanda_watcher};
use archie_vault::ContextDomain;

#[test]
fn an_export_assigned_to_work_stays_in_its_project() {
    let root = std::env::temp_dir().join(format!("wanda-work-export-{}", std::process::id()));
    if root.exists() { std::fs::remove_dir_all(&root).unwrap(); }
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("manager.txt");
    std::fs::write(&file, "[10/10/2026, 14:32:01] Manager: urgente: el trabajo vence mañana\n").unwrap();
    let domain = ContextDomain::work("studio", "building").unwrap();
    let report = wanda_watcher::ingest_export_file_in_domain(&root.join("vault"), &root.join("journal.json"), &file, "domain-fixture-salt", "work-ingest", &domain).unwrap();
    assert_eq!(report.files_ingested, 1);
    assert_eq!(report.contexts.len(), 1);
    assert_eq!(report.contexts[0].domain(), &domain);
    let vault = vault_io::open_vault(&root.join("vault")).unwrap();
    assert!(vault.sealed_entries().iter().all(|entry| domain.contains(&entry.reference)), "a work export must not be labelled personal/owner");
    drop(vault);
    std::fs::remove_dir_all(root).unwrap();
}
