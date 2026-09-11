// `sentry-camd` — the camera daemon.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §7.1
//
// The process that holds camera credentials, decodes their streams, and produces anonymous
// tracks. It is the `sentry-camd` the RFC's process-boundary table anticipates: it holds the
// secrets and is restricted to registered endpoints, and it produces bodies, never names —
// resolving a track to a person is a separate authority in a separate process.
//
// This is built only when GStreamer and ONNX Runtime are present, because without them there
// is no camera to serve. Its logic — every `CameraWorker` — is tested; this file is the thin
// wiring that gives each worker a real RTSP source and a real detector, plus the loop that
// ticks them, and it is validated on a machine with a camera and a GPU.
//
// # Credentials come from the environment of a hardened unit, never from a config file in git
//
// Each camera's RTSP URL, password and all, is read from an environment variable named by its
// id — `ARCHIE_CAM_<id>_URL` — which systemd populates from a credential store
// (`LoadCredential=`). It is never in this repository, never on the command line, never
// logged. The daemon reads it once, hands it to the source, and forgets it.

#include "sentry/runtime/camera_worker.hpp"
#include "sentry/runtime/factory.hpp"

#include <cstdio>

#if defined(SENTRY_WITH_GSTREAMER) && defined(SENTRY_WITH_ONNX)

#include <chrono>
#include <cstdlib>
#include <string>
#include <thread>
#include <vector>

#include "sentry/camera/authority.hpp"
#include "sentry/world/time.hpp"

namespace {

using namespace sentry::runtime;
using sentry::camera::AuthorityBasis;
using sentry::camera::BackoffSchedule;
using sentry::camera::CameraAuthority;
using sentry::camera::DesiredConnectionState;
using sentry::world::TimePoint;

// A monotonic millisecond clock for the worker's TimePoint. Wall-clock is not needed here —
// the worker measures intervals, not dates.
TimePoint now_ms() {
    const auto since = std::chrono::steady_clock::now().time_since_epoch();
    return TimePoint{std::chrono::duration_cast<std::chrono::milliseconds>(since).count()};
}

// Read a camera's RTSP URL from the credential environment. Returns empty when unset — a
// camera with no credential is simply not served, rather than the daemon guessing a URL.
std::string credential_url_for(const std::string& camera_id) {
    const std::string var = "ARCHIE_CAM_" + camera_id + "_URL";
    if (const char* value = std::getenv(var.c_str())) {
        return value;
    }
    return {};
}

CameraAuthority operator_authorized(const std::string& camera_id) {
    CameraAuthority authority;
    authority.camera_id = camera_id;
    authority.basis = AuthorityBasis::OperatorAuthorized;
    authority.granted_at = now_ms();
    authority.desired = DesiredConnectionState::Connected;
    return authority;
}

}  // namespace

int main(int argc, char** argv) {
    // The cameras to serve are named on the command line by id; their credentials come from
    // the environment. Nothing secret is an argument.
    if (argc < 3) {
        std::fputs("usage: sentry-camd <model.onnx> <camera-id> [<camera-id> ...]\n", stderr);
        return 64;  // EX_USAGE
    }

    const std::string model_path = argv[1];
    auto detector = make_onnx_detector(model_path, 0.4F);

    BackoffSchedule backoff;

    std::vector<CameraWorker> workers;
    for (int i = 2; i < argc; ++i) {
        const std::string camera_id = argv[i];
        const std::string url = credential_url_for(camera_id);
        if (url.empty()) {
            std::fprintf(stderr,
                         "sentry-camd: no credential for %s (set ARCHIE_CAM_%s_URL); skipping\n",
                         camera_id.c_str(), camera_id.c_str());
            continue;
        }
        // The URL — credential and all — goes straight into the source and is not retained
        // here.
        workers.emplace_back(operator_authorized(camera_id),
                             make_rtsp_source(camera_id, url), detector, backoff);
    }

    if (workers.empty()) {
        std::fputs("sentry-camd: no cameras with credentials to serve\n", stderr);
        return 69;  // EX_UNAVAILABLE
    }

    std::fprintf(stderr, "sentry-camd: serving %zu camera(s)\n", workers.size());

    // The tick loop. Each pass advances every worker once, then sleeps briefly. A real
    // deployment would surface the tracks over the daemon's IPC; here the loop is the
    // supervision, and the tracks are readable per worker.
    for (;;) {
        const auto now = now_ms();
        for (auto& worker : workers) {
            const auto report = worker.tick(now);
            if (report.camera_lost) {
                std::fprintf(stderr, "sentry-camd: %s has delivered nothing for too long\n",
                             std::string{worker.authority().camera_id}.c_str());
            }
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(33));  // ~30 fps
    }
}

#else  // no camera libraries

int main() {
    std::fputs("sentry-camd was built without GStreamer and ONNX Runtime; it cannot serve a "
               "camera. Rebuild with -DSENTRY_WITH_GSTREAMER=ON -DSENTRY_WITH_ONNX=ON on a "
               "machine that has them.\n",
               stderr);
    return 69;  // EX_UNAVAILABLE
}

#endif
