//! Module manifests.
//!
//! A module is a declarative unit of Archie functionality. Modules may be *installed*
//! without being *loaded*: the module database is expected to become large, and most of
//! it will be irrelevant on any given node. What loads is the intersection
//!
//! ```text
//! available capabilities ∩ trusted modules ∩ granted permissions ∩ profile requirements
//! ```
//!
//! computed at boot by the cognitive topology compiler.

use crate::capability::{Capability, CapabilitySet};
use crate::error::{Result, TypeError};
use crate::ids::ModuleId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Why a module was or was not loaded. Returned by the topology compiler so that
/// `archie modules` can explain every decision instead of just listing survivors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModuleState {
    /// Loaded and eligible to run.
    Loaded,
    /// Present and valid, but the node lacks required capabilities.
    UnsatisfiedCapabilities { missing: Vec<Capability> },
    /// A declared dependency is not installed.
    MissingDependency { missing: Vec<ModuleId> },
    /// Installed but not permitted to run on this node.
    PermissionDenied { reason: String },
    /// Excluded by the node profile.
    NotInProfile { profile: String },
    /// Architecture mismatch.
    UnsupportedArchitecture { architecture: String },
    /// Explicitly disabled by configuration.
    Disabled,
    /// Manifest failed validation.
    Invalid { reason: String },
}

impl ModuleState {
    pub fn is_loaded(&self) -> bool {
        matches!(self, Self::Loaded)
    }

    /// One-word status for CLI tables.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Loaded => "loaded",
            Self::UnsatisfiedCapabilities { .. } => "unsatisfied",
            Self::MissingDependency { .. } => "missing-dep",
            Self::PermissionDenied { .. } => "denied",
            Self::NotInProfile { .. } => "not-in-profile",
            Self::UnsupportedArchitecture { .. } => "wrong-arch",
            Self::Disabled => "disabled",
            Self::Invalid { .. } => "invalid",
        }
    }

    /// Human-readable explanation of the state.
    pub fn explain(&self) -> String {
        match self {
            Self::Loaded => "loaded".into(),
            Self::UnsatisfiedCapabilities { missing } => format!(
                "node does not provide: {}",
                missing
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::MissingDependency { missing } => format!(
                "requires modules not installed: {}",
                missing
                    .iter()
                    .map(|m| m.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::PermissionDenied { reason } => format!("permission denied: {reason}"),
            Self::NotInProfile { profile } => format!("not enabled for profile {profile}"),
            Self::UnsupportedArchitecture { architecture } => {
                format!("does not support architecture {architecture}")
            }
            Self::Disabled => "disabled by configuration".into(),
            Self::Invalid { reason } => format!("invalid manifest: {reason}"),
        }
    }
}

/// A permission a module requests. Permissions are coarse and human-auditable on
/// purpose; fine-grained enforcement happens through capability leases at call time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModulePermission {
    ReadKnowledge,
    WriteKnowledge,
    NetworkInternet,
    NetworkLan,
    SpawnWorkers,
    ReadTelemetry,
    /// Talk to physical devices. Never implies actuation.
    DeviceCommunicate,
    /// Command actuators. Requires explicit human authorization at install time and
    /// is refused outright unless the safety boundary is present.
    DeviceActuate,
    ManageModels,
    ReadSecrets,
}

impl ModulePermission {
    /// Whether granting this permission requires an explicit human decision.
    pub fn requires_human_authorization(&self) -> bool {
        matches!(
            self,
            Self::DeviceActuate | Self::ReadSecrets | Self::WriteKnowledge
        )
    }

    /// Whether this permission touches the physical world.
    pub fn is_physical(&self) -> bool {
        matches!(self, Self::DeviceActuate | Self::DeviceCommunicate)
    }
}

/// How a module's worker is executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WorkerImpl {
    /// Compiled into `archied`. Only control-plane modules may use this.
    Builtin { symbol: String },
    /// A Python worker from `python/workers/`.
    Python { module: String, entrypoint: String },
    /// An external process.
    Process { command: String, args: Vec<String> },
    /// Declarative only; the module contributes configuration, not execution.
    None,
}

impl WorkerImpl {
    /// Builtin workers run inside the trusted daemon, so a module that is not part of
    /// the control plane must never declare one.
    pub fn is_trusted(&self) -> bool {
        matches!(self, Self::Builtin { .. })
    }
}

