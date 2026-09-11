// Wanda — probabilistic social inference, and the reasons it is never an authority.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Identity
// ARCHITECTURE.md: "Wanda is a probabilistic social-inference mode within Cerebra, not an
// authority or a detector of factual intent."
//
// Wanda sits above every model rather than inside one. It is analytical: given evidence
// that already exists in the graph, it computes — deterministically, with no sampling and
// no generation — whether a tie is worth asking a person about. Same evidence in, same
// number out, on every machine, forever. That is the property that makes it safe to put
// at kernel level, and it is the property a language model cannot offer.
//
// # What Wanda is not allowed to do
//
// The dangerous operation in this subsystem was named long before this file existed:
// `EdgeBasis::CoPresenceInference` in `relationship.hpp`, refused for every relationship
// kind. Two people in the same room at the same time is not a friendship, and a year of
// that is not a social graph — it is a surveillance product assembled from people who
// consented to none of it.
//
// So Wanda produces hypotheses. A hypothesis is a question addressed to a human, carrying
// what would disprove it. The only route from hypothesis to asserted edge runs through
// `promote`, which demands that a participant declare it and rewrites the basis to say so.
// Nothing Wanda computes ever lands in the asserted graph on its own, and `path_to_owner`
// does not traverse hypotheses, so a guess can never become the justification for holding
// somebody's record.
//
// # Certainty is not available
//
// `confidence` is capped below one. Wanda is not a detector of factual intent and has no
// mechanism for being sure; a value of 1.0 would be a claim about the world rather than a
// summary of evidence. The cap is enforced in the type's construction rather than asked
// for politely.
//
// # Where models come in
//
// When the evidence is genuinely ambiguous, Wanda decides *that a model should be asked*
// and *which tier may be asked*. It never delegates the decision itself. Whatever comes
// back is absorbed at hypothesis standing and cannot raise it — `absorb` exists to make
// that mechanical rather than a convention, because the convention is what fails at 3am.

#pragma once

#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "sentry/identity/relationship.hpp"
#include "sentry/identity/subject.hpp"
#include "sentry/policy/consent.hpp"
#include "sentry/world/epistemic.hpp"
#include "sentry/world/time.hpp"

namespace sentry::identity::wanda {

using sentry::policy::ConsentLease;
using sentry::policy::OperatorPolicy;
using sentry::world::EpistemicState;
using sentry::world::TimePoint;

// A confidence in [0, kCeiling]. Deterministic, and never certain.
//
// A class rather than a double so that the ceiling cannot be forgotten at one call site.
// Whole permille internally: two runs on two machines produce bit-identical values, which
// matters because a hypothesis is compared against a threshold and a float that drifts by
// an ulp would make the same evidence escalate on one node and not on another.
class Confidence {
  public:
    // 0.95. Wanda summarises evidence; it does not know.
    static constexpr int kCeilingPermille = 950;

    constexpr Confidence() noexcept = default;

    // Clamps into range. Negative and over-ceiling inputs are programming errors that
    // should not become runtime surprises.
    static constexpr Confidence from_permille(int permille) noexcept {
        Confidence value;
        value.permille_ = permille < 0 ? 0 : (permille > kCeilingPermille ? kCeilingPermille
                                                                         : permille);
        return value;
    }

    [[nodiscard]] constexpr int permille() const noexcept { return permille_; }
    [[nodiscard]] constexpr double value() const noexcept {
        return static_cast<double>(permille_) / 1000.0;
    }
    [[nodiscard]] constexpr auto operator<=>(const Confidence&) const noexcept = default;

