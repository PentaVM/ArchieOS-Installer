// The canonical invariants, as executable rules.
//
// These are not unit tests of convenience. Each case here corresponds to a line in
// RFC-008's invariant list, and a failure means Sentry has stopped being the thing it
// was specified to be — not that a helper changed shape.

#include "harness.hpp"

#include "sentry/camera/authority.hpp"
#include "sentry/policy/consent.hpp"
#include "sentry/world/epistemic.hpp"
#include "sentry/world/time.hpp"

using namespace sentry;
using world::TimePoint;

namespace {

constexpr TimePoint at(world::UnixMillis ms) { return TimePoint{ms}; }

// A plausible "now" inside the accepted range: 2026-01-01T00:00:00Z.
constexpr world::UnixMillis kNow = 1'767'225'600'000;

// ---------------------------------------------------------------------------
// INVARIANT 4 — GENERATIVE OUTPUT IS NEVER VERIFIED WORLD STATE
// ---------------------------------------------------------------------------

void generative_output_never_becomes_world_state() {
    CASE("invariant 4: a generative model cannot assert world state");

    // Every state Sentry asserts, attempted by a language model, with every advantage:
    // plenty of independent sources, a legal source state, a current clock.
    for (const auto target : {world::EpistemicState::Verified,
                              world::EpistemicState::Corroborated,
                              world::EpistemicState::Inferred,
                              world::EpistemicState::Normalized,
                              world::EpistemicState::Retracted}) {
        const world::Transition t{
            .from = world::EpistemicState::Normalized,
            .to = target,
            .actor = world::Actor::GenerativeModel,
            .independent_sources = 99,
            .at = at(kNow),
            .agent_version = "some-large-model-v9",
        };
        CHECK_BECAUSE(
            world::refuse(t) == world::TransitionRefusal::GenerativeCannotAssert,
            "a model with unlimited confidence and unlimited sources still may not "
            "promote its own output; the moment it can, the evidence graph is worthless");
    }

    // The one thing it may do.
    const world::Transition hypothesis{
        .from = world::EpistemicState::Normalized,
        .to = world::EpistemicState::ArchieHypothesis,
        .actor = world::Actor::GenerativeModel,
        .independent_sources = 0,
        .at = at(kNow),
        .agent_version = "some-large-model-v9",
    };
    CHECK_BECAUSE(world::permitted(hypothesis),
                  "proposing a research lead is exactly what generative reasoning is for");

    // And a hypothesis is a dead end, not a rung on the ladder: the way it becomes world
    // state is that something goes and observes the thing.
    for (const auto target : {world::EpistemicState::Verified,
                              world::EpistemicState::Corroborated,
                              world::EpistemicState::Inferred}) {
        const world::Transition promote{
            .from = world::EpistemicState::ArchieHypothesis,
            .to = target,
            .actor = world::Actor::Fusion,
            .independent_sources = 99,
            .at = at(kNow),
            .agent_version = "fusion-v1",
        };
        CHECK_BECAUSE(world::refuse(promote) == world::TransitionRefusal::NotReachable,
                      "even the fusion engine cannot promote a hypothesis; a new "
                      "observation with its own provenance is required");
    }

    CHECK_BECAUSE(!world::is_asserted(world::EpistemicState::ArchieHypothesis),
                  "a hypothesis may be shown, never in the register of an observed fact");
    CHECK(!world::produces_evidence(world::Actor::GenerativeModel));
    CHECK(world::produces_evidence(world::Actor::SpecialistModel));
}

void corroboration_requires_genuinely_independent_sources() {
    CASE("corroboration counts independent roots, not documents");

    auto attempt = [](int sources) {
        return world::refuse(world::Transition{
            .from = world::EpistemicState::Normalized,
            .to = world::EpistemicState::Corroborated,
            .actor = world::Actor::Fusion,
            .independent_sources = sources,
            .at = at(kNow),
            .agent_version = "fusion-v1",
        });
    };

    CHECK_BECAUSE(attempt(1) == world::TransitionRefusal::InsufficientIndependence,
                  "one source agreeing with itself is not agreement; four outlets "
                  "reprinting one wire story arrive here as one");
    CHECK(attempt(0) == world::TransitionRefusal::InsufficientIndependence);
    CHECK(!attempt(2).has_value());
}

void a_withdrawn_fact_stays_withdrawn_unless_a_human_says_otherwise() {
    CASE("retracted and superseded are terminal for automation");

    for (const auto actor : {world::Actor::Fusion, world::Actor::Detector,
                             world::Actor::Normalizer, world::Actor::SpecialistModel}) {
        const world::Transition t{
            .from = world::EpistemicState::Retracted,
            .to = world::EpistemicState::Verified,
            .actor = actor,
            .independent_sources = 10,
            .at = at(kNow),
            .agent_version = "v1",
        };
        CHECK_BECAUSE(world::refuse(t) == world::TransitionRefusal::TerminalState,
                      "no automated pass may quietly resurrect a fact its source withdrew");
    }

    const world::Transition by_operator{
        .from = world::EpistemicState::Retracted,
        .to = world::EpistemicState::Verified,
        .actor = world::Actor::Operator,
        .independent_sources = 0,
        .at = at(kNow),
        .agent_version = {},
    };
    CHECK_BECAUSE(world::permitted(by_operator),
                  "a source that retracts in error is a real situation a human resolves");

    // An operator asserting something is not a model hypothesising it.
    const world::Transition operator_hypothesis{
        .from = world::EpistemicState::Normalized,
        .to = world::EpistemicState::ArchieHypothesis,
        .actor = world::Actor::Operator,
        .independent_sources = 0,
        .at = at(kNow),
        .agent_version = {},
    };
    CHECK(world::refuse(operator_hypothesis) == world::TransitionRefusal::NotReachable);
}

void a_stale_fact_re_earns_its_standing() {
    CASE("staleness demotes; recovery does not restore the old standing");

    const world::Transition recovered{
        .from = world::EpistemicState::Stale,
        .to = world::EpistemicState::Verified,
        .actor = world::Actor::Fusion,
        .independent_sources = 10,
        .at = at(kNow),
        .agent_version = "fusion-v1",
    };
    CHECK_BECAUSE(world::refuse(recovered) == world::TransitionRefusal::NotReachable,
                  "fresher data re-enters at Normalized and climbs again; resuming "
                  "Verified would carry an old verification onto new bytes");

    const world::Transition renormalized{
        .from = world::EpistemicState::Stale,
        .to = world::EpistemicState::Normalized,
        .actor = world::Actor::Normalizer,
        .independent_sources = 0,
        .at = at(kNow),
        .agent_version = "norm-v1",
    };
    CHECK(world::permitted(renormalized));
}

// ---------------------------------------------------------------------------
// INVARIANT 8 — TIME IS FIRST-CLASS
// ---------------------------------------------------------------------------

void replay_filters_on_knowledge_time_not_event_time() {
    CASE("invariant 8: what Archie knew at T is a question about fetch time");

    // An earthquake at 03:00, published 03:14, fetched 03:31.
    world::TemporalExtent quake;
    quake.observed_at = at(kNow);
    quake.published_at = at(kNow + 14 * 60'000);
    quake.fetched_at = at(kNow + 31 * 60'000);
    quake.normalized_at = at(kNow + 31 * 60'000);

    CHECK(quake.plausible());

    // At 03:15 the event had happened and been published, and Archie still did not have
    // it. A replay filtering on event time would hand this to a detector that could not
    // have had it, and every detector would look prescient.
    const auto three_fifteen = at(kNow + 15 * 60'000);
    CHECK_BECAUSE(!quake.known_by(three_fifteen),
                  "the event was over and published, but not yet fetched; this is the "
                  "distinction that stops replay flattering our own detectors");
    CHECK(quake.known_by(at(kNow + 31 * 60'000)));
    CHECK(quake.known_by(at(kNow + 90 * 60'000)));

    // Staleness is measured in event time, because that is what "how old is this" means.
    CHECK_EQ(quake.staleness_at(at(kNow + 60 * 60'000)), std::optional<std::int64_t>{3'600'000});
}

void a_record_about_no_moment_is_not_reported_as_fresh() {
    CASE("a record with no event time has no staleness, rather than zero");

    world::TemporalExtent catalogue_entry;
    catalogue_entry.fetched_at = at(kNow);
    catalogue_entry.normalized_at = at(kNow);

    CHECK_BECAUSE(!catalogue_entry.staleness_at(at(kNow + 86'400'000)).has_value(),
                  "a provider catalogue entry is not stale a day later; it is simply not "
                  "about a moment, and reporting zero would claim it is current");
    CHECK(!catalogue_entry.event_time().has_value());
}

void impossible_timestamps_are_caught_at_the_boundary() {
    CASE("a provider whose clock is ahead of ours is a fault, not data");

    world::TemporalExtent ahead;
    ahead.observed_at = at(kNow + 60'000);
    ahead.fetched_at = at(kNow);
    ahead.normalized_at = at(kNow);
    CHECK_BECAUSE(ahead.fault() == world::TemporalFault::ObservedAfterFetched,
                  "we cannot have retrieved a record before the thing it describes "
                  "happened; every downstream time alignment would be wrong");

    // Seconds mistaken for milliseconds — the classic ingestion bug — and the global
    // range check provably cannot catch it. A 2026 timestamp divided by a thousand lands
    // in January 1970, which is a perfectly ordinary date. Narrowing the window to
    // exclude it would reject UCDP records from 1946, which are real history.
    world::TemporalExtent unit_error;
    unit_error.observed_at = at(1'767'225'600);
    unit_error.fetched_at = at(kNow);
    unit_error.normalized_at = at(kNow);
    CHECK_BECAUSE(unit_error.plausible(),
                  "1970 is a plausible date; this is why the unit bug needs provider "
                  "context rather than a global window");

    // What separates the bug from the history is the provider. An earthquake feed
    // promising data minutes old has not legitimately returned a 1970 record.
    constexpr std::int64_t kFiveMinutes = 5 * 60'000;
    CHECK_BECAUSE(unit_error.implausibly_stale_for(at(kNow), kFiveMinutes),
                  "a near-real-time provider returning a 56-year-old observation has a "
                  "decoding fault, not a late record");

    // The same record from a historical archive, which makes no freshness promise, is
    // exactly what that archive is for.
    CHECK(!unit_error.implausibly_stale_for(at(kNow), 0));

    // And a genuinely late record from a fresh provider is late, not corrupt.
    world::TemporalExtent merely_late;
    merely_late.observed_at = at(kNow - 20 * 60'000);
    merely_late.fetched_at = at(kNow);
    merely_late.normalized_at = at(kNow);
    CHECK(!merely_late.implausibly_stale_for(at(kNow), kFiveMinutes));

    world::TemporalExtent inverted;
    inverted.fetched_at = at(kNow);
    inverted.normalized_at = at(kNow);
    inverted.valid_from = at(kNow + 1000);
    inverted.valid_until = at(kNow);
    CHECK(inverted.fault() == world::TemporalFault::ValidityInverted);

    // A pre-1970 conflict record is legitimate history, not corrupt data.
    world::TemporalExtent historical;
    historical.observed_at = at(-757'382'400'000);  // 1946
    historical.fetched_at = at(kNow);
    historical.normalized_at = at(kNow);
    CHECK_BECAUSE(historical.plausible(),
                  "UCDP reaches back to 1946; an unsigned epoch would wrap this into the "
                  "far future instead of accepting it");

    // An open-ended validity window has not expired.
    world::TemporalExtent open;
    open.fetched_at = at(kNow);
    open.normalized_at = at(kNow);
    open.valid_from = at(kNow);
    CHECK(open.valid_at(at(kNow + 10'000'000)));
}

// ---------------------------------------------------------------------------
// INVARIANTS 1 & 3 — CAMERA AUTHORITY IS DETERMINISTIC; HEALTH ≠ AUTHORIZATION
// ---------------------------------------------------------------------------

camera::CameraAuthority authorized_camera() {
    camera::CameraAuthority a;
    a.camera_id = "gate-north";
    a.basis = camera::AuthorityBasis::OperatorAuthorized;
    a.desired = camera::DesiredConnectionState::Connected;
    a.granted_at = at(kNow - 86'400'000);
    a.evidence = "work order 4471, written authorization from site owner";
    return a;
}

void no_connection_failure_ever_deauthorizes_a_camera() {
    CASE("invariant 3: sensor health is not authorization");

    const auto camera = authorized_camera();
    const auto now = at(kNow);

    for (const auto observed : {camera::ObservedConnectionState::Offline,
                                camera::ObservedConnectionState::AuthFailed,
                                camera::ObservedConnectionState::StreamStalled,
                                camera::ObservedConnectionState::CodecUnsupported,
                                camera::ObservedConnectionState::Backoff,
                                camera::ObservedConnectionState::Recovering}) {
        const auto line = camera::status_line(camera, observed, now);
        CHECK_BECAUSE(line.authorized,
                      "every one of these is a network or device fault; none of them is a "
                      "decision by the operator, and none may read as one");
        CHECK(!line.delivering);
    }

    // The sentence an operator most often misreads.
    const auto auth_failed =
        camera::status_line(camera, camera::ObservedConnectionState::AuthFailed, now);
    CHECK_BECAUSE(auth_failed.authority_reason == std::string_view{"authorized by operator"},
                  "the camera rejecting our password says nothing about our authority to "
                  "be asking");
    CHECK(auth_failed.connection_reason.find("credentials") != std::string_view::npos);
}

void automation_can_never_change_authority() {
    CASE("invariant 1: operator camera authority is deterministic");

    const auto camera = authorized_camera();
    const camera::AuthorityChange automated{
        .actor = camera::AuthorityActor::Automation,
        .new_basis = camera::AuthorityBasis::Blocked,
        .new_desired = camera::DesiredConnectionState::Disabled,
    };
    CHECK_BECAUSE(camera::refuse(camera, automated) ==
                      camera::AuthorityRefusal::OnlyOperatorMayDecide,
                  "a reconnect loop that has failed forty times still has no standing to "
                  "deauthorize; its only legitimate output is an observed state");

    const camera::AuthorityChange by_operator{
        .actor = camera::AuthorityActor::Operator,
        .new_basis = camera::AuthorityBasis::Blocked,
        .new_desired = camera::DesiredConnectionState::Disabled,
    };
    CHECK(!camera::refuse(camera, by_operator).has_value());
}

void a_discovered_camera_is_not_an_authorized_one() {
    CASE("finding an exposed camera is a security finding, not consent");

    camera::CameraAuthority discovered;
    discovered.camera_id = "203.0.113.7:554";
    discovered.basis = camera::AuthorityBasis::DiscoveredOnly;
    discovered.desired = camera::DesiredConnectionState::Connected;
    discovered.granted_at = at(kNow);

    CHECK_BECAUSE(!discovered.may_open_stream_at(at(kNow)),
                  "an Internet-wide scan tells us a device is exposed, which is a fact "
                  "about its owner's security, not permission from them");
    CHECK(!camera::permits_video(camera::AuthorityBasis::DiscoveredOnly));
    CHECK(!camera::permits_video(camera::AuthorityBasis::Blocked));
    CHECK(camera::permits_video(camera::AuthorityBasis::IntentionallyPublic));

    // Even the operator cannot promote a blocked camera in one step.
    camera::CameraAuthority blocked = discovered;
    blocked.basis = camera::AuthorityBasis::Blocked;
    const camera::AuthorityChange promote{
        .actor = camera::AuthorityActor::Operator,
        .new_basis = camera::AuthorityBasis::OperatorAuthorized,
        .new_desired = camera::DesiredConnectionState::Connected,
    };
    CHECK_BECAUSE(camera::refuse(blocked, promote) ==
                      camera::AuthorityRefusal::CameraIsBlocked,
                  "unblocking is its own action, so an accidental promotion cannot look "
                  "like a deliberate one in the audit trail");
}

void an_undecodable_stream_is_not_retried_forever() {
    CASE("a codec we cannot decode needs a human, not a hot loop");

    CHECK(!camera::warrants_retry(camera::ObservedConnectionState::CodecUnsupported));
    CHECK(camera::warrants_retry(camera::ObservedConnectionState::Offline));
    CHECK_BECAUSE(camera::warrants_retry(camera::ObservedConnectionState::AuthFailed),
                  "credentials get rotated back and cameras get rebooted; this one does "
                  "recover on its own");
}

void an_authorized_camera_the_operator_disabled_reads_as_such() {
    CASE("disabled and unauthorized are different sentences");

    auto camera = authorized_camera();
    camera.desired = camera::DesiredConnectionState::Disabled;
    const auto line =
        camera::status_line(camera, camera::ObservedConnectionState::Offline, at(kNow));

    CHECK_BECAUSE(line.authorized,
                  "the operator turned it off; they did not withdraw their authority");
    CHECK(line.authority_reason.find("disabled") != std::string_view::npos);
    CHECK(!camera.may_open_stream_at(at(kNow)));
}

// ---------------------------------------------------------------------------
// INVARIANT 2 — IDENTITY RESOLUTION IS CONSENT-GATED
// ---------------------------------------------------------------------------

policy::OperatorPolicy permissive_policy() {
    policy::OperatorPolicy p;
    p.biometrics_enabled = true;
    p.cloud_export_enabled = false;
    p.historical_replay_enabled = true;
    p.accepted_evidence = {policy::ConsentEvidence::SubjectSigned,
                           policy::ConsentEvidence::SelfConsent,
                           policy::ConsentEvidence::OperatorAttested};
    return p;
}

policy::ConsentLease signed_lease() {
    policy::ConsentLease lease;
    lease.subject_id = "subject-1";
    lease.scopes = {policy::ConsentScope::GpsLocation,
                    policy::ConsentScope::CameraIdentity};
    lease.valid_from = at(kNow - 86'400'000);
    lease.evidence = policy::ConsentEvidence::SubjectSigned;
    return lease;
}

void permission_is_the_intersection_of_three_authorities() {
    CASE("invariant 2: operator policy ∩ subject consent ∩ sensor authority");

    const auto policy_ = permissive_policy();
    const auto lease = signed_lease();

    policy::IdentityRequest request{
        .subject_id = "subject-1",
        .scope = policy::ConsentScope::CameraIdentity,
        .location = {},
        .sensor_authorized = true,
        .at = at(kNow),
    };
    CHECK(!policy::refuse(policy_, lease, request).has_value());

    // Consent cannot supply sensor authority.
    auto unauthorized_sensor = request;
    unauthorized_sensor.sensor_authorized = false;
    CHECK_BECAUSE(policy::refuse(policy_, lease, unauthorized_sensor) ==
                      policy::ConsentRefusal::SensorNotAuthorized,
                  "a subject agreeing to be identified does not authorize a camera the "
                  "operator never approved");

    // Sensor authority cannot supply consent.
    CHECK_BECAUSE(policy::refuse(policy_, std::nullopt, request) ==
                      policy::ConsentRefusal::NoLease,
                  "owning the camera is not consent from the people in front of it");

    // Operator policy is a ceiling no subject can raise.
    auto strict = policy_;
    strict.biometrics_enabled = false;
    auto biometric = request;
    biometric.scope = policy::ConsentScope::BiometricIdentity;
    auto willing = lease;
    willing.scopes.push_back(policy::ConsentScope::BiometricIdentity);
    CHECK_BECAUSE(policy::refuse(strict, willing, biometric) ==
                      policy::ConsentRefusal::ForbiddenByOperatorPolicy,
                  "a subject who would allow biometrics does not enable them on a node "
                  "whose operator switched them off");
}

void the_ceiling_is_reported_before_the_missing_lease() {
    CASE("the refusal names the fix that would actually work");

    auto strict = permissive_policy();
    strict.biometrics_enabled = false;
    const policy::IdentityRequest request{
        .subject_id = "subject-1",
        .scope = policy::ConsentScope::BiometricIdentity,
        .location = {},
        .sensor_authorized = true,
        .at = at(kNow),
    };
    CHECK_BECAUSE(policy::refuse(strict, std::nullopt, request) ==
                      policy::ConsentRefusal::ForbiddenByOperatorPolicy,
                  "reporting 'no lease' would send an operator to collect a consent form "
                  "that would not have helped");
}

void a_camera_grant_never_authorizes_earlier_captures() {
    CASE("camera authority starts at the operator grant, not at the beginning of time");

    const auto camera = authorized_camera();
    CHECK(!camera.in_force_at(at(kNow - 86'400'001)));
    CHECK(!camera.may_open_stream_at(at(kNow - 86'400'001)));
    CHECK(camera.in_force_at(at(kNow - 86'400'000)));
}

void a_consent_lease_is_bound_to_its_subject() {
    CASE("a consent lease cannot be reused for a different person");

    const auto policy_ = permissive_policy();
    auto lease = signed_lease();
    lease.subject_id = "subject-2";
    const policy::IdentityRequest request{
        .subject_id = "subject-1",
        .scope = policy::ConsentScope::CameraIdentity,
        .location = {},
        .sensor_authorized = true,
        .at = at(kNow),
    };
    CHECK_BECAUSE(policy::refuse(policy_, lease, request) ==
                      policy::ConsentRefusal::LeaseSubjectMismatch,
                  "a valid grant for someone else cannot identify this subject");
}

void consent_survives_everything_except_a_decision() {
    CASE("a lease is not ended by a sensor outage, a restart, or a model");

    const auto policy_ = permissive_policy();
    auto lease = signed_lease();

    // Nothing in this API can express "the camera went down, so consent lapsed". The
    // lease's own predicate depends only on the clock and the revocation flag.
    CHECK(lease.active_at(at(kNow)));
    CHECK(lease.active_at(at(kNow + 365LL * 86'400'000)));

    lease.valid_until = at(kNow + 3'600'000);
    CHECK(lease.active_at(at(kNow)));
    CHECK(!lease.active_at(at(kNow + 3'600'001)));

    policy::IdentityRequest request{
        .subject_id = "subject-1",
        .scope = policy::ConsentScope::CameraIdentity,
        .location = {},
        .sensor_authorized = true,
        .at = at(kNow + 7'200'000),
    };
    CHECK(policy::refuse(policy_, lease, request) == policy::ConsentRefusal::Expired);

    auto revoked = signed_lease();
    revoked.revocation = policy::RevocationState::Revoked;
    request.at = at(kNow);
    CHECK_BECAUSE(policy::refuse(policy_, revoked, request) ==
                      policy::ConsentRefusal::Revoked,
                  "withdrawn and lapsed call for different responses: one is a decision "
                  "to respect, the other a renewal to seek");
}

void one_scope_never_implies_another() {
    CASE("consenting to be located is not consenting to be recognised");

    const auto policy_ = permissive_policy();
    const auto lease = signed_lease();  // GPS + camera identity, not biometrics

    const policy::IdentityRequest biometric{
        .subject_id = "subject-1",
        .scope = policy::ConsentScope::BiometricIdentity,
        .location = {},
        .sensor_authorized = true,
        .at = at(kNow),
    };
    CHECK(policy::refuse(policy_, lease, biometric) ==
          policy::ConsentRefusal::ScopeNotGranted);

    const policy::IdentityRequest cloud{
        .subject_id = "subject-1",
        .scope = policy::ConsentScope::CloudProcessing,
        .location = {},
        .sensor_authorized = true,
        .at = at(kNow),
    };
    CHECK_BECAUSE(policy::refuse(policy_, lease, cloud) ==
                      policy::ConsentRefusal::ForbiddenByOperatorPolicy,
                  "cloud export is off on this node, so nothing about this subject leaves "
                  "it regardless of what they agreed to");

    // And nothing at all is permitted with no lease.
    for (const auto scope : {policy::ConsentScope::GpsLocation,
                             policy::ConsentScope::CameraIdentity,
                             policy::ConsentScope::BiometricIdentity,
                             policy::ConsentScope::Proximity,
                             policy::ConsentScope::HistoricalReplay,
                             policy::ConsentScope::CloudProcessing}) {
        CHECK(!policy::permitted_without_consent(scope));
    }
}

void a_lease_bounded_to_a_place_does_not_follow_the_subject() {
    CASE("a spatial scope is a restriction, not a hint");

    const auto policy_ = permissive_policy();
    auto lease = signed_lease();
    lease.spatial_scope = {"site-a"};

    policy::IdentityRequest inside{
        .subject_id = "subject-1",
        .scope = policy::ConsentScope::CameraIdentity,
        .location = "site-a",
        .sensor_authorized = true,
        .at = at(kNow),
    };
    CHECK(!policy::refuse(policy_, lease, inside).has_value());

    auto outside = inside;
    outside.location = "site-b";
    CHECK(policy::refuse(policy_, lease, outside) ==
          policy::ConsentRefusal::OutsideSpatialScope);

    // An operation with no location cannot be shown to be inside a bounded lease.
    auto nowhere = inside;
    nowhere.location = {};
    CHECK_BECAUSE(policy::refuse(policy_, lease, nowhere) ==
                      policy::ConsentRefusal::OutsideSpatialScope,
                  "unknown location is not the same as permitted location");
}

void hard_contradiction_defeats_visual_similarity() {
    CASE("a confident face match loses to physics");

    policy::AssociationEvidence looks_right;
    looks_right.appearance_match = true;
    CHECK_EQ(policy::resolve(looks_right), policy::IdentityAssociation::Candidate);

    // The subject's own authorized location provider puts them elsewhere.
    auto contradicted = looks_right;
    contradicted.contradicted_by_location = true;
    CHECK_BECAUSE(policy::resolve(contradicted) == policy::IdentityAssociation::Rejected,
                  "appearance matching is confidently wrong about twins, uniforms and "
                  "dark coats; an authorized location fix outranks it");

    // Two cameras too far apart to walk between in the time available.
    policy::AssociationEvidence impossible_transit;
    impossible_transit.appearance_match = true;
    impossible_transit.device_colocation = true;
    impossible_transit.contradicted_by_transit_time = true;
    CHECK_BECAUSE(policy::resolve(impossible_transit) ==
                      policy::IdentityAssociation::Rejected,
                  "no amount of agreement makes an impossible journey possible");

    // Independent kinds of evidence agreeing is worth more than either repeated.
    policy::AssociationEvidence both;
    both.appearance_match = true;
    both.device_colocation = true;
    CHECK_EQ(policy::resolve(both), policy::IdentityAssociation::Corroborated);

    policy::AssociationEvidence device_only;
    device_only.device_colocation = true;
    CHECK(policy::resolve(device_only) == policy::IdentityAssociation::Likely);

    policy::AssociationEvidence human;
    human.operator_identified = true;
    CHECK_EQ(policy::resolve(human), policy::IdentityAssociation::Confirmed);

    CHECK_EQ(policy::resolve({}), policy::IdentityAssociation::Unknown);
}

void weak_evidence_classes_can_be_refused_by_policy() {
    CASE("an operator may decline to accept their own attestation");

    auto strict = permissive_policy();
    strict.accepted_evidence = {policy::ConsentEvidence::SubjectSigned};

    auto attested = signed_lease();
    attested.evidence = policy::ConsentEvidence::OperatorAttested;

    const policy::IdentityRequest request{
        .subject_id = "subject-1",
        .scope = policy::ConsentScope::CameraIdentity,
        .location = {},
        .sensor_authorized = true,
        .at = at(kNow),
    };
    CHECK(policy::refuse(strict, attested, request) ==
          policy::ConsentRefusal::EvidenceNotAccepted);
    CHECK(!policy::refuse(strict, signed_lease(), request).has_value());
}

}  // namespace

int main() {
    generative_output_never_becomes_world_state();
    corroboration_requires_genuinely_independent_sources();
    a_withdrawn_fact_stays_withdrawn_unless_a_human_says_otherwise();
    a_stale_fact_re_earns_its_standing();

    replay_filters_on_knowledge_time_not_event_time();
    a_record_about_no_moment_is_not_reported_as_fresh();
    impossible_timestamps_are_caught_at_the_boundary();

    no_connection_failure_ever_deauthorizes_a_camera();
    automation_can_never_change_authority();
    a_discovered_camera_is_not_an_authorized_one();
    a_camera_grant_never_authorizes_earlier_captures();
    an_undecodable_stream_is_not_retried_forever();
    an_authorized_camera_the_operator_disabled_reads_as_such();

    permission_is_the_intersection_of_three_authorities();
    the_ceiling_is_reported_before_the_missing_lease();
    a_consent_lease_is_bound_to_its_subject();
    consent_survives_everything_except_a_decision();
    one_scope_never_implies_another();
    a_lease_bounded_to_a_place_does_not_follow_the_subject();
    hard_contradiction_defeats_visual_similarity();
    weak_evidence_classes_can_be_refused_by_policy();

    return sentry::test::report();
}