/// A fixed family of physical devices an adapter can address.
///
/// This deliberately matches the locally pairable device families without accepting a
/// free-form "firmware" label. Supporting a new family is a reviewed ArchieOS change,
/// not something a network peer or generated plugin can name into existence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceAdapterClass {
    Television,
    Router,
    Camera,
    Display,
    Mobile,
    Host,
    Printer,
    HomeAssistant,
}

impl DeviceAdapterClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Television => "television",
            Self::Router => "router",
            Self::Camera => "camera",
            Self::Display => "display",
            Self::Mobile => "mobile",
            Self::Host => "host",
            Self::Printer => "printer",
            Self::HomeAssistant => "home-assistant",
        }
    }
}

/// The entire operation vocabulary a native device adapter may declare.
///
/// There is intentionally no `Command`, `Shell`, `Upload`, or arbitrary protocol-frame
/// variant. The reviewed, typed adapter owns its wire encoding; its manifest can only
/// advertise these bounded behaviours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceAdapterOperation {
    ObserveState,
    ApplyDesiredState,
    FirmwareInspect,
    FirmwareStage,
    FirmwareApply,
    FirmwareRollback,
}

impl DeviceAdapterOperation {
    pub fn requires_actuation(&self) -> bool {
        !matches!(self, Self::ObserveState | Self::FirmwareInspect)
    }
}

/// Where an adapter declaration came from.
///
/// A generated proposal is useful to give an operator a concrete, reviewable scaffold,
/// but it is deliberately not executable. Only a reviewed implementation built into the
/// ISO can pass activation validation. Package signing and installed-adapter discovery
/// are later integration work; this type makes their absence fail closed today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceAdapterOrigin {
    BuiltIntoIso,
    GeneratedProposal,
}

/// A declared resource ceiling for an adapter process or its future sandbox.
///
/// The device does not host a model runtime. These values describe the native adapter's
/// local envelope and are intentionally small enough to be auditable before a resource
/// governor enforces them at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAdapterResourceBudget {
    pub max_resident_memory_mib: u16,
    pub max_cpu_millis_per_second: u16,
    pub max_gpu_memory_mib: u16,
}

/// A companion declaration for a physical-device module.
///
/// It contains no endpoint, hostname, credential, firmware image, raw payload, shell
/// command or executable path. Pairing identifies a target separately, and a durable
/// prepared operation plus a one-operation lease authorize one typed dispatch later.
/// This keeps a generated scaffold from becoming a universal remote-control plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAdapterManifest {
    pub adapter_id: String,
    pub module_id: ModuleId,
    pub origin: DeviceAdapterOrigin,
    pub supported_classes: Vec<DeviceAdapterClass>,
    pub operations: Vec<DeviceAdapterOperation>,
    pub resource_budget: DeviceAdapterResourceBudget,
    /// Every mutating operation must be able to produce a non-I/O preview first.
    pub dry_run_required: bool,
    /// Every mutating operation must reconcile a typed receipt with observed state.
    pub receipt_required: bool,
}

impl DeviceAdapterManifest {
    /// Produce a non-executable adapter proposal from typed requirements.
    ///
    /// This is the only automatic generation supported by the base contract: it creates
    /// something an operator can inspect and a native implementer can satisfy. It does
    /// not discover a target, load code, open a transport or grant any permission.
    pub fn generated_proposal(
        adapter_id: String,
        module_id: ModuleId,
        supported_classes: Vec<DeviceAdapterClass>,
        operations: Vec<DeviceAdapterOperation>,
        resource_budget: DeviceAdapterResourceBudget,
    ) -> Result<Self> {
        let mutates = operations
            .iter()
            .any(DeviceAdapterOperation::requires_actuation);
        let proposal = Self {
            adapter_id,
            module_id,
            origin: DeviceAdapterOrigin::GeneratedProposal,
            supported_classes,
            operations,
            resource_budget,
            dry_run_required: mutates,
            receipt_required: mutates,
        };
        proposal.validate()?;
        Ok(proposal)
    }

