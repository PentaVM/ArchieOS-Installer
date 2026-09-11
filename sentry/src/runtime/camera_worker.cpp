#include "sentry/runtime/camera_worker.hpp"

#include <algorithm>
#include <cmath>

namespace sentry::runtime {

std::string_view name(PixelFormat format) noexcept {
    switch (format) {
        case PixelFormat::Rgb8:  return "rgb8";
        case PixelFormat::Bgr8:  return "bgr8";
        case PixelFormat::Gray8: return "gray8";
    }
    return "unknown";
}

CameraWorker::CameraWorker(CameraAuthority authority, std::unique_ptr<FrameSource> source,
                           std::shared_ptr<PersonDetector> detector, BackoffSchedule backoff,
                           TrackerConfig tracker)
    : authority_(std::move(authority)),
      source_(std::move(source)),
      detector_(std::move(detector)),
      backoff_(backoff),
      tracker_(tracker) {
    connection_.camera_id = authority_.camera_id;
}

void CameraWorker::record_failure(TimePoint now) noexcept {
    ++connection_.consecutive_failures;
    connection_.last_attempt_at = now;
}

void CameraWorker::record_success(TimePoint now) noexcept {
    // A delivering stream resets the failure count, so a camera that works and then fails
    // once retries fast rather than starting from the slow end of the backoff.
    connection_.consecutive_failures = 0;
    connection_.last_frame_at = now;
}

TickReport CameraWorker::tick(TimePoint now) {
    TickReport report;

    // 1. What does policy say to do? The worker never decides this itself.
    const auto action =
        sentry::camera::decide_action(authority_, connection_, backoff_, now);
    report.action = action;

    switch (action) {
        case ConnectionAction::NotAuthorized:
        case ConnectionAction::StayDisabled:
        case ConnectionAction::ReportForHuman:
            // In none of these should a stream be held open. Tearing it down here is what
            // makes "the operator disabled this camera" actually stop the video, rather than
            // leaving a socket the credential is still flowing through.
            source_->disconnect();
            // A disabled or unauthorized camera has no tracks in front of it that Sentry may
            // claim to see. Clearing them is the honest state: we are not looking.
            report.expired_tracks += tracks_.size();
            tracks_.clear();
            return report;

        case ConnectionAction::Connect:
        case ConnectionAction::RetryNow:
            source_->connect();
            break;

        case ConnectionAction::HoldBackoff:
        case ConnectionAction::AwaitInFlight:
            // Nothing to do but wait. Still expire stale tracks below, so a camera that
            // stopped delivering does not keep reporting phantom bodies.
            break;

        case ConnectionAction::Hold:
        case ConnectionAction::HoldDegraded:
            break;
    }

    // 2. Poll the source. Non-blocking; it returns whatever it has now.
    const auto poll = source_->poll(now);
    connection_.observed = poll.observed;

    if (sentry::camera::is_delivering(poll.observed)) {
        // Delivering — even degraded — counts as alive for the lost check and the backoff
        // reset, whether or not this particular tick carried a new frame.
        record_success(now);
    } else if (poll.observed == ObservedConnectionState::Offline ||
               poll.observed == ObservedConnectionState::AuthFailed ||
               poll.observed == ObservedConnectionState::StreamStalled) {
        // A failure state, but only count it as a new failure at the moment we were trying —
        // a steady Offline while backing off must not inflate the count every tick and push
        // the backoff to its ceiling for no reason.
        if (action == ConnectionAction::Connect || action == ConnectionAction::RetryNow) {
            record_failure(now);
        }
    }

    // 3. A frame arrived. Detect the people in it.
    if (poll.has_frame) {
        report.frame_decoded = true;
        const auto found = detector_->detect(poll.frame);
        report.detections = found.detections.size();
        ingest_detections(found, now);
    }

    // 4. Expire tracks nobody has seen lately, always — a person who left the frame stops
    //    being present whether or not a new frame arrived this tick.
    report.expired_tracks += expire_stale(now);
    report.live_tracks = tracks_.size();
    report.camera_lost = sentry::camera::is_lost(authority_, connection_, backoff_, now);

    return report;
}

void CameraWorker::ingest_detections(const DetectionSet& found, TimePoint now) {
    const float threshold = detector_->minimum_confidence();

    for (const auto& detection : found.detections) {
        if (detection.person_confidence < threshold) {
            // Below the detector's own floor. Discarded before it can become a track, so a
            // flicker of low-confidence noise does not spawn a phantom body.
            continue;
        }

        // Nearest-centroid association within this camera. The closest live track inside the
        // association radius continues; otherwise a new anonymous track is born.
        Track* best = nullptr;
        float best_distance = tracker_.association_radius;
        for (auto& track : tracks_) {
            // A resolved or rejected track keeps its identity, but its position still
            // updates — association is by proximity, and identity is a separate axis.
            const float dx = detection.cx - track.last_cx;
            const float dy = detection.cy - track.last_cy;
            const float distance = std::sqrt(dx * dx + dy * dy);
            if (distance < best_distance) {
                best_distance = distance;
                best = &track;
            }
        }

        if (best != nullptr) {
            best->last_seen = now;
            ++best->detection_count;
            best->last_cx = detection.cx;
            best->last_cy = detection.cy;
        } else {
            Track track;
            track.track_id = authority_.camera_id + "-t" + std::to_string(next_track_seq_++);
            track.camera_id = authority_.camera_id;
            track.first_seen = now;
            track.last_seen = now;
            track.detection_count = 1;
            track.last_cx = detection.cx;
            track.last_cy = detection.cy;
            tracks_.push_back(std::move(track));
        }
    }
}

std::size_t CameraWorker::expire_stale(TimePoint now) {
    const std::size_t before = tracks_.size();
    std::erase_if(tracks_, [&](const Track& track) {
        return track.last_seen.to(now) > tracker_.staleness_ms;
    });
    return before - tracks_.size();
}

}  // namespace sentry::runtime
