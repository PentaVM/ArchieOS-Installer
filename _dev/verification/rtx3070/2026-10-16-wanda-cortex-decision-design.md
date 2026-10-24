# Structured decision_v1 context for Cortex — compatible design

Scope: transmit Wanda's concise strategic analysis as unprivileged context while keeping the Core advisory contract of exactly six fields unchanged. This describes a proposed typed channel; it does not claim the endpoint, Rust structs or Cortex consumer already exist.

## Existing contracts to preserve

- Core `archie-cerebra/src/advisory.rs::evaluate_output` consumes exactly proposal_id, hypothesis_id, stance, score_permille, evidence_refs, produced_at_ms. No decision_ref, rationale, expiry or seventh field is added. A decision document alone cannot pass this gate or count as independent evidence.
- Wanda PR38 already has `archie-wanda/src/types.rs::Recommendation` and QueryResponse, `advice.rs::validate`, local-only WandaClient and existing voice rendering. Keep these wire formats and validation behavior intact. No new strategic text is concatenated into the six-field JSON or automatically spoken by an old client.
- Cortex already consumes provenance-backed GraphSnapshot nodes and a versioned ContextRequest. `NodeKind::Document`, provenance_ref and existing sensitivity/context budgets are sufficient for an initial reference projection. Do not add a message type to Cortex wirev1/v2/v3 or pretend a new cognitive-event payload is already supported.

## Channel records

Producer: a versioned Wanda strategic output parser validates model-generated content against admitted evidence and people. A trusted caller then assigns correlation, domain, model/config identity, digest and timestamps. Neither the model's claimed identity nor its self-assigned domain grants trust. Store the accepted document through the existing domain-scoped Vault writer as an **inferred analysis** record, not a verified fact or relationship.

The host-authored envelope has exactly: `schema_version:1`, `kind:"wanda.decision_v1"`, `decision_id`, `task_id`, `domain`, `produced_at_ms`, `expires_at_ms`, `source_run_ref`, `objective_refs`, `evidence_refs`, `analysis`. All identifiers are bounded192bytes; task id also respects the existing128byte task limit. UTC epoch-ms times are positive signed64bit; expiry is after production and no more than24hours later, shortened to the admitted evidence's expiry when applicable. Document size<=16KiB UTF8. Objective refs<=8; evidence refs1..64distinct. These refs are checked against the admitted same-domain context, not merely string-shaped Vault references.

`analysis` contains exactly:

- `user_objectives`:1..8 records `{objective_ref, priority, horizon, summary}`; priority is an ordinal1..8 tied to what the user actually stated, not a model-created moral objective; horizon/summary each<=500characters.
- `alternatives`:1..8 records `{option_id, action_summary, supporting_refs, uncertainties, consequences, relational_costs}`. option_id is a unique bounded identifier; action_summary<=1200characters. These are descriptions and never executable tool calls.
- `selected_option_id`:one existing option id or null when the analysis does not yet choose.
- `justification`:concise evidence-linked explanation<=1200characters, not a request for hidden chain-of-thought.
- `uncertainty`: `{missing_information, competing_explanations, forecasts}`; each list<=8; each text<=500characters. A forecast contains `{event, probability_permille, resolution_condition, evidence_refs}` with probability0..1000, event/resolution<=500characters. Forecast probability is distinct from Core's score_permille; no automatic identity/conversion is assumed.
- `revision_conditions`:1..8 strings<=500characters describing facts/feedback that would change the recommendation.

An alternative's `supporting_refs` are<=16 admitted references; uncertainties<=8 short descriptions; consequences/relational_costs each<=8 records `{description, horizon, evidence_refs}` with bounded500character description/horizon and<=16 admitted refs. A consequence can be hypothetical and must be presented as such; it does not become observed evidence. Keep the envelope's16KiB ceiling even when every list is within its individual bound. Reject unknown keys, overlong/malformed data, duplicate option ids, selected unknown options, nonfinite/out-of-range probabilities and references outside the admitted domain. These are contract checks, not moral filters.

## Storage, correlation and Cortex consumption

Use the existing domain.reference mechanism for `wanda/decisions/<host-decision-id>` and a companion trusted correlation record binding task/trace, hypothesis/proposal ids, existing Recommendation ids when applicable, source model receipt and decision digest. That correlation lives outside ModelOutput; the model cannot create a trusted association by repeating an id. Do not change existing Recommendation or QueryResponse fields merely to carry it.

A versioned read interface, separately negotiated by new clients, returns the decision by opaque host-issued id under the same local authentication/domain admission as Wanda. No redirects/public endpoint or inferred cross-domain permission is introduced. Old voice and Core clients continue using their existing endpoints and payloads. Initial implementation may use a read-only local bridge returning a Vault reference rather than expose full strategic text. If there is no compatible consumer, report unavailable while preserving the old response; do not silently dump the analysis into an unrelated field.

The trusted graph projector may add a `Document` node with `provenance_ref=decision_ref`, its inferred standing in the bounded summary and the existing explicit sensitivity/domain admission. The graph writer, not Wanda/Cortex, decides admission. Cortex can retrieve that node through existing context frames and cite its reference. A reader can resolve the full structured document only through its normal domain-scoped context gateway. It must preserve 'model analysis' provenance and never count the projection and its original cited messages as independent corroborating sources.

Consumer behavior: validate version/digest/domain/expiry/correlation, read alternatives/uncertainty/consequences/costs/objectives, then make its own bounded proposal through existing Core gates. There are no permissions, leases, policy overrides, execution targets or authority-changing fields in this channel. It cannot verify facts, establish consent/relationships, dispatch tools, change priorities silently or authorize learning. No moral classifier, ethics reward or third refusal layer is introduced. Existing contracts continue to apply independently of a recommendation's topic or eloquence.

## Required compatibility tests and rollout

1. Existing six-field fixtures still parse identically; injecting decision fields still fails their strict parser. Existing Recommendation/QueryResponse old-client fixtures remain byte/schema compatible.
2. Strategic document round trip preserves each named semantic dimension. Reject wrong domain/task, mismatched correlation/digest, stale/oversized/unknown version, forged refs/persons, unsupported keys and selected nonexistent alternatives. Negative tests are structural/authority cases, not a moral topic score.
3. Projection is typed Document/inferred provenance, contributes no independent evidence vote and can be retrieved with original v1/v2/v3 requests; existing wire fixtures unchanged.
4. Old clients never receive new payloads without negotiation; absent/failed strategic channel preserves existing advisory/voice operation. No reasoning text is admitted to training automatically.
5. Real local query→validated document→Vault reference→Cortex retrieval→unchanged Core proposal is an integration test after training mechanics. Until it runs, mark channel designed/implemented/tested separately; do not claim live Cortex delivery from a JSON fixture.

Implementation follows the [trainer plan](2026-10-16-wanda-trainer-plan.md)'s independent integration task after initial native parity/backward/real-update gates. It does not block those gates or require GUI/WSL work.
