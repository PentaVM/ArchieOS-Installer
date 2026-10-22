//! Operator-owned runtime scope plus the existing Policy engine. Model text never
//! supplies this profile or a lease. Reload it for every admission and recall.
use archie_memory::resume::ConstraintSource;
use archie_memory::{
    cognitive::FocusRecord,
    execution::PinnedConstraints,
    inference::{sha256, InferenceBinding},
};
use archie_policy::{
    Action, CapabilityRequest, Constraints, Lease, Operation, PolicyEngine, Principal, Resource,
    Scope,
};
use archie_types::{PrivacyGrade, TaskId, Timestamp, TraceId};
use archie_vault::ContextDomain;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeProfile {
    pub schema_version: u32,
    pub enabled: bool,
    pub allowed_domains: Vec<ContextDomain>,
    pub allowed_runners: Vec<String>,
    pub max_privacy: PrivacyGrade,
    pub max_prompt_bytes: usize,
    pub max_output_tokens: u32,
    pub max_duration_ms: u64,
}
impl RuntimeProfile {
    pub fn for_domain(domain: ContextDomain) -> Self {
        Self {
            schema_version: 1,
            enabled: true,
            allowed_domains: vec![domain],
            allowed_runners: vec!["api".into(), "codex".into()],
            max_privacy: PrivacyGrade::Restricted,
            max_prompt_bytes: 32768,
            max_output_tokens: 2048,
            max_duration_ms: 60000,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.allowed_domains.is_empty()
            || self.allowed_domains.len() > 64
            || self.allowed_runners.is_empty()
            || self.allowed_runners.len() > 8
            || self
                .allowed_runners
                .iter()
                .any(|r| !matches!(r.as_str(), "api" | "codex"))
            || self.max_privacy == PrivacyGrade::Secret
            || !(1..=65536).contains(&self.max_prompt_bytes)
            || !(1..=8192).contains(&self.max_output_tokens)
            || !(100..=600000).contains(&self.max_duration_ms)
        {
            return Err("invalid operator inference profile".into());
        }
        Ok(())
    }
    pub fn create(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        self.validate()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|_| "cannot create inference profile directory")?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(path)
            .map_err(|_| "inference profile already exists or is not writable")?;
        file.write_all(
            toml::to_string_pretty(self)
                .map_err(|_| "cannot encode inference profile")?
                .as_bytes(),
        )
        .map_err(|_| "cannot write inference profile".to_owned())
    }
}

pub struct RuntimeAuthority {
    path: PathBuf,
    // A live host may share its actual Core engine here, including emergency pause.
    engine: Arc<RwLock<PolicyEngine>>,
}

