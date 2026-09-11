#include "sentry/vision/track.hpp"

namespace sentry::vision {

using sentry::identity::resolvable;
using sentry::policy::ConsentScope;
using sentry::policy::IdentityRequest;

std::string_view name(TrackIdentity identity) noexcept {
    switch (identity) {
        case TrackIdentity::Anonymous: return "anonymous";
        case TrackIdentity::Resolved:  return "resolved";
        case TrackIdentity::Rejected:  return "rejected";
    }
    return "unknown";
}

std::string_view describe(ResolutionRefusal refusal) noexcept {
    switch (refusal) {
        case ResolutionRefusal::NotAResolvableKind:
            return "this kind of subject is never resolved from a camera; being in the "
                   "public record is not consent to be recognised in the street";
        case ResolutionRefusal::CameraNotAuthorized:
            return "the camera is not authorized to deliver video, so there is nothing "
                   "legitimate to resolve an identity from";
        case ResolutionRefusal::ConsentRefused:
            return "the operator, subject or sensor authority does not permit resolving "
                   "this track to this subject";
        case ResolutionRefusal::EvidenceInsufficient:
            return "consent permits it, but the evidence does not support an association; "
                   "the track stays anonymous, which is the correct outcome";
        case ResolutionRefusal::Contradicted:
            return "the evidence rules this candidate out — a location or transit-time "
                   "contradiction defeats any amount of appearance similarity";
    }
    return "unknown refusal";
}

bool sightings_contradict_by_transit(const sentry::world::GeoPoint& a,
                                     const sentry::world::GeoPoint& b,
                                     std::int64_t elapsed_ms) noexcept {
    // A contradiction is precisely an implausible trip. The geo layer errs toward abstaining
    // — a fast train is not called impossible — so a true here is a real impossibility, not
    // a borderline call, which is the right threshold for something that rules a person out.
    return !sentry::world::transit_plausible(a, b, elapsed_ms);
}

ResolutionResult resolve_track(const Track& track,
                               const SubjectRecord& candidate,
                               const AssociationEvidence& evidence,
                               const ResolutionContext& context,
                               TimePoint now) noexcept {
    ResolutionResult result;

    // The track itself carries no bearing on whether resolution is permitted — that is
    // entirely the candidate, the consent and the evidence. It is in the signature because a
    // resolution is always about one specific track, and a caller passing the wrong one is a
    // bug worth making visible at the call site rather than hiding behind a free function.
    (void)track;

    // 1. Kind, before anything else. A public figure or an already-anonymous subject is not
    //    resolvable from a sensor, and no consent form changes that. Checked first so this
    //    truth about the candidate is never mistaken for a missing signature.
    if (!resolvable(candidate.kind)) {
        result.refusal = ResolutionRefusal::NotAResolvableKind;
        return result;
    }

    // 2a. The camera's own authority to be delivering video. A stream nobody authorized is
    //     not a source Sentry resolves identities from, whatever the pixels show.
    if (!context.camera_authority.may_open_stream_at(now) ||
        !context.sensor_authorized_and_live) {
        result.refusal = ResolutionRefusal::CameraNotAuthorized;
        return result;
    }

    // 2b. The operator ∩ subject ∩ sensor consent gate. Resolving a camera track to a person
    //     is a `CameraIdentity` operation; biometrics would be its own scope with its own
    //     lease. The sensor-authorized bit is passed through, never recomputed here, so the
    //     two authorities stay independent (§114).
    IdentityRequest request;
    request.subject_id = candidate.subject_id;
    request.scope = ConsentScope::CameraIdentity;
    request.location = context.location;
    request.sensor_authorized = context.sensor_authorized_and_live;
    request.at = now;

    if (const auto refusal =
            sentry::policy::refuse(context.operator_policy, context.lease, request)) {
        result.refusal = ResolutionRefusal::ConsentRefused;
        result.consent_detail = refusal;
        return result;
    }

    // 3. Only now does the evidence decide how sure. Consent has already said this
    //    resolution is permitted; the face model's opinion never got to make that call.
    const auto association = sentry::policy::resolve(evidence);
    result.association = association;

    switch (association) {
        case IdentityAssociation::Rejected:
            // A positive finding: this is not them. Marks the track Rejected so the same
            // wrong guess is not made again, unlike an insufficient-evidence anonymous track
            // that may resolve later with more.
            result.refusal = ResolutionRefusal::Contradicted;
            return result;
        case IdentityAssociation::Unknown:
        case IdentityAssociation::Candidate:
            // Not enough to name a person. The track stays anonymous — the §59 outcome —
            // and may resolve later if stronger evidence arrives.
            result.refusal = ResolutionRefusal::EvidenceInsufficient;
            return result;
        case IdentityAssociation::Likely:
        case IdentityAssociation::Corroborated:
        case IdentityAssociation::Confirmed:
            // Permitted, and supported. This is the only path that names a body.
            return result;
    }
    result.refusal = ResolutionRefusal::EvidenceInsufficient;
    return result;
}

void apply_resolution(Track& track, const SubjectRecord& candidate,
                      const ResolutionResult& result) noexcept {
    if (result.refusal == ResolutionRefusal::Contradicted) {
        // A ruling-out is applied even though `resolved()` is false: it is real information
        // about this track, and recording it is what stops the same wrong candidate being
        // tried again.
        track.identity = TrackIdentity::Rejected;
        track.association = IdentityAssociation::Rejected;
        track.subject_id.clear();
        return;
    }
    if (!result.resolved()) {
        // Every other refusal leaves the track exactly as it was. A caller that forgot to
        // check cannot name an anonymous body by calling this.
        return;
    }
    track.identity = TrackIdentity::Resolved;
    track.association = result.association;
    track.subject_id = candidate.subject_id;
}

}  // namespace sentry::vision
