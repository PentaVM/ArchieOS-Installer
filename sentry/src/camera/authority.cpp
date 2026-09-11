#include "sentry/camera/authority.hpp"

namespace sentry::camera {

std::string_view name(AuthorityBasis basis) noexcept {
    switch (basis) {
        case AuthorityBasis::ArchieOwned:         return "archie_owned";
        case AuthorityBasis::OperatorAuthorized:  return "operator_authorized";
        case AuthorityBasis::IntentionallyPublic: return "intentionally_public";
        case AuthorityBasis::PublicProvider:      return "public_provider";
        case AuthorityBasis::DiscoveredOnly:      return "discovered_only";
        case AuthorityBasis::Blocked:             return "blocked";
    }
    return "unknown";
}

std::string_view name(ObservedConnectionState state) noexcept {
    switch (state) {
        case ObservedConnectionState::Offline:          return "offline";
        case ObservedConnectionState::Connecting:       return "connecting";
        case ObservedConnectionState::Online:           return "online";
        case ObservedConnectionState::Degraded:         return "degraded";
        case ObservedConnectionState::AuthFailed:       return "auth_failed";
        case ObservedConnectionState::StreamStalled:    return "stream_stalled";
        case ObservedConnectionState::CodecUnsupported: return "codec_unsupported";
        case ObservedConnectionState::Backoff:          return "backoff";
        case ObservedConnectionState::Recovering:       return "recovering";
    }
    return "unknown";
}

std::string_view describe(AuthorityRefusal refusal) noexcept {
    switch (refusal) {
        case AuthorityRefusal::OnlyOperatorMayDecide:
            return "camera authority is the operator's decision; automation may report a "
                   "camera as unreachable but may never deauthorize it";
        case AuthorityRefusal::CameraIsBlocked:
            return "the camera is blocked; unblocking is itself an operator action";
    }
    return "unknown refusal";
}

std::optional<AuthorityRefusal> refuse(const CameraAuthority& current,
                                       const AuthorityChange& change) noexcept {
    // The whole point of the header. No automated component — not the reconnect loop
    // that has failed forty times, not a detector, not a language model — may change
    // what the operator decided. Automation's only legitimate output about a camera is
    // an observed connection state.
    if (change.actor != AuthorityActor::Operator) {
        return AuthorityRefusal::OnlyOperatorMayDecide;
    }

    // Blocked is sticky against everything except explicitly unblocking it. An operator
    // can lift it, but not by sideways-promoting the camera to another basis in one
    // step, which would make an accidental promotion indistinguishable from a
    // deliberate one in the audit trail.
    if (current.basis == AuthorityBasis::Blocked &&
        change.new_basis != AuthorityBasis::Blocked &&
        change.new_basis != AuthorityBasis::DiscoveredOnly) {
        return AuthorityRefusal::CameraIsBlocked;
    }

    return std::nullopt;
}

CameraStatusLine status_line(const CameraAuthority& authority,
                             ObservedConnectionState observed,
                             TimePoint now) noexcept {
    CameraStatusLine line;
    line.authorized = authority.in_force_at(now);
    line.delivering = is_delivering(observed);

    if (authority.basis == AuthorityBasis::Blocked) {
        line.authority_reason = "blocked by operator";
    } else if (authority.expires_at && now >= *authority.expires_at) {
        line.authority_reason = "operator authorization expired";
    } else if (!permits_video(authority.basis)) {
        line.authority_reason =
            "discovered only; the operator has not authorized opening this stream";
    } else if (authority.desired == DesiredConnectionState::Disabled) {
        line.authority_reason = "authorized, but the operator has it disabled";
    } else {
        line.authority_reason = "authorized by operator";
    }

    // The connection half never rephrases the authority half. In particular AuthFailed
    // says whose credentials failed, because that sentence is the one an operator most
    // often misreads as "Archie decided to stop watching".
    switch (observed) {
        case ObservedConnectionState::AuthFailed:
            line.connection_reason =
                "the camera rejected our credentials; operator authority is unaffected";
            break;
        case ObservedConnectionState::CodecUnsupported:
            line.connection_reason =
                "the stream's codec cannot be decoded here; retrying will not fix it";
            break;
        case ObservedConnectionState::StreamStalled:
            line.connection_reason = "connected but no frames are arriving";
            break;
        case ObservedConnectionState::Backoff:
            line.connection_reason = "waiting out a backoff before retrying";
            break;
        case ObservedConnectionState::Recovering:
            line.connection_reason = "reconnecting";
            break;
        case ObservedConnectionState::Connecting:
            line.connection_reason = "connecting";
            break;
        case ObservedConnectionState::Offline:
            line.connection_reason = "unreachable";
            break;
        case ObservedConnectionState::Degraded:
            line.connection_reason = "delivering frames, below expected rate";
            break;
        case ObservedConnectionState::Online:
            line.connection_reason = "delivering frames";
            break;
    }
    return line;
}

}  // namespace sentry::camera
