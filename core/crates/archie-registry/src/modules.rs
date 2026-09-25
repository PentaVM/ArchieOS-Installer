//! The module registry and the cognitive topology compiler.
//!
//! # The intersection
//!
//! At boot, what actually runs is
//!
//! ```text
//! available capabilities ∩ valid manifests ∩ granted permissions
//!                        ∩ node profile     ∩ satisfied dependencies
//! ```
//!
//! [`ModuleRegistry::compile_topology`] computes exactly that and, critically, records
//! *why* each excluded module was excluded. A module that silently fails to load is a
//! module nobody will ever debug.
//!
//! Modules may be installed without being loaded. The registry is expected to grow far
//! larger than any one node's loadable set.

use archie_types::{
    Capability, CapabilitySet, DeviceAdapterManifest, DeviceAdapterOrigin, ModuleId,
    ModuleManifest, ModulePermission, ModuleState, WorkerImpl,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModuleRegistryError {
    #[error("cannot read module directory {path}: {source}")]
    Directory {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("module manifest {path} is not valid TOML: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },

    #[error("duplicate module id {0}")]
    Duplicate(ModuleId),
}

/// What the node is willing to grant modules.
#[derive(Debug, Clone, Default)]
pub struct PermissionPolicy {
    granted: BTreeSet<ModulePermission>,
}

impl PermissionPolicy {
    /// Grants nothing. Every module requesting any permission is denied.
    pub fn deny_all() -> Self {
        Self::default()
    }

    /// The permissions a node grants by default: read-only, non-physical ones.
    ///
    /// Anything that writes knowledge, reads secrets, or touches hardware must be
    /// granted deliberately by the operator.
    pub fn default_safe() -> Self {
        Self {
            granted: [
                ModulePermission::ReadKnowledge,
                ModulePermission::ReadTelemetry,
                ModulePermission::NetworkLan,
                ModulePermission::SpawnWorkers,
            ]
            .into_iter()
            .collect(),
        }
    }

    pub fn grant(mut self, permission: ModulePermission) -> Self {
        self.granted.insert(permission);
        self
    }

    pub fn allows(&self, permission: &ModulePermission) -> bool {
        self.granted.contains(permission)
    }

    /// Permissions a module wants that this policy does not grant.
    pub fn denied_for(&self, manifest: &ModuleManifest) -> Vec<ModulePermission> {
        manifest
            .permissions
            .iter()
            .filter(|p| !self.allows(p))
            .copied()
            .collect()
    }
}

/// The environment a topology is compiled against.
#[derive(Debug, Clone)]
pub struct TopologyContext {
    pub capabilities: CapabilitySet,
    pub architecture: String,
    pub profile: String,
    pub permissions: PermissionPolicy,
    /// Modules the operator has explicitly turned off.
    pub disabled: BTreeSet<ModuleId>,
}

impl TopologyContext {
    pub fn new(
        capabilities: CapabilitySet,
        architecture: impl Into<String>,
        profile: impl Into<String>,
    ) -> Self {
        Self {
            capabilities,
            architecture: architecture.into(),
            profile: profile.into(),
            permissions: PermissionPolicy::default_safe(),
            disabled: BTreeSet::new(),
        }
    }

    pub fn with_permissions(mut self, permissions: PermissionPolicy) -> Self {
        self.permissions = permissions;
        self
    }

    pub fn disable(mut self, id: ModuleId) -> Self {
        self.disabled.insert(id);
        self
    }
}

/// The compiled result: what loads, what does not, and in what order.
#[derive(Debug, Clone, PartialEq)]
pub struct Topology {
    /// Every known module and its resolved state.
    pub states: BTreeMap<ModuleId, ModuleState>,
    /// Loadable modules in dependency-then-priority order.
    pub load_order: Vec<ModuleId>,
    /// Modules involved in a dependency cycle, if any.
    pub cycles: Vec<ModuleId>,
}

impl Topology {
    pub fn loaded(&self) -> Vec<&ModuleId> {
        self.load_order.iter().collect()
    }

    pub fn is_loaded(&self, id: &ModuleId) -> bool {
        self.states.get(id).is_some_and(|s| s.is_loaded())
    }

    pub fn loaded_count(&self) -> usize {
        self.load_order.len()
    }

    /// Modules that did not load, with an explanation each.
    pub fn excluded(&self) -> Vec<(&ModuleId, String)> {
        self.states
            .iter()
            .filter(|(_, state)| !state.is_loaded())
            .map(|(id, state)| (id, state.explain()))
            .collect()
    }
}

/// Installed module manifests.
#[derive(Debug, Clone, Default)]
pub struct ModuleRegistry {
    manifests: BTreeMap<ModuleId, ModuleManifest>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a manifest, rejecting a duplicate id.
    pub fn register(&mut self, manifest: ModuleManifest) -> Result<(), ModuleRegistryError> {
        if self.manifests.contains_key(&manifest.id) {
            return Err(ModuleRegistryError::Duplicate(manifest.id.clone()));
        }
        self.manifests.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    /// Load every `*.toml` manifest under `dir`, including subdirectories.
    ///
    /// The module tree is grouped by family on disk (`core/`, `cognition/`, `science/`,
    /// `engineering/`, `devices/`), so the walk has to descend. Grouping is presentation
    /// only — a module's identity is its `id`, never its path — which is why manifests
    /// from different directories share one flat namespace and a duplicate `id` across
    /// families is still an error.
    ///
    /// A missing directory yields an empty registry rather than an error: a node with
    /// no modules installed is a valid, if quiet, node. A malformed manifest *is* an
    /// error — silently skipping it would make a typo look like a missing feature.
    pub fn load_from_dir(dir: impl AsRef<Path>) -> Result<Self, ModuleRegistryError> {
        let dir = dir.as_ref();
        let mut registry = Self::new();
        if !dir.exists() {
            return Ok(registry);
        }

        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        collect_manifests(dir, &mut paths, 0)?;
        // Sorted so that a duplicate-id error names the same file every run, whatever
        // order the filesystem happened to return.
        paths.sort();

        for path in paths {
            let text = std::fs::read_to_string(&path).map_err(|source| {
                ModuleRegistryError::Directory {
                    path: path.display().to_string(),
                    source,
                }
            })?;
            let manifest: ModuleManifest =
                toml::from_str(&text).map_err(|source| ModuleRegistryError::Parse {
                    path: path.display().to_string(),
                    source,
                })?;
            registry.register(manifest)?;
        }
        Ok(registry)
    }

    pub fn get(&self, id: &ModuleId) -> Option<&ModuleManifest> {
        self.manifests.get(id)
    }

    pub fn manifests(&self) -> impl Iterator<Item = &ModuleManifest> {
        self.manifests.values()
    }

    pub fn len(&self) -> usize {
        self.manifests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.manifests.is_empty()
    }

    /// Resolve which modules load on this node, and why the rest do not.
    pub fn compile_topology(&self, context: &TopologyContext) -> Topology {
        let mut states: BTreeMap<ModuleId, ModuleState> = BTreeMap::new();

        // Pass 1: everything decidable without looking at other modules.
        for manifest in self.manifests.values() {
            let state = self.evaluate_standalone(manifest, context);
            states.insert(manifest.id.clone(), state);
        }

        // Pass 2: dependencies. A module can only load if every dependency is installed
        // *and* itself loaded, so this iterates to a fixpoint — one pass would let a
        // module load behind a dependency that fails later in the same pass.
        loop {
            let mut changed = false;
            let currently_loaded: HashSet<ModuleId> = states
                .iter()
                .filter(|(_, s)| s.is_loaded())
                .map(|(id, _)| id.clone())
                .collect();

            for manifest in self.manifests.values() {
                if !states.get(&manifest.id).is_some_and(|s| s.is_loaded()) {
                    continue;
                }
                let missing: Vec<ModuleId> = manifest
                    .dependencies
                    .iter()
                    .filter(|dep| !currently_loaded.contains(dep))
                    .cloned()
                    .collect();
                if !missing.is_empty() {
                    states.insert(
                        manifest.id.clone(),
                        ModuleState::MissingDependency { missing },
                    );
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        let cycles = self.find_dependency_cycle().unwrap_or_default();
        for id in &cycles {
            states.insert(
                id.clone(),
                ModuleState::Invalid {
                    reason: "participates in a dependency cycle".into(),
                },
            );
        }

        let load_order = self.compute_load_order(&states);

        Topology {
            states,
            load_order,
            cycles,
        }
    }

    /// Everything that can be decided from one manifest plus the node's environment.
    fn evaluate_standalone(
        &self,
        manifest: &ModuleManifest,
        context: &TopologyContext,
    ) -> ModuleState {
        if let Err(e) = manifest.validate() {
            return ModuleState::Invalid {
                reason: e.to_string(),
            };
        }
        if context.disabled.contains(&manifest.id) {
            return ModuleState::Disabled;
        }
        if !manifest.supports_architecture(&context.architecture) {
            return ModuleState::UnsupportedArchitecture {
                architecture: context.architecture.clone(),
            };
        }
        if !manifest.supports_profile(&context.profile) {
            return ModuleState::NotInProfile {
                profile: context.profile.clone(),
            };
        }

        let denied = context.permissions.denied_for(manifest);
        if !denied.is_empty() {
            return ModuleState::PermissionDenied {
                reason: format!(
                    "not granted: {}",
                    denied
                        .iter()
                        .map(|p| format!("{p:?}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
        }

        let missing: Vec<Capability> = manifest.missing_capabilities(&context.capabilities);
        if !missing.is_empty() {
            return ModuleState::UnsatisfiedCapabilities { missing };
        }

        // Dependencies that are not even installed are decidable here.
        let uninstalled: Vec<ModuleId> = manifest
            .dependencies
            .iter()
            .filter(|dep| !self.manifests.contains_key(dep))
            .cloned()
            .collect();
        if !uninstalled.is_empty() {
            return ModuleState::MissingDependency {
                missing: uninstalled,
            };
        }

        // Last, so that what this node lacks is said first: a module that could not run
        // here even once written should say why here, not only that it is unwritten.
        if let WorkerImpl::Unimplemented { reason } = &manifest.worker {
            return ModuleState::NotImplemented {
                reason: reason.clone(),
            };
        }

        ModuleState::Loaded
    }

    /// Topologically order the loadable modules.
    ///
    /// Dependencies always precede dependents; ties break on the manifest's `priority`,
    /// then on id. The result is deterministic, which matters because a topology that
    /// loads in a different order each boot is untestable.
    fn compute_load_order(&self, states: &BTreeMap<ModuleId, ModuleState>) -> Vec<ModuleId> {
        let loadable: BTreeSet<ModuleId> = states
            .iter()
            .filter(|(_, s)| s.is_loaded())
            .map(|(id, _)| id.clone())
            .collect();

        let mut ordered: Vec<ModuleId> = Vec::new();
        let mut placed: HashSet<ModuleId> = HashSet::new();

        // Candidates sorted by (priority, id) so each round picks deterministically.
        let mut candidates: Vec<&ModuleManifest> = loadable
            .iter()
            .filter_map(|id| self.manifests.get(id))
            .collect();
        candidates.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.id.cmp(&b.id)));

        // Repeatedly take every module whose dependencies are already placed.
        loop {
            let mut progressed = false;
            for manifest in &candidates {
                if placed.contains(&manifest.id) {
                    continue;
                }
                let ready = manifest
                    .dependencies
                    .iter()
                    .all(|dep| !loadable.contains(dep) || placed.contains(dep));
                if ready {
                    ordered.push(manifest.id.clone());
                    placed.insert(manifest.id.clone());
                    progressed = true;
                }
            }
            if !progressed {
                break;
            }
        }
        ordered
    }

    /// Detect a dependency cycle among installed modules.
    fn find_dependency_cycle(&self) -> Option<Vec<ModuleId>> {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            Visiting,
            Done,
        }
        let mut marks: BTreeMap<ModuleId, Mark> = BTreeMap::new();
        let mut stack: Vec<ModuleId> = Vec::new();

        for root in self.manifests.keys() {
            if marks.get(root) == Some(&Mark::Done) {
                continue;
            }
            let mut work: Vec<(ModuleId, bool)> = vec![(root.clone(), false)];
            while let Some((current, expanded)) = work.pop() {
                if expanded {
                    marks.insert(current, Mark::Done);
                    stack.pop();
                    continue;
                }
                match marks.get(&current) {
                    Some(Mark::Done) => continue,
                    Some(Mark::Visiting) => {
                        let start = stack.iter().position(|m| *m == current).unwrap_or(0);
                        let mut cycle = stack[start..].to_vec();
                        cycle.push(current);
                        cycle.sort();
                        cycle.dedup();
                        return Some(cycle);
                    }
                    None => {}
                }
                marks.insert(current.clone(), Mark::Visiting);
                stack.push(current.clone());
                work.push((current.clone(), true));
                if let Some(manifest) = self.manifests.get(&current) {
                    for dep in &manifest.dependencies {
                        if self.manifests.contains_key(dep) {
                            work.push((dep.clone(), false));
                        }
                    }
                }
            }
        }
        None
    }
}

/// Errors from the local, declarative device-adapter catalogue.
///
/// The catalogue does not load a worker, pair a target, or grant any device permission.
/// It is deliberately separate from topology compilation so that recording a generated
/// proposal cannot make it eligible to run.
#[derive(Debug, Error)]
pub enum DeviceAdapterRegistryError {
    #[error("device adapter declaration is invalid: {0}")]
    Invalid(#[from] archie_types::TypeError),

    #[error("device adapter id {0} is already recorded")]
    Duplicate(String),

    #[error("generated-proposal submission requires a generated proposal")]
    ProposalExpected,

    #[error("native-contract admission requires an ISO-built contract")]
    NativeContractExpected,

    #[error("device adapter hosting module {0} is not installed")]
    MissingHostingModule(ModuleId),
}

/// Local catalogue of adapter declarations.
///
/// Generated proposals and reviewed native contracts live in distinct maps by design.
/// Neither is a connection, a worker process, a paired target, or a source of authority.
/// A later activation pipeline must separately satisfy topology permissions, pairing,
/// one-operation policy/lease validation, dispatch and receipt reconciliation.
#[derive(Debug, Clone, Default)]
pub struct DeviceAdapterRegistry {
    generated_proposals: BTreeMap<String, DeviceAdapterManifest>,
    admitted_native_contracts: BTreeMap<String, DeviceAdapterManifest>,
}

impl DeviceAdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an automatically generated proposal for review. It stays non-executable.
    pub fn submit_generated_proposal(
        &mut self,
        manifest: DeviceAdapterManifest,
    ) -> Result<(), DeviceAdapterRegistryError> {
        manifest.validate()?;
        if manifest.origin != DeviceAdapterOrigin::GeneratedProposal {
            return Err(DeviceAdapterRegistryError::ProposalExpected);
        }
        self.ensure_available(&manifest.adapter_id)?;
        self.generated_proposals
            .insert(manifest.adapter_id.clone(), manifest);
        Ok(())
    }

    /// Record a reviewed native contract whose hosting module is actually installed.
    ///
    /// Admission validates the strict native module contract but intentionally does not
    /// compile topology or launch a worker. A physical device still cannot be reached
    /// from this method.
    pub fn admit_native_contract(
        &mut self,
        manifest: DeviceAdapterManifest,
        modules: &ModuleRegistry,
    ) -> Result<(), DeviceAdapterRegistryError> {
        if manifest.origin != DeviceAdapterOrigin::BuiltIntoIso {
            return Err(DeviceAdapterRegistryError::NativeContractExpected);
        }
        let module = modules.get(&manifest.module_id).ok_or_else(|| {
            DeviceAdapterRegistryError::MissingHostingModule(manifest.module_id.clone())
        })?;
        manifest.validate_activation(module)?;
        self.ensure_available(&manifest.adapter_id)?;
        self.admitted_native_contracts
            .insert(manifest.adapter_id.clone(), manifest);
        Ok(())
    }

    /// Inactive auto-generated declarations awaiting native implementation review.
    pub fn generated_proposals(&self) -> impl Iterator<Item = &DeviceAdapterManifest> {
        self.generated_proposals.values()
    }

    /// Reviewed contracts that may proceed to the later, still gated pairing pipeline.
    pub fn admitted_native_contracts(&self) -> impl Iterator<Item = &DeviceAdapterManifest> {
        self.admitted_native_contracts.values()
    }

    pub fn len(&self) -> usize {
        self.generated_proposals.len() + self.admitted_native_contracts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn ensure_available(&self, adapter_id: &str) -> Result<(), DeviceAdapterRegistryError> {
        if self.generated_proposals.contains_key(adapter_id)
            || self.admitted_native_contracts.contains_key(adapter_id)
        {
            return Err(DeviceAdapterRegistryError::Duplicate(
                adapter_id.to_string(),
            ));
        }
        Ok(())
    }
}

/// Depth-limited recursive walk for `*.toml` manifests.
///
/// The depth cap is deliberate: a symlink loop under `/etc/archie/modules` would
/// otherwise hang the boot sequence rather than failing it.
fn collect_manifests(
    dir: &Path,
    out: &mut Vec<std::path::PathBuf>,
    depth: usize,
) -> Result<(), ModuleRegistryError> {
    const MAX_DEPTH: usize = 8;
    if depth > MAX_DEPTH {
        return Ok(());
    }

    let entries = std::fs::read_dir(dir).map_err(|source| ModuleRegistryError::Directory {
        path: dir.display().to_string(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| ModuleRegistryError::Directory {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| ModuleRegistryError::Directory {
                path: path.display().to_string(),
                source,
            })?;

        if file_type.is_dir() {
            collect_manifests(&path, out, depth + 1)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            out.push(path);
        }
    }
    Ok(())
}
