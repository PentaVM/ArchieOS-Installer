use crate::DisplaySnapshot;
use archie_orb::{
    EntityKind, OrbConfig, OrbEntity, OrbFrame, OrbModel, Signal, SignalKind, TFOG_SCHEMA,
    TOKEN_DIMENSIONS,
};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(unix)]
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex;

/// The read-only TFOG projection owned by the display bridge.
#[derive(Clone)]
pub struct OrbService {
    model: Arc<Mutex<OrbModel>>,
    last_frame_ms: Arc<Mutex<i64>>,
    snapshot_ids: Arc<Mutex<BTreeSet<String>>>,
    runtime_ids: Arc<Mutex<BTreeSet<String>>>,
    runtime: Arc<StdMutex<crate::runtime::RuntimeInventory>>,
}

impl Default for OrbService {
    fn default() -> Self {
        Self {
            model: Arc::new(Mutex::new(OrbModel::new(OrbConfig::default()))),
            last_frame_ms: Arc::new(Mutex::new(0)),
            snapshot_ids: Arc::new(Mutex::new(BTreeSet::new())),
            runtime_ids: Arc::new(Mutex::new(BTreeSet::new())),
            runtime: Arc::new(StdMutex::new(crate::runtime::RuntimeInventory::default())),
        }
    }
}

impl OrbService {
    /// Processes, active service cgroups and devices come from local read-only kernel
    /// views. Collection runs off the async executor and never waits on archied.
    pub async fn observe_runtime(&self) {
        let inventory = self.runtime.clone();
        let Ok(entities) = tokio::task::spawn_blocking(move || {
            inventory
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .read()
        })
        .await
        else {
            return;
        };
        let at_ms = archie_types::Timestamp::now().as_millis().max(1);
        let current_ids = entities
            .iter()
            .map(|entity| entity.id.clone())
            .collect::<BTreeSet<_>>();
        let mut model = self.model.lock().await;
        let mut previous_ids = self.runtime_ids.lock().await;
        for id in previous_ids.difference(&current_ids) {
            let _ = model.apply(Signal {
                schema: TFOG_SCHEMA,
                at_ms,
                kind: SignalKind::EntityRemove { id: id.clone() },
            });
        }
        for entity in entities {
            let _ = model.apply(Signal {
                schema: TFOG_SCHEMA,
                at_ms,
                kind: SignalKind::EntityUpsert { entity },
            });
        }
        *previous_ids = current_ids;
    }

    pub async fn current(&self) -> OrbFrame {
        let now = archie_types::Timestamp::now().as_millis();
        let mut last = self.last_frame_ms.lock().await;
        let dt = if *last == 0 {
            0.016
        } else {
            (now.saturating_sub(*last) as f32 / 1000.0).clamp(0.0, 0.1)
        };
        *last = now;
        self.model.lock().await.frame(now, dt)
    }

