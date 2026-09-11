// Factories for the hardware-backed source and detector.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §7.1
//
// Declared unconditionally so the daemon can name them; defined only under the matching build
// guard. A build without GStreamer or ONNX Runtime does not link the daemon that calls them,
// so the missing definition is never a missing symbol — the pure library and its tests do not
// reference these at all.

#pragma once

#include <memory>
#include <string>

#include "sentry/runtime/detector.hpp"
#include "sentry/runtime/frame_source.hpp"

namespace sentry::runtime {

#if defined(SENTRY_WITH_GSTREAMER)
// Build an RTSP source. `rtsp_url` carries the credential and is never logged; `camera_id`
// is the non-secret handle. Defined in `gstreamer_source.cpp`.
[[nodiscard]] std::unique_ptr<FrameSource> make_rtsp_source(std::string camera_id,
                                                            std::string rtsp_url);
#endif

#if defined(SENTRY_WITH_ONNX)
// Build a person detector from an ONNX detection model. Defined in `onnx_detector.cpp`.
[[nodiscard]] std::shared_ptr<PersonDetector> make_onnx_detector(const std::string& model_path,
                                                                 float confidence_floor);
#endif

}  // namespace sentry::runtime
