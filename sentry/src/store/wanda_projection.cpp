#include "sentry/store/wanda_projection.hpp"

#include <set>
#include <utility>

namespace sentry::store {
using namespace sentry::identity;

std::string_view describe(ProjectionError error) noexcept {
    switch (error) {
        case ProjectionError::TooLarge: return "graph projection exceeds its bounded output or record limit";
        case ProjectionError::InvalidGraph: return "graph projection requires unique bounded IDs and existing endpoints";
        case ProjectionError::InvalidUtf8: return "graph projection contains invalid UTF-8";
    }
    return "unknown projection error";
}

namespace {

bool utf8(std::string_view value) {
    for (std::size_t i = 0; i < value.size();) {
        const auto lead = static_cast<unsigned char>(value[i++]);
        if (lead < 0x80) {
            continue;
        }
        unsigned remaining = 0;
        unsigned point = 0;
        unsigned floor = 0;
        if (lead >= 0xc2 && lead <= 0xdf) {
            remaining = 1; point = lead & 0x1f; floor = 0x80;
        } else if (lead >= 0xe0 && lead <= 0xef) {
            remaining = 2; point = lead & 0x0f; floor = 0x800;
        } else if (lead >= 0xf0 && lead <= 0xf4) {
            remaining = 3; point = lead & 0x07; floor = 0x10000;
        } else {
            return false;
        }
        if (remaining > value.size() - i) {
            return false;
        }
        while (remaining-- > 0) {
            const auto next = static_cast<unsigned char>(value[i++]);
            if ((next & 0xc0) != 0x80) {
                return false;
            }
            point = (point << 6) | (next & 0x3f);
        }
        if (point < floor || point > 0x10ffff || (point >= 0xd800 && point <= 0xdfff)) {
            return false;
        }
    }
    return true;
}

class Json {
  public:
    void raw(std::string_view text) {
        if (text.size() > kMaxProjectionBytes - out_.size()) {
            throw ProjectionError::TooLarge;
        }
        out_.append(text);
    }

    void string(std::string_view text) {
        if (!utf8(text)) {
            throw ProjectionError::InvalidUtf8;
        }
        raw("\"");
        constexpr char hex[] = "0123456789abcdef";
        for (const unsigned char c : text) {
            switch (c) {
                case '"': raw("\\\""); break;
                case '\\': raw("\\\\"); break;
                case '\n': raw("\\n"); break;
                case '\r': raw("\\r"); break;
                case '\t': raw("\\t"); break;
                case '\b': raw("\\b"); break;
                case '\f': raw("\\f"); break;
                default:
                    if (c < 0x20) {
                        const char escaped[]{'\\', 'u', '0', '0', hex[c >> 4], hex[c & 15]};
                        raw(std::string_view{escaped, sizeof escaped});
                    } else {
                        const char byte = static_cast<char>(c);
                        raw(std::string_view{&byte, 1});
                    }
            }
        }
        raw("\"");
    }

    std::string take() { return std::move(out_); }

