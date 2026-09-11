#include "archie/cortex/contracts.hpp"

#include <algorithm>
#include <cctype>
#include <cmath>
#include <set>
#include <sstream>
#include <unordered_map>
#include <unordered_set>

namespace archie::cortex {
namespace {

constexpr std::int64_t kMillisecondsPerDay = 86'400'000;

bool valid_identifier(const std::string& value) noexcept {
    return !value.empty() && value.size() <= kMaximumIdentifierBytes;
}

bool valid_reference(const std::string& value) noexcept {
    if (!valid_identifier(value)) return false;
    return std::all_of(value.begin(), value.end(), [](unsigned char character) {
        return std::isalnum(character) != 0 || character == ':' || character == '.' ||
               character == '_' || character == '-' || character == '/';
    });
}

bool valid_sha256_digest(const std::string& value) noexcept {
    constexpr std::string_view prefix{"sha256:"};
    if (value.size() != prefix.size() + 64U || !value.starts_with(prefix)) return false;
    return std::all_of(value.begin() + static_cast<std::ptrdiff_t>(prefix.size()), value.end(),
                       [](unsigned char character) {
                           return std::isdigit(character) != 0 ||
                                  (character >= 'a' && character <= 'f');
                       });
}

bool unique_references(const std::vector<std::string>& values) {
    std::unordered_set<std::string> seen;
    for (const auto& value : values) {
        if (!valid_reference(value) || !seen.insert(value).second) return false;
    }
    return true;
}

bool contains(const std::vector<Sensitivity>& values, Sensitivity sought) noexcept {
    return std::find(values.begin(), values.end(), sought) != values.end();
}

std::vector<std::string> tokens(std::string_view text) {
    std::vector<std::string> result;
    std::string current;
    for (const unsigned char character : text) {
        if (std::isalnum(character) != 0) {
            current.push_back(static_cast<char>(std::tolower(character)));
        } else if (current.size() >= 2U) {
            result.push_back(std::move(current));
            current.clear();
        } else {
            current.clear();
        }
    }
    if (current.size() >= 2U) {
        result.push_back(std::move(current));
    }
    std::sort(result.begin(), result.end());
    result.erase(std::unique(result.begin(), result.end()), result.end());
    return result;
}

double lexical_overlap(const std::vector<std::string>& question,
                       const GraphNode& node) {
    std::string searchable = node.id + " " + node.summary;
    for (const auto& alias : node.aliases) {
        searchable += " ";
        searchable += alias;
    }
    const auto node_tokens = tokens(searchable);
    if (question.empty() || node_tokens.empty()) {
        return 0.0;
    }
    std::size_t hits = 0;
    for (const auto& token : question) {
        if (std::binary_search(node_tokens.begin(), node_tokens.end(), token)) {
            ++hits;
        }
    }
    return static_cast<double>(hits) / static_cast<double>(question.size());
}

double recency(std::int64_t now_ms, std::int64_t observed_at_ms) noexcept {
    if (observed_at_ms <= 0 || now_ms <= observed_at_ms) {
        return observed_at_ms == 0 ? 0.25 : 1.0;
    }
    const auto age_days = static_cast<double>(now_ms - observed_at_ms) /
                          static_cast<double>(kMillisecondsPerDay);
    return std::exp(-age_days / 180.0);
}

std::uint32_t estimated_tokens(const GraphNode& node) noexcept {
    // The bundle also carries an id/source label and an item marker. Keep the estimate
    // conservative so a provider is not sent a context larger than the promised budget.
    const auto text_tokens = (node.summary.size() + node.id.size() +
                              node.provenance_ref.size() + 3U) / 4U;
    const auto overhead = static_cast<std::size_t>(16U);
    const auto total = std::min<std::size_t>(text_tokens + overhead, 65'535U);
    return static_cast<std::uint32_t>(total);
}

struct Candidate {
    const GraphNode* node = nullptr;
    double relevance = 0.0;
    double utility = 0.0;
    std::uint32_t cost = 0;
    std::vector<std::string> path{};
};

bool by_utility_then_id(const Candidate& left, const Candidate& right) noexcept {
    const auto left_ratio = left.utility / static_cast<double>(std::max(1U, left.cost));
    const auto right_ratio = right.utility / static_cast<double>(std::max(1U, right.cost));
    if (left_ratio != right_ratio) {
        return left_ratio > right_ratio;
    }
    if (left.relevance != right.relevance) {
        return left.relevance > right.relevance;
    }
    return left.node->id < right.node->id;
}

}  // namespace

std::string_view name(NodeKind value) noexcept {
    switch (value) {
        case NodeKind::Person: return "person";
        case NodeKind::Message: return "message";
        case NodeKind::Conversation: return "conversation";
        case NodeKind::Claim: return "claim";
        case NodeKind::Event: return "event";
        case NodeKind::Place: return "place";
        case NodeKind::Device: return "device";
        case NodeKind::Sensor: return "sensor";
        case NodeKind::Project: return "project";
        case NodeKind::Task: return "task";
        case NodeKind::Process: return "process";
        case NodeKind::Model: return "model";
        case NodeKind::Document: return "document";
        case NodeKind::Paper: return "paper";
        case NodeKind::Equation: return "equation";
        case NodeKind::Component: return "component";
        case NodeKind::Simulation: return "simulation";
        case NodeKind::Company: return "company";
        case NodeKind::Country: return "country";
        case NodeKind::NewsArticle: return "news_article";
        case NodeKind::ProviderObservation: return "provider_observation";
        case NodeKind::Episode: return "episode";
        case NodeKind::Unknown: return "unknown";
    }
    return "unknown";
}

std::string_view name(EdgeKind value) noexcept {
    switch (value) {
        case EdgeKind::Said: return "said";
        case EdgeKind::Mentions: return "mentions";
        case EdgeKind::ReplyTo: return "reply_to";
        case EdgeKind::Knows: return "knows";
        case EdgeKind::FriendOf: return "friend_of";
        case EdgeKind::ParentOf: return "parent_of";
        case EdgeKind::WorksWith: return "works_with";
        case EdgeKind::Owns: return "owns";
        case EdgeKind::LocatedAt: return "located_at";
        case EdgeKind::Uses: return "uses";
        case EdgeKind::DependsOn: return "depends_on";
        case EdgeKind::Produced: return "produced";
        case EdgeKind::Contradicts: return "contradicts";
        case EdgeKind::Supports: return "supports";
        case EdgeKind::DerivedFrom: return "derived_from";
        case EdgeKind::ObservedBy: return "observed_by";
        case EdgeKind::RelatedTo: return "related_to";
        case EdgeKind::ExecutedOn: return "executed_on";
        case EdgeKind::BlockedBy: return "blocked_by";
    }
    return "unknown";
}

std::string_view name(Sensitivity value) noexcept {
    switch (value) {
        case Sensitivity::Public: return "public";
        case Sensitivity::LocalRestricted: return "local_restricted";
        case Sensitivity::PersonalPrivate: return "personal_private";
        case Sensitivity::Secret: return "secret";
        case Sensitivity::Biometric: return "biometric";
        case Sensitivity::Location: return "location";
    }
    return "unknown";
}

std::string_view name(CorrectionKind value) noexcept {
    switch (value) {
        case CorrectionKind::EntityCorrection: return "entity_correction";
        case CorrectionKind::RelationCorrection: return "relation_correction";
        case CorrectionKind::CoreferenceCorrection: return "coreference_correction";
        case CorrectionKind::RetrievalPositive: return "retrieval_positive";
        case CorrectionKind::RetrievalNegative: return "retrieval_negative";
        case CorrectionKind::MissingContextLabel: return "missing_context_label";
        case CorrectionKind::StaleKnowledgeCorrection: return "stale_knowledge_correction";
        case CorrectionKind::ContextPreference: return "context_preference";
        case CorrectionKind::AnswerFailure: return "answer_failure";
    }
    return "unknown";
}

std::string_view name(ModelBackend value) noexcept {
    switch (value) {
        case ModelBackend::DeterministicBaseline: return "deterministic_baseline";
        case ModelBackend::OnnxRuntime: return "onnx_runtime";
        case ModelBackend::TensorRt: return "tensorrt";
    }
    return "unknown";
}

std::string_view name(ProposalKind value) noexcept {
    switch (value) {
        case ProposalKind::CandidatePolicyChange: return "candidate_policy_change";
        case ProposalKind::CandidateSchedulerChange: return "candidate_scheduler_change";
        case ProposalKind::CandidateContractChange: return "candidate_contract_change";
        case ProposalKind::KnowledgeGap: return "knowledge_gap";
    }
    return "unknown";
}

std::string_view describe(ContractRefusal value) noexcept {
    switch (value) {
        case ContractRefusal::EmptyIdentifier:
            return "a stable graph or request identifier is required";
        case ContractRefusal::IdentifierTooLong:
            return "an identifier exceeds the fixed contract bound";
        case ContractRefusal::DuplicateNode:
            return "a graph snapshot contains the same node identifier twice";
        case ContractRefusal::DuplicateEdge:
            return "a graph snapshot contains the same edge identifier twice";
        case ContractRefusal::UnknownEdgeEndpoint:
            return "a graph edge names a node absent from this immutable snapshot";
        case ContractRefusal::GraphTooLarge:
            return "the graph slice exceeds the bounded analytical request envelope";
        case ContractRefusal::SummaryTooLong:
            return "a graph summary exceeds the bounded context contract";
        case ContractRefusal::MissingProvenance:
            return "every graph node and edge must retain a provenance reference";
        case ContractRefusal::InvalidTimestamp:
            return "the temporal graph contract requires a positive event timestamp";
        case ContractRefusal::EmptyQuestion:
            return "context compilation requires a non-empty question";
        case ContractRefusal::QuestionTooLong:
            return "the question exceeds the bounded local IPC contract";
        case ContractRefusal::InvalidTokenBudget:
            return "the requested model context budget is outside the supported range";
        case ContractRefusal::InvalidExpansionDepth:
            return "graph expansion must be bounded to between zero and three hops";
        case ContractRefusal::MissingSensitivityPolicy:
            return "a context request must carry an explicit policy-filtered sensitivity allow-list";
        case ContractRefusal::MalformedIpcFrame:
            return "the local Cortex IPC frame is malformed, oversized or uses an unknown version";
        case ContractRefusal::SnapshotUnavailable:
            return "the trusted immutable graph snapshot is unavailable or invalid";
        case ContractRefusal::SnapshotVersionMismatch:
            return "the request does not name the graph snapshot that was supplied";
        case ContractRefusal::InvalidModelManifest:
            return "a model manifest lacks a reproducible identity or rollback target";
        case ContractRefusal::UnpromotedModel:
            return "a candidate model may run only in replay or shadow mode, not production";
        case ContractRefusal::InvalidCorrection:
            return "a correction needs operator, snapshot, query reference and target evidence";
    }
    return "unknown cortex contract refusal";
}

std::optional<ContractRefusal> validate(const GraphSnapshot& snapshot) noexcept {
    if (!valid_identifier(snapshot.snapshot_id)) {
        return snapshot.snapshot_id.empty() ? ContractRefusal::EmptyIdentifier
                                            : ContractRefusal::IdentifierTooLong;
    }
    if (snapshot.captured_at_ms <= 0) {
        return ContractRefusal::InvalidTimestamp;
    }
    if (snapshot.nodes.size() > kMaximumGraphNodes || snapshot.edges.size() > kMaximumGraphEdges) {
        return ContractRefusal::GraphTooLarge;
    }
    std::unordered_set<std::string> ids;
    ids.reserve(snapshot.nodes.size());
    for (const auto& node : snapshot.nodes) {
        if (!valid_identifier(node.id)) {
            return node.id.empty() ? ContractRefusal::EmptyIdentifier
                                   : ContractRefusal::IdentifierTooLong;
        }
        if (node.summary.size() > kMaximumSummaryBytes) {
            return ContractRefusal::SummaryTooLong;
        }
        if (!valid_reference(node.provenance_ref)) {
            return ContractRefusal::MissingProvenance;
        }
        if (node.observed_at_ms <= 0 || node.observed_at_ms > snapshot.captured_at_ms) {
            return ContractRefusal::InvalidTimestamp;
        }
        if (!ids.insert(node.id).second) {
            return ContractRefusal::DuplicateNode;
        }
        if (node.aliases.size() > 256U ||
            std::any_of(node.aliases.begin(), node.aliases.end(),
                        [](const auto& alias) { return !valid_identifier(alias); })) {
            return ContractRefusal::IdentifierTooLong;
        }
    }
    std::unordered_set<std::string> edge_ids;
    edge_ids.reserve(snapshot.edges.size());
    for (const auto& edge : snapshot.edges) {
        if (!valid_identifier(edge.id) || !valid_identifier(edge.source_id) ||
            !valid_identifier(edge.target_id)) {
            return ContractRefusal::EmptyIdentifier;
        }
        if (!valid_reference(edge.provenance_ref)) {
            return ContractRefusal::MissingProvenance;
        }
        if (edge.observed_at_ms <= 0 || edge.observed_at_ms > snapshot.captured_at_ms) {
            return ContractRefusal::InvalidTimestamp;
        }
        if (!edge_ids.insert(edge.id).second) {
            return ContractRefusal::DuplicateEdge;
        }
        if (!ids.contains(edge.source_id) || !ids.contains(edge.target_id)) {
            return ContractRefusal::UnknownEdgeEndpoint;
        }
    }
    return std::nullopt;
}

std::optional<ContractRefusal> validate(const ContextRequest& request) noexcept {
    if (!valid_identifier(request.request_id) || !valid_identifier(request.graph_snapshot_id)) {
        return ContractRefusal::EmptyIdentifier;
    }
    if (request.question.empty()) {
        return ContractRefusal::EmptyQuestion;
    }
    if (request.question.size() > kMaximumQuestionBytes) {
        return ContractRefusal::QuestionTooLong;
    }
    if (request.token_budget < kMinimumContextTokenBudget ||
        request.token_budget > kMaximumContextTokenBudget) {
        return ContractRefusal::InvalidTokenBudget;
    }
    if (request.maximum_items == 0U || request.maximum_expansion_hops > 3U) {
        return ContractRefusal::InvalidExpansionDepth;
    }
    if (request.permitted_sensitivities.empty() || request.permitted_sensitivities.size() > 6 ||
        std::any_of(request.permitted_sensitivities.begin(), request.permitted_sensitivities.end(),
                    [](auto value) { return static_cast<unsigned>(value) > static_cast<unsigned>(Sensitivity::Location); })) {
        return ContractRefusal::MissingSensitivityPolicy;
    }
    if (request.requested_at_ms <= 0) return ContractRefusal::InvalidTimestamp;
    if (request.retrieval_cues.size() > 8) return ContractRefusal::InvalidExpansionDepth;
    std::set<std::pair<std::string,std::string>> cues;
    for (const auto& cue : request.retrieval_cues) {
        if (!valid_reference(cue.grounding_node_id) || cue.question.empty() || cue.question.size() > 256 ||
            !cues.emplace(cue.grounding_node_id,cue.question).second) return ContractRefusal::InvalidExpansionDepth;
    }
    if (request.focus) {
        const auto& focus = *request.focus;
        if (!valid_reference(focus.focus_id) || focus.half_life_ms <= 0 ||
            focus.activations.size() > 128) return ContractRefusal::InvalidExpansionDepth;
        std::unordered_set<std::string> activated;
        for (const auto& a : focus.activations) {
            if (!valid_reference(a.node_id) || !activated.insert(a.node_id).second ||
                !std::isfinite(a.strength) || a.strength < 0 || a.strength > 1 ||
                a.activated_at_ms <= 0 || a.activated_at_ms > request.requested_at_ms)
                return ContractRefusal::InvalidTimestamp;
        }
    }
    return std::nullopt;
}

std::optional<ContractRefusal> validate(const CorrectionEvent& correction) noexcept {
    if (!valid_reference(correction.correction_id) || !valid_reference(correction.operator_id) ||
        !valid_reference(correction.graph_snapshot_id) ||
        !valid_sha256_digest(correction.query_digest) ||
        !valid_reference(correction.raw_correction_ref) ||
        !valid_reference(correction.rationale_ref) || correction.target_ids.empty() ||
        correction.target_ids.size() > kMaximumCorrectionTargets ||
        correction.recorded_at_ms <= 0) {
        return ContractRefusal::InvalidCorrection;
    }
    if (!unique_references(correction.target_ids)) {
        return ContractRefusal::InvalidCorrection;
    }
    return std::nullopt;
}

std::optional<ContractRefusal> validate(const GraphCorrectionReceipt& receipt) noexcept {
    if (!valid_reference(receipt.correction_id) ||
        !valid_reference(receipt.source_graph_snapshot_id) ||
        !valid_reference(receipt.resulting_graph_snapshot_id) ||
        !valid_reference(receipt.graph_update_ref) || receipt.applied_at_ms <= 0) {
        return ContractRefusal::InvalidCorrection;
    }
    if (receipt.source_graph_snapshot_id == receipt.resulting_graph_snapshot_id) {
        return ContractRefusal::InvalidCorrection;
    }
    return std::nullopt;
}

std::optional<ContractRefusal> validate(const CorrectionTrajectory& trajectory) noexcept {
    if (!valid_reference(trajectory.trajectory_id) || validate(trajectory.correction).has_value() ||
        validate(trajectory.graph_update).has_value() ||
        trajectory.graph_update.correction_id != trajectory.correction.correction_id ||
        trajectory.graph_update.source_graph_snapshot_id != trajectory.correction.graph_snapshot_id ||
        trajectory.graph_update.applied_at_ms < trajectory.correction.recorded_at_ms ||
        trajectory.selected_context_ids.size() > kMaximumCorrectionContextIds ||
        trajectory.omitted_candidate_ids.size() > kMaximumCorrectionContextIds ||
        !unique_references(trajectory.selected_context_ids) ||
        !unique_references(trajectory.omitted_candidate_ids) ||
        !valid_reference(trajectory.response_ref) ||
        !valid_sha256_digest(trajectory.response_digest) ||
        !valid_reference(trajectory.corrected_interpretation_ref) ||
        !valid_reference(trajectory.model_id) || !valid_sha256_digest(trajectory.model_hash) ||
        trajectory.captured_at_ms <= 0) {
        return ContractRefusal::InvalidCorrection;
    }
    return std::nullopt;
}

std::optional<ContractRefusal> validate(const ModelManifest& manifest) noexcept {
    if (!valid_identifier(manifest.model_id) || manifest.model_hash.empty() ||
        manifest.training_dataset_version.empty() || manifest.graph_schema_version.empty() ||
        manifest.architecture.empty() || manifest.created_at_ms <= 0) {
        return ContractRefusal::InvalidModelManifest;
    }
    if (manifest.backend != ModelBackend::DeterministicBaseline &&
        (manifest.rollback_model_id.empty() || !manifest.promoted_at_ms.has_value())) {
        return ContractRefusal::InvalidModelManifest;
    }
    if (!manifest.promoted) {
        return ContractRefusal::UnpromotedModel;
    }
    return std::nullopt;
}

std::string ContextBundle::render(std::string_view question) const {
    std::ostringstream stream;
    stream << "QUESTION\n" << question << "\n\nRELEVANT FACTS\n";
    for (const auto& fact : facts) {
        stream << "- [" << fact.node_id << "] " << fact.summary << " (source: "
               << fact.provenance_ref << ")\n";
    }
    stream << "\nRELATIONSHIPS\n";
    for (const auto& edge : relationships) {
        stream << "- " << edge.source_id << " --" << name(edge.kind) << "--> "
               << edge.target_id << " (source: " << edge.provenance_ref << ")\n";
    }
    stream << "\nUNCERTAINTIES\n";
    for (const auto& uncertainty : uncertainties) {
        stream << "- " << uncertainty << "\n";
    }
    stream << "\nPROVENANCE\nselector=" << selector_version << " snapshot="
           << graph_snapshot_id << " tokens=" << metrics.token_count << "/"
           << metrics.budget << "\n";
    return stream.str();
}

std::optional<ContractRefusal> DeterministicContextCompiler::compile(
    const GraphSnapshot& snapshot,
    const ContextRequest& request,
    ContextBundle* destination) const noexcept {
    if (destination == nullptr) {
        return ContractRefusal::InvalidModelManifest;
    }
    if (const auto refusal = validate(snapshot)) {
        return refusal;
    }
    if (const auto refusal = validate(request)) {
        return refusal;
    }
    if (snapshot.snapshot_id != request.graph_snapshot_id) {
        return ContractRefusal::SnapshotVersionMismatch;
    }
    // Filtering observation timestamps cannot undo later corrections of old facts.
    // Historical replay must supply the snapshot that actually existed at the cutoff.
    if (snapshot.captured_at_ms > request.requested_at_ms) return ContractRefusal::InvalidTimestamp;

    const auto question_tokens = tokens(request.question);
    std::unordered_map<std::string, const GraphNode*> nodes;
    nodes.reserve(snapshot.nodes.size());
    std::vector<Candidate> candidates;
    candidates.reserve(snapshot.nodes.size());
    for (const auto& node : snapshot.nodes) {
        nodes.emplace(node.id, &node);
        if (!contains(request.permitted_sensitivities, node.sensitivity) ||
            node.observed_at_ms > request.requested_at_ms) {
            continue;
        }
        const auto lexical = lexical_overlap(question_tokens, node);
        const auto project_match = !request.active_project_id.empty() &&
                                   (node.id == request.active_project_id ||
                                    std::find(node.aliases.begin(), node.aliases.end(),
                                              request.active_project_id) != node.aliases.end());
        double attention = 0;
        if (request.focus && !request.focus->suspended) {
            for (const auto& a : request.focus->activations) if (a.node_id == node.id) {
                attention = a.strength * (a.pinned ? 1.0 : std::exp2(
                    -static_cast<double>(request.requested_at_ms - a.activated_at_ms) /
                    static_cast<double>(request.focus->half_life_ms)));
            }
        }
        if (lexical <= 0.0 && !project_match && attention <= 1e-6) {
            continue;
        }
        const auto freshness = recency(request.requested_at_ms, node.observed_at_ms);
        const auto provenance = node.provenance_ref.empty() ? 0.0 : 1.0;
        const auto relevance = std::min(1.0, 0.70 * lexical + 0.20 * freshness +
                                                 0.10 * provenance +
                                                 (project_match ? 0.25 : 0.0) + 0.25 * attention);
        candidates.push_back(Candidate{.node = &node,
                                       .relevance = relevance,
                                       .utility = relevance,
                                       .cost = estimated_tokens(node)});
    }

    // Bounded expansion happens only from lexical/project seeds. It is intentionally
    // smaller than direct relevance: a path can supply context, never dominate it merely
    // because a graph is dense.
    std::unordered_map<std::string, Candidate> by_id;
    by_id.reserve(candidates.size() * 2U + 1U);
    for (const auto& candidate : candidates) {
        by_id.emplace(candidate.node->id, candidate);
    }
    // Two bounded rounds of explicit prospection. Read only the preceding round:
    // cue order cannot grow an unbounded chain. Cue text never becomes a fact.
    // Every prospective candidate retains its grounding path for context packing.
    for (unsigned round = 0; round < 2 && !request.retrieval_cues.empty(); ++round) {
        std::unordered_map<std::string, Candidate> next;
        for (const auto& cue : request.retrieval_cues) {
            const auto anchor = by_id.find(cue.grounding_node_id);
            if (anchor == by_id.end()) continue;
            const auto cue_tokens = tokens(cue.question);
            for (const auto& node : snapshot.nodes) {
                if (by_id.contains(node.id) || !contains(request.permitted_sensitivities,node.sensitivity) ||
                    node.observed_at_ms > request.requested_at_ms) continue;
                const auto overlap = lexical_overlap(cue_tokens,node);
                if (overlap <= 0) continue;
                const auto relevance = std::min(anchor->second.relevance, 0.70 * overlap + 0.10) * 0.80;
                Candidate probe{.node=&node,.relevance=relevance,.utility=relevance,.cost=estimated_tokens(node)};
                probe.path = anchor->second.path;
                probe.path.push_back(cue.grounding_node_id);
                const auto previous = next.find(node.id);
                if (previous == next.end() || probe.relevance > previous->second.relevance ||
                    (probe.relevance == previous->second.relevance && probe.path < previous->second.path))
                    next.insert_or_assign(node.id,std::move(probe));
            }
        }
        if (next.empty()) break;
        by_id.insert(next.begin(),next.end());
    }
    for (std::uint8_t hop = 0; hop < request.maximum_expansion_hops; ++hop) {
        // Read only the previous layer. Inserting into by_id while scanning edges
        // makes one hop traverse an entire ordered chain and invalidates iterators.
        std::unordered_map<std::string, Candidate> next;
        for (const auto& edge : snapshot.edges) {
            if (edge.observed_at_ms > request.requested_at_ms) continue;
            const auto source = by_id.find(edge.source_id);
            const auto target = by_id.find(edge.target_id);
            const bool source_known = source != by_id.end();
            const bool target_known = target != by_id.end();
            if (source_known == target_known) {
                continue;
            }
            const auto& known = source_known ? source->second : target->second;
            const auto& neighbor_id = source_known ? edge.target_id : edge.source_id;
            const auto node_it = nodes.find(neighbor_id);
            if (node_it == nodes.end() ||
                !contains(request.permitted_sensitivities, node_it->second->sensitivity) ||
                node_it->second->observed_at_ms > request.requested_at_ms) {
                continue;
            }
            const auto relevance = known.relevance * 0.55;
            Candidate expanded{.node = node_it->second,
                               .relevance = relevance,
                               .utility = relevance * 0.80,
                               .cost = estimated_tokens(*node_it->second)};
            expanded.path = known.path;
            expanded.path.push_back(known.node->id);
            const auto previous = next.find(neighbor_id);
            if (previous == next.end() || expanded.relevance > previous->second.relevance ||
                (expanded.relevance == previous->second.relevance && expanded.path < previous->second.path))
                next.insert_or_assign(neighbor_id, expanded);
        }
        if (next.empty()) {
            break;
        }
        by_id.insert(next.begin(), next.end());
    }

    candidates.clear();
    candidates.reserve(by_id.size());
    for (const auto& [_, candidate] : by_id) {
        candidates.push_back(candidate);
    }
    std::sort(candidates.begin(), candidates.end(), by_utility_then_id);

    ContextBundle bundle;
    if (!request.retrieval_cues.empty()) bundle.selector_version = "prospective-v1";
    bundle.request_id = request.request_id;
    bundle.graph_snapshot_id = snapshot.snapshot_id;
    bundle.metrics.candidate_nodes = static_cast<std::uint32_t>(candidates.size());
    bundle.metrics.budget = request.token_budget;
    // Account for the complete rendered envelope, including question, citations,
    // relationships and uncertainty text. This is a byte/4 estimate, not a claim
    // about a provider tokenizer; the provider must enforce its exact token limit.
    const auto charge = [&](ContextBundle& value) {
        value.metrics.token_count = request.token_budget; // reserve maximum digit width
        value.metrics.token_count = static_cast<std::uint32_t>((value.render(request.question).size() + 3U) / 4U + 8U);
        return value.metrics.token_count <= request.token_budget;
    };
    bundle.uncertainties = {"No policy-visible graph node matched within the context budget."};
    if (!charge(bundle)) return ContractRefusal::InvalidTokenBudget;
    std::unordered_map<std::string, std::vector<std::string>> conflicts;
    for (const auto& edge : snapshot.edges) {
        if (edge.kind != EdgeKind::Contradicts || edge.observed_at_ms > request.requested_at_ms) continue;
        const auto source = nodes.at(edge.source_id), target = nodes.at(edge.target_id);
        if (source->observed_at_ms > request.requested_at_ms || target->observed_at_ms > request.requested_at_ms ||
            !contains(request.permitted_sensitivities, source->sensitivity) ||
            !contains(request.permitted_sensitivities, target->sensitivity)) continue;
        conflicts[edge.source_id].push_back(edge.target_id);
        conflicts[edge.target_id].push_back(edge.source_id);
    }
    for (auto& [id, neighbors] : conflicts) {
        (void)id;
        std::sort(neighbors.begin(), neighbors.end());
        neighbors.erase(std::unique(neighbors.begin(), neighbors.end()), neighbors.end());
    }
    std::unordered_set<std::string> selected;
    for (const auto& candidate : candidates) {
        if (selected.contains(candidate.node->id)) continue;
        auto trial = bundle;
        auto included = selected;
        auto closure = candidate.path;
        closure.push_back(candidate.node->id);
        // Keep visible contradictions together. A score cannot hide disagreement.
        for (std::size_t i = 0; i < closure.size() && closure.size() <= request.maximum_items; ++i) {
            for (const auto& other : conflicts[closure[i]]) {
                const auto visible = nodes.find(other);
                if (!other.empty() && visible != nodes.end() &&
                    visible->second->observed_at_ms <= request.requested_at_ms &&
                    contains(request.permitted_sensitivities, visible->second->sensitivity) &&
                    !included.contains(other) && std::find(closure.begin(), closure.end(), other) == closure.end()) {
                    by_id.try_emplace(other, Candidate{.node = visible->second, .cost = estimated_tokens(*visible->second)});
                    closure.push_back(other);
                    if (closure.size() > request.maximum_items) break;
                }
            }
        }
        if (closure.size() > request.maximum_items) continue;
        for (const auto& id : closure) {
            if (!included.insert(id).second) continue;
            const auto& item = by_id.at(id);
            trial.facts.push_back(ContextItem{.node_id = id,
                .provenance_ref = item.node->provenance_ref, .summary = item.node->summary,
                .relevance = item.relevance, .utility = item.utility, .estimated_tokens = item.cost});
        }
        if (trial.facts.size() > request.maximum_items) continue;
        trial.relationships.clear();
        trial.uncertainties.clear();
        if (!request.retrieval_cues.empty()) trial.uncertainties.push_back(
            "Prospective cues guide retrieval; they do not establish factual relationships.");
        bool contradiction = false;
        for (const auto& edge : snapshot.edges) {
            if (edge.observed_at_ms > request.requested_at_ms ||
                !included.contains(edge.source_id) || !included.contains(edge.target_id)) continue;
            trial.relationships.push_back(ContextEdge{edge.id, edge.source_id, edge.target_id, edge.kind, edge.provenance_ref});
            contradiction = contradiction || edge.kind == EdgeKind::Contradicts;
        }
        if (contradiction) trial.uncertainties.push_back("Selected sources contradict one another; no resolution is inferred.");
        std::sort(trial.relationships.begin(), trial.relationships.end(),
                  [](const auto& left, const auto& right) { return left.edge_id < right.edge_id; });
        if (!charge(trial)) continue;
        bundle = std::move(trial);
        selected = std::move(included);
    }
    bundle.metrics.selected_nodes = static_cast<std::uint32_t>(bundle.facts.size());
    bundle.metrics.selected_edges = static_cast<std::uint32_t>(bundle.relationships.size());
    *destination = std::move(bundle);
    return std::nullopt;
}

}  // namespace archie::cortex
