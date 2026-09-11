// Camera authority — the operator decides, and nothing else gets a vote.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Camera authority
//
// Two things about a camera are constantly confused, and the confusion produces the
// worst failure mode a surveillance-capable system can have: a camera the operator
// authorized quietly stops working and nothing says why, or worse, a model decides it
// should not be watching and silently stops.
//
//     AUTHORITY   may Sentry look at this camera at all?    decided by the operator
//     HEALTH      can Sentry reach it right now?            decided by the network
//
// These are independent axes and this header keeps them that way in the type system.
// There is no function here that takes a health state and returns an authority, because
// no such function should exist.
//
// The rule, stated once:
//
//     A camera the operator marked authorized stays authorized until the operator says
//     otherwise. A timeout is not a revocation. An expired password is not a
//     revocation. A model's opinion is not a revocation.

#pragma once

#include <optional>
#include <string>
#include <string_view>

#include "sentry/world/time.hpp"

namespace sentry::camera {

using sentry::world::TimePoint;

// On what grounds Sentry may look at this camera.
enum class AuthorityBasis {
    // Archie's own hardware.
    ArchieOwned,
    // Someone else's camera the operator has asserted authority over.
    OperatorAuthorized,
    // A camera whose owner publishes it deliberately — a harbour cam, a summit cam.
    IntentionallyPublic,
    // Reached through a provider that licenses it, under that provider's terms.
    PublicProvider,
    // Seen in an Internet-wide scan and nothing more. This is cyber intelligence about
    // an exposed service, not permission to open the stream. Sentry records that it
    // exists and does not fetch a single pixel.
    DiscoveredOnly,
    // Explicitly forbidden. Survives any later attempt to promote it.
    Blocked,
};

[[nodiscard]] std::string_view name(AuthorityBasis basis) noexcept;

// Whether this basis permits opening the video stream.
//
// `DiscoveredOnly` is the important false: finding a camera-like service on a scanning
// provider tells you a device is exposed, which is a security finding about its owner,
// not consent from them.
[[nodiscard]] constexpr bool permits_video(AuthorityBasis basis) noexcept {
    return basis == AuthorityBasis::ArchieOwned ||
           basis == AuthorityBasis::OperatorAuthorized ||
           basis == AuthorityBasis::IntentionallyPublic ||
           basis == AuthorityBasis::PublicProvider;
}

// What the operator wants the connection to be doing.
enum class DesiredConnectionState {
    Connected,
    Disabled,
};

// What the connection is actually doing. None of these values may alter authority.
enum class ObservedConnectionState {
    Offline,
    Connecting,
    Online,
    Degraded,
    // The *remote* device rejected our credentials. This is the one most likely to be
    // misread as a permission problem on our side. It is not: it means the camera's own
    // password is wrong or was rotated, and the operator's authority is untouched.
    AuthFailed,
    StreamStalled,
    CodecUnsupported,
    Backoff,
    Recovering,
};

[[nodiscard]] std::string_view name(ObservedConnectionState state) noexcept;

// Whether frames are currently arriving.
[[nodiscard]] constexpr bool is_delivering(ObservedConnectionState state) noexcept {
    return state == ObservedConnectionState::Online ||
           state == ObservedConnectionState::Degraded;
}

// Whether this failure is worth retrying on a timer.
//
// `CodecUnsupported` is not: retrying a stream we cannot decode produces an identical
// failure forever and buries the real problem in reconnect noise. It needs a human or a
// profile change, so it backs off to a slow probe rather than a hot loop.
[[nodiscard]] constexpr bool warrants_retry(ObservedConnectionState state) noexcept {
    return state != ObservedConnectionState::CodecUnsupported;
}

// Who asked for a change of authority.
enum class AuthorityActor {
    Operator,
    // Any automated component: a detector, the fusion engine, a language model, the
    // reconnect loop. Deliberately one value, because the answer is the same for all of
    // them and splitting it would invite an exception.
    Automation,
};

// Why an authority change was refused.
enum class AuthorityRefusal {
    // Automation tried to change authority. It never may.
    OnlyOperatorMayDecide,
    // A blocked camera cannot be promoted by anything short of unblocking it first,
    // which is itself an operator action.
    CameraIsBlocked,
};

[[nodiscard]] std::string_view describe(AuthorityRefusal refusal) noexcept;

// The operator's standing decision about one camera.
struct CameraAuthority {
    std::string camera_id{};
    AuthorityBasis basis = AuthorityBasis::DiscoveredOnly;
    DesiredConnectionState desired = DesiredConnectionState::Disabled;
    // When the operator made this decision, for the audit trail.
    TimePoint granted_at{};
    // Optional expiry. Absent means standing — the common case, and deliberately the
    // default, because a camera that silently deauthorizes itself at an unremembered
    // moment is exactly the failure this design exists to prevent.
    std::optional<TimePoint> expires_at{};
    // Free-text record of what the operator asserted, e.g. a work order or a written
    // authorization from the camera's owner.
    std::string evidence{};

    // Whether the authority itself is currently in force.
    //
    // Depends only on the operator's decision and the clock. It takes no connection
    // state, and it cannot: there is no parameter through which a network failure could
    // reach this answer.
    [[nodiscard]] bool in_force_at(TimePoint now) const noexcept {
        if (basis == AuthorityBasis::Blocked) {
            return false;
        }
        // A grant cannot make a capture from before the operator's decision
        // retrospectively permissible.
        if (now < granted_at) {
            return false;
        }
        if (expires_at && now >= *expires_at) {
            return false;
        }
        return true;
    }

    // Whether Sentry may open this camera's stream right now.
    [[nodiscard]] bool may_open_stream_at(TimePoint now) const noexcept {
        return in_force_at(now) && permits_video(basis) &&
               desired == DesiredConnectionState::Connected;
    }
};

// A proposed change to a camera's authority.
struct AuthorityChange {
    AuthorityActor actor{};
    AuthorityBasis new_basis{};
    DesiredConnectionState new_desired{};
};

// Whether the change is permitted, and if not, why.
[[nodiscard]] std::optional<AuthorityRefusal> refuse(const CameraAuthority& current,
                                                     const AuthorityChange& change) noexcept;

// What an operator should be told about a camera that is authorized but not delivering.
//
// The two halves are reported separately and always in this order, because an operator
// reading "camera down" needs to know immediately that it is still authorized — that
// the system has not silently decided to stop watching.
struct CameraStatusLine {
    bool authorized = false;
    bool delivering = false;
    std::string_view authority_reason{};
    std::string_view connection_reason{};
};

[[nodiscard]] CameraStatusLine status_line(const CameraAuthority& authority,
                                           ObservedConnectionState observed,
                                           TimePoint now) noexcept;

}  // namespace sentry::camera
