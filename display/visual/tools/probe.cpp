// `archie-visual-probe` — run the observatory's decisions without a screen.
//
// The visual model can be tested on a machine with no GPU, but a passing test tells you
// a rule held; it does not let you *look* at what the observatory would do. This walks a
// scripted scenario and prints each frame in full: the tier and why, the camera, every
// node with its colour, detail and whether it is allowed to move, and the events.
//
// It exists for two reasons. It is a diagnostic — on a node whose display will not start,
// this still answers "what would it be showing?" — and it is a way to review the model's
// behaviour end to end without waiting for hardware that can render it.
//
// The scenario is fixed and the model has no clock or randomness, so two runs produce
// byte-identical output. That makes it usable as a regression baseline.

#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "archie/visual/observatory.hpp"

using namespace archie::visual;

namespace {

const char* mode_name(SceneMode mode) {
    switch (mode) {
        case SceneMode::Cerebra:
            return "Cerebra";
        case SceneMode::Brain:
            return "Brain";
        case SceneMode::Watcher:
            return "Watcher";
        case SceneMode::Fabric:
            return "Fabric";
    }
    return "?";
}

const char* tier_name(RenderTier tier) {
    switch (tier) {
        case RenderTier::Ultra:
            return "ULTRA";
        case RenderTier::Normal:
            return "NORMAL";
        case RenderTier::Conserve:
            return "CONSERVE";
        case RenderTier::Headless:
            return "HEADLESS";
    }
    return "?";
}

const char* reason_name(TierReason reason) {
    switch (reason) {
        case TierReason::DisplayAbsent:
            return "no display";
        case TierReason::ThermalCritical:
            return "thermal critical";
        case TierReason::ThermalThrottling:
            return "thermal throttling";
        case TierReason::ThermalWarm:
            return "chassis warm";
        case TierReason::PressureSaturated:
            return "cpu saturated";
        case TierReason::PressureContended:
            return "cpu contended";
        case TierReason::PressureLight:
            return "light load";
        case TierReason::PressureIdle:
            return "idle";
        case TierReason::TelemetryUnavailable:
            return "telemetry unavailable";
        case TierReason::TelemetryStale:
            return "telemetry stale";
    }
    return "?";
}

const char* freshness_name(Freshness freshness) {
    switch (freshness) {
        case Freshness::Live:
            return "live";
        case Freshness::Lagging:
            return "lagging";
        case Freshness::Stale:
            return "stale";
        case Freshness::Expired:
            return "expired";
    }
    return "?";
}

const char* detail_name(DetailLevel detail) {
    switch (detail) {
        case DetailLevel::Silhouette:
            return "silhouette";
        case DetailLevel::Simplified:
            return "simplified";
        case DetailLevel::Full:
            return "full";
        case DetailLevel::Inspect:
            return "inspect";
    }
    return "?";
}

const char* shape_name(NodeShape shape) {
    switch (shape) {
        case NodeShape::Cube:
            return "cube";
        case NodeShape::Sphere:
            return "sphere";
        case NodeShape::Octahedron:
            return "octahedron";
        case NodeShape::Ring:
            return "ring";
        case NodeShape::Cone:
            return "cone";
        case NodeShape::Capsule:
            return "capsule";
        case NodeShape::Placeholder:
            return "placeholder";
    }
    return "?";
}

const char* presence_mark(NodePresence presence) {
    switch (presence) {
        case NodePresence::Entering:
            return "+";
        case NodePresence::Present:
            return " ";
        case NodePresence::Departing:
            return "-";
    }
    return "?";
}

const char* camera_name(CameraState state) {
    switch (state) {
        case CameraState::Resting:
            return "resting";
        case CameraState::Transitioning:
            return "moving";
        case CameraState::Focused:
            return "focused";
        case CameraState::Returning:
            return "returning";
    }
    return "?";
}

const char* event_name(EventKind kind) {
    switch (kind) {
        case EventKind::Appeared:
            return "appeared";
        case EventKind::Departed:
            return "departed";
        case EventKind::Returned:
            return "returned";
        case EventKind::Removed:
            return "removed";
        case EventKind::SeverityChanged:
            return "severity";
        case EventKind::FreshnessChanged:
            return "freshness";
    }
    return "?";
}

/// Snap values that are a rounding error away from zero.
///
/// A camera passing through the origin produces coordinates like -0.0001, and printf
/// renders that as "-0.0" on one C library and "0.0" on another. That difference is
/// invisible on screen and fatal to using this output as a regression baseline, so it is
/// removed here rather than argued about later.
double shown(float value) {
    const double v = static_cast<double>(value);
    return (v > -0.05 && v < 0.05) ? 0.0 : v;
}

Value module(const std::string& id, const std::string& state,
             std::vector<Value> dependencies = {}) {
    return Value::object({
        member("id", Value{id}),
        member("state", Value{state}),
        member("dependencies", Value::array(std::move(dependencies))),
    });
}

Value kernel(const char* verdict, const char* thermal) {
    return Value::object({
        member("schema_version", Value{1}),
        member("pressure_availability", Value::object({member("state", Value{"available"})})),
        member("verdict", Value{verdict}),
        member("thermal_availability", Value::object({member("state", Value{"available"})})),
        member("thermal_state", Value{thermal}),
    });
}

Value observation(const std::string& id, const char* grade, double magnitude, double latitude,
                  double longitude, bool authoritative) {
    return Value::object({
        member("id", Value{id}),
        member("position", Value::object({member("latitude", Value{latitude}),
                                          member("longitude", Value{longitude})})),
        member("summary", Value{id}),
        member("details", Value::object({member("kind", Value{"earthquake"}),
                                         member("magnitude", Value{magnitude}),
                                         member("severity", Value{grade}),
                                         member("tsunami_warning", Value{false})})),
        member("provenance", Value::object({member("authority", Value{"USGS"}),
                                            member("authoritative", Value{authoritative})})),
    });
}

Value world_batch() {
    return Value::object(
        {member("schema_version", Value{1}),
         member("observations",
                Value::array({
                    observation("us.tokyo", "strong", 6.4, 35.68, 139.69, true),
                    observation("us.andes", "moderate", 5.1, -33.45, -70.66, true),
                    observation("us.aleutian", "major", 8.1, 51.88, -176.65, true),
                    observation("relay.iceland", "minor", 3.2, 64.13, -21.90, false),
                }))});
}

/// A plausible ArchieOS module registry, so the scene has real shape rather than a row
/// of identical boxes.
Value registry(const char* vision_state, const char* cortex_state) {
    return Value::object({member(
        "modules",
        Value::array({
            module("core.archie", "loaded"),
            module("core.memory", "loaded", {Value{"core.archie"}}),
            module("core.policy", "loaded", {Value{"core.archie"}}),
            module("sentry.world", "loaded", {Value{"core.memory"}, Value{"core.policy"}}),
            module("sentry.vision", vision_state, {Value{"sentry.world"}}),
            module("cortex.context", cortex_state, {Value{"core.memory"}}),
            module("science.physics", "unsatisfied", {Value{"cortex.context"}}),
        })),
         member("nodes", Value::array({
                             Value::object({member("node_id", Value{"node-b"}),
                                            member("hostname", Value{"atlas.local"}),
                                            member("health", Value{"ok"}),
                                            member("reachable", Value{true}),
                                            member("capabilities",
                                                   Value::array({Value{"compute.cuda"},
                                                                 Value{"storage.bulk"}}))}),
                             Value::object({member("node_id", Value{"node-c"}),
                                            member("hostname", Value{"pi.local"}),
                                            member("health", Value{"unreachable"}),
                                            member("reachable", Value{false}),
                                            member("capabilities", Value::array({}))}),
                         })),
         member("models", Value::array({
                              Value::object({member("id", Value{"local.reasoner"}),
                                             member("availability", Value{"ready"}),
                                             member("context_window", Value{128000})}),
                              Value::object({member("id", Value{"local.embedder"}),
                                             member("availability", Value{"cold"}),
                                             member("context_window", Value{8192})}),
                          }))});
}

void print_frame(const char* caption, const Frame& frame) {
    std::printf("\n=== %s\n", caption);
    std::printf("  mode      %s", mode_name(frame.mode));
    if (frame.mode != frame.incoming_mode) {
        std::printf("  ->  %s  (%.0f%%)", mode_name(frame.incoming_mode),
                    shown(frame.morph_progress) * 100.0);
    }
    std::printf("\n");
    std::printf("  tier      %-9s (%s)   %d fps cap, msaa %d, <=%d nodes, post %s\n",
                tier_name(frame.tier), reason_name(frame.tier_reason),
                frame.profile.frame_interval_ms > 0 ? 1000 / frame.profile.frame_interval_ms : 0,
                frame.profile.msaa_samples, frame.profile.max_visible_nodes,
                frame.profile.post_processing ? "on" : "off");
    std::printf("  camera    %-10s eye (%6.1f %6.1f %6.1f)  fov %.0f",
                camera_name(frame.camera_state), shown(frame.camera.eye.x),
                shown(frame.camera.eye.y), shown(frame.camera.eye.z),
                shown(frame.camera.fov_degrees));
    if (!frame.focused.empty()) {
        std::printf("  on %s", frame.focused.c_str());
    }
    std::printf("\n");
    std::printf("  scene     %d nodes, %d drawn, %d aggregated, %zu edges\n", frame.total_nodes,
                static_cast<int>(frame.nodes.size()), frame.aggregated, frame.edges.size());

    if (!frame.ingest.clean()) {
        std::printf("  payload   %d dropped, %d defects, %d unknown fields\n",
                    frame.ingest.dropped, static_cast<int>(frame.ingest.defects.size()),
                    frame.ingest.unknown_fields);
    }
    if (!frame.colliding_ids.empty()) {
        std::printf("  clash     ");
        for (const std::string& id : frame.colliding_ids) {
            std::printf("%s ", id.c_str());
        }
        std::printf("(claimed by more than one thing)\n");
    }
    if (!frame.dependency_cycle.empty()) {
        std::printf("  cycle     ");
        for (const std::string& id : frame.dependency_cycle) {
            std::printf("%s ", id.c_str());
        }
        std::printf("\n");
    }

    if (frame.nodes.empty()) {
        std::printf("  (nothing drawn)\n");
    } else {
        std::printf("    %-18s %-11s %-11s %-9s %5s %6s %6s %5s  %s\n", "node", "shape",
                    "detail", "data", "sev", "alpha", "glow", "moves", "where");
        for (const DrawnNode& node : frame.nodes) {
            char where[40] = "";
            if (node.location.has_value()) {
                std::snprintf(where, sizeof(where), "%.1f%c %.1f%c",
                              std::fabs(node.location->latitude),
                              node.location->latitude >= 0.0 ? 'N' : 'S',
                              std::fabs(node.location->longitude),
                              node.location->longitude >= 0.0 ? 'E' : 'W');
            }
            std::printf("  %s %-18s %-11s %-11s %-9s %5d %6.2f %6.2f %5s  %s%s\n",
                        presence_mark(node.presence), node.id.c_str(), shape_name(node.shape),
                        detail_name(node.detail), freshness_name(node.freshness), node.severity,
                        shown(node.colour.alpha), shown(node.colour.emissive),
                        node.animate ? "yes" : "no", where,
                        node.authoritative ? "" : " (second-hand)");
        }
    }

    if (frame.events_elsewhere > 0) {
        std::printf("  elsewhere %d change(s) this view cannot show\n", frame.events_elsewhere);
    }
    if (!frame.events.events.empty() || frame.events.dropped > 0) {
        std::printf("  events    ");
        for (const VisualEvent& event : frame.events.events) {
            std::printf("%s:%s ", event.node_id.c_str(), event_name(event.kind));
        }
        if (frame.events.dropped > 0) {
            std::printf("(+%d dropped)", frame.events.dropped);
        }
        std::printf("\n");
    }
}

/// What the machine is currently reporting. A real bridge polls this every second or so
/// whatever else is happening, and the probe has to as well: telemetry that stops
/// arriving is telemetry going stale, and the tier would spend most of the scenario
/// backing off from a stalled reader the story never mentions.
const char* g_verdict = "light";
const char* g_thermal = "nominal";
bool g_display = true;

void poll_kernel(Observatory& observatory, std::int64_t now_ms) {
    observatory.ingest_kernel_payload(kernel(g_verdict, g_thermal), 0, g_display, now_ms);
}

/// Advance without printing, so a scripted wait costs one line here rather than ten.
void settle(Observatory& observatory, std::int64_t from_ms, std::int64_t to_ms) {
    for (std::int64_t t = from_ms; t <= to_ms; t += 16) {
        if (t % 512 == 0) {
            poll_kernel(observatory, t);
        }
        observatory.tick(t);
    }
}

/// Tick once at `now`, having first polled the kernel as the bridge would.
Frame step(Observatory& observatory, std::int64_t now_ms) {
    poll_kernel(observatory, now_ms);
    return observatory.tick(now_ms);
}

}  // namespace

