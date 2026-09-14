//! The sequencer: the thing that calls the decisions in order.

use crate::observation::Observation;
use archie_mcp::{Admission, CallerAuthority, McpGate, McpRegistry, Refusal, ToolProposal};
use archie_memory::admission::{admit_learning, AdmissionPolicy, LearningCandidate};
use archie_memory::floor::{decide_floor, FloorOutput};
use archie_observability::{
    CognitiveTrace, Stage, StageCost, StageOutcome, StageRecord, Subject, TraceError,
};
use archie_policy::{Lease, Principal};
use archie_registry::ModelRegistry;
use archie_router::{Router, RoutingOutcome, RoutingRequest};
use archie_search::{SearchAdmission, SearchAuthority, SearchGate, SearchProposal, SearchRegistry};
use archie_stage::{StageAction, StageAdmission, StageGate, StagePolicy, StageProposal};
use archie_types::{Capability, Locality, ModelId, PrivacyGrade, Timestamp, TraceId};
use archie_vault::{ContextCapsule, ContextGateway, ContextRequest, GatewayError, Vault};
use std::collections::BTreeSet;

/// What the loop insists on, independent of any single observation.
#[derive(Debug, Clone)]
pub struct CerebraConfig {
    /// How long an assembled capsule may live.
    pub capsule_ttl_ms: i64,
    pub min_quality_tier: u8,
    /// Whether speech is available at all: a muted node, a headless one, a meeting.
    pub speech_available: bool,
    /// What the reasoning step requires of a model.
    pub inference_capability: Capability,
}

impl Default for CerebraConfig {
    fn default() -> Self {
        Self {
            capsule_ttl_ms: 60_000,
            min_quality_tier: 0,
            speech_available: true,
            inference_capability: Capability::known("compute.inference"),
        }
    }
}

/// What the pass concluded should happen next.
///
/// [`Intent::StaySilent`] is not a failure. It is the expected ending for most passes,
/// and a loop whose only expressible result is an action produces an assistant that
/// always acts.
#[derive(Debug)]
pub enum Intent {
    /// Nothing further. The reason is a stable code and is already in the trace.
    StaySilent { reason: &'static str },
    /// Context is assembled and a resource is chosen. The caller performs the inference.
    ///
    /// The capsule is moved rather than copied: it expires, it is bound to this task,
    /// and it disposes itself when dropped, so the caller cannot quietly retain it.
    Reason {
        // Boxed: this variant is far larger than `StaySilent`, and an enum sized for its
        // widest arm would make the common case — silence — expensive to return.
        capsule: Box<ContextCapsule>,
        model: ModelId,
        /// How an answer should be delivered, when a person is waiting on a turn.
        floor: Option<FloorOutput>,
    },
}

impl Intent {
    pub fn is_silent(&self) -> bool {
        matches!(self, Self::StaySilent { .. })
    }

    pub fn model(&self) -> Option<&ModelId> {
        match self {
            Self::Reason { model, .. } => Some(model),
            Self::StaySilent { .. } => None,
        }
    }

    pub fn capsule(&self) -> Option<&ContextCapsule> {
        match self {
            Self::Reason { capsule, .. } => Some(capsule),
            Self::StaySilent { .. } => None,
        }
    }
}

/// One pass: what to do, and the record of how that was decided.
#[derive(Debug)]
pub struct Pass {
    pub trace: CognitiveTrace,
    pub intent: Intent,
}

/// The cognitive loop.
///
/// # Order is the design
///
/// The floor is consulted **before** context is assembled. Deciding whether to speak is
/// cheap and touches nothing private; deciding what to say means decrypting vault
/// entries and building a capsule. In the other order, every pause mid-sentence would
/// have Archie read someone's private notes in order to conclude it should stay quiet.
///
/// A pass that ends in silence therefore leaves no capsule behind, because none was
/// ever made. There is a test that asserts exactly that, by checking the vault's audit
/// log is empty afterwards.
///
/// # What this does not do
///
/// It does not call a model, and it does not execute a tool. [`Cerebra::consider`] ends
/// by handing back a capsule and the identifier of the resource that should receive it;
/// inference happens in `archie-model-runtime` and execution in `archie-mcp`, both of
/// which are I/O. Everything here is a decision over state, which is why it is testable
/// without a network.
#[derive(Debug)]
pub struct Cerebra {
    router: Router,
    gateway: ContextGateway,
    config: CerebraConfig,
    context_utility: Option<archie_memory::utility::ContextUtilityModel>,
}

impl Cerebra {
    pub fn new(router: Router, gateway: ContextGateway, config: CerebraConfig) -> Self {
        Self {
            router,
            gateway,
            config,
            context_utility: None,
        }
    }

