use archie_core::cortex::refresh_module_graph;
use archie_registry::ModuleRegistry;
use archie_state::StateStore;
use archie_types::ModuleManifest;

fn module(id: &str, dependencies: &str) -> ModuleManifest {
    toml::from_str(&format!("id = '{id}'\nversion = '1'\ndescription = 'Analytical module'\ndependencies = [{dependencies}]\n[worker]\nkind = 'none'\n")).unwrap()
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn new_modules_automatically_become_cognitive_nodes_and_repeated_boot_is_incremental() {
    let state = StateStore::open_test().unwrap();
    use archie_memory::cognitive::{CognitiveEvent, CognitivePayload, FocusRecord};
    let focus = FocusRecord {
        focus_id: "focus:ongoing".into(),
        revision: 1,
        objective_ref: "vault:objective".into(),
        parent_focus_id: None,
        suspended: false,
        half_life_ms: 1000,
        updated_at_ms: 50,
        activations: vec![],
    };
    state
        .record_cognitive_event(&CognitiveEvent {
            schema_version: 1,
            event_id: "focus:event".into(),
            recorded_at_ms: 50,
            payload: CognitivePayload::FocusUpdated(focus.clone()),
        })
        .unwrap();
    let mut registry = ModuleRegistry::new();
    registry.register(module("new-tool", "")).unwrap();
    let first = refresh_module_graph(&state, &registry, 100).unwrap();
    assert_eq!(first.nodes.len(), 1);
    assert!(first.nodes[0]
        .summary
        .contains("Execution authority remains with policy"));
    assert_eq!(refresh_module_graph(&state, &registry, 200).unwrap(), first);
    registry
        .register(module("new-analysis", "'new-tool'"))
        .unwrap();
    let next = refresh_module_graph(&state, &registry, 300).unwrap();
    assert_eq!(next.nodes.len(), 2);
    assert_eq!(next.edges.len(), 1);
    assert_eq!(state.cortex_graph_at(200).unwrap().unwrap(), first);
    assert_eq!(
        state.cognitive_focus_at("focus:ongoing", 300).unwrap(),
        Some(focus)
    );
}
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn dependency_added_later_is_connected_without_changing_parent_manifest() {
    let state = StateStore::open_test().unwrap();
    let mut registry = ModuleRegistry::new();
    registry.register(module("analysis", "'backend'")).unwrap();
    let first = refresh_module_graph(&state, &registry, 100).unwrap();
    assert!(first.edges.is_empty());
    registry.register(module("backend", "")).unwrap();
    let next = refresh_module_graph(&state, &registry, 200).unwrap();
    assert_eq!(next.edges.len(), 1);
    assert_eq!(next.edges[0].target_id, "os/module/backend");
}
#[cfg(unix)]
#[test]
#[ignore = "needs a PostgreSQL cluster: run `node tests/with-postgres.mjs --core` from tools/engineering-dashboard"]
fn snapshot_publication_is_atomic_readable_and_leaves_no_temporary_files() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!(
        "cortex-export-{}-{}",
        std::process::id(),
        archie_types::Timestamp::now().as_millis()
    ));
    let paths = archie_state::StatePaths::rooted(&root);
    let state = StateStore::open_test().unwrap();
    let mut registry = ModuleRegistry::new();
    registry.register(module("new-tool", "")).unwrap();
    let first = refresh_module_graph(&state, &registry, 100).unwrap();
    archie_core::cortex::export_module_graph(&paths, &first).unwrap();
    let file = paths.state_dir.join("knowledge/graph/current.acg");
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let bytes = std::fs::read(&file).unwrap();
    archie_core::cortex::export_module_graph(&paths, &first).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    assert_eq!(
        std::fs::read_dir(file.parent().unwrap()).unwrap().count(),
        1
    );
    std::fs::remove_file(file).unwrap();
    // The temporary root contains no operator files; leave empty directories to the OS.
}
