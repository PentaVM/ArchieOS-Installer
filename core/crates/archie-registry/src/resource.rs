//! The runtime resource graph.
//!
//! This is the structure that answers, programmatically:
//!
//! * What compute exists? → [`ResourceGraph::totals`]
//! * Where? → [`ResourceGraph::nodes`]
//! * What is currently available? → [`ResourceGraph::schedulable_nodes`]
//! * Which node has capability X? → [`ResourceGraph::providers_of`]
//! * What models fit where? → [`ResourceGraph::nodes_with_accelerator_memory`]
//!
//! # Eligibility is a conjunction
//!
//! A node is schedulable only if *all* of these hold: it is reachable, it reports
//! usable health, and its pinned trust permits it. Any one of them failing removes the
//! node. Combining them in one place stops each caller from inventing its own
//! half-correct version of the check.

use archie_state::TrustState;
use archie_types::{
    Capability, CapabilitySet, LinkClass, NodeDescriptor, NodeId, NodeProfile, Timestamp,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Whether a node is this machine or a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeOrigin {
    Local,
    Peer,
}

/// One node in the graph, with everything needed to decide whether to use it.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeRecord {
    pub descriptor: NodeDescriptor,
    pub origin: NodeOrigin,
    /// Trust as held by *this* node. Never self-asserted by the peer.
    pub trust: TrustState,
    pub reachable: bool,
    pub last_seen: Timestamp,
}

impl NodeRecord {
    /// The local node, which is always reachable and always trusted by itself.
    pub fn local(descriptor: NodeDescriptor) -> Self {
        Self {
            descriptor,
            origin: NodeOrigin::Local,
            trust: TrustState::Trusted,
            reachable: true,
            last_seen: Timestamp::now(),
        }
    }

    pub fn node_id(&self) -> NodeId {
        self.descriptor.node_id
    }

    pub fn is_local(&self) -> bool {
        self.origin == NodeOrigin::Local
    }

    /// Whether work may be placed here at all.
    pub fn schedulable(&self) -> bool {
        self.reachable && self.descriptor.health.schedulable() && self.trust.schedulable()
    }

    /// Why this node is not schedulable, for diagnostics.
    ///
    /// Returns every failing reason rather than the first, so an operator sees the
    /// whole problem instead of fixing one and rediscovering the next.
    pub fn unschedulable_reasons(&self) -> Vec<String> {
        let mut reasons = Vec::new();
        if !self.reachable {
            reasons.push("unreachable".into());
        }
        if !self.descriptor.health.schedulable() {
            reasons.push(format!("health is {}", self.descriptor.health));
        }
        if !self.trust.schedulable() {
            reasons.push(format!("trust is {}", self.trust.as_str()));
        }
        reasons
    }

    /// Whether this node may hold data of a sensitive grade.
    pub fn may_hold_sensitive_data(&self) -> bool {
        self.trust.may_receive_sensitive_data()
            && self.descriptor.profile.may_hold_canonical_state()
    }
}

/// Aggregate compute across the fabric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResourceTotals {
    pub nodes: u32,
    pub schedulable_nodes: u32,
    pub logical_cores: u64,
    pub memory_bytes: u64,
    pub accelerator_bytes: u64,
    pub accelerators: u32,
}

/// The fabric's compute, as this node currently understands it.
#[derive(Debug, Clone, Default)]
pub struct ResourceGraph {
    nodes: BTreeMap<NodeId, NodeRecord>,
}