    pub fn config(&self) -> &CerebraConfig {
        &self.config
    }

    /// Opt in to local context ranking. With no training, the original retrieval
    /// order and set are unchanged. The floor still runs before any retrieval.
    pub fn with_context_utility(
        mut self,
        model: archie_memory::utility::ContextUtilityModel,
    ) -> Self {
        self.context_utility = Some(model);
        self
    }

    /// Observe, decide whether to act, assemble context, choose a resource.
    ///
    /// Stops at the first stage that concludes nothing should happen, and says why in
    /// the trace rather than returning a bare `None`.
    pub fn consider(
        &self,
        vault: &mut Vault,
        models: &ModelRegistry,
        observation: &Observation,
        trace_id: TraceId,
        now_ms: i64,
    ) -> Result<Pass, CerebraError> {
        let mut trace = CognitiveTrace::new(trace_id, &observation.goal_label, observation.at_ms);
        for reference in &observation.context_refs {
            trace = trace.sourced_from(Subject::VaultEntry {
                reference: reference.to_string(),
            });
        }
        // Validated before anything else runs: an over-long goal label is a caller
        // pasting an utterance in, and that should fail here rather than after the
        // vault has been opened.
        trace.validate()?;
        trace.record(StageRecord::new(
            Stage::Observe,
            observation.at_ms,
            StageOutcome::Proceeded,
        ))?;

        // 1. Should anything happen at all? Cheap, and touches nothing private.
        let floor = match self.assess_floor(&mut trace, observation, now_ms)? {
            FloorVerdict::Silent { reason } => {
                return Ok(Pass {
                    trace,
                    intent: Intent::StaySilent { reason },
                })
            }
            FloorVerdict::Proceed(output) => output,
        };

        // 2. Only now is it worth opening the vault.
        let capsule = match self.contextualise(vault, observation, now_ms) {
            Ok(capsule) => {
                trace.record(
                    StageRecord::new(Stage::Contextualize, now_ms, StageOutcome::Proceeded)
                        .handling(observation.privacy)
                        .about(Subject::Capsule {
                            id: capsule.capsule_id().to_string(),
                        }),
                )?;
                capsule
            }
            Err(error) => {
                let reason = gateway_refusal(&error);
                trace.record(
                    StageRecord::new(
                        Stage::Contextualize,
                        now_ms,
                        StageOutcome::Refused {
                            reason: reason.into(),
                        },
                    )
                    .handling(observation.privacy),
                )?;
                return Ok(Pass {
                    trace,
                    intent: Intent::StaySilent { reason },
                });
            }
        };

        // 3. Which resource is permitted to think about it?
        self.route(trace, capsule, floor, observation, models, now_ms)
    }

    fn route(
        &self,
        mut trace: CognitiveTrace,
        capsule: ContextCapsule,
        floor: Option<FloorOutput>,
        observation: &Observation,
        models: &ModelRegistry,
        now_ms: i64,
    ) -> Result<Pass, CerebraError> {
        let request = RoutingRequest::new(
            observation.privacy,
            observation.input_tokens,
            observation.max_output_tokens,
        )
        .requiring(self.config.inference_capability.clone())
        .min_quality(self.config.min_quality_tier);

        match self.router.route(models, &request) {
            RoutingOutcome::Selected { model, .. } => {
                trace.record(
                    StageRecord::new(Stage::Reason, now_ms, StageOutcome::Proceeded)
                        .handling(observation.privacy)
                        .about(Subject::Model {
                            id: model.to_string(),
                        })
                        .costing(StageCost {
                            input_tokens: Some(observation.input_tokens),
                            ..Default::default()
                        }),
                )?;
                Ok(Pass {
                    trace,
                    intent: Intent::Reason {
                        capsule: Box::new(capsule),
                        model,
                        floor,
                    },
                })
            }
            RoutingOutcome::Escalate {
                dominant_reason, ..
            } => {
                // Recorded as a failure rather than a refusal: nothing was forbidden,
                // the problem was larger than what is available. The caller is expected
                // to decompose it rather than simplify it until the local model copes,
                // and the distinction survives into the trace for whoever reads it.
                trace.record(
                    StageRecord::new(
                        Stage::Reason,
                        now_ms,
                        StageOutcome::Failed {
                            reason: sanitise(&dominant_reason),
                        },
                    )
                    .handling(observation.privacy),
                )?;
                Ok(Pass {
                    trace,
                    intent: Intent::StaySilent {
                        reason: "escalation_required",
                    },
                })
            }
            RoutingOutcome::NoModelAvailable { .. } => {
                trace.record(
                    StageRecord::new(
                        Stage::Reason,
                        now_ms,
                        StageOutcome::Refused {
                            reason: "no_model_permitted".into(),
                        },
                    )
                    .handling(observation.privacy),
                )?;
                Ok(Pass {
                    trace,
                    intent: Intent::StaySilent {
                        reason: "no_model_permitted",
                    },
                })
            }
        }
    }

