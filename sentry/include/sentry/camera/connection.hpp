// Camera connection control — deciding what to do about a stream over time, without ever
// deciding it is unauthorized.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Camera
//
// `authority.hpp` says who may look at a camera and what the connection is doing right now.
// This says what to do next: connect, retry, back off, hold, or report the camera lost. It
// is the deterministic half of "control de cámaras" — the socket, the RTSP handshake, the
// decoder are the other half, and they need real hardware.
//
// Two rules from RFC-008 §113 live here, because this is the layer most likely to break
// them:
//
//   - A connection failure is a connection failure, never a consent failure. A camera that
//     stopped answering is not a camera the operator stopped authorizing. This code
//     reconnects; it never revokes, and it never treats an auth-failed handshake (the
//     camera's own password is wrong) as the operator withdrawing permission.
//   - An operator-authorized camera is never silently disabled. Backoff slows retries; it
//     does not give up. The one state that stops retrying — a codec we cannot decode — is
//     reported loudly for a human, not dropped.

#pragma once

#include <cstdint>
#include <string>
#include <string_view>

#include "sentry/camera/authority.hpp"
#include "sentry/world/time.hpp"

namespace sentry::camera {

using sentry::world::TimePoint;

// What the supervisor decides to do next about one camera.
enum class ConnectionAction {
    // Open the stream now: authorized, wanted, and nothing is in the way.
    Connect,
    // A connection attempt is in flight; wait for it to resolve.
    AwaitInFlight,
    // Retry now — a backoff interval has elapsed.
    RetryNow,
    // Wait: a backoff interval is still running.
    HoldBackoff,
    // The stream is up. Keep it, and keep watching.
    Hold,
    // The stream is up but impaired. Keep it — degraded frames are better than none — and
    // watch for it to recover or fail outright.
    HoldDegraded,
    // Stop trying on a timer and surface it for a human. Reserved for failures that a retry
    // cannot fix, like an undecodable codec. Never means "unauthorized".
    ReportForHuman,
    // The operator has disabled this camera. Do not connect. This is the only path to not
    // connecting an authorized-and-wanted camera, and it is an operator decision, not a
    // consequence of failure.
    StayDisabled,
    // The camera is not authorized. Do not connect. Also not a failure — authority is a
    // separate question this action simply reports.
    NotAuthorized,
};

[[nodiscard]] std::string_view name(ConnectionAction action) noexcept;

// The backoff schedule. Bounded on both ends: a floor so a flapping camera does not become
// a hot reconnect loop, a ceiling so an authorized camera that has been down for an hour is
// still retried every ceiling-interval rather than every several hours. The ceiling is what
// keeps "back off" from becoming "give up".
struct BackoffSchedule {
    std::int64_t base_ms = 1'000;
    std::int64_t ceiling_ms = 60'000;
    // How many consecutive failures before the ceiling is reached, for documentation; the
    // maths caps regardless.
    int steps_to_ceiling = 6;

    // The delay after `failures` consecutive failures. Exponential, capped at the ceiling.
    [[nodiscard]] std::int64_t delay_after(int failures) const noexcept;
};

// What the supervisor knows about one camera's connection.
struct ConnectionState {
    std::string camera_id{};
    ObservedConnectionState observed = ObservedConnectionState::Offline;
    // Consecutive failed attempts since the last success. Reset to zero on a delivering
    // stream, so a camera that works and then fails once retries fast, not slowly.
    int consecutive_failures = 0;
    // When the current backoff interval began. Meaningful only while backing off.
    TimePoint last_attempt_at{};
    // When frames last arrived. Distinguishes "degraded but alive" from "gone", and is how
    // "lost" is decided: an authorized camera delivering nothing for long enough is lost —
    // reported, still authorized, still retried.
    TimePoint last_frame_at{};
};

// Decide the next action for a camera.
//
// Pure: it reads the authority, the observed state and the clock, and returns a decision.
// It performs no I/O and changes no authority — a supervisor that could revoke on failure
// would be the §113 violation this is written to prevent.
[[nodiscard]] ConnectionAction decide_action(const CameraAuthority& authority,
                                             const ConnectionState& state,
                                             const BackoffSchedule& backoff,
                                             TimePoint now) noexcept;

// How long an authorized camera may deliver nothing before it is reported lost. Reported,
// not disabled: a lost camera is still retried and still authorized. Twelve times the
// backoff ceiling — long enough that a slow reconnect is not called lost, short enough that
// a truly dead camera is surfaced within minutes.
[[nodiscard]] bool is_lost(const CameraAuthority& authority, const ConnectionState& state,
                           const BackoffSchedule& backoff, TimePoint now) noexcept;

}  // namespace sentry::camera
