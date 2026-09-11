// A source of video frames — the seam between the camera daemon's logic and a real RTSP
// stream.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §7.1
//
// Everything the camera worker does — drive the connection, pull frames, hand them to a
// detector, build tracks — is orchestration, and orchestration is testable. What is not
// testable here is the RTSP handshake and the H.264 decode, which need a real camera. So
// they sit behind this interface: a `GStreamerRtspSource` implements it against libgstreamer
// on a machine that has one, and a `FakeFrameSource` implements it against a script, so the
// worker's logic is exercised without a camera in the loop.
//
// The interface is deliberately narrow. A source connects, it reports what the connection is
// doing in the vocabulary the authority layer already speaks, and it hands over a decoded
// frame when one is ready. It holds the credentials and the socket; it makes no decision
// about whether it *should* be connected — that is the worker's job, driven by
// `camera/connection.hpp`, so the one component that holds a camera's password is not also
// the one that decides policy.

#pragma once

#include <cstdint>
#include <span>
#include <string>
#include <string_view>

#include "sentry/camera/authority.hpp"
#include "sentry/world/time.hpp"

namespace sentry::runtime {

using sentry::camera::ObservedConnectionState;
using sentry::world::TimePoint;

// How a decoded frame's bytes are laid out. The detector reads this to interpret the buffer;
// the worker never looks at a pixel.
enum class PixelFormat {
    // 8-bit RGB, tightly packed, row-major. The common denominator most detectors want.
    Rgb8,
    // 8-bit BGR, what OpenCV hands back by default.
    Bgr8,
    // Single-channel grayscale.
    Gray8,
};

[[nodiscard]] std::string_view name(PixelFormat format) noexcept;

// One decoded frame. Owned by the source until the next `poll`; the detector must consume it
// synchronously rather than retain the span, because the source is free to reuse the buffer.
struct Frame {
    std::string camera_id{};
    TimePoint captured_at{};
    int width = 0;
    int height = 0;
    PixelFormat format = PixelFormat::Rgb8;
    // The decoded pixels. Empty in a test that only exercises orchestration; a real source
    // fills it and a real detector reads it.
    std::span<const std::uint8_t> pixels{};

    [[nodiscard]] bool has_pixels() const noexcept { return !pixels.empty(); }
};

// What a poll of the source produced.
struct PollResult {
    // What the connection is doing, in the authority layer's own vocabulary, so the worker
    // can feed it straight into `decide_action` without translation.
    ObservedConnectionState observed = ObservedConnectionState::Offline;
    // True when `frame` below holds a newly decoded frame. False for every non-delivering
    // observed state, and also for a delivering state that simply had no new frame this
    // tick.
    bool has_frame = false;
    Frame frame{};
};

// A video source for one camera. Implementations hold the credentials and the socket.
//
// Not thread-safe by contract: the worker polls one source from one thread. A source that
// decodes on a background thread hands the latest frame back on `poll`, so the threading
// stays inside the implementation and out of the orchestration.
class FrameSource {
  public:
    virtual ~FrameSource() = default;

    // Begin connecting. Idempotent: calling it while already connected is a no-op, not a
    // reconnect, so the worker deciding "Connect" on a source that is already up costs
    // nothing.
    virtual void connect() = 0;

    // Tear the connection down. Called when authority is withdrawn or the operator disables
    // the camera — the credential and the socket go with it.
    virtual void disconnect() = 0;

    // Advance the source and report what happened. Non-blocking: it returns whatever the
    // source has right now, so a worker ticking many cameras is never stalled on one.
    [[nodiscard]] virtual PollResult poll(TimePoint now) = 0;

    [[nodiscard]] virtual std::string_view camera_id() const noexcept = 0;
};

}  // namespace sentry::runtime
