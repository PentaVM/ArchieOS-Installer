//! Trusted in-process composition. No wire caller can assert human authentication.
use archie_policy::PolicyEngine;
use archie_sentry::{
    contracts::Intent,
    engine::{CoreAuthority, EdgeAdapter, Engine, EngineError, ExecutionGuard, ExecutionResult},
    registry::Registry,
};
use archie_state::{StateError, StateStore};
use std::sync::{Arc, Mutex};

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
    /// Uses the booted node's state and policy; default policy refuses actuation.
    pub fn sentry_service(&self, registry: Arc<Mutex<Registry>>) -> SentryService {
        SentryService::new(self.state.clone(), registry, self.policy().clone())
    }
}
