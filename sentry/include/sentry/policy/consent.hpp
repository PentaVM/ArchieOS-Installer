// Consent — permission to look at a place is not permission to identify a person.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Consent
//
// A system that can watch a camera and run a face model is one careless conjunction away
// from identifying everyone who walks past. The conjunction is prevented here, by making
// the permitted operation the *intersection* of three independent authorities:
//
//     operator policy   ∩   subject consent   ∩   sensor authority   =   permitted
//
// None of the three can substitute for another. Owning the camera does not grant
// consent from the people in front of it. A subject consenting to be tracked does not
// authorize a camera the operator never approved. And an operator policy that forbids
// biometrics is not overridden by a subject who would have allowed it.
//
// Where identity consent is absent, work does not stop — it degrades to anonymous. A
// person with no consent on file is still counted for occupancy and still avoided by a
// robot; they are simply never resolved to a name.

#pragma once

#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "sentry/world/time.hpp"

namespace sentry::policy {

using sentry::world::TimePoint;

// What a lease permits. Each is granted separately: consenting to be located by GPS is
// not consenting to face recognition, and the scopes are deliberately not ordered so
// that no code can accidentally treat one as implying another.
enum class ConsentScope {
    // Device location, from a phone or tag the subject carries.
    GpsLocation,
    // Resolving a camera track to this subject, without biometrics — by device
    // co-location, an operator's visual identification, or a worn tag.
    CameraIdentity,
    // Face or gait embeddings.
    BiometricIdentity,
    // Proximity radios: BLE, UWB.
    Proximity,
    // Re-examining this subject's history. Distinct from live tracking: someone may
    // agree to be tracked now and not to having last year re-analysed.
    HistoricalReplay,
    // Sending anything about this subject off the node.
    CloudProcessing,
};

[[nodiscard]] std::string_view name(ConsentScope scope) noexcept;

// What kind of evidence backs the lease.
enum class ConsentEvidence {
    // The subject signed something.
    SubjectSigned,
    // The subject is the operator, consenting for themselves.
    SelfConsent,
    // The operator attests they obtained permission out of band. Weaker, and policy may
    // refuse it for the scopes that matter most.
    OperatorAttested,
    // Carried in from another system.
    ImportedAuthorization,
};

[[nodiscard]] std::string_view name(ConsentEvidence evidence) noexcept;

// Whether the lease has been withdrawn.
enum class RevocationState {
    Active,
    // Withdrawn by the subject or the operator. Terminal.
    Revoked,
};

// A standing grant from one subject.
struct ConsentLease {
    std::string subject_id{};
    std::vector<ConsentScope> scopes{};
    TimePoint valid_from{};
    // Absent means standing. A lease does not lapse because a sensor went down, a
    // process restarted, or a model changed its mind: only the clock, an explicit
    // revocation, or a policy change can end it.
    std::optional<TimePoint> valid_until{};
    // Where the lease applies. Empty means everywhere the operator's policy allows.
    std::vector<std::string> spatial_scope{};
    ConsentEvidence evidence = ConsentEvidence::OperatorAttested;
    RevocationState revocation = RevocationState::Active;
    // How long derived data about this subject may be kept, in days. Zero means the
    // operator's default.
    int retention_days = 0;

    [[nodiscard]] bool covers(ConsentScope scope) const noexcept;

    // Whether the lease is in force at `now`.
    [[nodiscard]] bool active_at(TimePoint now) const noexcept {
        if (revocation != RevocationState::Active) {
            return false;
        }
        if (now < valid_from) {
            return false;
        }
        if (valid_until && now >= *valid_until) {
            return false;
        }
        return true;
    }
};

// What the operator's policy permits on this node, regardless of any subject's wishes.
//
// This is the ceiling. A subject can consent to biometric identification on a node whose
// operator has disabled biometrics entirely, and the answer is still no.
struct OperatorPolicy {
    bool biometrics_enabled = false;
    bool cloud_export_enabled = false;
    bool historical_replay_enabled = true;
    // Evidence classes this operator accepts. An empty list accepts none, which is a
    // safe default only because the constructor of a real policy always sets it.
    std::vector<ConsentEvidence> accepted_evidence{};

