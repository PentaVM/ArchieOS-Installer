// The rules that stop the relationship graph from becoming a surveillance product.
//
// Every case here corresponds to a prohibition in RFC-008: identity resolution is
// consent-gated, the public-figure record is public professional data only, and a model's
// output is never world state. They are tests rather than comments because the graph will
// be built by several hands, and a rule nothing checks is a rule that survives exactly as
// long as everyone remembers it.

#include "harness.hpp"

#include "sentry/camera/authority.hpp"
#include "sentry/identity/relationship.hpp"
#include "sentry/identity/subject.hpp"

using namespace sentry::identity;
using sentry::policy::ConsentEvidence;
using sentry::policy::ConsentLease;
using sentry::policy::ConsentScope;
using sentry::policy::OperatorPolicy;
using sentry::world::TimePoint;

namespace {

TimePoint at(std::int64_t ms) { return TimePoint{ms}; }

SubjectRecord person(std::string id, SubjectKind kind) {
    SubjectRecord s;
    s.subject_id = std::move(id);
    s.kind = kind;
    s.display_label = s.subject_id;
    return s;
}

OperatorPolicy permissive_policy() {
    OperatorPolicy p;
    p.biometrics_enabled = true;
    p.cloud_export_enabled = false;
    p.historical_replay_enabled = true;
    p.accepted_evidence = {ConsentEvidence::SubjectSigned, ConsentEvidence::SelfConsent};
    return p;
}

ConsentLease lease_for(std::string id, std::vector<ConsentScope> scopes,
                       ConsentEvidence evidence = ConsentEvidence::SubjectSigned) {
    ConsentLease l;
    l.subject_id = std::move(id);
    l.scopes = std::move(scopes);
    l.valid_from = at(0);
    l.evidence = evidence;
    return l;
}

RelationEdge edge(std::string from, std::string to, RelationKind kind, EdgeBasis basis) {
    RelationEdge e;
    e.from_id = std::move(from);
    e.to_id = std::move(to);
    e.kind = kind;
    e.basis = basis;
    return e;
}

// ---------------------------------------------------------------- resolvability

void being_in_the_news_is_not_consent_to_be_recognised() {
    CASE("a public figure is never resolvable from a sensor");

    // The conjunction this prevents: Sentry knows who a reporter is because of their
    // byline, a camera sees someone who looks like them, and the two facts meet. A byline
    // database plus a face model is face recognition, assembled out of two things that
    // each looked harmless.
    CHECK_BECAUSE(!resolvable(SubjectKind::PublicFigure),
                  "knowing of someone from the public record is not permission to "
                  "recognise them walking past a camera");
    CHECK(!resolvable(SubjectKind::AnonymousTrack));
    CHECK(resolvable(SubjectKind::Owner));
    CHECK(resolvable(SubjectKind::ConsentedPerson));
}

// ---------------------------------------------------------------- §40

void the_public_figure_record_holds_public_professional_data_only() {
    CASE("private data is refused on a public figure");

    const auto policy = permissive_policy();
    const auto reporter = person("reporter", SubjectKind::PublicFigure);

    // RFC-008 §40, named example by named example.
    for (const auto attribute : {AttributeClass::HomeAddress, AttributeClass::PersonalContact,
                                 AttributeClass::FamilyMembers, AttributeClass::PrivateTravel}) {
        const auto refusal = refuse_attribute(policy, reporter, attribute, std::nullopt, at(1));
        CHECK_BECAUSE(refusal == AttributeRefusal::PrivateDataOnPublicFigure,
                      "collecting home addresses, contacts, family details or private "
                      "travel about someone because they are in the news is the "
                      "definition of the thing §40 forbids");
    }

    // And the refusal does not soften when a lease appears. There is nobody to sign it:
    // a public figure is in the graph because of what they published.
    const auto with_lease = refuse_attribute(
        policy, reporter, AttributeClass::HomeAddress,
        lease_for("reporter", {ConsentScope::GpsLocation}), at(1));
    CHECK_BECAUSE(with_lease == AttributeRefusal::PrivateDataOnPublicFigure,
                  "a lease cannot promote a public figure into a consented subject");

    // What is permitted stays permitted. This is a research tool, not a wall.
    for (const auto attribute : {AttributeClass::PublicByline, AttributeClass::PublicRole,
                                 AttributeClass::PublicAffiliation,
                                 AttributeClass::PublicStatement}) {
        CHECK(!refuse_attribute(policy, reporter, attribute, std::nullopt, at(1)));
    }
}

void an_anonymous_track_cannot_acquire_an_identity() {
    CASE("nothing may be attached to an anonymous track");

    const auto policy = permissive_policy();
    const auto track = person("track-17", SubjectKind::AnonymousTrack);

    // Even a legal name, which is public professional data on anyone else. The first
    // attribute hung on a track is the moment it stops being anonymous, so there is no
    // safe one to allow.
    CHECK_EQ(refuse_attribute(policy, track, AttributeClass::LegalName, std::nullopt, at(1)),
             std::optional{AttributeRefusal::AnonymousTrackCannotCarryIdentity});
    CHECK_EQ(refuse_attribute(policy, track, AttributeClass::BiometricTemplate,
                              lease_for("track-17", {ConsentScope::BiometricIdentity}), at(1)),
             std::optional{AttributeRefusal::AnonymousTrackCannotCarryIdentity});
}

void the_operator_is_bound_by_their_own_policy() {
    CASE("the owner gets no exemption from the policy they set");

    OperatorPolicy strict = permissive_policy();
    strict.biometrics_enabled = false;

    const auto owner = person("nelson", SubjectKind::Owner);
    const auto self_consent =
        lease_for("nelson", {ConsentScope::BiometricIdentity}, ConsentEvidence::SelfConsent);

    CHECK_BECAUSE(refuse_attribute(strict, owner, AttributeClass::BiometricTemplate,
                                   self_consent, at(1)) ==
                      AttributeRefusal::ForbiddenByOperatorPolicy,
                  "a policy the operator can step around is a preference, and the owner "
                  "is the person most likely to hold a biometric template");

    // With biometrics enabled and self-consent on file, it goes through.
    CHECK(!refuse_attribute(permissive_policy(), owner, AttributeClass::BiometricTemplate,
                            self_consent, at(1)));

    // Without a lease it does not, owner or otherwise.
    CHECK_EQ(refuse_attribute(permissive_policy(), owner, AttributeClass::BiometricTemplate,
                              std::nullopt, at(1)),
             std::optional{AttributeRefusal::ScopeNotGranted});

    CHECK_EQ(refuse_attribute(permissive_policy(), owner, AttributeClass::BiometricTemplate,
                              lease_for("ana", {ConsentScope::BiometricIdentity}), at(1)),
             std::optional{AttributeRefusal::LeaseSubjectMismatch});
}

void a_revoked_lease_stops_working_immediately() {
    CASE("withdrawing consent withdraws it");

    auto revoked = lease_for("ana", {ConsentScope::BiometricIdentity});
    revoked.revocation = sentry::policy::RevocationState::Revoked;

    const auto ana = person("ana", SubjectKind::ConsentedPerson);
    CHECK_EQ(refuse_attribute(permissive_policy(), ana, AttributeClass::BiometricTemplate,
                              revoked, at(1)),
             std::optional{AttributeRefusal::ScopeNotGranted});
}

void an_attestation_is_not_a_signature() {
    CASE("evidence the operator does not accept is reported as such");

    // A different fix from "no lease": someone needs to sign, not to be asked.
    const auto attested =
        lease_for("ana", {ConsentScope::BiometricIdentity}, ConsentEvidence::OperatorAttested);
    const auto ana = person("ana", SubjectKind::ConsentedPerson);

    CHECK_EQ(refuse_attribute(permissive_policy(), ana, AttributeClass::BiometricTemplate,
                              attested, at(1)),
             std::optional{AttributeRefusal::EvidenceNotAccepted});
}

// ---------------------------------------------------------------- owner biometric gate

OwnerBiometricEnrollment owner_enrollment() {
    OwnerBiometricEnrollment enrollment;
    enrollment.owner_id = "nelson";
    enrollment.template_sha256 =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    enrollment.enrolled_at = at(1'000);
    return enrollment;
}

OwnerBiometricAttempt live_owner_attempt() {
    OwnerBiometricAttempt attempt;
    attempt.template_sha256 = owner_enrollment().template_sha256;
    attempt.challenge_sha256 =
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    attempt.sensor_id = "hallway-camera";
    attempt.sensor_authorized = true;
    attempt.result = OwnerBiometricMatch::MatchedWithLiveness;
    attempt.captured_at = at(2'000);
    return attempt;
}

OwnerBiometricChallenge owner_challenge() {
    OwnerBiometricChallenge challenge;
    challenge.challenge_sha256 = live_owner_attempt().challenge_sha256;
    challenge.issued_at = at(1'990);
    challenge.expires_at = at(2'020);
    return challenge;
}

sentry::camera::CameraAuthority owner_camera() {
    sentry::camera::CameraAuthority camera;
    camera.camera_id = "hallway-camera";
    camera.basis = sentry::camera::AuthorityBasis::ArchieOwned;
    camera.desired = sentry::camera::DesiredConnectionState::Connected;
    camera.granted_at = at(1'980);
    camera.evidence = "owner enrollment camera";
    return camera;
}

void the_owner_can_enroll_and_prove_live_presence_under_current_authority() {
    CASE("initial owner biometric recognition is an explicit, local-only contract");

    const auto owner = person("nelson", SubjectKind::Owner);
    const auto lease = lease_for("nelson", {ConsentScope::BiometricIdentity},
                                 ConsentEvidence::SelfConsent);
    const auto enrollment = owner_enrollment();
    CHECK(!refuse_owner_biometric_enrollment(permissive_policy(), owner, lease, enrollment,
                                              at(1'000)));
    CHECK(!refuse_owner_biometric_recognition(permissive_policy(), owner, lease, enrollment,
                                               owner_challenge(), live_owner_attempt(), at(2'010)));

    auto worker_untrusted = live_owner_attempt();
    worker_untrusted.sensor_authorized = false;
    CHECK(!refuse_owner_biometric_recognition_from_camera(
        permissive_policy(), owner, lease, enrollment, owner_challenge(), worker_untrusted,
        owner_camera(), at(2'010)));

    auto another_owner_enrollment = enrollment;
    another_owner_enrollment.owner_id = "ana";
    CHECK_EQ(refuse_owner_biometric_enrollment(permissive_policy(), owner, lease,
                                               another_owner_enrollment, at(1'000)),
             std::optional{OwnerBiometricRefusal::SubjectIdentifierMismatch});
}

void owner_biometric_recognition_binds_the_result_to_a_real_camera_grant() {
    CASE("camera-backed owner recognition cannot borrow another sensor's authority");

    const auto owner = person("nelson", SubjectKind::Owner);
    const auto lease = lease_for("nelson", {ConsentScope::BiometricIdentity},
                                 ConsentEvidence::SelfConsent);
    const auto enrollment = owner_enrollment();

    auto mismatched_camera = owner_camera();
    mismatched_camera.camera_id = "garage-camera";
    CHECK_EQ(refuse_owner_biometric_recognition_from_camera(
                 permissive_policy(), owner, lease, enrollment, owner_challenge(),
                 live_owner_attempt(), mismatched_camera, at(2'010)),
             std::optional{OwnerBiometricRefusal::SensorAuthorityRecordMismatch});

    auto after_the_capture = owner_camera();
    after_the_capture.granted_at = at(2'001);
    CHECK_EQ(refuse_owner_biometric_recognition_from_camera(
                 permissive_policy(), owner, lease, enrollment, owner_challenge(),
                 live_owner_attempt(), after_the_capture, at(2'010)),
             std::optional{OwnerBiometricRefusal::SensorUnauthorized});
}

void owner_biometric_recognition_never_becomes_general_face_recognition() {
    CASE("only the owner can use this first biometric gate");

    const auto other = person("ana", SubjectKind::ConsentedPerson);
    const auto lease = lease_for("ana", {ConsentScope::BiometricIdentity});
    auto enrollment = owner_enrollment();
    enrollment.owner_id = "ana";
    CHECK_EQ(refuse_owner_biometric_enrollment(permissive_policy(), other, lease, enrollment,
                                               at(1'000)),
             std::optional{OwnerBiometricRefusal::SubjectIsNotOwner});
}

void owner_biometric_recognition_requires_current_consent_sensor_liveness_and_freshness() {
    CASE("a biometric match cannot paper over a missing authority or stale capture");

    const auto owner = person("nelson", SubjectKind::Owner);
    const auto enrollment = owner_enrollment();
    const auto self_consent = lease_for("nelson", {ConsentScope::BiometricIdentity},
                                        ConsentEvidence::SelfConsent);
    auto strict = permissive_policy();
    strict.biometrics_enabled = false;
    CHECK_EQ(refuse_owner_biometric_recognition(strict, owner, self_consent, enrollment,
                                                owner_challenge(), live_owner_attempt(), at(2'010)),
             std::optional{OwnerBiometricRefusal::BiometricPolicyDisabled});

    auto revoked = self_consent;
    revoked.revocation = sentry::policy::RevocationState::Revoked;
    CHECK_EQ(refuse_owner_biometric_recognition(permissive_policy(), owner, revoked, enrollment,
                                                owner_challenge(), live_owner_attempt(), at(2'010)),
             std::optional{OwnerBiometricRefusal::BiometricConsentRevoked});

    auto unauthorized_sensor = live_owner_attempt();
    unauthorized_sensor.sensor_authorized = false;
    CHECK_EQ(refuse_owner_biometric_recognition(permissive_policy(), owner, self_consent,
                                                enrollment, owner_challenge(), unauthorized_sensor, at(2'010)),
             std::optional{OwnerBiometricRefusal::SensorUnauthorized});

    const auto another_subjects_consent = lease_for("ana", {ConsentScope::BiometricIdentity},
                                                    ConsentEvidence::SelfConsent);
    CHECK_EQ(refuse_owner_biometric_recognition(permissive_policy(), owner,
                                                another_subjects_consent, enrollment,
                                                owner_challenge(), live_owner_attempt(), at(2'010)),
             std::optional{OwnerBiometricRefusal::BiometricConsentNotGranted});

    auto no_liveness = live_owner_attempt();
    no_liveness.result = OwnerBiometricMatch::LivenessNotEstablished;
    CHECK_EQ(refuse_owner_biometric_recognition(permissive_policy(), owner, self_consent,
                                                enrollment, owner_challenge(), no_liveness, at(2'010)),
             std::optional{OwnerBiometricRefusal::LivenessNotEstablished});

    auto stale = live_owner_attempt();
    stale.captured_at = at(2'000);
    CHECK_EQ(refuse_owner_biometric_recognition(permissive_policy(), owner, self_consent,
                                                enrollment, owner_challenge(), stale, at(62'001)),
             std::optional{OwnerBiometricRefusal::CaptureStale});
}

void owner_biometric_recognition_binds_liveness_to_one_fresh_challenge() {
    CASE("a live owner match cannot be replayed outside its short challenge window");

    const auto owner = person("nelson", SubjectKind::Owner);
    const auto lease = lease_for("nelson", {ConsentScope::BiometricIdentity},
                                 ConsentEvidence::SelfConsent);
    const auto enrollment = owner_enrollment();
    const auto attempt = live_owner_attempt();

    auto mismatched = owner_challenge();
    mismatched.challenge_sha256 =
        "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
    CHECK_EQ(refuse_owner_biometric_recognition(permissive_policy(), owner, lease, enrollment,
                                                mismatched, attempt, at(2'010)),
             std::optional{OwnerBiometricRefusal::ChallengeMismatch});

    auto expired = owner_challenge();
    expired.expires_at = at(2'005);
    CHECK_EQ(refuse_owner_biometric_recognition(permissive_policy(), owner, lease, enrollment,
                                                expired, attempt, at(2'010)),
             std::optional{OwnerBiometricRefusal::ChallengeExpired});

    auto invalid = owner_challenge();
    invalid.expires_at = at(32'001);
    CHECK_EQ(refuse_owner_biometric_recognition(permissive_policy(), owner, lease, enrollment,
                                                invalid, attempt, at(2'010)),
             std::optional{OwnerBiometricRefusal::InvalidChallenge});

    auto pre_enrollment = owner_challenge();
    pre_enrollment.issued_at = at(990);
    CHECK_EQ(refuse_owner_biometric_recognition(permissive_policy(), owner, lease, enrollment,
                                                pre_enrollment, attempt, at(2'010)),
             std::optional{OwnerBiometricRefusal::ChallengePrecedesEnrollment});
}

// ---------------------------------------------------------------- edges

void co_presence_is_not_a_relationship() {
    CASE("no edge may be inferred from people having been in the same place");

    const auto a = person("ana", SubjectKind::ConsentedPerson);
    const auto b = person("beto", SubjectKind::ConsentedPerson);

    // Refused for every kind, including the ones that look most innocuous. A year of
    // "these two were in the lobby at the same time" is a social graph assembled from
    // surveillance, and colleague edges are exactly how it would start.
    for (const auto kind : {RelationKind::Colleague, RelationKind::Household,
                            RelationKind::Family, RelationKind::Friend,
                            RelationKind::AuthorizedGuest, RelationKind::Employer}) {
        const auto refusal = refuse_edge(
            edge("ana", "beto", kind, EdgeBasis::CoPresenceInference), a, b, "ana");
        CHECK_BECAUSE(refusal == EdgeRefusal::InferredFromCoPresence,
                      "co-presence is refused for every relationship kind, with no "
                      "exception that could be widened later");
    }
}

void a_model_proposes_and_never_asserts() {
    CASE("generative output is a hypothesis, not an edge");

    const auto a = person("ana", SubjectKind::ConsentedPerson);
    const auto b = person("beto", SubjectKind::ConsentedPerson);

    CHECK_EQ(refuse_edge(edge("ana", "beto", RelationKind::Colleague,
                              EdgeBasis::ModelHypothesis),
                         a, b, "ana"),
             std::optional{EdgeRefusal::GenerativeCannotAssert});

    // And a hypothesis never becomes the justification for holding a record. The graph
    // keeps it readable — a lead is meant to be chased — but out of the traversal.
    RelationGraph graph{"nelson"};
    graph.hypothesise_edge(
        edge("nelson", "stranger", RelationKind::Colleague, EdgeBasis::ModelHypothesis));

    CHECK_BECAUSE(!graph.path_to_owner("stranger").has_value(),
                  "if a guess could form part of the path back to the operator, the guess "
                  "would be the reason a record about a stranger is kept");
    CHECK_EQ(graph.hypotheses().size(), std::size_t{1});
}

void family_is_declared_by_family() {
    CASE("personal ties are never read out of the public record");

    const auto owner = person("nelson", SubjectKind::Owner);
    const auto relative = person("mama", SubjectKind::ConsentedPerson);

    CHECK_EQ(refuse_edge(edge("nelson", "mama", RelationKind::Family, EdgeBasis::PublicRecord),
                         owner, relative, ""),
             std::optional{EdgeRefusal::PersonalTieFromPublicRecord});

    // Declared by a participant: fine. This is the ordinary case the whole graph exists
    // for — the operator saying who is in their own life.
    CHECK(!refuse_edge(edge("nelson", "mama", RelationKind::Family, EdgeBasis::OwnerDeclared),
                       owner, relative, "nelson"));

    // Declared by someone it is not about: refused. Entering a family tree for two people
    // you read about is asserting facts about lives that did not ask.
    const auto x = person("x", SubjectKind::PublicFigure);
    const auto y = person("y", SubjectKind::PublicFigure);
    CHECK_EQ(refuse_edge(edge("x", "y", RelationKind::Family, EdgeBasis::OwnerDeclared),
                         x, y, "nelson"),
             std::optional{EdgeRefusal::NotDeclaredByAParticipant});
}

void professional_ties_may_come_from_the_public_record() {
    CASE("the reporter graph works as intended");

    const auto reporter = person("reporter", SubjectKind::PublicFigure);
    const auto editor = person("editor", SubjectKind::PublicFigure);

    // Two public figures, a professional tie, sourced from what they published. Nothing
    // here needs consent, and refusing it would make the media graph useless while
    // protecting nobody.
    CHECK(!refuse_edge(
        edge("reporter", "editor", RelationKind::Colleague, EdgeBasis::PublicRecord),
        reporter, editor, ""));
}

void an_edge_to_a_track_is_refused() {
    CASE("an anonymous track has no relationships");

    const auto owner = person("nelson", SubjectKind::Owner);
    const auto track = person("track-3", SubjectKind::AnonymousTrack);

    CHECK_BECAUSE(refuse_edge(edge("nelson", "track-3", RelationKind::AuthorizedGuest,
                                   EdgeBasis::OwnerDeclared),
                              owner, track, "nelson") ==
                      EdgeRefusal::EndpointNotIdentifiable,
                  "an edge to a track is how a track acquires an identity through the "
                  "back door");
}

void an_access_grant_is_about_this_node() {
    CASE("a guest edge must involve the operator");

    const auto a = person("ana", SubjectKind::ConsentedPerson);
    const auto b = person("beto", SubjectKind::ConsentedPerson);
    const auto owner = person("nelson", SubjectKind::Owner);

    CHECK_EQ(refuse_edge(edge("ana", "beto", RelationKind::AuthorizedGuest,
                              EdgeBasis::OwnerDeclared),
                         a, b, "nelson"),
             std::optional{EdgeRefusal::GuestTieMustInvolveOwner});

    CHECK(!refuse_edge(edge("nelson", "ana", RelationKind::AuthorizedGuest,
                            EdgeBasis::OwnerDeclared),
                       owner, a, "nelson"));
}

void a_self_edge_is_refused() {
    CASE("nobody is their own colleague");

    const auto a = person("ana", SubjectKind::ConsentedPerson);
    CHECK_EQ(refuse_edge(edge("ana", "ana", RelationKind::Colleague, EdgeBasis::PublicRecord),
                         a, a, ""),
             std::optional{EdgeRefusal::SelfEdge});
}

// ---------------------------------------------------------------- the tree

void every_record_has_a_path_back_to_the_operator() {
    CASE("the graph is rooted, and the path is the justification");

    RelationGraph graph{"nelson"};
    graph.assert_edge(edge("nelson", "ana", RelationKind::Colleague, EdgeBasis::OwnerDeclared));
    graph.assert_edge(edge("ana", "beto", RelationKind::Colleague, EdgeBasis::PublicRecord));

    CHECK_EQ(graph.degree("nelson"), std::optional{std::size_t{0}});
    CHECK_EQ(graph.degree("ana"), std::optional{std::size_t{1}});
    CHECK_EQ(graph.degree("beto"), std::optional{std::size_t{2}});

    const auto path = graph.path_to_owner("beto");
    CHECK(path.has_value());
    if (path) {
        const std::vector<std::string> expected{"nelson", "ana", "beto"};
        CHECK_BECAUSE(*path == expected,
                      "the path is the answer to 'why does Sentry hold a record about "
                      "this person', so it must read as a chain from the operator");
    }

    // Direction does not decide reachability. "Nelson employs Ana" and "Ana is employed
    // by Nelson" are the same tie, and a justification that depended on which way round
    // it was entered would be arbitrary.
    RelationGraph reversed{"nelson"};
    reversed.assert_edge(
        edge("ana", "nelson", RelationKind::Employer, EdgeBasis::OwnerDeclared));
    CHECK_EQ(reversed.degree("ana"), std::optional{std::size_t{1}});
}

void someone_with_no_path_to_the_operator_is_reported() {
    CASE("orphans are surfaced, not silently kept");

    RelationGraph graph{"nelson"};
    graph.assert_edge(edge("nelson", "ana", RelationKind::Colleague, EdgeBasis::OwnerDeclared));
    // A disconnected component: two people who have nothing to do with this node.
    graph.assert_edge(edge("x", "y", RelationKind::Colleague, EdgeBasis::PublicRecord));

    const auto orphans = graph.orphans();
    const std::vector<std::string> expected{"x", "y"};
    CHECK_BECAUSE(orphans == expected,
                  "a record with no chain back to the operator has no reason to exist, "
                  "and reporting it is how the bug that produced it gets found");

    CHECK(!graph.path_to_owner("x").has_value());
    CHECK(graph.orphans().empty() == false);
}

void the_shortest_chain_is_the_one_reported() {
    CASE("the justification is the most direct one that exists");

    RelationGraph graph{"nelson"};
    graph.assert_edge(edge("nelson", "ana", RelationKind::Colleague, EdgeBasis::OwnerDeclared));
    graph.assert_edge(edge("nelson", "beto", RelationKind::Colleague, EdgeBasis::OwnerDeclared));
    graph.assert_edge(edge("beto", "ana", RelationKind::Colleague, EdgeBasis::PublicRecord));

    CHECK_EQ(graph.degree("ana"), std::optional{std::size_t{1}});
}

void an_empty_graph_still_knows_the_operator() {
    CASE("the operator is in their own graph");

    const RelationGraph graph{"nelson"};
    CHECK_EQ(graph.degree("nelson"), std::optional{std::size_t{0}});
    CHECK(!graph.path_to_owner("anyone").has_value());
    CHECK(graph.orphans().empty());
}

}  // namespace

int main() {
    being_in_the_news_is_not_consent_to_be_recognised();
    the_public_figure_record_holds_public_professional_data_only();
    an_anonymous_track_cannot_acquire_an_identity();
    the_operator_is_bound_by_their_own_policy();
    a_revoked_lease_stops_working_immediately();
    an_attestation_is_not_a_signature();
    the_owner_can_enroll_and_prove_live_presence_under_current_authority();
    owner_biometric_recognition_binds_the_result_to_a_real_camera_grant();
    owner_biometric_recognition_never_becomes_general_face_recognition();
    owner_biometric_recognition_requires_current_consent_sensor_liveness_and_freshness();
    owner_biometric_recognition_binds_liveness_to_one_fresh_challenge();

    co_presence_is_not_a_relationship();
    a_model_proposes_and_never_asserts();
    family_is_declared_by_family();
    professional_ties_may_come_from_the_public_record();
    an_edge_to_a_track_is_refused();
    an_access_grant_is_about_this_node();
    a_self_edge_is_refused();

    every_record_has_a_path_back_to_the_operator();
    someone_with_no_path_to_the_operator_is_reported();
    the_shortest_chain_is_the_one_reported();
    an_empty_graph_still_knows_the_operator();

    return sentry::test::report();
}
