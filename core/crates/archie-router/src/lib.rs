//! # archie-router
//!
//! Choosing which model answers a request.
//!
//! # Two stages, never merged
//!
//! 1. **Disqualification** — hard rules. A model that fails any of them is removed and
//!    can never be chosen, no matter how good it scores. Privacy lives here.
//! 2. **Scoring** — soft preferences among the survivors: quality, latency, cost,
//!    locality.
//!
//! Keeping them apart is the whole point. If privacy were merely a heavily-weighted
//! scoring term, a sufficiently attractive cloud model could outweigh it. As a
//! disqualifier it cannot.
//!
//! # The local model must not become an epistemic bottleneck
//!
//! When nothing local can handle a problem, the correct response is **not** to simplify
//! the problem until the local model understands it. [`RoutingOutcome::Escalate`]
//! preserves the original request and reports what was missing, so the caller can split
//! out specific subproblems for a more capable model. See `docs/MODEL_ROUTING.md`.

#![forbid(unsafe_code)]

use archie_registry::ModelRegistry;
use archie_types::{Capability, Locality, ModelDescriptor, ModelId, PrivacyGrade, TaskId, TraceId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What the caller needs.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutingRequest {
    /// Capabilities the model must provide, e.g. `compute.inference`.
    pub required_capabilities: Vec<Capability>,
    /// Sensitivity of the data in this request.
    pub privacy: PrivacyGrade,
    pub input_tokens: u32,
    pub max_output_tokens: u32,
    /// Refuse models below this tier. Used when a task genuinely needs reasoning depth.
    pub min_quality_tier: u8,
    /// Cost ceiling in currency micro-units. `None` means no ceiling.
    pub max_cost_micros: Option<u64>,
    /// Prefer staying local even at some quality cost.
    pub prefer_local: bool,
}

impl RoutingRequest {
    pub fn new(privacy: PrivacyGrade, input_tokens: u32, max_output_tokens: u32) -> Self {
        Self {
            required_capabilities: vec![],
            privacy,
            input_tokens,
            max_output_tokens,
            min_quality_tier: 0,
            max_cost_micros: None,
            prefer_local: false,
        }
    }

    pub fn requiring(mut self, capability: Capability) -> Self {
        self.required_capabilities.push(capability);
        self
    }

    pub fn min_quality(mut self, tier: u8) -> Self {
        self.min_quality_tier = tier;
        self
    }
}

/// Why a model was removed from consideration.
///
/// String codes rather than an enum with payloads: these end up in audit records and
/// `archie status` output, where a stable machine-readable code matters more than
/// structure.
pub mod reasons {
    pub const UNAVAILABLE: &str = "model_unavailable";
    pub const PRIVACY: &str = "privacy_grade_not_permitted";
    pub const MISSING_CAPABILITY: &str = "missing_capability";
    pub const CONTEXT_EXCEEDED: &str = "context_window_exceeded";
    pub const OUTPUT_EXCEEDED: &str = "output_limit_exceeded";
    pub const QUALITY_TOO_LOW: &str = "below_min_quality_tier";
    pub const TOO_EXPENSIVE: &str = "exceeds_cost_ceiling";
}

/// How a single model fared.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub model: ModelId,
    /// Empty when the model is eligible.
    pub disqualifiers: Vec<String>,
    /// Only meaningful when `disqualifiers` is empty.
    pub score: f64,
    pub components: BTreeMap<String, f64>,
    pub estimated_cost_micros: u64,
}

impl Candidate {
    pub fn is_eligible(&self) -> bool {
        self.disqualifiers.is_empty()
    }
}

/// Relative importance of each soft preference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    pub quality: f64,
    pub latency: f64,
    pub cost: f64,
    pub locality: f64,
}

impl Default for Weights {
    fn default() -> Self {
        // Quality dominates: Archie is a research system, and a cheap wrong answer is
        // worth less than an expensive right one. Cost still matters enough to break
        // ties between comparable models.
        Self {
            quality: 0.50,
            latency: 0.15,
            cost: 0.15,
            locality: 0.20,
        }
    }
}

impl Weights {
    fn total(&self) -> f64 {
        self.quality + self.latency + self.cost + self.locality
    }
}

