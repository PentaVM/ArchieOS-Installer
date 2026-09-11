// Subjects — what kind of person a node in the world model is, and what may be known
// about each kind.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Identity
//
// Sentry deals with people in four different capacities, and the whole safety of the
// identity layer rests on not confusing them:
//
//   Owner            the operator. Consents for themselves.
//   ConsentedPerson  someone with a lease on file.
//   PublicFigure     someone in the public professional record — a reporter, an
//                    official, a company spokesperson. Sentry may *know of* them.
//   AnonymousTrack   a body a sensor saw. Counted, avoided, never named.
//
// The rule that makes this worth having as a type rather than a comment: knowing who a
// journalist is does not make them resolvable from a camera. `PublicFigure` and
// `ConsentedPerson` are different capacities, and a system that lets the first quietly
// satisfy a check meant for the second has built face recognition on a byline database.
//
// So resolvability is a property of the *kind*, checked before any evidence is weighed,
// and a `PublicFigure` fails it no matter how good the appearance match is.

#pragma once

#include <optional>
#include <string>
#include <string_view>

#include "sentry/policy/consent.hpp"

namespace sentry::camera {
struct CameraAuthority;
}

namespace sentry::identity {

using sentry::policy::ConsentScope;

// The capacity in which Sentry holds a person.
enum class SubjectKind {
    // The operator of this node. Consents for themselves; still bound by their own
    // policy, because a policy that the operator can bypass is a preference.
    Owner,
    // A person with a consent lease on file.
    ConsentedPerson,
    // A person in the public professional record. Known of, never resolved.
    PublicFigure,
    // A sensor observation with no identity attached, and none permitted.
    AnonymousTrack,
};

[[nodiscard]] std::string_view name(SubjectKind kind) noexcept;

// Whether a sensor observation may ever be resolved to this kind of person.
//
// Checked before evidence, deliberately. The alternative — weighing appearance first and
// consulting the kind afterwards — puts a face model's confidence in the position of
// deciding whether the rule applies, and a model that is confidently wrong about twins
// and uniforms is the last thing that should hold that position.
//
// `PublicFigure` is false. Being in the public record is not consent to be recognised
// walking down a street, and the two are conflated so easily that the type refuses it
// rather than leaving the distinction to a comment.
[[nodiscard]] constexpr bool resolvable(SubjectKind kind) noexcept {
    return kind == SubjectKind::Owner || kind == SubjectKind::ConsentedPerson;
}

// A class of thing that might be recorded about a person.
//
// Split by whether it belongs to someone's public professional life or to their private
// one, because RFC-008 §40 draws exactly that line and a free-form attribute bag would
// erase it on the first convenient field.
enum class AttributeClass {
    // --- Public professional record. Permitted for a PublicFigure.
    LegalName,
    PublicRole,
    PublicAffiliation,
    // A byline, a filed report, an authored paper.
    PublicByline,
    // A verified account the person publishes under.
    PublicAccount,
    // Something they said publicly, on the record.
    PublicStatement,