impl ResourceGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a node.
    pub fn upsert(&mut self, record: NodeRecord) {
        self.nodes.insert(record.node_id(), record);
    }

    pub fn remove(&mut self, node_id: &NodeId) -> Option<NodeRecord> {
        self.nodes.remove(node_id)
    }

    pub fn get(&self, node_id: &NodeId) -> Option<&NodeRecord> {
        self.nodes.get(node_id)
    }

    /// Every node, ordered by id so output is stable between runs.
    pub fn nodes(&self) -> impl Iterator<Item = &NodeRecord> {
        self.nodes.values()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn local(&self) -> Option<&NodeRecord> {
        self.nodes.values().find(|n| n.is_local())
    }

    pub fn schedulable_nodes(&self) -> impl Iterator<Item = &NodeRecord> {
        self.nodes.values().filter(|n| n.schedulable())
    }

    /// Schedulable nodes providing `capability`.
    pub fn providers_of(&self, capability: &Capability) -> Vec<&NodeRecord> {
        self.nodes
            .values()
            .filter(|n| n.schedulable() && n.descriptor.capabilities.provides(capability))
            .collect()
    }

    /// Every node providing `capability`, schedulable or not.
    ///
    /// Distinct from [`Self::providers_of`] on purpose: "nobody has a GPU" and "the
    /// only GPU node is untrusted" are different problems and need different fixes.
    pub fn all_providers_of(&self, capability: &Capability) -> Vec<&NodeRecord> {
        self.nodes
            .values()
            .filter(|n| n.descriptor.capabilities.provides(capability))
            .collect()
    }

    /// Schedulable nodes providing every capability in `required`.
    pub fn providers_of_all(&self, required: &[Capability]) -> Vec<&NodeRecord> {
        self.nodes
            .values()
            .filter(|n| {
                n.schedulable()
                    && required
                        .iter()
                        .all(|cap| n.descriptor.capabilities.provides(cap))
            })
            .collect()
    }

    /// Schedulable nodes with at least `bytes` of accelerator memory.
    ///
    /// Nodes whose accelerator memory is unknown are excluded: placing a model on a
    /// device that might not fit it is worse than not placing it.
    pub fn nodes_with_accelerator_memory(&self, bytes: u64) -> Vec<&NodeRecord> {
        self.nodes
            .values()
            .filter(|n| n.schedulable() && n.descriptor.total_accelerator_memory_bytes() >= bytes)
            .collect()
    }

    /// The union of every capability any schedulable node provides.
    pub fn available_capabilities(&self) -> CapabilitySet {
        self.schedulable_nodes()
            .flat_map(|n| n.descriptor.capabilities.iter().cloned())
            .collect()
    }

    /// capability → the nodes providing it. Ordered for stable rendering.
    pub fn capability_index(&self) -> BTreeMap<Capability, Vec<NodeId>> {
        let mut index: BTreeMap<Capability, Vec<NodeId>> = BTreeMap::new();
        for node in self.nodes.values() {
            for capability in node.descriptor.capabilities.iter() {
                index
                    .entry(capability.clone())
                    .or_default()
                    .push(node.node_id());
            }
        }
        for ids in index.values_mut() {
            ids.sort_by_key(|id| id.to_string());
        }
        index
    }

    /// Aggregate compute over schedulable nodes only.
    ///
    /// Counting unreachable nodes would report capacity Archie cannot actually use.
    pub fn totals(&self) -> ResourceTotals {
        let mut totals = ResourceTotals {
            nodes: self.nodes.len() as u32,
            ..Default::default()
        };
        for node in self.schedulable_nodes() {
            totals.schedulable_nodes += 1;
            totals.logical_cores += node.descriptor.cpu.logical_cores as u64;
            totals.memory_bytes += node.descriptor.memory.total_bytes;
            totals.accelerator_bytes += node.descriptor.total_accelerator_memory_bytes();
            totals.accelerators += node.descriptor.accelerators.len() as u32;
        }
        totals
    }

    /// Every eligible node for a workload, best first.
    ///
    /// Preference order: local first (no network cost), then most free memory, then id
    /// as a tiebreak. Determinism matters more than optimality here — an unpredictable
    /// placement makes a performance problem impossible to reproduce.
    ///
    /// Callers that track occupancy (the scheduler) walk this list and take the first
    /// node with room, which is what lets work spread across the fabric instead of
    /// queueing on whichever node merely ranks highest.
    pub fn rank_nodes(
        &self,
        required: &[Capability],
        min_accelerator_bytes: Option<u64>,
    ) -> Vec<&NodeRecord> {
        let mut candidates: Vec<&NodeRecord> = self
            .providers_of_all(required)
            .into_iter()
            .filter(|n| match min_accelerator_bytes {
                Some(bytes) => n.descriptor.total_accelerator_memory_bytes() >= bytes,
                None => true,
            })
            .collect();

        candidates.sort_by(|a, b| {
            b.is_local()
                .cmp(&a.is_local())
                .then(
                    b.descriptor
                        .memory
                        .available_bytes
                        .cmp(&a.descriptor.memory.available_bytes),
                )
                .then(a.node_id().to_string().cmp(&b.node_id().to_string()))
        });
        candidates
    }

    /// The single best node for a workload, ignoring occupancy.
    pub fn select_node(
        &self,
        required: &[Capability],
        min_accelerator_bytes: Option<u64>,
    ) -> Option<&NodeRecord> {
        self.rank_nodes(required, min_accelerator_bytes)
            .into_iter()
            .next()
    }

    /// Whether a model may be split across `a` and `b`.
    ///
    /// Tensor-parallel execution across a slow link is a design error, not a tuning
    /// problem: it turns every layer boundary into a network round trip.
    pub fn permits_tensor_parallel(&self, a: &NodeId, b: &NodeId) -> bool {
        if a == b {
            return true;
        }
        let (Some(a), Some(b)) = (self.nodes.get(a), self.nodes.get(b)) else {
            return false;
        };
        a.descriptor.best_link_class().permits_tensor_parallel()
            && b.descriptor.best_link_class().permits_tensor_parallel()
    }

    /// The worst link class among schedulable nodes, which bounds what placement
    /// strategies are sound fabric-wide.
    pub fn worst_link_class(&self) -> LinkClass {
        self.schedulable_nodes()
            .map(|n| n.descriptor.best_link_class())
            .max()
            .unwrap_or(LinkClass::Loopback)
    }

    /// Node ids present here but not in `previous`.
    pub fn appeared_since(&self, previous: &BTreeSet<NodeId>) -> Vec<NodeId> {
        self.nodes
            .keys()
            .filter(|id| !previous.contains(id))
            .copied()
            .collect()
    }

    /// Node ids in `previous` that are gone now.
    pub fn disappeared_since(&self, previous: &BTreeSet<NodeId>) -> Vec<NodeId> {
        previous
            .iter()
            .filter(|id| !self.nodes.contains_key(id))
            .copied()
            .collect()
    }

    pub fn node_ids(&self) -> BTreeSet<NodeId> {
        self.nodes.keys().copied().collect()
    }

    /// Nodes by profile, for operator-facing summaries.
    pub fn count_by_profile(&self) -> BTreeMap<String, u32> {
        let mut counts = BTreeMap::new();
        for node in self.nodes.values() {
            *counts
                .entry(node.descriptor.profile.as_str().to_string())
                .or_insert(0) += 1;
        }
        counts
    }

    /// Whether any node in the graph can hold canonical state.
    ///
    /// A fabric of nothing but dev VMs has nowhere durable to put knowledge, and should
    /// say so rather than silently writing to a disposable node.
    pub fn has_canonical_state_holder(&self) -> bool {
        self.nodes
            .values()
            .any(|n| n.schedulable() && n.descriptor.profile.may_hold_canonical_state())
    }

    /// Profiles present, for the boot summary.
    pub fn profiles(&self) -> BTreeSet<NodeProfile> {
        self.nodes.values().map(|n| n.descriptor.profile).collect()
    }
}
