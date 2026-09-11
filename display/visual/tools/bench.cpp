// What one frame of the visual model actually costs.
//
// The render budget hands the renderer a frame interval — 16ms at Ultra and Normal, 33
// at Conserve — and that budget is for *drawing*. Whatever this library spends deciding
// what to draw comes out of the same 16ms before a single triangle is submitted, so a
// model that takes 6ms to think has quietly taken a third of the frame away from the
// thing it exists to serve.
//
// This measures it rather than assuming it, at the populations the tiers actually
// permit: 400 nodes at Conserve, 1500 at Normal, 4000 at Ultra.
//
// Not a test. It prints numbers and never fails, because a timing threshold in CI is a
// flake generator on shared runners — the number is for a person to read.

#include <chrono>
#include <cstdio>
#include <string>
#include <vector>

#include "archie/visual/observatory.hpp"

using namespace archie::visual;

namespace {

Value module_record(const std::string& id, std::vector<Value> dependencies) {
    return Value::object({member("id", Value{id}),
                          member("state", Value{"loaded"}),
                          member("dependencies", Value::array(std::move(dependencies)))});
}

/// A registry shaped like a real one: a few foundations, most things one or two levels
/// up, a long tail of leaves. A flat list of independent nodes would make the depth walk
/// look free when it is not.
Value registry_of(int count) {
    std::vector<Value> modules;
    modules.reserve(static_cast<std::size_t>(count));
    for (int i = 0; i < count; ++i) {
        std::vector<Value> dependencies;
        if (i > 0) {
            dependencies.push_back(Value{"m" + std::to_string(i / 8)});
        }
        if (i > 64) {
            dependencies.push_back(Value{"m" + std::to_string(i / 64)});
        }
        modules.push_back(module_record("m" + std::to_string(i), std::move(dependencies)));
    }
    return Value::object({member("modules", Value::array(std::move(modules))),
                          member("nodes", Value::array({})),
                          member("models", Value::array({}))});
}

Value kernel_idle() {
    return Value::object({
        member("schema_version", Value{1}),
        member("pressure_availability", Value::object({member("state", Value{"available"})})),
        member("verdict", Value{"idle"}),
        member("thermal_availability", Value::object({member("state", Value{"available"})})),
        member("thermal_state", Value{"nominal"}),
    });
}

struct Timing {
    double per_frame_ms = 0.0;
    double per_ingest_ms = 0.0;
    std::size_t drawn = 0;
};

Timing measure(int nodes, int frames) {
    Observatory observatory{};
    const Value registry = registry_of(nodes);

    observatory.ingest_kernel_payload(kernel_idle(), 0, true, 0);
    observatory.ingest_snapshot(registry, 0, 0);

    // Reach the Ultra tier so the budget is not quietly capping the work.
    for (std::int64_t t = 0; t <= 8000; t += 250) {
        observatory.ingest_kernel_payload(kernel_idle(), 0, true, t);
        observatory.tick(t);
    }

    const auto frame_start = std::chrono::steady_clock::now();
    std::size_t drawn = 0;
    for (int i = 0; i < frames; ++i) {
        const Frame frame = observatory.tick(9000 + i * 16);
        drawn = frame.nodes.size();
    }
    const auto frame_end = std::chrono::steady_clock::now();

    // Ingest is measured separately: it runs on the projection's schedule, not the
    // frame's, so folding it into the per-frame number would overstate what a frame
    // costs and understate what a poll costs.
    const auto ingest_start = std::chrono::steady_clock::now();
    const int ingests = 20;
    for (int i = 0; i < ingests; ++i) {
        observatory.ingest_snapshot(registry, 0, 100000 + i * 1000);
    }
    const auto ingest_end = std::chrono::steady_clock::now();

    Timing timing{};
    timing.per_frame_ms =
        std::chrono::duration<double, std::milli>(frame_end - frame_start).count() / frames;
    timing.per_ingest_ms =
        std::chrono::duration<double, std::milli>(ingest_end - ingest_start).count() / ingests;
    timing.drawn = drawn;
    return timing;
}

}  // namespace

int main() {
    std::printf("archie-visual-bench - what deciding a frame costs, before anything is drawn\n\n");
    std::printf("%8s  %8s  %12s  %14s  %s\n", "nodes", "drawn", "frame (ms)", "ingest (ms)",
                "share of a 16ms frame");

    for (const int nodes : {100, 400, 1500, 4000}) {
        const Timing timing = measure(nodes, 200);
        std::printf("%8d  %8zu  %12.3f  %14.3f  %.1f%%\n", nodes, timing.drawn,
                    timing.per_frame_ms, timing.per_ingest_ms,
                    timing.per_frame_ms / 16.0 * 100.0);
    }

    std::printf(
        "\nThe frame column is what the model spends before the renderer starts. Ingest runs\n"
        "on the projection's poll, not on every frame.\n");
    return 0;
}
