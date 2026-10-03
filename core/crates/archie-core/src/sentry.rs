//! Trusted in-process composition. No wire caller can assert human authentication.
use archie_policy::PolicyEngine;
use archie_sentry::adapters::ipp::IppClient;
use archie_sentry::{
    contracts::Intent,
    engine::{CoreAuthority, EdgeAdapter, Engine, EngineError, ExecutionGuard, ExecutionResult},
    registry::Registry,
};
use archie_state::{StateError, StateStore};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Read-only operator bindings. A reported UUID is an identity check, not a
/// cryptographic pairing proof or authority to actuate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SentrySection {
    pub printers: Vec<PrinterBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrinterBinding {
    pub resource_id: String,
    pub endpoint: String,
    pub expected_uuid: String,
}

impl SentrySection {
    pub fn validate(&self) -> Result<(), crate::config::ConfigError> {
        let invalid =
            || crate::config::ConfigError::Invalid("invalid Sentry printer binding".into());
        if self.printers.len() > 64 {
            return Err(invalid());
        }
        let mut resources = std::collections::BTreeSet::new();
        for printer in &self.printers {
            archie_sentry::contracts::validate_id(&printer.resource_id).map_err(|_| invalid())?;
            if !resources.insert(&printer.resource_id) {
                return Err(invalid());
            }
            IppClient::new(&printer.endpoint).map_err(|_| invalid())?;
            let raw = printer
                .expected_uuid
                .strip_prefix("urn:uuid:")
                .ok_or_else(invalid)?;
            if raw.len() != 36
                || !raw.bytes().enumerate().all(|(i, b)| {
                    if [8, 13, 18, 23].contains(&i) {
                        b == b'-'
                    } else {
                        b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
                    }
                })
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

struct StateGuard {
    state: Arc<StateStore>,
    policy: PolicyEngine,
}
impl ExecutionGuard for StateGuard {
    fn validate(&self, intent: &Intent, authority: &CoreAuthority) -> Result<(), EngineError> {
        authority.check_policy(intent, &self.policy)?;
        self.state
            .validate_prepared_device_operation(
                intent.operation_id,
                &intent.resource_id,
                intent.trace_id,
                &intent.digest()?,
            )
            .map_err(|_| EngineError::Authority)?;
        Ok(())
    }
}
pub struct SentryService {
    state: Arc<StateStore>,
    registry: Arc<Mutex<Registry>>,
    guard: Arc<StateGuard>,
}
impl SentryService {
    /// Registry routes and policy are trusted Core configuration.
    pub fn new(
        state: Arc<StateStore>,
        registry: Arc<Mutex<Registry>>,
        policy: PolicyEngine,
    ) -> Self {
        let guard = Arc::new(StateGuard {
            state: state.clone(),
            policy,
        });
        Self {
            state,
            registry,
            guard,
        }
    }
    /// Returns the allocated intent that the operator must review. The input's
    /// positive operation ID is a placeholder and grants no permission.
    pub fn prepare(&self, mut intent: Intent) -> Result<Intent, EngineError> {
        intent.validate()?;
        let resource = intent.resource_id.clone();
        let trace = intent.trace_id;
        self.state
            .prepare_bound_device_operation(&resource, trace, |id| {
                intent.operation_id = id;
                intent
                    .digest()
                    .map_err(|_| StateError::Corrupt("invalid typed Sentry intent".into()))
            })
            .map_err(|_| EngineError::Authority)?;
        Ok(intent)
    }
    pub async fn execute(
        &self,
        intent: &Intent,
        authority: &CoreAuthority,
        adapter: &mut dyn EdgeAdapter,
    ) -> Result<ExecutionResult, EngineError> {
        let journal = self.state.sentry_journal()?;
        Engine::new(self.registry.clone(), journal)
            .with_guard(self.guard.clone())
            .execute(intent, authority, adapter)
            .await
    }
}
impl crate::Runtime {
    /// Queries one configured printer. Wire clients cannot supply a URL or expand
    /// the configured network scope. This never prepares or sends a print job.
    pub async fn sentry_printer(&self, resource_id: &str) -> crate::control::ControlResponse {
        use crate::control::ControlResponse;
        if archie_sentry::contracts::validate_id(resource_id).is_err() {
            return ControlResponse::error("invalid Sentry printer resource");
        }
        let Some(binding) = self
            .config
            .sentry
            .printers
            .iter()
            .find(|p| p.resource_id == resource_id)
        else {
            return ControlResponse::error("Sentry printer resource is not configured");
        };
        let printer = match IppClient::new(&binding.endpoint) {
            Ok(printer) => printer,
            Err(_) => return ControlResponse::error("invalid Sentry printer binding"),
        };
        match printer.get_printer_attributes(1).await {
            Ok(snapshot) if snapshot.uuid.as_deref() == Some(binding.expected_uuid.as_str()) => {
                ControlResponse::SentryPrinter {
                    resource_id: resource_id.into(),
                    snapshot,
                }
            }
            Ok(_) => ControlResponse::error("Sentry printer identity mismatch"),
            Err(error) => ControlResponse::error(format!("Sentry printer query failed: {error}")),
        }
    }

    /// Uses the booted node's state and policy; default policy refuses actuation.
    pub fn sentry_service(&self, registry: Arc<Mutex<Registry>>) -> SentryService {
        SentryService::new(self.state.clone(), registry, self.policy().clone())
    }
}
