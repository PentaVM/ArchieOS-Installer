//! World proposals admitted by Core's existing operator profile and Policy engine.
use crate::{
    control::{ControlRequest, ControlResponse, WorldJobAction},
    Runtime,
};
use archie_geofabric::exploration::{
    self, ExplorationDomain, ExplorationLayer, ExplorationRequest,
};
use archie_policy::{
    Action, CapabilityRequest, Constraints, Operation, Principal, Resource, Scope,
};
use archie_types::{PrivacyGrade, Timestamp, TraceId};
use archie_vault::ContextDomain;
use archie_workstation::ai_policy::RuntimeProfile;
use archie_worldd::exploration::ExploreCommand;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

/// Captured from a live socket, never deserializable from model output.
pub(crate) struct WorldOperator {
    uid: u32,
}
impl WorldOperator {
    #[cfg(target_os = "linux")]
    pub(crate) fn from_stream(stream: &crate::endpoint::ControlStream) -> Option<Self> {
        Some(Self {
            uid: stream.peer_cred().ok()?.uid(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    pub(crate) fn from_stream(_: &crate::endpoint::ControlStream) -> Option<Self> {
        None
    }
}

fn geo_domain(domain: &ContextDomain) -> ExplorationDomain {
    if domain.is_work() {
        ExplorationDomain::Work
    } else {
        ExplorationDomain::Personal
    }
}

fn admit_command(
    profile: &RuntimeProfile,
    domain: &ContextDomain,
    privacy: PrivacyGrade,
    command: &ExploreCommand,
) -> Result<Vec<&'static str>, String> {
    profile.validate()?;
    if !profile.enabled
        || !profile.allowed_domains.contains(domain)
        || privacy > profile.max_privacy
        || privacy == PrivacyGrade::Secret
    {
        return Err("world operation is outside operator domain/privacy scope".into());
    }
    let mut hosts = Vec::new();
    match command {
        ExploreCommand::Plan { request } | ExploreCommand::Explore { request } => {
            if request.domain != geo_domain(domain) {
                return Err("geographic domain differs from admitted context domain".into());
            }
            exploration::plan(request).map_err(|error| error.to_string())?;
            if matches!(command, ExploreCommand::Explore { .. }) {
                for layer in &request.layers {
                    let host = match layer {
                        ExplorationLayer::Terrain => "elevation-tiles-prod.s3.amazonaws.com",
                        ExplorationLayer::Buildings => "overpass-api.de",
                        _ => {
                            return Err("world layer has no admitted fixed capture provider".into())
                        }
                    };
                    if !hosts.contains(&host) {
                        hosts.push(host);
                    }
                }
            }
        }
        ExploreCommand::Resolve {
            query,
            domain: requested,
        } => {
            if *requested != geo_domain(domain)
                || query.trim().is_empty()
                || query.chars().count() > 256
            {
                return Err("invalid world place query or domain".into());
            }
            hosts.push("photon.komoot.io");
        }
        ExploreCommand::Status { domain: requested } if *requested == geo_domain(domain) => {}
        ExploreCommand::Pause {
            id,
            domain: requested,
        }
        | ExploreCommand::Cancel {
            id,
            domain: requested,
        } if *requested == geo_domain(domain) && valid_job_id(id) => {}
        _ => return Err("world operation is outside this gateway".into()),
    }
    // These third parties have no verified retention exemption. Public data only,
    // including queries/coordinates; a Work scope never becomes Personal to pass.
    if !hosts.is_empty() && privacy != PrivacyGrade::Public {
        return Err("external world providers require explicitly public data".into());
    }
    Ok(hosts)
}

fn capabilities(
    uid: u32,
    domain: &ContextDomain,
    privacy: PrivacyGrade,
    command: &ExploreCommand,
    hosts: &[&str],
) -> Result<Vec<CapabilityRequest>, String> {
    if matches!(command, ExploreCommand::Resume { .. }) && hosts.is_empty() {
        return Err("resume requires providers derived from the owned job request".into());
    }
    let canonical = serde_json::to_vec(
        &serde_json::json!({"domain":domain,"privacy":privacy,"command":command}),
    )
    .map_err(|e| e.to_string())?;
    let digest = Sha256::digest(canonical);
    let id = u64::from_be_bytes(
        digest[..8]
            .try_into()
            .map_err(|_| "invalid operation binding")?,
    );
    let resources: Vec<_> = if hosts.is_empty() {
        vec![Resource::Knowledge {
            collection: format!("world:{}", domain.path()),
        }]
    } else {
        hosts
            .iter()
            .map(|host| Resource::Network {
                host: (*host).into(),
                port: 443,
            })
            .collect()
    };
    Ok(resources
        .into_iter()
        .map(|resource| CapabilityRequest {
            trace_id: TraceId::new(),
            task_id: None,
            principal: Principal::Human {
                id: format!("unix-uid-{uid}"),
                authenticated: true,
            },
            resource,
            scope: Scope::OneOperation { operation: id },
            constraints: Constraints::with_actions([if hosts.is_empty() {
                if matches!(
                    command,
                    ExploreCommand::Pause { .. } | ExploreCommand::Cancel { .. }
                ) {
                    Action::Write
                } else {
                    Action::Read
                }
            } else {
                Action::Execute
            }])
            .limited_to(1)
            .expiring_in_ms(90_000),
            justification: "operator-scoped bounded world operation".into(),
        })
        .collect())
}

fn operation(request: &CapabilityRequest) -> Operation {
    Operation {
        principal: request.principal.clone(),
        resource: request.resource.clone(),
        action: request
            .constraints
            .actions
            .iter()
            .next()
            .copied()
            .unwrap_or(Action::Read),
        task_id: None,
        user_present: false,
    }
}

fn operator_peer(peer: Option<&WorldOperator>) -> Result<&WorldOperator, String> {
    peer.ok_or_else(|| "authenticated local operator session required".into())
}

fn valid_job_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn require_job_binding(
    id: &str,
    domain: &ContextDomain,
    ledger: &BTreeMap<String, ContextDomain>,
) -> Result<(), String> {
    if !valid_job_id(id) || ledger.get(id) != Some(domain) {
        return Err("job is not bound to this exact context domain".into());
    }
    Ok(())
}

fn owned_job_request(
    status: &serde_json::Value,
    id: &str,
    domain: &ContextDomain,
    ledger: &BTreeMap<String, ContextDomain>,
) -> Result<ExplorationRequest, String> {
    require_job_binding(id, domain, ledger)?;
    let jobs = status["jobs"].as_array().ok_or("invalid world status")?;
    let mut matching = jobs.iter().filter(|job| job["id"].as_str() == Some(id));
    let job = matching.next().ok_or("owned world job is unavailable")?;
    if matching.next().is_some() || job["domain"] != serde_json::json!(geo_domain(domain)) {
        return Err("world job identity or domain mismatch".into());
    }
    let request: ExplorationRequest = serde_json::from_value(job["request"].clone())
        .map_err(|_| "invalid stored world request")?;
    if request.domain != geo_domain(domain) {
        return Err("stored request domain mismatch".into());
    }
    exploration::plan(&request).map_err(|error| error.to_string())?;
    Ok(request)
}

fn recheck_profile(
    expected: &str,
    fresh: &str,
    profile: &RuntimeProfile,
    domain: &ContextDomain,
    privacy: PrivacyGrade,
    command: &ExploreCommand,
) -> Result<(), String> {
    if expected != fresh {
        return Err("operator world scope changed during operation".into());
    }
    admit_command(profile, domain, privacy, command).map(|_| ())
}

/// This ledger stays in Core state. worldd never receives Core paths or keys.
fn bindings(
    directory: &Path,
    insert: Option<(&str, &ContextDomain)>,
) -> Result<BTreeMap<String, ContextDomain>, String> {
    use archie_worldd::exploration_control::secure_owner_path;
    use archie_worldd::exploration_runtime::{atomic_write, private_directory};
    use fs2::FileExt;
    private_directory(directory)?;
    secure_owner_path(directory, true)?;
    let lock_path = directory.join("bindings.lock");
    if std::fs::symlink_metadata(&lock_path).is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err("invalid world binding lock".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(&lock_path)
        .map_err(|_| "world binding lock unavailable")?;
    secure_owner_path(&lock_path, false)?;
    let waiting = std::time::Instant::now();
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(_) if waiting.elapsed() < std::time::Duration::from_millis(500) => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(_) => return Err("world binding ledger busy".into()),
        }
    }
    let path = directory.join("bindings.json");
    let mut result: BTreeMap<String, ContextDomain> = if path.exists() {
        secure_owner_path(&path, false)?;
        if std::fs::metadata(&path)
            .map_err(|_| "world binding metadata unavailable")?
            .len()
            > 256 * 1024
        {
            return Err("world binding ledger exceeds bound".into());
        }
        serde_json::from_slice(
            &std::fs::read(&path).map_err(|_| "world binding ledger unavailable")?,
        )
        .map_err(|_| "invalid world binding ledger")?
    } else {
        BTreeMap::new()
    };
    if result.len() > 512 {
        return Err("world binding ledger exceeds entry bound".into());
    }
    if let Some((id, domain)) = insert {
        if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("invalid accepted world job identifier".into());
        }
        if result.get(id).is_some_and(|old| old != domain) {
            return Err("world job already bound to another context domain".into());
        }
        if result.len() >= 512 && !result.contains_key(id) {
            return Err("world binding history limit reached".into());
        }
        result.insert(id.into(), domain.clone());
        atomic_write(
            &path,
            &serde_json::to_vec(&result).map_err(|_| "cannot encode world bindings")?,
        )?;
        secure_owner_path(&path, false)?;
    }
    Ok(result)
}

fn scoped_status(
    value: &mut serde_json::Value,
    domain: &ContextDomain,
    bindings: &BTreeMap<String, ContextDomain>,
) {
    if let Some(jobs) = value["jobs"].as_array_mut() {
        jobs.retain(|job| {
            job["id"]
                .as_str()
                .is_some_and(|id| bindings.get(id) == Some(domain))
        });
        for job in jobs {
            if let Some(fields) = job.as_object_mut() {
                for field in ["plan", "request", "preview", "artifacts", "camera"] {
                    fields.remove(field);
                }
            }
        }
    }
    // Provider aggregates include captures belonging to other context domains.
    if let Some(fields) = value.as_object_mut() {
        fields.remove("providers");
    }
    value["context_domain"] = serde_json::json!(domain);
}

async fn cancel_accepted(value: &serde_json::Value, domain: &ContextDomain) -> bool {
    let Some(id) = value["job"]["id"].as_str() else {
        return false;
    };
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    let cancel = ExploreCommand::Cancel {
        id: id.into(),
        domain: geo_domain(domain),
    };
    matches!(tokio::time::timeout(std::time::Duration::from_secs(3),
        archie_worldd::exploration_control::send(cancel)).await,
        Ok(Ok(reply)) if reply["job"]["state"] == "cancelled")
}

impl Runtime {
    /// The model supplies only an owned ID. Resume providers come from worldd's
    /// normalized stored request, read under an independent local Policy lease.
    async fn owned_resume_request(
        &self,
        peer: &WorldOperator,
        domain: &ContextDomain,
        privacy: PrivacyGrade,
        revision: &str,
        id: &str,
    ) -> Result<ExplorationRequest, String> {
        let directory = self.paths.state_dir.join("world-context-bindings");
        require_job_binding(id, domain, &bindings(&directory, None)?)?;
        if privacy != PrivacyGrade::Public {
            return Err("resume external providers requires explicitly public data".into());
        }
        let command = ExploreCommand::Status {
            domain: geo_domain(domain),
        };
        let authority = self.ai_authority();
        authority.admit_operator_uid(peer.uid)?;
        let (profile, fresh_revision) = authority.profile()?;
        recheck_profile(
            revision,
            &fresh_revision,
            &profile,
            domain,
            privacy,
            &command,
        )?;
        let request = capabilities(peer.uid, domain, privacy, &command, &[])?.remove(0);
        let decision = self
            .live_policy()
            .read()
            .map_err(|_| "policy unavailable")?
            .decide(&request);
        self.state
            .append_audit(&decision.to_audit(self.node_id(), &request))
            .map_err(|_| "world read audit unavailable")?;
        let mut lease = decision
            .lease
            .ok_or("Core Policy refused owned world job read")?;
        let Scope::OneOperation {
            operation: operation_id,
        } = request.scope
        else {
            return Err("world read lease not scoped".into());
        };
        lease
            .authorises_one_operation(&operation(&request), operation_id, Timestamp::now())
            .map_err(|_| "Core lease refused world job read")?;
        lease.consume();
        let duration = lease
            .constraints
            .expires_at
            .ok_or("world read lease has no expiry")?
            .as_millis()
            .saturating_sub(Timestamp::now().as_millis())
            .clamp(0, 5_000);
        if duration == 0 {
            return Err("world read lease expired".into());
        }
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(duration as u64);
        let status = tokio::time::timeout(
            std::time::Duration::from_millis(duration as u64),
            archie_worldd::exploration_control::send(command.clone()),
        )
        .await
        .map_err(|_| "world job read timed out")??;
        let now = Timestamp::now();
        if std::time::Instant::now() >= deadline
            || lease.revoked
            || now < lease.issued_at
            || lease
                .constraints
                .expires_at
                .is_none_or(|expiry| now >= expiry)
        {
            return Err("world read lease expired before use".into());
        }
        authority.admit_operator_uid(peer.uid)?;
        let (profile, fresh_revision) = authority.profile()?;
        recheck_profile(
            revision,
            &fresh_revision,
            &profile,
            domain,
            privacy,
            &command,
        )?;
        owned_job_request(&status, id, domain, &bindings(&directory, None)?)
    }
    pub(crate) async fn handle_operator_world(
        &self,
        peer: Option<&WorldOperator>,
        request: ControlRequest,
    ) -> ControlResponse {
        let peer = match operator_peer(peer) {
            Ok(peer) => peer,
            Err(error) => return ControlResponse::error(error),
        };
        self.metrics
            .increment(&format!("control.request.{}", request.name()), 1);
        let result = self.dispatch_world(peer, request).await;
        match result {
            Ok(value) => ControlResponse::WorldResult { value },
            Err(error) => ControlResponse::error(error),
        }
    }

