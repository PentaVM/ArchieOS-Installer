// Person tracking, and the narrow path by which a body ever gets a name.
//
// A track is anonymous by construction. These cases are about the one operation that can
// change that — resolution — and the three independent checks it has to pass. The through
// line: a perfect appearance match is not enough on its own for any of them, because the
// whole point is that a face model's confidence never decides whether the rule applies.

#include "harness.hpp"

#include "sentry/vision/track.hpp"

using namespace sentry::vision;
using sentry::camera::AuthorityBasis;
using sentry::camera::CameraAuthority;
using sentry::identity::SubjectKind;
using sentry::identity::SubjectRecord;
using sentry::policy::AssociationEvidence;
using sentry::policy::ConsentEvidence;
using sentry::policy::ConsentLease;
using sentry::policy::ConsentScope;
using sentry::policy::IdentityAssociation;
using sentry::policy::OperatorPolicy;
using sentry::world::TimePoint;

namespace {

TimePoint at(std::int64_t ms) { return TimePoint{ms}; }

SubjectRecord subject(std::string id, SubjectKind kind) {
    SubjectRecord s;
    s.subject_id = std::move(id);
    s.kind = kind;
    s.display_label = s.subject_id;
    return s;
}

Track anon_track() {
    Track t;
    t.track_id = "cam1-track-7";
    t.camera_id = "cam1";
    t.first_seen = at(1'000);
    t.last_seen = at(2'000);
    t.detection_count = 12;
    return t;
}

// A camera the operator authorized, in force now.
CameraAuthority authorized_camera() {
    CameraAuthority a;
    a.camera_id = "cam1";
    a.basis = AuthorityBasis::OperatorAuthorized;
    a.granted_at = at(0);
    a.desired = sentry::camera::DesiredConnectionState::Connected;
    return a;
}

OperatorPolicy policy_allowing_camera_identity() {
    OperatorPolicy p;
    p.biometrics_enabled = false;
    p.cloud_export_enabled = false;
    p.historical_replay_enabled = true;
    p.accepted_evidence = {ConsentEvidence::SubjectSigned, ConsentEvidence::SelfConsent};
    return p;
}

ConsentLease camera_lease(std::string id) {
    ConsentLease l;
    l.subject_id = std::move(id);
    l.scopes = {ConsentScope::CameraIdentity};
    l.valid_from = at(0);
    l.evidence = ConsentEvidence::SubjectSigned;
    return l;
}

ResolutionContext consenting_context(const std::string& subject_id) {
    ResolutionContext c;
    c.camera_authority = authorized_camera();
    c.sensor_authorized_and_live = true;
    c.operator_policy = policy_allowing_camera_identity();
    c.lease = camera_lease(subject_id);
    c.location = "lobby";
    return c;
}

AssociationEvidence strong_match() {
    AssociationEvidence e;
    e.appearance_match = true;
    e.device_colocation = true;
    return e;
}

// ---------------------------------------------------------------- the safe default

void a_track_is_anonymous_and_still_counts() {
    CASE("a body with no name is still a body");

    const auto t = anon_track();
    CHECK_EQ(t.identity, TrackIdentity::Anonymous);
    CHECK(!t.is_named());
    CHECK_BECAUSE(t.counts_as_present(),
                  "an anonymous track still counts for occupancy and avoidance; a robot "
                  "that will not stop for someone it cannot name is the worse outcome");
}

// ---------------------------------------------------------------- 1. kind

void a_public_figure_is_never_resolved_from_a_camera() {
    CASE("§59/§114: being in the public record is not consent to be recognised");

    auto track = anon_track();
    const auto reporter = subject("reporter", SubjectKind::PublicFigure);

    // Everything else is maximally permissive: an authorized camera, a signed lease, a
    // perfect match. It does not matter. The kind decides, before any of that is weighed.
    auto context = consenting_context("reporter");
    const auto result = resolve_track(track, reporter, strong_match(), context, at(1'500));

    CHECK_BECAUSE(result.refusal == ResolutionRefusal::NotAResolvableKind,
                  "a byline plus a face model is face recognition; the kind check is what "
                  "refuses it before the face model is even consulted");
    apply_resolution(track, reporter, result);
    CHECK_BECAUSE(track.identity == TrackIdentity::Anonymous,
                  "the track is left exactly as it was: anonymous");
}

void an_anonymous_subject_is_not_resolvable() {
    CASE("a track cannot be resolved to a track");

    auto track = anon_track();
    const auto other = subject("track-99", SubjectKind::AnonymousTrack);
    const auto result = resolve_track(track, other, strong_match(),
                                      consenting_context("track-99"), at(1'500));
    CHECK_EQ(result.refusal, std::optional{ResolutionRefusal::NotAResolvableKind});
}

// ---------------------------------------------------------------- 2. consent

void without_consent_a_sighting_stays_anonymous() {
    CASE("§59: no lease means an anonymous track, not a name");

    auto track = anon_track();
    const auto rafa = subject("rafa", SubjectKind::ConsentedPerson);

    auto context = consenting_context("rafa");
    context.lease = std::nullopt;  // no consent on file

    const auto result = resolve_track(track, rafa, strong_match(), context, at(1'500));
    CHECK_BECAUSE(result.refusal == ResolutionRefusal::ConsentRefused,
                  "a consented person with no lease is still not resolvable — the match is "
                  "irrelevant without consent");
    apply_resolution(track, rafa, result);
    CHECK_EQ(track.identity, TrackIdentity::Anonymous);
}

void a_camera_that_is_not_authorized_resolves_nothing() {
    CASE("§114: sensor health is not authorization, and neither is a good picture");

    auto track = anon_track();
    const auto rafa = subject("rafa", SubjectKind::ConsentedPerson);

    auto context = consenting_context("rafa");
    context.camera_authority.basis = AuthorityBasis::DiscoveredOnly;  // found, not authorized

    const auto result = resolve_track(track, rafa, strong_match(), context, at(1'500));
    CHECK_EQ(result.refusal, std::optional{ResolutionRefusal::CameraNotAuthorized});
}

void a_live_but_unauthorized_sensor_resolves_nothing() {
    CASE("delivering video is not the same as being allowed to");

    auto track = anon_track();
    const auto rafa = subject("rafa", SubjectKind::ConsentedPerson);

    auto context = consenting_context("rafa");
    context.sensor_authorized_and_live = false;  // the stream is up but not authorized

    const auto result = resolve_track(track, rafa, strong_match(), context, at(1'500));
    CHECK_EQ(result.refusal, std::optional{ResolutionRefusal::CameraNotAuthorized});
}

// ---------------------------------------------------------------- 3. evidence

void consent_and_a_match_together_name_the_body() {
    CASE("the one path that resolves");

    auto track = anon_track();
    const auto rafa = subject("rafa", SubjectKind::ConsentedPerson);

    const auto result =
        resolve_track(track, rafa, strong_match(), consenting_context("rafa"), at(1'500));
    CHECK(result.resolved());
    CHECK_BECAUSE(result.association == IdentityAssociation::Corroborated,
                  "appearance plus device co-location is two independent signals");

    apply_resolution(track, rafa, result);
    CHECK_EQ(track.identity, TrackIdentity::Resolved);
    CHECK_EQ(track.subject_id, std::string{"rafa"});
    CHECK(track.is_named());
}

void consent_without_enough_evidence_stays_anonymous() {
    CASE("permitted is not the same as proven");

    auto track = anon_track();
    const auto rafa = subject("rafa", SubjectKind::ConsentedPerson);

    // A single weak signal. Consent permits resolution; the evidence does not carry it.
    AssociationEvidence weak;
    weak.appearance_match = true;

    const auto result = resolve_track(track, rafa, weak, consenting_context("rafa"), at(1'500));
    CHECK_BECAUSE(result.refusal == ResolutionRefusal::EvidenceInsufficient,
                  "one appearance match is a candidate, not a name; the track stays "
                  "anonymous and may resolve later with more");
    apply_resolution(track, rafa, result);
    CHECK_EQ(track.identity, TrackIdentity::Anonymous);
}

void a_transit_time_contradiction_rules_the_candidate_out() {
    CASE("two cameras that cannot be walked between did not see the same person");

    auto track = anon_track();
    const auto rafa = subject("rafa", SubjectKind::ConsentedPerson);

    // A perfect appearance match, defeated by a hard contradiction. This is the case
    // appearance matching is most confidently wrong about — twins, uniforms, a dark coat.
    AssociationEvidence contradicted = strong_match();
    contradicted.contradicted_by_transit_time = true;

    const auto result =
        resolve_track(track, rafa, contradicted, consenting_context("rafa"), at(1'500));
    CHECK_BECAUSE(result.refusal == ResolutionRefusal::Contradicted,
                  "a contradiction defeats any amount of similarity");

    apply_resolution(track, rafa, result);
    CHECK_BECAUSE(track.identity == TrackIdentity::Rejected,
                  "the ruling-out is recorded, so the same wrong candidate is not tried "
                  "again — distinct from an anonymous track that merely lacks evidence");
    CHECK(track.subject_id.empty());
}

void a_location_contradiction_also_rules_out() {
    CASE("the subject's own device put them somewhere else");

    auto track = anon_track();
    const auto rafa = subject("rafa", SubjectKind::ConsentedPerson);

    AssociationEvidence contradicted = strong_match();
    contradicted.contradicted_by_location = true;

    const auto result =
        resolve_track(track, rafa, contradicted, consenting_context("rafa"), at(1'500));
    CHECK_EQ(result.refusal, std::optional{ResolutionRefusal::Contradicted});
}

// ---------------------------------------------------------------- ordering

void the_kind_is_checked_before_consent_so_the_reason_is_actionable() {
    CASE("a public figure with no lease is refused for kind, not for consent");

    // If consent were checked first, the refusal would be ConsentRefused and someone would
    // go looking for a signature that should never be sought. Kind first makes the reason
    // point at the real thing: this person is not resolvable from a camera at all.
    auto track = anon_track();
    const auto reporter = subject("reporter", SubjectKind::PublicFigure);

    auto context = consenting_context("reporter");
    context.lease = std::nullopt;

    const auto result = resolve_track(track, reporter, strong_match(), context, at(1'500));
    CHECK_EQ(result.refusal, std::optional{ResolutionRefusal::NotAResolvableKind});
}

void a_cross_camera_match_over_an_impossible_distance_is_contradicted() {
    CASE("geography rules out a face match across two distant cameras");

    // The end-to-end safety property: a perfect appearance match, two cameras a minute
    // apart in New York and London. The geo layer computes the impossibility; the evidence
    // carries it; resolution refuses and the track is Rejected. No hand-set flag anywhere.
    const sentry::world::GeoPoint ny{40.71, -74.01, 0.0};
    const sentry::world::GeoPoint london{51.51, -0.13, 0.0};

    AssociationEvidence evidence = strong_match();
    evidence.contradicted_by_transit_time =
        sightings_contradict_by_transit(ny, london, 60'000);
    CHECK_BECAUSE(evidence.contradicted_by_transit_time,
                  "5500 km in a minute is impossible, and the check says so from geography");

    auto track = anon_track();
    const auto rafa = subject("rafa", SubjectKind::ConsentedPerson);
    const auto result = resolve_track(track, rafa, evidence, consenting_context("rafa"),
                                      at(1'500));
    CHECK_EQ(result.refusal, std::optional{ResolutionRefusal::Contradicted});

    // While a plausible cross-camera trip carries no contradiction.
    const sentry::world::GeoPoint downtown{40.710, -74.010, 0.0};
    const sentry::world::GeoPoint uptown{40.760, -74.010, 0.0};
    CHECK(!sightings_contradict_by_transit(downtown, uptown, 30 * 60'000));
}

void apply_resolution_ignores_a_refused_result() {
    CASE("a caller that forgets to check cannot name an anonymous body");

    auto track = anon_track();
    const auto rafa = subject("rafa", SubjectKind::ConsentedPerson);

    ResolutionResult refused;
    refused.refusal = ResolutionRefusal::ConsentRefused;
    apply_resolution(track, rafa, refused);

    CHECK_BECAUSE(track.identity == TrackIdentity::Anonymous,
                  "apply_resolution is a no-op on a refusal, so a missing check cannot leak "
                  "a name onto a track");
}

}  // namespace

int main() {
    a_track_is_anonymous_and_still_counts();

    a_public_figure_is_never_resolved_from_a_camera();
    an_anonymous_subject_is_not_resolvable();

    without_consent_a_sighting_stays_anonymous();
    a_camera_that_is_not_authorized_resolves_nothing();
    a_live_but_unauthorized_sensor_resolves_nothing();

    consent_and_a_match_together_name_the_body();
    consent_without_enough_evidence_stays_anonymous();
    a_transit_time_contradiction_rules_the_candidate_out();
    a_location_contradiction_also_rules_out();

    the_kind_is_checked_before_consent_so_the_reason_is_actionable();
    a_cross_camera_match_over_an_impossible_distance_is_contradicted();
    apply_resolution_ignores_a_refused_result();

    return sentry::test::report();
}
