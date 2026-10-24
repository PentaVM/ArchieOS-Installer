#include "sentry/identity/relationship.hpp"

#include <algorithm>
#include <iterator>
#include <deque>
#include <map>
#include <set>

namespace sentry::identity {

std::string_view name(RelationKind kind) noexcept {
    switch (kind) {
        case RelationKind::Colleague:        return "colleague";
        case RelationKind::Employer:         return "employer";
        case RelationKind::Coauthor:         return "coauthor";
        case RelationKind::PublishedBy:      return "published_by";
        case RelationKind::Represents:       return "represents";
        case RelationKind::Manager:          return "manager";
        case RelationKind::Household:        return "household";
        case RelationKind::Family:           return "family";
        case RelationKind::Friend:           return "friend";
        case RelationKind::EmergencyContact: return "emergency_contact";
        case RelationKind::Partner:          return "partner";
        case RelationKind::AuthorizedGuest:  return "authorized_guest";
    }
    return "unknown";
}

std::string_view name(EdgeBasis basis) noexcept {
    switch (basis) {
        case EdgeBasis::OwnerDeclared:       return "owner_declared";
        case EdgeBasis::SubjectDeclared:     return "subject_declared";
        case EdgeBasis::PublicRecord:        return "public_record";
        case EdgeBasis::ModelHypothesis:     return "model_hypothesis";
        case EdgeBasis::CoPresenceInference: return "co_presence_inference";
    }
    return "unknown";
}

std::string_view describe(EdgeRefusal refusal) noexcept {
    switch (refusal) {
        case EdgeRefusal::EndpointNotIdentifiable:
            return "one end of this edge is an anonymous track; an edge to a track is how "
                   "a track acquires an identity through the back door";
        case EdgeRefusal::PersonalTieFromPublicRecord:
            return "family, household and friendship are not read out of the public "
                   "record; someone must have declared them";
        case EdgeRefusal::InferredFromCoPresence:
            return "two people being in the same place at the same time is not a "
                   "relationship, and a year of that is a social graph built out of "
                   "surveillance";
        case EdgeRefusal::GenerativeCannotAssert:
            return "a model may propose a relationship as a hypothesis, never assert one "
                   "into the graph Sentry stands behind";
        case EdgeRefusal::NotDeclaredByAParticipant:
            return "a personal relationship must be declared by one of the people it is "
                   "about; declaring one between two other people is asserting facts "
                   "about lives that did not ask";
        case EdgeRefusal::GuestTieMustInvolveOwner:
            return "an access grant is a statement about this node, so an edge asserting "
                   "one between two other people is describing someone else's house";
        case EdgeRefusal::SelfEdge:
            return "an edge from someone to themselves says nothing and breaks every "
                   "traversal that assumes it cannot happen";
    }
    return "unknown refusal";
}

std::optional<EdgeRefusal> refuse_edge(const RelationEdge& edge,
                                       const SubjectRecord& from,
                                       const SubjectRecord& to,
                                       std::string_view declarer_id) noexcept {
    if (edge.from_id == edge.to_id) {
        return EdgeRefusal::SelfEdge;
    }

    // The two constitutional refusals come first, before anything about who these people
    // are. Neither is fixable by producing a consent form or a better source, so an
    // operator should never be shown a refusal that implies it might be.
    if (edge.basis == EdgeBasis::CoPresenceInference) {
        return EdgeRefusal::InferredFromCoPresence;
    }
    if (edge.basis == EdgeBasis::ModelHypothesis) {
        // Not a rejection of the content. `hypothesise_edge` accepts exactly this, and it
        // is refused here because `refuse_edge` guards the asserted graph specifically.
        return EdgeRefusal::GenerativeCannotAssert;
    }

    if (from.kind == SubjectKind::AnonymousTrack || to.kind == SubjectKind::AnonymousTrack) {
        return EdgeRefusal::EndpointNotIdentifiable;
    }

    if (is_personal(edge.kind)) {
        if (edge.basis == EdgeBasis::PublicRecord) {
            return EdgeRefusal::PersonalTieFromPublicRecord;
        }
        // Someone it is about has to be the one saying it. The operator may say who their
        // own family is; they may not enter a family tree for two people they read about.
        if (declarer_id.empty() ||
            (declarer_id != edge.from_id && declarer_id != edge.to_id)) {
            return EdgeRefusal::NotDeclaredByAParticipant;
        }
    }

    if (edge.kind == RelationKind::AuthorizedGuest &&
        from.kind != SubjectKind::Owner && to.kind != SubjectKind::Owner) {
        return EdgeRefusal::GuestTieMustInvolveOwner;
    }

    return std::nullopt;
}

void RelationGraph::assert_edge(RelationEdge edge) { asserted_.push_back(std::move(edge)); }

void RelationGraph::hypothesise_edge(RelationEdge edge) {
    hypotheses_.push_back(std::move(edge));
}

namespace {

// Undirected adjacency over the asserted edges only.
//
// Hypotheses are deliberately absent. If a model's guess could form part of a path back
// to the operator, that guess would become the justification for holding a record about
// someone — which is the whole thing the separation exists to prevent.
using Adjacency = std::map<std::string, std::vector<std::string>, std::less<>>;

Adjacency adjacency_of(const std::vector<RelationEdge>& edges) {
    Adjacency adjacency;
    for (const auto& edge : edges) {
        adjacency[edge.from_id].push_back(edge.to_id);
        adjacency[edge.to_id].push_back(edge.from_id);
    }
    return adjacency;
}

}  // namespace

std::optional<std::vector<std::string>> RelationGraph::path_to_owner(
    std::string_view subject_id) const {
    if (subject_id == owner_id_) {
        return std::vector<std::string>{owner_id_};
    }

    const auto adjacency = adjacency_of(asserted_);

    // Breadth-first, so the path returned is the shortest one. That matters for how this
    // reads to a person: the justification for holding a record should be the most direct
    // chain that exists, not whichever one the traversal happened to find first.
    std::map<std::string, std::string, std::less<>> came_from;
    std::set<std::string, std::less<>> seen{owner_id_};
    std::deque<std::string> queue{owner_id_};

    while (!queue.empty()) {
        const std::string current = queue.front();
        queue.pop_front();

        const auto neighbours = adjacency.find(current);
        if (neighbours == adjacency.end()) {
            continue;
        }
        for (const auto& next : neighbours->second) {
            if (!seen.insert(next).second) {
                continue;
            }
            came_from[next] = current;
            if (next == subject_id) {
                std::vector<std::string> path{next};
                for (std::string at = current; at != owner_id_; at = came_from[at]) {
                    path.push_back(at);
                }
                path.push_back(owner_id_);
                std::reverse(path.begin(), path.end());
                return path;
            }
            queue.push_back(next);
        }
    }

    return std::nullopt;
}

std::vector<std::string> RelationGraph::neighbours(std::string_view id) const {
    std::set<std::string, std::less<>> found;
    for (const auto& edge : asserted_) {
        if (edge.from_id == id) {
            found.insert(edge.to_id);
        } else if (edge.to_id == id) {
            found.insert(edge.from_id);
        }
    }
    return {found.begin(), found.end()};
}

std::vector<std::string> RelationGraph::mutual(std::string_view a, std::string_view b) const {
    const auto left = neighbours(a);
    const auto right = neighbours(b);

    std::vector<std::string> shared;
    std::ranges::set_intersection(left, right, std::back_inserter(shared));

    // Neither endpoint counts as its own mutual acquaintance. A direct edge between the
    // two puts each in the other's neighbour list, and without this every connected pair
    // would report each other as a friend they have in common.
    std::erase_if(shared, [a, b](const std::string& id) { return id == a || id == b; });
    return shared;
}

std::optional<std::size_t> RelationGraph::degree(std::string_view subject_id) const {
    const auto path = path_to_owner(subject_id);
    if (!path) {
        return std::nullopt;
    }
    return path->size() - 1;
}

std::vector<std::string> RelationGraph::orphans() const {
    std::set<std::string, std::less<>> everyone;
    for (const auto& edge : asserted_) {
        everyone.insert(edge.from_id);
        everyone.insert(edge.to_id);
    }

    std::vector<std::string> orphaned;
    for (const auto& id : everyone) {
        if (!path_to_owner(id)) {
            orphaned.push_back(id);
        }
    }
    return orphaned;
}

}  // namespace sentry::identity
