// The camera worker — one camera, driven from authority to anonymous tracks.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §7.1
//
// This is the orchestration the whole camera daemon is built from, and it is pure logic so it
// is testable without a camera or a GPU. Per tick it:
//
//   1. asks `camera/connection.hpp` what to do, given the authority, the connection state and
//      the clock;
//   2. drives the `FrameSource` accordingly — connect, disconnect, or pull a frame;
//   3. runs the `PersonDetector` on a frame that arrived;
//   4. links the detections into anonymous tracks and expires the stale ones.
//
// What it deliberately does not do is name anyone. Every track it produces is anonymous —
// `vision/track.hpp` is where a track may, consent permitting, be resolved to a person, and
// that is a separate step with a separate authority. The worker holds a camera's credentials
// and produces bodies; it never produces names.
//
// The tracker is intentionally simple: nearest-centroid association within one camera, with a
// staleness timeout. Cross-camera association is re-identification, which is consent-gated and
// not a property of a single camera's worker.

#pragma once

#include <cstddef>
#include <memory>
#include <string>
#include <vector>

#include "sentry/camera/authority.hpp"
#include "sentry/camera/connection.hpp"
#include "sentry/runtime/detector.hpp"
#include "sentry/runtime/frame_source.hpp"
#include "sentry/vision/track.hpp"

namespace sentry::runtime {

using sentry::camera::BackoffSchedule;
using sentry::camera::CameraAuthority;
using sentry::camera::ConnectionAction;
using sentry::camera::ConnectionState;
using sentry::vision::Track;

// What one tick of the worker did, for the daemon's log and for tests.
struct TickReport {
    ConnectionAction action{};
    bool frame_decoded = false;
    std::size_t detections = 0;
    // Tracks that are currently live (seen recently enough not to be stale).
    std::size_t live_tracks = 0;
    // Tracks expired this tick for going stale.
    std::size_t expired_tracks = 0;
    // True when the camera is authorized and wanted but has delivered nothing for too long.
    bool camera_lost = false;
};

// How the tracker behaves. Defaults are deliberate and documented at the point of use.
struct TrackerConfig {
    // A detection within this normalised distance of a track's last centroid continues that
    // track; beyond it, a new track is born. In frame-fraction units (0..1), so it is
    // resolution-independent.
    float association_radius = 0.12F;
    // A track that has not been seen for this long is expired. Long enough to survive a
    // missed frame or two; short enough that a person who left is not counted present.
    std::int64_t staleness_ms = 3'000;
};

// One camera, from its authority to the anonymous tracks in front of it.
class CameraWorker {
  public:
    CameraWorker(CameraAuthority authority, std::unique_ptr<FrameSource> source,
                 std::shared_ptr<PersonDetector> detector, BackoffSchedule backoff,
                 TrackerConfig tracker = {});

    // Advance the worker one tick.
    TickReport tick(TimePoint now);

    // The current authority. Replaced by the daemon when the operator changes it; the worker
    // never changes it itself.
    void set_authority(const CameraAuthority& authority) noexcept { authority_ = authority; }
    [[nodiscard]] const CameraAuthority& authority() const noexcept { return authority_; }

    // The anonymous tracks currently in front of the camera.
    [[nodiscard]] const std::vector<Track>& tracks() const noexcept { return tracks_; }

    [[nodiscard]] const ConnectionState& connection() const noexcept { return connection_; }

  private:
    void record_failure(TimePoint now) noexcept;
    void record_success(TimePoint now) noexcept;
    void ingest_detections(const DetectionSet& found, TimePoint now);
    std::size_t expire_stale(TimePoint now);

    CameraAuthority authority_;
    std::unique_ptr<FrameSource> source_;
    std::shared_ptr<PersonDetector> detector_;
    BackoffSchedule backoff_;
    TrackerConfig tracker_;

    ConnectionState connection_;
    std::vector<Track> tracks_;
    std::uint64_t next_track_seq_ = 1;
};

}  // namespace sentry::runtime
