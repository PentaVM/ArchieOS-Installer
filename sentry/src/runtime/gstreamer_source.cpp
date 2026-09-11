// A real RTSP frame source, backed by GStreamer.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §7.1
//
// This is the hardware-bound half of the camera daemon: the RTSP handshake, the H.264 (or
// H.265) decode, the pull of decoded frames. It implements `FrameSource`, so the worker's
// orchestration — tested against a scripted source — drives this without change.
//
// It is compiled only when GStreamer is present (`-DSENTRY_WITH_GSTREAMER=ON`), so a build
// that has no camera libraries — the ISO build, this development host — is unaffected. The
// logic that governs it is tested; this file is validated on a machine that has a camera and
// libgstreamer, which is the only place it can be.
//
// # The credential never appears in a log or an argv
//
// RFC-008 §90: credentials live in a secret reference, never in git, a command line, or a
// log line. The RTSP URL with its embedded password is assembled here, held in this object,
// used to build the pipeline, and never printed — not in an error, not in a status. The
// status this reports upward is the connection *state*, which carries no secret.
//
// # It reports state, it does not decide policy
//
// A GStreamer bus error becomes an `ObservedConnectionState` — `AuthFailed` for a 401,
// `StreamStalled` for a decode timeout, `CodecUnsupported` for a caps negotiation failure —
// and hands it back on `poll`. What to *do* about that state is the worker's decision, driven
// by `camera/connection.hpp`. This file never reconnects on its own schedule and never
// decides a camera is unauthorized; it connects when told and reports what it sees.

#include "sentry/runtime/frame_source.hpp"

#if defined(SENTRY_WITH_GSTREAMER)

#include <gst/app/gstappsink.h>
#include <gst/gst.h>

#include <atomic>
#include <memory>
#include <mutex>
#include <string>
#include <vector>

namespace sentry::runtime {

namespace {

// Map a GStreamer bus message to the authority layer's vocabulary. The one place a decode
// or handshake failure is classified, so the worker never has to know GStreamer's error
// domains.
ObservedConnectionState classify_bus_error(const GError* error) noexcept {
    if (error == nullptr) {
        return ObservedConnectionState::StreamStalled;
    }
    // rtspsrc reports authentication failures in its own domain; the message text carries
    // the 401. We match conservatively: an auth failure is the one most dangerous to
    // misclassify, because reading it as anything else would loop on a password that will
    // never work, and reading a real network blip as auth-failed would be equally wrong.
    if (error->message != nullptr) {
        const std::string message = error->message;
        if (message.find("401") != std::string::npos ||
            message.find("Unauthorized") != std::string::npos) {
            return ObservedConnectionState::AuthFailed;
        }
        if (message.find("decode") != std::string::npos ||
            message.find("caps") != std::string::npos) {
            return ObservedConnectionState::CodecUnsupported;
        }
    }
    return ObservedConnectionState::StreamStalled;
}

}  // namespace

// An RTSP source for one camera. Owns its own GStreamer pipeline and the latest decoded
// frame; the decode runs on GStreamer's streaming thread and the newest frame is handed back
// on `poll` from the worker's thread, under a mutex, so the threading stays inside here.
class GStreamerRtspSource final : public FrameSource {
  public:
    // `rtsp_url` carries the credential. It is stored and never logged. `camera_id` is the
    // non-secret handle everything upward uses.
    GStreamerRtspSource(std::string camera_id, std::string rtsp_url)
        : camera_id_(std::move(camera_id)), rtsp_url_(std::move(rtsp_url)) {
        gst_init(nullptr, nullptr);
    }

    ~GStreamerRtspSource() override { teardown(); }

    GStreamerRtspSource(const GStreamerRtspSource&) = delete;
    GStreamerRtspSource& operator=(const GStreamerRtspSource&) = delete;

    void connect() override {
        std::lock_guard<std::mutex> lock(mutex_);
        if (pipeline_ != nullptr) {
            return;  // idempotent: already connected or connecting
        }

        // rtspsrc -> decodebin -> videoconvert -> appsink, decoding to RGB. `latency` is
        // kept low; `protocols=tcp` avoids the UDP hole-punching that fails behind NAT. The
        // URL — with its credential — is set as a property, never interpolated into a
        // launch string that might be logged.
        pipeline_ = gst_pipeline_new("archie-cam");
        GstElement* src = gst_element_factory_make("rtspsrc", "src");
        GstElement* depay = gst_element_factory_make("decodebin", "decode");
        GstElement* convert = gst_element_factory_make("videoconvert", "convert");
        GstElement* sink = gst_element_factory_make("appsink", "sink");
        if (pipeline_ == nullptr || src == nullptr || depay == nullptr || convert == nullptr ||
            sink == nullptr) {
            observed_.store(ObservedConnectionState::CodecUnsupported);
            teardown_locked();
            return;
        }

        g_object_set(src, "location", rtsp_url_.c_str(), "latency", 100, "protocols", 0x4,
                     nullptr);
        // Ask appsink for RGB so the detector reads a known layout.
        GstCaps* caps = gst_caps_new_simple("video/x-raw", "format", G_TYPE_STRING, "RGB",
                                            nullptr);
        g_object_set(sink, "caps", caps, "emit-signals", FALSE, "sync", FALSE, "max-buffers",
                     1, "drop", TRUE, nullptr);
        gst_caps_unref(caps);

        gst_bin_add_many(GST_BIN(pipeline_), src, depay, convert, sink, nullptr);
        gst_element_link_many(depay, convert, sink, nullptr);
        // rtspsrc's pads are dynamic; link them when they appear.
        g_signal_connect(src, "pad-added", G_CALLBACK(&GStreamerRtspSource::on_pad_added), depay);

        appsink_ = sink;
        bus_ = gst_element_get_bus(pipeline_);
        observed_.store(ObservedConnectionState::Connecting);
        gst_element_set_state(pipeline_, GST_STATE_PLAYING);
    }

