// Footprint geometry, checked against shapes whose area and orientation are known without
// trusting the triangulator or the extruder to get it right.

#include "archie/visual/mesh.hpp"

#include <algorithm>
#include <cmath>
#include <limits>

#include "harness.hpp"

using namespace archie::visual;

namespace {

bool near(double a, double b, double tolerance = 1e-3) { return std::fabs(a - b) <= tolerance; }

Vec3 sub(const Vec3& a, const Vec3& b) { return Vec3{a.x - b.x, a.y - b.y, a.z - b.z}; }

Vec3 cross(const Vec3& a, const Vec3& b) {
    return Vec3{a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x};
}

double dot(const Vec3& a, const Vec3& b) { return a.x * b.x + a.y * b.y + a.z * b.z; }

double length(const Vec3& v) { return std::sqrt(dot(v, v)); }

/// Sum of triangle areas in a mesh's index list -- an independent way to check "how much
/// surface is this" without trusting whichever function produced it.
double total_triangle_area(const std::vector<Vec3>& vertices, const TriangleIndices& indices) {
    double area = 0.0;
    for (std::size_t t = 0; t + 2 < indices.size(); t += 3) {
        const Vec3& a = vertices[indices[t]];
        const Vec3& b = vertices[indices[t + 1]];
        const Vec3& c = vertices[indices[t + 2]];
        area += 0.5 * length(cross(sub(b, a), sub(c, a)));
    }
    return area;
}

void local_position_places_the_origin_at_zero() {
    CASE("a point at the origin is the local origin");
    const GeoPoint origin{35.68, 139.69};
    const LocalPoint here = local_position(origin, origin);
    CHECK(near(here.east_m, 0.0) && near(here.north_m, 0.0));
}

void local_position_matches_a_hand_computed_offset() {
    CASE("a small north offset is arc-length times earth's radius, no more, no less");
    const GeoPoint origin{0.0, 0.0};
    // One hundredth of a degree of latitude, at the equator where a degree of longitude
    // and a degree of latitude cover the same ground.
    const GeoPoint north{0.01, 0.0};
    const LocalPoint offset = local_position(origin, north);
    const double expected_m = 0.01 * (3.14159265358979323846 / 180.0) * 6371000.0;
    CHECK_BECAUSE(near(offset.north_m, expected_m, 1.0),
                  "hand-computed from the arc-length formula independently of local_position's "
                  "own implementation");
    CHECK(near(offset.east_m, 0.0));
}

void local_position_shrinks_east_west_toward_the_poles() {
    CASE("a degree of longitude covers less ground away from the equator");
    const GeoPoint equator_origin{0.0, 0.0};
    const GeoPoint equator_east{0.0, 0.01};
    const double equator_offset = local_position(equator_origin, equator_east).east_m;

    const GeoPoint high_lat_origin{60.0, 0.0};
    const GeoPoint high_lat_east{60.0, 0.01};
    const double high_lat_offset = local_position(high_lat_origin, high_lat_east).east_m;

    CHECK_BECAUSE(near(high_lat_offset, equator_offset * std::cos(60.0 * 3.14159265358979323846 /
                                                                    180.0),
                        0.5),
                  "cos(60 degrees) is exactly one half, so this is a known-answer check, not "
                  "just 'smaller than the equator'");
}

void a_ccw_square_triangulates_to_its_own_area() {
    CASE("a unit square, wound counter-clockwise, covers exactly one square meter");
    const Ring square{{0.0, 0.0}, {1.0, 0.0}, {1.0, 1.0}, {0.0, 1.0}};
    const TriangleIndices triangles = triangulate(square);
    CHECK(triangles.size() == 6);  // two triangles

    std::vector<Vec3> flat;
    for (const LocalPoint& p : square) {
        flat.push_back(Vec3{static_cast<float>(p.east_m), 0.0F, static_cast<float>(p.north_m)});
    }
    CHECK(near(total_triangle_area(flat, triangles), 1.0));
}

void winding_direction_does_not_change_the_covered_area() {
    CASE("the same square wound the other way covers the same ground");
    const Ring clockwise{{0.0, 0.0}, {0.0, 1.0}, {1.0, 1.0}, {1.0, 0.0}};
    const TriangleIndices triangles = triangulate(clockwise);
    CHECK(triangles.size() == 6);

    std::vector<Vec3> flat;
    for (const LocalPoint& p : clockwise) {
        flat.push_back(Vec3{static_cast<float>(p.east_m), 0.0F, static_cast<float>(p.north_m)});
    }
    CHECK(near(total_triangle_area(flat, triangles), 1.0));
}

void a_closed_ring_that_repeats_its_first_point_still_works() {
    CASE("a way that closes on itself, as a real extract stores it");
    const Ring closed{{0.0, 0.0}, {2.0, 0.0}, {2.0, 2.0}, {0.0, 2.0}, {0.0, 0.0}};
    const TriangleIndices triangles = triangulate(closed);
    CHECK(triangles.size() == 6);
    for (const std::uint32_t index : triangles) {
        CHECK_BECAUSE(index < 4, "the repeated closing point must never be referenced as its "
                                  "own vertex, or the mesh gains a degenerate zero-area corner");
    }
}

void a_concave_l_shape_triangulates_to_its_true_area() {
    CASE("an L-shape stresses the reflex-vertex handling ear clipping exists for");
    // A 2x2 square with a 1x1 bite taken out of its top-right corner: area = 4 - 1 = 3.
    const Ring l_shape{{0.0, 0.0}, {2.0, 0.0}, {2.0, 1.0}, {1.0, 1.0}, {1.0, 2.0}, {0.0, 2.0}};
    const TriangleIndices triangles = triangulate(l_shape);
    CHECK(triangles.size() == 12);  // 6 - 2 = 4 triangles

    std::vector<Vec3> flat;
    for (const LocalPoint& p : l_shape) {
        flat.push_back(Vec3{static_cast<float>(p.east_m), 0.0F, static_cast<float>(p.north_m)});
    }
    CHECK(near(total_triangle_area(flat, triangles), 3.0));
}

void degenerate_rings_are_not_drawable() {
    CASE("fewer than three points, or points on one line, cover no interior");
    CHECK(triangulate(Ring{{0.0, 0.0}, {1.0, 0.0}}).empty());
    CHECK(triangulate(Ring{{0.0, 0.0}, {1.0, 0.0}, {2.0, 0.0}}).empty());
    CHECK(triangulate(Ring{}).empty());
}

void an_invalid_height_extrudes_to_nothing() {
    CASE("no real height means no solid, not a guessed one");
    const Ring square{{0.0, 0.0}, {1.0, 0.0}, {1.0, 1.0}, {0.0, 1.0}};
    CHECK(extrude_polygon(square, 0.0F).vertices.empty());
    CHECK(extrude_polygon(square, -3.0F).vertices.empty());
    CHECK(extrude_polygon(square, std::numeric_limits<float>::infinity()).vertices.empty());
    CHECK(extrude_polygon(square, std::nanf("")).vertices.empty());
}

void a_square_prism_has_the_right_shape() {
    CASE("a 3x4 meter footprint, 10 meters tall, has exactly the surface area that implies");
    const Ring rect{{0.0, 0.0}, {3.0, 0.0}, {3.0, 4.0}, {0.0, 4.0}};
    const Mesh mesh = extrude_polygon(rect, 10.0F);

    // 4 walls * 4 vertices each, plus 4 roof vertices.
    CHECK(mesh.vertices.size() == 20);
    CHECK(mesh.normals.size() == 20);
    // 4 walls * 2 triangles + roof's 2 triangles, all as index triples.
    CHECK(mesh.indices.size() == 30);

    const double wall_area = 2.0 * (3.0 * 10.0) + 2.0 * (4.0 * 10.0);  // perimeter * height
    const double roof_area = 3.0 * 4.0;
    const double expected_total = wall_area + roof_area;
    CHECK_BECAUSE(near(total_triangle_area(mesh.vertices, mesh.indices), expected_total, 0.01),
                  "computed independently from perimeter*height plus footprint area, not from "
                  "the extruder's own triangle count");

    float min_y = mesh.vertices.front().y;
    float max_y = mesh.vertices.front().y;
    for (const Vec3& v : mesh.vertices) {
        min_y = std::min(min_y, v.y);
        max_y = std::max(max_y, v.y);
    }
    CHECK(near(min_y, 0.0));
    CHECK_BECAUSE(near(max_y, 10.0), "nothing about a building floats above its own roofline");
}

void every_triangle_normal_agrees_with_its_stored_normal() {
    CASE("winding and the per-vertex normal must point the same way, or backface culling "
         "makes half the building invisible");
    const Ring rect{{0.0, 0.0}, {3.0, 0.0}, {3.0, 4.0}, {0.0, 4.0}};
    const Mesh mesh = extrude_polygon(rect, 5.0F);

    for (std::size_t t = 0; t + 2 < mesh.indices.size(); t += 3) {
        const std::uint32_t ia = mesh.indices[t];
        const std::uint32_t ib = mesh.indices[t + 1];
        const std::uint32_t ic = mesh.indices[t + 2];
        const Vec3 geometric_normal =
            cross(sub(mesh.vertices[ib], mesh.vertices[ia]), sub(mesh.vertices[ic], mesh.vertices[ia]));
        // The three corners of a triangle share one stored normal in this mesh (flat
        // shading per quad), so any of them is the one to check against.
        const Vec3& stored_normal = mesh.normals[ia];
        CHECK_BECAUSE(dot(geometric_normal, stored_normal) > 0.0,
                      "triangle winding disagrees with its assigned normal for at least one "
                      "face; a renderer that trusts winding for culling would show a hole where "
                      "this face should be");
    }
}

void roof_normals_point_straight_up() {
    CASE("the roof is flat ground raised up, so its normal is exactly world-up");
    const Ring square{{0.0, 0.0}, {1.0, 0.0}, {1.0, 1.0}, {0.0, 1.0}};
    const Mesh mesh = extrude_polygon(square, 2.0F);
    // The last 4 vertices are the roof cap's own (walls come first).
    for (std::size_t i = mesh.vertices.size() - 4; i < mesh.vertices.size(); ++i) {
        CHECK(near(mesh.normals[i].x, 0.0) && near(mesh.normals[i].y, 1.0) &&
              near(mesh.normals[i].z, 0.0));
    }
}

void place_on_globe_matches_hand_arithmetic_at_the_reference_point() {
    CASE("at (0,0), east/up/north map straight onto the scene's X/Y/Z -- no trig needed to check it");
    // GeoPoint{0,0} is test_geo.cpp's own reference landmark: faces +Z, and by
    // globe_position's stated convention Y is north -- the globe's polar axis, not a
    // building's own local vertical, which only coincides with +Z here at the equator on
    // the prime meridian. At exactly this point, place_on_globe's east/up/north basis is
    // (+X, +Z, +Y) respectively, so the expected numbers are arithmetic, not trigonometry.
    //
    // Coordinates near the sphere's radius (20 scene units) are compared with a looser
    // tolerance than a bare offset from zero: `float` has roughly 2e-6 of absolute
    // precision at that magnitude, well above a meters-scale building's own offset, which
    // is an honest limit of representing a whole planet and a single building in the same
    // single-precision frame -- not a defect in the arithmetic being checked here.
    const GlobePolicy policy{};  // radius 20, base_elevation 0.4 (irrelevant to this function)
    Mesh local;
    local.vertices = {Vec3{100.0F, 5.0F, 50.0F}};  // 100m east, 5m up, 50m north
    local.normals = {Vec3{0.0F, 1.0F, 0.0F}};       // straight up

    const Mesh placed = place_on_globe(local, GeoPoint{0.0, 0.0}, policy);
    CHECK(placed.vertices.size() == 1);
    CHECK(placed.normals.size() == 1);

    const double scale = static_cast<double>(policy.radius) / 6371000.0;
    CHECK(near(placed.vertices[0].x, 100.0 * scale, 1e-7));   // east: pure offset from zero
    CHECK(near(placed.vertices[0].y, 50.0 * scale, 1e-7));    // north: pure offset from zero
    CHECK_BECAUSE(near(placed.vertices[0].z, policy.radius + 5.0 * scale, 1e-5),
                  "no standoff: a building's ground level sits at exactly the bare radius, "
                  "unlike globe_position's marker elevation");
    CHECK_BECAUSE(near(placed.normals[0].x, 0.0) && near(placed.normals[0].y, 0.0) &&
                      near(placed.normals[0].z, 1.0),
                  "straight up locally is the same +Z a marker at this landmark stands off "
                  "along, at this one point where the two frames coincide");
}

void place_on_globe_is_an_isometry_at_an_arbitrary_point() {
    CASE("distances between local points survive placement, wherever on the globe it happens");
    // A point with no special-cased trig (not 0, not a pole, not a multiple of 90 degrees):
    // the check below has to hold for the rotation to be a real orthonormal basis, not by
    // coincidence of a landmark's simple numbers.
    const GeoPoint origin{-22.9, -43.2};
    const GlobePolicy policy{};
    Mesh local;
    local.vertices = {Vec3{0.0F, 0.0F, 0.0F}, Vec3{30.0F, 12.0F, -18.0F}};
    const Mesh placed = place_on_globe(local, origin, policy);

    const double local_dx = 30.0 - 0.0;
    const double local_dy = 12.0 - 0.0;
    const double local_dz = -18.0 - 0.0;
    const double local_distance_m = std::sqrt(local_dx * local_dx + local_dy * local_dy + local_dz * local_dz);
    const double scale = static_cast<double>(policy.radius) / 6371000.0;

    const Vec3 diff = sub(placed.vertices[1], placed.vertices[0]);
    const double world_distance = length(diff);
    const double expected_distance = local_distance_m * scale;
    // Both placed vertices sit near the sphere's radius (20 scene units) and are then
    // subtracted to recover a difference thousands of times smaller -- exactly the
    // cancellation the reference-point test's comment describes, so the tolerance is
    // relative to the expected distance rather than a tiny fixed number.
    CHECK_BECAUSE(near(world_distance, expected_distance, expected_distance * 1e-3),
                  "a rotation and a uniform scale preserve distances between points; if this "
                  "fails by more than float32's own precision at this magnitude, the basis "
                  "vectors are not actually orthonormal");
}

void place_on_globes_up_normal_matches_globe_positions_own_notion_of_up() {
    CASE("straight up locally must be the same direction a marker's own elevation uses");
    const GeoPoint origin{51.5, -0.1};
    const GlobePolicy policy{};
    Mesh local;
    local.normals = {Vec3{0.0F, 1.0F, 0.0F}};
    const Mesh placed = place_on_globe(local, origin, policy);

    // globe_position with base_elevation 0 and magnitude 0 returns exactly radius times the
    // surface's own outward direction -- the same "up" this function derives independently,
    // so the two must agree without either one's trig being taken on faith here.
    const GlobePolicy unit_policy{1.0F, 0.0F, 0.0F};
    const Vec3 expected_up = globe_position(origin, 0.0F, unit_policy);

    CHECK_BECAUSE(near(placed.normals[0].x, expected_up.x, 1e-6) &&
                      near(placed.normals[0].y, expected_up.y, 1e-6) &&
                      near(placed.normals[0].z, expected_up.z, 1e-6),
                  "a building's roof and an event marker floating above the same point must "
                  "agree on which way is up, or one of the two frames is simply wrong");
}

void an_extruded_building_keeps_its_area_once_placed_on_the_globe() {
    CASE("extrude_polygon and place_on_globe agree on how big the building actually is");
    const Ring rect{{0.0, 0.0}, {3.0, 0.0}, {3.0, 4.0}, {0.0, 4.0}};
    const Mesh local_mesh = extrude_polygon(rect, 10.0F);
    const double local_area = total_triangle_area(local_mesh.vertices, local_mesh.indices);

    const GlobePolicy policy{};
    const Mesh placed = place_on_globe(local_mesh, GeoPoint{35.0, 139.0}, policy);
    const double placed_area = total_triangle_area(placed.vertices, placed.indices);

    const double scale = static_cast<double>(policy.radius) / 6371000.0;
    const double expected_area = local_area * scale * scale;
    // Area comes from cross products of vertex differences taken near the sphere's own
    // radius (20 scene units), which compounds the same float32 cancellation the isometry
    // test hits; simulating this exact pipeline in double precision with every intermediate
    // value rounded to float32, as the real one is, measures that cancellation at about 1.2%
    // relative error for a building this size at this distance from the origin -- so 5% is a
    // tolerance with real headroom above the honest floor, not a loosened check. It would
    // still fail loudly on a wrong formula (a missing scale-squared factor is five orders of
    // magnitude, not five percent).
    CHECK_BECAUSE(near(placed_area, expected_area, expected_area * 0.05),
                  "area scales as the square of a uniform length scale; this is what makes a "
                  "building genuinely invisible from planetary distance rather than merely "
                  "drawn wrong");
}

}  // namespace

int main() {
    local_position_places_the_origin_at_zero();
    local_position_matches_a_hand_computed_offset();
    local_position_shrinks_east_west_toward_the_poles();
    a_ccw_square_triangulates_to_its_own_area();
    winding_direction_does_not_change_the_covered_area();
    a_closed_ring_that_repeats_its_first_point_still_works();
    a_concave_l_shape_triangulates_to_its_true_area();
    degenerate_rings_are_not_drawable();
    an_invalid_height_extrudes_to_nothing();
    a_square_prism_has_the_right_shape();
    every_triangle_normal_agrees_with_its_stored_normal();
    roof_normals_point_straight_up();
    place_on_globe_matches_hand_arithmetic_at_the_reference_point();
    place_on_globe_is_an_isometry_at_an_arbitrary_point();
    place_on_globes_up_normal_matches_globe_positions_own_notion_of_up();
    an_extruded_building_keeps_its_area_once_placed_on_the_globe();
    return archie::visual::test::report();
}
