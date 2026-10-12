// Relationships — the graph of people around the operator, and the rules that keep it
// from becoming a surveillance product.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Identity
//
// The graph is rooted at the operator. Every person in it is reachable from them, and the
// path is the justification: "why does Sentry hold a record about this person" always has
// an answer, and the answer is a chain of declared or publicly recorded relationships
// back to the person who runs the node. A node with no path to the root has no reason to
// exist and is reported as orphaned rather than kept.
//
// # The edge that must not exist
//
// The dangerous operation in this whole subsystem is not face recognition. It is
// inference: two people were seen in the same place at the same time, therefore they know
// each other. Repeat that across a year of camera history and the result is a social
// graph built out of surveillance, assembled from people who never consented to any of
// it — including everyone who merely walked past.
//
// So `EdgeBasis::CoPresenceInference` exists as a named value and is refused for every
// relationship kind. It is spelled out rather than omitted because the alternative is
// that someone adds it later as an obvious feature, and a value that is present and
// refused is a decision, while a value that is absent is a gap.
//
// # Who may assert an edge
//
// Declared relationships come from people: the operator says who their family is, or a
// subject declares their own. Public relationships come from the public professional
// record: a byline links a reporter to an outlet. A generative model may propose, and
// what it produces is a hypothesis with the standing that word implies — never an edge in
// the asserted graph. That is RFC-008's rule that generative output is not verified world
// state, applied to the place it would do the most damage.

#pragma once

#include <cstddef>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "sentry/identity/subject.hpp"

namespace sentry::identity {

// What kind of tie an edge asserts.
//
// Grouped by how it may be established, which is the only grouping that matters here: a
// professional tie can come from the public record, a personal one can only be declared,
// and a sensitive one needs both ends to have agreed.
enum class RelationKind {
    // --- Professional. Establishable from the public record.
    Colleague,
    Employer,
    Coauthor,
    // A reporter and the outlet that published them.
    PublishedBy,
    // An official and the body they speak for.
    Represents,
    // `from_id` is managed by `to_id`; distinct from the employing organisation.
    Manager,

    // --- Personal. Declared, never derived.
    Household,
    Family,
    Friend,
    EmergencyContact,
    // A declared romantic partnership. Never established from public record/inference.
    Partner,

    // --- Operational. About this node, not about the people.
    // Someone the operator has granted access to a space or device.
    AuthorizedGuest,
};

[[nodiscard]] std::string_view name(RelationKind kind) noexcept;

// Whether a relationship kind is part of someone's private life.
//
// Family, household and friendship are the categories §40 names when it forbids
// collecting family details. They may exist in the graph — the operator saying who lives
// in their own house is the ordinary case — but never from the public record and never
// from inference.
[[nodiscard]] constexpr bool is_personal(RelationKind kind) noexcept {
    switch (kind) {
        case RelationKind::Household:
        case RelationKind::Family:
        case RelationKind::Friend:
        case RelationKind::EmergencyContact:
        case RelationKind::Partner:
            return true;
        case RelationKind::Colleague:
        case RelationKind::Employer:
        case RelationKind::Coauthor:
        case RelationKind::PublishedBy:
        case RelationKind::Represents:
        case RelationKind::Manager:
        case RelationKind::AuthorizedGuest:
            return false;
    }
    return true;  // A kind added later is personal until someone says otherwise.
}

// Where an edge came from.
enum class EdgeBasis {
    // The operator said so, about their own life.
    OwnerDeclared,
    // The subject said so, about themselves.
    SubjectDeclared,
    // The public professional record: a byline, a masthead, a corporate filing.
    PublicRecord,
    // A generative model proposed it. Never an assertion — see `refuse_edge`.
    ModelHypothesis,
    // Two people were observed in the same place at the same time.
    //
    // Present so that it can be refused. This is the basis that turns a security system
    // into a social graph, and naming it is how the refusal stays visible to whoever
    // reads this file next.
    CoPresenceInference,
};

[[nodiscard]] std::string_view name(EdgeBasis basis) noexcept;

// One asserted tie between two people.
struct RelationEdge {
    std::string from_id{};
    std::string to_id{};
    RelationKind kind{};
    EdgeBasis basis = EdgeBasis::ModelHypothesis;
    // Who asserted this. Empty for a basis with no person behind it, such as the public
    // record.
    //
    // Part of the edge rather than a parameter alone, because a store that cannot say who
    // declared a personal tie cannot re-check `NotDeclaredByAParticipant` when it reads the
    // edge back — and a rule that only holds at the moment of writing is not a rule.
    std::string declarer_id{};
    // Free text describing where this came from — a URL, a filing, an operator's note.
    // Evidence for a human reading the graph, not something code branches on.
    std::string source_note{};
};

// Why an edge was refused.
enum class EdgeRefusal {
    // An endpoint is an anonymous track. An edge to a track is how a track acquires an
    // identity through the back door.
    EndpointNotIdentifiable,
    // A personal relationship — family, household, friendship — asserted from the public
    // record. §40: do not collect family details.
    PersonalTieFromPublicRecord,
    // Derived from people having been in the same place. Refused for every kind.
    InferredFromCoPresence,
    // A generative model tried to assert rather than propose.
    GenerativeCannotAssert,
    // A personal tie needs the declaration to come from someone it is about.
    NotDeclaredByAParticipant,
    // An access grant that does not involve the operator. `AuthorizedGuest` is a
    // statement about this node — who may enter this space, use this device — so an
    // edge asserting it between two other people is describing someone else's house.
    GuestTieMustInvolveOwner,
    // An edge from someone to themselves.
    SelfEdge,
};

[[nodiscard]] std::string_view describe(EdgeRefusal refusal) noexcept;

// Whether an edge may enter the asserted graph. Returns nothing when it may.
//
// `declarer_id` is who is asserting it, empty for a basis that has no person behind it.
// It matters for personal ties: the operator may say who their own family is, and a
// subject may say who theirs is, but neither may declare a relationship between two other
// people — that is asserting facts about lives that did not ask.
[[nodiscard]] std::optional<EdgeRefusal> refuse_edge(const RelationEdge& edge,
                                                     const SubjectRecord& from,
                                                     const SubjectRecord& to,
                                                     std::string_view declarer_id) noexcept;

// How an edge is held once it is accepted.
//
// A hypothesis is kept apart from an assertion in the type, not in a flag, so that code
// walking the asserted graph cannot pick up a model's guess by forgetting to filter.
enum class EdgeStanding {
    Asserted,
    Hypothesis,
};

// The relationship graph, rooted at the operator.
//
// Pure: it holds edges and answers questions about reachability. No store, no I/O, no
// clock. Everything that decides whether an edge belongs here has already happened in
// `refuse_edge` by the time `assert_edge` is called.
class RelationGraph {
  public:
    explicit RelationGraph(std::string owner_id) : owner_id_(std::move(owner_id)) {}