    /// Validate only the declaration itself. This is safe for a generated proposal and
    /// intentionally does not make it loadable or able to reach a device.
    pub fn validate(&self) -> Result<()> {
        if !is_adapter_identifier(&self.adapter_id) {
            return Err(TypeError::InvalidManifest(
                "adapter_id must be lower-case ASCII words separated by '-'".into(),
            ));
        }
        if self.supported_classes.is_empty() {
            return Err(TypeError::InvalidManifest(
                "a device adapter must declare at least one supported class".into(),
            ));
        }
        if has_duplicates(&self.supported_classes) {
            return Err(TypeError::InvalidManifest(
                "supported device classes must not repeat".into(),
            ));
        }
        if self.operations.is_empty() {
            return Err(TypeError::InvalidManifest(
                "a device adapter must declare at least one operation".into(),
            ));
        }
        if has_duplicates(&self.operations) {
            return Err(TypeError::InvalidManifest(
                "device adapter operations must not repeat".into(),
            ));
        }
        if self.resource_budget.max_resident_memory_mib == 0
            || self.resource_budget.max_resident_memory_mib > 1024
        {
            return Err(TypeError::InvalidManifest(
                "adapter resident-memory budget must be between 1 and 1024 MiB".into(),
            ));
        }
        if self.resource_budget.max_cpu_millis_per_second == 0
            || self.resource_budget.max_cpu_millis_per_second > 1000
        {
            return Err(TypeError::InvalidManifest(
                "adapter CPU budget must be between 1 and 1000 milliseconds per second".into(),
            ));
        }

        let firmware_apply = self
            .operations
            .contains(&DeviceAdapterOperation::FirmwareApply);
        if firmware_apply
            && ![
                DeviceAdapterOperation::FirmwareInspect,
                DeviceAdapterOperation::FirmwareStage,
                DeviceAdapterOperation::FirmwareRollback,
            ]
            .iter()
            .all(|required| self.operations.contains(required))
        {
            return Err(TypeError::InvalidManifest(
                "firmware-apply requires firmware-inspect, firmware-stage and firmware-rollback"
                    .into(),
            ));
        }

        if self
            .operations
            .iter()
            .any(DeviceAdapterOperation::requires_actuation)
            && (!self.dry_run_required || !self.receipt_required)
        {
            return Err(TypeError::InvalidManifest(
                "mutating device adapters require dry-run and receipt reconciliation".into(),
            ));
        }
        Ok(())
    }

    /// Whether this declaration could ever be activated after a later package and
    /// pairing check. Generated code is never eligible by declaration alone.
    pub fn is_activation_eligible(&self) -> bool {
        matches!(self.origin, DeviceAdapterOrigin::BuiltIntoIso)
    }

    /// Validate this declaration against the module that would host it.
    ///
    /// This is intentionally stricter than generic module validation: physical adapters
    /// must be native ISO code rather than Python or an arbitrary external process, must
    /// communicate explicitly, and must request actuation only when their fixed
    /// vocabulary includes a mutation. It still does not authorize a target or open I/O.
    pub fn validate_activation(&self, module: &ModuleManifest) -> Result<()> {
        self.validate()?;
        module.validate()?;

        if !self.is_activation_eligible() {
            return Err(TypeError::InvalidManifest(
                "a generated device-adapter proposal cannot be activated".into(),
            ));
        }
        if self.module_id != module.id {
            return Err(TypeError::InvalidManifest(
                "device adapter module_id does not match its hosting module".into(),
            ));
        }
        if !matches!(module.worker, WorkerImpl::Builtin { .. }) {
            return Err(TypeError::InvalidManifest(
                "active device adapters must be native code built into the ISO".into(),
            ));
        }
        if module.config_schema.is_some() || !module.labels.is_empty() {
            return Err(TypeError::InvalidManifest(
                "active device adapters must obtain target details only through pairing, not generic module configuration or labels".into(),
            ));
        }
        if !module
            .permissions
            .contains(&ModulePermission::DeviceCommunicate)
        {
            return Err(TypeError::InvalidManifest(
                "device adapter requires device-communicate".into(),
            ));
        }
        if self
            .operations
            .iter()
            .any(DeviceAdapterOperation::requires_actuation)
            && !module
                .permissions
                .contains(&ModulePermission::DeviceActuate)
        {
            return Err(TypeError::InvalidManifest(
                "mutating device adapter requires device-actuate".into(),
            ));
        }
        Ok(())
    }
}

fn is_adapter_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn has_duplicates<T: Ord + Copy>(values: &[T]) -> bool {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted.windows(2).any(|pair| pair[0] == pair[1])
}

/// How to tell whether a loaded module is healthy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum HealthCheck {
    /// The worker is expected to report in at least this often.
    Heartbeat { interval_seconds: u32 },
    /// Run a command; exit status zero means healthy.
    Command { command: String, args: Vec<String> },
    /// No health signal. The module is assumed healthy while its process lives.
    None,
}