    [[nodiscard]] bool accepts(ConsentEvidence evidence) const noexcept;
    [[nodiscard]] bool permits(ConsentScope scope) const noexcept;
};

// Why an identity operation was refused. Each maps to a different fix, which is why
// this is an enum and not a bool.
enum class ConsentRefusal {
    // No lease exists for this subject.
    NoLease,
    // A real lease exists, but belongs to a different subject. This is an integrity
    // failure, not a permission the owner may satisfy by reusing somebody else's grant.
    LeaseSubjectMismatch,
    // A lease exists but does not cover the scope being attempted.
    ScopeNotGranted,
    // The lease has expired.
    Expired,
    // The lease was explicitly withdrawn.
    Revoked,
    // The operator's policy forbids this scope on this node, whatever the subject said.
    ForbiddenByOperatorPolicy,
    // The lease is backed by evidence this operator does not accept for this scope.
    EvidenceNotAccepted,
    // The camera or sensor is not authorized, so there is nothing to identify from.
    SensorNotAuthorized,
    // The lease does not extend to where this happened.
    OutsideSpatialScope,
};

[[nodiscard]] std::string_view describe(ConsentRefusal refusal) noexcept;

// One identity operation being attempted.
struct IdentityRequest {
    std::string subject_id{};
    ConsentScope scope{};
    // Where it happened. Empty when the operation has no location.
    std::string location{};
    // Whether the sensor the data came from is itself authorized. Supplied by the
    // camera layer; this module does not reach across to compute it, so the two
    // authorities stay genuinely independent.
    bool sensor_authorized = false;
    TimePoint at{};
};

// The intersection. Returns nothing when the operation is permitted.
//
// Checks the operator's ceiling first: a node with biometrics disabled should say so
// regardless of whether a lease happens to exist, because "no lease" would send an
// operator looking for the wrong fix.
[[nodiscard]] std::optional<ConsentRefusal> refuse(const OperatorPolicy& policy,
                                                   const std::optional<ConsentLease>& lease,
                                                   const IdentityRequest& request) noexcept;

// How sure Sentry is that a track belongs to a subject.
//
// Ordered, so a stronger association can be compared against a weaker one — but never
// collapsed into a single float, because the evidence behind Likely-from-appearance and
// Likely-from-device-colocation is not interchangeable.
enum class IdentityAssociation {
    Unknown,
    Candidate,
    Likely,
    Corroborated,
    Confirmed,
    // Actively ruled out. Distinct from Unknown: we looked and it is not them.
    Rejected,
};

[[nodiscard]] std::string_view name(IdentityAssociation state) noexcept;

// Evidence for or against one association.
struct AssociationEvidence {
    // Visual similarity, device co-location, an operator's identification.
    bool appearance_match = false;
    bool device_colocation = false;
    bool operator_identified = false;
    // Contradiction: the subject's authorized location provider put them somewhere else
    // at the same moment.
    bool contradicted_by_location = false;
    // Contradiction: the two sightings are too far apart to be the same person in the
    // time available.
    bool contradicted_by_transit_time = false;
};

// Resolve evidence to an association state.
//
// A hard contradiction defeats any amount of similarity. Two cameras that cannot be
// walked between in the time available did not see the same person, however alike the
// two crops look — and appearance matching is exactly the kind of evidence that is
// confidently wrong about twins, uniforms and dark coats.
[[nodiscard]] IdentityAssociation resolve(const AssociationEvidence& evidence) noexcept;

// What anonymous processing is always permitted, with no lease at all.
//
// Not a loophole: none of these resolve to a person. A robot that will not stop for
// someone it has no consent record for is a worse outcome than one that counts an
// anonymous obstacle, and occupancy figures need bodies, not names.
[[nodiscard]] constexpr bool permitted_without_consent(ConsentScope scope) noexcept {
    // Every named scope resolves or exports identity. None is permitted without a lease.
    (void)scope;
    return false;
}

}  // namespace sentry::policy