    [[nodiscard]] const std::string& owner_id() const noexcept { return owner_id_; }

    // Add an edge that has already passed `refuse_edge`.
    //
    // Model hypotheses go into a separate list. They are readable — the point of a lead is
    // that someone chases it — but they are never traversed by `path_to_owner`, so a
    // guess can never become the justification for holding a record.
    void assert_edge(RelationEdge edge);
    void hypothesise_edge(RelationEdge edge);

    [[nodiscard]] const std::vector<RelationEdge>& asserted() const noexcept { return asserted_; }
    [[nodiscard]] const std::vector<RelationEdge>& hypotheses() const noexcept {
        return hypotheses_;
    }

    // The chain of people from the operator to `subject_id`, or nothing if there is none.
    //
    // Edges are treated as undirected for reachability: "Nelson employs Ana" and "Ana is
    // employed by Nelson" describe the same tie, and a graph where the justification
    // depended on which way round it was entered would be arbitrary.
    [[nodiscard]] std::optional<std::vector<std::string>> path_to_owner(
        std::string_view subject_id) const;

    // Everyone connected to both of these people.
    //
    // "Amigos en comun": the answer a profile wants when it shows two people side by side.
    // Computed over asserted edges only, for the same reason `path_to_owner` is — a shared
    // acquaintance that exists because a model guessed at one of the two edges is not a
    // shared acquaintance.
    //
    // Sorted, and never includes either endpoint: a direct edge between the two is a
    // relationship, not a mutual one, and returning them would have every pair share
    // themselves.
    [[nodiscard]] std::vector<std::string> mutual(std::string_view a,
                                                  std::string_view b) const;

    // Everyone directly connected to this person, over asserted edges. Sorted.
    [[nodiscard]] std::vector<std::string> neighbours(std::string_view id) const;

    // How many relationships away from the operator someone is. The operator is 0.
    [[nodiscard]] std::optional<std::size_t> degree(std::string_view subject_id) const;

    // Everyone in the graph with no path back to the operator.
    //
    // These are the records with no reason to exist. Reported rather than deleted: this
    // class decides nothing about retention, and a graph that quietly dropped nodes would
    // hide the bug that produced them.
    [[nodiscard]] std::vector<std::string> orphans() const;

  private:
    std::string owner_id_;
    std::vector<RelationEdge> asserted_;
    std::vector<RelationEdge> hypotheses_;
};

}  // namespace sentry::identity
