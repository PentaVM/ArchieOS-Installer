// The owner's authority over their own node.
//
// The defaults elsewhere describe what the automated pipeline may do unattended. These
// cases describe what the owner may do, and the answer is: everything that is a permission
// question, on the record. A system whose operator cannot overrule it has decided it knows
// better than the person who runs it, which is the wrong shape for a machine that lives in
// someone's house.
//
// The end-to-end cases are the ones that matter most here. It is easy to write a
// permissive-looking `can_override` and still leave the owner blocked three checks later,
// so several of these drive a real operation from refusal through override to success.

#include "harness.hpp"

#include "sentry/identity/override.hpp"
#include "sentry/identity/retention.hpp"

using namespace sentry::identity;
using sentry::policy::attested_by_owner;
using sentry::policy::can_override;
using sentry::policy::ConsentRefusal;
using sentry::policy::ConsentScope;
using sentry::policy::IdentityRequest;
using sentry::policy::OperatorPolicy;
using sentry::policy::OverrideVerdict;
using sentry::policy::OwnerOverride;
using sentry::policy::refuse;
using sentry::policy::unrestricted_policy;
using sentry::world::TimePoint;

namespace {

OwnerOverride nelson() {
    return OwnerOverride{"nelson", "my node, my house, I asked them in person",
                         TimePoint{1000}};
}

SubjectRecord person(std::string id, SubjectKind kind) {
    SubjectRecord s;
    s.subject_id = std::move(id);
    s.kind = kind;
    s.display_label = s.subject_id;
    return s;
}

RelationEdge edge(std::string from, std::string to, RelationKind kind, EdgeBasis basis) {
    RelationEdge e;
    e.from_id = std::move(from);
    e.to_id = std::move(to);
    e.kind = kind;
    e.basis = basis;
    return e;
}

// ---------------------------------------------------------------- the sweep

void every_permission_refusal_yields_to_the_owner() {
    CASE("the owner can lift every consent refusal that is about permission");

    const auto owner = nelson();
    for (const auto refusal :
         {ConsentRefusal::NoLease, ConsentRefusal::ScopeNotGranted, ConsentRefusal::Expired,
          ConsentRefusal::Revoked, ConsentRefusal::ForbiddenByOperatorPolicy,
          ConsentRefusal::EvidenceNotAccepted, ConsentRefusal::OutsideSpatialScope}) {
        CHECK_BECAUSE(can_override(refusal, owner) == OverrideVerdict::LiftedOnTheRecord,
                      "permission on this node is the owner's to give, and a refusal they "
                      "cannot argue with is a system overruling its operator");
    }

    // The exception is factual rather than permissive: instructing the system to proceed
    // does not make an unapproved camera produce a frame.
    CHECK_EQ(can_override(ConsentRefusal::SensorNotAuthorized, owner),
             OverrideVerdict::NotAPermissionQuestion);
    CHECK_EQ(can_override(ConsentRefusal::LeaseSubjectMismatch, owner),
             OverrideVerdict::NotAPermissionQuestion);
    CHECK_EQ(can_override(AttributeRefusal::LeaseSubjectMismatch, owner),
             OverrideVerdict::NotAPermissionQuestion);
}

void an_override_needs_a_reason() {
    CASE("an unstated override is not an override");

    // Not bureaucracy. Six months later nobody can tell an override from a bug, and the
    // most common reader of this field is the person who wrote it.
    const OwnerOverride nameless{"", "because", TimePoint{1}};
    const OwnerOverride silent{"nelson", "", TimePoint{1}};

    CHECK_EQ(can_override(ConsentRefusal::NoLease, nameless),
             OverrideVerdict::OverrideNotStated);
    CHECK_EQ(can_override(ConsentRefusal::NoLease, silent), OverrideVerdict::OverrideNotStated);
    CHECK_EQ(can_override(EdgeRefusal::InferredFromCoPresence, silent),
             OverrideVerdict::OverrideNotStated);
    CHECK_EQ(can_override(AttributeRefusal::ScopeNotGranted, silent),
             OverrideVerdict::OverrideNotStated);
}

// ---------------------------------------------------------------- end to end

void the_owner_can_unlock_biometric_identification_completely() {
    CASE("refused by default, permitted once the owner says so — end to end");

    const auto owner = nelson();

    // Default node: biometrics off, only signatures accepted. Everything is refused.
    OperatorPolicy strict;
    strict.accepted_evidence = {sentry::policy::ConsentEvidence::SubjectSigned};

    IdentityRequest request;
    request.subject_id = "ana";
    request.scope = ConsentScope::BiometricIdentity;
    request.sensor_authorized = true;
    request.at = TimePoint{2000};

    CHECK_EQ(refuse(strict, std::nullopt, request),
             std::optional{ConsentRefusal::ForbiddenByOperatorPolicy});

    // The owner turns their node on and attests to the permission they obtained. Nothing
    // exotic: `unrestricted_policy` and `attested_by_owner` are the two named pieces.
    const auto policy = unrestricted_policy();
    const auto lease = attested_by_owner(owner, "ana", {ConsentScope::BiometricIdentity});

    CHECK_BECAUSE(!refuse(policy, lease, request).has_value(),
                  "with the owner's policy and the owner's attestation the operation goes "
                  "through; the contracts gate the pipeline, not the person who runs it");

    // And the same thing at the attribute layer, which is the one that actually stores
    // a face template.
    const auto ana = person("ana", SubjectKind::ConsentedPerson);
    CHECK(!refuse_attribute(policy, ana, AttributeClass::BiometricTemplate, lease,
                            TimePoint{2000}));
}

void the_owner_can_record_family_they_know_about() {
    CASE("a personal tie the owner knows of, on their own say-so");

    const auto owner = nelson();
    const auto x = person("x", SubjectKind::PublicFigure);
    const auto y = person("y", SubjectKind::PublicFigure);

    // The pipeline may not do this: §40 is about what gets harvested at volume.
    const auto refusal =
        refuse_edge(edge("x", "y", RelationKind::Family, EdgeBasis::PublicRecord), x, y, "");
    CHECK_EQ(refusal, std::optional{EdgeRefusal::PersonalTieFromPublicRecord});

    // The owner reading an obituary and recording that two people are siblings is doing
    // research, and refusing them that would make the graph useless for its purpose.
    CHECK_EQ(can_override(*refusal, owner), OverrideVerdict::LiftedOnTheRecord);
}

void co_presence_is_refused_to_the_pipeline_and_offered_to_the_owner() {
    CASE("the difference is who is looking, and how many times");

    const auto owner = nelson();
    const auto a = person("ana", SubjectKind::ConsentedPerson);
    const auto b = person("beto", SubjectKind::ConsentedPerson);

    // A job walking a year of camera history and emitting a social graph: still refused,
    // because `refuse_edge` is what that job calls and it takes no override.
    const auto refusal = refuse_edge(
        edge("ana", "beto", RelationKind::Colleague, EdgeBasis::CoPresenceInference), a, b,
        "nelson");
    CHECK_EQ(refusal, std::optional{EdgeRefusal::InferredFromCoPresence});

    // The owner looking at one pair and deciding: available, and recorded as their call
    // rather than as evidence of a relationship.
    CHECK_BECAUSE(can_override(*refusal, owner) == OverrideVerdict::LiftedOnTheRecord,
                  "the dangerous version is unattended and at volume; a person agreeing "
                  "with one observation is a different act with a different failure mode");
}

// ---------------------------------------------------------------- the one limit

void a_guess_cannot_be_stamped_as_something_archie_verified() {
    CASE("the one refusal that stands against the owner, and stands for them");

    const auto owner = nelson();

    CHECK_BECAUSE(can_override(EdgeRefusal::GenerativeCannotAssert, owner) ==
                      OverrideVerdict::WouldCorruptWhatArchieKnows,
                  "an owner who cannot tell what Archie observed from what it inferred has "
                  "lost the ability to supervise it, which is what the authority is for");
}

void agreeing_with_a_guess_is_promotion_not_override() {
    CASE("the owner is never blocked — the claim is re-attributed instead");

    const auto owner = nelson();
    const auto a = person("ana", SubjectKind::ConsentedPerson);
    const auto b = person("beto", SubjectKind::ConsentedPerson);

    auto hypothesis =
        edge("ana", "beto", RelationKind::Colleague, EdgeBasis::ModelHypothesis);
    hypothesis.source_note = "both cited in the same filing";

    // As a model's assertion: refused.
    CHECK_EQ(refuse_edge(hypothesis, a, b, "nelson"),
             std::optional{EdgeRefusal::GenerativeCannotAssert});

    // As the owner's declaration: accepted. Same content, honest attribution.
    const auto declared = promote(hypothesis, owner);
    CHECK_EQ(declared.basis, EdgeBasis::OwnerDeclared);
    CHECK_BECAUSE(!refuse_edge(declared, a, b, "nelson").has_value(),
                  "there is no dead end here: the owner agrees, and the claim now rests on "
                  "a person's judgement rather than a model's confidence");

    // Both facts survive in the note. Either alone is less informative than the pair.
    const auto& note = declared.source_note;
    CHECK(note.find("model_hypothesis") != std::string::npos);
    CHECK(note.find("nelson") != std::string::npos);
    CHECK_BECAUSE(note.find("both cited in the same filing") != std::string::npos,
                  "the model's original reasoning is not thrown away when a person "
                  "accepts it");
}

void promotion_does_not_exempt_a_claim_from_the_rules() {
    CASE("who is speaking changes; what may be said does not");

    const auto owner = nelson();
    const auto a = person("ana", SubjectKind::ConsentedPerson);
    const auto track = person("track-9", SubjectKind::AnonymousTrack);

    // Promoting an edge to an anonymous track still fails, because the problem was never
    // who was asserting it.
    const auto hypothesis =
        edge("ana", "track-9", RelationKind::Colleague, EdgeBasis::ModelHypothesis);
    const auto declared = promote(hypothesis, owner);

    CHECK_EQ(refuse_edge(declared, a, track, "nelson"),
             std::optional{EdgeRefusal::EndpointNotIdentifiable});
}

void overriding_cannot_conjure_an_identity_that_does_not_exist() {
    CASE("a track with no identity is a missing fact, not a withheld permission");

    const auto owner = nelson();

    // Saying yes here would write "track-17 is Ana's colleague" while track-17 still
    // refers to nobody. The fix is to identify the track, which the owner may also do.
    CHECK_EQ(can_override(EdgeRefusal::EndpointNotIdentifiable, owner),
             OverrideVerdict::NotAPermissionQuestion);
    CHECK_EQ(can_override(AttributeRefusal::AnonymousTrackCannotCarryIdentity, owner),
             OverrideVerdict::NotAPermissionQuestion);
    CHECK_EQ(can_override(EdgeRefusal::SelfEdge, owner),
             OverrideVerdict::NotAPermissionQuestion);
}

// ---------------------------------------------------------------- the lease itself

void an_owner_attested_lease_is_an_ordinary_lease() {
    CASE("attestation is recorded as attestation, not disguised as a signature");

    const auto lease = attested_by_owner(nelson(), "ana", {ConsentScope::CameraIdentity});

    CHECK_EQ(lease.evidence, sentry::policy::ConsentEvidence::OperatorAttested);
    CHECK(lease.covers(ConsentScope::CameraIdentity));
    CHECK(!lease.covers(ConsentScope::BiometricIdentity));
    CHECK(lease.active_at(TimePoint{5000}));

    // Standing until withdrawn: an owner attesting to permission obtained in person has
    // no natural expiry to offer, and inventing one produces a lapse nobody chose.
    CHECK(!lease.valid_until.has_value());

    // A node that does not accept attestation still does not accept this one. The
    // override is explicit, not a disguise.
    OperatorPolicy signatures_only;
    signatures_only.biometrics_enabled = true;
    signatures_only.accepted_evidence = {sentry::policy::ConsentEvidence::SubjectSigned};

    IdentityRequest request;
    request.subject_id = "ana";
    request.scope = ConsentScope::CameraIdentity;
    request.sensor_authorized = true;
    request.at = TimePoint{5000};

    CHECK_BECAUSE(refuse(signatures_only, lease, request) ==
                      ConsentRefusal::EvidenceNotAccepted,
                  "the owner's attestation is visible as an attestation, so a policy that "
                  "requires signatures can still tell the difference");
}

void the_unrestricted_policy_really_is_unrestricted() {
    CASE("every dial on, in one recognised configuration");

    const auto policy = unrestricted_policy();
    for (const auto scope :
         {ConsentScope::GpsLocation, ConsentScope::CameraIdentity,
          ConsentScope::BiometricIdentity, ConsentScope::Proximity,
          ConsentScope::HistoricalReplay, ConsentScope::CloudProcessing}) {
        CHECK(policy.permits(scope));
    }
    for (const auto evidence :
         {sentry::policy::ConsentEvidence::SubjectSigned,
          sentry::policy::ConsentEvidence::SelfConsent,
          sentry::policy::ConsentEvidence::OperatorAttested,
          sentry::policy::ConsentEvidence::ImportedAuthorization}) {
        CHECK(policy.accepts(evidence));
    }
}

}  // namespace

int main() {
    every_permission_refusal_yields_to_the_owner();
    an_override_needs_a_reason();

    the_owner_can_unlock_biometric_identification_completely();
    the_owner_can_record_family_they_know_about();
    co_presence_is_refused_to_the_pipeline_and_offered_to_the_owner();

    a_guess_cannot_be_stamped_as_something_archie_verified();
    agreeing_with_a_guess_is_promotion_not_override();
    promotion_does_not_exempt_a_claim_from_the_rules();
    overriding_cannot_conjure_an_identity_that_does_not_exist();

    an_owner_attested_lease_is_an_ordinary_lease();
    the_unrestricted_policy_really_is_unrestricted();

    return sentry::test::report();
}
