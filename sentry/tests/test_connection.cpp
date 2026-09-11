// Camera connection control, and the §113 line it must not cross.
//
// The supervisor reconnects; it never revokes. Every case here is either the backoff maths
// or one of the ways a connection problem could be mistaken for an authority problem — an
// auth-failed handshake, a camera gone silent, a codec that will not decode — and the rule
// that keeps each a connection problem.

#include "harness.hpp"

#include "sentry/camera/connection.hpp"

using namespace sentry::camera;
using sentry::world::TimePoint;

namespace {

TimePoint at(std::int64_t ms) { return TimePoint{ms}; }

CameraAuthority authorized() {
    CameraAuthority a;
    a.camera_id = "cam1";
    a.basis = AuthorityBasis::OperatorAuthorized;
    a.granted_at = at(0);
    a.desired = DesiredConnectionState::Connected;
    return a;
}

BackoffSchedule schedule() {
    BackoffSchedule b;
    b.base_ms = 1'000;
    b.ceiling_ms = 60'000;
    return b;
}

ConnectionState in_state(ObservedConnectionState observed, int failures) {
    ConnectionState s;
    s.camera_id = "cam1";
    s.observed = observed;
    s.consecutive_failures = failures;
    s.last_attempt_at = at(0);
    s.last_frame_at = at(0);
    return s;
}

// ---------------------------------------------------------------- backoff maths

void backoff_is_exponential_and_bounded() {
    CASE("floor, doubling, ceiling");

    const auto b = schedule();
    CHECK_EQ(b.delay_after(0), std::int64_t{0});
    CHECK_EQ(b.delay_after(1), std::int64_t{1'000});
    CHECK_EQ(b.delay_after(2), std::int64_t{2'000});
    CHECK_EQ(b.delay_after(3), std::int64_t{4'000});
    CHECK_EQ(b.delay_after(4), std::int64_t{8'000});

    // Capped, and — the point — never zero and never unbounded. An authorized camera down
    // for an hour is still retried every ceiling, which is what keeps back-off from becoming
    // give-up.
    CHECK_EQ(b.delay_after(100), std::int64_t{60'000});
    CHECK_BECAUSE(b.delay_after(1'000'000) == 60'000,
                  "a huge failure count must not overflow into a tiny or negative delay");
}

// ---------------------------------------------------------------- authority is not failure

void an_auth_failed_handshake_is_retried_not_treated_as_revocation() {
    CASE("§113: the camera's own password being wrong is not the operator withdrawing");

    // The state most likely to be misread. AuthFailed means the camera rejected our
    // credentials — rotated password, wrong secret — which is fixed by reconnecting, not by
    // deciding authority is gone.
    const auto action = decide_action(authorized(), in_state(ObservedConnectionState::AuthFailed, 0),
                                      schedule(), at(1));
    CHECK_BECAUSE(action == ConnectionAction::Connect,
                  "an auth-failed camera is reconnected; it is never marked unauthorized");
}

void a_silent_camera_is_reported_lost_but_stays_authorized() {
    CASE("lost is a report, not a disabling");

    auto state = in_state(ObservedConnectionState::Offline, 3);
    state.last_frame_at = at(0);
    // Long past the lost threshold.
    const auto now = at(60'000LL * 12 + 1);

    CHECK(is_lost(authorized(), state, schedule(), now));
    // And it is still being retried, not given up on.
    const auto action = decide_action(authorized(), state, schedule(), now);
    CHECK_BECAUSE(action == ConnectionAction::RetryNow || action == ConnectionAction::HoldBackoff,
                  "a lost camera is still authorized and still retried; lost surfaces it for "
                  "attention, it does not disable it");
}

void an_undecodable_codec_is_surfaced_not_looped_and_not_disabled() {
    CASE("the one non-retryable failure is reported, never revoked");

    const auto action = decide_action(authorized(),
                                      in_state(ObservedConnectionState::CodecUnsupported, 5),
                                      schedule(), at(1));
    CHECK_BECAUSE(action == ConnectionAction::ReportForHuman,
                  "retrying a stream we cannot decode loops forever; it needs a human, and "
                  "the camera stays authorized while it waits for one");
}

// ---------------------------------------------------------------- backoff behaviour

void a_first_failure_connects_immediately() {
    CASE("a camera that worked and failed once retries fast");

    const auto action = decide_action(authorized(), in_state(ObservedConnectionState::Offline, 0),
                                      schedule(), at(1));
    CHECK_EQ(action, ConnectionAction::Connect);
}

void backoff_holds_until_the_interval_elapses() {
    CASE("do not hammer a camera that is down");

    auto state = in_state(ObservedConnectionState::Offline, 3);  // 4000ms delay
    state.last_attempt_at = at(1'000);

    // 2000ms later — not yet.
    CHECK_EQ(decide_action(authorized(), state, schedule(), at(3'000)),
             ConnectionAction::HoldBackoff);
    // 4000ms later — now.
    CHECK_EQ(decide_action(authorized(), state, schedule(), at(5'000)),
             ConnectionAction::RetryNow);
}

// ---------------------------------------------------------------- delivering states

void an_online_stream_is_held() {
    CASE("keep a working stream");
    CHECK_EQ(decide_action(authorized(), in_state(ObservedConnectionState::Online, 0),
                           schedule(), at(1)),
             ConnectionAction::Hold);
}

void a_degraded_stream_is_kept_not_dropped() {
    CASE("impaired frames are better than none");
    CHECK_EQ(decide_action(authorized(), in_state(ObservedConnectionState::Degraded, 0),
                           schedule(), at(1)),
             ConnectionAction::HoldDegraded);
    // And a delivering-but-degraded camera is not lost.
    auto state = in_state(ObservedConnectionState::Degraded, 0);
    CHECK(!is_lost(authorized(), state, schedule(), at(60'000LL * 100)));
}

void an_in_flight_attempt_is_awaited() {
    CASE("do not start a second connect while one is running");
    CHECK_EQ(decide_action(authorized(), in_state(ObservedConnectionState::Connecting, 1),
                           schedule(), at(1)),
             ConnectionAction::AwaitInFlight);
}

// ---------------------------------------------------------------- authority gates

void an_unauthorized_camera_is_not_connected() {
    CASE("authority is a separate question, reported not failed");

    auto found_only = authorized();
    found_only.basis = AuthorityBasis::DiscoveredOnly;
    CHECK_EQ(decide_action(found_only, in_state(ObservedConnectionState::Offline, 0), schedule(),
                           at(1)),
             ConnectionAction::NotAuthorized);
    CHECK_BECAUSE(!is_lost(found_only, in_state(ObservedConnectionState::Offline, 5), schedule(),
                           at(60'000LL * 100)),
                  "an unauthorized camera delivering nothing is expected, not lost");
}

void a_disabled_camera_stays_disabled_by_operator_choice() {
    CASE("the only way an authorized camera is left unconnected");

    auto disabled = authorized();
    disabled.desired = DesiredConnectionState::Disabled;
    CHECK_EQ(decide_action(disabled, in_state(ObservedConnectionState::Offline, 0), schedule(),
                           at(1)),
             ConnectionAction::StayDisabled);
    CHECK_BECAUSE(!is_lost(disabled, in_state(ObservedConnectionState::Offline, 5), schedule(),
                           at(60'000LL * 100)),
                  "a camera the operator disabled is not lost; it is off on purpose");
}

}  // namespace

int main() {
    backoff_is_exponential_and_bounded();

    an_auth_failed_handshake_is_retried_not_treated_as_revocation();
    a_silent_camera_is_reported_lost_but_stays_authorized();
    an_undecodable_codec_is_surfaced_not_looped_and_not_disabled();

    a_first_failure_connects_immediately();
    backoff_holds_until_the_interval_elapses();

    an_online_stream_is_held();
    a_degraded_stream_is_kept_not_dropped();
    an_in_flight_attempt_is_awaited();

    an_unauthorized_camera_is_not_connected();
    a_disabled_camera_stays_disabled_by_operator_choice();

    return sentry::test::report();
}
