#pragma once

#include "archie/cortex/contracts.hpp"

#include <map>

namespace archie::cortex {

// Nodes describe analyzable things, not individual models or running processes.
// All instances of a head share its implementation and future checkpoint.
enum class AnalyticalHead { Relevance, Expansion, ContextUtility, Resource, Outcome, Contradiction };

struct CapabilityDescriptor {
    std::string id{};
    std::string graph_node_id{};
    std::vector<std::string> capabilities{};
    std::vector<std::string> inputs{};
    std::vector<std::string> outputs{};
    std::vector<std::string> dependencies{};
    std::vector<std::string> telemetry_fields{};
    std::vector<std::string> semantic_tags{};
    std::string authority_boundary_ref{};
    std::string provenance_ref{};
    std::uint64_t version = 1;
};

struct AnalyticalNeuron {
    std::string id{};
    std::string graph_node_id{};
    std::vector<AnalyticalHead> heads{};
    std::uint64_t descriptor_version = 0;
};

// Value-owned and bounded. Callers serialize mutations; queries expose no mutable state.
class NeuronRegistry {
public:
    [[nodiscard]] bool upsert(const CapabilityDescriptor& descriptor);
    [[nodiscard]] bool attach_graph(const GraphSnapshot& snapshot);
    [[nodiscard]] const std::map<std::string, AnalyticalNeuron>& neurons() const { return neurons_; }
private:
    std::map<std::string, CapabilityDescriptor> descriptors_{};
    std::map<std::string, AnalyticalNeuron> neurons_{};
};

// Receipt-bound corrections continue to use CorrectionTrajectory. This event adds
// measured outcomes and explicit feedback; lack of feedback is never success.
enum class LearningSignal { ContextUseful, ContextIrrelevant, MissingContext, TaskOutcome, ProcessSample };
struct UtilityObservation {
    std::optional<double> quality{};
    std::optional<double> relevance{};
    std::optional<double> confidence{};
    std::optional<double> latency_ms{};
    std::optional<double> tokens{};
    std::optional<double> cpu_ms{};
    std::optional<double> memory_bytes{};
    std::optional<double> energy_joules{};
    std::optional<double> privacy_exposure{};
    std::optional<double> risk{};
    std::optional<bool> task_success{};
};
struct TrainingEvent {
    std::string event_id{};
    std::string subject_id{};
    std::string snapshot_id{};
    std::string focus_id{};
    std::string evidence_ref{};
    LearningSignal signal = LearningSignal::ProcessSample;
    UtilityObservation observed{};
    std::int64_t recorded_at_ms = 0;
};
[[nodiscard]] bool valid(const TrainingEvent& event);

struct ProcessPrediction {
    std::string subject_id{};
    std::size_t samples = 0;
    std::optional<double> latency_ms{};
    std::optional<double> memory_bytes{};
    std::optional<double> failure_probability{};
    bool sufficient_data = false;
};
// Replay-derived empirical comparator for future shared learned heads. No scheduler
// policy, task priority or execution grant can be changed through this interface.
[[nodiscard]] std::optional<ProcessPrediction> predict_process(
    const std::vector<TrainingEvent>& history, std::string_view subject_id,
    std::int64_t as_of_ms, std::size_t minimum_samples = 5);

struct ConsolidationGroup {
    std::string representative_id{};
    std::vector<std::string> redundant_ids{};
    std::vector<std::string> evidence_refs{};
};
// Exact duplicates only, within the same kind/privacy class. Suggestions do not delete
// evidence, rewrite facts, or merge conflicting claims.
[[nodiscard]] std::vector<ConsolidationGroup> consolidation_candidates(
    const GraphSnapshot& snapshot, const ContextRequest& policy_view);

struct ReplayMetrics {
    double required_recall = 0.0;
    double irrelevant_rate = 0.0;
    double recall_per_1000_tokens = 0.0;
    std::uint32_t context_tokens = 0;
};
[[nodiscard]] ReplayMetrics evaluate_context(const ContextBundle& bundle,
                                            const std::vector<std::string>& required_ids);

// Only already selected, policy-visible context is projected. No filesystem access
// or inbound Obsidian mutation channel. The caller owns atomic file publication.
[[nodiscard]] std::string cognitive_projection(const ContextBundle& bundle);

}  // namespace archie::cortex