/// The declarative description of a module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleManifest {
    pub id: ModuleId,
    pub version: String,
    pub description: String,

    /// Capabilities the node MUST provide for this module to load.
    #[serde(default)]
    pub required_capabilities: Vec<Capability>,
    /// Capabilities that enable extra behaviour but are not required.
    #[serde(default)]
    pub optional_capabilities: Vec<Capability>,
    /// Other modules that must be installed and loaded first.
    #[serde(default)]
    pub dependencies: Vec<ModuleId>,
    #[serde(default)]
    pub permissions: Vec<ModulePermission>,
    /// Architectures this module supports. Empty means "any".
    #[serde(default)]
    pub architectures: Vec<String>,
    /// Node profiles this module is intended for. Empty means "any".
    #[serde(default)]
    pub profiles: Vec<String>,

    /// Load-order hint within a topology. Lower loads earlier.
    #[serde(default = "default_priority")]
    pub priority: i32,

    pub worker: WorkerImpl,
    #[serde(default = "default_health_check")]
    pub health_check: HealthCheck,
    /// JSON Schema describing this module's configuration, if it takes any.
    #[serde(default)]
    pub config_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

fn default_priority() -> i32 {
    100
}

fn default_health_check() -> HealthCheck {
    HealthCheck::None
}

impl ModuleManifest {
    pub fn validate(&self) -> Result<()> {
        if self.version.trim().is_empty() {
            return Err(TypeError::InvalidManifest(
                "version must not be empty".into(),
            ));
        }
        if self.description.trim().is_empty() {
            return Err(TypeError::InvalidManifest(
                "description must not be empty".into(),
            ));
        }
        if self.dependencies.contains(&self.id) {
            return Err(TypeError::InvalidManifest(
                "a module cannot depend on itself".into(),
            ));
        }
        // A module that can actuate must also be allowed to talk to the device; the
        // reverse is not true. Catching this here keeps a half-declared physical
        // module from loading at all.
        if self.permissions.contains(&ModulePermission::DeviceActuate)
            && !self
                .permissions
                .contains(&ModulePermission::DeviceCommunicate)
        {
            return Err(TypeError::InvalidManifest(
                "device-actuate requires device-communicate".into(),
            ));
        }
        if let Some(schema) = &self.config_schema {
            if !schema.is_object() {
                return Err(TypeError::InvalidManifest(
                    "config_schema must be a JSON object".into(),
                ));
            }
        }
        Ok(())
    }

    /// Whether this module supports `architecture`.
    pub fn supports_architecture(&self, architecture: &str) -> bool {
        self.architectures.is_empty() || self.architectures.iter().any(|a| a == architecture)
    }

    /// Whether this module is intended for `profile`.
    pub fn supports_profile(&self, profile: &str) -> bool {
        self.profiles.is_empty() || self.profiles.iter().any(|p| p == profile)
    }

    /// Capabilities required but not provided by `available`.
    pub fn missing_capabilities(&self, available: &CapabilitySet) -> Vec<Capability> {
        available.missing(self.required_capabilities.iter())
    }

    /// Whether this module asks for anything that touches the physical world.
    pub fn is_physical(&self) -> bool {
        self.permissions.iter().any(|p| p.is_physical())
    }
}

