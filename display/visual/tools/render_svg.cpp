// `archie-visual-render` — draw the frame, without a GPU.
//
// This is not a screenshot of the observatory. There is no observatory to screenshot:
// the QML that would consume a `Frame` is not written. What this does is project a real
// frame through its own camera and emit an SVG, so a person can look at what the model
// decided instead of reading it in a table.
//
// Everything here comes from the frame. Positions are the ones `layout.hpp` computed,
// colours are the ones `palette.hpp` resolved, the camera is the pose `camera.hpp`
// arrived at, sizes are the bounded scale, opacity is freshness and authority. The only
// things invented are the projection itself and the choice of 2D outline per shape —
// and both are stated plainly here rather than smuggled in.
//
// What it therefore cannot show: materials, lighting, depth of field, motion. A real
// renderer will look different. It will be in the same places.

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include "archie/visual/observatory.hpp"

using namespace archie::visual;

namespace {

constexpr double kWidth = 980.0;
constexpr double kHeight = 620.0;

struct Projected {
    double x = 0.0;
    double y = 0.0;
    double depth = 0.0;  // distance along the view axis; larger is further away
    double radius = 0.0;
    bool visible = false;
};

/// Standard look-at plus a perspective divide. World up is +Y, matching the scene.
Projected project(const Vec3& point, const CameraPose& camera, double size) {
    const Vec3 f{camera.target.x - camera.eye.x, camera.target.y - camera.eye.y,
                 camera.target.z - camera.eye.z};
    const double flen = std::sqrt(f.x * f.x + f.y * f.y + f.z * f.z);
    if (flen < 1e-6) {
        return {};
    }
    const double fx = f.x / flen, fy = f.y / flen, fz = f.z / flen;

    // right = forward x worldUp, with worldUp = +Y, which reduces to (-fz, 0, fx). Then
    // up = right x forward. The cross degenerates when looking straight down, which the
    // Fabric camera very nearly does, so the fallback axis is not decoration.
    double rx = -fz;
    double ry = 0.0;
    double rz = fx;
    double rlen = std::sqrt(rx * rx + ry * ry + rz * rz);
    if (rlen < 1e-4) {
        rx = 1.0;
        ry = 0.0;
        rz = 0.0;
        rlen = 1.0;
    }
    rx /= rlen;
    ry /= rlen;
    rz /= rlen;

    const double ux = ry * fz - rz * fy;
    const double uy = rz * fx - rx * fz;
    const double uz = rx * fy - ry * fx;

    const Vec3 v{point.x - camera.eye.x, point.y - camera.eye.y, point.z - camera.eye.z};
    const double vx = v.x * rx + v.y * ry + v.z * rz;
    const double vy = v.x * ux + v.y * uy + v.z * uz;
    const double vz = v.x * fx + v.y * fy + v.z * fz;

    if (vz <= 0.35) {
        return {};  // behind the camera, or on top of it
    }

    const double focal =
        (kHeight * 0.5) / std::tan(static_cast<double>(camera.fov_degrees) * 3.14159265 / 360.0);

    Projected out{};
    out.x = kWidth * 0.5 + vx / vz * focal;
    out.y = kHeight * 0.5 - vy / vz * focal;
    out.depth = vz;
    out.radius = size / vz * focal;
    out.visible = true;
    return out;
}

std::string rgb(const Rgb& colour, double gain = 1.0) {
    const auto channel = [gain](float v) {
        const double scaled = static_cast<double>(v) * gain * 255.0;
        const int clamped = static_cast<int>(scaled < 0.0 ? 0.0 : (scaled > 255.0 ? 255.0 : scaled));
        return clamped;
    };
    char buffer[32];
    std::snprintf(buffer, sizeof(buffer), "rgb(%d,%d,%d)", channel(colour.r), channel(colour.g),
                  channel(colour.b));
    return buffer;
}

/// A 2D outline per shape. The shape grammar is the model's; these particular polygons
/// are this file's, and a real renderer would use actual meshes.
void draw_shape(NodeShape shape, double x, double y, double r, const std::string& fill,
                double alpha) {
    switch (shape) {
        case NodeShape::Cube:
            std::printf("<rect x='%.2f' y='%.2f' width='%.2f' height='%.2f' rx='%.2f' "
                        "fill='%s' fill-opacity='%.3f'/>\n",
                        x - r, y - r, r * 2, r * 2, r * 0.18, fill.c_str(), alpha);
            break;
        case NodeShape::Sphere:
            std::printf("<circle cx='%.2f' cy='%.2f' r='%.2f' fill='%s' fill-opacity='%.3f'/>\n",
                        x, y, r, fill.c_str(), alpha);
            break;
        case NodeShape::Octahedron:
            std::printf("<polygon points='%.2f,%.2f %.2f,%.2f %.2f,%.2f %.2f,%.2f' fill='%s' "
                        "fill-opacity='%.3f'/>\n",
                        x, y - r * 1.25, x + r, y, x, y + r * 1.25, x - r, y, fill.c_str(), alpha);
            break;
        case NodeShape::Capsule:
            std::printf("<rect x='%.2f' y='%.2f' width='%.2f' height='%.2f' rx='%.2f' "
                        "fill='%s' fill-opacity='%.3f'/>\n",
                        x - r * 1.4, y - r * 0.7, r * 2.8, r * 1.4, r * 0.7, fill.c_str(), alpha);
            break;
        case NodeShape::Cone:
            std::printf("<polygon points='%.2f,%.2f %.2f,%.2f %.2f,%.2f' fill='%s' "
                        "fill-opacity='%.3f'/>\n",
                        x, y - r * 1.2, x + r, y + r * 0.9, x - r, y + r * 0.9, fill.c_str(),
                        alpha);
            break;
        case NodeShape::Ring:
            std::printf("<circle cx='%.2f' cy='%.2f' r='%.2f' fill='none' stroke='%s' "
                        "stroke-width='%.2f' stroke-opacity='%.3f'/>\n",
                        x, y, r, fill.c_str(), r * 0.45, alpha);
            break;
        case NodeShape::Placeholder:
            std::printf("<circle cx='%.2f' cy='%.2f' r='%.2f' fill='none' stroke='%s' "
                        "stroke-width='1.6' stroke-dasharray='4 3' stroke-opacity='%.3f'/>\n",
                        x, y, r, fill.c_str(), alpha);
            break;
    }
}

const char* mode_name(SceneMode m) {
    switch (m) {
        case SceneMode::Cerebra: return "CEREBRA";
        case SceneMode::Brain: return "BRAIN";
        case SceneMode::Watcher: return "WATCHER";
        case SceneMode::Fabric: return "FABRIC";
    }
    return "?";
}
const char* tier_name(RenderTier t) {
    switch (t) {
        case RenderTier::Ultra: return "ULTRA";
        case RenderTier::Normal: return "NORMAL";
        case RenderTier::Conserve: return "CONSERVE";
        case RenderTier::Headless: return "HEADLESS";
    }
    return "?";
}
const char* reason_name(TierReason r) {
    switch (r) {
        case TierReason::DisplayAbsent: return "no display";
        case TierReason::ThermalCritical: return "thermal critical";
        case TierReason::ThermalThrottling: return "thermal throttling";
        case TierReason::ThermalWarm: return "chassis warm";
        case TierReason::PressureSaturated: return "cpu saturated";
        case TierReason::PressureContended: return "cpu contended";
        case TierReason::PressureLight: return "light load";
        case TierReason::PressureIdle: return "idle";
        case TierReason::TelemetryUnavailable: return "telemetry unavailable";
        case TierReason::TelemetryStale: return "telemetry stale";
    }
    return "?";
}
const char* freshness_name(Freshness f) {
    switch (f) {
        case Freshness::Live: return "live";
        case Freshness::Lagging: return "lagging";
        case Freshness::Stale: return "stale";
        case Freshness::Expired: return "expired";
    }
    return "?";
}

std::string escape(const std::string& text) {
    std::string out;
    for (const char c : text) {
        if (c == '&') out += "&amp;";
        else if (c == '<') out += "&lt;";
        else if (c == '>') out += "&gt;";
        else out += c;
    }
    return out;
}

void render(const Frame& frame, const GlobePolicy& globe, bool draw_globe) {
    std::printf("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 %.0f %.0f' "
                "font-family=\"ui-monospace,Menlo,Consolas,monospace\">\n",
                kWidth, kHeight);
    std::printf("<defs><radialGradient id='vig' cx='50%%' cy='45%%' r='75%%'>"
                "<stop offset='0%%' stop-color='#0d1117'/>"
                "<stop offset='100%%' stop-color='#05070a'/></radialGradient></defs>\n");
    std::printf("<rect width='%.0f' height='%.0f' fill='url(#vig)'/>\n", kWidth, kHeight);

    // The globe, when the scene has things standing on it. Its radius is the same policy
    // number the markers were placed against.
    if (draw_globe) {
        const Projected centre =
            project(Vec3{0.0F, 0.0F, 0.0F}, frame.camera, static_cast<double>(globe.radius));
        if (centre.visible) {
            std::printf("<circle cx='%.2f' cy='%.2f' r='%.2f' fill='#0e1a24' stroke='#1d3345' "
                        "stroke-width='1.2'/>\n",
                        centre.x, centre.y, centre.radius);
            for (int i = 1; i < 4; ++i) {
                std::printf("<ellipse cx='%.2f' cy='%.2f' rx='%.2f' ry='%.2f' fill='none' "
                            "stroke='#16283a' stroke-width='0.7'/>\n",
                            centre.x, centre.y, centre.radius,
                            centre.radius * (1.0 - i * 0.28));
            }
        }
    }

    // Edges first, so nodes sit on top of them.
    for (const DrawnEdge& edge : frame.edges) {
        const DrawnNode* from = nullptr;
        const DrawnNode* to = nullptr;
        for (const DrawnNode& node : frame.nodes) {
            if (node.id == edge.from) from = &node;
            if (node.id == edge.to) to = &node;
        }
        if (from == nullptr || to == nullptr) continue;
        const Projected a = project(from->position, frame.camera, 1.0);
        const Projected b = project(to->position, frame.camera, 1.0);
        if (!a.visible || !b.visible) continue;
        std::printf("<line x1='%.2f' y1='%.2f' x2='%.2f' y2='%.2f' stroke='#2b4256' "
                    "stroke-width='1.1' stroke-opacity='0.55'/>\n",
                    a.x, a.y, b.x, b.y);
    }

    // Back to front, so nearer nodes occlude further ones.
    struct Item {
        const DrawnNode* node;
        Projected at;
    };
    std::vector<Item> items;
    for (const DrawnNode& node : frame.nodes) {
        const Projected at =
            project(node.position, frame.camera, static_cast<double>(node.scale) * 0.62);
        if (at.visible) items.push_back(Item{&node, at});
    }
    std::sort(items.begin(), items.end(),
              [](const Item& a, const Item& b) { return a.at.depth > b.at.depth; });

    for (const Item& item : items) {
        const DrawnNode& node = *item.node;
        const std::string fill = rgb(node.colour.base);
        const double r = std::max(2.0, item.at.radius);

        if (node.colour.emissive > 0.01F) {
            std::printf("<circle cx='%.2f' cy='%.2f' r='%.2f' fill='%s' fill-opacity='%.3f'/>\n",
                        item.at.x, item.at.y, r * 2.6, fill.c_str(),
                        0.16 * static_cast<double>(node.colour.emissive) *
                            static_cast<double>(node.colour.alpha));
        }
        draw_shape(node.shape, item.at.x, item.at.y, r, fill,
                   static_cast<double>(node.colour.alpha));

        const bool labelled =
            node.detail == DetailLevel::Full || node.detail == DetailLevel::Inspect;
        if (labelled && r > 3.0) {
            std::printf("<text x='%.2f' y='%.2f' font-size='11' fill='#c8d6e2' "
                        "fill-opacity='%.3f' text-anchor='middle'>%s</text>\n",
                        item.at.x, item.at.y - r - 6.0,
                        static_cast<double>(node.colour.alpha), escape(node.label).c_str());
        }
        if (node.id == frame.focused) {
            std::printf("<circle cx='%.2f' cy='%.2f' r='%.2f' fill='none' stroke='#7fd4ff' "
                        "stroke-width='1.4' stroke-dasharray='3 3'/>\n",
                        item.at.x, item.at.y, r * 1.9);
        }
    }

    // The read-out, every number of it taken from the frame.
    std::printf("<g font-size='12'>\n");
    std::printf("<text x='22' y='34' font-size='15' fill='#e6f0f7' letter-spacing='2'>%s</text>\n",
                mode_name(frame.mode));
    if (frame.mode != frame.incoming_mode) {
        std::printf("<text x='22' y='52' fill='#7fd4ff'>&#8594; %s  %.0f%%</text>\n",
                    mode_name(frame.incoming_mode),
                    static_cast<double>(frame.morph_progress) * 100.0);
    }
    std::printf("<text x='22' y='%d' fill='#8fa5b6'>%s &#183; %s</text>\n",
                frame.mode != frame.incoming_mode ? 72 : 54, tier_name(frame.tier),
                reason_name(frame.tier_reason));
    std::printf("<text x='22' y='%d' fill='#6d8598'>%d nodes &#183; %zu drawn &#183; %d "
                "aggregated &#183; %d filtered</text>\n",
                frame.mode != frame.incoming_mode ? 90 : 72, frame.total_nodes,
                frame.nodes.size(), frame.aggregated, frame.filtered_out);

    if (!frame.nodes.empty()) {
        Freshness worst = Freshness::Live;
        for (const DrawnNode& node : frame.nodes) worst = worse_of(worst, node.freshness);
        std::printf("<text x='22' y='%d' fill='%s'>data: %s</text>\n",
                    frame.mode != frame.incoming_mode ? 108 : 90,
                    worst == Freshness::Live ? "#6d8598" : "#d9a05b", freshness_name(worst));
    }
    if (!frame.focused.empty()) {
        std::printf("<text x='%.0f' y='34' fill='#7fd4ff' text-anchor='end'>inspecting %s</text>\n",
                    kWidth - 22, escape(frame.focused).c_str());
    }
    if (frame.events_elsewhere > 0) {
        std::printf("<text x='%.0f' y='%.0f' fill='#d9a05b' text-anchor='end'>%d change(s) "
                    "elsewhere</text>\n",
                    kWidth - 22, kHeight - 22, frame.events_elsewhere);
    }
    std::printf("</g>\n</svg>\n");
}

Value module(const std::string& id, const std::string& state,
             std::vector<Value> dependencies = {}) {
    return Value::object({member("id", Value{id}), member("state", Value{state}),
                          member("dependencies", Value::array(std::move(dependencies)))});
}

Value peer(const std::string& id, const char* host, const char* health, bool reachable,
           std::vector<Value> capabilities) {
    return Value::object({member("node_id", Value{id}), member("hostname", Value{host}),
                          member("health", Value{health}), member("reachable", Value{reachable}),
                          member("capabilities", Value::array(std::move(capabilities)))});
}

Value model(const std::string& id, const char* availability, double window) {
    return Value::object({member("id", Value{id}), member("availability", Value{availability}),
                          member("context_window", Value{window})});
}

Value registry(const char* vision_state) {
    return Value::object(
        {member("modules",
                Value::array({
                    module("core.archie", "loaded"),
                    module("core.memory", "loaded", {Value{"core.archie"}}),
                    module("core.policy", "loaded", {Value{"core.archie"}}),
                    module("core.fabric", "loaded", {Value{"core.archie"}}),
                    module("sentry.world", "loaded", {Value{"core.memory"}, Value{"core.policy"}}),
                    module("sentry.vision", vision_state, {Value{"sentry.world"}}),
                    module("cortex.context", "loaded", {Value{"core.memory"}}),
                    module("cortex.recall", "loaded", {Value{"cortex.context"}}),
                    module("science.physics", "unsatisfied", {Value{"cortex.context"}}),
                    module("device.television", "denied", {Value{"core.policy"}}),
                })),
         member("nodes", Value::array({
                             peer("node-b", "atlas.local", "ok", true,
                                  {Value{"compute.cuda"}, Value{"storage.bulk"}}),
                             peer("node-c", "pi.local", "unreachable", false, {}),
                             peer("node-d", "vault.local", "degraded", true, {Value{"storage.bulk"}}),
                         })),
         member("models", Value::array({
                              model("local.reasoner", "ready", 128000),
                              model("local.embedder", "cold", 8192),
                          }))});
}

Value observation(const std::string& id, const char* grade, double magnitude, double lat,
                  double lon, bool authoritative) {
    return Value::object(
        {member("id", Value{id}),
         member("position", Value::object({member("latitude", Value{lat}),
                                           member("longitude", Value{lon})})),
         member("summary", Value{id}),
         member("details", Value::object({member("kind", Value{"earthquake"}),
                                          member("magnitude", Value{magnitude}),
                                          member("severity", Value{grade}),
                                          member("tsunami_warning", Value{false})})),
         member("provenance",
                Value::object({member("authoritative", Value{authoritative})}))});
}

Value world() {
    return Value::object({member("observations",
                                 Value::array({
                                     observation("us.tokyo", "strong", 6.4, 35.68, 139.69, true),
                                     observation("us.andes", "moderate", 5.1, -33.45, -70.66, true),
                                     observation("us.aleutian", "major", 8.1, 51.88, -176.65, true),
                                     observation("us.crete", "moderate", 5.4, 35.24, 25.13, true),
                                     observation("relay.iceland", "minor", 3.2, 64.13, -21.9, false),
                                 }))});
}

Value kernel(const char* verdict, const char* thermal) {
    return Value::object(
        {member("pressure_availability", Value::object({member("state", Value{"available"})})),
         member("verdict", Value{verdict}),
         member("thermal_availability", Value::object({member("state", Value{"available"})})),
         member("thermal_state", Value{thermal})});
}

}  // namespace