/// What routing concluded.
#[derive(Debug, Clone, PartialEq)]
pub enum RoutingOutcome {
    /// A model was chosen.
    Selected {
        model: ModelId,
        score: f64,
        /// Every model considered, eligible or not.
        considered: Vec<Candidate>,
    },
    /// Nothing eligible, but a *more capable* model exists that this request's
    /// constraints excluded.
    ///
    /// The caller must not respond by weakening the problem. It should either relax an
    /// explicit constraint with the operator's knowledge, or decompose the task and
    /// route specific subproblems.
    Escalate {
        /// Why each model was excluded, so the caller knows which constraint bound.
        considered: Vec<Candidate>,
        /// The dominant reason across all candidates.
        dominant_reason: String,
    },
    /// No model at all could serve this, and no escalation would help.
    NoModelAvailable { considered: Vec<Candidate> },
}

impl RoutingOutcome {
    pub fn selected_model(&self) -> Option<&ModelId> {
        match self {
            Self::Selected { model, .. } => Some(model),
            _ => None,
        }
    }

    pub fn considered(&self) -> &[Candidate] {
        match self {
            Self::Selected { considered, .. }
            | Self::Escalate { considered, .. }
            | Self::NoModelAvailable { considered } => considered,
        }
    }

    /// Human-readable explanation of what happened.
    pub fn explain(&self) -> String {
        match self {
            Self::Selected { model, score, .. } => {
                format!("selected {model} (score {score:.3})")
            }
            Self::Escalate {
                dominant_reason, ..
            } => format!(
                "no local model is eligible ({dominant_reason}); escalate specific subproblems \
                 rather than simplifying the task"
            ),
            Self::NoModelAvailable { .. } => "no model is registered that could serve this".into(),
        }
    }
}

/// Selects models.
#[derive(Debug, Clone, Default)]
pub struct Router {
    weights: Weights,
}

impl Router {
    pub fn new(weights: Weights) -> Self {
        Self { weights }
    }

    pub fn weights(&self) -> Weights {
        self.weights
    }