impl fmt::Display for ModuleManifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.id, self.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str) -> ModuleManifest {
        ModuleManifest {
            id: ModuleId::parse(id).unwrap(),
            version: "0.1.0".into(),
            description: "a test module".into(),
            required_capabilities: vec![],
            optional_capabilities: vec![],
            dependencies: vec![],
            permissions: vec![],
            architectures: vec![],
            profiles: vec![],
            priority: 100,
            worker: WorkerImpl::None,
            health_check: HealthCheck::None,
            config_schema: None,
            labels: BTreeMap::new(),
        }
    }

    fn native_device_module(id: &str) -> ModuleManifest {
        let mut module = manifest(id);
        module.permissions = vec![
            ModulePermission::DeviceCommunicate,
            ModulePermission::DeviceActuate,
        ];
        module.worker = WorkerImpl::Builtin {
            symbol: "archie_device_adapter".into(),
        };
        module
    }

    fn device_adapter(module_id: &str) -> DeviceAdapterManifest {
        DeviceAdapterManifest {
            adapter_id: "living-room-tv".into(),
            module_id: ModuleId::parse(module_id).unwrap(),
            origin: DeviceAdapterOrigin::BuiltIntoIso,
            supported_classes: vec![DeviceAdapterClass::Television],
            operations: vec![
                DeviceAdapterOperation::ObserveState,
                DeviceAdapterOperation::ApplyDesiredState,
            ],
            resource_budget: DeviceAdapterResourceBudget {
                max_resident_memory_mib: 64,
                max_cpu_millis_per_second: 100,
                max_gpu_memory_mib: 0,
            },
            dry_run_required: true,
            receipt_required: true,
        }
    }

    #[test]
    fn valid_manifest_passes() {
        assert!(manifest("core.archie").validate().is_ok());
    }

    #[test]
    fn rejects_self_dependency() {
        let mut m = manifest("core.archie");
        m.dependencies.push(m.id.clone());
        assert!(m.validate().is_err());
    }

    #[test]
    fn actuation_requires_communication_permission() {
        let mut m = manifest("devices.robot");
        m.permissions.push(ModulePermission::DeviceActuate);
        assert!(
            m.validate().is_err(),
            "actuate without communicate must be rejected"
        );
        m.permissions.push(ModulePermission::DeviceCommunicate);
        assert!(m.validate().is_ok());
    }

    #[test]
    fn rejects_non_object_config_schema() {
        let mut m = manifest("core.archie");
        m.config_schema = Some(serde_json::json!("not an object"));
        assert!(m.validate().is_err());
    }

    #[test]
    fn empty_architecture_list_means_any() {
        let m = manifest("core.archie");
        assert!(m.supports_architecture("x86_64"));
        assert!(m.supports_architecture("aarch64"));
    }

    #[test]
    fn explicit_architecture_list_is_enforced() {
        let mut m = manifest("core.archie");
        m.architectures.push("x86_64".into());
        assert!(m.supports_architecture("x86_64"));
        assert!(!m.supports_architecture("aarch64"));
    }

    #[test]
    fn missing_capabilities_are_reported() {
        let mut m = manifest("science.physics");
        m.required_capabilities
            .push(Capability::known("compute.cuda"));
        m.required_capabilities
            .push(Capability::known("compute.cpu"));
        let available: CapabilitySet = [Capability::known("compute.cpu")].into_iter().collect();
        assert_eq!(
            m.missing_capabilities(&available),
            vec![Capability::known("compute.cuda")]
        );
    }

    #[test]
    fn dangerous_permissions_require_human_authorization() {
        assert!(ModulePermission::DeviceActuate.requires_human_authorization());
        assert!(ModulePermission::ReadSecrets.requires_human_authorization());
        assert!(!ModulePermission::ReadTelemetry.requires_human_authorization());
    }

    #[test]
    fn only_builtin_workers_are_trusted() {
        assert!(WorkerImpl::Builtin { symbol: "x".into() }.is_trusted());
        assert!(!WorkerImpl::Python {
            module: "m".into(),
            entrypoint: "run".into()
        }
        .is_trusted());
    }

    #[test]
    fn module_state_explains_itself() {
        let state = ModuleState::UnsatisfiedCapabilities {
            missing: vec![Capability::known("compute.cuda")],
        };
        assert!(state.explain().contains("compute.cuda"));
        assert!(!state.is_loaded());
        assert_eq!(ModuleState::Loaded.label(), "loaded");
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let mut m = manifest("cognition.critic");
        m.permissions.push(ModulePermission::ReadKnowledge);
        m.worker = WorkerImpl::Python {
            module: "workers.critic".into(),
            entrypoint: "main".into(),
        };
        m.health_check = HealthCheck::Heartbeat {
            interval_seconds: 30,
        };
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<ModuleManifest>(&json).unwrap(), m);
    }

    #[test]
    fn manifest_parses_from_toml_with_defaults() {
        // Modules ship as TOML on disk; defaults must fill in the optional fields.
        let toml_src = r#"
id = "cognition.researcher"
version = "0.1.0"
description = "Literature discovery"
required_capabilities = ["network.internet"]

[worker]
kind = "python"
module = "workers.research"
entrypoint = "main"
"#;
        let m: ModuleManifest = toml::from_str(toml_src).unwrap();
        assert_eq!(m.priority, 100);
        assert_eq!(m.health_check, HealthCheck::None);
        assert!(m.permissions.is_empty());
        assert!(m.validate().is_ok());
    }

    #[test]
    fn native_device_adapter_requires_a_matching_builtin_module() {
        let adapter = device_adapter("devices.television");
        let module = native_device_module("devices.television");
        assert!(adapter.validate_activation(&module).is_ok());

        let mut wrong_id = module.clone();
        wrong_id.id = ModuleId::parse("devices.other").unwrap();
        assert!(adapter.validate_activation(&wrong_id).is_err());

        let mut process = module;
        process.worker = WorkerImpl::Process {
            command: "/usr/bin/unreviewed-adapter".into(),
            args: vec![],
        };
        assert!(adapter.validate_activation(&process).is_err());

        let mut generic_configuration = native_device_module("devices.television");
        generic_configuration.config_schema = Some(serde_json::json!({"host": "string"}));
        assert!(adapter.validate_activation(&generic_configuration).is_err());
    }

    #[test]
    fn generated_device_adapter_is_a_reviewable_but_inactive_proposal() {
        let adapter = DeviceAdapterManifest::generated_proposal(
            "living-room-tv".into(),
            ModuleId::parse("devices.television").unwrap(),
            vec![DeviceAdapterClass::Television],
            vec![
                DeviceAdapterOperation::ObserveState,
                DeviceAdapterOperation::ApplyDesiredState,
            ],
            DeviceAdapterResourceBudget {
                max_resident_memory_mib: 64,
                max_cpu_millis_per_second: 100,
                max_gpu_memory_mib: 0,
            },
        )
        .unwrap();
        let module = native_device_module("devices.television");

        assert!(adapter.validate().is_ok());
        assert!(!adapter.is_activation_eligible());
        assert!(adapter.validate_activation(&module).is_err());
    }

    #[test]
    fn generated_proposal_refuses_an_incomplete_firmware_request() {
        let proposal = DeviceAdapterManifest::generated_proposal(
            "home-router".into(),
            ModuleId::parse("devices.router").unwrap(),
            vec![DeviceAdapterClass::Router],
            vec![DeviceAdapterOperation::FirmwareApply],
            DeviceAdapterResourceBudget {
                max_resident_memory_mib: 64,
                max_cpu_millis_per_second: 100,
                max_gpu_memory_mib: 0,
            },
        );
        assert!(proposal.is_err());
    }

    #[test]
    fn adapter_operation_permissions_follow_the_declared_vocabulary() {
        let mut observe_only = device_adapter("devices.television");
        observe_only.operations = vec![DeviceAdapterOperation::ObserveState];
        observe_only.dry_run_required = false;
        observe_only.receipt_required = false;
        let mut communicating_module = native_device_module("devices.television");
        communicating_module
            .permissions
            .retain(|permission| *permission != ModulePermission::DeviceActuate);
        assert!(observe_only
            .validate_activation(&communicating_module)
            .is_ok());

        let mut mutating_module = native_device_module("devices.television");
        mutating_module
            .permissions
            .retain(|permission| *permission != ModulePermission::DeviceActuate);
        assert!(device_adapter("devices.television")
            .validate_activation(&mutating_module)
            .is_err());
    }

    #[test]
    fn firmware_apply_requires_a_rehearsed_recovery_path() {
        let mut adapter = device_adapter("devices.router");
        adapter.adapter_id = "home-router".into();
        adapter.module_id = ModuleId::parse("devices.router").unwrap();
        adapter.supported_classes = vec![DeviceAdapterClass::Router];
        adapter.operations = vec![DeviceAdapterOperation::FirmwareApply];
        assert!(adapter.validate().is_err());

        adapter.operations = vec![
            DeviceAdapterOperation::FirmwareInspect,
            DeviceAdapterOperation::FirmwareStage,
            DeviceAdapterOperation::FirmwareApply,
            DeviceAdapterOperation::FirmwareRollback,
        ];
        assert!(adapter
            .validate_activation(&native_device_module("devices.router"))
            .is_ok());
    }

    #[test]
    fn adapter_contract_rejects_ambiguous_names_and_duplicate_privileges() {
        let mut adapter = device_adapter("devices.television");
        adapter.adapter_id = "Living--room".into();
        assert!(adapter.validate().is_err());

        let mut duplicate_class = device_adapter("devices.television");
        duplicate_class
            .supported_classes
            .push(DeviceAdapterClass::Television);
        assert!(duplicate_class.validate().is_err());

        let mut duplicate_operation = device_adapter("devices.television");
        duplicate_operation
            .operations
            .push(DeviceAdapterOperation::ObserveState);
        assert!(duplicate_operation.validate().is_err());
    }
}
