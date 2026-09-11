#pragma once

// Turning a flat footprint into a solid the camera can walk around.
//
// A building arrives as a ring of ground coordinates and, sometimes, a height. Two
// honest questions live here: which triangles cover a flat polygon (ear clipping --
// correct for the simple, non-self-intersecting rings a map extract ships), and what the
// walls and roof of a prism over that polygon look like as a mesh a GPU can draw
// (extrusion). Neither question involves a network, a clock, a texture or a screen, so
// like the rest of this library it is pure arithmetic that a test can pin down exactly.
//
// This is deliberately the *only* half of building extrusion that lives here. Reading an
// actual map extract needs zlib and a protobuf-shaped decoder, which would break this
// library's one rule: it depends on nothing. That half lives in its own library and hands
// this one plain rings and a height in meters -- the same separation Sentry draws between
// its zero-dependency core and its optional camera pipeline.

#include <cstdint>
#include <vector>

#include "archie/visual/camera.hpp"  // Vec3
#include "archie/visual/geo.hpp"     // GeoPoint

namespace archie::visual {

/// A point on a flat, local tangent plane: meters east and north of some chosen origin.
///
/// Distinct from `GeoPoint` (a latitude/longitude on the sphere) and from `Vec3` (a scene
/// position): this is the intermediate, unprojected coordinate that footprint geometry is
/// easiest to reason and test in, before a later stage decides how it sits in the scene.
struct LocalPoint {
    double east_m = 0.0;
    double north_m = 0.0;
};

/// A polygon ring in local coordinates. May or may not repeat its first point as its last
/// (a map extract usually does, to mark the ring closed); every function here accepts
/// either form. Winding may be either direction -- clockwise or counter-clockwise, as
/// seen from above -- and is normalised internally, because a real extract does not
/// promise one.
using Ring = std::vector<LocalPoint>;

/// East/north meters from `origin` to `point`, on the flat plane tangent to the sphere at
/// `origin`.
///
/// A local approximation, not a geodesic: the plane departs from the sphere as the
/// square of the distance from `origin`, which is unnoticeable across a building
/// footprint or a city block and wrong to rely on across anything larger. Longitude
/// scales by the cosine of the origin's latitude, which is why the same span of
/// longitude covers fewer meters near the poles than at the equator.
[[nodiscard]] LocalPoint local_position(const GeoPoint& origin, const GeoPoint& point) noexcept;

/// A flat triangle list: three consecutive indices per triangle, into whatever point
/// array the caller provides. Kept separate from any vertex/normal buffer so this
/// function's only job is deciding which triples of an existing ring cover it.
using TriangleIndices = std::vector<std::uint32_t>;

/// Ear-clipping triangulation of a single simple polygon: no holes, no self-intersection.
///
/// Returns an empty list for a ring with fewer than three distinct points, or one whose
/// signed area is zero (collinear points) -- a caller must treat that as "not drawable",
/// never as a smaller building. Winding direction does not matter; the result always
/// covers the same area the input ring encloses.
[[nodiscard]] TriangleIndices triangulate(const Ring& ring) noexcept;

/// A drawable solid: interleaved vertex positions, a matching per-vertex normal for each
/// one, and a triangle index list. Everything a GPU vertex/index buffer needs and nothing
/// it doesn't. Positions are in meters on the same local plane as the `Ring` that produced
/// them (X = east, Y = up, Z = north) -- placing this in the scene, at scene scale, is a
/// later stage's job, not this one's.
struct Mesh {
    std::vector<Vec3> vertices;
    std::vector<Vec3> normals;
    TriangleIndices indices;
};

/// Extrude a ground-level ring upward by `height_m` into walls and a roof cap.
///
/// There is no floor cap: it sits against the ground and is never seen, so a real
/// renderer does not spend triangles on it and neither does this one. `height_m` must be
/// positive and finite -- a footprint with no known height is a decision for the caller
/// (a placeholder height, or not drawing it at all), never a guess made here, so an
/// invalid height returns an empty mesh rather than inventing one.
[[nodiscard]] Mesh extrude_polygon(const Ring& ring, float height_m) noexcept;

/// Places a locally-extruded mesh onto the globe's surface at `origin`, in the same world
/// frame and scale `globe_position` uses -- so a building and the markers already placed
/// by `globe_position` sit correctly relative to each other with no separate reconciling
/// step.
///
/// `local_mesh`'s coordinates are meters on the flat plane `local_position` produced them
/// on (X = east, Y = up, Z = north); the result is scene units in the globe's fixed XYZ
/// axes, with `local_mesh`'s ground level (Y = 0) resting exactly on the bare sphere at
/// `policy.radius` -- no standoff, unlike a `globe_position` marker, because a building
/// sits on the ground rather than floating above it to stay legible on the limb.
///
/// The rotation applied is the local East-North-Up frame's own basis at `origin`, derived
/// from `globe_position`'s formula itself so the two can never silently disagree about
/// which way is north or which side faces up. Distances scale by the real ratio of the
/// globe's scene-unit radius to Earth's actual radius -- which is also why a building is
/// invisible from any camera distance that shows more than a neighbourhood: at planetary
/// scale it truly is that small, and no rendering trick here changes that.
[[nodiscard]] Mesh place_on_globe(const Mesh& local_mesh, const GeoPoint& origin,
                                   const GlobePolicy& policy) noexcept;

}  // namespace archie::visual
