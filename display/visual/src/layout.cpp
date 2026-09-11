#include "archie/visual/layout.hpp"

#include <algorithm>
#include <cmath>

namespace archie::visual {
namespace {

enum class Mark { Unvisited, InProgress, Done };

/// Longest chain beneath `id`, iteratively.
///
/// Deliberately not recursive. A dependency graph comes from a payload, and a payload
/// can declare a chain thousands deep — by accident or otherwise. Recursion would put
/// that on the call stack and turn a malformed module registry into a crashed display.
int resolve_depth(const std::string& id, const std::map<std::string, SceneNode>& nodes,
                  std::map<std::string, int>& depth, std::map<std::string, Mark>& mark,
                  std::vector<std::string>& in_cycle) {
    struct Frame {
        std::string id;
        std::size_t next = 0;
        int best = 0;
    };

    if (mark[id] == Mark::Done) {
        return depth[id];
    }

    std::vector<Frame> stack;
    stack.push_back(Frame{id, 0, 0});
    mark[id] = Mark::InProgress;

    while (!stack.empty()) {
        Frame& frame = stack.back();
        const auto node = nodes.find(frame.id);

        // A pointer rather than a reference: binding a const reference across a ternary
        // whose other arm is an empty vector silently copies the dependency list on
        // every iteration of this loop, which on the deep-chain path is a copy per
        // level.
        static const std::vector<std::string> kNoDependencies{};
        const std::vector<std::string>* dependencies =
            node == nodes.end() ? &kNoDependencies : &node->second.spec.depends_on;

        if (frame.next < dependencies->size()) {
            const std::string dependency = (*dependencies)[frame.next];
            ++frame.next;

            // An edge to something not on screen contributes no level: a dependency the
            // operator cannot see cannot explain a node's distance from the centre.
            if (nodes.find(dependency) == nodes.end()) {
                continue;
            }

            const Mark state = mark[dependency];
            if (state == Mark::Done) {
                frame.best = std::max(frame.best, depth[dependency] + 1);
                continue;
            }
            if (state == Mark::InProgress) {
                // A cycle. Contribute nothing through this edge so the walk terminates,
                // and record *every* node on the loop rather than only the two ends of
                // the edge that closed it. The stack holds the current path, so the
                // members are exactly the frames from `dependency` upwards. Reporting
                // just the endpoints would name two of three tangled modules and leave
                // an operator unable to see which link to break.
                const auto start =
                    std::find_if(stack.begin(), stack.end(),
                                 [&](const Frame& f) { return f.id == dependency; });
                for (auto member = start; member != stack.end(); ++member) {
                    if (std::find(in_cycle.begin(), in_cycle.end(), member->id) ==
                        in_cycle.end()) {
                        in_cycle.push_back(member->id);
                    }
                }
                continue;
            }

            mark[dependency] = Mark::InProgress;
            stack.push_back(Frame{dependency, 0, 0});
            continue;
        }

        const int settled = frame.best;
        const std::string finished = frame.id;
        depth[finished] = settled;
        mark[finished] = Mark::Done;
        stack.pop_back();

        if (!stack.empty()) {
            stack.back().best = std::max(stack.back().best, settled + 1);
        }
    }

    return depth[id];
}

}  // namespace

DepthReport compute_depths(const std::map<std::string, SceneNode>& nodes) {
    DepthReport report{};
    std::map<std::string, Mark> mark;

    for (const auto& [id, node] : nodes) {
        (void)node;
        mark[id] = Mark::Unvisited;
    }

    // `nodes` is a std::map, so this walks in id order and the result does not depend on
    // which node happened to be reconciled first.
    for (const auto& [id, node] : nodes) {
        (void)node;
        resolve_depth(id, nodes, report.depth, mark, report.in_cycle);
    }

    std::sort(report.in_cycle.begin(), report.in_cycle.end());
    return report;
}

Vec3 place(int slot, int depth, int severity, const LayoutPolicy& policy) noexcept {
    const int safe_slot = slot > 0 ? slot : 0;
    const int safe_depth = depth > 0 ? depth : 0;
    const int safe_severity = severity > 0 ? severity : 0;

    const float radius =
        policy.base_radius + static_cast<float>(safe_depth) * policy.shell_spacing;

    // Reduced in double before it reaches a float. At four thousand slots the product
    // is large enough that single precision quantises neighbouring angles onto the same
    // value, which would put two nodes in exactly the same place at the far end of a
    // busy shell — the one thing the angle is here to prevent.
    const double degrees =
        std::fmod(static_cast<double>(policy.golden_angle_degrees) * static_cast<double>(safe_slot),
                  360.0);
    const float radians = static_cast<float>(degrees * 3.14159265358979323846 / 180.0);

    Vec3 position{};
    position.x = radius * std::cos(radians);
    position.z = radius * std::sin(radians);

    // Severity lifts a node out of the plane so a problem is visible from any camera
    // angle, not only from the one facing it. The slot term is a small deterministic
    // spread that keeps a shell from reading as a single flat ring.
    position.y = static_cast<float>(safe_severity) * policy.height_per_severity +
                 static_cast<float>(safe_slot % 7) * policy.slot_height_jitter;

    return position;
}

std::map<std::string, Vec3> lay_out(const std::map<std::string, SceneNode>& nodes,
                                    const LayoutPolicy& policy) {
    return lay_out(nodes, compute_depths(nodes), policy);
}

std::map<std::string, Vec3> lay_out(const std::map<std::string, SceneNode>& nodes,
                                    const DepthReport& depths, const LayoutPolicy& policy) {
    std::map<std::string, Vec3> placed;
    for (const auto& [id, node] : nodes) {
        if (node.spec.location.has_value()) {
            // A node that knows where it is goes there. Dependency depth is meaningless
            // for an earthquake, and a shell position would be a second, contradictory
            // answer to a question the payload already answered.
            placed[id] = globe_position(*node.spec.location, node.spec.magnitude, policy.globe);
            continue;
        }
        const auto found = depths.depth.find(id);
        const int depth = found == depths.depth.end() ? 0 : found->second;
        placed[id] = place(node.slot, depth, node.spec.severity, policy);
    }
    return placed;
}

}  // namespace archie::visual
