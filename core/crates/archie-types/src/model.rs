//! Models and providers.
//!
//! # Models are resources, not identity
//!
//! Archie Core is the single user-facing intelligence. The model behind any particular
//! inference is an implementation detail that can be swapped, upgraded, or run on
//! another continent. Nothing in this module is allowed to leak upward as "who Archie
//! is" — it describes *what compute is available to think with*.

use crate::capability::Capability;
use crate::ids::{ModelId, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Where inference physically happens. This is the axis privacy decisions turn on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Locality {
    /// This machine.
    Local,
    /// Another node in the operator's own fabric.
    Fabric,
    /// A third-party API. Data leaves the operator's control.
    Cloud,
}

impl Locality {
    /// Whether sending data here means disclosing it to a third party.
    pub fn discloses_to_third_party(&self) -> bool {
        matches!(self, Self::Cloud)
    }
}

/// The sensitivity grade of the data in a request.
///
/// Ordered from least to most restrictive so that comparisons read naturally:
/// a model is eligible only if it is cleared for *at least* the request's grade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrivacyGrade {
    /// Freely shareable.
    Public,
    /// Operator's own non-sensitive work.
    Internal,
    /// Must not leave the operator's fabric.
    Sensitive,
    /// Must not leave this machine.
    Restricted,
    /// Must not reach a model at all — not even one running on this machine.
    ///
    /// Every other grade answers *where* inference may happen. This one answers
    /// *whether*, and the answer is no. Passwords, API keys, private keys and raw
    /// tokens have no semantic contribution to make to a prompt: a model that has
    /// been told a password reasons no better for it, and the disclosure cannot be
    /// withdrawn. Such material is held in the vault and reaches the outbound request
    /// through a credential broker at the execution boundary, never through context.
    Secret,
}

impl PrivacyGrade {
    /// The most permissive locality allowed for data of this grade.
    ///
    /// `None` means no locality is permitted at all: the data may not reach a model
    /// even locally. Callers must treat `None` as a refusal, not as "unconstrained" —
    /// which is why this returns an `Option` rather than defaulting to [`Locality::Local`].
    pub fn max_permitted_locality(&self) -> Option<Locality> {
        match self {
            Self::Public | Self::Internal => Some(Locality::Cloud),
            Self::Sensitive => Some(Locality::Fabric),
            Self::Restricted => Some(Locality::Local),
            Self::Secret => None,
        }
    }

    /// Whether data of this grade may be sent to `locality`.
    pub fn permits(&self, locality: Locality) -> bool {
        matches!(self.max_permitted_locality(), Some(max) if locality <= max)
    }

    /// Whether data of this grade may reach a model anywhere.
    pub fn reaches_models(&self) -> bool {
        self.max_permitted_locality().is_some()
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Sensitive => "sensitive",
            Self::Restricted => "restricted",
            Self::Secret => "secret",
        }
    }
}

/// What a provider does with the data a request discloses to it.
///
/// # Offered is not verified
///
/// A provider advertising zero data retention and *this deployment* having confirmed
/// it are different facts, and only the second one is a safety property. Configuring
/// an endpoint does not establish it; an operator (or a provisioning step that checked
/// the account's actual setting) does. So the default is [`DataRetention::Unknown`],
/// and unknown is treated exactly like [`DataRetention::Retains`] — the failure mode of
/// guessing wrong here is an unrecallable disclosure.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum DataRetention {
    /// Nothing has been established. Treated as [`DataRetention::Retains`].
    #[default]
    Unknown,
    /// The provider keeps request data — for training, abuse monitoring, or logs.
    Retains,
    /// Zero retention, verified for this deployment rather than read off a brochure.
    ZeroVerified,
}

impl DataRetention {
    /// Whether a third party holding this posture may receive non-public data.
    pub fn admits_private_disclosure(&self) -> bool {
        matches!(self, Self::ZeroVerified)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Retains => "retains",
            Self::ZeroVerified => "zero-verified",
        }
    }
}

impl fmt::Display for DataRetention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for PrivacyGrade {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The runtime that serves a model.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    LlamaCpp,
    Vllm,
    Sglang,
    /// NVIDIA-specific serving stack.
    NvidiaTriton,
    /// NVIDIA NIM speaking the OpenAI-compatible inference API.
    NvidiaNim,
    /// Nebius Token Factory's OpenAI-compatible cloud API.
    NebiusTokenFactory,
    Tenstorrent,
    OpenAi,
    Anthropic,
    Kimi,
    /// Any endpoint speaking the OpenAI wire format.
    OpenAiCompatible,
    /// Reserved for the future first-party Archie inference runtime.
    ArchieRuntime,
    Other(String),
}

