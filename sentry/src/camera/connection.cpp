#include "sentry/camera/connection.hpp"

namespace sentry::camera {

std::string_view name(ConnectionAction action) noexcept {
    switch (action) {
        case ConnectionAction::Connect:        return "connect";
        case ConnectionAction::AwaitInFlight:  return "await_in_flight";
        case ConnectionAction::RetryNow:       return "retry_now";
        case ConnectionAction::HoldBackoff:    return "hold_backoff";
        case ConnectionAction::Hold:           return "hold";
        case ConnectionAction::HoldDegraded:   return "hold_degraded";
        case ConnectionAction::ReportForHuman: return "report_for_human";
        case ConnectionAction::StayDisabled:   return "stay_disabled";
        case ConnectionAction::NotAuthorized:  return "not_authorized";
    }
    return "unknown";
}

std::int64_t BackoffSchedule::delay_after(int failures) const noexcept {
    if (failures <= 0) {
        return 0;
    }
    // Exponential, but computed by doubling rather than by shifting so a large failure count
    // cannot overflow the shift: after enough doublings the value simply stops rising once
    // it passes the ceiling, and the loop exits.
    std::int64_t delay = base_ms;
    for (int i = 1; i < failures; ++i) {
        if (delay >= ceiling_ms) {
            return ceiling_ms;
        }
        delay *= 2;
    }
    return delay < ceiling_ms ? delay : ceiling_ms;
}

ConnectionAction decide_action(const CameraAuthority& authority,
                               const ConnectionState& state,
                               const BackoffSchedule& backoff,
                               TimePoint now) noexcept {
    // Authority first, and it is a report, not a judgement about failure. A camera the
    // operator never authorized is not connected — but this is never reached *because* a
    // connection failed, and it never changes as a result of one.
    if (authority.basis == AuthorityBasis::Blocked || !authority.in_force_at(now) ||
        !permits_video(authority.basis)) {
        return ConnectionAction::NotAuthorized;
    }

    // The operator's own choice to disable is the only way an authorized camera is left
    // unconnected. Distinct from every failure state below.
    if (authority.desired == DesiredConnectionState::Disabled) {
        return ConnectionAction::StayDisabled;
    }

    switch (state.observed) {
        case ObservedConnectionState::Online:
            return ConnectionAction::Hold;
        case ObservedConnectionState::Degraded:
            // Impaired frames are better than none. Held, not dropped, and watched.
            return ConnectionAction::HoldDegraded;
        case ObservedConnectionState::Connecting:
        case ObservedConnectionState::Recovering:
            return ConnectionAction::AwaitInFlight;

        case ObservedConnectionState::CodecUnsupported:
            // The one failure a retry cannot fix. Surfaced for a human rather than looped
            // on — and pointedly *not* disabled, because the camera is still authorized and
            // a profile change may yet make it decodable.
            return ConnectionAction::ReportForHuman;

        case ObservedConnectionState::Offline:
        case ObservedConnectionState::AuthFailed:
        case ObservedConnectionState::StreamStalled:
        case ObservedConnectionState::Backoff: {
            // Everything here is a retryable failure, `AuthFailed` included: the camera's
            // own credential is wrong or was rotated, which is fixed by reconnecting with
            // the right one, not by deciding the operator withdrew authority.
            if (state.consecutive_failures <= 0) {
                return ConnectionAction::Connect;
            }
            const auto wait = backoff.delay_after(state.consecutive_failures);
            const auto elapsed = state.last_attempt_at.to(now);
            if (elapsed >= wait) {
                return ConnectionAction::RetryNow;
            }
            return ConnectionAction::HoldBackoff;
        }
    }

    // An observed state added later has no decision here, so the safe answer is to wait for
    // an in-flight attempt rather than to invent a connect or a give-up.
    return ConnectionAction::AwaitInFlight;
}

bool is_lost(const CameraAuthority& authority, const ConnectionState& state,
             const BackoffSchedule& backoff, TimePoint now) noexcept {
    // Only an authorized, wanted camera can be "lost" — a disabled or unauthorized camera
    // delivering nothing is expected, not lost.
    if (!authority.may_open_stream_at(now)) {
        return false;
    }
    if (is_delivering(state.observed)) {
        return false;
    }
    // Twelve ceiling-intervals of no frames. Long enough that a slow reconnect is not
    // maligned as dead; short enough to surface a truly dead camera within minutes.
    const auto silent_for = state.last_frame_at.to(now);
    return silent_for > backoff.ceiling_ms * 12;
}

}  // namespace sentry::camera