    void disconnect() override {
        std::lock_guard<std::mutex> lock(mutex_);
        teardown_locked();
        observed_.store(ObservedConnectionState::Offline);
    }

    PollResult poll(TimePoint now) override {
        std::lock_guard<std::mutex> lock(mutex_);
        drain_bus_locked();

        PollResult result;
        result.observed = observed_.load();

        if (pipeline_ == nullptr || appsink_ == nullptr) {
            return result;
        }

        // Pull the newest frame if one is ready. `try_pull_sample` is non-blocking, so a
        // worker ticking many cameras is never stalled on one.
        GstSample* sample = gst_app_sink_try_pull_sample(GST_APP_SINK(appsink_), 0);
        if (sample == nullptr) {
            return result;
        }

        GstBuffer* buffer = gst_sample_get_buffer(sample);
        GstCaps* caps = gst_sample_get_caps(sample);
        GstMapInfo map;
        if (buffer != nullptr && caps != nullptr && gst_buffer_map(buffer, &map, GST_MAP_READ)) {
            GstStructure* s = gst_caps_get_structure(caps, 0);
            int width = 0;
            int height = 0;
            gst_structure_get_int(s, "width", &width);
            gst_structure_get_int(s, "height", &height);

            frame_bytes_.assign(map.data, map.data + map.size);
            gst_buffer_unmap(buffer, &map);

            observed_.store(ObservedConnectionState::Online);
            result.observed = ObservedConnectionState::Online;
            result.has_frame = true;
            result.frame.camera_id = camera_id_;
            result.frame.captured_at = now;
            result.frame.width = width;
            result.frame.height = height;
            result.frame.format = PixelFormat::Rgb8;
            result.frame.pixels = std::span<const std::uint8_t>(frame_bytes_.data(),
                                                                frame_bytes_.size());
        }
        gst_sample_unref(sample);
        return result;
    }

    std::string_view camera_id() const noexcept override { return camera_id_; }

  private:
    static void on_pad_added(GstElement*, GstPad* pad, gpointer decode) {
        GstElement* sink_element = static_cast<GstElement*>(decode);
        GstPad* sink_pad = gst_element_get_static_pad(sink_element, "sink");
        if (sink_pad != nullptr && gst_pad_is_linked(sink_pad) == FALSE) {
            gst_pad_link(pad, sink_pad);
        }
        if (sink_pad != nullptr) {
            gst_object_unref(sink_pad);
        }
    }

    void drain_bus_locked() {
        if (bus_ == nullptr) {
            return;
        }
        while (GstMessage* msg = gst_bus_pop(bus_)) {
            if (GST_MESSAGE_TYPE(msg) == GST_MESSAGE_ERROR) {
                GError* error = nullptr;
                gst_message_parse_error(msg, &error, nullptr);
                observed_.store(classify_bus_error(error));
                if (error != nullptr) {
                    g_error_free(error);  // and never logged — it may carry the URL
                }
            }
            gst_message_unref(msg);
        }
    }

    void teardown() {
        std::lock_guard<std::mutex> lock(mutex_);
        teardown_locked();
    }

    void teardown_locked() {
        if (pipeline_ != nullptr) {
            gst_element_set_state(pipeline_, GST_STATE_NULL);
        }
        if (bus_ != nullptr) {
            gst_object_unref(bus_);
            bus_ = nullptr;
        }
        if (pipeline_ != nullptr) {
            gst_object_unref(pipeline_);
            pipeline_ = nullptr;
        }
        appsink_ = nullptr;
    }

    std::string camera_id_;
    std::string rtsp_url_;  // holds the credential; never logged
    std::mutex mutex_;
    std::atomic<ObservedConnectionState> observed_{ObservedConnectionState::Offline};
    GstElement* pipeline_ = nullptr;
    GstElement* appsink_ = nullptr;
    GstBus* bus_ = nullptr;
    std::vector<std::uint8_t> frame_bytes_;
};

std::unique_ptr<FrameSource> make_rtsp_source(std::string camera_id, std::string rtsp_url) {
    return std::make_unique<GStreamerRtspSource>(std::move(camera_id), std::move(rtsp_url));
}

}  // namespace sentry::runtime

#endif  // SENTRY_WITH_GSTREAMER
