#include "sentry/identity/wanda.hpp"

#include <algorithm>

namespace sentry::identity::wanda {
namespace {

// Milliseconds in a day, for hypothesis lifetimes.
constexpr std::int64_t kDayMs = 24LL * 60LL * 60LL * 1000LL;

// Where repeated sightings stop telling you anything new.
constexpr int kSaturatingEncounters = 4;

// Whether a lease permits resolving this person from a sensor at all.
//
// `CameraIdentity` rather than `BiometricIdentity`: co-presence evidence comes from
// knowing who was in a room, which a worn tag or an operator's identification can
// establish without a face model ever running.
bool may_be_resolved(const std::optional<ConsentLease>& lease, TimePoint now) noexcept {
    return lease.has_value() && lease->active_at(now) &&
           lease->covers(sentry::policy::ConsentScope::CameraIdentity);
}

bool may_leave_the_node(const std::optional<ConsentLease>& lease, TimePoint now) noexcept {
    return lease.has_value() && lease->active_at(now) &&
           lease->covers(sentry::policy::ConsentScope::CloudProcessing);
}

}  // namespace

std::string_view describe(WandaRefusal refusal) noexcept {
    switch (refusal) {
        case WandaRefusal::EndpointNotResolvable:
            return "one of these people may never be resolved from a sensor";
        case WandaRefusal::EndpointHasNoIdentityConsent:
            return "one of these people has not consented to being identified";
        case WandaRefusal::SelfHypothesis:
            return "a person cannot have a relationship with themselves";
        case WandaRefusal::NeitherEndpointKnownToOwner:
            return "neither of these people is anyone the operator knows";
        case WandaRefusal::NoEvidence:
            return "nothing was observed";
        case WandaRefusal::ForbiddenByOperatorPolicy:
            return "this node does not infer social ties";
    }
    return "refused";
}

std::string_view name(ModelTier tier) noexcept {
    switch (tier) {
        case ModelTier::NotNeeded:
            return "not_needed";
        case ModelTier::LocalOnly:
            return "local_only";
        case ModelTier::FrontierPermitted:
            return "frontier_permitted";
        case ModelTier::RefusedByPolicy:
            return "refused_by_policy";
    }
    return "not_needed";
}

Confidence score(const CoPresence& evidence) noexcept {
    // Negative counts are a caller error; treat them as absent rather than letting them
    // subtract from real evidence.
    const int encounters = std::max(evidence.encounters, 0);
    const int places = std::max(evidence.distinct_places, 0);
    const int days = std::max(evidence.distinct_days, 0);

    // Raw sightings saturate. After about four, another one at the same place on the
    // same day carries no new information — it is the same fact observed again — and
    // letting volume accumulate without bound is precisely how a shared commute
    // out-scores a real acquaintance. Variety is what distinguishes them, so places and
    // days keep counting and encounters stop.
    const int weighted = 30 * std::min(encounters, kSaturatingEncounters) + 90 * places +
                         80 * days;
    return Confidence::from_permille(weighted);
}

std::optional<WandaRefusal> refuse(const WandaPolicy& policy,
                                   const SubjectRecord& from,
                                   const SubjectRecord& to,
                                   const std::optional<ConsentLease>& from_lease,
                                   const std::optional<ConsentLease>& to_lease,
                                   const CoPresence& evidence,
                                   bool from_known_to_owner,
                                   bool to_known_to_owner,
                                   TimePoint now) noexcept {
    // The operator's ceiling first, so a node with inference disabled says so rather than
    // sending somebody looking for a missing consent lease.
    if (!policy.social_inference_enabled) {
        return WandaRefusal::ForbiddenByOperatorPolicy;
    }
    if (from.subject_id == to.subject_id) {
        return WandaRefusal::SelfHypothesis;
    }
    // Resolvability is a property of the kind and is checked before any evidence is
    // weighed — the same ordering `subject.hpp` uses, and for the same reason.
    if (!from.is_resolvable() || !to.is_resolvable()) {
        return WandaRefusal::EndpointNotResolvable;
    }
    if (!may_be_resolved(from_lease, now) || !may_be_resolved(to_lease, now)) {
        return WandaRefusal::EndpointHasNoIdentityConsent;
    }
    if (!from_known_to_owner && !to_known_to_owner) {
        return WandaRefusal::NeitherEndpointKnownToOwner;
    }
    if (evidence.encounters <= 0) {
        return WandaRefusal::NoEvidence;
    }
    return std::nullopt;
}

std::optional<SocialHypothesis> propose(const WandaPolicy& policy,
                                        const SubjectRecord& from,
                                        const SubjectRecord& to,
                                        const std::optional<ConsentLease>& from_lease,
                                        const std::optional<ConsentLease>& to_lease,
                                        const CoPresence& evidence,
                                        bool from_known_to_owner,
                                        bool to_known_to_owner,
                                        TimePoint now) noexcept {
    if (refuse(policy, from, to, from_lease, to_lease, evidence, from_known_to_owner,
               to_known_to_owner, now)
            .has_value()) {
        return std::nullopt;
    }

    SocialHypothesis hypothesis;
    hypothesis.from_id = from.subject_id;
    hypothesis.to_id = to.subject_id;
    // The weakest kind that fits. Wanda noticing two people together is evidence that
    // they know each other, not evidence of how — proposing `Family` from a corridor
    // would be asserting something about a life nobody described.
    hypothesis.kind = RelationKind::Friend;
    hypothesis.confidence = score(evidence);
    hypothesis.falsifier =
        "ask either of them; a denial from a participant ends this, and no amount of "
        "further co-presence revives it";
    const std::int64_t lifetime =
        static_cast<std::int64_t>(std::max(policy.hypothesis_lifetime_days, 0)) * kDayMs;
    hypothesis.expires_at = TimePoint{now.millis() + lifetime};
    hypothesis.state = EpistemicState::Inferred;
    return hypothesis;
}

bool is_live(const SocialHypothesis& hypothesis, TimePoint now) noexcept {
    return now < hypothesis.expires_at;
}

std::optional<RelationEdge> promote(const SocialHypothesis& hypothesis,
                                    std::string_view declarer_id,
                                    TimePoint now) {
    if (!is_live(hypothesis, now)) {
        return std::nullopt;
    }
    // Only somebody the hypothesis is about may turn it into an assertion. An operator
    // declaring a friendship between two other people is describing lives that did not
    // ask, which is the rule `refuse_edge` already enforces for personal ties.
    if (declarer_id != hypothesis.from_id && declarer_id != hypothesis.to_id) {
        return std::nullopt;
    }

    RelationEdge edge;
    edge.from_id = hypothesis.from_id;
    edge.to_id = hypothesis.to_id;
    edge.kind = hypothesis.kind;
    // Not `ModelHypothesis`, and emphatically not `CoPresenceInference`. The edge exists
    // because a person said so; Wanda's part is a note in the margin.
    edge.basis = EdgeBasis::SubjectDeclared;
    edge.declarer_id = std::string(declarer_id);
    edge.source_note = "confirmed by a participant after Wanda raised it";
    return edge;
}

EscalationDecision escalate(const WandaPolicy& policy,
                            const OperatorPolicy& operator_policy,
                            const SocialHypothesis& hypothesis,
                            const std::optional<ConsentLease>& from_lease,
                            const std::optional<ConsentLease>& to_lease,
                            TimePoint now) noexcept {
    if (!is_live(hypothesis, now)) {
        return {ModelTier::NotNeeded, "the hypothesis has expired and is not worth asking about"};
    }
    const int permille = hypothesis.confidence.permille();
    if (permille < policy.ambiguous_from_permille) {
        return {ModelTier::NotNeeded, "the evidence is too thin to be worth a model's opinion"};
    }
    if (permille > policy.ambiguous_to_permille) {
        return {ModelTier::NotNeeded, "Wanda has decided; a model would only add confidence"};
    }

    // Ambiguous. A model may help — but sending one person's data to answer a question
    // about two of them exports both, so both must have agreed.
    if (!operator_policy.cloud_export_enabled) {
        return {ModelTier::LocalOnly, "this node does not send anything about people off it"};
    }
    if (!may_leave_the_node(from_lease, now) || !may_leave_the_node(to_lease, now)) {
        return {ModelTier::LocalOnly,
                "a question about two people exports both, and one has not agreed to that"};
    }
    return {ModelTier::FrontierPermitted,
            "both subjects permit cloud processing and the operator allows export"};
}

SocialHypothesis absorb(const WandaPolicy& policy,
                        const SocialHypothesis& hypothesis,
                        Confidence model_confidence) noexcept {
    SocialHypothesis absorbed = hypothesis;
    // A model may move the number inside the band Wanda was already unsure about, and no
    // further. Letting it out of the band in either direction would make a generative
    // system the thing that decides a social claim.
    const int low = policy.ambiguous_from_permille;
    const int high = policy.ambiguous_to_permille;
    const int clamped = std::clamp(model_confidence.permille(), low, high);
    absorbed.confidence = Confidence::from_permille(clamped);
    // Whatever the model said about its own certainty, this is still inferred.
    absorbed.state = EpistemicState::Inferred;
    return absorbed;
}

}  // namespace sentry::identity::wanda
