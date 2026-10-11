//! Trusted in-process composition. No wire caller can assert human authentication.
use archie_policy::PolicyEngine;
use archie_sentry::adapters::ipp::IppClient;
use archie_sentry::{
    contracts::Intent,
    engine::{CoreAuthority, EdgeAdapter, Engine, EngineError, ExecutionGuard, ExecutionResult},
    registry::Registry,
};
use archie_state::{StateError, StateStore};

use std::sync::{Arc, Mutex};

pub use crate::sentry_config::{PrinterBinding, SentrySection};

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
struct CompositeGuard {
    state: Arc<StateGuard>,
    extra: Arc<dyn ExecutionGuard>,
}
impl ExecutionGuard for CompositeGuard {
    fn validate(&self, intent: &Intent, authority: &CoreAuthority) -> Result<(), EngineError> {
        self.state.validate(intent, authority)?;
        self.extra.validate(intent, authority)
    }
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
    pub(crate) async fn execute_with_guard(
        &self,
        intent: &Intent,
        authority: &CoreAuthority,
        adapter: &mut dyn EdgeAdapter,
        extra: Arc<dyn ExecutionGuard>,
    ) -> Result<ExecutionResult, EngineError> {
        let journal = self.state.sentry_journal()?;
        Engine::new(self.registry.clone(), journal)
            .with_guard(Arc::new(CompositeGuard {
                state: self.guard.clone(),
                extra,
            }))
            .execute(intent, authority, adapter)
            .await
    }
}
impl crate::Runtime {
    pub async fn sentry_home_status(&self, resource_id: &str) -> crate::control::ControlResponse {
        use crate::control::ControlResponse;
        if archie_sentry::contracts::validate_id(resource_id).is_err() {
            return ControlResponse::error("invalid Sentry home resource");
        }
        let Some(binding) = &self.config.sentry.home_assistant else {
            return ControlResponse::error("Sentry Home Assistant is not configured");
        };
        if !binding
            .resources
            .iter()
            .chain(&binding.telemetry_resources)
            .any(|r| r.resource_id == resource_id)
        {
            return ControlResponse::error("Sentry home resource is not configured");
        }
        let adapter = match binding.adapter() {
            Ok(adapter) => adapter,
            Err(_) => {
                return ControlResponse::error("Sentry home binding or credentials unavailable")
            }
        };
        match adapter.snapshot(resource_id).await {
            Ok(snapshot) => ControlResponse::SentryHomeStatus { snapshot },
            Err(error) => ControlResponse::error(format!("Sentry home query failed: {error}")),
        }
    }
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
