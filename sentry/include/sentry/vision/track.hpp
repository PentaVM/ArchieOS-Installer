// Person tracking — a body a camera saw, and the narrow path by which it may ever get a
// name.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Vision
//
// A vision model watching an authorized camera produces detections: there is a person here,
// at this place in frame, now. A tracker links consecutive detections into a track: the
// same body, moving, over a few seconds. None of that is identity. A track is anonymous by
// construction — it is counted for occupancy, it is a thing a robot stops for, and it has an
// ephemeral per-camera id that is not a person.
//
// The dangerous step, the one this file exists to gate, is turning a track into a name.
// RFC-008 §59: without identity consent, a camera sighting produces an anonymous track only,
// and Sentry does not resolve it to a known identity. §114: subject identity resolution is
// consent-gated. So resolution here is not a similarity threshold — it is the composition of
// three independent checks that already exist, and a track that fails any of them stays
// exactly what it was, a body with no name.
//
//   1. The candidate must be a *resolvable kind*. A public figure is not resolvable from a
//      camera however good the appearance match — being in the public record is not consent
//      to be recognised in the street (`subject.hpp`).
//   2. The operator ∩ subject ∩ sensor gate must permit it (`policy/consent.hpp`).
//   3. Only then does the evidence decide *how sure* — and a hard contradiction, two cameras
//      that cannot be walked between in the time available, defeats any amount of similarity.
//
// The tracker and the detector — the RTSP decode, the neural network — are not here. They
// are the parts that need a real camera and a GPU. This is the layer that decides what their
// output is allowed to become, and it is pure logic so that the rule is checkable anywhere.

#pragma once

#include <cstddef>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "sentry/camera/authority.hpp"
#include "sentry/identity/subject.hpp"
#include "sentry/policy/consent.hpp"
#include "sentry/world/geo.hpp"
#include "sentry/world/time.hpp"

namespace sentry::vision {

using sentry::identity::SubjectRecord;
using sentry::policy::AssociationEvidence;
using sentry::policy::ConsentLease;
using sentry::policy::IdentityAssociation;
using sentry::policy::OperatorPolicy;
using sentry::world::TimePoint;

// One thing a vision model saw in one frame: a person is here, roughly there in frame, now.
//
// Names nobody. The confidence is that this is a person at all, not who it is — conflating
// the two is the first step toward a face model's certainty standing in for consent.
struct Detection {
    std::string camera_id{};
    TimePoint at{};
    // Normalised frame position, 0..1. Where, not who.
    float cx = 0.0F;
    float cy = 0.0F;
    // How sure the detector is that this is a person.
    float person_confidence = 0.0F;
};

// How a track is currently held with respect to identity.
enum class TrackIdentity {
    // The default and the safe state. A body, counted and avoided, with no name.
    Anonymous,
    // Resolution was attempted and permitted, and the evidence supports an association.
    Resolved,
    // Resolution was attempted and actively ruled out — we looked and it is not them.
    // Distinct from Anonymous: this track will not be re-offered to the same candidate.
    Rejected,
};

[[nodiscard]] std::string_view name(TrackIdentity identity) noexcept;

// A sequence of detections believed to be one body within one camera.
//
// The id is ephemeral and per-camera. It is not a person, and it is deliberately not stable
// across cameras: making it stable is re-identification, which is the consent-gated step, not
// a property of the track.
struct Track {
    std::string track_id{};
    std::string camera_id{};
    TimePoint first_seen{};
    TimePoint last_seen{};
    std::size_t detection_count = 0;

    // The track's last known position in the frame, normalised 0..1. Where the body is,
    // not who it is — a tracker associates detections by proximity, and this is what it
    // compares against. Meaningful only while the track is live.
    float last_cx = 0.0F;
    float last_cy = 0.0F;

    TrackIdentity identity = TrackIdentity::Anonymous;
    // The strength of the association, when resolved. Meaningless while Anonymous.
    IdentityAssociation association = IdentityAssociation::Unknown;
    // The subject this track resolved to. Empty unless `identity == Resolved`.
    std::string subject_id{};

