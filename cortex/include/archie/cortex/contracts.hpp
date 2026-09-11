// Archie Analytical Cortex — typed, unprivileged analytical contracts.
//
// RFC: docs/RFC-009-ANALYTICAL-CORTEX.md
//
// The Cortex learns how to retrieve and organize graph-backed knowledge. It is not the
// knowledge authority: it cannot assert verified facts, modify a lease, execute a task,
// or directly change a scheduler/policy decision. Its outputs are context bundles and
// typed proposals for the already-authoritative layers to evaluate.

#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace archie::cortex {

constexpr std::size_t kMaximumGraphNodes = 4'096;
constexpr std::size_t kMaximumGraphEdges = 16'384;
constexpr std::size_t kMaximumIdentifierBytes = 192;
constexpr std::size_t kMaximumSummaryBytes = 8'192;
constexpr std::size_t kMaximumQuestionBytes = 16'384;
constexpr std::uint32_t kMinimumContextTokenBudget = 64;
constexpr std::uint32_t kMaximumContextTokenBudget = 32'768;
constexpr std::size_t kMaximumCorrectionTargets = 128;
constexpr std::size_t kMaximumCorrectionContextIds = 4'096;

enum class NodeKind {
    Person,
    Message,
    Conversation,
    Claim,
    Event,
    Place,
    Device,
    Sensor,
    Project,
    Task,
    Process,
    Model,
    Document,
    Paper,
    Equation,
    Component,
    Simulation,
    Company,
    Country,
    NewsArticle,
    ProviderObservation,
    Episode,
    Unknown,
};

enum class EdgeKind {
    Said,
    Mentions,
    ReplyTo,
    Knows,
    FriendOf,
    ParentOf,
    WorksWith,
    Owns,
    LocatedAt,
    Uses,
    DependsOn,
    Produced,
    Contradicts,
    Supports,
    DerivedFrom,
    ObservedBy,
    RelatedTo,
    ExecutedOn,
    BlockedBy,
};

// These are labels on graph data, not a hierarchy. A caller must state each permitted
// class explicitly; requesting PersonalPrivate does not imply Biometric or Secret.
enum class Sensitivity {
    Public,
    LocalRestricted,
    PersonalPrivate,
    Secret,
    Biometric,
    Location,
};

enum class CorrectionKind {
    EntityCorrection,
    RelationCorrection,
    CoreferenceCorrection,
    RetrievalPositive,
    RetrievalNegative,
    MissingContextLabel,
    StaleKnowledgeCorrection,
    ContextPreference,
    AnswerFailure,
};

enum class ModelBackend {
    DeterministicBaseline,
    OnnxRuntime,
    TensorRt,
};

enum class ProposalKind {
    CandidatePolicyChange,
    CandidateSchedulerChange,
    CandidateContractChange,
    KnowledgeGap,
};

enum class ContractRefusal {
    EmptyIdentifier,
    IdentifierTooLong,
    DuplicateNode,
    DuplicateEdge,
    UnknownEdgeEndpoint,
    GraphTooLarge,
    SummaryTooLong,
    MissingProvenance,
    InvalidTimestamp,
    EmptyQuestion,
    QuestionTooLong,
    InvalidTokenBudget,
    InvalidExpansionDepth,
    MissingSensitivityPolicy,
    MalformedIpcFrame,
    SnapshotUnavailable,
    SnapshotVersionMismatch,
    InvalidModelManifest,
    UnpromotedModel,
    InvalidCorrection,
};

[[nodiscard]] std::string_view name(NodeKind value) noexcept;
[[nodiscard]] std::string_view name(EdgeKind value) noexcept;
[[nodiscard]] std::string_view name(Sensitivity value) noexcept;
[[nodiscard]] std::string_view name(CorrectionKind value) noexcept;
[[nodiscard]] std::string_view name(ModelBackend value) noexcept;
[[nodiscard]] std::string_view name(ProposalKind value) noexcept;
[[nodiscard]] std::string_view describe(ContractRefusal value) noexcept;

// The immutable read view supplied by the trusted knowledge layer. The Cortex may cache
// derived scores for this snapshot version but must never change it.
struct GraphNode {
    std::string id{};
    NodeKind kind = NodeKind::Unknown;
    std::string summary{};
    std::vector<std::string> aliases{};
    std::string provenance_ref{};
    Sensitivity sensitivity = Sensitivity::LocalRestricted;
    std::int64_t observed_at_ms = 0;
    std::uint64_t version = 0;
};

struct GraphEdge {
    std::string id{};
    std::string source_id{};
    std::string target_id{};
    EdgeKind kind = EdgeKind::RelatedTo;
    std::string provenance_ref{};
    std::int64_t observed_at_ms = 0;
    std::uint64_t version = 0;
};

struct GraphSnapshot {
    std::string snapshot_id{};
    std::int64_t captured_at_ms = 0;
    std::vector<GraphNode> nodes{};
    std::vector<GraphEdge> edges{};
};

// A correction is both high-value supervision and an immutable reference to the
// authoritative graph update performed elsewhere. Raw conversation text is deliberately
// absent: the event contains ids and a digest/reference, not a second private vault.
struct CorrectionEvent {
    std::string correction_id{};
    CorrectionKind kind = CorrectionKind::AnswerFailure;
    std::string operator_id{};
    std::string graph_snapshot_id{};
    std::string query_digest{};
    std::string raw_correction_ref{};
    std::string rationale_ref{};
    std::vector<std::string> target_ids{};
    std::int64_t recorded_at_ms = 0;
};

