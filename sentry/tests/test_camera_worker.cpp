// The camera worker's orchestration, with a scripted source and detector in place of a real
// camera and GPU.
//
// The point of the seam is exactly this: the connect/retry/decode/detect/track sequence is
// logic, and logic is tested. The cases below are about the sequence — a guest camera holds
// no tracks, a stream that fails backs off, a person who leaves the frame expires, a
// low-confidence flicker never becomes a body — none of which needs a pixel.

#include "harness.hpp"

#include "sentry/runtime/camera_worker.hpp"

#include <deque>
#include <memory>

using namespace sentry::runtime;
using sentry::camera::AuthorityBasis;
using sentry::camera::BackoffSchedule;
using sentry::camera::CameraAuthority;
using sentry::camera::ConnectionAction;
using sentry::camera::DesiredConnectionState;
using sentry::camera::ObservedConnectionState;
using sentry::vision::Detection;
using sentry::world::TimePoint;

namespace {

TimePoint at(std::int64_t ms) { return TimePoint{ms}; }

// A source that replays a script of poll results, and records connect/disconnect calls.
class ScriptedSource : public FrameSource {
  public:
    explicit ScriptedSource(std::string camera_id, std::deque<PollResult> script)
        : camera_id_(std::move(camera_id)), script_(std::move(script)) {}

    void connect() override { ++connects; }
    void disconnect() override { ++disconnects; }

    PollResult poll(TimePoint) override {
        if (script_.empty()) {
            return PollResult{ObservedConnectionState::Offline, false, {}};
        }
        auto next = script_.front();
        script_.pop_front();
        return next;
    }

    std::string_view camera_id() const noexcept override { return camera_id_; }

    int connects = 0;
    int disconnects = 0;

  private:
    std::string camera_id_;
    std::deque<PollResult> script_;
};

// A detector that returns a fixed set of detections for every frame, above threshold.
class ScriptedDetector : public PersonDetector {
  public:
    explicit ScriptedDetector(std::vector<Detection> detections)
        : detections_(std::move(detections)) {}

    DetectionSet detect(const Frame&) override {
        DetectionSet set;
        set.detections = detections_;
        return set;
    }
    float minimum_confidence() const noexcept override { return 0.5F; }
    std::string_view model_name() const noexcept override { return "scripted"; }