    /// Put a tool call the model proposed through the gate, and record what happened.
    ///
    /// Takes a [`ToolProposal`] and never an authorised call. That is the point: this is
    /// the only route from a model's suggestion to something executable, and the type
    /// system rather than a convention is what makes it so.
    pub fn propose_tool(
        &self,
        trace: &mut CognitiveTrace,
        servers: &McpRegistry,
        authority: &CallerAuthority,
        proposal: &ToolProposal,
        now_ms: i64,
    ) -> Result<Admission, TraceError> {
        let admission = McpGate::authorize(servers, authority, proposal);
        let outcome = match &admission {
            Admission::Allowed(_) => StageOutcome::Proceeded,
            Admission::NeedsApproval { .. } => StageOutcome::AwaitingApproval {
                reason: "operator_approval_required".into(),
            },
            Admission::Refused(refusal) => StageOutcome::Refused {
                reason: refusal_code(refusal).into(),
            },
        };
        trace.record(
            StageRecord::new(Stage::SelectCapability, now_ms, outcome)
                .about(Subject::Tool {
                    server: proposal.server_id.clone(),
                    tool: proposal.tool.clone(),
                })
                .handling(proposal.argument_grade),
        )?;
        Ok(admission)
    }

    /// Put a web search the model proposed through its gate, and record what happened.
    ///
    /// The same shape as [`Cerebra::propose_tool`], for the same reason: a query is a
    /// disclosure, and a model's suggested query becomes something sendable only by
    /// passing the gate. The trace records the provider, the outcome and the grade the
    /// query was handled under. It never records the query.
    pub fn propose_search(
        &self,
        trace: &mut CognitiveTrace,
        providers: &SearchRegistry,
        authority: &SearchAuthority,
        proposal: &SearchProposal,
        now_ms: i64,
    ) -> Result<SearchAdmission, TraceError> {
        let admission = SearchGate::authorize(providers, authority, proposal);
        let outcome = match &admission {
            SearchAdmission::Allowed(_) => StageOutcome::Proceeded,
            SearchAdmission::NeedsApproval { .. } => StageOutcome::AwaitingApproval {
                reason: "operator_approval_required".into(),
            },
            SearchAdmission::Refused(refusal) => StageOutcome::Refused {
                reason: refusal.code().into(),
            },
        };
        trace.record(
            StageRecord::new(Stage::SelectCapability, now_ms, outcome)
                .about(Subject::Tool {
                    server: proposal.provider_id.clone(),
                    tool: "search".into(),
                })
                .handling(proposal.query_grade),
        )?;
        Ok(admission)
    }

    /// Put an action on an application — a click, a drag, typed text — through the stage
    /// gate, and record what happened.
    ///
    /// Recorded at [`Stage::Authorize`], because that is what the gate decides, so a hold
    /// for approval is exactly what the HUD's authorisation field shows. The trace names
    /// the kind of action and the grade of any text. It never records the text, and it
    /// does not record the application: the compositor draws that locally, and a trace is
    /// the thing that travels.
    #[allow(clippy::too_many_arguments)]
    pub fn propose_stage_action(
        &self,
        trace: &mut CognitiveTrace,
        policy: &StagePolicy,
        lease: &Lease,
        principal: &Principal,
        user_present: bool,
        proposal: &StageProposal,
        now_ms: i64,
    ) -> Result<StageAdmission, TraceError> {
        let admission = StageGate::authorize(
            policy,
            lease,
            principal,
            user_present,
            proposal,
            Timestamp::from_millis(now_ms),
        );
        let outcome = match &admission {
            StageAdmission::Allowed(_) => StageOutcome::Proceeded,
            StageAdmission::NeedsApproval { .. } => StageOutcome::AwaitingApproval {
                reason: "operator_approval_required".into(),
            },
            StageAdmission::Refused(refusal) => StageOutcome::Refused {
                reason: refusal.code().into(),
            },
        };
        let grade = match proposal.action {
            StageAction::Text { .. } => proposal.text_grade,
            _ => PrivacyGrade::Public,
        };
        trace.record(
            StageRecord::new(Stage::Authorize, now_ms, outcome)
                .about(Subject::Tool {
                    server: "stage".into(),
                    tool: proposal.action.kind().as_str().into(),
                })
                .handling(grade),
        )?;
        Ok(admission)
    }