// This receipt is emitted by the trusted temporal-graph writer, not by the Cortex. A
// correction trajectory cannot be accepted for training until the receipt binds the
// correction to an atomically published successor graph snapshot.
struct GraphCorrectionReceipt {
    std::string correction_id{};
    std::string source_graph_snapshot_id{};
    std::string resulting_graph_snapshot_id{};
    std::string graph_update_ref{};
    std::int64_t applied_at_ms = 0;
};

// A replayable supervision record. Private voice/text/model records stay behind their
// references in the vault; no raw payload gets copied to the Cortex or state database.
struct CorrectionTrajectory {
    std::string trajectory_id{};
    CorrectionEvent correction{};
    GraphCorrectionReceipt graph_update{};
    std::vector<std::string> selected_context_ids{};
    std::vector<std::string> omitted_candidate_ids{};
    std::string response_ref{};
    std::string response_digest{};
    std::string corrected_interpretation_ref{};
    std::string model_id{};
    std::string model_hash{};
    std::int64_t captured_at_ms = 0;
};

// A promoted artifact is selected by the authoritative model registry. The Cortex
// validates that a neural artifact has the reproducibility/rollback identity it needs;
// it does not promote an artifact itself.
struct ModelManifest {
    std::string model_id{};
    std::string model_hash{};
    std::string training_dataset_version{};
    std::string graph_schema_version{};
    ModelBackend backend = ModelBackend::DeterministicBaseline;
    std::string architecture{};
    std::string rollback_model_id{};
    bool promoted = false;
    std::int64_t created_at_ms = 0;
    std::optional<std::int64_t> promoted_at_ms{};
};

// Everything supplied to one context compilation must be explicit. In particular the
// sensitivity allow-list is a policy-filtered input; no model score can widen it.
struct FocusActivation {
    std::string node_id{};
    double strength = 1.0;
    std::int64_t activated_at_ms = 0;
    bool pinned = false;
};

// A trusted, historical attention view; it cannot alter node summaries or policy.
struct FocusState {
    std::string focus_id{};
    bool suspended = false;
    std::int64_t half_life_ms = 86'400'000;
    std::vector<FocusActivation> activations{};
};

// A proposed future information need is a retrieval cue, never a remembered fact.
// It can activate only after its grounding node is already in the visible frontier.
struct RetrievalCue {
    std::string grounding_node_id{};
    std::string question{};
};

struct ContextRequest {
    std::string request_id{};
    std::string question{};
    std::string active_project_id{};
    std::string graph_snapshot_id{};
    std::vector<Sensitivity> permitted_sensitivities{};
    std::uint32_t token_budget = 0;
    std::uint16_t maximum_items = 24;
    std::uint8_t maximum_expansion_hops = 1;
    std::int64_t requested_at_ms = 0;
    std::optional<FocusState> focus{};
    std::vector<RetrievalCue> retrieval_cues{};
};

struct ContextItem {
    std::string node_id{};
    std::string provenance_ref{};
    std::string summary{};
    double relevance = 0.0;
    double utility = 0.0;
    std::uint32_t estimated_tokens = 0;
};

struct ContextEdge {
    std::string edge_id{};
    std::string source_id{};
    std::string target_id{};
    EdgeKind kind = EdgeKind::RelatedTo;
    std::string provenance_ref{};
};

struct ContextMetrics {
    std::uint32_t candidate_nodes = 0;
    std::uint32_t selected_nodes = 0;
    std::uint32_t selected_edges = 0;
    std::uint32_t token_count = 0;
    std::uint32_t budget = 0;
    bool deterministic_fallback = true;
};

struct ContextBundle {
    std::string request_id{};
    std::string graph_snapshot_id{};
    std::string selector_version{"deterministic-v2"};
    std::vector<ContextItem> facts{};
    std::vector<ContextEdge> relationships{};
    std::vector<std::string> uncertainties{};
    ContextMetrics metrics{};

    // Stable rendering for a replaceable generative provider. Context remains structured
    // on the wire; this is only the last, bounded textual form a provider consumes.
    [[nodiscard]] std::string render(std::string_view question) const;
};

struct AnalyticalProposal {
    ProposalKind kind = ProposalKind::KnowledgeGap;
    std::string proposal_id{};
    std::string rationale{};
    std::vector<std::string> evidence_refs{};
    std::int64_t proposed_at_ms = 0;
};

[[nodiscard]] std::optional<ContractRefusal> validate(const GraphSnapshot& snapshot) noexcept;
[[nodiscard]] std::optional<ContractRefusal> validate(const ContextRequest& request) noexcept;
[[nodiscard]] std::optional<ContractRefusal> validate(const CorrectionEvent& correction) noexcept;
[[nodiscard]] std::optional<ContractRefusal> validate(const GraphCorrectionReceipt& receipt) noexcept;
[[nodiscard]] std::optional<ContractRefusal> validate(const CorrectionTrajectory& trajectory) noexcept;
[[nodiscard]] std::optional<ContractRefusal> validate(const ModelManifest& manifest) noexcept;

// The always-installed L0 baseline. It ranks policy-visible nodes by lexical relevance,
// recency and provenance, expands only bounded graph neighbors, and solves a deterministic
// utility-per-token selection. It is the correctness comparator every neural candidate
// must beat in replay before promotion.
class DeterministicContextCompiler {
public:
    [[nodiscard]] std::optional<ContractRefusal> compile(
        const GraphSnapshot& snapshot,
        const ContextRequest& request,
        ContextBundle* destination) const noexcept;
};

}  // namespace archie::cortex
