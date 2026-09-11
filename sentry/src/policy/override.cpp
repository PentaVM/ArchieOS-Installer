#include "sentry/policy/override.hpp"

namespace sentry::policy {

std::string_view name(OverrideVerdict verdict) noexcept {
    switch (verdict) {
        case OverrideVerdict::LiftedOnTheRecord:          return "lifted_on_the_record";
        case OverrideVerdict::NotAPermissionQuestion:     return "not_a_permission_question";
        case OverrideVerdict::WouldCorruptWhatArchieKnows: return "would_corrupt_knowledge";
        case OverrideVerdict::OverrideNotStated:          return "override_not_stated";
    }
    return "unknown";
}

std::string_view describe(OverrideVerdict verdict) noexcept {
    switch (verdict) {
        case OverrideVerdict::LiftedOnTheRecord:
            return "the owner's authority covers this; proceeding, and writing a record "
                   "that names them and their stated reason";
        case OverrideVerdict::NotAPermissionQuestion:
            return "nothing here is being withheld for permission reasons, so overriding "
                   "would not make the operation possible — only untrue. Supply the "
                   "missing thing instead";
        case OverrideVerdict::WouldCorruptWhatArchieKnows:
            return "lifting this would let a model's guess be stamped as something Archie "
                   "verified, and the owner would lose the ability to tell the two apart. "
                   "Read the hypothesis and declare it yourself instead";
        case OverrideVerdict::OverrideNotStated:
            return "an override needs an owner and a stated reason; one with neither is "
                   "indistinguishable from a bug six months later";
    }
    return "unknown verdict";
}

OverrideVerdict can_override(ConsentRefusal refusal, const OwnerOverride& override_) noexcept {
    if (!override_.is_stated()) {
        return OverrideVerdict::OverrideNotStated;
    }

    switch (refusal) {
        case ConsentRefusal::NoLease:
        case ConsentRefusal::ScopeNotGranted:
        case ConsentRefusal::Expired:
        case ConsentRefusal::Revoked:
        case ConsentRefusal::ForbiddenByOperatorPolicy:
        case ConsentRefusal::EvidenceNotAccepted:
        case ConsentRefusal::OutsideSpatialScope:
            // All of these are the same question in different clothes: does permission
            // exist on this node? On this node, the owner is who decides that. Each is
            // lifted, and each leaves a record — including `Revoked`, which is the one an
            // operator is most likely to be asked about afterwards.
            return OverrideVerdict::LiftedOnTheRecord;

        case ConsentRefusal::LeaseSubjectMismatch:
            // A lease naming somebody else is malformed evidence for this request, not
            // a missing permission. The owner can issue the right grant, but cannot turn
            // an unrelated subject's consent into this subject's consent by override.
            return OverrideVerdict::NotAPermissionQuestion;

        case ConsentRefusal::SensorNotAuthorized:
            // Factual, not permissive. The camera is not approved or not reachable, and
            // instructing the system to proceed anyway does not produce a frame. The fix
            // is to authorize the camera, which is also the owner's to do — just
            // somewhere else, in `camera/authority.hpp`.
            return OverrideVerdict::NotAPermissionQuestion;
    }
    return OverrideVerdict::NotAPermissionQuestion;
}

ConsentLease attested_by_owner(const OwnerOverride& override_,
                               std::string subject_id,
                               std::vector<ConsentScope> scopes) noexcept {
    ConsentLease lease;
    lease.subject_id = std::move(subject_id);
    lease.scopes = std::move(scopes);
    lease.valid_from = override_.at;
    // Standing until withdrawn. An owner attesting to permission they obtained in person
    // has no natural expiry to offer, and inventing one would produce a lease that lapses
    // for a reason nobody chose.
    lease.valid_until = std::nullopt;
    // The subject consenting for themselves would be `SelfConsent`; this is the operator
    // saying they have permission. Recorded as what it is, so that a later reader can see
    // the difference — and so that an operator policy which does not accept attestation
    // for biometrics still does not accept it here.
    lease.evidence = ConsentEvidence::OperatorAttested;
    lease.revocation = RevocationState::Active;
    return lease;
}

OperatorPolicy unrestricted_policy() noexcept {
    OperatorPolicy policy;
    policy.biometrics_enabled = true;
    policy.cloud_export_enabled = true;
    policy.historical_replay_enabled = true;
    // Every evidence class, attestation included. This is the configuration of an owner
    // who has said "my node, my rules" — and it is a deliberate choice they make, not the
    // default, because a default that accepted everything would make the distinction
    // between a signature and an assertion invisible to whoever inherits the machine.
    policy.accepted_evidence = {
        ConsentEvidence::SubjectSigned,
        ConsentEvidence::SelfConsent,
        ConsentEvidence::OperatorAttested,
        ConsentEvidence::ImportedAuthorization,
    };
    return policy;
}

}  // namespace sentry::policy