    /// Route a request against the registry.
    pub fn route(&self, registry: &ModelRegistry, request: &RoutingRequest) -> RoutingOutcome {
        let mut candidates: Vec<Candidate> = registry
            .iter()
            .map(|model| self.evaluate(model, request))
            .collect();

        // Deterministic order: best score first, then model id. Two runs with the same
        // registry must produce the same choice.
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.model.as_str().cmp(b.model.as_str()))
        });

        if candidates.is_empty() {
            return RoutingOutcome::NoModelAvailable { considered: vec![] };
        }

        if let Some(best) = candidates.iter().find(|c| c.is_eligible()) {
            return RoutingOutcome::Selected {
                model: best.model.clone(),
                score: best.score,
                considered: candidates,
            };
        }

        // Nothing eligible. Decide whether this is escalatable.
        //
        // A privacy exclusion is NOT escalatable: sending restricted data to a cloud
        // model is exactly what the rule forbids, and "escalate" must never become a
        // euphemism for that. A capability or context shortfall, by contrast, is a real
        // case for decomposing the task.
        let dominant = dominant_reason(&candidates);
        let escalatable = matches!(
            dominant.as_str(),
            reasons::CONTEXT_EXCEEDED
                | reasons::OUTPUT_EXCEEDED
                | reasons::QUALITY_TOO_LOW
                | reasons::MISSING_CAPABILITY
        );

        if escalatable {
            RoutingOutcome::Escalate {
                considered: candidates,
                dominant_reason: dominant,
            }
        } else {
            RoutingOutcome::NoModelAvailable {
                considered: candidates,
            }
        }
    }

    /// Apply the hard rules, then score if the model survives.
    fn evaluate(&self, model: &ModelDescriptor, request: &RoutingRequest) -> Candidate {
        let mut disqualifiers = Vec::new();

        if !model.availability.usable() {
            disqualifiers.push(reasons::UNAVAILABLE.to_string());
        }

        // Privacy is a hard gate, checked before anything else can compensate for it.
        if !model.permits_privacy(request.privacy) {
            disqualifiers.push(reasons::PRIVACY.to_string());
        }

        for capability in &request.required_capabilities {
            if !model.provides(capability) {
                disqualifiers.push(format!(
                    "{}:{}",
                    reasons::MISSING_CAPABILITY,
                    capability.as_str()
                ));
            }
        }

        if request.max_output_tokens > model.max_output_tokens {
            disqualifiers.push(reasons::OUTPUT_EXCEEDED.to_string());
        } else if !model.fits_context(request.input_tokens, request.max_output_tokens) {
            disqualifiers.push(reasons::CONTEXT_EXCEEDED.to_string());
        }

        if model.quality_tier < request.min_quality_tier {
            disqualifiers.push(reasons::QUALITY_TOO_LOW.to_string());
        }

        let estimated_cost_micros = model.cost.estimate_micros(
            request.input_tokens as u64,
            request.max_output_tokens as u64,
        );
        if let Some(ceiling) = request.max_cost_micros {
            if estimated_cost_micros > ceiling {
                disqualifiers.push(reasons::TOO_EXPENSIVE.to_string());
            }
        }

        let (score, components) = if disqualifiers.is_empty() {
            self.score(model, request, estimated_cost_micros)
        } else {
            (f64::NEG_INFINITY, BTreeMap::new())
        };

        Candidate {
            model: model.id.clone(),
            disqualifiers,
            score,
            components,
            estimated_cost_micros,
        }
    }

    /// Weighted score over normalised components, each in 0.0..=1.0.
    fn score(
        &self,
        model: &ModelDescriptor,
        request: &RoutingRequest,
        cost_micros: u64,
    ) -> (f64, BTreeMap<String, f64>) {
        let quality = model.quality_tier as f64 / 100.0;

        // Latency: 0 ms → 1.0, decaying toward 0 as it approaches 10 s. Unknown latency
        // scores mid-range rather than best, so an unmeasured model does not win by
        // default.
        let latency = match model.latency_ema_ms {
            Some(ms) => (1.0 - (ms as f64 / 10_000.0)).clamp(0.0, 1.0),
            None => 0.5,
        };

        // Cost: free scores 1.0, decaying as cost rises. The reference point is one
        // currency unit (1e6 micros) for the call.
        let cost = (1.0 - (cost_micros as f64 / 1_000_000.0)).clamp(0.0, 1.0);

        let locality = match model.locality {
            Locality::Local => 1.0,
            Locality::Fabric => 0.7,
            Locality::Cloud => {
                if request.prefer_local {
                    0.0
                } else {
                    0.4
                }
            }
        };

        let weights = self.weights;
        let raw = quality * weights.quality
            + latency * weights.latency
            + cost * weights.cost
            + locality * weights.locality;
        let total = weights.total();
        let score = if total > 0.0 { raw / total } else { 0.0 };

        let mut components = BTreeMap::new();
        components.insert("quality".to_string(), quality);
        components.insert("latency".to_string(), latency);
        components.insert("cost".to_string(), cost);
        components.insert("locality".to_string(), locality);

        (score, components)
    }
}

/// The reason that excluded the most candidates.
fn dominant_reason(candidates: &[Candidate]) -> String {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for candidate in candidates {
        for reason in &candidate.disqualifiers {
            // Collapse `missing_capability:compute.cuda` to its family so the dominant
            // reason is a category rather than one specific capability.
            let key = reason
                .split_once(':')
                .map(|(head, _)| head.to_string())
                .unwrap_or_else(|| reason.clone());
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(reason, _)| reason)
        .unwrap_or_else(|| "unknown".to_string())
}

/// A serializable record of a routing decision, for audit and `archie status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingRecord {
    pub selected: Option<String>,
    pub explanation: String,
    pub rejected: BTreeMap<String, Vec<String>>,
}

impl RoutingRecord {
    pub fn from_outcome(outcome: &RoutingOutcome) -> Self {
        let rejected = outcome
            .considered()
            .iter()
            .filter(|c| !c.is_eligible())
            .map(|c| (c.model.to_string(), c.disqualifiers.clone()))
            .collect();
        Self {
            selected: outcome.selected_model().map(|m| m.to_string()),
            explanation: outcome.explain(),
            rejected,
        }
    }
}

/// Where an inference result may be delivered after the server-side worker completes.
///
/// This is deliberately a *result* target, not an inference placement. A paired camera,
/// router, Roku, or display may receive a small typed result, but it never receives a
/// model selection, provider endpoint, credential, prompt, or model runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HarnessResultTarget {
    ControlPlane,
    PairedDevice { device_reference: String },
}

