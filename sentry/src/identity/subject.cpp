#include "sentry/identity/subject.hpp"

#include "sentry/camera/authority.hpp"

namespace sentry::identity {

namespace {

constexpr std::int64_t kMaximumOwnerBiometricCaptureAgeMs = 60'000;
constexpr std::int64_t kMaximumOwnerBiometricChallengeLifetimeMs = 30'000;

bool is_lowercase_sha256(std::string_view value) noexcept {
    if (value.size() != 64U) {
        return false;
    }
    for (const char character : value) {
        if (!((character >= '0' && character <= '9') ||
              (character >= 'a' && character <= 'f'))) {
            return false;
        }
    }
    return true;
}

OwnerBiometricRefusal map_attribute_refusal(AttributeRefusal refusal) noexcept {
    switch (refusal) {
        case AttributeRefusal::PrivateDataOnPublicFigure:
        case AttributeRefusal::AnonymousTrackCannotCarryIdentity:
            return OwnerBiometricRefusal::SubjectIsNotOwner;
        case AttributeRefusal::ScopeNotGranted:
        case AttributeRefusal::LeaseSubjectMismatch:
            return OwnerBiometricRefusal::BiometricConsentNotGranted;
        case AttributeRefusal::ForbiddenByOperatorPolicy:
            return OwnerBiometricRefusal::BiometricPolicyDisabled;
        case AttributeRefusal::EvidenceNotAccepted:
            return OwnerBiometricRefusal::BiometricEvidenceNotAccepted;
    }
    return OwnerBiometricRefusal::BiometricConsentNotGranted;
}

OwnerBiometricRefusal map_consent_refusal(
    sentry::policy::ConsentRefusal refusal) noexcept {
    switch (refusal) {
        case sentry::policy::ConsentRefusal::NoLease:
        case sentry::policy::ConsentRefusal::LeaseSubjectMismatch:
        case sentry::policy::ConsentRefusal::ScopeNotGranted:
            return OwnerBiometricRefusal::BiometricConsentNotGranted;
        case sentry::policy::ConsentRefusal::Expired:
            return OwnerBiometricRefusal::BiometricConsentExpired;
        case sentry::policy::ConsentRefusal::Revoked:
            return OwnerBiometricRefusal::BiometricConsentRevoked;
        case sentry::policy::ConsentRefusal::ForbiddenByOperatorPolicy:
            return OwnerBiometricRefusal::BiometricPolicyDisabled;
        case sentry::policy::ConsentRefusal::EvidenceNotAccepted:
            return OwnerBiometricRefusal::BiometricEvidenceNotAccepted;
        case sentry::policy::ConsentRefusal::SensorNotAuthorized:
            return OwnerBiometricRefusal::SensorUnauthorized;
        case sentry::policy::ConsentRefusal::OutsideSpatialScope:
            return OwnerBiometricRefusal::SensorOutsideConsentScope;
    }
    return OwnerBiometricRefusal::BiometricConsentNotGranted;
}

}  // namespace

std::string_view name(SubjectKind kind) noexcept {
    switch (kind) {
        case SubjectKind::Owner:           return "owner";
        case SubjectKind::ConsentedPerson: return "consented_person";
        case SubjectKind::PublicFigure:    return "public_figure";
        case SubjectKind::AnonymousTrack:  return "anonymous_track";
    }
    return "unknown";
}

std::string_view name(AttributeClass attribute) noexcept {
    switch (attribute) {
        case AttributeClass::LegalName:           return "legal_name";
        case AttributeClass::PublicRole:          return "public_role";
        case AttributeClass::PublicAffiliation:   return "public_affiliation";
        case AttributeClass::PublicByline:        return "public_byline";
        case AttributeClass::PublicAccount:       return "public_account";
        case AttributeClass::PublicStatement:     return "public_statement";
        case AttributeClass::HomeAddress:         return "home_address";
        case AttributeClass::PersonalContact:     return "personal_contact";
        case AttributeClass::FamilyMembers:       return "family_members";
        case AttributeClass::PrivateTravel:       return "private_travel";
        case AttributeClass::HealthStatus:        return "health_status";
        case AttributeClass::BiometricTemplate:   return "biometric_template";
        case AttributeClass::DeviceIdentifier:    return "device_identifier";
        case AttributeClass::PreciseLiveLocation: return "precise_live_location";
    }
    return "unknown";
}

std::string_view describe(AttributeRefusal refusal) noexcept {
    switch (refusal) {
        case AttributeRefusal::PrivateDataOnPublicFigure:
            return "the public-figure record holds public professional data only; home "
                   "addresses, personal contacts, family details and private travel are "
                   "not collected about someone merely because they are in the news";
        case AttributeRefusal::AnonymousTrackCannotCarryIdentity:
            return "an anonymous track has no identity to attach anything to; attaching "
                   "one is how a track stops being anonymous";
        case AttributeRefusal::ScopeNotGranted:
            return "this attribute needs a consent scope the subject has not granted, or "
                   "granted and later withdrew";
        case AttributeRefusal::LeaseSubjectMismatch:
            return "the supplied consent lease belongs to a different subject and cannot "
                   "authorize an attribute about this person";
        case AttributeRefusal::ForbiddenByOperatorPolicy:
            return "the operator's policy forbids this class of data on this node, "
                   "whatever the subject consented to";
        case AttributeRefusal::EvidenceNotAccepted:
            return "the lease covers this scope but rests on evidence this operator does "
                   "not accept for it";
    }
    return "unknown refusal";
}

std::string_view name(OwnerBiometricMatch match) noexcept {
    switch (match) {
        case OwnerBiometricMatch::MatchedWithLiveness:   return "matched_with_liveness";
        case OwnerBiometricMatch::NoMatch:               return "no_match";
        case OwnerBiometricMatch::LivenessNotEstablished:return "liveness_not_established";
        case OwnerBiometricMatch::MatcherUnavailable:    return "matcher_unavailable";
    }
    return "unknown";
}

std::string_view describe(OwnerBiometricRefusal refusal) noexcept {
    switch (refusal) {
        case OwnerBiometricRefusal::SubjectIsNotOwner:
            return "the initial biometric gate can enroll or recognize the node owner only";
        case OwnerBiometricRefusal::SubjectIdentifierMismatch:
            return "the enrollment, subject and recognition result must name the same owner";
        case OwnerBiometricRefusal::InvalidEnrollment:
            return "the enrollment requires a non-empty owner id, a lower-case SHA-256 "
                   "template reference, and a non-future enrollment time";
        case OwnerBiometricRefusal::BiometricPolicyDisabled:
            return "the operator policy disables biometric identity on this node";
        case OwnerBiometricRefusal::BiometricConsentNotGranted:
            return "the owner has not granted an active biometric-identity lease";
        case OwnerBiometricRefusal::BiometricConsentExpired:
            return "the biometric-identity lease expired before this recognition attempt";
        case OwnerBiometricRefusal::BiometricConsentRevoked:
            return "the biometric-identity lease was revoked";
        case OwnerBiometricRefusal::BiometricEvidenceNotAccepted:
            return "the lease evidence is not accepted for biometric identity on this node";
        case OwnerBiometricRefusal::SensorUnauthorized:
            return "the sensor that supplied this result is not authorized";
        case OwnerBiometricRefusal::SensorOutsideConsentScope:
            return "the sensor is outside the owner's biometric consent scope";
        case OwnerBiometricRefusal::SensorNotNamed:
            return "a biometric recognition result must name its authorized sensor";
        case OwnerBiometricRefusal::SensorAuthorityRecordMismatch:
            return "the biometric result's sensor does not match the operator-authorized "
                   "camera record";
        case OwnerBiometricRefusal::CapturePrecedesEnrollment:
            return "the capture predates the current owner biometric enrollment";
        case OwnerBiometricRefusal::CaptureFromFuture:
            return "the capture time is later than the authority check";
        case OwnerBiometricRefusal::CaptureStale:
            return "the owner-presence capture is older than sixty seconds";
        case OwnerBiometricRefusal::InvalidChallenge:
            return "the owner-presence challenge must be a lower-case SHA-256 reference "
                   "with a positive lifetime no longer than thirty seconds";
        case OwnerBiometricRefusal::ChallengePrecedesEnrollment:
            return "the owner-presence challenge predates the current biometric enrollment";
        case OwnerBiometricRefusal::ChallengeMismatch:
            return "the isolated biometric worker did not bind this result to the current "
                   "owner-presence challenge";
        case OwnerBiometricRefusal::CapturePrecedesChallenge:
            return "the capture predates the owner-presence challenge";
        case OwnerBiometricRefusal::ChallengeExpired:
            return "the owner-presence challenge expired before this recognition was used";
        case OwnerBiometricRefusal::TemplateMismatch:
            return "the matcher result is not bound to the currently enrolled template";
        case OwnerBiometricRefusal::NoMatch:
            return "the isolated biometric worker reported no owner match";
        case OwnerBiometricRefusal::LivenessNotEstablished:
            return "the isolated biometric worker did not establish liveness";
        case OwnerBiometricRefusal::MatcherUnavailable:
            return "the isolated biometric worker was unavailable";
    }
    return "unknown owner biometric refusal";
}

std::optional<OwnerBiometricRefusal> refuse_owner_biometric_enrollment(
    const sentry::policy::OperatorPolicy& policy,
    const SubjectRecord& subject,
    const std::optional<sentry::policy::ConsentLease>& lease,
    const OwnerBiometricEnrollment& enrollment,
    sentry::world::TimePoint now) noexcept {
    if (subject.kind != SubjectKind::Owner) {
        return OwnerBiometricRefusal::SubjectIsNotOwner;
    }
    if (subject.subject_id.empty() || enrollment.owner_id.empty() ||
        !is_lowercase_sha256(enrollment.template_sha256) || enrollment.enrolled_at > now) {
        return OwnerBiometricRefusal::InvalidEnrollment;
    }
    if (enrollment.owner_id != subject.subject_id) {
        return OwnerBiometricRefusal::SubjectIdentifierMismatch;
    }
    // Preserve the semantic reason while retaining `refuse_attribute` as the authority
    // check below. The latter intentionally folds inactive leases together because an
    // attribute write only needs a yes/no answer; an owner-presence UI must distinguish
    // a withdrawal from a renewal that simply lapsed.
    if (policy.permits(sentry::policy::ConsentScope::BiometricIdentity) && lease) {
        if (lease->subject_id != subject.subject_id) {
            return OwnerBiometricRefusal::BiometricConsentNotGranted;
        }
        if (lease->revocation != sentry::policy::RevocationState::Active) {
            return OwnerBiometricRefusal::BiometricConsentRevoked;
        }
        if (lease->valid_until && now >= *lease->valid_until) {
            return OwnerBiometricRefusal::BiometricConsentExpired;
        }
    }
    if (const auto refusal = refuse_attribute(policy, subject, AttributeClass::BiometricTemplate,
                                              lease, now)) {
        return map_attribute_refusal(*refusal);
    }
    return std::nullopt;
}

std::optional<OwnerBiometricRefusal> refuse_owner_biometric_recognition(
    const sentry::policy::OperatorPolicy& policy,
    const SubjectRecord& subject,
    const std::optional<sentry::policy::ConsentLease>& lease,
    const OwnerBiometricEnrollment& enrollment,
    const OwnerBiometricChallenge& challenge,
    const OwnerBiometricAttempt& attempt,
    sentry::world::TimePoint now) noexcept {
    if (const auto refusal =
            refuse_owner_biometric_enrollment(policy, subject, lease, enrollment, now)) {
        return refusal;
    }
    if (attempt.sensor_id.empty()) {
        return OwnerBiometricRefusal::SensorNotNamed;
    }
    const sentry::policy::IdentityRequest request{
        .subject_id = subject.subject_id,
        .scope = sentry::policy::ConsentScope::BiometricIdentity,
        .location = attempt.sensor_id,
        .sensor_authorized = attempt.sensor_authorized,
        .at = attempt.captured_at,
    };
    if (const auto refusal = sentry::policy::refuse(policy, lease, request)) {
        return map_consent_refusal(*refusal);
    }
    if (attempt.captured_at < enrollment.enrolled_at) {
        return OwnerBiometricRefusal::CapturePrecedesEnrollment;
    }
    if (attempt.captured_at > now) {
        return OwnerBiometricRefusal::CaptureFromFuture;
    }
    if (attempt.captured_at.to(now) > kMaximumOwnerBiometricCaptureAgeMs) {
        return OwnerBiometricRefusal::CaptureStale;
    }
    if (!is_lowercase_sha256(challenge.challenge_sha256) || challenge.issued_at > challenge.expires_at ||
        challenge.issued_at.to(challenge.expires_at) > kMaximumOwnerBiometricChallengeLifetimeMs) {
        return OwnerBiometricRefusal::InvalidChallenge;
    }
    // A challenge proves liveness for the template that was current when it was issued.
    // After enrollment or re-enrollment, issue a new one rather than carrying a prior
    // nonce across the change in biometric authority.
    if (challenge.issued_at < enrollment.enrolled_at) {
        return OwnerBiometricRefusal::ChallengePrecedesEnrollment;
    }
    if (attempt.challenge_sha256 != challenge.challenge_sha256) {
        return OwnerBiometricRefusal::ChallengeMismatch;
    }
    if (attempt.captured_at < challenge.issued_at) {
        return OwnerBiometricRefusal::CapturePrecedesChallenge;
    }
    if (now > challenge.expires_at) {
        return OwnerBiometricRefusal::ChallengeExpired;
    }
    if (attempt.template_sha256 != enrollment.template_sha256) {
        return OwnerBiometricRefusal::TemplateMismatch;
    }
    switch (attempt.result) {
        case OwnerBiometricMatch::MatchedWithLiveness:
            return std::nullopt;
        case OwnerBiometricMatch::NoMatch:
            return OwnerBiometricRefusal::NoMatch;
        case OwnerBiometricMatch::LivenessNotEstablished:
            return OwnerBiometricRefusal::LivenessNotEstablished;
        case OwnerBiometricMatch::MatcherUnavailable:
            return OwnerBiometricRefusal::MatcherUnavailable;
    }
    return OwnerBiometricRefusal::MatcherUnavailable;
}

std::optional<OwnerBiometricRefusal> refuse_owner_biometric_recognition_from_camera(
    const sentry::policy::OperatorPolicy& policy,
    const SubjectRecord& subject,
    const std::optional<sentry::policy::ConsentLease>& lease,
    const OwnerBiometricEnrollment& enrollment,
    const OwnerBiometricChallenge& challenge,
    const OwnerBiometricAttempt& attempt,
    const sentry::camera::CameraAuthority& camera,
    sentry::world::TimePoint now) noexcept {
    // This is the seam between an untrusted worker result and the operator's recorded
    // authority. A worker cannot borrow another camera's grant by relabeling a capture.
    if (camera.camera_id.empty() || camera.camera_id != attempt.sensor_id) {
        return OwnerBiometricRefusal::SensorAuthorityRecordMismatch;
    }
    if (!camera.may_open_stream_at(attempt.captured_at)) {
        return OwnerBiometricRefusal::SensorUnauthorized;
    }

    // The boolean is supplied by the worker and is not itself authority. Once the
    // camera record is verified, the existing policy/consent/freshness gate applies.
    auto verified_attempt = attempt;
    verified_attempt.sensor_authorized = true;
    return refuse_owner_biometric_recognition(policy, subject, lease, enrollment, challenge,
                                              verified_attempt, now);
}

std::optional<AttributeRefusal> refuse_attribute(
    const sentry::policy::OperatorPolicy& policy,
    const SubjectRecord& subject,
    AttributeClass attribute,
    const std::optional<sentry::policy::ConsentLease>& lease,
    sentry::world::TimePoint now) noexcept {
    // A track is a body a sensor saw. Every attribute here names a person in some way,
    // and there is no such thing as a private attribute that is safe to hang on an
    // anonymous observation: the first one hung on it makes it no longer anonymous.
    if (subject.kind == SubjectKind::AnonymousTrack) {
        return AttributeRefusal::AnonymousTrackCannotCarryIdentity;
    }

    // §40, checked before consent. A public figure has not consented to anything — they
    // are in the graph because of what they published, not because they agreed — so
    // "no lease" would be a misleading answer that sends an operator looking for a
    // consent form that should never be sought.
    if (subject.kind == SubjectKind::PublicFigure && is_private(attribute)) {
        return AttributeRefusal::PrivateDataOnPublicFigure;
    }

    const auto scope = required_scope(attribute);
    if (!scope) {
        // Public professional record, or a private attribute on a person Sentry holds by
        // consent. Neither is gated by a scope; the kind check above is the whole rule.
        return std::nullopt;
    }

    // The operator's ceiling first. A node with biometrics disabled says so regardless of
    // what any lease contains, because the fix is a policy change and not a signature.
    if (!policy.permits(*scope)) {
        return AttributeRefusal::ForbiddenByOperatorPolicy;
    }

    // No exemption for the owner. An operator who can bypass their own policy has a
    // preference rather than a policy — and the owner is the person most likely to be
    // holding a biometric template, so it is the last place to make an exception.
    if (!lease) {
        return AttributeRefusal::ScopeNotGranted;
    }
    if (lease->subject_id != subject.subject_id) {
        return AttributeRefusal::LeaseSubjectMismatch;
    }
    if (!lease->active_at(now) || !lease->covers(*scope)) {
        return AttributeRefusal::ScopeNotGranted;
    }
    if (!policy.accepts(lease->evidence)) {
        return AttributeRefusal::EvidenceNotAccepted;
    }

    return std::nullopt;
}

}  // namespace sentry::identity