  private:
    std::string out_;
};

void trait_json(Json& out, const Trait& trait, bool reveal) {
    const bool redacted = !reveal && trait.effective_sensitivity() == Sensitivity::Sensitive;
    out.raw("{\"key\":"); out.string(trait.key);
    out.raw(",\"value\":");
    if (redacted) { out.raw("null"); } else { out.string(trait.value); }
    out.raw(",\"basis\":"); out.string(name(trait.basis));
    out.raw(",\"sensitivity\":"); out.string(name(trait.effective_sensitivity()));
    out.raw(",\"source_note\":"); out.string(redacted ? std::string_view{} : trait.source_note);
    out.raw(redacted ? ",\"redacted\":true}" : ",\"redacted\":false}");
}

bool valid_id(std::string_view id) {
    return !id.empty() && id.size() <= kMaxProjectionIdBytes;
}

}  // namespace

std::expected<std::string, ProjectionError> project_wanda_json(const PersonGraph& graph, bool reveal) {
    if (graph.profiles.size() > kMaxProjectionNodes ||
        graph.asserted_edges.size() > kMaxProjectionEdges ||
        graph.hypotheses.size() > kMaxProjectionEdges - graph.asserted_edges.size()) {
        return std::unexpected(ProjectionError::TooLarge);
    }
    std::set<std::string_view> ids;
    for (const auto& profile : graph.profiles) {
        if (!valid_id(profile.subject.subject_id) || !ids.insert(profile.subject.subject_id).second) {
            return std::unexpected(ProjectionError::InvalidGraph);
        }
    }
    if (!valid_id(graph.owner_id) || !ids.contains(graph.owner_id)) {
        return std::unexpected(ProjectionError::InvalidGraph);
    }
    const auto valid_edges = [&ids](const auto& edges) {
        for (const auto& edge : edges) {
            if (!ids.contains(edge.from_id) || !ids.contains(edge.to_id) ||
                (!edge.declarer_id.empty() && !valid_id(edge.declarer_id))) {
                return false;
            }
        }
        return true;
    };
    if (!valid_edges(graph.asserted_edges) || !valid_edges(graph.hypotheses)) {
        return std::unexpected(ProjectionError::InvalidGraph);
    }
    try {
        Json out;
        const auto relations = graph.to_relation_graph();
        out.raw("{\"schema_version\":1,\"owner_id\":"); out.string(graph.owner_id);
        out.raw(",\"nodes\":[");
        bool first = true;
        for (const auto& profile : graph.profiles) {
            if (!first) { out.raw(","); } first = false;
            out.raw("{\"id\":"); out.string(profile.subject.subject_id);
            out.raw(",\"label\":"); out.string(profile.subject.display_label);
            out.raw(",\"kind\":"); out.string(name(profile.subject.kind));
            const auto path = relations.path_to_owner(profile.subject.subject_id);
            out.raw(",\"degree\":");
            out.raw(path ? std::to_string(path->size() - 1) : "null");
            out.raw(",\"path_to_owner\":");
            if (path) {
                out.raw("["); bool first_id = true;
                for (const auto& id : *path) {
                    if (!first_id) { out.raw(","); } first_id = false;
                    out.string(id);
                }
                out.raw("]");
            } else { out.raw("null"); }
            out.raw(",\"tags\":["); bool first_trait = true;
            for (const auto& trait : profile.traits) {
                if (trait.key != "wanda.tag") { continue; }
                if (!first_trait) { out.raw(","); } first_trait = false;
                trait_json(out, trait, reveal);
            }
            out.raw("],\"traits\":["); first_trait = true;
            for (const auto& trait : profile.traits) {
                if (!first_trait) { out.raw(","); } first_trait = false;
                trait_json(out, trait, reveal);
            }
            out.raw("]}");
        }
        out.raw("],\"edges\":["); first = true;
        const auto emit_edges = [&](const auto& edges, std::string_view standing) {
            for (const auto& edge : edges) {
                if (!first) { out.raw(","); } first = false;
                out.raw("{\"from_id\":"); out.string(edge.from_id);
                out.raw(",\"to_id\":"); out.string(edge.to_id);
                out.raw(",\"kind\":"); out.string(name(edge.kind));
                out.raw(",\"basis\":"); out.string(name(edge.basis));
                out.raw(",\"declarer_id\":"); out.string(edge.declarer_id);
                out.raw(",\"source_note\":"); out.string(edge.source_note);
                out.raw(",\"standing\":"); out.string(standing); out.raw("}");
            }
        };
        emit_edges(graph.asserted_edges, "asserted");
        emit_edges(graph.hypotheses, "hypothesis");
        out.raw("]}");
        return out.take();
    } catch (ProjectionError error) {
        return std::unexpected(error);
    }
}

}  // namespace sentry::store