impl HarnessResultTarget {
    fn validate(&self) -> Result<(), HarnessRequestError> {
        if let Self::PairedDevice { device_reference } = self {
            if device_reference.trim().is_empty()
                || device_reference.len() > 128
                || device_reference.contains('\0')
            {
                return Err(HarnessRequestError::InvalidDeviceReference);
            }
        }
        Ok(())
    }
}

/// An inference request that is admitted on the server, then handed later to a
/// sandboxed provider worker. The actual prompt is intentionally not part of this
/// planning contract: it stays in the provider worker's leased input channel, while the
/// immutable digest makes the planned work auditable without copying sensitive text into
/// routing logs.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteHarnessRequest {
    pub trace_id: TraceId,
    pub task_id: Option<TaskId>,
    pub routing: RoutingRequest,
    pub input_sha256: String,
    pub result_target: HarnessResultTarget,
}

impl RemoteHarnessRequest {
    pub fn new(
        trace_id: TraceId,
        task_id: Option<TaskId>,
        routing: RoutingRequest,
        input_sha256: impl Into<String>,
        result_target: HarnessResultTarget,
    ) -> Result<Self, HarnessRequestError> {
        let request = Self {
            trace_id,
            task_id,
            routing,
            input_sha256: input_sha256.into(),
            result_target,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), HarnessRequestError> {
        if self.input_sha256.len() != 64
            || !self
                .input_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(HarnessRequestError::InvalidInputDigest);
        }
        self.result_target.validate()
    }
}

/// What failed before a provider worker was even considered. These are local contract
/// failures, so retrying them unchanged would only create useless work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessRequestError {
    InvalidInputDigest,
    InvalidDeviceReference,
}

impl std::fmt::Display for HarnessRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidInputDigest => {
                "harness input must be identified by a lower-case SHA-256 digest"
            }
            Self::InvalidDeviceReference => {
                "harness result target must name one paired device reference"
            }
        })
    }
}

impl std::error::Error for HarnessRequestError {}

/// The only kind of inference plan this crate can produce. Its absence of a network
/// endpoint and credentials is intentional: provider workers own those separately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInferencePlan {
    pub trace_id: TraceId,
    pub task_id: Option<TaskId>,
    pub model: ModelId,
    pub input_sha256: String,
    pub input_tokens: u32,
    pub max_output_tokens: u32,
    pub result_target: HarnessResultTarget,
}

/// Server-side admission result. `Refused` retains the routing record so an operator
/// can see a privacy, cost, capability, or availability gate; it never weakens one.
#[derive(Debug, Clone, PartialEq)]
pub enum HarnessAdmission {
    Planned(ServerInferencePlan),
    Refused(RoutingRecord),
}

/// The server-resident model harness planner.
///
/// It deliberately does not invoke an HTTP API or a local runtime. The future provider
/// worker consumes [`ServerInferencePlan`] over the executor boundary, with a separate
/// credential and input-data channel. That keeps device agents near-zero-resource and
/// prevents a model from becoming a direct actuator.
#[derive(Debug, Clone, Default)]
pub struct RemoteHarness {
    router: Router,
}

impl RemoteHarness {
    pub fn new(router: Router) -> Self {
        Self { router }
    }