    // --- Private. Never permitted for a PublicFigure, and gated for everyone else.
    HomeAddress,
    PersonalContact,
    // Who someone's family is. Explicitly named in §40's prohibition.
    FamilyMembers,
    PrivateTravel,
    HealthStatus,
    // A face or gait template. The most restricted thing here.
    BiometricTemplate,
    // A phone, tag or radio that identifies a person by proxy.
    DeviceIdentifier,
    // Where they are right now, precisely.
    PreciseLiveLocation,
};

[[nodiscard]] std::string_view name(AttributeClass attribute) noexcept;

// Whether an attribute is part of someone's private life rather than their public
// professional record.
[[nodiscard]] constexpr bool is_private(AttributeClass attribute) noexcept {
    switch (attribute) {
        case AttributeClass::LegalName:
        case AttributeClass::PublicRole:
        case AttributeClass::PublicAffiliation:
        case AttributeClass::PublicByline:
        case AttributeClass::PublicAccount:
        case AttributeClass::PublicStatement:
            return false;
        case AttributeClass::HomeAddress:
        case AttributeClass::PersonalContact:
        case AttributeClass::FamilyMembers:
        case AttributeClass::PrivateTravel:
        case AttributeClass::HealthStatus:
        case AttributeClass::BiometricTemplate:
        case AttributeClass::DeviceIdentifier:
        case AttributeClass::PreciseLiveLocation:
            return true;
    }
    return true;  // An attribute added later is private until someone says otherwise.
}

// The consent scope an attribute needs, when it needs one.
//
// Absent means the attribute is not gated by a lease — either because it is public
// professional record, or because it is refused outright for the kind in question and no
// lease is the relevant question.
[[nodiscard]] constexpr std::optional<ConsentScope> required_scope(
    AttributeClass attribute) noexcept {
    switch (attribute) {
        case AttributeClass::BiometricTemplate:
            return ConsentScope::BiometricIdentity;
        case AttributeClass::DeviceIdentifier:
            return ConsentScope::Proximity;
        case AttributeClass::PreciseLiveLocation:
            return ConsentScope::GpsLocation;
        case AttributeClass::LegalName:
        case AttributeClass::PublicRole:
        case AttributeClass::PublicAffiliation:
        case AttributeClass::PublicByline:
        case AttributeClass::PublicAccount:
        case AttributeClass::PublicStatement:
        case AttributeClass::HomeAddress:
        case AttributeClass::PersonalContact:
        case AttributeClass::FamilyMembers:
        case AttributeClass::PrivateTravel:
        case AttributeClass::HealthStatus:
            return std::nullopt;
    }
    return std::nullopt;
}

// Why storing an attribute was refused.
enum class AttributeRefusal {
    // §40: the media and public-figure graph is public professional data only. Private
    // addresses, phone numbers, family details and travel are the named examples.
    PrivateDataOnPublicFigure,
    // An anonymous track has no identity to attach anything to. Attaching one is how a
    // track stops being anonymous.
    AnonymousTrackCannotCarryIdentity,
    // The attribute needs a consent scope this subject has not granted.
    ScopeNotGranted,
    // The supplied lease names a different person. A consent lease is evidence about
    // precisely one subject, never a transferable capability.
    LeaseSubjectMismatch,
    // The operator's policy forbids this class of data on this node entirely.
    ForbiddenByOperatorPolicy,
    // A lease exists and covers the scope, but rests on evidence this operator does not
    // accept — typically an attestation where a signature is required. Kept apart from
    // `ScopeNotGranted` because the fix is a signature, not a conversation.
    EvidenceNotAccepted,
};

[[nodiscard]] std::string_view describe(AttributeRefusal refusal) noexcept;

// One person, as Sentry holds them.
//
// Deliberately thin. This is the node in the relationship graph and the anchor for
// consent, not a dossier — the attributes live behind `refuse_attribute` rather than in
// a bag of strings that nothing checks.
struct SubjectRecord {
    std::string subject_id{};
    SubjectKind kind = SubjectKind::AnonymousTrack;
    // How this person is referred to. For an `AnonymousTrack` this is a track label, not
    // a name, and the type is what says so.
    std::string display_label{};

