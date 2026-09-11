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
}

impl PrivacyGrade {
    /// The most permissive locality allowed for data of this grade.
    pub fn max_permitted_locality(&self) -> Locality {
        match self {
            Self::Public | Self::Internal => Locality::Cloud,
            Self::Sensitive => Locality::Fabric,
            Self::Restricted => Locality::Local,
        }
    }

    /// Whether data of this grade may be sent to `locality`.
    pub fn permits(&self, locality: Locality) -> bool {
        locality <= self.max_permitted_locality()
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Sensitive => "sensitive",
            Self::Restricted => "restricted",
        }
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
    /// Two independent gates: the model's own clearance, and the physical locality.
    /// Both must pass — a "restricted-cleared" model that happens to run in the cloud
    /// is still disqualified for restricted data.
    pub fn permits_privacy(&self, grade: PrivacyGrade) -> bool {
        grade <= self.max_privacy_grade && grade.permits(self.locality)
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
        let m = model("cloud.gpt", ProviderKind::OpenAi, Locality::Cloud);
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
