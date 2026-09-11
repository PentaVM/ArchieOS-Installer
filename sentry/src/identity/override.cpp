#include "sentry/identity/override.hpp"

#include <string>

namespace sentry::identity {

OverrideVerdict can_override(EdgeRefusal refusal, const OwnerOverride& override_) noexcept {
    if (!override_.is_stated()) {
        return OverrideVerdict::OverrideNotStated;
    }

    switch (refusal) {
        case EdgeRefusal::PersonalTieFromPublicRecord:
            // Refused to the pipeline because §40 is about what gets harvested at volume.
            // An owner reading a public obituary and recording that two people are
            // siblings is doing research, not harvesting, and refusing them that would
            // make the graph useless for the work it exists to support.
            return OverrideVerdict::LiftedOnTheRecord;

        case EdgeRefusal::InferredFromCoPresence:
            // The dangerous version of this is automatic and unattended: a job walking a
            // year of camera history and emitting a social graph nobody asked for. That
            // stays refused, because `refuse_edge` is what the pipeline calls and it has
            // no override.
            //
            // The owner looking at one pair of people, deciding they are colleagues, and
            // saying so is a different act with a different failure mode. Available, and
            // recorded, so the graph can still show which of its edges came from a person
            // agreeing with an observation rather than from evidence of a relationship.
            return OverrideVerdict::LiftedOnTheRecord;

        case EdgeRefusal::NotDeclaredByAParticipant:
        case EdgeRefusal::GuestTieMustInvolveOwner:
            // Both restrict who may speak for whom, which is the owner's call on their own
            // node.
            return OverrideVerdict::LiftedOnTheRecord;

        case EdgeRefusal::EndpointNotIdentifiable:
            // An anonymous track has no identity, so there is nothing an override could
            // permit. Saying yes here would write "track-17 is Ana's colleague" while
            // track-17 still refers to nobody. The fix is to identify the track first —
            // which the owner may do, with `attested_by_owner` and a reclassification.
            return OverrideVerdict::NotAPermissionQuestion;

        case EdgeRefusal::SelfEdge:
            // Not forbidden. Meaningless, and it breaks every traversal that assumes it
            // cannot happen.
            return OverrideVerdict::NotAPermissionQuestion;

        case EdgeRefusal::GenerativeCannotAssert:
            // The one that stands against the owner, and stands for them. See `promote`.
            return OverrideVerdict::WouldCorruptWhatArchieKnows;
    }
    return OverrideVerdict::NotAPermissionQuestion;
}

OverrideVerdict can_override(AttributeRefusal refusal, const OwnerOverride& override_) noexcept {
    if (!override_.is_stated()) {
        return OverrideVerdict::OverrideNotStated;
    }

    switch (refusal) {
        case AttributeRefusal::PrivateDataOnPublicFigure:
            // §40 constrains what the system collects on its own. An owner who has a
            // reason to record a contact detail for someone they are researching is making
            // a decision they can be asked about, which is what the record is for.
            return OverrideVerdict::LiftedOnTheRecord;

        case AttributeRefusal::ScopeNotGranted:
        case AttributeRefusal::ForbiddenByOperatorPolicy:
        case AttributeRefusal::EvidenceNotAccepted:
            // Permission questions on the owner's own node.
            return OverrideVerdict::LiftedOnTheRecord;

        case AttributeRefusal::LeaseSubjectMismatch:
            // A grant issued for somebody else is malformed evidence for this request.
            // The owner can record a new, correctly bound grant; they cannot convert an
            // unrelated subject's consent into this subject's consent by override.
            return OverrideVerdict::NotAPermissionQuestion;

        case AttributeRefusal::AnonymousTrackCannotCarryIdentity:
            // Same as the edge case: there is no identity here to attach anything to, and
            // an override would write a statement about nobody.
            return OverrideVerdict::NotAPermissionQuestion;
    }
    return OverrideVerdict::NotAPermissionQuestion;
}

RelationEdge promote(const RelationEdge& hypothesis, const OwnerOverride& override_) {
    RelationEdge declared = hypothesis;
    declared.basis = EdgeBasis::OwnerDeclared;

    // Both facts, kept together. That Archie suggested it and that a person accepted it
    // are each less informative alone than the pair is: the first without the second looks
    // like an unreviewed guess, and the second without the first hides where the idea came
    // from.
    std::string note = "promoted from ";
    note += name(hypothesis.basis);
    note += " by owner ";
    note += override_.owner_id;
    note += ": ";
    note += override_.justification;
    if (!hypothesis.source_note.empty()) {
        note += " (original note: ";
        note += hypothesis.source_note;
        note += ")";
    }
    declared.source_note = std::move(note);

    return declared;
}

}  // namespace sentry::identity