int main() {
    std::printf("archie-visual-probe — the observatory's decisions, without a screen\n");

    Observatory observatory{};

    // --- A node coming up.
    observatory.ingest_snapshot(registry("loaded", "loaded"), 0, 0);
    print_frame("startup: the registry arrives", step(observatory, 0));

    settle(observatory, 16, 1'000);
    print_frame("settled: entries have played, nothing is happening", step(observatory, 1'000));

    // --- Something breaks.
    observatory.ingest_snapshot(registry("invalid", "loaded"), 0, 2'000);
    print_frame("a module goes invalid", step(observatory, 2'000));

    // --- The machine gets busy.
    g_verdict = "saturated";
    g_thermal = "warm";
    print_frame("the machine saturates: the display yields", step(observatory, 3'000));

    // --- And goes quiet again. The promotion has to be earned.
    g_verdict = "idle";
    g_thermal = "nominal";
    print_frame("it goes quiet: the richer tier is not granted yet", step(observatory, 4'000));

    for (std::int64_t t = 4'016; t <= 10'000; t += 250) {
        poll_kernel(observatory, t);
        observatory.tick(t);
    }
    print_frame("five seconds of quiet later: ULTRA", step(observatory, 10'000));

    // --- The link to the daemon degrades.
    observatory.ingest_snapshot(registry("invalid", "loaded"), 240'000, 11'000);
    print_frame("the snapshot is four minutes old: motion stops", step(observatory, 11'000));

    // --- Contact restored, and the operator looks at something.
    observatory.ingest_snapshot(registry("invalid", "loaded"), 0, 12'000);
    step(observatory, 12'000);
    observatory.focus("sentry.vision");
    print_frame("the operator inspects the broken module", step(observatory, 12'016));

    settle(observatory, 12'032, 13'000);
    print_frame("the camera has arrived", step(observatory, 13'000));

    // --- The thing being inspected goes away.
    observatory.ingest_snapshot(
        Value::object({member("modules", Value::array({module("core.archie", "loaded")}))}), 0,
        13'500);
    print_frame("everything else vanishes from the payload: grace, not death",
                step(observatory, 13'500));

    settle(observatory, 13'516, 17'000);
    print_frame("grace expired: the camera let go of what it was watching",
                step(observatory, 17'000));

    // --- A malformed payload.
    observatory.ingest_snapshot(
        Value::object({member("modules", Value::array({
                                             module("core.archie", "loaded"),
                                             Value{"this is not a module record"},
                                             Value::object({member("state", Value{"loaded"})}),
                                             module("future.thing", "quantum-entangled"),
                                         }))}),
        0, 18'000);
    print_frame("a payload with two unusable records and one unknown state",
                step(observatory, 18'000));

    // --- The world feed arrives. It says nothing about modules, and must not be read as
    //     though it did.
    observatory.ingest_world_payload(world_batch(), 0, 18500);
    print_frame("a world batch lands: four quakes, one from a fallback source",
                step(observatory, 18500));

    // --- Changing view.
    observatory.request_mode(SceneMode::Watcher);
    print_frame("the operator asks for the world view", step(observatory, 18'516));
    settle(observatory, 18'532, 20'000);
    print_frame("arrived in Watcher", step(observatory, 20'000));

    // --- The kernel reader stops answering. Nothing else changes, and the display must
    //     not keep spending a cinematic budget on evidence that is no longer arriving.
    for (std::int64_t t = 20'016; t <= 40'000; t += 250) {
        observatory.tick(t);
    }
    print_frame("the kernel reader goes silent: the tier backs off",
                observatory.tick(40'000));

    // --- What the timeline kept.
    const History& history = observatory.history();
    const auto [from, to] = history.span();
    std::printf("\n=== timeline\n");
    std::printf("  %zu entries spanning %lldms..%lldms, %d evicted, %d out of order\n",
                history.size(), static_cast<long long>(from), static_cast<long long>(to),
                history.evicted(), history.out_of_order());
    for (const std::int64_t when : {std::int64_t{-1}, std::int64_t{1'000}, std::int64_t{2'500},
                                    std::int64_t{11'500}, std::int64_t{999'999}}) {
        const HistoryEntry* entry = history.at_or_before(when);
        if (entry == nullptr) {
            std::printf("  at %8lldms  -> nothing recorded yet\n", static_cast<long long>(when));
            continue;
        }
        int worst = 0;
        for (const NodeSample& sample : entry->nodes) {
            worst = sample.severity > worst ? sample.severity : worst;
        }
        std::printf("  at %8lldms  -> entry %lldms, %s, %zu nodes, worst severity %d\n",
                    static_cast<long long>(when), static_cast<long long>(entry->at_ms),
                    tier_name(entry->tier), entry->nodes.size(), worst);
    }

    std::printf("\n");
    return 0;
}
