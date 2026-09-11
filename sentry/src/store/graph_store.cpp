#include "sentry/store/graph_store.hpp"

#include <algorithm>
#include <array>
#include <cstdio>
#include <fstream>
#include <iterator>
#include <map>
#include <system_error>
#include <utility>

#if defined(_WIN32)
#include <process.h>
#else
#include <unistd.h>
#endif

namespace sentry::store {

using namespace sentry::identity;

std::string_view name(RejectionKind kind) noexcept {
    switch (kind) {
        case RejectionKind::Malformed:        return "malformed";
        case RejectionKind::RefusedByPolicy:  return "refused_by_policy";
        case RejectionKind::UnknownSubject:   return "unknown_subject";
        case RejectionKind::DuplicateSubject: return "duplicate_subject";
    }
    return "unknown";
}

std::string_view describe(StoreError error) noexcept {
    switch (error) {
        case StoreError::NotFound:
            return "no graph file at that path; this is a first run, not a failure";
        case StoreError::Unreadable:
            return "the graph file exists but could not be read";
        case StoreError::Unwritable:
            return "the graph could not be written; the previous file is untouched";
        case StoreError::MissingVersion:
            return "the file does not begin with a version record, so nothing in it can be "
                   "safely interpreted";
        case StoreError::UnsupportedVersion:
            return "the file was written by a version of Sentry this build does not know; "
                   "every line in it might mean something other than what it appears to";
        case StoreError::MissingOwner:
            return "the file declares no owner, so nothing in it has a root to be justified "
                   "against";
    }
    return "unknown error";
}

RelationGraph PersonGraph::to_relation_graph() const {
    RelationGraph graph{owner_id};
    for (const auto& edge : asserted_edges) {
        graph.assert_edge(edge);
    }
    for (const auto& edge : hypotheses) {
        graph.hypothesise_edge(edge);
    }
    return graph;
}

const Profile* PersonGraph::find(std::string_view subject_id) const {
    const auto it = std::ranges::find_if(profiles, [subject_id](const Profile& p) {
        return p.subject.subject_id == subject_id;
    });
    return it == profiles.end() ? nullptr : &*it;
}

Profile* PersonGraph::find_mutable(std::string_view subject_id) {
    const auto it = std::ranges::find_if(profiles, [subject_id](const Profile& p) {
        return p.subject.subject_id == subject_id;
    });
    return it == profiles.end() ? nullptr : &*it;
}

namespace {

// ---------------------------------------------------------------- escaping

// Three escapes and no more. A field may hold anything a person types, including tabs and
// newlines in a note, and the format's two structural characters have to survive that.
std::string escape(std::string_view raw) {
    std::string out;
    out.reserve(raw.size());
    for (const char c : raw) {
        switch (c) {
            case '\\': out += "\\\\"; break;
            case '\t': out += "\\t";  break;
            case '\n': out += "\\n";  break;
            case '\r': out += "\\r";  break;
            default:   out.push_back(c);
        }
    }
    return out;
}

// Returns nothing when the field ends in a dangling backslash or carries an escape this
// version does not define. Both mean the line was produced by something other than this
// serialiser, and guessing at the intent of an unknown escape is how a parser starts
// accepting data it should not.
std::optional<std::string> unescape(std::string_view field) {
    std::string out;
    out.reserve(field.size());
    for (std::size_t i = 0; i < field.size(); ++i) {
        if (field[i] != '\\') {
            out.push_back(field[i]);
            continue;
        }
        if (i + 1 >= field.size()) {
            return std::nullopt;  // dangling backslash
        }
        switch (field[++i]) {
            case '\\': out.push_back('\\'); break;
            case 't':  out.push_back('\t'); break;
            case 'n':  out.push_back('\n'); break;
            case 'r':  out.push_back('\r'); break;
            default:   return std::nullopt;
        }
    }
    return out;
}

std::vector<std::string_view> split(std::string_view line, char sep) {
    std::vector<std::string_view> fields;
    std::size_t start = 0;
    while (true) {
        const auto at = line.find(sep, start);
        if (at == std::string_view::npos) {
            fields.push_back(line.substr(start));
            return fields;
        }
        fields.push_back(line.substr(start, at - start));
        start = at + 1;
    }
}

// Unescape every field, or fail the whole line. A record half of whose fields decoded is
// not a partially valid record; it is a line that means nothing.
std::optional<std::vector<std::string>> decode_fields(
    const std::vector<std::string_view>& raw) {
    std::vector<std::string> out;
    out.reserve(raw.size());
    for (const auto field : raw) {
        auto decoded = unescape(field);
        if (!decoded) {
            return std::nullopt;
        }
        out.push_back(std::move(*decoded));
    }
    return out;
}

// ---------------------------------------------------------------- enum round trips
//
// Written against `name()` rather than against integers. A record that stored an enum's
// numeric value would silently change meaning the day someone inserts a value in the middle
// of an enum, and the graph would reinterpret every stored edge without a single error.

template <typename E, std::size_t N>
std::optional<E> parse_enum(std::string_view text, const std::array<E, N>& all) {
    for (const auto value : all) {
        if (name(value) == text) {
            return value;
        }
    }
    return std::nullopt;
}

constexpr std::array kSubjectKinds{SubjectKind::Owner, SubjectKind::ConsentedPerson,
                                   SubjectKind::PublicFigure, SubjectKind::AnonymousTrack};

constexpr std::array kTraitBases{TraitBasis::OwnerDeclared, TraitBasis::SubjectDeclared,
                                 TraitBasis::PublicRecord, TraitBasis::InferredFromMessages,
                                 TraitBasis::ModelInferred};

constexpr std::array kSensitivities{Sensitivity::PublicProfessional, Sensitivity::Personal,
                                    Sensitivity::Sensitive};

constexpr std::array kRelationKinds{
    RelationKind::Colleague, RelationKind::Employer,  RelationKind::Coauthor,
    RelationKind::PublishedBy, RelationKind::Represents, RelationKind::Household,
    RelationKind::Family,    RelationKind::Friend,    RelationKind::EmergencyContact,
    RelationKind::AuthorizedGuest};

constexpr std::array kEdgeBases{EdgeBasis::OwnerDeclared, EdgeBasis::SubjectDeclared,
                                EdgeBasis::PublicRecord, EdgeBasis::ModelHypothesis,
                                EdgeBasis::CoPresenceInference};

// The placeholder for an absent optional. A field that was simply empty could not be told
// apart from a value someone deliberately set to the empty string.
constexpr std::string_view kAbsent = "-";

}  // namespace

// ---------------------------------------------------------------- serialise

std::string serialise(const PersonGraph& graph) {
    std::string out;
    out += "v\t" + std::to_string(kFormatVersion) + "\n";
    out += "owner\t" + escape(graph.owner_id) + "\n";

    for (const auto& profile : graph.profiles) {
        out += "subject\t" + escape(profile.subject.subject_id) + "\t" +
               std::string{name(profile.subject.kind)} + "\t" +
               escape(profile.subject.display_label) + "\n";
    }

    // Traits after every subject, so a reader never meets a trait before the subject it
    // belongs to and the parser needs no second pass.
    for (const auto& profile : graph.profiles) {
        for (const auto& trait : profile.traits) {
            out += "trait\t" + escape(profile.subject.subject_id) + "\t" + escape(trait.key) +
                   "\t" + escape(trait.value) + "\t" + std::string{name(trait.basis)} + "\t" +
                   (trait.declared_sensitivity
                        ? std::string{name(*trait.declared_sensitivity)}
                        : std::string{kAbsent}) +
                   "\t" + escape(trait.source_note) + "\n";
        }
    }

    const auto write_edge = [&out](std::string_view tag, const RelationEdge& edge) {
        out += std::string{tag} + "\t" + escape(edge.from_id) + "\t" + escape(edge.to_id) +
               "\t" + std::string{name(edge.kind)} + "\t" + std::string{name(edge.basis)} +
               "\t" + escape(edge.declarer_id) + "\t" + escape(edge.source_note) + "\n";
    };
    for (const auto& edge : graph.asserted_edges) {
        write_edge("edge", edge);
    }
    for (const auto& edge : graph.hypotheses) {
        write_edge("hyp", edge);
    }

    return out;
}

// ---------------------------------------------------------------- parse

namespace {

// A hypothesis is not run through `refuse_edge` — it would fail on `GenerativeCannotAssert`
// by construction, since that is exactly what makes it a hypothesis. It still may not name
// an anonymous track or point at itself: those are properties of the endpoints, not of the
// edge's standing, and a guess about a track is still a track acquiring an identity.
std::optional<EdgeRefusal> refuse_hypothesis(const RelationEdge& edge,
                                             const SubjectRecord& from,
                                             const SubjectRecord& to) {
    if (edge.from_id == edge.to_id) {
        return EdgeRefusal::SelfEdge;
    }
    if (from.kind == SubjectKind::AnonymousTrack || to.kind == SubjectKind::AnonymousTrack) {
        return EdgeRefusal::EndpointNotIdentifiable;
    }
    return std::nullopt;
}

}  // namespace

std::expected<LoadResult, StoreError> parse(std::string_view text) {
    LoadResult result;
    std::map<std::string, std::size_t, std::less<>> index_of;  // subject id -> profile index

    const auto lines = split(text, '\n');
    std::size_t line_number = 0;
    bool version_seen = false;
    bool owner_seen = false;

    const auto reject = [&result, &line_number](RejectionKind kind, std::string detail) {
        result.rejections.push_back(Rejection{line_number, kind, std::move(detail)});
    };

    for (const auto raw_line : lines) {
        ++line_number;
        // Tolerate CRLF: a store edited on Windows and read on Arch is an ordinary thing to
        // happen, and losing the whole graph to a carriage return would be absurd.
        auto line = raw_line;
        if (!line.empty() && line.back() == '\r') {
            line.remove_suffix(1);
        }
        if (line.empty()) {
            continue;
        }

        const auto raw_fields = split(line, '\t');
        const auto tag = raw_fields.front();

        // The version must come first and must be understood before anything else is read.
        // Unlike a bad line, an unknown format means every following line might mean
        // something other than what it appears to, so this is fatal rather than a rejection.
        if (!version_seen) {
            if (tag != "v" || raw_fields.size() != 2) {
                return std::unexpected(StoreError::MissingVersion);
            }
            int version = 0;
            const auto& digits = raw_fields[1];
            if (digits.empty() ||
                !std::ranges::all_of(digits, [](char c) { return c >= '0' && c <= '9'; })) {
                return std::unexpected(StoreError::MissingVersion);
            }
            for (const char c : digits) {
                version = version * 10 + (c - '0');
            }
            if (version != kFormatVersion) {
                return std::unexpected(StoreError::UnsupportedVersion);
            }
            version_seen = true;
            continue;
        }

        const auto fields = decode_fields(raw_fields);
        if (!fields) {
            reject(RejectionKind::Malformed, "a field carries an escape this build does not "
                                             "define, or ends in a dangling backslash");
            continue;
        }

        if (tag == "owner") {
            if (fields->size() != 2 || (*fields)[1].empty()) {
                reject(RejectionKind::Malformed, "an owner record needs exactly one non-empty "
                                                 "identifier");
                continue;
            }
            result.graph.owner_id = (*fields)[1];
            owner_seen = true;
            continue;
        }

        if (tag == "subject") {
            if (fields->size() != 4) {
                reject(RejectionKind::Malformed, "a subject record needs an id, a kind and a "
                                                 "label");
                continue;
            }
            const auto kind = parse_enum((*fields)[2], kSubjectKinds);
            if (!kind) {
                reject(RejectionKind::Malformed, "unknown subject kind");
                continue;
            }
            if ((*fields)[1].empty()) {
                reject(RejectionKind::Malformed, "a subject needs an identifier");
                continue;
            }
            if (index_of.contains((*fields)[1])) {
                // Two declarations of one person are not mergeable without guessing which is
                // current, and a store that guesses is a store that quietly changes someone's
                // record.
                reject(RejectionKind::DuplicateSubject, "this subject is already declared");
                continue;
            }
            Profile profile;
            profile.subject.subject_id = (*fields)[1];
            profile.subject.kind = *kind;
            profile.subject.display_label = (*fields)[3];
            index_of.emplace(profile.subject.subject_id, result.graph.profiles.size());
            result.graph.profiles.push_back(std::move(profile));
            continue;
        }

        if (tag == "trait") {
            if (fields->size() != 7) {
                reject(RejectionKind::Malformed, "a trait record needs a subject, key, value, "
                                                 "basis, sensitivity and note");
                continue;
            }
            const auto at = index_of.find((*fields)[1]);
            if (at == index_of.end()) {
                reject(RejectionKind::UnknownSubject, "a trait for a subject this file never "
                                                      "declared");
                continue;
            }
            const auto basis = parse_enum((*fields)[4], kTraitBases);
            if (!basis) {
                reject(RejectionKind::Malformed, "unknown trait basis");
                continue;
            }
            std::optional<Sensitivity> declared;
            if ((*fields)[5] != kAbsent) {
                declared = parse_enum((*fields)[5], kSensitivities);
                if (!declared) {
                    reject(RejectionKind::Malformed, "unknown sensitivity");
                    continue;
                }
            }
            if ((*fields)[2].empty()) {
                reject(RejectionKind::Malformed, "a trait needs a key");
                continue;
            }

            Trait trait;
            trait.key = (*fields)[2];
            trait.value = (*fields)[3];
            trait.basis = *basis;
            trait.declared_sensitivity = declared;
            trait.source_note = (*fields)[6];

            auto& profile = result.graph.profiles[at->second];
            // The trust boundary. This file may have been written before the rule existed,
            // edited by hand, or restored from a backup — and a trait that would be refused
            // today is refused today.
            if (const auto refusal = refuse_trait(profile.subject, trait)) {
                reject(RejectionKind::RefusedByPolicy,
                       std::string{describe(*refusal)});
                continue;
            }
            profile.traits.push_back(std::move(trait));
            continue;
        }

        if (tag == "edge" || tag == "hyp") {
            if (fields->size() != 7) {
                reject(RejectionKind::Malformed, "an edge record needs two endpoints, a kind, "
                                                 "a basis, a declarer and a note");
                continue;
            }
            const auto kind = parse_enum((*fields)[3], kRelationKinds);
            const auto basis = parse_enum((*fields)[4], kEdgeBases);
            if (!kind || !basis) {
                reject(RejectionKind::Malformed, "unknown relation kind or edge basis");
                continue;
            }
            const auto from = index_of.find((*fields)[1]);
            const auto to = index_of.find((*fields)[2]);
            if (from == index_of.end() || to == index_of.end()) {
                reject(RejectionKind::UnknownSubject, "an edge naming a subject this file "
                                                      "never declared");
                continue;
            }

            RelationEdge edge;
            edge.from_id = (*fields)[1];
            edge.to_id = (*fields)[2];
            edge.kind = *kind;
            edge.basis = *basis;
            edge.declarer_id = (*fields)[5];
            edge.source_note = (*fields)[6];

            const auto& from_subject = result.graph.profiles[from->second].subject;
            const auto& to_subject = result.graph.profiles[to->second].subject;

            const auto refusal =
                tag == "edge"
                    ? refuse_edge(edge, from_subject, to_subject, edge.declarer_id)
                    : refuse_hypothesis(edge, from_subject, to_subject);
            if (refusal) {
                reject(RejectionKind::RefusedByPolicy, std::string{describe(*refusal)});
                continue;
            }

            if (tag == "edge") {
                result.graph.asserted_edges.push_back(std::move(edge));
            } else {
                result.graph.hypotheses.push_back(std::move(edge));
            }
            continue;
        }

        reject(RejectionKind::Malformed, "unknown record tag");
    }

    if (!version_seen) {
        return std::unexpected(StoreError::MissingVersion);
    }
    if (!owner_seen || result.graph.owner_id.empty()) {
        // Without a root, `path_to_owner` has nothing to answer against and every record in
        // the file is unjustified by construction.
        return std::unexpected(StoreError::MissingOwner);
    }

    return result;
}

// ---------------------------------------------------------------- disk

std::expected<LoadResult, StoreError> GraphStore::load() const {
    std::error_code ec;
    if (!std::filesystem::exists(path_, ec) || ec) {
        return std::unexpected(StoreError::NotFound);
    }

    std::ifstream in(path_, std::ios::binary);
    if (!in) {
        return std::unexpected(StoreError::Unreadable);
    }
    std::string text((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
    if (in.bad()) {
        return std::unexpected(StoreError::Unreadable);
    }

    return parse(text);
}

std::expected<std::size_t, StoreError> GraphStore::save(const PersonGraph& graph) const {
    // Refuse first, write second. Persisting a record that cannot be loaded back produces a
    // store that silently loses data on every round trip, and the loss would look like a
    // load bug rather than a save one.
    PersonGraph writable;
    writable.owner_id = graph.owner_id;

    std::map<std::string, const SubjectRecord*, std::less<>> subjects;
    for (const auto& profile : graph.profiles) {
        Profile kept;
        kept.subject = profile.subject;
        for (const auto& trait : profile.traits) {
            if (!refuse_trait(profile.subject, trait)) {
                kept.traits.push_back(trait);
            }
        }
        subjects.emplace(kept.subject.subject_id, nullptr);
        writable.profiles.push_back(std::move(kept));
    }
    for (auto& profile : writable.profiles) {
        subjects[profile.subject.subject_id] = &profile.subject;
    }

    const auto endpoints = [&subjects](const RelationEdge& edge)
        -> std::optional<std::pair<const SubjectRecord*, const SubjectRecord*>> {
        const auto from = subjects.find(edge.from_id);
        const auto to = subjects.find(edge.to_id);
        if (from == subjects.end() || to == subjects.end()) {
            return std::nullopt;
        }
        return std::pair{from->second, to->second};
    };

    for (const auto& edge : graph.asserted_edges) {
        const auto found = endpoints(edge);
        if (found && !refuse_edge(edge, *found->first, *found->second, edge.declarer_id)) {
            writable.asserted_edges.push_back(edge);
        }
    }
    for (const auto& edge : graph.hypotheses) {
        const auto found = endpoints(edge);
        if (found && !refuse_hypothesis(edge, *found->first, *found->second)) {
            writable.hypotheses.push_back(edge);
        }
    }

    const std::string text = serialise(writable);

    // Same directory, because a rename across filesystems is a copy and a copy is not
    // atomic. The pid keeps two processes writing the same store from colliding on the
    // temporary itself.
    auto temp = path_;
    temp += ".tmp." + std::to_string(static_cast<long long>(
#if defined(_WIN32)
                          _getpid()
#else
                          ::getpid()
#endif
                              ));

    {
        std::ofstream out(temp, std::ios::binary | std::ios::trunc);
        if (!out) {
            return std::unexpected(StoreError::Unwritable);
        }
        out.write(text.data(), static_cast<std::streamsize>(text.size()));
        out.flush();
        if (!out) {
            std::error_code ignored;
            std::filesystem::remove(temp, ignored);
            return std::unexpected(StoreError::Unwritable);
        }
    }

    std::error_code ec;
    std::filesystem::rename(temp, path_, ec);
    if (ec) {
        std::error_code ignored;
        std::filesystem::remove(temp, ignored);
        return std::unexpected(StoreError::Unwritable);
    }

    return writable.profiles.size();
}

}  // namespace sentry::store