    /// Feed only bounded, privacy-minimised entities from the existing authoritative
    /// display snapshot. Content used to make feature vectors never leaves this process.
    pub async fn observe_snapshot(&self, snapshot: &DisplaySnapshot) {
        if !snapshot.daemon_reachable || snapshot.source != crate::SnapshotSource::Live {
            return;
        }
        let at_ms = snapshot.generated_at.as_millis().max(1);
        let mut entities = Vec::with_capacity(
            snapshot.modules.len() + snapshot.models.len() + snapshot.nodes.len() + 1,
        );
        let mut system_classes = BTreeMap::new();
        system_classes.insert("kernel".to_string(), 0.5);
        system_classes.insert("service".to_string(), 0.5);
        entities.push(OrbEntity {
            id: "system:archieos".into(),
            parent_id: None,
            related_ids: Vec::new(),
            kind: EntityKind::System,
            token: archie_orb::semantic_features(&format!(
                "{} {} {}",
                snapshot
                    .status
                    .as_ref()
                    .map_or("ArchieOS", |status| status.os.as_str()),
                snapshot
                    .status
                    .as_ref()
                    .map_or("", |status| status.kernel.as_str()),
                snapshot
                    .status
                    .as_ref()
                    .map_or("", |status| status.archieos_version.as_str())
            )),
            class_probs: system_classes,
            size_bytes: snapshot.totals.memory_bytes,
            activity: if snapshot.daemon_reachable { 0.15 } else { 0.0 },
            importance: 1.0,
            uncertainty: if snapshot.daemon_reachable { 0.05 } else { 0.9 },
            runtime: Vec::new(),
            label: "ArchieOS".into(),
        });

        for module in &snapshot.modules {
            let id = format!("module:{}", module.id);
            let mut classes = BTreeMap::new();
            let lower = module.id.to_ascii_lowercase();
            if lower.starts_with("core.") || lower.contains("kernel") {
                classes.insert("kernel".into(), 0.8);
            }
            if lower.contains("driver") {
                classes.insert("driver".into(), 0.8);
            }
            if module.state == "loaded" {
                classes.insert("executable".into(), 0.7);
            }
            if lower.contains("service") || lower.starts_with("core.") {
                classes.insert("service".into(), 0.5);
            }
            let dependencies = module
                .dependencies
                .iter()
                .map(|dependency| format!("module:{dependency}"))
                .take(64)
                .collect::<Vec<_>>();
            let token_text = format!(
                "{} {} {} {}",
                module.id, module.version, module.state, module.explanation
            );
            entities.push(OrbEntity {
                id,
                parent_id: Some("system:archieos".into()),
                related_ids: dependencies,
                kind: EntityKind::Module,
                token: archie_orb::semantic_features(&token_text),
                class_probs: classes,
                size_bytes: 0,
                activity: if module.state == "loaded" { 0.18 } else { 0.0 },
                importance: if module.id.starts_with("core.") {
                    0.9
                } else {
                    0.45
                },
                uncertainty: if module.state == "loaded" { 0.08 } else { 0.4 },
                runtime: Vec::new(),
                label: module.id.chars().take(96).collect(),
            });
        }

        for model in &snapshot.models {
            let id = format!("model:{}", model.id);
            let mut classes = BTreeMap::new();
            classes.insert("model".into(), 1.0);
            entities.push(OrbEntity {
                id,
                parent_id: Some("system:archieos".into()),
                related_ids: Vec::new(),
                kind: EntityKind::Model,
                token: archie_orb::semantic_features(&format!(
                    "{} {} {} {}",
                    model.id, model.provider, model.locality, model.availability
                )),
                class_probs: classes,
                size_bytes: 0,
                activity: if model.availability == "available" {
                    0.3
                } else {
                    0.0
                },
                importance: 0.6,
                uncertainty: if model.availability == "available" {
                    0.1
                } else {
                    0.6
                },
                runtime: vec![model.context_window as f32 / 1_000_000.0],
                label: model.id.chars().take(96).collect(),
            });
        }

        for peer in &snapshot.nodes {
            let id = format!("node:{}", peer.node_id);
            let mut classes = BTreeMap::new();
            classes.insert("device".into(), 1.0);
            entities.push(OrbEntity {
                id,
                parent_id: Some("system:archieos".into()),
                related_ids: Vec::new(),
                kind: EntityKind::Node,
                token: archie_orb::semantic_features(&format!(
                    "{} {} {}",
                    peer.profile,
                    peer.trust,
                    peer.capabilities.join(" ")
                )),
                class_probs: classes,
                size_bytes: 0,
                activity: if peer.reachable { 0.25 } else { 0.0 },
                importance: 0.5,
                uncertainty: if peer.reachable { 0.15 } else { 0.7 },
                runtime: Vec::new(),
                label: "node".into(),
            });
        }

        let current_ids = entities
            .iter()
            .map(|entity| entity.id.clone())
            .collect::<BTreeSet<_>>();
        let mut model = self.model.lock().await;
        let mut previous_ids = self.snapshot_ids.lock().await;
        for id in previous_ids.difference(&current_ids) {
            let _ = model.apply(Signal {
                schema: TFOG_SCHEMA,
                at_ms,
                kind: SignalKind::EntityRemove { id: id.clone() },
            });
        }
        for entity in entities {
            let _ = model.apply(Signal {
                schema: TFOG_SCHEMA,
                at_ms,
                kind: SignalKind::EntityUpsert { entity },
            });
        }
        *previous_ids = current_ids;
        debug_assert!(previous_ids.len() <= TOKEN_DIMENSIONS * 64);
    }

    #[cfg(unix)]
    pub async fn receive(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let receiver = archie_orb::SignalReceiver::bind(path).await?;
        receiver.receive(&self.model).await
    }
}
