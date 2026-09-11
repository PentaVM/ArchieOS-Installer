// A person detector — the seam between the camera daemon's logic and a neural network.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §7.1
//
// A detector answers one question about a frame: where are the people? Not who — where. It
// returns boxes and a confidence that each is a person, and nothing about identity, because
// a detector that returned names would be a face recogniser wearing a detector's interface,
// and naming is the consent-gated step that happens later in `vision/track.hpp`.
//
// The real implementation runs a model — YOLO, an SSD, whatever — on a GPU. That needs a GPU
// and a model file, so it sits behind this interface: an `OnnxDetector` implements it where
// those exist, a `FakeDetector` returns a script so the worker's logic is testable without
// either.

#pragma once

#include <cstddef>
#include <string_view>
#include <vector>

#include "sentry/runtime/frame_source.hpp"
#include "sentry/vision/track.hpp"

namespace sentry::runtime {

using sentry::vision::Detection;

// What the detector found in one frame: zero or more people, each a box and a confidence.
// Never a name.
struct DetectionSet {
    std::vector<Detection> detections{};
    // How long the inference took, so the worker can back off a detector that is falling
    // behind rather than queueing frames it will never catch up on.
    std::int64_t inference_ms = 0;
};

// A person detector.
//
// Stateless by contract: each frame is detected independently. Tracking — linking one frame's
// detections to the last — is the worker's job, not the detector's, so a swap of the model
// does not change how tracks are formed.
class PersonDetector {
  public:
    virtual ~PersonDetector() = default;

    // Find the people in a frame. A frame with no pixels (an orchestration test) yields no
    // detections from a real detector and whatever the fake was scripted to return.
    [[nodiscard]] virtual DetectionSet detect(const Frame& frame) = 0;

    // The confidence below which a detection is discarded before it ever becomes a track. A
    // property of the detector because it is the detector that knows its own score
    // distribution; the worker reads it rather than hard-coding a threshold that only suits
    // one model.
    [[nodiscard]] virtual float minimum_confidence() const noexcept = 0;

    [[nodiscard]] virtual std::string_view model_name() const noexcept = 0;
};

}  // namespace sentry::runtime