    // Whether this track counts toward occupancy and obstacle avoidance. Always true: an
    // anonymous body is still a body, and a robot that will not stop for someone it cannot
    // name is a worse outcome than one that stops for an anonymous obstacle.
    [[nodiscard]] bool counts_as_present() const noexcept { return detection_count > 0; }

    [[nodiscard]] bool is_named() const noexcept { return identity == TrackIdentity::Resolved; }
};

// Why a track was not resolved to the candidate. Each maps to a different fix, or to no fix
// at all, which is why it is an enum.
enum class ResolutionRefusal {
    // The candidate is a kind that is never resolvable from a sensor — a public figure, or
    // an already-anonymous subject. Not a permission question; being in the news is not
    // consent to be recognised.
    NotAResolvableKind,
    // The camera is not authorized to deliver video, so there is nothing legitimate to
    // resolve from.
    CameraNotAuthorized,
    // The operator ∩ subject ∩ sensor consent gate refused. The nested reason says which of
    // the three, so the fix lands in the right place.
    ConsentRefused,
    // Consent permits it, but the evidence does not support an association — or a hard
    // contradiction defeats it. The track stays anonymous, correctly.
    EvidenceInsufficient,
    // The evidence actively rules the candidate out. The track is marked Rejected.
    Contradicted,
};

[[nodiscard]] std::string_view describe(ResolutionRefusal refusal) noexcept;

// Everything needed to attempt a resolution, gathered so the function reaches across to
// nothing and can be tested against every combination.
struct ResolutionContext {
    // The camera's authority to be delivering video at all.
    sentry::camera::CameraAuthority camera_authority{};
    // Whether the camera is actually delivering — supplied, never inferred here, so sensor
    // health and authorization stay independent (§114).
    bool sensor_authorized_and_live = false;
    // The operator's policy and the candidate's consent lease.
    OperatorPolicy operator_policy{};
    std::optional<ConsentLease> lease{};
    // Where the sighting happened, for the lease's spatial scope.
    std::string location{};
};

// The result of attempting to resolve a track to a candidate.
struct ResolutionResult {
    // Present when the attempt failed. The track is left as it was (Anonymous), except for
    // `Contradicted`, which marks it Rejected.
    std::optional<ResolutionRefusal> refusal{};
    // When the attempt was refused because of the consent gate, the specific consent reason.
    std::optional<sentry::policy::ConsentRefusal> consent_detail{};
    // The association reached, when it succeeded.
    IdentityAssociation association = IdentityAssociation::Unknown;

    [[nodiscard]] bool resolved() const noexcept { return !refusal.has_value(); }
};

// Whether two sightings at two camera locations are too far apart to be the same person in
// the time between them.
//
// This is what should set `AssociationEvidence::contradicted_by_transit_time` when a
// re-identification spans two cameras — computed from geography and the clock, not set by
// hand. A cross-camera face match across an impossible distance is the case appearance
// matching is most confidently wrong about, and this is the check that catches it before it
// names anyone.
[[nodiscard]] bool sightings_contradict_by_transit(const sentry::world::GeoPoint& a,
                                                   const sentry::world::GeoPoint& b,
                                                   std::int64_t elapsed_ms) noexcept;

// Attempt to resolve a track to a named subject. The whole gate.
//
// Composes the three independent checks in order, and the order matters: kind before
// consent, because "a public figure is not resolvable" is a truth about the candidate that a
// consent form cannot change, and reporting `ConsentRefused` for it would send someone
// looking for a signature that should never be sought. Consent before evidence, because the
// evidence is a face model's opinion and it does not get to decide whether the rule applies.
//
// A refusal leaves the track anonymous — the §59 outcome — except `Contradicted`, which is a
// positive finding that this is *not* the candidate and marks the track Rejected so the same
// wrong guess is not made again.
[[nodiscard]] ResolutionResult resolve_track(const Track& track,
                                             const SubjectRecord& candidate,
                                             const AssociationEvidence& evidence,
                                             const ResolutionContext& context,
                                             TimePoint now) noexcept;

// Apply a successful resolution to a track. A no-op on a refused result, so a caller that
// forgets to check `resolved()` cannot accidentally name an anonymous body.
void apply_resolution(Track& track, const SubjectRecord& candidate,
                      const ResolutionResult& result) noexcept;

}  // namespace sentry::vision
