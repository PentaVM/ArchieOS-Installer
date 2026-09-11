#include "sentry/policy/consent.hpp"

#include <algorithm>

namespace sentry::policy {

std::string_view name(ConsentScope scope) noexcept {
    switch (scope) {
        case ConsentScope::GpsLocation:       return "gps_location";
        case ConsentScope::CameraIdentity:    return "camera_identity";
        case ConsentScope::BiometricIdentity: return "biometric_identity";
        case ConsentScope::Proximity:         return "proximity";
        case ConsentScope::HistoricalReplay:  return "historical_replay";
        case ConsentScope::CloudProcessing:   return "cloud_processing";
    }
    return "unknown";
}

std::string_view name(ConsentEvidence evidence) noexcept {
    switch (evidence) {
        case ConsentEvidence::SubjectSigned:         return "subject_signed";
        case ConsentEvidence::SelfConsent:           return "self_consent";
        case ConsentEvidence::OperatorAttested:      return "operator_attested";
        case ConsentEvidence::ImportedAuthorization: return "imported_authorization";
    }
    return "unknown";
}

std::string_view name(IdentityAssociation state) noexcept {
    switch (state) {
        case IdentityAssociation::Unknown:      return "unknown";
        case IdentityAssociation::Candidate:    return "candidate";
        case IdentityAssociation::Likely:       return "likely";
        case IdentityAssociation::Corroborated: return "corroborated";
        case IdentityAssociation::Confirmed:    return "confirmed";
        case IdentityAssociation::Rejected:     return "rejected";
    }
    return "unknown";
}

std::string_view describe(ConsentRefusal refusal) noexcept {
    switch (refusal) {
        case ConsentRefusal::NoLease:
            return "no consent lease exists for this subject; tracking continues "
                   "anonymously";
        case ConsentRefusal::LeaseSubjectMismatch:
            return "the supplied consent lease belongs to a different subject and cannot "
                   "be reused to identify this person";
        case ConsentRefusal::ScopeNotGranted:
            return "the lease does not cover this scope; consenting to one kind of "
                   "identification is not consenting to another";
        case ConsentRefusal::Expired:
            return "the consent lease has expired";
        case ConsentRefusal::Revoked:
            return "the consent lease was withdrawn";
        case ConsentRefusal::ForbiddenByOperatorPolicy:
            return "this node's operator policy forbids this scope, whatever the subject "
                   "consented to";
        case ConsentRefusal::EvidenceNotAccepted:
            return "the lease is backed by an evidence class this operator does not "
                   "accept for this scope";
        case ConsentRefusal::SensorNotAuthorized:
            return "the sensor is not authorized; consent does not create sensor "
                   "authority";
        case ConsentRefusal::OutsideSpatialScope:
            return "the lease does not extend to where this happened";
    }
    return "unknown refusal";
}

bool ConsentLease::covers(ConsentScope scope) const noexcept {
    return std::ranges::find(scopes, scope) != scopes.end();
}

bool OperatorPolicy::accepts(ConsentEvidence evidence) const noexcept {
    return std::ranges::find(accepted_evidence, evidence) != accepted_evidence.end();
}

bool OperatorPolicy::permits(ConsentScope scope) const noexcept {
    switch (scope) {
        case ConsentScope::BiometricIdentity:
            return biometrics_enabled;
        case ConsentScope::CloudProcessing:
            return cloud_export_enabled;
        case ConsentScope::HistoricalReplay:
            return historical_replay_enabled;
        case ConsentScope::GpsLocation:
        case ConsentScope::CameraIdentity:
        case ConsentScope::Proximity:
            return true;
    }
    return false;
}

std::optional<ConsentRefusal> refuse(const OperatorPolicy& policy,
                                     const std::optional<ConsentLease>& lease,
                                     const IdentityRequest& request) noexcept {
    // The operator's ceiling comes first. On a node with biometrics disabled the answer
    // is the same whether or not a lease exists, and reporting "no lease" would send an
    // operator off to collect a consent form that would not have helped.
    if (!policy.permits(request.scope)) {
        return ConsentRefusal::ForbiddenByOperatorPolicy;
    }

    // Sensor authority is a separate authority and cannot be supplied by consent. A
    // subject agreeing to be identified does not authorize a camera the operator never
    // approved, so this is checked independently rather than folded into the lease.
    if (!request.sensor_authorized) {
        return ConsentRefusal::SensorNotAuthorized;
    }

    if (!lease) {
        return ConsentRefusal::NoLease;
    }
    if (lease->subject_id != request.subject_id) {
        return ConsentRefusal::LeaseSubjectMismatch;
    }

    // Revocation and expiry are reported apart. "Withdrawn" and "lapsed" call for
    // different responses — one is a decision to respect, the other a renewal to seek.
    if (lease->revocation != RevocationState::Active) {
        return ConsentRefusal::Revoked;
    }
    if (!lease->active_at(request.at)) {
        return ConsentRefusal::Expired;
    }

    if (!lease->covers(request.scope)) {
        return ConsentRefusal::ScopeNotGranted;
    }

    if (!policy.accepts(lease->evidence)) {
        return ConsentRefusal::EvidenceNotAccepted;
    }

    // An empty spatial scope means everywhere the operator allows, which is the ordinary
    // case. A non-empty one is a genuine restriction and a location that is not in it —
    // including an operation with no location at all — falls outside.
    if (!lease->spatial_scope.empty()) {
        if (request.location.empty() ||
            std::ranges::find(lease->spatial_scope, request.location) ==
                lease->spatial_scope.end()) {
            return ConsentRefusal::OutsideSpatialScope;
        }
    }

    return std::nullopt;
}

IdentityAssociation resolve(const AssociationEvidence& evidence) noexcept {
    // A hard contradiction ends it. Two sightings too far apart to walk between were not
    // the same person, and an authorized location provider putting the subject elsewhere
    // outranks any similarity score — appearance matching is confidently wrong about
    // twins, uniforms and dark coats, and this is the check that stops it being believed.
    if (evidence.contradicted_by_location || evidence.contradicted_by_transit_time) {
        return IdentityAssociation::Rejected;
    }

    // A human who looked and said who it is.
    if (evidence.operator_identified) {
        return IdentityAssociation::Confirmed;
    }

    // Two independent kinds of evidence. Device co-location and appearance fail in
    // unrelated ways, so agreement between them means considerably more than either
    // repeated.
    if (evidence.device_colocation && evidence.appearance_match) {
        return IdentityAssociation::Corroborated;
    }

    // A carried device is strong on its own; appearance alone is not, and is capped
    // below it however good the match looked.
    if (evidence.device_colocation) {
        return IdentityAssociation::Likely;
    }
    if (evidence.appearance_match) {
        return IdentityAssociation::Candidate;
    }

    return IdentityAssociation::Unknown;
}

}  // namespace sentry::policy