/// The actual granted authority remains local runtime state. A fresh descriptive
/// policy view cannot renew this grant's wall-clock or monotonic deadline.
pub struct RuntimeGrant {
    pub pinned: PinnedConstraints,
    lease: Lease,
    operation: Operation,
    deadline: Instant,
    duration_ms: u64,
}
impl RuntimeGrant {
    pub fn remaining(&self) -> Result<Duration, String> {
        let now = Timestamp::now();
        let expiry = self
            .lease
            .constraints
            .expires_at
            .ok_or("original lease has no expiry")?;
        if self.lease.revoked || now < self.lease.issued_at || now >= expiry {
            return Err("original inference lease expired or clock rolled back".into());
        }
        let remaining =
            self.deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(
                    (expiry.as_millis() - now.as_millis()) as u64,
                ));
        if remaining.is_zero() {
            return Err("original inference lease expired".into());
        }
        Ok(remaining)
    }
    pub fn dispatch(&mut self) -> Result<Duration, String> {
        self.lease
            .authorises(&self.operation, Timestamp::now())
            .map_err(|_| "original Core lease refused dispatch")?;
        let remaining = self.remaining()?;
        self.lease.consume();
        Ok(remaining)
    }
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
    pub fn expires_at_ms(&self) -> i64 {
        self.lease
            .constraints
            .expires_at
            .expect("grant constructor requires expiry")
            .as_millis()
    }
}
impl RuntimeAuthority {
    /// The kernel-reported peer must own the operator profile. JSON never carries
    /// this identity; Windows loopback cannot establish it.
    pub fn admit_operator_uid(&self, uid: u32) -> Result<(), String> {
        self.profile()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = std::fs::symlink_metadata(&self.path)
                .map_err(|_| "operator profile unavailable")?;
            if metadata.uid() == uid {
                return Ok(());
            }
        }
        let _ = uid;
        Err("inference requires the kernel-identified profile owner".into())
    }
    /// Re-read the operator profile and the live engine immediately before an
    /// external operation: an emergency pause or profile edit made after
    /// admission must stop dispatch, not only the later commit.
    pub fn dispatch(&self, grant: &mut RuntimeGrant) -> Result<Duration, String> {
        let (_, revision) = self.profile()?;
        if revision != grant.pinned.policy_revision_ref {
            return Err("policy changed after admission".into());
        }
        grant.dispatch()
    }
    pub fn new(path: PathBuf, engine: Arc<RwLock<PolicyEngine>>) -> Self {
        Self { path, engine }
    }
    pub fn profile(&self) -> Result<(RuntimeProfile, String), String> {
        let meta = std::fs::symlink_metadata(&self.path)
            .map_err(|_| "operator inference profile is required")?;
        if !meta.is_file() || meta.len() > 32768 {
            return Err("invalid inference profile file".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o022 != 0 {
                return Err("inference profile is writable by another user".into());
            }
        }
        let bytes = std::fs::read(&self.path).map_err(|_| "cannot read inference profile")?;
        if bytes.len() > 32768 {
            return Err("oversized inference profile".into());
        }
        let profile: RuntimeProfile =
            toml::from_str(std::str::from_utf8(&bytes).map_err(|_| "invalid inference profile")?)
                .map_err(|_| "invalid inference profile")?;
        profile.validate()?;
        if !profile.enabled
            || self
                .engine
                .read()
                .map_err(|_| "policy unavailable")?
                .config()
                .emergency_pause
        {
            return Err("inference is paused by policy".into());
        }
        Ok((profile, format!("policy:{}", sha256(&bytes))))
    }
    pub fn admit(
        &self,
        binding: &InferenceBinding,
        privacy: PrivacyGrade,
        model: &str,
        now_ms: i64,
    ) -> Result<PinnedConstraints, String> {
        let mut grant = self.admit_grant(binding, privacy, model, now_ms)?;
        grant.dispatch()?;
        Ok(grant.pinned)
    }
    pub fn admit_grant(
        &self,
        binding: &InferenceBinding,
        privacy: PrivacyGrade,
        model: &str,
        now_ms: i64,
    ) -> Result<RuntimeGrant, String> {
        binding.validate()?;
        let (profile, revision) = self.profile()?;
        if !profile.allowed_domains.contains(&binding.domain)
            || !profile.allowed_runners.contains(&binding.runner_id)
            || privacy > profile.max_privacy
            || privacy == PrivacyGrade::Secret
        {
            return Err("inference outside operator domain/runner/privacy scope".into());
        }
        let task = binding
            .task_id
            .parse::<TaskId>()
            .map_err(|_| "inference task must be a UUID")?;
        let principal = Principal::Module {
            module: "unified-inference".into(),
        };
        let resource = Resource::Model {
            model: model.to_owned(),
        };
        let operation = Operation {
            principal: principal.clone(),
            resource: resource.clone(),
            action: Action::Execute,
            task_id: Some(task),
            user_present: false,
        };
        let request = CapabilityRequest {
            trace_id: TraceId::new(),
            task_id: Some(task),
            principal,
            resource,
            scope: Scope::OneTask { task },
            constraints: Constraints::with_actions([Action::Execute])
                .limited_to(1)
                .expiring_in_ms(profile.max_duration_ms as i64),
            justification: "bounded attributed text inference".into(),
        };
        let lease = self
            .engine
            .read()
            .map_err(|_| "policy unavailable")?
            .decide(&request)
            .lease
            .ok_or("Core policy refused inference")?;
        lease
            .authorises(&operation, Timestamp::now())
            .map_err(|_| "Core lease refused inference")?;
        let pinned = self.constraints_for(
            &FocusRecord {
                focus_id: binding.focus_id.clone(),
                revision: 1,
                objective_ref: binding.objective_ref.clone(),
                parent_focus_id: None,
                suspended: false,
                half_life_ms: 60000,
                updated_at_ms: now_ms,
                activations: vec![],
            },
            now_ms,
        )?;
        if pinned.policy_revision_ref != revision {
            return Err("policy changed during admission".into());
        }
        let expiry = lease
            .constraints
            .expires_at
            .ok_or("Core grant has no expiry")?;
        let duration_ms = expiry
            .as_millis()
            .saturating_sub(Timestamp::now().as_millis());
        if duration_ms <= 0 {
            return Err("Core grant expired during admission".into());
        }
        Ok(RuntimeGrant {
            pinned,
            lease,
            operation,
            deadline: Instant::now() + Duration::from_millis(duration_ms as u64),
            duration_ms: duration_ms as u64,
        })
    }
}
impl ConstraintSource for RuntimeAuthority {
    fn constraints_for(
        &self,
        focus: &FocusRecord,
        now_ms: i64,
    ) -> Result<PinnedConstraints, String> {
        let (_, revision) = self.profile()?;
        if focus.suspended || now_ms <= 0 {
            return Err("Focus cannot resume".into());
        }
        Ok(PinnedConstraints {
            focus_id: focus.focus_id.clone(),
            policy_revision_ref: revision,
            issued_at_ms: now_ms,
            authority_ref: "core:policy".into(),
            privacy_ref: "policy:exact-domain".into(),
            capability_ref: "model:text-proposal".into(),
            operator_constraint_refs: vec![],
            invariant_refs: vec![
                "invariant:no-tool-authority".into(),
                "invariant:attributed-unverified-result".into(),
            ],
            approval_requirement_refs: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_scope_and_live_pause_are_rechecked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inference.toml");
        RuntimeProfile::for_domain(ContextDomain::personal("owner").unwrap())
            .create(&path)
            .unwrap();
        let engine = Arc::new(RwLock::new(PolicyEngine::default()));
        let policy = RuntimeAuthority::new(path, engine.clone());
        let mut binding = InferenceBinding {
            schema_version: 1,
            domain: ContextDomain::personal("owner").unwrap(),
            focus_id: "focus:test".into(),
            task_id: TaskId::new().to_string(),
            objective_ref: "objective:test".into(),
            runner_id: "codex".into(),
            provider_id: "openai".into(),
            account_id: "account:test".into(),
        };
        assert!(policy
            .admit(
                &binding,
                PrivacyGrade::Public,
                "model",
                Timestamp::now().as_millis()
            )
            .is_ok());
        binding.domain = ContextDomain::work("other", "project").unwrap();
        assert!(policy
            .admit(
                &binding,
                PrivacyGrade::Public,
                "model",
                Timestamp::now().as_millis()
            )
            .is_err());
        binding.domain = ContextDomain::personal("owner").unwrap();
        engine.write().unwrap().set_emergency_pause(true);
        assert!(policy
            .admit(
                &binding,
                PrivacyGrade::Public,
                "model",
                Timestamp::now().as_millis()
            )
            .is_err());
    }

    #[test]
    fn a_pause_after_admission_refuses_dispatch_of_the_held_grant() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inference.toml");
        RuntimeProfile::for_domain(ContextDomain::personal("owner").unwrap())
            .create(&path)
            .unwrap();
        let engine = Arc::new(RwLock::new(PolicyEngine::default()));
        let policy = RuntimeAuthority::new(path, engine.clone());
        let binding = InferenceBinding {
            schema_version: 1,
            domain: ContextDomain::personal("owner").unwrap(),
            focus_id: "focus:test".into(),
            task_id: TaskId::new().to_string(),
            objective_ref: "objective:test".into(),
            runner_id: "api".into(),
            provider_id: "fixture".into(),
            account_id: "account:test".into(),
        };
        let mut grant = policy
            .admit_grant(
                &binding,
                PrivacyGrade::Public,
                "model",
                Timestamp::now().as_millis(),
            )
            .unwrap();
        engine.write().unwrap().set_emergency_pause(true);
        assert!(policy.dispatch(&mut grant).is_err());
        engine.write().unwrap().set_emergency_pause(false);
        assert!(policy.dispatch(&mut grant).is_ok());
    }
}