    pub fn admit(
        &self,
        registry: &ModelRegistry,
        request: &RemoteHarnessRequest,
    ) -> Result<HarnessAdmission, HarnessRequestError> {
        request.validate()?;
        let outcome = self.router.route(registry, &request.routing);
        match outcome {
            RoutingOutcome::Selected { model, .. } => {
                Ok(HarnessAdmission::Planned(ServerInferencePlan {
                    trace_id: request.trace_id,
                    task_id: request.task_id,
                    model,
                    input_sha256: request.input_sha256.clone(),
                    input_tokens: request.routing.input_tokens,
                    max_output_tokens: request.routing.max_output_tokens,
                    result_target: request.result_target.clone(),
                }))
            }
            refused => Ok(HarnessAdmission::Refused(RoutingRecord::from_outcome(
                &refused,
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_types::{Availability, DataRetention, ProviderKind, TokenCost};

    fn model(id: &str, provider: ProviderKind, locality: Locality, quality: u8) -> ModelDescriptor {
        ModelDescriptor {
            id: ModelId::parse(id).unwrap(),
            display_name: id.into(),
            provider,
            locality,
            node: None,
            context_window: 8192,
            max_output_tokens: 2048,
            capabilities: vec![Capability::parse("compute.inference").unwrap()],
            max_privacy_grade: PrivacyGrade::Restricted,
            retention: DataRetention::Unknown,
            availability: Availability::Ready,
            cost: TokenCost::FREE,
            latency_ema_ms: Some(500),
            quality_tier: quality,
            required_accelerator_bytes: None,
            labels: BTreeMap::new(),
        }
    }

    fn local(quality: u8) -> ModelDescriptor {
        model(
            "local.qwen3",
            ProviderKind::LlamaCpp,
            Locality::Local,
            quality,
        )
    }

    fn cloud(quality: u8) -> ModelDescriptor {
        let mut m = model(
            "cloud.big",
            ProviderKind::Anthropic,
            Locality::Cloud,
            quality,
        );
        m.max_privacy_grade = PrivacyGrade::Internal;
        // A cloud model cleared for internal work has had its retention checked; an
        // unverified one is exercised separately, where it is expected to lose.
        m.retention = DataRetention::ZeroVerified;
        m.context_window = 200_000;
        m.max_output_tokens = 8192;
        m.cost = TokenCost {
            input_micros_per_mtok: 3_000_000,
            output_micros_per_mtok: 15_000_000,
        };
        m
    }

    fn registry(models: Vec<ModelDescriptor>) -> ModelRegistry {
        let mut registry = ModelRegistry::new();
        for m in models {
            registry.register(m).unwrap();
        }
        registry
    }

    fn request() -> RoutingRequest {
        RoutingRequest::new(PrivacyGrade::Internal, 1000, 512)
    }

    const INPUT_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn harness_request(
        routing: RoutingRequest,
        result_target: HarnessResultTarget,
    ) -> RemoteHarnessRequest {
        RemoteHarnessRequest::new(
            TraceId::new(),
            Some(TaskId::new()),
            routing,
            INPUT_DIGEST,
            result_target,
        )
        .unwrap()
    }

    #[test]
    fn an_eligible_model_is_selected() {
        let registry = registry(vec![local(60)]);
        let outcome = Router::default().route(&registry, &request());
        assert_eq!(
            outcome.selected_model().map(|m| m.as_str()),
            Some("local.qwen3")
        );
    }

    #[test]
    fn harness_plans_inference_on_the_server_even_when_a_device_gets_the_result() {
        let registry = registry(vec![local(60)]);
        let request = harness_request(
            request(),
            HarnessResultTarget::PairedDevice {
                device_reference: "paired-display-01".into(),
            },
        );

        let admission = RemoteHarness::default().admit(&registry, &request).unwrap();
        match admission {
            HarnessAdmission::Planned(plan) => {
                assert_eq!(plan.model.as_str(), "local.qwen3");
                assert_eq!(plan.input_sha256, INPUT_DIGEST);
                assert!(matches!(
                    plan.result_target,
                    HarnessResultTarget::PairedDevice { .. }
                ));
            }
            other => panic!("expected a server plan, got {other:?}"),
        }
    }

    #[test]
    fn harness_does_not_turn_restricted_device_work_into_cloud_inference() {
        let registry = registry(vec![cloud(100)]);
        let mut routing = request();
        routing.privacy = PrivacyGrade::Restricted;
        let request = harness_request(
            routing,
            HarnessResultTarget::PairedDevice {
                device_reference: "paired-camera-02".into(),
            },
        );

        let admission = RemoteHarness::default().admit(&registry, &request).unwrap();
        match admission {
            HarnessAdmission::Refused(record) => {
                assert!(record.selected.is_none());
                assert_eq!(
                    record.rejected.get("cloud.big").unwrap(),
                    &vec![reasons::PRIVACY.to_string()]
                );
            }
            other => panic!("restricted work must be refused, got {other:?}"),
        }
    }

    #[test]
    fn harness_rejects_an_invalid_digest_before_any_model_is_selected() {
        let request = RemoteHarnessRequest::new(
            TraceId::new(),
            None,
            request(),
            "not-a-digest",
            HarnessResultTarget::ControlPlane,
        );
        assert_eq!(
            request.unwrap_err(),
            HarnessRequestError::InvalidInputDigest
        );
    }

    #[test]
    fn an_empty_registry_reports_no_model() {
        let outcome = Router::default().route(&ModelRegistry::new(), &request());
        assert!(matches!(outcome, RoutingOutcome::NoModelAvailable { .. }));
    }

    #[test]
    fn restricted_data_never_reaches_a_cloud_model_however_good_it_is() {
        // The cloud model is far better on every soft axis. Privacy is a hard gate, so
        // it must lose anyway.
        let registry = registry(vec![local(20), cloud(100)]);
        let mut req = request();
        req.privacy = PrivacyGrade::Restricted;

        let outcome = Router::default().route(&registry, &req);
        assert_eq!(
            outcome.selected_model().map(|m| m.as_str()),
            Some("local.qwen3"),
            "a high-scoring cloud model must not be able to outweigh a privacy rule"
        );

        let cloud_candidate = outcome
            .considered()
            .iter()
            .find(|c| c.model.as_str() == "cloud.big")
            .unwrap();
        assert!(cloud_candidate
            .disqualifiers
            .contains(&reasons::PRIVACY.to_string()));
    }

    #[test]
    fn restricted_data_with_only_a_cloud_model_does_not_escalate() {
        // Escalation must never become a euphemism for "send it to the cloud anyway".
        let registry = registry(vec![cloud(100)]);
        let mut req = request();
        req.privacy = PrivacyGrade::Restricted;

        let outcome = Router::default().route(&registry, &req);
        assert!(
            matches!(outcome, RoutingOutcome::NoModelAvailable { .. }),
            "a privacy exclusion is not an escalation opportunity"
        );
    }

    #[test]
    fn a_context_overflow_escalates_rather_than_truncating() {
        // The local model cannot hold the problem. The correct outcome is escalation of
        // the intact problem, not silently shrinking it.
        let registry = registry(vec![local(60)]);
        let mut req = request();
        req.input_tokens = 100_000;

        let outcome = Router::default().route(&registry, &req);
        match &outcome {
            RoutingOutcome::Escalate {
                dominant_reason, ..
            } => assert_eq!(dominant_reason, reasons::CONTEXT_EXCEEDED),
            other => panic!("expected escalation, got {other:?}"),
        }
        assert!(outcome.explain().contains("rather than simplifying"));
    }

    #[test]
    fn a_quality_floor_escalates_when_nothing_local_is_good_enough() {
        let registry = registry(vec![local(20)]);
        let req = request().min_quality(90);
        let outcome = Router::default().route(&registry, &req);
        match outcome {
            RoutingOutcome::Escalate {
                dominant_reason, ..
            } => assert_eq!(dominant_reason, reasons::QUALITY_TOO_LOW),
            other => panic!("expected escalation, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_capability_disqualifies_and_escalates() {
        let registry = registry(vec![local(60)]);
        let req = request().requiring(Capability::parse("compute.embedding").unwrap());
        let outcome = Router::default().route(&registry, &req);
        assert!(matches!(outcome, RoutingOutcome::Escalate { .. }));
        assert!(outcome.considered()[0].disqualifiers[0].contains("compute.embedding"));
    }

    #[test]
    fn an_offline_model_is_disqualified() {
        let mut offline = local(90);
        offline.availability = Availability::Offline;
        let registry = registry(vec![offline]);
        let outcome = Router::default().route(&registry, &request());
        assert!(outcome.selected_model().is_none());
        assert!(outcome.considered()[0]
            .disqualifiers
            .contains(&reasons::UNAVAILABLE.to_string()));
    }

    #[test]
    fn a_cost_ceiling_excludes_an_expensive_model() {
        let registry = registry(vec![cloud(100)]);
        let mut req = request();
        req.max_cost_micros = Some(1);
        let outcome = Router::default().route(&registry, &req);
        assert!(outcome.considered()[0]
            .disqualifiers
            .contains(&reasons::TOO_EXPENSIVE.to_string()));
    }

    #[test]
    fn higher_quality_wins_between_two_eligible_local_models() {
        let mut weak = local(30);
        weak.id = ModelId::parse("local.weak").unwrap();
        let mut strong = local(90);
        strong.id = ModelId::parse("local.strong").unwrap();

        let registry = registry(vec![weak, strong]);
        let outcome = Router::default().route(&registry, &request());
        assert_eq!(
            outcome.selected_model().map(|m| m.as_str()),
            Some("local.strong")
        );
    }

    #[test]
    fn prefer_local_pushes_the_choice_off_the_cloud() {
        // Comparable quality; locality should decide.
        let mut near = local(70);
        near.id = ModelId::parse("local.near").unwrap();
        let mut far = cloud(75);
        far.id = ModelId::parse("cloud.far").unwrap();

        let registry = registry(vec![near, far]);
        let mut req = request();
        req.prefer_local = true;

        assert_eq!(
            Router::default()
                .route(&registry, &req)
                .selected_model()
                .map(|m| m.as_str()),
            Some("local.near")
        );
    }

    #[test]
    fn routing_is_deterministic() {
        let registry = registry(vec![local(60), cloud(60)]);
        let router = Router::default();
        let first = router
            .route(&registry, &request())
            .selected_model()
            .cloned();
        for _ in 0..10 {
            assert_eq!(
                router
                    .route(&registry, &request())
                    .selected_model()
                    .cloned(),
                first
            );
        }
    }

    #[test]
    fn identical_models_break_ties_by_id_not_registration_order() {
        let mut a = local(60);
        a.id = ModelId::parse("local.aaa").unwrap();
        let mut b = local(60);
        b.id = ModelId::parse("local.bbb").unwrap();

        let forward = registry(vec![a.clone(), b.clone()]);
        let backward = registry(vec![b, a]);
        let router = Router::default();
        assert_eq!(
            router.route(&forward, &request()).selected_model(),
            router.route(&backward, &request()).selected_model()
        );
    }

    #[test]
    fn every_decision_records_why_each_model_lost() {
        let registry = registry(vec![local(20), cloud(100)]);
        let mut req = request();
        req.privacy = PrivacyGrade::Restricted;
        let outcome = Router::default().route(&registry, &req);

        let record = RoutingRecord::from_outcome(&outcome);
        assert_eq!(record.selected.as_deref(), Some("local.qwen3"));
        assert!(record.rejected.contains_key("cloud.big"));
        assert!(!record.explanation.is_empty());
    }

    #[test]
    fn score_components_are_all_reported() {
        let registry = registry(vec![local(60)]);
        let outcome = Router::default().route(&registry, &request());
        let winner = &outcome.considered()[0];
        for key in ["quality", "latency", "cost", "locality"] {
            assert!(
                winner.components.contains_key(key),
                "missing score component {key}"
            );
        }
        assert!(winner.score >= 0.0 && winner.score <= 1.0);
    }

    #[test]
    fn unknown_latency_does_not_score_as_perfect() {
        let mut unmeasured = local(60);
        unmeasured.id = ModelId::parse("local.unmeasured").unwrap();
        unmeasured.latency_ema_ms = None;
        let mut fast = local(60);
        fast.id = ModelId::parse("local.fast").unwrap();
        fast.latency_ema_ms = Some(10);

        let registry = registry(vec![unmeasured, fast]);
        assert_eq!(
            Router::default()
                .route(&registry, &request())
                .selected_model()
                .map(|m| m.as_str()),
            Some("local.fast"),
            "an unmeasured model must not beat a measured fast one by default"
        );
    }

    #[test]
    fn output_limit_is_reported_distinctly_from_context_overflow() {
        let registry = registry(vec![local(60)]);
        let mut req = request();
        req.max_output_tokens = 100_000;
        let outcome = Router::default().route(&registry, &req);
        assert!(outcome.considered()[0]
            .disqualifiers
            .contains(&reasons::OUTPUT_EXCEEDED.to_string()));
    }

    #[test]
    fn sensitive_data_may_use_a_fabric_model_but_not_the_cloud() {
        let mut fabric = model("fabric.mid", ProviderKind::Vllm, Locality::Fabric, 70);
        fabric.id = ModelId::parse("fabric.mid").unwrap();
        let registry = registry(vec![fabric, cloud(100)]);

        let mut req = request();
        req.privacy = PrivacyGrade::Sensitive;
        assert_eq!(
            Router::default()
                .route(&registry, &req)
                .selected_model()
                .map(|m| m.as_str()),
            Some("fabric.mid")
        );
    }
}
