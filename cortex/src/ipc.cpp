#include "archie/cortex/ipc.hpp"

#include <algorithm>
#include <cmath>
#include <limits>
#include <utility>

namespace archie::cortex::ipc {
namespace {

class Writer {
public:
    void byte(std::uint8_t value) { bytes_.push_back(value); }
    void u16(std::uint16_t value) {
        byte(static_cast<std::uint8_t>((value >> 8U) & 0xffU));
        byte(static_cast<std::uint8_t>(value & 0xffU));
    }
    void u32(std::uint32_t value) {
        for (int shift = 24; shift >= 0; shift -= 8) {
            byte(static_cast<std::uint8_t>((value >> static_cast<unsigned>(shift)) & 0xffU));
        }
    }
    void u64(std::uint64_t value) {
        for (int shift = 56; shift >= 0; shift -= 8) {
            byte(static_cast<std::uint8_t>((value >> static_cast<unsigned>(shift)) & 0xffU));
        }
    }
    void i64(std::int64_t value) { u64(static_cast<std::uint64_t>(value)); }
    void text(const std::string& value) {
        const auto size = std::min<std::size_t>(value.size(), std::numeric_limits<std::uint32_t>::max());
        u32(static_cast<std::uint32_t>(size));
        bytes_.insert(bytes_.end(), value.begin(), value.begin() + size);
    }
    [[nodiscard]] std::vector<std::uint8_t> take() { return std::move(bytes_); }

private:
    std::vector<std::uint8_t> bytes_{};
};

class Reader {
public:
    explicit Reader(const std::vector<std::uint8_t>& bytes) : bytes_(bytes) {}

