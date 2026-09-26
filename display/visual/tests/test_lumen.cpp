// Lumenfield's per-frame half: the physics settles, decoration stays where it is told,
// the membrane covers the field, and every mesh is a closed, finite shape.

#include "archie/visual/lumen.hpp"

#include <algorithm>
#include <cmath>
#include <set>

#include "harness.hpp"

using namespace archie::visual;
using namespace archie::visual::lumen;

namespace {

float length(const Vec3& v) { return std::sqrt(v.x * v.x + v.y * v.y + v.z * v.z); }
Vec3 sub(const Vec3& a, const Vec3& b) { return Vec3{a.x - b.x, a.y - b.y, a.z - b.z}; }
bool finite(const Vec3& v) { return std::isfinite(v.x) && std::isfinite(v.y) && std::isfinite(v.z); }

void a_released_orb_comes_back_to_its_anchor_and_stops() {
    CASE("a released orb comes back to its anchor and stops");
    std::vector<Body> bodies{{Vec3{40, 0, 0}, Vec3{0, 0, 0}, Vec3{}, false},
                             {Vec3{}, Vec3{5, 5, 5}, Vec3{}, true}};
    bool moving = true;
    int frames = 0;
    for (; frames < 2000 && moving; ++frames) {
        moving = step(bodies, std::nullopt, 1.0F / 60.0F, Motion{});
    }
    CHECK_BECAUSE(!moving, "a field at rest must stop asking for frames");
    CHECK(length(sub(bodies[0].position, bodies[0].anchor)) < 0.5F);
    CHECK_BECAUSE(bodies[1].position == Vec3{}, "the root is held at its anchor");
}

void the_pointer_pushes_only_what_it_reaches() {
    CASE("the pointer pushes only what it reaches");
    std::vector<Body> bodies{{Vec3{10, 0, 0}, Vec3{10, 0, 0}, Vec3{}, false},
                             {Vec3{100, 0, 0}, Vec3{100, 0, 0}, Vec3{}, false}};
    step(bodies, Vec3{0, 0, 0}, 1.0F / 60.0F, Motion{});
    CHECK_BECAUSE(bodies[0].position.x > 10.0F, "within reach, pushed away from the pointer");
    CHECK_EQ(bodies[1].position.x, 100.0F);
}

void nothing_leaves_the_field_or_becomes_nan() {
    CASE("nothing leaves the field or becomes NaN");
    Motion motion;
    motion.radius = 50.0F;
    std::vector<Body> bodies{{Vec3{500, 0, 0}, Vec3{0, 0, 0}, Vec3{}, false},
                             {Vec3{1, 1, 1}, Vec3{1, 1, 1}, Vec3{}, false}};
    for (int i = 0; i < 120; ++i) {
        step(bodies, Vec3{1, 1, 1}, 0.2F, motion);
    }
    for (const Body& body : bodies) {
        CHECK(finite(body.position));
        CHECK(length(body.position) <= 50.0F + 1e-3F);
    }
}

void a_selected_module_spreads_only_its_own_members() {
    CASE("a selected module spreads only its own members");
    // 0 root, 1 module, 2 and 3 its members, 4 elsewhere.
    const std::vector<Vec3> anchors{{0, 0, 0}, {50, 0, 0}, {60, 0, 0}, {50, 10, 0}, {-50, 0, 0}};
    const std::vector<int> parent{-1, 0, 1, 1, 0};
    const auto spread = focus_region(anchors, parent, 1, 0.5F, 200.0F);
    CHECK(spread[1] == anchors[1]);
    CHECK_EQ(spread[2].x, 65.0F);
    CHECK_EQ(spread[3].y, 15.0F);
    CHECK(spread[4] == anchors[4]);
    const auto clamped = focus_region(anchors, parent, 1, 0.5F, 58.0F);
    CHECK(length(clamped[2]) <= 58.0F * 1.05F + 1e-3F);
    CHECK(focus_region(anchors, parent, -1, 0.5F, 200.0F).size() == anchors.size());
}

void relation_arcs_start_and_end_on_their_orbs() {
    CASE("relation arcs start and end on their orbs");
    const Vec3 a{-30, 4, 2};
    const Vec3 b{40, -8, 12};
    CHECK(length(sub(curve(a, b, 0.0F), a)) < 1e-4F);
    CHECK(length(sub(curve(a, b, 1.0F), b)) < 1e-3F);
    const Vec3 middle = curve(a, b, 0.5F);
    CHECK_BECAUSE(middle.y > (a.y + b.y) / 2.0F, "arcs lift, so they read over the straight links");
}

void satellites_are_bounded_and_stay_around_their_module() {
    CASE("satellites are bounded and stay around their module");
    CHECK_EQ(satellite_budget(0), 15);
    CHECK_EQ(satellite_budget(1), 15);
    CHECK_EQ(satellite_budget(10'000), 260);
    CHECK(satellite_budget(4) == 30);
    const float radius = satellite_radius(40, 200.0F);
    CHECK(radius <= 200.0F * 0.16F);
    const auto offsets = satellite_offsets(80, radius, 7);
    CHECK_EQ(offsets.size(), 80U);
    for (const Vec3& o : offsets) {
        CHECK(finite(o));
        CHECK(length(o) <= radius + 1e-3F);
        CHECK(length(o) >= radius * 0.29F - 1e-3F);
    }
    CHECK_BECAUSE(satellite_offsets(80, radius, 7)[5] == offsets[5], "the same seed draws the same picture");
}

void ghosts_gather_around_modules_by_their_size() {
    CASE("ghosts gather around modules by their size");
    const std::vector<Vec3> centres{{100, 0, 0}, {-100, 0, 0}};
    const std::vector<std::size_t> members{400, 4};
    const auto points = ghosts(centres, members, 600, 200.0F);
    CHECK_EQ(points.size(), 600U);
    int near_big = 0;
    for (const Vec3& p : points) {
        CHECK(finite(p));
        if (p.x > 0) {
            ++near_big;
        }
    }
    CHECK_BECAUSE(near_big > 400, "a module ten times the members holds most of the density");
    const auto loose = ghosts({}, {}, 100, 200.0F);
    CHECK_EQ(loose.size(), 100U);
    CHECK(std::all_of(loose.begin(), loose.end(), [](const Vec3& p) { return length(p) <= 200.0F; }));
}

void the_backbone_prefers_branches_and_respects_its_budget() {
    CASE("the backbone prefers branches and respects its budget");
    std::vector<bool> branch(100, false);
    for (int i = 0; i < 20; ++i) {
        branch[static_cast<std::size_t>(i * 5)] = true;
    }
    const auto chosen = backbone(branch, 10);
    CHECK_EQ(chosen.size(), 10U);
    const auto branches = std::count_if(chosen.begin(), chosen.end(), [&](std::size_t i) { return branch[i]; });
    CHECK_EQ(branches, 7);
    CHECK_EQ(backbone(branch, 1000).size(), 100U);
    CHECK(backbone(branch, 0).empty());
}

void the_lit_neighbourhood_starts_with_the_way_home() {
    CASE("the lit neighbourhood starts with the way home");
    // 0 root; 1, 2 under it; 3, 4 under 1; 5 under 3.
    const std::vector<int> parent{-1, 0, 0, 1, 1, 3};
    const auto lit = neighbourhood(parent, 3, 16);
    CHECK(lit.size() >= 3);
    CHECK_EQ(lit[0], 3U);
    CHECK_EQ(lit[1], 1U);
    const std::set<std::size_t> set(lit.begin(), lit.end());
    CHECK(set.count(5) == 1 && set.count(4) == 1);
    CHECK_BECAUSE(set.count(0) == 0, "the root has no link to a parent to light");
    CHECK_EQ(neighbourhood(parent, 3, 2).size(), 2U);
    CHECK(neighbourhood(parent, 99, 16).empty());
}

void the_membrane_covers_the_field_and_is_finite() {
    CASE("the membrane covers the field and is finite");
    std::vector<Vec3> nodes;
    std::vector<Rgb> colours;
    for (int i = 0; i < 50; ++i) {
        const float a = static_cast<float>(i) * 0.7F;
        nodes.push_back(Vec3{std::cos(a) * 80.0F, std::sin(a * 1.3F) * 40.0F, std::sin(a) * 80.0F});
        colours.push_back(Rgb{0.2F, 0.5F, 0.8F});
    }
    const Rgb warm = to_linear(*parse_hex("#e8b47d"));
    const Membrane shell = membrane(nodes, colours, warm, 200.0F, false, 1.0F, 20, 28);
    CHECK_EQ(shell.vertices.size(), static_cast<std::size_t>(21 * 29 * 10));
    CHECK(shell.radius > 0.0F && shell.radius <= 200.0F);
    for (std::size_t v = 0; v < shell.vertices.size(); v += 10) {
        const Vec3 p{shell.vertices[v], shell.vertices[v + 1], shell.vertices[v + 2]};
        const Vec3 n{shell.vertices[v + 3], shell.vertices[v + 4], shell.vertices[v + 5]};
        CHECK(finite(p) && finite(n));
        CHECK(length(p) >= shell.radius * 0.99F && length(p) <= shell.radius * 1.05F);
        CHECK(std::fabs(length(n) - 1.0F) < 1e-3F);
        for (std::size_t c = 6; c < 10; ++c) {
            CHECK(shell.vertices[v + c] >= 0.0F && shell.vertices[v + c] <= 1.0F);
        }
    }
    const auto vertex_count = static_cast<std::uint32_t>(shell.vertices.size() / 10);
    CHECK(std::all_of(shell.indices.begin(), shell.indices.end(), [&](std::uint32_t i) { return i < vertex_count; }));
    CHECK_EQ(shell.indices.size() % 3, 0U);
    CHECK_EQ(membrane(nodes, colours, warm, 200.0F, true, 1.0F, 20, 28).radius, 200.0F);
}

void orb_meshes_are_closed_unit_spheres() {
    CASE("orb meshes are closed unit spheres");
    for (int level = 0; level <= 3; ++level) {
        const Mesh sphere = icosphere(level);
        std::size_t faces = 20;
        for (int k = 0; k < level; ++k) {
            faces *= 4;
        }
        CHECK_EQ(sphere.indices.size(), faces * 3);
        CHECK_BECAUSE(sphere.vertices.size() == faces / 2 + 2, "Euler: a closed genus-0 surface");
        for (const Vec3& v : sphere.vertices) {
            CHECK(std::fabs(length(v) - 1.0F) < 1e-5F);
        }
        CHECK_EQ(sphere.normals.size(), sphere.vertices.size());
    }
    const Mesh ring = torus(1.0F, 0.035F, 64, 5);
    CHECK_EQ(ring.vertices.size(), static_cast<std::size_t>(65 * 6));
    CHECK_EQ(ring.indices.size(), static_cast<std::size_t>(64 * 5 * 6));
}

void guides_ring_the_field() {
    CASE("guides ring the field");
    const Guides g = guides(100.0F);
    CHECK_EQ(g.points.size(), g.family.size() * 2);
    CHECK(!g.points.empty());
    for (const Vec3& p : g.points) {
        CHECK(finite(p));
        CHECK(length(p) <= 100.0F * 1.07F + 1e-2F);
        CHECK(length(p) >= 100.0F * 0.9F);
    }
}

void colours_parse_as_the_settings_write_them() {
    CASE("colours parse as the settings write them");
    const auto amber = parse_hex("#E8b47d");
    CHECK(amber.has_value());
    CHECK(std::fabs(amber->r - 232.0F / 255.0F) < 1e-6F);
    CHECK(!parse_hex("e8b47d").has_value());
    CHECK(!parse_hex("#e8b47").has_value());
    CHECK(!parse_hex("#e8b4zz").has_value());
    CHECK(to_linear(Rgb{1, 1, 1}) == (Rgb{1, 1, 1}));
    CHECK(to_linear(Rgb{0.5F, 0.5F, 0.5F}).r < 0.25F);
    CHECK(hash01(7) == hash01(7));
    CHECK(hash01(7) >= 0.0F && hash01(7) <= 1.0F);
}

}  // namespace

int main() {
    a_released_orb_comes_back_to_its_anchor_and_stops();
    the_pointer_pushes_only_what_it_reaches();
    nothing_leaves_the_field_or_becomes_nan();
    a_selected_module_spreads_only_its_own_members();
    relation_arcs_start_and_end_on_their_orbs();
    satellites_are_bounded_and_stay_around_their_module();
    ghosts_gather_around_modules_by_their_size();
    the_backbone_prefers_branches_and_respects_its_budget();
    the_lit_neighbourhood_starts_with_the_way_home();
    the_membrane_covers_the_field_and_is_finite();
    orb_meshes_are_closed_unit_spheres();
    guides_ring_the_field();
    colours_parse_as_the_settings_write_them();
    return archie::visual::test::report();
}
