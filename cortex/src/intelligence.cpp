#include "archie/cortex/intelligence.hpp"

#include <algorithm>
#include <cmath>
#include <set>
#include <sstream>
#include <tuple>

namespace archie::cortex {
namespace {
bool reference(const std::string& s) {
    return !s.empty() && s.size() <= kMaximumIdentifierBytes &&
           s.find_first_not_of("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789:._-/") == std::string::npos;
}
bool references(const std::vector<std::string>& values) {
    return values.size() <= 128 && std::all_of(values.begin(), values.end(), reference) &&
           std::set<std::string>(values.begin(), values.end()).size() == values.size();
}
bool measured(const std::optional<double>& v, double maximum) {
    return !v || (std::isfinite(*v) && *v >= 0 && *v <= maximum);
}
std::string escaped(std::string_view text) {
    std::string out;
    for (char c : text) {
        // Prevent imported text from becoming HTML, wiki links, tags, or headings.
        if (c == '<') out += "&lt;";
        else if (c == '>') out += "&gt;";
        else if (c == '&') out += "&amp;";
        else if (std::string_view("\\`*_{}[]()#+!|").find(c) != std::string_view::npos) { out += '\\'; out += c; }
        else if (c == '\n' || c == '\r') out += ' ';
        else out += c;
    }
    return out;
}
}  // namespace

bool NeuronRegistry::upsert(const CapabilityDescriptor& d) {
    if (!reference(d.id) || d.id.starts_with("graph/") || !reference(d.graph_node_id) || !reference(d.provenance_ref) ||
        !reference(d.authority_boundary_ref) || d.version == 0 ||
        !references(d.capabilities) || !references(d.inputs) || !references(d.outputs) ||
        !references(d.dependencies) || !references(d.telemetry_fields) || !references(d.semantic_tags)) return false;
    auto prior = descriptors_.find(d.id);
    if (prior != descriptors_.end() && d.version <= prior->second.version) return false;
    if (prior == descriptors_.end() && neurons_.size() >= kMaximumGraphNodes) return false;
    AnalyticalNeuron neuron{d.id, d.graph_node_id, {AnalyticalHead::Relevance, AnalyticalHead::ContextUtility}, d.version};
    if (!d.dependencies.empty()) neuron.heads.push_back(AnalyticalHead::Expansion);
    if (std::find(d.telemetry_fields.begin(), d.telemetry_fields.end(), "latency_ms") != d.telemetry_fields.end() ||
        std::find(d.telemetry_fields.begin(), d.telemetry_fields.end(), "memory_bytes") != d.telemetry_fields.end())
        neuron.heads.push_back(AnalyticalHead::Resource);
    if (std::find(d.telemetry_fields.begin(), d.telemetry_fields.end(), "task_success") != d.telemetry_fields.end())
        neuron.heads.push_back(AnalyticalHead::Outcome);
    descriptors_.insert_or_assign(d.id, d);
    neurons_.insert_or_assign(d.id, std::move(neuron));
    return true;
}

bool NeuronRegistry::attach_graph(const GraphSnapshot& snapshot) {
    if (validate(snapshot)) return false;
    // Rebuild the disposable graph view without losing registered descriptors.
    std::map<std::string, AnalyticalNeuron> next;
    for (const auto& [id, descriptor] : descriptors_) {
        (void)descriptor;
        next.emplace(id, neurons_.at(id));
    }
    for (const auto& node : snapshot.nodes) {
        const auto id = "graph/" + node.id;
        if (next.contains(id)) return false;
        next.emplace(id, AnalyticalNeuron{id, node.id,
                     {AnalyticalHead::Relevance, AnalyticalHead::Expansion, AnalyticalHead::ContextUtility}, node.version});
    }
    if (next.size() > kMaximumGraphNodes) return false;
    neurons_ = std::move(next);
    return true;
}

bool valid(const TrainingEvent& e) {
    const auto& u = e.observed;
    if (!reference(e.event_id) || !reference(e.subject_id) || !reference(e.snapshot_id) ||
        !reference(e.evidence_ref) || (!e.focus_id.empty() && !reference(e.focus_id)) || e.recorded_at_ms <= 0) return false;
    if (!measured(u.quality, 1) || !measured(u.relevance, 1) || !measured(u.confidence, 1) ||
        !measured(u.privacy_exposure, 1) || !measured(u.risk, 1) || !measured(u.latency_ms, 1e15) ||
        !measured(u.tokens, 1e15) || !measured(u.cpu_ms, 1e15) || !measured(u.memory_bytes, 1e18) ||
        !measured(u.energy_joules, 1e18)) return false;
    switch (e.signal) {
        case LearningSignal::TaskOutcome: return u.task_success.has_value();
        case LearningSignal::ProcessSample: return u.latency_ms.has_value() || u.memory_bytes.has_value() || u.cpu_ms.has_value();
        case LearningSignal::ContextUseful:
        case LearningSignal::ContextIrrelevant:
        case LearningSignal::MissingContext: return !u.task_success.has_value();
    }
    return false;
}

std::optional<ProcessPrediction> predict_process(const std::vector<TrainingEvent>& history,
    std::string_view subject_id, std::int64_t as_of_ms, std::size_t minimum_samples) {
    if (history.size() > 65'536 || minimum_samples == 0 || as_of_ms <= 0) return std::nullopt;
    ProcessPrediction p; p.subject_id = subject_id;
    std::size_t latencies = 0, memories = 0, outcomes = 0, failures = 0;
    double latency = 0, memory = 0;
    std::set<std::string> seen;
    for (const auto& e : history) {
        if (!valid(e) || !seen.insert(e.event_id).second) return std::nullopt;
        if (e.subject_id != subject_id || e.recorded_at_ms > as_of_ms) continue;
        if (e.signal != LearningSignal::ProcessSample && e.signal != LearningSignal::TaskOutcome) continue;
        ++p.samples;
        if (e.observed.latency_ms) { latency += *e.observed.latency_ms; ++latencies; }
        if (e.observed.memory_bytes) { memory += *e.observed.memory_bytes; ++memories; }
        if (e.observed.task_success) { ++outcomes; failures += !*e.observed.task_success; }
    }
    if (latencies >= minimum_samples) p.latency_ms = latency / static_cast<double>(latencies);
    if (memories >= minimum_samples) p.memory_bytes = memory / static_cast<double>(memories);
    if (outcomes >= minimum_samples) p.failure_probability = static_cast<double>(failures) / static_cast<double>(outcomes);
    p.sufficient_data = p.latency_ms.has_value() || p.memory_bytes.has_value() || p.failure_probability.has_value();
    return p;
}

std::vector<ConsolidationGroup> consolidation_candidates(const GraphSnapshot& snapshot,
                                                        const ContextRequest& view) {
    if (validate(snapshot) || validate(view) || snapshot.snapshot_id != view.graph_snapshot_id ||
        snapshot.captured_at_ms > view.requested_at_ms) return {};
    using Key = std::tuple<NodeKind, Sensitivity, std::string>;
    std::map<Key, std::vector<const GraphNode*>> groups;
    std::set<std::string> disputed;
    for (const auto& e : snapshot.edges) if (e.kind == EdgeKind::Contradicts) {
        disputed.insert(e.source_id); disputed.insert(e.target_id);
    }
    for (const auto& n : snapshot.nodes) {
        if (n.summary.empty() || disputed.contains(n.id) || n.observed_at_ms > view.requested_at_ms ||
            std::find(view.permitted_sensitivities.begin(), view.permitted_sensitivities.end(), n.sensitivity) == view.permitted_sensitivities.end()) continue;
        groups[{n.kind, n.sensitivity, n.summary}].push_back(&n);
    }
    std::vector<ConsolidationGroup> result;
    for (auto& [key, nodes] : groups) {
        (void)key;
        if (nodes.size() < 2) continue;
        std::sort(nodes.begin(), nodes.end(), [](auto a, auto b) { return a->id < b->id; });
        ConsolidationGroup group; group.representative_id = nodes.front()->id;
        for (const auto* n : nodes) {
            if (n != nodes.front()) group.redundant_ids.push_back(n->id);
            group.evidence_refs.push_back(n->provenance_ref);
        }
        result.push_back(std::move(group));
    }
    return result;
}

ReplayMetrics evaluate_context(const ContextBundle& bundle, const std::vector<std::string>& required_ids) {
    std::set<std::string> required(required_ids.begin(), required_ids.end()), selected;
    std::size_t hits = 0;
    for (const auto& n : bundle.facts) if (selected.insert(n.node_id).second && required.contains(n.node_id)) ++hits;
    ReplayMetrics m;
    m.required_recall = required.empty() ? 1.0 : static_cast<double>(hits) / static_cast<double>(required.size());
    m.irrelevant_rate = selected.empty() ? 0.0 : 1.0 - static_cast<double>(hits) / static_cast<double>(selected.size());
    m.context_tokens = bundle.metrics.token_count;
    m.recall_per_1000_tokens = m.context_tokens == 0 ? 0 : m.required_recall * 1000 / m.context_tokens;
    return m;
}

std::string cognitive_projection(const ContextBundle& bundle) {
    std::ostringstream out;
    out << "# Cognitive context\n\nRead-only context projection; relationships retain source status.\n\nSnapshot: " << escaped(bundle.graph_snapshot_id) << "\n";
    for (const auto& n : bundle.facts) out << "\n## " << escaped(n.node_id) << "\n\n" << escaped(n.summary) << "\n\nSource: " << escaped(n.provenance_ref) << "\n";
    out << "\n## Relationships\n\n";
    for (const auto& e : bundle.relationships) out << "- " << escaped(e.source_id) << " → " << name(e.kind) << " → " << escaped(e.target_id) << " (" << escaped(e.provenance_ref) << ")\n";
    for (const auto& u : bundle.uncertainties) out << "\n> " << escaped(u) << "\n";
    return out.str();
}
}  // namespace archie::cortex
