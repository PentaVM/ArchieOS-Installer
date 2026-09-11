// Where nodes go, and the rule that arriving does not disturb anyone.

#include "archie/visual/layout.hpp"

#include <algorithm>
#include <cmath>
#include <string>
#include <vector>

#include "harness.hpp"

using namespace archie::visual;

namespace {

constexpr LayoutPolicy kPolicy{};

NodeSpec spec_of(std::string id, std::vector<std::string> deps = {}, int severity = 0) {
    NodeSpec s{};
    s.id = std::move(id);
    s.kind = NodeKind::Module;
    s.severity = severity;
    s.depends_on = std::move(deps);
    return s;
}

SceneGraph graph_of(const std::vector<NodeSpec>& specs) {
    SceneGraph graph{};
    graph.apply(specs, 0);
    return graph;
}

float distance(const Vec3& a, const Vec3& b) {
    const float dx = a.x - b.x;
    const float dy = a.y - b.y;
    const float dz = a.z - b.z;
    return std::sqrt(dx * dx + dy * dy + dz * dz);
}

float radius_of(const Vec3& v) { return std::sqrt(v.x * v.x + v.z * v.z); }

// ---------------------------------------------------------------------------
// Depth: the thing radius actually means.
// ---------------------------------------------------------------------------

void depth_is_the_longest_chain_beneath_a_node() {
    CASE("a module built on three layers sits three shells out");

    const SceneGraph graph = graph_of({
        spec_of("base"),
        spec_of("middle", {"base"}),
        spec_of("top", {"middle"}),
        spec_of("peak", {"top", "base"}),
    });

    const DepthReport depths = compute_depths(graph.nodes());
    CHECK_EQ(depths.depth.at("base"), 0);
    CHECK_EQ(depths.depth.at("middle"), 1);
    CHECK_EQ(depths.depth.at("top"), 2);
    CHECK_BECAUSE(depths.depth.at("peak") == 3,
                  "the longest chain, not the shortest: a node is as high up the stack as its "
                  "deepest foundation, and taking the minimum would put a node that also "
                  "depends on something fundamental back in the middle of the scene");
    CHECK(depths.in_cycle.empty());
}

void an_invisible_dependency_contributes_nothing() {
    CASE("an edge to a node that is not on screen cannot explain a distance");

    const SceneGraph graph = graph_of({spec_of("visible", {"never.installed"})});
    const DepthReport depths = compute_depths(graph.nodes());

    CHECK_BECAUSE(depths.depth.at("visible") == 0,
                  "pushing a node outward because of a dependency the operator cannot see "
                  "makes its position unreadable: there is nothing at the centre explaining "
                  "why it is far from it");
}

void a_dependency_cycle_neither_hangs_nor_hides() {
    CASE("a tangled registry still draws, and says it is tangled");

    const SceneGraph graph = graph_of({
        spec_of("a", {"b"}),
        spec_of("b", {"c"}),
        spec_of("c", {"a"}),
        spec_of("innocent"),
    });

    const DepthReport depths = compute_depths(graph.nodes());

    CHECK_BECAUSE(depths.depth.size() == 4U,
                  "every node still gets a finite depth; a display that hung on a malformed "
                  "module registry would be a worse bug than the registry");
    CHECK_EQ(depths.in_cycle.size(), 3U);
    CHECK(std::find(depths.in_cycle.begin(), depths.in_cycle.end(), "a") != depths.in_cycle.end());
    CHECK(std::find(depths.in_cycle.begin(), depths.in_cycle.end(), "innocent") ==
          depths.in_cycle.end());
}

void a_self_dependency_is_not_a_cycle_here() {
    CASE("the graph already refuses self-edges, so depth never sees one");

    const SceneGraph graph = graph_of({spec_of("loop", {"loop"})});
    const DepthReport depths = compute_depths(graph.nodes());
    CHECK_EQ(depths.depth.at("loop"), 0);
}

void a_long_chain_does_not_use_the_call_stack() {
    CASE("a payload declaring a chain thousands deep must not crash the display");

    std::vector<NodeSpec> specs;
    specs.push_back(spec_of("n0"));
    for (int i = 1; i < 5'000; ++i) {
        specs.push_back(spec_of("n" + std::to_string(i), {"n" + std::to_string(i - 1)}));
    }

    const SceneGraph graph = graph_of(specs);
    const DepthReport depths = compute_depths(graph.nodes());

    CHECK_BECAUSE(depths.depth.at("n4999") == 4'999,
                  "recursion here would put a payload-controlled depth on the call stack and "
                  "turn a malformed registry into a crashed observatory");
    CHECK(depths.in_cycle.empty());
}

void depth_does_not_depend_on_reconciliation_order() {
    CASE("the same registry gives the same depths whatever order it arrived in");

    std::vector<NodeSpec> forwards{spec_of("a"), spec_of("b", {"a"}), spec_of("c", {"b"})};
    std::vector<NodeSpec> backwards{spec_of("c", {"b"}), spec_of("b", {"a"}), spec_of("a")};

    const DepthReport first = compute_depths(graph_of(forwards).nodes());
    const DepthReport second = compute_depths(graph_of(backwards).nodes());

    CHECK_EQ(first.depth.at("c"), second.depth.at("c"));
    CHECK_EQ(first.depth.at("c"), 2);
}

// ---------------------------------------------------------------------------
// Placement.
// ---------------------------------------------------------------------------

void position_is_a_pure_function_of_what_the_node_is() {
    CASE("the same node in the same state is always in the same place");

    const Vec3 first = place(11, 2, 1, kPolicy);
    const Vec3 second = place(11, 2, 1, kPolicy);
    CHECK(first == second);
    CHECK_BECAUSE(!(place(11, 2, 1, kPolicy) == place(12, 2, 1, kPolicy)),
                  "two nodes at the same depth and severity must still be distinguishable, or "
                  "the scene stacks them on top of each other");
}

void radius_reads_outward_from_the_foundations() {
    CASE("depth is legible as distance from the centre");

    CHECK_EQ(radius_of(place(0, 0, 0, kPolicy)), kPolicy.base_radius);
    CHECK(radius_of(place(3, 1, 0, kPolicy)) > radius_of(place(3, 0, 0, kPolicy)));
    CHECK(radius_of(place(3, 5, 0, kPolicy)) > radius_of(place(3, 1, 0, kPolicy)));

    const float step = radius_of(place(0, 2, 0, kPolicy)) - radius_of(place(0, 1, 0, kPolicy));
    CHECK_BECAUSE(std::fabs(step - kPolicy.shell_spacing) < 0.001F,
                  "shells are evenly spaced so the operator can count levels by eye");
}

void trouble_lifts_out_of_the_plane() {
    CASE("severity is visible from any camera angle");

    CHECK(place(4, 0, 3, kPolicy).y > place(4, 0, 0, kPolicy).y);
    CHECK_BECAUSE(place(4, 0, 1, kPolicy).y < place(4, 0, 2, kPolicy).y,
                  "a problem hidden behind the scene from one angle is a problem the operator "
                  "finds by luck");
}

void a_new_arrival_does_not_move_anybody() {
    CASE("the whole reason angle comes from the slot and not from a sorted position");

    SceneGraph graph{};
    graph.apply({spec_of("b"), spec_of("c")}, 0);
    const std::map<std::string, Vec3> before = lay_out(graph.nodes(), kPolicy);

    // A node whose id sorts before both of them. Under any name-ordered ring this would
    // shift every existing node.
    graph.apply({spec_of("a"), spec_of("b"), spec_of("c")}, 1'000);
    const std::map<std::string, Vec3> after = lay_out(graph.nodes(), kPolicy);

    CHECK_BECAUSE(after.at("b") == before.at("b") && after.at("c") == before.at("c"),
                  "sorting nodes into a ring would look tidier and would move half the scene "
                  "every time a module called 'a.something' was installed, so every animation "
                  "in flight would be interrupted by an unrelated arrival");
    CHECK(after.count("a") == 1U);
}

void a_departure_frees_a_place_that_is_reused_exactly() {
    CASE("slot reuse makes the layout reproducible across a node's whole life");

    GraphPolicy policy{};
    policy.departure_grace_ms = 1'000;
    SceneGraph graph{policy};

    graph.apply({spec_of("first"), spec_of("keeper")}, 0);
    const Vec3 vacated = lay_out(graph.nodes(), kPolicy).at("first");

    graph.apply({spec_of("keeper")}, 500);
    graph.collect(2'000);

    graph.apply({spec_of("keeper"), spec_of("second")}, 3'000);
    CHECK_BECAUSE(lay_out(graph.nodes(), kPolicy).at("second") == vacated,
                  "the same sequence of events always produces the same scene, which is what "
                  "makes a screenshot comparable and a bug reproducible");
}

void the_golden_angle_actually_spreads_the_slots() {
    CASE("no two of the first two hundred slots land on top of each other");

    // The claim in the header is that this distributes any prefix evenly. Checking it
    // rather than trusting it, and checking it at the population the Ultra tier actually
    // permits: a constant that looks close enough by eye can still be catastrophic. 137
    // degrees, for instance, is coprime with 360 and so repeats *exactly* every 360
    // slots, putting node 0 and node 360 on the same point — invisible in any test that
    // only looks at the first couple of hundred.
    std::vector<float> angles;
    angles.reserve(4'000);
    for (int slot = 0; slot < 4'000; ++slot) {
        const Vec3 p = place(slot, 0, 0, kPolicy);
        angles.push_back(std::atan2(p.z, p.x));
    }
    std::sort(angles.begin(), angles.end());

    float closest = 1e9F;
    for (std::size_t i = 1; i < angles.size(); ++i) {
        closest = std::min(closest, angles[i] - angles[i - 1]);
    }
    CHECK_BECAUSE(closest > 0.0F,
                  "two slots resolving to the same angle draw two nodes in exactly the same "
                  "place at the same radius, and the operator cannot even tell there are two");

    // And neighbouring slots are far apart, so an arrival is visibly separate from the
    // node that preceded it.
    const Vec3 first = place(0, 0, 0, kPolicy);
    const Vec3 second = place(1, 0, 0, kPolicy);
    CHECK(distance(first, second) > kPolicy.base_radius);
}

void degenerate_inputs_are_clamped_rather_than_reflected() {
    CASE("a negative slot or depth does not put a node inside out");

    const Vec3 origin = place(0, 0, 0, kPolicy);
    CHECK(place(-5, 0, 0, kPolicy) == origin);
    CHECK(place(0, -3, 0, kPolicy) == origin);
    CHECK_BECAUSE(place(0, 0, -2, kPolicy) == origin,
                  "a negative severity would push a node below the plane and read as the "
                  "opposite of the problem it is");
}

void every_node_in_a_scene_is_placed() {
    CASE("lay_out covers the whole graph, including departing nodes");

    SceneGraph graph{};
    graph.apply({spec_of("a"), spec_of("b", {"a"}), spec_of("c")}, 0);
    graph.apply({spec_of("a"), spec_of("b", {"a"})}, 1'000);

    const std::map<std::string, Vec3> placed = lay_out(graph.nodes(), kPolicy);
    CHECK_BECAUSE(placed.size() == 3U,
                  "a departing node is still on screen for its grace period, and a node with "
                  "no position cannot be drawn leaving");
    CHECK(radius_of(placed.at("b")) > radius_of(placed.at("a")));
}

}  // namespace

int main() {
    depth_is_the_longest_chain_beneath_a_node();
    an_invisible_dependency_contributes_nothing();
    a_dependency_cycle_neither_hangs_nor_hides();
    a_self_dependency_is_not_a_cycle_here();
    a_long_chain_does_not_use_the_call_stack();
    depth_does_not_depend_on_reconciliation_order();
    position_is_a_pure_function_of_what_the_node_is();
    radius_reads_outward_from_the_foundations();
    trouble_lifts_out_of_the_plane();
    a_new_arrival_does_not_move_anybody();
    a_departure_frees_a_place_that_is_reused_exactly();
    the_golden_angle_actually_spreads_the_slots();
    degenerate_inputs_are_clamped_rather_than_reflected();
    every_node_in_a_scene_is_placed();
    return archie::visual::test::report();
}