    [[nodiscard]] bool byte(std::uint8_t* result) noexcept {
        if (result == nullptr || remaining() < 1U) return false;
        *result = bytes_[cursor_++];
        return true;
    }
    [[nodiscard]] bool u16(std::uint16_t* result) noexcept {
        std::uint8_t high = 0;
        std::uint8_t low = 0;
        if (!byte(&high) || !byte(&low) || result == nullptr) return false;
        *result = static_cast<std::uint16_t>((static_cast<std::uint16_t>(high) << 8U) | low);
        return true;
    }
    [[nodiscard]] bool u32(std::uint32_t* result) noexcept {
        if (result == nullptr || remaining() < 4U) return false;
        std::uint32_t value = 0;
        for (int index = 0; index < 4; ++index) value = (value << 8U) | bytes_[cursor_++];
        *result = value;
        return true;
    }
    [[nodiscard]] bool u64(std::uint64_t* result) noexcept {
        if (result == nullptr || remaining() < 8U) return false;
        std::uint64_t value = 0;
        for (int index = 0; index < 8; ++index) value = (value << 8U) | bytes_[cursor_++];
        *result = value;
        return true;
    }
    [[nodiscard]] bool i64(std::int64_t* result) noexcept {
        std::uint64_t value = 0;
        if (!u64(&value) || result == nullptr) return false;
        *result = static_cast<std::int64_t>(value);
        return true;
    }
    [[nodiscard]] bool text(std::string* result, std::uint32_t maximum) noexcept {
        std::uint32_t size = 0;
        if (!u32(&size) || result == nullptr || size > maximum || remaining() < size) return false;
        result->assign(reinterpret_cast<const char*>(bytes_.data() + cursor_), size);
        cursor_ += size;
        return true;
    }
    [[nodiscard]] bool done() const noexcept { return cursor_ == bytes_.size(); }

private:
    [[nodiscard]] std::size_t remaining() const noexcept { return bytes_.size() - cursor_; }
    const std::vector<std::uint8_t>& bytes_;
    std::size_t cursor_ = 0;
};

void header(Writer* writer, MessageType type) {
    writer->byte(kProtocolVersion);
    writer->byte(static_cast<std::uint8_t>(type));
}

[[nodiscard]] bool header(Reader* reader, MessageType expected) noexcept {
    std::uint8_t version = 0;
    std::uint8_t type = 0;
    return reader->byte(&version) && reader->byte(&type) && version == kProtocolVersion &&
           type == static_cast<std::uint8_t>(expected);
}

void graph_node(Writer* writer, const GraphNode& node) {
    writer->text(node.id);
    writer->byte(static_cast<std::uint8_t>(node.kind));
    writer->text(node.summary);
    writer->u32(static_cast<std::uint32_t>(node.aliases.size()));
    for (const auto& alias : node.aliases) writer->text(alias);
    writer->text(node.provenance_ref);
    writer->byte(static_cast<std::uint8_t>(node.sensitivity));
    writer->i64(node.observed_at_ms);
    writer->u64(node.version);
}

[[nodiscard]] bool graph_node(Reader* reader, GraphNode* node) noexcept {
    std::uint8_t kind = 0;
    std::uint32_t aliases = 0;
    std::uint8_t sensitivity = 0;
    if (!reader->text(&node->id, kMaximumIdentifierBytes) || !reader->byte(&kind) ||
        !reader->text(&node->summary, kMaximumSummaryBytes) || !reader->u32(&aliases) ||
        aliases > 256U) return false;
    node->aliases.clear();
    node->aliases.reserve(aliases);
    for (std::uint32_t index = 0; index < aliases; ++index) {
        std::string alias;
        if (!reader->text(&alias, kMaximumIdentifierBytes)) return false;
        node->aliases.push_back(std::move(alias));
    }
    if (!reader->text(&node->provenance_ref, kMaximumIdentifierBytes) ||
        !reader->byte(&sensitivity) || !reader->i64(&node->observed_at_ms) ||
        !reader->u64(&node->version)) return false;
    if (kind > static_cast<std::uint8_t>(NodeKind::Unknown) ||
        sensitivity > static_cast<std::uint8_t>(Sensitivity::Location)) return false;
    node->kind = static_cast<NodeKind>(kind);
    node->sensitivity = static_cast<Sensitivity>(sensitivity);
    return true;
}

void graph_edge(Writer* writer, const GraphEdge& edge) {
    writer->text(edge.id);
    writer->text(edge.source_id);
    writer->text(edge.target_id);
    writer->byte(static_cast<std::uint8_t>(edge.kind));
    writer->text(edge.provenance_ref);
    writer->i64(edge.observed_at_ms);
    writer->u64(edge.version);
}

[[nodiscard]] bool graph_edge(Reader* reader, GraphEdge* edge) noexcept {
    std::uint8_t kind = 0;
    if (!reader->text(&edge->id, kMaximumIdentifierBytes) ||
        !reader->text(&edge->source_id, kMaximumIdentifierBytes) ||
        !reader->text(&edge->target_id, kMaximumIdentifierBytes) || !reader->byte(&kind) ||
        !reader->text(&edge->provenance_ref, kMaximumIdentifierBytes) ||
        !reader->i64(&edge->observed_at_ms) || !reader->u64(&edge->version)) return false;
    if (kind > static_cast<std::uint8_t>(EdgeKind::BlockedBy)) return false;
    edge->kind = static_cast<EdgeKind>(kind);
    return true;
}

}  // namespace

std::vector<std::uint8_t> encode(const GraphSnapshot& snapshot) {
    Writer writer;
    header(&writer, MessageType::GraphSnapshot);
    writer.text(snapshot.snapshot_id);
    writer.i64(snapshot.captured_at_ms);
    writer.u32(static_cast<std::uint32_t>(snapshot.nodes.size()));
    for (const auto& node : snapshot.nodes) graph_node(&writer, node);
    writer.u32(static_cast<std::uint32_t>(snapshot.edges.size()));
    for (const auto& edge : snapshot.edges) graph_edge(&writer, edge);
    return writer.take();
}

std::vector<std::uint8_t> encode(const ContextRequest& request) {
    Writer writer;
    // v1/v2 remain byte-identical without cues. v3 adds bounded prospective needs;
    // graph/response/refusal framing remains v1.
    writer.byte(!request.retrieval_cues.empty() ? 3U : (request.focus ? 2U : kProtocolVersion));
    writer.byte(static_cast<std::uint8_t>(MessageType::ContextRequest));
    writer.text(request.request_id);
    writer.text(request.question);
    writer.text(request.active_project_id);
    writer.text(request.graph_snapshot_id);
    writer.u32(static_cast<std::uint32_t>(request.permitted_sensitivities.size()));
    for (const auto sensitivity : request.permitted_sensitivities) {
        writer.byte(static_cast<std::uint8_t>(sensitivity));
    }
    writer.u32(request.token_budget);
    writer.u16(request.maximum_items);
    writer.byte(request.maximum_expansion_hops);
    writer.i64(request.requested_at_ms);
    if (!request.retrieval_cues.empty()) writer.byte(request.focus ? 1U : 0U);
    if (request.focus) {
        writer.text(request.focus->focus_id);
        writer.byte(request.focus->suspended ? 1U : 0U);
        writer.i64(request.focus->half_life_ms);
        writer.u32(static_cast<std::uint32_t>(request.focus->activations.size()));
        for (const auto& a : request.focus->activations) {
            writer.text(a.node_id);
            writer.u32(std::isfinite(a.strength) && a.strength >= 0 && a.strength <= 1
                           ? static_cast<std::uint32_t>(a.strength * 1'000'000.0) : 1'000'001U);
            writer.i64(a.activated_at_ms);
            writer.byte(a.pinned ? 1U : 0U);
        }
    }
    if (!request.retrieval_cues.empty()) {
        writer.u32(static_cast<std::uint32_t>(request.retrieval_cues.size()));
        for (const auto& cue : request.retrieval_cues) {
            writer.text(cue.grounding_node_id);
            writer.text(cue.question);
        }
    }
    return writer.take();
}

std::vector<std::uint8_t> encode(const ContextBundle& bundle) {
    Writer writer;
    header(&writer, MessageType::ContextResponse);
    writer.text(bundle.request_id);
    writer.text(bundle.graph_snapshot_id);
    writer.text(bundle.selector_version);
    writer.u32(static_cast<std::uint32_t>(bundle.facts.size()));
    for (const auto& fact : bundle.facts) {
        writer.text(fact.node_id);
        writer.text(fact.provenance_ref);
        writer.text(fact.summary);
        writer.u64(static_cast<std::uint64_t>(fact.relevance * 1'000'000.0));
        writer.u64(static_cast<std::uint64_t>(fact.utility * 1'000'000.0));
        writer.u32(fact.estimated_tokens);
    }
    writer.u32(static_cast<std::uint32_t>(bundle.relationships.size()));
    for (const auto& edge : bundle.relationships) {
        writer.text(edge.edge_id);
        writer.text(edge.source_id);
        writer.text(edge.target_id);
        writer.byte(static_cast<std::uint8_t>(edge.kind));
        writer.text(edge.provenance_ref);
    }
    writer.u32(static_cast<std::uint32_t>(bundle.uncertainties.size()));
    for (const auto& uncertainty : bundle.uncertainties) writer.text(uncertainty);
    writer.u32(bundle.metrics.candidate_nodes);
    writer.u32(bundle.metrics.selected_nodes);
    writer.u32(bundle.metrics.selected_edges);
    writer.u32(bundle.metrics.token_count);
    writer.u32(bundle.metrics.budget);
    writer.byte(bundle.metrics.deterministic_fallback ? 1U : 0U);
    return writer.take();
}

std::vector<std::uint8_t> encode(const Refusal& refusal) {
    Writer writer;
    header(&writer, MessageType::Refusal);
    writer.u16(static_cast<std::uint16_t>(refusal.code));
    return writer.take();
}

std::optional<GraphSnapshot> decode_graph_snapshot(const std::vector<std::uint8_t>& bytes) noexcept {
    Reader reader(bytes);
    GraphSnapshot snapshot;
    std::uint32_t nodes = 0;
    std::uint32_t edges = 0;
    if (!header(&reader, MessageType::GraphSnapshot) ||
        !reader.text(&snapshot.snapshot_id, kMaximumIdentifierBytes) ||
        !reader.i64(&snapshot.captured_at_ms) || !reader.u32(&nodes) ||
        nodes > kMaximumGraphNodes) return std::nullopt;
    snapshot.nodes.reserve(nodes);
    for (std::uint32_t index = 0; index < nodes; ++index) {
        GraphNode node;
        if (!graph_node(&reader, &node)) return std::nullopt;
        snapshot.nodes.push_back(std::move(node));
    }
    if (!reader.u32(&edges) || edges > kMaximumGraphEdges) return std::nullopt;
    snapshot.edges.reserve(edges);
    for (std::uint32_t index = 0; index < edges; ++index) {
        GraphEdge edge;
        if (!graph_edge(&reader, &edge)) return std::nullopt;
        snapshot.edges.push_back(std::move(edge));
    }
    if (!reader.done() || validate(snapshot).has_value()) return std::nullopt;
    return snapshot;
}

std::optional<ContextRequest> decode_context_request(
    const std::vector<std::uint8_t>& bytes) noexcept {
    if (bytes.size() > kMaximumFrameBytes) return std::nullopt;
    Reader reader(bytes);
    ContextRequest request;
    std::uint32_t sensitivities = 0;
    std::uint8_t version = 0, type = 0;
    if (!reader.byte(&version) || (version != 1 && version != 2 && version != 3) ||
        !reader.byte(&type) || type != static_cast<std::uint8_t>(MessageType::ContextRequest) ||
        !reader.text(&request.request_id, kMaximumIdentifierBytes) ||
        !reader.text(&request.question, kMaximumQuestionBytes) ||
        !reader.text(&request.active_project_id, kMaximumIdentifierBytes) ||
        !reader.text(&request.graph_snapshot_id, kMaximumIdentifierBytes) ||
        !reader.u32(&sensitivities) || sensitivities > 6U) return std::nullopt;
    request.permitted_sensitivities.reserve(sensitivities);
    for (std::uint32_t index = 0; index < sensitivities; ++index) {
        std::uint8_t sensitivity = 0;
        if (!reader.byte(&sensitivity) || sensitivity > static_cast<std::uint8_t>(Sensitivity::Location)) {
            return std::nullopt;
        }
        request.permitted_sensitivities.push_back(static_cast<Sensitivity>(sensitivity));
    }
    if (!reader.u32(&request.token_budget) || !reader.u16(&request.maximum_items) ||
        !reader.byte(&request.maximum_expansion_hops) || !reader.i64(&request.requested_at_ms)) return std::nullopt;
    std::uint8_t has_focus = version == 2 ? 1U : 0U;
    if (version == 3 && (!reader.byte(&has_focus) || has_focus > 1U)) return std::nullopt;
    if (has_focus) {
        FocusState focus;
        std::uint8_t suspended = 0;
        std::uint32_t count = 0;
        if (!reader.text(&focus.focus_id, kMaximumIdentifierBytes) || !reader.byte(&suspended) || suspended > 1 ||
            !reader.i64(&focus.half_life_ms) || !reader.u32(&count) || count > 128) return std::nullopt;
        focus.suspended = suspended != 0;
        for (std::uint32_t i = 0; i < count; ++i) {
            FocusActivation activation;
            std::uint32_t strength = 0;
            std::uint8_t pinned = 0;
            if (!reader.text(&activation.node_id, kMaximumIdentifierBytes) || !reader.u32(&strength) || strength > 1'000'000 ||
                !reader.i64(&activation.activated_at_ms) || !reader.byte(&pinned) || pinned > 1) return std::nullopt;
            activation.strength = static_cast<double>(strength) / 1'000'000.0;
            activation.pinned = pinned != 0;
            focus.activations.push_back(std::move(activation));
        }
        request.focus = std::move(focus);
    }
    if (version == 3) {
        std::uint32_t count = 0;
        if (!reader.u32(&count) || count > 8) return std::nullopt;
        for (std::uint32_t i = 0; i < count; ++i) {
            RetrievalCue cue;
            if (!reader.text(&cue.grounding_node_id,kMaximumIdentifierBytes) ||
                !reader.text(&cue.question,256)) return std::nullopt;
            request.retrieval_cues.push_back(std::move(cue));
        }
    }
    if (!reader.done() || validate(request).has_value()) return std::nullopt;
    return request;
}

std::optional<ContextBundle> decode_context_response(
    const std::vector<std::uint8_t>& bytes) noexcept {
    Reader reader(bytes);
    ContextBundle bundle;
    std::uint32_t facts = 0;
    if (!header(&reader, MessageType::ContextResponse) ||
        !reader.text(&bundle.request_id, kMaximumIdentifierBytes) ||
        !reader.text(&bundle.graph_snapshot_id, kMaximumIdentifierBytes) ||
        !reader.text(&bundle.selector_version, kMaximumIdentifierBytes) || !reader.u32(&facts) ||
        facts > kMaximumGraphNodes) return std::nullopt;
    bundle.facts.reserve(facts);
    for (std::uint32_t index = 0; index < facts; ++index) {
        ContextItem item;
        std::uint64_t relevance = 0;
        std::uint64_t utility = 0;
        if (!reader.text(&item.node_id, kMaximumIdentifierBytes) ||
            !reader.text(&item.provenance_ref, kMaximumIdentifierBytes) ||
            !reader.text(&item.summary, kMaximumSummaryBytes) || !reader.u64(&relevance) ||
            !reader.u64(&utility) || !reader.u32(&item.estimated_tokens)) return std::nullopt;
        item.relevance = static_cast<double>(relevance) / 1'000'000.0;
        item.utility = static_cast<double>(utility) / 1'000'000.0;
        bundle.facts.push_back(std::move(item));
    }
    std::uint32_t edges = 0;
    if (!reader.u32(&edges) || edges > kMaximumGraphEdges) return std::nullopt;
    bundle.relationships.reserve(edges);
    for (std::uint32_t index = 0; index < edges; ++index) {
        ContextEdge edge;
        std::uint8_t kind = 0;
        if (!reader.text(&edge.edge_id, kMaximumIdentifierBytes) ||
            !reader.text(&edge.source_id, kMaximumIdentifierBytes) ||
            !reader.text(&edge.target_id, kMaximumIdentifierBytes) || !reader.byte(&kind) ||
            !reader.text(&edge.provenance_ref, kMaximumIdentifierBytes)) return std::nullopt;
        if (kind > static_cast<std::uint8_t>(EdgeKind::BlockedBy)) return std::nullopt;
        edge.kind = static_cast<EdgeKind>(kind);
        bundle.relationships.push_back(std::move(edge));
    }
    std::uint32_t uncertainties = 0;
    if (!reader.u32(&uncertainties) || uncertainties > 256U) return std::nullopt;
    bundle.uncertainties.reserve(uncertainties);
    for (std::uint32_t index = 0; index < uncertainties; ++index) {
        std::string uncertainty;
        if (!reader.text(&uncertainty, kMaximumSummaryBytes)) return std::nullopt;
        bundle.uncertainties.push_back(std::move(uncertainty));
    }
    std::uint8_t fallback = 0;
    if (!reader.u32(&bundle.metrics.candidate_nodes) || !reader.u32(&bundle.metrics.selected_nodes) ||
        !reader.u32(&bundle.metrics.selected_edges) || !reader.u32(&bundle.metrics.token_count) ||
        !reader.u32(&bundle.metrics.budget) || !reader.byte(&fallback) || !reader.done() ||
        fallback > 1U) return std::nullopt;
    bundle.metrics.deterministic_fallback = fallback == 1U;
    return bundle;
}

std::optional<Refusal> decode_refusal(const std::vector<std::uint8_t>& bytes) noexcept {
    Reader reader(bytes);
    std::uint16_t code = 0;
    if (!header(&reader, MessageType::Refusal) || !reader.u16(&code) || !reader.done() ||
        code > static_cast<std::uint16_t>(ContractRefusal::InvalidCorrection)) return std::nullopt;
    return Refusal{.code = static_cast<ContractRefusal>(code)};
}

}  // namespace archie::cortex::ipc