    async fn dispatch_world(
        &self,
        peer: &WorldOperator,
        request: ControlRequest,
    ) -> Result<serde_json::Value, String> {
        let (domain, privacy, command) = match request {
            ControlRequest::WorldJob {
                action,
                id,
                domain,
                privacy,
            } => {
                let requested = geo_domain(&domain);
                let command = match action {
                    WorldJobAction::Pause => ExploreCommand::Pause {
                        id,
                        domain: requested,
                    },
                    WorldJobAction::Resume => ExploreCommand::Resume {
                        id,
                        domain: requested,
                    },
                    WorldJobAction::Cancel => ExploreCommand::Cancel {
                        id,
                        domain: requested,
                    },
                };
                (domain, privacy, command)
            }
            ControlRequest::WorldPlan {
                request,
                domain,
                privacy,
            } => (domain, privacy, ExploreCommand::Plan { request }),
            ControlRequest::WorldExplore {
                request,
                domain,
                privacy,
            } => (domain, privacy, ExploreCommand::Explore { request }),
            ControlRequest::WorldResolve {
                query,
                domain,
                privacy,
            } => {
                let requested = geo_domain(&domain);
                (
                    domain,
                    privacy,
                    ExploreCommand::Resolve {
                        query,
                        domain: requested,
                    },
                )
            }
            ControlRequest::WorldStatus { domain, privacy } => {
                let requested = geo_domain(&domain);
                (
                    domain,
                    privacy,
                    ExploreCommand::Status { domain: requested },
                )
            }
            _ => return Err("invalid world dispatch".into()),
        };
        let authority = self.ai_authority();
        authority.admit_operator_uid(peer.uid)?;
        let (profile, revision) = authority.profile()?;
        let binding_dir = self.paths.state_dir.join("world-context-bindings");
        let managed_id = match &command {
            ExploreCommand::Pause { id, .. }
            | ExploreCommand::Resume { id, .. }
            | ExploreCommand::Cancel { id, .. } => Some(id),
            _ => None,
        };
        if let Some(id) = managed_id {
            require_job_binding(id, &domain, &bindings(&binding_dir, None)?)?;
        }
        let admission_command = if let ExploreCommand::Resume { id, .. } = &command {
            ExploreCommand::Explore {
                request: self
                    .owned_resume_request(peer, &domain, privacy, &revision, id)
                    .await?,
            }
        } else {
            command.clone()
        };
        let hosts = admit_command(&profile, &domain, privacy, &admission_command)?;
        let requests = capabilities(peer.uid, &domain, privacy, &command, &hosts)?;
        let engine = self.live_policy();
        let mut grants = Vec::new();
        for request in requests {
            let decision = engine
                .read()
                .map_err(|_| "policy unavailable")?
                .decide(&request);
            self.state
                .append_audit(&decision.to_audit(self.node_id(), &request))
                .map_err(|_| "world authorization audit unavailable")?;
            let lease = decision
                .lease
                .ok_or("Core Policy refused world operation")?;
            grants.push((request, lease));
        }
        // Profile edits and the live emergency pause are observed again immediately
        // before dispatch. Only the exact command gets a single use of these leases.
        authority.admit_operator_uid(peer.uid)?;
        let (fresh, fresh_revision) = authority.profile()?;
        recheck_profile(
            &revision,
            &fresh_revision,
            &fresh,
            &domain,
            privacy,
            &admission_command,
        )?;
        if let Some(id) = managed_id {
            require_job_binding(id, &domain, &bindings(&binding_dir, None)?)?;
        }
        for (request, lease) in &mut grants {
            let Scope::OneOperation { operation: id } = request.scope else {
                return Err("world lease is not operation scoped".into());
            };
            lease
                .authorises_one_operation(&operation(request), id, Timestamp::now())
                .map_err(|_| "Core world lease refused dispatch")?;
            lease.consume();
        }
        let status = matches!(command, ExploreCommand::Status { .. });
        let explore = matches!(command, ExploreCommand::Explore { .. });
        let starts_capture = explore || matches!(command, ExploreCommand::Resume { .. });
        // Verify ledger access before accepting a background job.
        if explore && bindings(&binding_dir, None)?.len() >= 512 {
            return Err("world binding history limit reached".into());
        }
        let now = Timestamp::now();
        let lifetime_ms = grants
            .iter()
            .filter_map(|(_, lease)| lease.constraints.expires_at)
            .map(|expiry| expiry.as_millis().saturating_sub(now.as_millis()))
            .min()
            .unwrap_or(0)
            .clamp(0, 90_000);
        if lifetime_ms == 0 {
            return Err("Core world lease expired before acknowledgement".into());
        }
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(lifetime_ms as u64);
        let mut value = tokio::time::timeout(
            std::time::Duration::from_millis(lifetime_ms as u64),
            archie_worldd::exploration_control::send(command.clone()),
        )
        .await
        .map_err(|_| "world operation acknowledgement timed out".to_string())??;
        // A revoked/edited profile or emergency pause during the awaited operation
        // must prevent disclosure and persistence under the earlier admission.
        let reauthorized = (|| -> Result<(), String> {
            let now = Timestamp::now();
            if std::time::Instant::now() >= deadline
                || grants.iter().any(|(_, lease)| {
                    lease.revoked
                        || now < lease.issued_at
                        || lease
                            .constraints
                            .expires_at
                            .is_none_or(|expiry| now >= expiry)
                })
            {
                return Err("original Core world lease expired before disclosure".into());
            }
            authority.admit_operator_uid(peer.uid)?;
            let (fresh, fresh_revision) = authority.profile()?;
            recheck_profile(
                &revision,
                &fresh_revision,
                &fresh,
                &domain,
                privacy,
                &admission_command,
            )
        })();
        if let Err(error) = reauthorized {
            if starts_capture {
                let stopped = cancel_accepted(&value, &domain).await;
                return Err(format!(
                    "{error}; accepted capture cancellation {}",
                    if stopped { "confirmed" } else { "unconfirmed" }
                ));
            }
            return Err(error);
        }
        if explore {
            let id = value["job"]["id"]
                .as_str()
                .ok_or("world acceptance omitted job id")?;
            if let Err(error) = bindings(&binding_dir, Some((id, &domain))) {
                // Roll back the accepted capture if its exact scope cannot be made
                // durable. Do not leave a live job hidden behind a failed response.
                return if cancel_accepted(&value, &domain).await {
                    Err(format!(
                        "accepted capture cancelled because its Core binding failed: {error}"
                    ))
                } else {
                    Err(format!("capture {id} was accepted but Core binding failed ({error}); cancellation is unconfirmed; inspect it with the local owner CLI"))
                };
            }
        }
        if status {
            scoped_status(&mut value, &domain, &bindings(&binding_dir, None)?);
        }
        if serde_json::to_vec(&value)
            .map_err(|_| "invalid world response")?
            .len()
            > crate::control::MAX_CONTROL_MESSAGE_BYTES - 1024
        {
            return Err("world response exceeds Core control bound".into());
        }
        Ok(value)
    }
}

/// Voice clients carry no worldd command credential: only Core's admitted dispatcher
/// can reach that channel. The operator endpoint authenticates the live kernel peer.
pub async fn send(request: ControlRequest) -> Result<serde_json::Value, String> {
    if !request.is_world() {
        return Err("not a world control request".into());
    }
    let mut stream = crate::endpoint::connect(&archie_state::StatePaths::from_env())
        .await
        .map_err(|_| "Core operator endpoint unavailable")?;
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(95),
        crate::control::request_over(&mut stream, &request),
    )
    .await
    .map_err(|_| "Core world operation timed out")?
    .map_err(|_| "Core world response unavailable")?;
    match response {
        ControlResponse::WorldResult { value } => Ok(value),
        ControlResponse::Error { message } => Err(message),
        _ => Err("unexpected Core world response".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_geofabric::exploration::{ExplorationDomain, ExplorationLayer, ExplorationRequest};
    use archie_policy::{PolicyEngine, Scope};
    use archie_types::{PrivacyGrade, Timestamp};
    use archie_vault::ContextDomain;
    use archie_workstation::ai_policy::RuntimeProfile;
    use archie_worldd::exploration::ExploreCommand;

    fn scope() -> ContextDomain {
        ContextDomain::personal("owner").unwrap()
    }

    #[test]
    fn job_controls_require_exact_binding_and_stored_request() {
        let id = "a".repeat(32);
        let a = ContextDomain::work("org", "a").unwrap();
        let b = ContextDomain::work("org", "b").unwrap();
        let request = ExplorationRequest {
            domain: ExplorationDomain::Work,
            layers: vec![ExplorationLayer::Buildings],
            ..ExplorationRequest::default()
        };
        let status = serde_json::json!({"jobs":[{"id":id,"domain":"work","request":request}]});
        let mut ledger = BTreeMap::new();
        assert!(owned_job_request(&status, &id, &a, &ledger).is_err());
        ledger.insert(id.clone(), b);
        assert!(owned_job_request(&status, &id, &a, &ledger).is_err());
        ledger.insert(id.clone(), a.clone());
        let stored = owned_job_request(&status, &id, &a, &ledger).unwrap();
        assert_eq!(stored.layers, vec![ExplorationLayer::Buildings]);
        let profile = RuntimeProfile::for_domain(a.clone());
        assert_eq!(
            admit_command(
                &profile,
                &a,
                PrivacyGrade::Public,
                &ExploreCommand::Explore { request: stored }
            )
            .unwrap(),
            vec!["overpass-api.de"]
        );
        let mut mismatched = status.clone();
        mismatched["jobs"][0]["request"]["domain"] = serde_json::json!("personal");
        assert!(owned_job_request(&mismatched, &id, &a, &ledger).is_err());
        assert!(owned_job_request(&serde_json::json!({"jobs":[]}), &id, &a, &ledger).is_err());
        assert!(owned_job_request(&status, "../bad", &a, &ledger).is_err());
    }

    #[test]
    fn job_control_rpc_is_typed_and_requires_operator_dispatch() {
        for action in ["pause", "resume", "cancel"] {
            let request: ControlRequest = serde_json::from_value(serde_json::json!({
                "request":"world_job","action":action,"id":"a".repeat(32),
                "domain":"work/org/a","privacy":"public"
            }))
            .unwrap();
            assert!(request.is_world());
            assert!(request.is_mutating());
            let ControlRequest::WorldJob { action: typed, .. } = request else {
                panic!("job action");
            };
            assert_eq!(serde_json::to_value(typed).unwrap(), action);
        }
        assert!(serde_json::from_value::<ControlRequest>(serde_json::json!({
            "request":"world_job","action":"execute","id":"a".repeat(32),
            "domain":"work/org/a","privacy":"public"
        }))
        .is_err());
    }

    #[test]
    fn pause_and_cancel_are_local_write_operations_not_network_or_global_pause() {
        let profile = RuntimeProfile::for_domain(scope());
        for command in [
            ExploreCommand::Pause {
                id: "a".repeat(32),
                domain: ExplorationDomain::Personal,
            },
            ExploreCommand::Cancel {
                id: "a".repeat(32),
                domain: ExplorationDomain::Personal,
            },
        ] {
            assert!(
                admit_command(&profile, &scope(), PrivacyGrade::Restricted, &command)
                    .unwrap()
                    .is_empty()
            );
            let requests =
                capabilities(1000, &scope(), PrivacyGrade::Restricted, &command, &[]).unwrap();
            assert!(matches!(requests[0].resource, Resource::Knowledge { .. }));
            assert_eq!(operation(&requests[0]).action, Action::Write);
        }
        let resume = ExploreCommand::Resume {
            id: "a".repeat(32),
            domain: ExplorationDomain::Personal,
        };
        assert!(capabilities(1000, &scope(), PrivacyGrade::Public, &resume, &[]).is_err());
        let requests = capabilities(
            1000,
            &scope(),
            PrivacyGrade::Public,
            &resume,
            &["overpass-api.de"],
        )
        .unwrap();
        assert_eq!(operation(&requests[0]).action, Action::Execute);
    }
    fn explore(domain: ExplorationDomain) -> ExploreCommand {
        ExploreCommand::Explore {
            request: ExplorationRequest {
                domain,
                ..ExplorationRequest::default()
            },
        }
    }

    #[test]
    fn requests_have_no_ambient_operator_authority_or_arbitrary_endpoint() {
        assert!(operator_peer(None).is_err());
        let json = r#"{"request":"world_resolve","query":"Santiago","domain":"personal/owner","privacy":"public"}"#;
        let request: ControlRequest = serde_json::from_str(json).unwrap();
        assert!(request.is_world());
        assert!(request.is_mutating());
        let injected = json.replace(
            "\"query\":\"Santiago\"",
            "\"query\":\"Santiago\",\"endpoint\":\"https://example.com\"",
        );
        assert!(serde_json::from_str::<ControlRequest>(&injected).is_err());
        let missing_grade = json.replace(",\"privacy\":\"public\"", "");
        assert!(serde_json::from_str::<ControlRequest>(&missing_grade).is_err());
    }

    #[test]
    fn status_hides_foreign_work_projects_unbound_jobs_and_global_usage() {
        let a = ContextDomain::work("org", "a").unwrap();
        let b = ContextDomain::work("org", "b").unwrap();
        let scopes = [("a".into(), a.clone()), ("b".into(), b)].into();
        let mut value = serde_json::json!({"jobs":[
            {"id":"a","state":"running","camera":{"latitude":1},"preview":{"secret":"a"}},
            {"id":"b","state":"running","camera":{"latitude":2}},
            {"id":"cli-unbound","camera":{"latitude":3}}
        ],"providers":[{"bytes_received":12345}]});
        scoped_status(&mut value, &a, &scopes);
        assert_eq!(
            value["jobs"],
            serde_json::json!([{"id":"a","state":"running"}])
        );
        assert!(value.get("providers").is_none());
        assert_eq!(value["context_domain"], "work/org/a");
    }

    #[test]
    fn profile_change_or_revocation_during_reply_refuses_disclosure() {
        let mut profile = RuntimeProfile::for_domain(scope());
        let command = explore(ExplorationDomain::Personal);
        assert!(recheck_profile(
            "old",
            "old",
            &profile,
            &scope(),
            PrivacyGrade::Public,
            &command
        )
        .is_ok());
        assert!(recheck_profile(
            "old",
            "new",
            &profile,
            &scope(),
            PrivacyGrade::Public,
            &command
        )
        .is_err());
        profile.enabled = false;
        assert!(recheck_profile(
            "old",
            "old",
            &profile,
            &scope(),
            PrivacyGrade::Public,
            &command
        )
        .is_err());
    }

    #[test]
    fn core_binding_ledger_persists_exact_scope_and_refuses_reassignment() {
        let directory = std::env::temp_dir().join(format!("world-bindings-{}", TraceId::new()));
        let id = "a".repeat(32);
        bindings(&directory, Some((&id, &scope()))).unwrap();
        assert_eq!(bindings(&directory, None).unwrap()[&id], scope());
        assert!(bindings(
            &directory,
            Some((&id, &ContextDomain::work("org", "project").unwrap()))
        )
        .is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn foreign_domains_restricted_egress_and_disabled_profiles_are_refused() {
        let mut profile = RuntimeProfile::for_domain(scope());
        for privacy in [
            PrivacyGrade::Internal,
            PrivacyGrade::Sensitive,
            PrivacyGrade::Restricted,
            PrivacyGrade::Secret,
        ] {
            assert!(admit_command(
                &profile,
                &scope(),
                privacy,
                &explore(ExplorationDomain::Personal)
            )
            .is_err());
        }
        let work = ContextDomain::work("org", "project").unwrap();
        assert!(admit_command(
            &profile,
            &work,
            PrivacyGrade::Public,
            &explore(ExplorationDomain::Work)
        )
        .is_err());
        assert!(admit_command(
            &profile,
            &scope(),
            PrivacyGrade::Public,
            &explore(ExplorationDomain::Work)
        )
        .is_err());
        profile.enabled = false;
        assert!(admit_command(
            &profile,
            &scope(),
            PrivacyGrade::Public,
            &explore(ExplorationDomain::Personal)
        )
        .is_err());
    }

    #[test]
    fn scoped_public_work_uses_fixed_hosts_and_policy_pause_blocks_lease() {
        let work = ContextDomain::work("org", "project").unwrap();
        let profile = RuntimeProfile::for_domain(work.clone());
        let command = explore(ExplorationDomain::Work);
        let hosts = admit_command(&profile, &work, PrivacyGrade::Public, &command).unwrap();
        assert_eq!(hosts, vec!["elevation-tiles-prod.s3.amazonaws.com"]);
        let requests = capabilities(1000, &work, PrivacyGrade::Public, &command, &hosts).unwrap();
        let mut engine = PolicyEngine::default();
        assert!(engine.decide(&requests[0]).lease.is_some());
        engine.set_emergency_pause(true);
        assert!(engine.decide(&requests[0]).lease.is_none());
        let unsupported = ExploreCommand::Explore {
            request: ExplorationRequest {
                domain: ExplorationDomain::Work,
                layers: vec![ExplorationLayer::Mesh],
                ..ExplorationRequest::default()
            },
        };
        assert!(admit_command(&profile, &work, PrivacyGrade::Public, &unsupported).is_err());
    }

    #[test]
    fn one_operation_lease_cannot_be_substituted_or_reused() {
        let command = explore(ExplorationDomain::Personal);
        let requests = capabilities(
            1000,
            &scope(),
            PrivacyGrade::Public,
            &command,
            &["elevation-tiles-prod.s3.amazonaws.com"],
        )
        .unwrap();
        let request = &requests[0];
        let mut lease = PolicyEngine::default().decide(request).lease.unwrap();
        let operation = operation(request);
        let Scope::OneOperation { operation: id } = request.scope else {
            panic!("one operation");
        };
        assert!(lease
            .authorises_one_operation(&operation, id, Timestamp::now())
            .is_ok());
        assert!(lease
            .authorises_one_operation(&operation, id.wrapping_add(1), Timestamp::now())
            .is_err());
        lease.consume();
        assert!(lease
            .authorises_one_operation(&operation, id, Timestamp::now())
            .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn existing_authority_accepts_profile_owner_and_refuses_another_uid() {
        use std::os::unix::fs::MetadataExt;
        let directory =
            std::env::temp_dir().join(format!("world-profile-{}", archie_types::TraceId::new()));
        let path = directory.join("inference.toml");
        RuntimeProfile::for_domain(scope()).create(&path).unwrap();
        let uid = std::fs::metadata(&path).unwrap().uid();
        let authority = archie_workstation::ai_policy::RuntimeAuthority::new(
            path,
            std::sync::Arc::new(std::sync::RwLock::new(PolicyEngine::default())),
        );
        assert!(authority.admit_operator_uid(uid).is_ok());
        assert!(authority.admit_operator_uid(uid.wrapping_add(1)).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