int main(int argc, char** argv) {
    const std::string which = argc > 1 ? argv[1] : "cerebra";

    Observatory observatory{};
    const auto poll = [&](std::int64_t t, const char* v, const char* th) {
        observatory.ingest_kernel_payload(kernel(v, th), 0, true, t);
    };

    observatory.ingest_snapshot(registry("invalid"), 0, 0);
    observatory.ingest_world_payload(world(), 0, 0);
    poll(0, "idle", "nominal");
    observatory.tick(0);

    // Earn the Ultra tier honestly, keeping every projection current.
    for (std::int64_t t = 250; t <= 9000; t += 250) {
        observatory.ingest_snapshot(registry("invalid"), 0, t);
        observatory.ingest_world_payload(world(), 0, t);
        poll(t, "idle", "nominal");
        observatory.tick(t);
    }

    std::int64_t now = 9000;
    const auto advance = [&](std::int64_t until) {
        for (std::int64_t t = now + 100; t <= until; t += 100) {
            observatory.ingest_snapshot(registry("invalid"), 0, t);
            observatory.ingest_world_payload(world(), 0, t);
            poll(t, "idle", "nominal");
            observatory.tick(t);
        }
        now = until;
    };

    if (which == "watcher") {
        observatory.request_mode(SceneMode::Watcher);
        advance(16000);
    } else if (which == "fabric") {
        observatory.request_mode(SceneMode::Fabric);
        advance(16000);
    } else if (which == "focus") {
        observatory.focus("sentry.vision");
        advance(12000);
    } else if (which == "morph") {
        advance(11000);
        observatory.request_mode(SceneMode::Watcher);
        for (std::int64_t t = now + 100; t <= now + 500; t += 100) {
            observatory.ingest_snapshot(registry("invalid"), 0, t);
            observatory.ingest_world_payload(world(), 0, t);
            poll(t, "idle", "nominal");
            observatory.tick(t);
        }
        now += 500;
    } else if (which == "saturated") {
        advance(10000);
        for (std::int64_t t = now + 100; t <= now + 2000; t += 100) {
            observatory.ingest_snapshot(registry("invalid"), 0, t);
            observatory.ingest_world_payload(world(), 0, t);
            poll(t, "saturated", "warm");
            observatory.tick(t);
        }
        now += 2000;
    } else if (which == "silent") {
        // Everything stops answering. Nothing else changes.
        for (std::int64_t t = now + 250; t <= now + 200000; t += 250) {
            observatory.tick(t);
        }
        now += 200000;
    } else {
        advance(11000);
    }

    const Frame frame = observatory.tick(now + 1);
    render(frame, ObservatoryPolicy{}.layout.globe,
           which == "watcher" || which == "morph" || which == "silent" || which == "cerebra" ||
               which == "saturated" || which == "focus");
    return 0;
}