    [[nodiscard]] bool is_resolvable() const noexcept { return resolvable(kind); }
};

// ---------------------------------------------------------------- owner biometric gate
//
// This is deliberately narrower than a general face-recognition service. It describes
// enrolment and a live recognition decision for the node owner only. No pixels,
// embeddings, model score, camera URL or credentials cross this boundary: a future
// isolated biometric worker keeps those and reports a finite result. That means this
// contract can prove who may use a result without quietly becoming a camera client.

// An opaque SHA-256 reference to an owner template held by a future dedicated secure
// store. It is a change detector and audit handle, not the template itself and not a
// reversible representation of a face.
struct OwnerBiometricEnrollment {
    std::string owner_id{};
    std::string template_sha256{};
    sentry::world::TimePoint enrolled_at{};
};

// An opaque, short-lived challenge issued by the authority service. The worker receives
// the nonce through its own secure channel; this contract holds only the SHA-256 binding
// so a stored video/result cannot be replayed as present-owner evidence later.
struct OwnerBiometricChallenge {
    std::string challenge_sha256{};
    sentry::world::TimePoint issued_at{};
    sentry::world::TimePoint expires_at{};
};

// The only answers the isolated biometric worker may return to this authority layer.
// There is intentionally no numerical similarity score here: threshold calibration is
// owned by the model worker and must never be mistaken for consent or authorization.
enum class OwnerBiometricMatch {
    MatchedWithLiveness,
    NoMatch,
    LivenessNotEstablished,
    MatcherUnavailable,
};

struct OwnerBiometricAttempt {
    std::string template_sha256{};
    std::string challenge_sha256{};
    std::string sensor_id{};
    bool sensor_authorized = false;
    OwnerBiometricMatch result = OwnerBiometricMatch::MatcherUnavailable;
    sentry::world::TimePoint captured_at{};
};

// Why an owner biometric operation was refused. Each case tells the caller whether the
// correction belongs in the operator policy, consent store, camera authorization,
// enrollment flow, or biometric worker.
enum class OwnerBiometricRefusal {
    SubjectIsNotOwner,
    SubjectIdentifierMismatch,
    InvalidEnrollment,
    BiometricPolicyDisabled,
    BiometricConsentNotGranted,
    BiometricConsentExpired,
    BiometricConsentRevoked,
    BiometricEvidenceNotAccepted,
    SensorUnauthorized,
    SensorOutsideConsentScope,
    SensorNotNamed,
    SensorAuthorityRecordMismatch,
    CapturePrecedesEnrollment,
    CaptureFromFuture,
    CaptureStale,
    InvalidChallenge,
    ChallengePrecedesEnrollment,
    ChallengeMismatch,
    CapturePrecedesChallenge,
    ChallengeExpired,
    TemplateMismatch,
    NoMatch,
    LivenessNotEstablished,
    MatcherUnavailable,
};

[[nodiscard]] std::string_view name(OwnerBiometricMatch match) noexcept;
[[nodiscard]] std::string_view describe(OwnerBiometricRefusal refusal) noexcept;

// Validate storing a first owner template. The caller must supply the owner's current
// self-consent lease; enrollment does not gain an exception from the owner's role.
[[nodiscard]] std::optional<OwnerBiometricRefusal> refuse_owner_biometric_enrollment(
    const sentry::policy::OperatorPolicy& policy,
    const SubjectRecord& subject,
    const std::optional<sentry::policy::ConsentLease>& lease,
    const OwnerBiometricEnrollment& enrollment,
    sentry::world::TimePoint now) noexcept;

// Validate a fresh result from a future isolated biometric worker. This function never
// opens a sensor, stores a template or decides a match; it validates the result's
// identity, liveness, authority, consent and freshness before another component may use
// it as an owner-presence signal.
[[nodiscard]] std::optional<OwnerBiometricRefusal> refuse_owner_biometric_recognition(
    const sentry::policy::OperatorPolicy& policy,
    const SubjectRecord& subject,
    const std::optional<sentry::policy::ConsentLease>& lease,
    const OwnerBiometricEnrollment& enrollment,
    const OwnerBiometricChallenge& challenge,
    const OwnerBiometricAttempt& attempt,
    sentry::world::TimePoint now) noexcept;

// A stricter camera-backed entry point. It derives sensor authorization from the
// operator-owned camera record rather than trusting the boolean carried by an
// untrusted worker result. The camera id must match and the grant must predate capture.
// This contract does not open a stream.
[[nodiscard]] std::optional<OwnerBiometricRefusal>
refuse_owner_biometric_recognition_from_camera(
    const sentry::policy::OperatorPolicy& policy,
    const SubjectRecord& subject,
    const std::optional<sentry::policy::ConsentLease>& lease,
    const OwnerBiometricEnrollment& enrollment,
    const OwnerBiometricChallenge& challenge,
    const OwnerBiometricAttempt& attempt,
    const sentry::camera::CameraAuthority& camera,
    sentry::world::TimePoint now) noexcept;

// Whether an attribute may be recorded against a subject. Returns nothing when it may.
//
// `granted` is the set membership test for the subject's lease, passed in rather than
// looked up: this header holds no store and reaches across to nothing, so the consent
// module stays the single place a lease is interpreted.
[[nodiscard]] std::optional<AttributeRefusal> refuse_attribute(
    const sentry::policy::OperatorPolicy& policy,
    const SubjectRecord& subject,
    AttributeClass attribute,
    const std::optional<sentry::policy::ConsentLease>& lease,
    sentry::world::TimePoint now) noexcept;

}  // namespace sentry::identity
