//! Trusted module-to-graph projection. Installed/declared is distinct from running
//! or authorized. Refreshes preserve other knowledge and do not reset active Focus.
use archie_memory::graph::{EdgeKind, GraphEdge, GraphNode, GraphSnapshot, NodeKind, Sensitivity};
use archie_memory::storage::GraphStore;
use archie_registry::ModuleRegistry;
use archie_state::StatePaths;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
#[cfg(unix)]
use std::io::Write;

pub fn refresh_module_graph<S: GraphStore>(
    store: &S,
    modules: &ModuleRegistry,
    now_ms: i64,
) -> Result<GraphSnapshot, String>
where
    S::Error: std::fmt::Display,
{
    let previous = store.graph_at(i64::MAX).map_err(|e| e.to_string())?;
    let mut snapshot = previous.clone().unwrap_or(GraphSnapshot {
        snapshot_id: "graph:empty".into(),
        captured_at_ms: now_ms,
        nodes: vec![],
        edges: vec![],
    });
    let mut nodes: BTreeMap<String, GraphNode> = snapshot
        .nodes
        .iter()
        .cloned()
        .map(|n| (n.id.clone(), n))
        .collect();
    let mut edges: BTreeMap<String, GraphEdge> = snapshot
        .edges
        .iter()
        .cloned()
        .map(|e| (e.id.clone(), e))
        .collect();
    let mut changed = previous.is_none();
    for m in modules.manifests() {
        let id = format!("os/module/{}", m.id);
        let bytes = serde_json::to_vec(m).map_err(|e| e.to_string())?;
        let provenance = format!("manifest:{:x}", Sha256::digest(bytes));
        let same = nodes
            .get(&id)
            .is_some_and(|n| n.provenance_ref == provenance);
        let version = if same {
            nodes[&id].version
        } else {
            nodes.get(&id).map_or(Ok(1), |n| {
                n.version.checked_add(1).ok_or("module version overflow")
            })?
        };
        if !same {
            changed = true;
            nodes.insert(id.clone(), GraphNode {
                id: id.clone(), kind: NodeKind::Process,
                summary: format!("Declared module {} version {}: {}. Execution authority remains with policy and scheduler.", m.id, m.version, m.description),
                aliases: m.required_capabilities.iter().map(ToString::to_string).collect(),
                provenance_ref: provenance.clone(), sensitivity: Sensitivity::LocalRestricted,
                observed_at_ms: now_ms, version,
            });
        }
        let mut desired = std::collections::BTreeSet::new();
        for dep in &m.dependencies {
            if modules.get(dep).is_none() {
                continue;
            }
            let edge_id = format!(
                "os/dependency/{:x}",
                Sha256::digest(format!("{}/{}", m.id, dep))
            );
            desired.insert(edge_id.clone());
            if edges
                .get(&edge_id)
                .is_some_and(|e| e.provenance_ref == provenance)
            {
                continue;
            }
            changed = true;
            edges.insert(
                edge_id.clone(),
                GraphEdge {
                    id: edge_id,
                    source_id: id.clone(),
                    target_id: format!("os/module/{dep}"),
                    kind: EdgeKind::DependsOn,
                    provenance_ref: provenance.clone(),
                    observed_at_ms: now_ms,
                    version,
                },
            );
        }
        edges.retain(|edge_id, e| {
            let remove = e.source_id == id
                && edge_id.starts_with("os/dependency/")
                && !desired.contains(edge_id);
            if remove {
                changed = true;
            }
            !remove
        });
    }
    if !changed {
        return Ok(snapshot);
    }
    if previous
        .as_ref()
        .is_some_and(|p| now_ms <= p.captured_at_ms)
    {
        return Err("module refresh time does not advance current snapshot".into());
    }
    snapshot.nodes = nodes.into_values().collect();
    snapshot.edges = edges.into_values().collect();
    snapshot.captured_at_ms = now_ms;
    let identity = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    snapshot.snapshot_id = format!("graph:{:x}", Sha256::digest(identity));
    store
        .publish_graph(&snapshot, previous.as_ref().map(|p| p.snapshot_id.as_str()))
        .map_err(|e| e.to_string())?;
    Ok(snapshot)
}

/// Publish only the public/local derived view in Cortex's existing read-only input
/// directory. Private labels never enter this shared snapshot. Linux rename+fsync
/// provides atomic replacement; the DB remains authoritative if export fails.
#[cfg(unix)]
pub fn export_module_graph(paths: &StatePaths, snapshot: &GraphSnapshot) -> Result<(), String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let directory = paths.state_dir.join("knowledge/graph");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let temporary = directory.join(format!(
        ".current-{}-{}.tmp",
        std::process::id(),
        snapshot.captured_at_ms
    ));
    let mut created = false;
    let result = (|| {
        let bytes = snapshot.encode_policy_view(
            &[Sensitivity::Public, Sensitivity::LocalRestricted],
            snapshot.captured_at_ms,
        )?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o640)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        created = true;
        file.set_permissions(std::fs::Permissions::from_mode(0o640))
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, directory.join("current.acg")).map_err(|e| e.to_string())?;
        std::fs::File::open(&directory)
            .and_then(|d| d.sync_all())
            .map_err(|e| e.to_string())
    })();
    if result.is_err() && created {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(unix))]
pub fn export_module_graph(_paths: &StatePaths, _snapshot: &GraphSnapshot) -> Result<(), String> {
    Err("production snapshot publication requires Unix atomic rename and directory fsync".into())
}