    std::vector<Detection> detections_;
};

Detection person(float cx, float cy, float confidence = 0.9F) {
    Detection d;
    d.camera_id = "cam1";
    d.cx = cx;
    d.cy = cy;
    d.person_confidence = confidence;
    return d;
}

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

PollResult delivering(float count_frames) {
    (void)count_frames;
    PollResult p;
    p.observed = ObservedConnectionState::Online;
    p.has_frame = true;
    p.frame.camera_id = "cam1";
    return p;
}

std::unique_ptr<ScriptedSource> source(std::deque<PollResult> script) {
    return std::make_unique<ScriptedSource>("cam1", std::move(script));
}

// ---------------------------------------------------------------- authority drives the source

void an_authorized_camera_connects_and_tracks() {
    CASE("a body in front of an authorized camera becomes an anonymous track");

    auto src = source({delivering(1)});
    auto* raw = src.get();
    CameraWorker worker(authorized(), std::move(src),
                        std::make_shared<ScriptedDetector>(std::vector{person(0.5F, 0.5F)}),
                        schedule());

    const auto report = worker.tick(at(100));
    CHECK_EQ(report.action, ConnectionAction::Connect);
    CHECK(raw->connects >= 1);
    CHECK(report.frame_decoded);
    CHECK_EQ(report.detections, std::size_t{1});
    CHECK_EQ(report.live_tracks, std::size_t{1});

    CHECK_BECAUSE(!worker.tracks().empty() &&
                      worker.tracks()[0].identity == sentry::vision::TrackIdentity::Anonymous,
                  "a track the worker produces is always anonymous; naming is a separate, "
                  "consent-gated step");
}

void a_disabled_camera_holds_no_tracks_and_disconnects() {
    CASE("the operator disabling a camera stops the video and clears the tracks");

    auto disabled = authorized();
    disabled.desired = DesiredConnectionState::Disabled;

    auto src = source({delivering(1)});
    auto* raw = src.get();
    CameraWorker worker(disabled, std::move(src),
                        std::make_shared<ScriptedDetector>(std::vector{person(0.5F, 0.5F)}),
                        schedule());

    const auto report = worker.tick(at(100));
    CHECK_EQ(report.action, ConnectionAction::StayDisabled);
    CHECK_BECAUSE(raw->disconnects >= 1,
                  "a disabled camera's stream is torn down, credential and all, not left "
                  "open");
    CHECK(worker.tracks().empty());
    CHECK(!report.frame_decoded);
}

void an_unauthorized_camera_never_looks() {
    CASE("a discovered-only camera produces no tracks");

    auto found_only = authorized();
    found_only.basis = AuthorityBasis::DiscoveredOnly;

    auto src = source({delivering(1)});
    CameraWorker worker(found_only, std::move(src),
                        std::make_shared<ScriptedDetector>(std::vector{person(0.5F, 0.5F)}),
                        schedule());

    const auto report = worker.tick(at(100));
    CHECK_EQ(report.action, ConnectionAction::NotAuthorized);
    CHECK(worker.tracks().empty());
}

// ---------------------------------------------------------------- tracking

void detections_near_each_other_continue_one_track() {
    CASE("a person moving a little is one track, not many");

    std::deque<PollResult> script;
    for (int i = 0; i < 3; ++i) {
        script.push_back(delivering(1));
    }
    // The detector returns a person drifting slightly each frame.
    auto detector = std::make_shared<ScriptedDetector>(std::vector{person(0.50F, 0.50F)});
    CameraWorker worker(authorized(), source(std::move(script)), detector, schedule());

    worker.tick(at(100));
    detector->detections_ = {person(0.52F, 0.51F)};
    worker.tick(at(200));
    detector->detections_ = {person(0.54F, 0.52F)};
    worker.tick(at(300));

    CHECK_BECAUSE(worker.tracks().size() == 1,
                  "three nearby detections are one moving body, associated by proximity");
    CHECK_EQ(worker.tracks()[0].detection_count, std::size_t{3});
}

void a_far_detection_starts_a_new_track() {
    CASE("two people are two tracks");

    auto detector = std::make_shared<ScriptedDetector>(
        std::vector{person(0.2F, 0.2F), person(0.8F, 0.8F)});
    CameraWorker worker(authorized(), source({delivering(1)}), detector, schedule());

    worker.tick(at(100));
    CHECK_EQ(worker.tracks().size(), std::size_t{2});
}

void a_person_who_leaves_the_frame_expires() {
    CASE("a track nobody has seen lately stops being present");

    auto detector = std::make_shared<ScriptedDetector>(std::vector{person(0.5F, 0.5F)});
    std::deque<PollResult> script{delivering(1)};
    // Then the person is gone: delivering frames, but no detections.
    for (int i = 0; i < 3; ++i) {
        script.push_back(delivering(1));
    }
    CameraWorker worker(authorized(), source(std::move(script)), detector, schedule());

    worker.tick(at(100));
    CHECK_EQ(worker.tracks().size(), std::size_t{1});

    detector->detections_.clear();
    worker.tick(at(1'000));
    CHECK_BECAUSE(worker.tracks().size() == 1, "still within the staleness window");

    const auto report = worker.tick(at(5'000));  // past staleness_ms = 3000
    CHECK_BECAUSE(worker.tracks().empty(),
                  "a person who left the frame is no longer counted present");
    CHECK_EQ(report.expired_tracks, std::size_t{1});
}

void a_low_confidence_flicker_never_becomes_a_body() {
    CASE("noise below the detector's floor spawns nothing");

    auto detector = std::make_shared<ScriptedDetector>(std::vector{person(0.5F, 0.5F, 0.2F)});
    CameraWorker worker(authorized(), source({delivering(1)}), detector, schedule());

    const auto report = worker.tick(at(100));
    CHECK_EQ(report.detections, std::size_t{1});  // the detector reported it
    CHECK_BECAUSE(worker.tracks().empty(),
                  "but below the confidence floor it never becomes a track");
}

// ---------------------------------------------------------------- connection failures

void a_failing_stream_backs_off_then_retries() {
    CASE("a camera that will not connect is retried, on a schedule");

    std::deque<PollResult> script;
    for (int i = 0; i < 4; ++i) {
        script.push_back(PollResult{ObservedConnectionState::Offline, false, {}});
    }
    auto src = source(std::move(script));
    auto* raw = src.get();
    CameraWorker worker(authorized(), std::move(src),
                        std::make_shared<ScriptedDetector>(std::vector<Detection>{}), schedule());

    // First tick: connect, fails.
    worker.tick(at(0));
    CHECK_EQ(worker.connection().consecutive_failures, 1);

    // Immediately after, it should be holding backoff, not hammering connect.
    const auto held = worker.tick(at(1));
    CHECK_BECAUSE(held.action == ConnectionAction::HoldBackoff,
                  "a failed camera is not reconnected on the very next tick");
    const int connects_during_backoff = raw->connects;

    // After the backoff interval, it retries.
    const auto retry = worker.tick(at(2'000));
    CHECK_EQ(retry.action, ConnectionAction::RetryNow);
    CHECK_BECAUSE(raw->connects > connects_during_backoff,
                  "once the interval elapses, it actually reconnects");
}

void a_steady_offline_does_not_inflate_the_failure_count() {
    CASE("backing off is not the same as failing again");

    std::deque<PollResult> script;
    for (int i = 0; i < 10; ++i) {
        script.push_back(PollResult{ObservedConnectionState::Offline, false, {}});
    }
    CameraWorker worker(authorized(), source(std::move(script)),
                        std::make_shared<ScriptedDetector>(std::vector<Detection>{}), schedule());

    worker.tick(at(0));  // connect, fail: 1
    worker.tick(at(1));  // hold backoff
    worker.tick(at(2));  // hold backoff
    CHECK_BECAUSE(worker.connection().consecutive_failures == 1,
                  "ticks spent waiting out the backoff must not each count as a fresh "
                  "failure, or the backoff races to its ceiling for no reason");
}

}  // namespace

int main() {
    an_authorized_camera_connects_and_tracks();
    a_disabled_camera_holds_no_tracks_and_disconnects();
    an_unauthorized_camera_never_looks();

    detections_near_each_other_continue_one_track();
    a_far_detection_starts_a_new_track();
    a_person_who_leaves_the_frame_expires();
    a_low_confidence_flicker_never_becomes_a_body();

    a_failing_stream_backs_off_then_retries();
    a_steady_offline_does_not_inflate_the_failure_count();

    return sentry::test::report();
}