  private:
    int permille_ = 0;
};

// What Wanda is allowed to weigh.
//
// Counts of things that already happened, nothing more. There is no free-form feature bag
// here on purpose: a model that can be fed arbitrary features is a model whose reasoning
// nobody can reconstruct afterwards, and the entire point of putting this above the
// language models is that its reasoning can be reconstructed.
struct CoPresence {
    // Distinct encounters at which both subjects were present.
    int encounters = 0;
    // How many different places. Repetition at one place is weak evidence — a bus stop, a
    // lobby, a shared commute — and counting it as strong is how a stranger becomes a
    // friend in the graph.
    int distinct_places = 0;
    // How many different days. Ten sightings in one afternoon is one occasion.
    int distinct_days = 0;
};

// One observation that people were somewhere together.
struct Encounter {
    std::string encounter_id{};
    std::string place{};
    TimePoint at{};
    std::vector<std::string> participant_ids{};
};

// A proposed identity for someone seen but not resolved.
//
// Separate from a hypothesis about a *relationship*: this one is about who a body was,
// and it is the step where an `AnonymousTrack` would acquire a name if nothing stopped it.
struct Candidate {
    std::string encounter_id{};
    std::string subject_id{};
    sentry::policy::IdentityAssociation association =
        sentry::policy::IdentityAssociation::Unknown;
};

// What Wanda proposes: a tie worth asking a person about.
struct SocialHypothesis {
    std::string from_id{};
    std::string to_id{};
    RelationKind kind{};
    Confidence confidence{};
    // What would settle it. Present because a hypothesis nobody can disprove is not a
    // hypothesis, and because the person reading it deserves to know what to ask.
    std::string falsifier{};
    // After this, the hypothesis is stale and must not be counted. Evidence about people
    // ages: two colleagues who shared an office in 2024 may not have met since.
    TimePoint expires_at{};
    // Always `Inferred`. Held as a field so that a reader of a stored hypothesis sees the
    // standing without consulting a comment.
    EpistemicState state = EpistemicState::Inferred;
};

// Why Wanda refused to propose anything.
enum class WandaRefusal {
    // An endpoint is an anonymous track or a public figure. Proposing a tie to a track is
    // exactly how a track stops being anonymous.
    EndpointNotResolvable,
    // An endpoint has no consent lease covering camera identity. Someone who never agreed
    // to be resolved does not become a node because a sensor kept seeing them.
    EndpointHasNoIdentityConsent,
    // A tie from someone to themselves.
    SelfHypothesis,
    // Neither person is anyone the operator knows.
    //
    // The rule that keeps this from becoming a surveillance graph. Wanda may notice that
    // somebody the owner knows keeps appearing with a third person; it may not build a
    // web of relationships between strangers who merely share the owner's city.
    NeitherEndpointKnownToOwner,
    // Nothing happened. Zero encounters is not weak evidence, it is no evidence.
    NoEvidence,
    // The operator's policy forbids inferring social ties on this node at all.
    ForbiddenByOperatorPolicy,
};

[[nodiscard]] std::string_view describe(WandaRefusal refusal) noexcept;

// Whether the operator has enabled social inference on this node.
//
// A separate switch from biometrics: a node may resolve its owner's face and still have
// no business guessing who anybody's friends are.
struct WandaPolicy {
    bool social_inference_enabled = false;
    // How long a hypothesis stands before it must be re-earned, in days.
    int hypothesis_lifetime_days = 30;
    // The band in which Wanda considers the evidence genuinely ambiguous and worth asking
    // a model about. Outside it, Wanda has decided and no model is consulted.
    int ambiguous_from_permille = 300;
    int ambiguous_to_permille = 700;
};

// Score co-presence, deterministically.
//
// Weights are integers and the arithmetic is exact. Distinct places and distinct days
// count for more than raw sightings, because the thing that distinguishes an acquaintance
// from a stranger on the same commute is variety, not volume.
//
// Monotonic: more of any component never lowers the result. A scoring function that could
// fall as evidence accumulated would be impossible to reason about and impossible to
// explain to the person being asked about.
[[nodiscard]] Confidence score(const CoPresence& evidence) noexcept;

// Whether Wanda may propose anything about this pair. Nothing means it may.
//
// `from_known_to_owner` and `to_known_to_owner` are passed in rather than looked up: this
// header holds no graph, so `RelationGraph::degree` stays the single place reachability is
// decided.
[[nodiscard]] std::optional<WandaRefusal> refuse(const WandaPolicy& policy,
                                                 const SubjectRecord& from,
                                                 const SubjectRecord& to,
                                                 const std::optional<ConsentLease>& from_lease,
                                                 const std::optional<ConsentLease>& to_lease,
                                                 const CoPresence& evidence,
                                                 bool from_known_to_owner,
                                                 bool to_known_to_owner,
                                                 TimePoint now) noexcept;

// Propose a tie, or explain why not.
//
// Returns a hypothesis and never an edge. There is deliberately no overload that returns
// a `RelationEdge` at asserted standing: the type system is where this rule lives.
[[nodiscard]] std::optional<SocialHypothesis> propose(const WandaPolicy& policy,
                                                      const SubjectRecord& from,
                                                      const SubjectRecord& to,
                                                      const std::optional<ConsentLease>& from_lease,
                                                      const std::optional<ConsentLease>& to_lease,
                                                      const CoPresence& evidence,
                                                      bool from_known_to_owner,
                                                      bool to_known_to_owner,
                                                      TimePoint now) noexcept;

// Whether a hypothesis is still standing at `now`.
[[nodiscard]] bool is_live(const SocialHypothesis& hypothesis, TimePoint now) noexcept;

// The edge a hypothesis becomes when a person declares it true.
//
// The only route out of hypothesis standing. The declarer must be one of the two people
// the hypothesis is about — the same rule `refuse_edge` enforces for personal ties — and
// the basis is rewritten to record who said so. Wanda's evidence survives only as a note,
// because the reason the edge exists afterwards is that somebody declared it, not that a
// score crossed a threshold.
[[nodiscard]] std::optional<RelationEdge> promote(const SocialHypothesis& hypothesis,
                                                  std::string_view declarer_id,
                                                  TimePoint now);

// ------------------------------------------------------------------ escalation to models

// Which tier of model, if any, may be asked to look at an ambiguous case.
enum class ModelTier {
    // Wanda has decided. Asking a model would add a confident opinion to a settled
    // question, which is the mechanism by which hallucinations enter a system that had
    // no need of them.
    NotNeeded,
    // A model on this machine may be asked. Nothing leaves the node.
    LocalOnly,
    // A frontier model over an API may be asked, because both the operator's policy and
    // the subject's lease allow this person's data to leave.
    FrontierPermitted,
    // The case is ambiguous and no model may be asked, so it stays ambiguous. A correct
    // outcome: an unanswered question is better than an unauthorised answer.
    RefusedByPolicy,
};

[[nodiscard]] std::string_view name(ModelTier tier) noexcept;

struct EscalationDecision {
    ModelTier tier = ModelTier::NotNeeded;
    // Why, in words, for the operator reading an audit trail.
    std::string reason{};
};

// Decide whether a model should see this, and which one may.
//
// Wanda decides; the model advises. The frontier tier additionally requires that *both*
// subjects have granted cloud processing, because sending one person's data to answer a
// question about two of them exports both.
[[nodiscard]] EscalationDecision escalate(const WandaPolicy& policy,
                                          const OperatorPolicy& operator_policy,
                                          const SocialHypothesis& hypothesis,
                                          const std::optional<ConsentLease>& from_lease,
                                          const std::optional<ConsentLease>& to_lease,
                                          TimePoint now) noexcept;

// Fold a model's answer back in, at hypothesis standing and no higher.
//
// A model may move Wanda's confidence within the band it was already unsure about, and
// may not push it past the ambiguity band in either direction: a generative system that
// could drive a social claim to near-certainty on its own has become the authority this
// module exists to prevent. The returned hypothesis is always `Inferred`, never
// corroborated and never verified, whatever the model reported about itself.
[[nodiscard]] SocialHypothesis absorb(const WandaPolicy& policy,
                                      const SocialHypothesis& hypothesis,
                                      Confidence model_confidence) noexcept;

}  // namespace sentry::identity::wanda