    /// Offer what happened to memory, and record whether memory took any of it.
    ///
    /// Most of the time it should not, and a refusal here is not an error. The admission
    /// firewall is what stands between an always-on perceptual system and a store that
    /// grows without bound; a loop that treated its refusals as failures would, under
    /// pressure to make the failures stop, end up removing it.
    pub fn remember(
        &self,
        trace: &mut CognitiveTrace,
        candidates: &[LearningCandidate],
        policy: &AdmissionPolicy,
        verified_receipts: &BTreeSet<String>,
        already_admitted: &BTreeSet<String>,
        now_ms: i64,
    ) -> Result<RememberSummary, CerebraError> {
        let batch = admit_learning(
            candidates,
            policy,
            now_ms,
            verified_receipts,
            already_admitted,
        )
        .map_err(|e| CerebraError::Admission(e.to_string()))?;

        let admitted = batch.training_records().len();
        let outcome = if admitted > 0 {
            StageOutcome::Proceeded
        } else {
            StageOutcome::Silent {
                reason: "nothing_worth_keeping".into(),
            }
        };
        trace.record(StageRecord::new(Stage::Remember, now_ms, outcome))?;

        Ok(RememberSummary {
            offered: candidates.len(),
            admitted,
        })
    }

    fn assess_floor(
        &self,
        trace: &mut CognitiveTrace,
        observation: &Observation,
        now_ms: i64,
    ) -> Result<FloorVerdict, TraceError> {
        let Some(state) = &observation.floor else {
            // Nothing is waiting on a turn. A timer or a sensor reading proceeds without
            // asking for the floor, because there is no floor to take.
            return Ok(FloorVerdict::Proceed(None));
        };

        let decision = decide_floor(
            state,
            self.config.speech_available,
            observation.critical_alert,
            now_ms,
        );
        if decision.output == FloorOutput::Silent {
            trace.record(StageRecord::new(
                Stage::Interpret,
                now_ms,
                StageOutcome::Silent {
                    reason: sanitise(decision.reason),
                },
            ))?;
            return Ok(FloorVerdict::Silent {
                reason: decision.reason,
            });
        }
        trace.record(StageRecord::new(
            Stage::Interpret,
            now_ms,
            StageOutcome::Proceeded,
        ))?;
        Ok(FloorVerdict::Proceed(Some(decision.output)))
    }