impl ProviderKind {
    /// The locality this provider kind necessarily implies, where it is fixed.
    ///
    /// `None` means it depends on deployment — vLLM can be local or on a fabric node,
    /// so the manifest must say which.
    pub fn implied_locality(&self) -> Option<Locality> {
        match self {
            Self::OpenAi | Self::Anthropic | Self::Kimi | Self::NebiusTokenFactory => {
                Some(Locality::Cloud)
            }
            Self::LlamaCpp => Some(Locality::Local),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::LlamaCpp => "llama-cpp",
            Self::Vllm => "vllm",
            Self::Sglang => "sglang",
            Self::NvidiaTriton => "nvidia-triton",
            Self::NvidiaNim => "nvidia-nim",
            Self::NebiusTokenFactory => "nebius-token-factory",
            Self::Tenstorrent => "tenstorrent",
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Kimi => "kimi",
            Self::OpenAiCompatible => "openai-compatible",
            Self::ArchieRuntime => "archie-runtime",
            Self::Other(name) => name,
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a model can currently be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Availability {
    Ready,
    /// Registered but not loaded into memory.
    Cold,
    /// Temporarily unreachable.
    Offline,
    /// Administratively turned off.
    Disabled,
}

impl Availability {
    pub fn usable(&self) -> bool {
        matches!(self, Self::Ready | Self::Cold)
    }
}

/// Cost in micro-units of currency per million tokens.
///
/// Integer micro-units rather than floats: routing decisions must be reproducible, and
/// accumulating float costs across thousands of calls drifts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenCost {
    pub input_micros_per_mtok: u64,
    pub output_micros_per_mtok: u64,
}

impl TokenCost {
    pub const FREE: Self = Self {
        input_micros_per_mtok: 0,
        output_micros_per_mtok: 0,
    };

    /// Estimated cost in micro-units for a call of this shape.
    pub fn estimate_micros(&self, input_tokens: u64, output_tokens: u64) -> u64 {
        let input = self.input_micros_per_mtok.saturating_mul(input_tokens) / 1_000_000;
        let output = self.output_micros_per_mtok.saturating_mul(output_tokens) / 1_000_000;
        input.saturating_add(output)
    }

    pub fn is_free(&self) -> bool {
        *self == Self::FREE
    }
}

/// A registered model, described well enough for the router to choose it without
/// calling it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    pub id: ModelId,
    pub display_name: String,
    pub provider: ProviderKind,
    pub locality: Locality,
    /// Which fabric node serves it. `None` for cloud providers.
    pub node: Option<NodeId>,

    pub context_window: u32,
    pub max_output_tokens: u32,
    /// What this model can do, e.g. `compute.inference`, `compute.embedding`.
    pub capabilities: Vec<Capability>,
    /// The most sensitive grade of data permitted to reach this model.
    pub max_privacy_grade: PrivacyGrade,
    /// What the serving party does with request data. Only consulted for providers
    /// that disclose to a third party; local and fabric inference never leaves the
    /// operator's control, so retention there is the operator's own business.
    #[serde(default)]
    pub retention: DataRetention,

    pub availability: Availability,
    pub cost: TokenCost,
    /// Observed latency, used as a prior before history accumulates.
    pub latency_ema_ms: Option<u32>,
    /// Rough capability tier, 0..=100. Operator-assigned, not a benchmark score.
    pub quality_tier: u8,
    /// Accelerator memory needed to load it, where known.
    pub required_accelerator_bytes: Option<u64>,

    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

impl ModelDescriptor {
    /// Whether this model may legally receive data of `grade`.
    ///
    /// Three independent gates: the model's own clearance, the physical locality, and
    /// — for third parties only — verified retention. All must pass. A
    /// "restricted-cleared" model that happens to run in the cloud is still
    /// disqualified for restricted data, and a cloud model of unknown retention is
    /// disqualified for anything that is not already public.
    ///
    /// [`PrivacyGrade::Secret`] fails the locality gate for every model everywhere,
    /// which is the property the whole grade exists for.
    pub fn permits_privacy(&self, grade: PrivacyGrade) -> bool {
        self.permits_privacy_grade(grade) && self.retention_admits(grade)
    }

    /// The clearance and locality halves alone, without the retention gate.
    ///
    /// Exists so a caller can tell the two refusals apart when reporting one. "This
    /// model may not see restricted data" and "this provider has not had its retention
    /// verified" are different problems with different remedies — the first needs a
    /// different model, the second needs someone to check an account setting — and a
    /// single `privacy` disqualifier in a routing trace would hide which one bound.
    /// Enforcement still goes through [`ModelDescriptor::permits_privacy`].
    pub fn permits_privacy_grade(&self, grade: PrivacyGrade) -> bool {
        grade <= self.max_privacy_grade && grade.permits(self.locality)
    }

    /// Whether this model's retention posture admits data of `grade`.
    ///
    /// Local and fabric inference stays inside the operator's control, so retention is
    /// not consulted. For a third party, anything above [`PrivacyGrade::Public`]
    /// requires [`DataRetention::ZeroVerified`].
    pub fn retention_admits(&self, grade: PrivacyGrade) -> bool {
        grade == PrivacyGrade::Public
            || !self.locality.discloses_to_third_party()
            || self.retention.admits_private_disclosure()
    }

    pub fn provides(&self, required: &Capability) -> bool {
        self.capabilities.iter().any(|c| c.satisfies(required))
    }

    /// Whether a request of this shape fits the context window.
    pub fn fits_context(&self, input_tokens: u32, output_tokens: u32) -> bool {
        output_tokens <= self.max_output_tokens
            && input_tokens.saturating_add(output_tokens) <= self.context_window
    }

    /// Whether the declared locality contradicts what the provider kind implies.
    ///
    /// Catches a manifest that claims an OpenAI model is `local`, which would silently
    /// defeat every privacy rule above.
    pub fn locality_is_consistent(&self) -> bool {
        match self.provider.implied_locality() {
            Some(implied) => implied == self.locality,
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, provider: ProviderKind, locality: Locality) -> ModelDescriptor {
        ModelDescriptor {
            id: ModelId::parse(id).unwrap(),
            display_name: "Test".into(),
            provider,
            locality,
            node: None,
            context_window: 8192,
            max_output_tokens: 2048,
            capabilities: vec![Capability::known("compute.inference")],
            max_privacy_grade: PrivacyGrade::Restricted,
            retention: DataRetention::Unknown,
            availability: Availability::Ready,
            cost: TokenCost::FREE,
            latency_ema_ms: None,
            quality_tier: 50,
            required_accelerator_bytes: None,
            labels: BTreeMap::new(),
        }
    }

    #[test]
    fn restricted_data_stays_on_this_machine() {
        assert!(PrivacyGrade::Restricted.permits(Locality::Local));
        assert!(!PrivacyGrade::Restricted.permits(Locality::Fabric));
        assert!(!PrivacyGrade::Restricted.permits(Locality::Cloud));
    }

    #[test]
    fn sensitive_data_may_use_the_fabric_but_not_the_cloud() {
        assert!(PrivacyGrade::Sensitive.permits(Locality::Local));
        assert!(PrivacyGrade::Sensitive.permits(Locality::Fabric));
        assert!(!PrivacyGrade::Sensitive.permits(Locality::Cloud));
    }

    #[test]
    fn public_data_may_go_anywhere() {
        for locality in [Locality::Local, Locality::Fabric, Locality::Cloud] {
            assert!(PrivacyGrade::Public.permits(locality));
        }
    }

    #[test]
    fn cloud_model_is_disqualified_for_restricted_data_despite_clearance() {
        // The model claims it may hold restricted data, but it lives in the cloud.
        // Locality must veto the clearance, not the other way round.
        //
        // Retention is verified here so that this test keeps failing for the reason it
        // was written for: with an unverified provider the internal case below would
        // pass for the wrong reason, and the locality veto would go unexercised.
        let mut m = model("cloud.gpt", ProviderKind::OpenAi, Locality::Cloud);
        m.retention = DataRetention::ZeroVerified;
        assert_eq!(m.max_privacy_grade, PrivacyGrade::Restricted);
        assert!(!m.permits_privacy(PrivacyGrade::Restricted));
        assert!(!m.permits_privacy(PrivacyGrade::Sensitive));
        assert!(m.permits_privacy(PrivacyGrade::Internal));
    }

    #[test]
    fn model_clearance_also_constrains() {
        let mut m = model("local.small", ProviderKind::LlamaCpp, Locality::Local);
        m.max_privacy_grade = PrivacyGrade::Internal;
        assert!(m.permits_privacy(PrivacyGrade::Internal));
        assert!(!m.permits_privacy(PrivacyGrade::Sensitive));
    }

    #[test]
    fn detects_locality_contradicting_provider() {
        let bad = model("sneaky.gpt", ProviderKind::OpenAi, Locality::Local);
        assert!(
            !bad.locality_is_consistent(),
            "an OpenAI model cannot be local"
        );
        let good = model("cloud.gpt", ProviderKind::OpenAi, Locality::Cloud);
        assert!(good.locality_is_consistent());
        // Providers with no fixed locality are always consistent.
        assert!(model("x.vllm", ProviderKind::Vllm, Locality::Fabric).locality_is_consistent());
    }

    #[test]
    fn context_fitting_accounts_for_output() {
        let m = model("local.small", ProviderKind::LlamaCpp, Locality::Local);
        assert!(m.fits_context(4000, 2000));
        assert!(!m.fits_context(7000, 2000), "8192 window exceeded");
        assert!(!m.fits_context(10, 4096), "output limit exceeded");
    }

    #[test]
    fn cost_estimate_is_integer_and_saturating() {
        let cost = TokenCost {
            input_micros_per_mtok: 3_000_000,
            output_micros_per_mtok: 15_000_000,
        };
        // 1M input + 1M output = 3.0 + 15.0 currency units, in micros.
        assert_eq!(cost.estimate_micros(1_000_000, 1_000_000), 18_000_000);
        assert_eq!(cost.estimate_micros(0, 0), 0);
        assert!(TokenCost::FREE.is_free());
        // Absurd inputs must not panic on overflow.
        let _ = cost.estimate_micros(u64::MAX, u64::MAX);
    }

    #[test]
    fn secret_data_reaches_no_model_anywhere() {
        // The other grades answer "where". This one answers "whether", and no
        // locality — not even this machine — is an answer to it.
        assert!(!PrivacyGrade::Secret.reaches_models());
        for locality in [Locality::Local, Locality::Fabric, Locality::Cloud] {
            assert!(!PrivacyGrade::Secret.permits(locality));
        }
        // Including against a model that declares itself cleared for secrets, which is
        // the case a single-gate implementation would wave through.
        let mut m = model("local.qwen3", ProviderKind::LlamaCpp, Locality::Local);
        m.max_privacy_grade = PrivacyGrade::Secret;
        assert!(!m.permits_privacy(PrivacyGrade::Secret));
    }

    #[test]
    fn every_other_grade_still_reaches_a_model_somewhere() {
        // Guards against the Secret arm being written in a way that swallows its
        // neighbours — a refusal that refuses everything is not a privacy control.
        for grade in [
            PrivacyGrade::Public,
            PrivacyGrade::Internal,
            PrivacyGrade::Sensitive,
            PrivacyGrade::Restricted,
        ] {
            assert!(grade.reaches_models(), "{grade} must remain servable");
            assert!(grade.permits(Locality::Local));
        }
    }

    #[test]
    fn unverified_cloud_retention_blocks_private_data_but_not_public() {
        let mut cloud = model("cloud.big", ProviderKind::OpenAi, Locality::Cloud);
        cloud.max_privacy_grade = PrivacyGrade::Internal;

        for retention in [DataRetention::Unknown, DataRetention::Retains] {
            cloud.retention = retention;
            assert!(
                !cloud.permits_privacy(PrivacyGrade::Internal),
                "{retention} must not receive internal data"
            );
            // Public data is public: retention is irrelevant to it.
            assert!(cloud.permits_privacy(PrivacyGrade::Public));
        }

        cloud.retention = DataRetention::ZeroVerified;
        assert!(cloud.permits_privacy(PrivacyGrade::Internal));
    }

    #[test]
    fn retention_is_not_consulted_for_inference_the_operator_already_controls() {
        // Retention describes what a *third party* does. A local model of "unknown"
        // retention is not a disclosure at all, and treating it as one would push work
        // to the cloud in the name of privacy.
        for locality in [Locality::Local, Locality::Fabric] {
            let mut m = model("m", ProviderKind::Vllm, locality);
            m.retention = DataRetention::Unknown;
            assert!(m.permits_privacy(PrivacyGrade::Sensitive));
        }
    }

    #[test]
    fn retention_defaults_to_unknown_when_a_manifest_omits_it() {
        // An older manifest that predates this field must not be read as a zero
        // retention claim nobody made.
        let mut m = model("cloud.big", ProviderKind::OpenAi, Locality::Cloud);
        m.retention = DataRetention::ZeroVerified;
        let mut json: serde_json::Value = serde_json::to_value(&m).unwrap();
        json.as_object_mut().unwrap().remove("retention");
        let parsed: ModelDescriptor = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.retention, DataRetention::Unknown);
        assert!(!parsed.permits_privacy(PrivacyGrade::Internal));
    }

    #[test]
    fn availability_gates_usability() {
        assert!(Availability::Ready.usable());
        assert!(Availability::Cold.usable());
        assert!(!Availability::Offline.usable());
        assert!(!Availability::Disabled.usable());
    }

    #[test]
    fn capability_matching_honours_specificity() {
        let mut m = model("local.embed", ProviderKind::LlamaCpp, Locality::Local);
        m.capabilities = vec![Capability::known("compute.embedding.text")];
        assert!(m.provides(&Capability::known("compute.embedding")));
        assert!(!m.provides(&Capability::known("compute.reranking")));
    }

    #[test]
    fn descriptor_round_trips_through_json() {
        let m = model("local.qwen3", ProviderKind::LlamaCpp, Locality::Local);
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<ModelDescriptor>(&json).unwrap(), m);
    }

    #[test]
    fn cloud_providers_disclose_to_third_parties() {
        assert!(Locality::Cloud.discloses_to_third_party());
        assert!(!Locality::Local.discloses_to_third_party());
        assert!(!Locality::Fabric.discloses_to_third_party());
    }
}
