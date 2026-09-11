#include "archie/visual/mesh.hpp"

#include <algorithm>
#include <cmath>
#include <numeric>

namespace archie::visual {
namespace {

constexpr double kPi = 3.14159265358979323846;
constexpr double kDegreesToRadians = kPi / 180.0;
constexpr double kEarthRadiusMeters = 6371000.0;

// Tolerance on meter-scale coordinates: far below a building's smallest real edge, so it
// only absorbs floating-point noise, never a genuine vertex.
constexpr double kEpsilonMeters = 1e-9;

/// Signed shoelace cross of two vectors from `o`: positive when `o -> a -> b` turns left.
double cross2(const LocalPoint& o, const LocalPoint& a, const LocalPoint& b) noexcept {
    return (a.east_m - o.east_m) * (b.north_m - o.north_m) -
           (a.north_m - o.north_m) * (b.east_m - o.east_m);
}

/// Twice the polygon's signed area. Positive for counter-clockwise winding as seen from
/// above (east right, north up) -- the convention every function below normalises to.
double signed_area(const std::vector<LocalPoint>& pts) noexcept {
    double area = 0.0;
    const std::size_t n = pts.size();
    for (std::size_t i = 0; i < n; ++i) {
        const LocalPoint& a = pts[i];
        const LocalPoint& b = pts[(i + 1) % n];
        area += a.east_m * b.north_m - b.east_m * a.north_m;
    }
    return area;
}

/// Drops a ring's closing repeat of its first point, if present. A real extract usually
/// stores a closed way with the same node id first and last; internally, every point is
/// distinct so no code here has to special-case index n-1 wrapping to 0 versus to 1.
std::vector<LocalPoint> without_closing_repeat(const Ring& ring) {
    std::vector<LocalPoint> pts(ring.begin(), ring.end());
    if (pts.size() >= 2) {
        const LocalPoint& first = pts.front();
        const LocalPoint& last = pts.back();
        if (std::fabs(first.east_m - last.east_m) < kEpsilonMeters &&
            std::fabs(first.north_m - last.north_m) < kEpsilonMeters) {
            pts.pop_back();
        }
    }
    return pts;
}

bool point_in_triangle(const LocalPoint& p, const LocalPoint& a, const LocalPoint& b,
                        const LocalPoint& c) noexcept {
    const double d1 = cross2(a, b, p);
    const double d2 = cross2(b, c, p);
    const double d3 = cross2(c, a, p);
    const bool has_neg = (d1 < -kEpsilonMeters) || (d2 < -kEpsilonMeters) || (d3 < -kEpsilonMeters);
    const bool has_pos = (d1 > kEpsilonMeters) || (d2 > kEpsilonMeters) || (d3 > kEpsilonMeters);
    // Strictly outside only when the point sits on the far side of at least one edge and
    // the near side of another; anything else (including exactly on an edge) is treated
    // as blocking the ear, which is the conservative choice for a polygon that repeats a
    // collinear point.
    return !(has_neg && has_pos);
}

}  // namespace

LocalPoint local_position(const GeoPoint& origin, const GeoPoint& point) noexcept {
    const double origin_lat = clamp_latitude(origin.latitude) * kDegreesToRadians;
    const double delta_lat_deg = clamp_latitude(point.latitude) - clamp_latitude(origin.latitude);
    const double delta_lon_deg = wrap_longitude(point.longitude - origin.longitude);

    LocalPoint local{};
    local.north_m = delta_lat_deg * kDegreesToRadians * kEarthRadiusMeters;
    local.east_m = delta_lon_deg * kDegreesToRadians * kEarthRadiusMeters * std::cos(origin_lat);
    return local;
}

TriangleIndices triangulate(const Ring& ring) noexcept {
    const std::vector<LocalPoint> pts = without_closing_repeat(ring);
    const std::size_t n = pts.size();
    if (n < 3) {
        return {};
    }
    if (std::fabs(signed_area(pts)) < kEpsilonMeters) {
        return {};  // collinear: no interior to cover
    }

    // Ear clipping needs one fixed winding sense to tell a convex vertex (an ear
    // candidate) from a reflex one; the polygon is walked in whichever direction of
    // `order` makes that sense counter-clockwise, so the branch lives here once instead
    // of in every convexity test below.
    std::vector<std::uint32_t> order(n);
    std::iota(order.begin(), order.end(), 0U);
    if (signed_area(pts) < 0.0) {
        std::reverse(order.begin(), order.end());
    }

    TriangleIndices triangles;
    triangles.reserve((n - 2) * 3);

    // A simple polygon yields exactly n - 2 ears; a few extra passes cover the case where
    // an earlier scan starts mid-way through the remaining ring. Beyond that, floating
    // point noise on a near-degenerate edge is stopping progress, and returning the ears
    // already found is more honest than spinning forever or asserting.
    const std::size_t guard_limit = n * n + 8;
    std::size_t guard = 0;
    while (order.size() > 3 && guard < guard_limit) {
        ++guard;
        const std::size_t m = order.size();
        bool clipped = false;
        for (std::size_t i = 0; i < m; ++i) {
            const std::size_t prev_i = (i + m - 1) % m;
            const std::size_t next_i = (i + 1) % m;
            const LocalPoint& prev = pts[order[prev_i]];
            const LocalPoint& cur = pts[order[i]];
            const LocalPoint& next = pts[order[next_i]];

            if (cross2(prev, cur, next) <= kEpsilonMeters) {
                continue;  // reflex or degenerate: cannot be an ear
            }

            bool any_inside = false;
            for (std::size_t j = 0; j < m; ++j) {
                if (j == prev_i || j == i || j == next_i) {
                    continue;
                }
                if (point_in_triangle(pts[order[j]], prev, cur, next)) {
                    any_inside = true;
                    break;
                }
            }
            if (any_inside) {
                continue;
            }

            triangles.push_back(order[prev_i]);
            triangles.push_back(order[i]);
            triangles.push_back(order[next_i]);
            order.erase(order.begin() + static_cast<std::ptrdiff_t>(i));
            clipped = true;
            break;
        }
        if (!clipped) {
            break;
        }
    }
    if (order.size() == 3) {
        triangles.push_back(order[0]);
        triangles.push_back(order[1]);
        triangles.push_back(order[2]);
    }
    return triangles;
}

Mesh extrude_polygon(const Ring& ring, float height_m) noexcept {
    Mesh mesh;
    if (!std::isfinite(height_m) || !(height_m > 0.0F)) {
        return mesh;
    }

    std::vector<LocalPoint> pts = without_closing_repeat(ring);
    const std::size_t n = pts.size();
    if (n < 3 || std::fabs(signed_area(pts)) < kEpsilonMeters) {
        return mesh;
    }

    // Every wall's outward normal and the roof's upward normal are derived below for a
    // ring wound counter-clockwise as seen from above; a clockwise ring is reversed once
    // here so that derivation only has to hold in one case.
    if (signed_area(pts) < 0.0) {
        std::reverse(pts.begin(), pts.end());
    }

    // Walls: one quad (two triangles) per edge, spanning its base and roof vertices.
    // The winding below (base_a, roof_b, base_b) and (base_a, roof_a, roof_b) -- not the
    // more obvious (base_a, base_b, roof_b) -- is the one whose right-hand-rule normal
    // actually points outward for a counter-clockwise-from-above ring; test_mesh.cpp
    // checks this against the independently-computed `normal` below rather than trusting
    // the derivation by eye.
    for (std::size_t i = 0; i < n; ++i) {
        const LocalPoint& a = pts[i];
        const LocalPoint& b = pts[(i + 1) % n];

        const Vec3 base_a{static_cast<float>(a.east_m), 0.0F, static_cast<float>(a.north_m)};
        const Vec3 base_b{static_cast<float>(b.east_m), 0.0F, static_cast<float>(b.north_m)};
        const Vec3 roof_a{static_cast<float>(a.east_m), height_m, static_cast<float>(a.north_m)};
        const Vec3 roof_b{static_cast<float>(b.east_m), height_m, static_cast<float>(b.north_m)};

        const float edge_east = static_cast<float>(b.east_m - a.east_m);
        const float edge_north = static_cast<float>(b.north_m - a.north_m);
        const float length = std::sqrt(edge_east * edge_east + edge_north * edge_north);
        Vec3 normal{0.0F, 0.0F, 0.0F};
        if (length > 0.0F) {
            // The edge direction rotated -90 degrees in the east/north plane: outward,
            // away from the interior, for a ring wound counter-clockwise from above.
            normal = Vec3{edge_north / length, 0.0F, -edge_east / length};
        }

        const auto base_index = static_cast<std::uint32_t>(mesh.vertices.size());
        mesh.vertices.push_back(base_a);
        mesh.vertices.push_back(base_b);
        mesh.vertices.push_back(roof_b);
        mesh.vertices.push_back(roof_a);
        mesh.normals.push_back(normal);
        mesh.normals.push_back(normal);
        mesh.normals.push_back(normal);
        mesh.normals.push_back(normal);

        mesh.indices.push_back(base_index + 0);
        mesh.indices.push_back(base_index + 2);
        mesh.indices.push_back(base_index + 1);
        mesh.indices.push_back(base_index + 0);
        mesh.indices.push_back(base_index + 3);
        mesh.indices.push_back(base_index + 2);
    }

    // Roof: the same footprint, ear-clipped, lifted to `height_m`. `triangulate` winds
    // its output counter-clockwise in the east/north plane, which is the *other* sense
    // from a triangle wound counter-clockwise around an upward 3D normal, so each
    // triangle's last two indices are swapped on the way in.
    const TriangleIndices roof_triangles = triangulate(Ring(pts.begin(), pts.end()));
    if (!roof_triangles.empty()) {
        const auto roof_base = static_cast<std::uint32_t>(mesh.vertices.size());
        for (const LocalPoint& p : pts) {
            mesh.vertices.push_back(
                Vec3{static_cast<float>(p.east_m), height_m, static_cast<float>(p.north_m)});
            mesh.normals.push_back(Vec3{0.0F, 1.0F, 0.0F});
        }
        for (std::size_t t = 0; t + 2 < roof_triangles.size(); t += 3) {
            mesh.indices.push_back(roof_base + roof_triangles[t]);
            mesh.indices.push_back(roof_base + roof_triangles[t + 2]);
            mesh.indices.push_back(roof_base + roof_triangles[t + 1]);
        }
    }

    return mesh;
}

namespace {

/// The unit outward-radial direction at a point on the globe -- the same formula
/// `globe_position` uses for its own placement, kept in one spot so the marker frame and
/// the building frame can never silently disagree about which way is "up" at a point.
Vec3 surface_direction(const GeoPoint& point) noexcept {
    const double latitude = clamp_latitude(point.latitude) * kDegreesToRadians;
    const double longitude = wrap_longitude(point.longitude) * kDegreesToRadians;
    const double ring = std::cos(latitude);
    return Vec3{static_cast<float>(ring * std::sin(longitude)), static_cast<float>(std::sin(latitude)),
                static_cast<float>(ring * std::cos(longitude))};
}

/// The local East-North-Up frame's own basis vectors at `point`, in the globe's fixed
/// XYZ axes: the partial derivatives of the sphere's own parameterisation with respect to
/// longitude and latitude, each normalised. `up` is `surface_direction` itself.
struct LocalFrame {
    Vec3 east;
    Vec3 north;
    Vec3 up;
};

LocalFrame local_frame(const GeoPoint& point) noexcept {
    const double latitude = clamp_latitude(point.latitude) * kDegreesToRadians;
    const double longitude = wrap_longitude(point.longitude) * kDegreesToRadians;
    const double sin_lat = std::sin(latitude);
    const double cos_lat = std::cos(latitude);
    const double sin_lon = std::sin(longitude);
    const double cos_lon = std::cos(longitude);

    LocalFrame frame{};
    frame.up = Vec3{static_cast<float>(cos_lat * sin_lon), static_cast<float>(sin_lat),
                     static_cast<float>(cos_lat * cos_lon)};
    // Increasing longitude moves east; this is that direction, normalised (the
    // un-normalised derivative carries a factor of cos(latitude), which vanishes at the
    // poles -- where "east" is not a single direction anyway).
    frame.east = Vec3{static_cast<float>(cos_lon), 0.0F, static_cast<float>(-sin_lon)};
    // Increasing latitude moves north.
    frame.north = Vec3{static_cast<float>(-sin_lat * sin_lon), static_cast<float>(cos_lat),
                        static_cast<float>(-sin_lat * cos_lon)};
    return frame;
}

Vec3 add(const Vec3& a, const Vec3& b) noexcept { return Vec3{a.x + b.x, a.y + b.y, a.z + b.z}; }

Vec3 scaled(const Vec3& v, float s) noexcept { return Vec3{v.x * s, v.y * s, v.z * s}; }

/// `e * east + u * up + n * north` -- the change of basis a local (east, up, north) point
/// or direction goes through to land in the globe's fixed XYZ axes.
Vec3 combine(const LocalFrame& frame, float e, float u, float n) noexcept {
    return add(add(scaled(frame.east, e), scaled(frame.up, u)), scaled(frame.north, n));
}

}  // namespace

Mesh place_on_globe(const Mesh& local_mesh, const GeoPoint& origin,
                     const GlobePolicy& policy) noexcept {
    constexpr double kEarthRadiusMeters = 6371000.0;
    const auto meters_to_scene = static_cast<float>(static_cast<double>(policy.radius) / kEarthRadiusMeters);

    const LocalFrame frame = local_frame(origin);
    const Vec3 anchor = scaled(surface_direction(origin), policy.radius);

    Mesh placed;
    placed.vertices.reserve(local_mesh.vertices.size());
    placed.normals.reserve(local_mesh.normals.size());
    for (const Vec3& v : local_mesh.vertices) {
        const Vec3 offset = combine(frame, v.x * meters_to_scene, v.y * meters_to_scene, v.z * meters_to_scene);
        placed.vertices.push_back(add(anchor, offset));
    }
    for (const Vec3& n : local_mesh.normals) {
        // A direction, not a position: the same basis change, with no anchor added and no
        // distance scaling -- an orthonormal change of basis already preserves length.
        placed.normals.push_back(combine(frame, n.x, n.y, n.z));
    }
    placed.indices = local_mesh.indices;
    return placed;
}

}  // namespace archie::visual
