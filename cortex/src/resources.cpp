#include "archie/cortex/resources.hpp"

#include <algorithm>
#include <cmath>
#include <set>
#include <sstream>

namespace archie::cortex {
namespace {

std::size_t code_points(std::string_view text) {
    return static_cast<std::size_t>(std::count_if(
        text.begin(), text.end(), [](char c) { return (static_cast<unsigned char>(c) & 0xC0) != 0x80; }));
}

bool finite_in(double value, double low, double high) {
    return std::isfinite(value) && value >= low && value <= high;
}

std::string_view mode_name(FocusMode mode) {
    switch (mode) {
        case FocusMode::Off: return "off";
        case FocusMode::Focused: return "focused";
        case FocusMode::Open: return "open";
        case FocusMode::FocusedAndOpen: return "focused_and_open";
    }
    return "off";
}

void json_string(std::ostringstream& out, std::string_view text) {
    out << '"';
    for (const unsigned char c : text) {
        switch (c) {
            case '"': out << "\\\""; break;
            case '\\': out << "\\\\"; break;
            case '\n': out << "\\n"; break;
            case '\r': out << "\\r"; break;
            case '\t': out << "\\t"; break;
            default:
                if (c < 0x20) {
                    static constexpr char hex[] = "0123456789abcdef";
                    out << "\\u00" << hex[c >> 4] << hex[c & 0xF];
                } else {
                    out << static_cast<char>(c);
                }
        }
    }
    out << '"';
}

double decayed(const FocusActivation& activation, std::int64_t half_life_ms, std::int64_t as_of_ms) {
    if (activation.pinned || half_life_ms <= 0) return activation.strength;
    const auto age = static_cast<double>(std::max<std::int64_t>(0, as_of_ms - activation.activated_at_ms));
    return activation.strength * std::exp2(-age / static_cast<double>(half_life_ms));
}

}  // namespace

std::optional<ContractRefusal> validate(const ResourceAdvice& advice) noexcept {
    if (advice.advice_id.empty() || advice.focus_state_id.empty()) return ContractRefusal::EmptyIdentifier;
    if (advice.advice_id.size() > 128 || advice.focus_state_id.size() > 128) return ContractRefusal::IdentifierTooLong;
    if (advice.advised_at_ms <= 0) return ContractRefusal::InvalidTimestamp;
    if (advice.foci.size() > kMaxAdvisedFoci) return ContractRefusal::GraphTooLarge;
    std::set<std::string_view> seen;
    double total = 0.0;
    for (const auto& focus : advice.foci) {
        if (focus.focus_id.empty()) return ContractRefusal::EmptyIdentifier;
        if (focus.focus_id.size() > 128) return ContractRefusal::IdentifierTooLong;
        if (!seen.insert(focus.focus_id).second) return ContractRefusal::DuplicateNode;
        if (!finite_in(focus.share, 0.0, 1.0) || !finite_in(focus.max_fps, 0.0, kMaxAdvisedFps)) {
            return ContractRefusal::InvalidCorrection;
        }
        if (focus.phrases.size() > kMaxFocusPhrases) return ContractRefusal::SummaryTooLong;
        for (const auto& phrase : focus.phrases) {
            if (phrase.empty() || code_points(phrase) > kMaxPhraseChars) return ContractRefusal::SummaryTooLong;
        }
        total += focus.share;
    }
    if (total > 1.0 + 1e-9) return ContractRefusal::InvalidCorrection;
    for (const auto& model : advice.models) {
        if (model.subject_id.empty()) return ContractRefusal::EmptyIdentifier;
        if ((model.latency_ms && !finite_in(*model.latency_ms, 0.0, 1e9)) ||
            (model.memory_bytes && !finite_in(*model.memory_bytes, 0.0, 1e15))) {
            return ContractRefusal::InvalidCorrection;
        }
    }
    return std::nullopt;
}

std::optional<ResourceAdvice> advise_resources(const FocusState& focus,
                                               const std::vector<FocusSource>& sources,
                                               const std::vector<TrainingEvent>& history,
                                               const std::vector<std::string>& model_subjects,
                                               std::int64_t as_of_ms) {
    if (focus.focus_id.empty() || as_of_ms <= 0 || sources.size() > kMaxAdvisedFoci) return std::nullopt;
    ResourceAdvice advice;
    advice.advice_id = "resource-advice:" + focus.focus_id + ":" + std::to_string(as_of_ms);
    advice.focus_state_id = focus.focus_id;
    advice.advised_at_ms = as_of_ms;
    std::vector<double> weights;
    double total = 0.0;
    for (const auto& source : sources) {
        double weight = 0.0;
        if (!focus.suspended) {
            for (const auto& activation : focus.activations) {
                if (activation.node_id == source.node_id && std::isfinite(activation.strength) &&
                    activation.strength > 0.0) {
                    weight = std::max(weight, decayed(activation, focus.half_life_ms, as_of_ms));
                }
            }
        }
        weights.push_back(weight);
        total += weight;
    }
    for (std::size_t i = 0; i < sources.size(); ++i) {
        const auto& source = sources[i];
        FocusAdvice item;
        item.focus_id = source.node_id;
        item.share = total > 0.0 ? weights[i] / total : 0.0;
        item.phrases = source.phrases;
        if (item.phrases.size() > kMaxFocusPhrases) item.phrases.resize(kMaxFocusPhrases);
        item.read_text = source.has_text && item.share > 0.0;
        item.max_fps = item.share > 0.0 ? std::clamp(source.max_fps, 0.0, kMaxAdvisedFps) : 0.0;
        if (item.share <= 0.0) {
            item.mode = FocusMode::Off;
        } else if (source.kind == FocusSourceKind::Screen) {
            item.mode = item.phrases.empty() ? FocusMode::Open : FocusMode::FocusedAndOpen;
        } else {
            item.mode = item.phrases.empty() ? FocusMode::Open : FocusMode::Focused;
        }
        advice.foci.push_back(std::move(item));
    }
    for (const auto& subject : model_subjects) {
        ModelAdvice model;
        model.subject_id = subject;
        if (const auto prediction = predict_process(history, subject, as_of_ms); prediction) {
            model.latency_ms = prediction->latency_ms;
            model.memory_bytes = prediction->memory_bytes;
        }
        advice.models.push_back(std::move(model));
    }
    if (validate(advice)) return std::nullopt;
    return advice;
}

AnalyticalProposal propose(const ResourceAdvice& advice) {
    AnalyticalProposal proposal;
    proposal.kind = ProposalKind::CandidateSchedulerChange;
    proposal.proposal_id = advice.advice_id;
    std::ostringstream rationale;
    rationale << "advisory share of IRIS compute across " << advice.foci.size()
              << " foci from focus state " << advice.focus_state_id
              << "; applied by archie-accel only inside the Policy envelope";
    proposal.rationale = rationale.str();
    proposal.evidence_refs.push_back("focus:" + advice.focus_state_id);
    for (const auto& model : advice.models) proposal.evidence_refs.push_back("process:" + model.subject_id);
    proposal.proposed_at_ms = advice.advised_at_ms;
    return proposal;
}

std::string focus_plan_json(const ResourceAdvice& advice) {
    std::size_t strongest = advice.foci.size();
    for (std::size_t i = 0; i < advice.foci.size(); ++i) {
        if (advice.foci[i].share > 0.0 &&
            (strongest == advice.foci.size() || advice.foci[i].share > advice.foci[strongest].share)) {
            strongest = i;
        }
    }
    std::ostringstream out;
    out.precision(17);
    out << "{\"advice\":[";
    for (std::size_t i = 0; i < advice.foci.size(); ++i) {
        const auto& focus = advice.foci[i];
        if (i > 0) out << ',';
        out << "{\"focus\":";
        json_string(out, focus.focus_id);
        out << ",\"share\":" << focus.share << ",\"mode\":";
        json_string(out, focus.mode == FocusMode::FocusedAndOpen ? "focused_and_open" : mode_name(focus.mode));
        out << ",\"phrases\":[";
        for (std::size_t p = 0; p < focus.phrases.size(); ++p) {
            if (p > 0) out << ',';
            json_string(out, focus.phrases[p]);
        }
        out << "],\"read_text\":" << (focus.read_text ? "true" : "false") << ",\"priority\":";
        json_string(out, i == strongest ? "active_focus" : "background");
        out << ",\"max_fps\":" << focus.max_fps << '}';
    }
    out << "]}";
    return out.str();
}

std::vector<CapabilityDescriptor> model_capability_descriptors() {
    const std::vector<std::string> job_telemetry{
        "latency_ms", "memory_bytes", "energy_joules", "task_success", "queue_ms", "batch_size"};
    auto descriptor = [&](std::string id, std::vector<std::string> capabilities,
                          std::vector<std::string> inputs, std::vector<std::string> outputs,
                          std::vector<std::string> dependencies, std::vector<std::string> tags) {
        CapabilityDescriptor d;
        d.id = std::move(id);
        d.graph_node_id = "process:" + d.id;
        d.capabilities = std::move(capabilities);
        d.inputs = std::move(inputs);
        d.outputs = std::move(outputs);
        d.dependencies = std::move(dependencies);
        d.telemetry_fields = job_telemetry;
        d.semantic_tags = std::move(tags);
        d.authority_boundary_ref = "authority:proposes-only";
        d.provenance_ref = "plan:PLAN-CORTEX-ROUTER-SLINKS-2026-10-20";
        return d;
    };
    return {
        descriptor("iris.detect", {"detect", "track", "instance-embedding"}, {"frame", "phrases"},
                   {"scene.state", "scene.people"}, {"archie-accel"}, {"vision", "gpu"}),
        descriptor("iris.read", {"read-text"}, {"frame", "damage"}, {"scene.text"}, {"iris.detect"},
                   {"vision", "text", "gpu"}),
        descriptor("wanda.lncm", {"social-decision", "social-state-update"},
                   {"scene.text", "scene.people", "conversation"}, {"social.intent", "social.focus"},
                   {"archie-accel"}, {"social", "gpu-or-cpu"}),
        descriptor("llm.bonsai", {"generate"}, {"context-bundle", "social.intent"}, {"llm.partial"},
                   {"llama-server"}, {"language", "gpu-or-cpu"}),
    };
}

}  // namespace archie::cortex