    fn contextualise(
        &self,
        vault: &mut Vault,
        observation: &Observation,
        now_ms: i64,
    ) -> Result<ContextCapsule, GatewayError> {
        // Where the context may travel follows from its grade. `Secret` permits no
        // locality at all, and a capsule for it would be refused entry-by-entry anyway;
        // resolving to Local keeps the refusal specific rather than turning it into a
        // panic on an unwrap.
        let destination = observation
            .privacy
            .max_permitted_locality()
            .unwrap_or(Locality::Local);

        let mut request = ContextRequest::new(
            observation.task_id.clone(),
            observation.goal_label.clone(),
            observation.trigger.as_str(),
            destination,
            self.config.capsule_ttl_ms,
        );
        request.references = observation.context_refs.clone();
        if let Some(model) = &self.context_utility {
            let requested = request
                .references
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            request.references = model
                .select_indices(&requested, now_ms)
                .into_iter()
                .filter_map(|i| observation.context_refs.get(i).cloned())
                .collect();
        }
        self.gateway.assemble(vault, &request, now_ms)
    }
}

enum FloorVerdict {
    Silent { reason: &'static str },
    Proceed(Option<FloorOutput>),
}

/// How much memory was offered, and how much it kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RememberSummary {
    pub offered: usize,
    pub admitted: usize,
}

impl RememberSummary {
    pub fn kept_nothing(&self) -> bool {
        self.admitted == 0
    }
}

/// Turn a human-readable reason into a trace code.
///
/// The floor and the router return prose written for a person reading a log; the trace
/// refuses prose on purpose. Converting here means neither format has to be bent to suit
/// the other, and the conversion is total — every reason either crate can produce comes
/// out as something the trace accepts, which a test asserts by exhausting them.
fn sanitise(reason: &str) -> String {
    let mut out = String::with_capacity(reason.len());
    let mut last_underscore = true; // suppresses a leading underscore
    for c in reason.chars() {
        if c.is_ascii_alphanumeric() {
            out.extend(c.to_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out.truncate(64);
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("unspecified");
    }
    out
}

fn gateway_refusal(error: &GatewayError) -> &'static str {
    match error {
        GatewayError::Undisclosable { .. } => "context_undisclosable_at_destination",
        GatewayError::Retrieval { .. } => "context_retrieval_failed",
        GatewayError::Expired { .. } => "capsule_expired",
        GatewayError::Disposed => "capsule_disposed",
        GatewayError::WrongTask { .. } => "capsule_wrong_task",
        GatewayError::InvalidTtl => "capsule_invalid_ttl",
    }
}

fn refusal_code(refusal: &Refusal) -> &'static str {
    match refusal {
        Refusal::UnknownServer(_) => "server_unknown",
        Refusal::ServerDisabled(_) => "server_disabled",
        Refusal::ToolNotAllowed { .. } => "tool_not_allowlisted",
        Refusal::CapabilityNotGranted(_) => "capability_not_granted",
        Refusal::PrivacyExceeded { .. } => "arguments_exceed_server_grade",
        Refusal::SecretInArguments => "secret_in_arguments",
        Refusal::ArgumentsTooLarge { .. } => "arguments_too_large",
        Refusal::ArgumentsNotAnObject => "arguments_not_an_object",
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CerebraError {
    #[error("could not record the pass: {0}")]
    Trace(#[from] TraceError),
    #[error("learning admission failed: {0}")]
    Admission(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_becomes_a_trace_code() {
        assert_eq!(sanitise("thought not complete"), "thought_not_complete");
        assert_eq!(sanitise("a -- b"), "a_b");
        assert_eq!(sanitise("  leading and trailing  "), "leading_and_trailing");
        assert!(sanitise(&"very long reason ".repeat(20)).len() <= 64);
        // A reason made entirely of punctuation still has to produce a usable code.
        assert_eq!(sanitise("---"), "unspecified");
    }

    #[test]
    fn every_reason_the_floor_can_produce_is_acceptable_to_a_trace() {
        // Exhaustive against the literals in archie_memory::floor. If one is added there
        // that this conversion mangles, the trace would reject the record at runtime,
        // and a dropped record of a refusal is the worst thing this crate could do.
        for prose in [
            "speech unavailable or disallowed",
            "invalid or stale observation",
            "trusted critical alert",
            "acknowledgement adds no response demand",
            "retain context while user explains",
            "insufficient context or nonpositive response utility",
            "valuable bounded intervention candidate",
            "speaker likely continuing",
            "uninterrupted speech remains useful",
            "thought not complete",
            "completed question",
            "explicitly requested contribution",
            "no demonstrated demand for unsolicited speech",
        ] {
            let record = StageRecord::new(
                Stage::Interpret,
                1,
                StageOutcome::Silent {
                    reason: sanitise(prose),
                },
            );
            assert!(record.validate().is_ok(), "{prose} produced a bad code");
        }
    }

    #[test]
    fn every_router_reason_is_acceptable_to_a_trace() {
        for code in [
            archie_router::reasons::UNAVAILABLE,
            archie_router::reasons::PRIVACY,
            archie_router::reasons::RETENTION_UNVERIFIED,
            archie_router::reasons::MISSING_CAPABILITY,
            archie_router::reasons::CONTEXT_EXCEEDED,
            archie_router::reasons::OUTPUT_EXCEEDED,
            archie_router::reasons::QUALITY_TOO_LOW,
            archie_router::reasons::TOO_EXPENSIVE,
        ] {
            let record = StageRecord::new(
                Stage::Reason,
                1,
                StageOutcome::Failed {
                    reason: sanitise(code),
                },
            );
            assert!(record.validate().is_ok(), "{code} produced a bad code");
        }
    }
}
