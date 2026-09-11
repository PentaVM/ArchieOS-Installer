// Geography — where a thing is on the Earth, how far two things are, and whether a person
// could have got from one to the other in the time available.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §World
//
// This does two jobs that turn out to be the same maths.
//
// The globe needs to place a real observation — an earthquake at a real latitude and
// longitude — at the right point on a 3D sphere. That is a lat/lon to Cartesian conversion.
//
// The tracker needs to answer a safety question: two cameras saw a similar-looking person a
// few minutes apart; could it be the same person, or are the cameras too far apart to walk
// between in the time available? RFC-008's `contradicted_by_transit_time` is exactly this,
// and until now nothing computed it — the flag was set by hand. A great-circle distance and
// a maximum plausible speed answer it, and the answer is what stops a face match across two
// distant cameras from naming someone who was demonstrably somewhere else.
//
// Pure maths, no I/O, so both the render and the safety check are testable without a globe
// or a camera.

#pragma once

#include <cstdint>

namespace sentry::world {

// A point on the Earth. Degrees, because that is how every provider reports one, converted
// to radians only inside the maths.
struct GeoPoint {
    double latitude_deg = 0.0;
    double longitude_deg = 0.0;
    // Metres above the reference sphere. Zero for a surface point; a positive value lifts a
    // marker off the globe so it does not z-fight with the texture.
    double altitude_m = 0.0;
};

// A position in the globe's 3D space, unit-sphere plus altitude. The renderer scales this to
// whatever radius the scene uses; the ratios are what matter.
struct Cartesian {
    double x = 0.0;
    double y = 0.0;
    double z = 0.0;
};

// The Earth's mean radius, metres. A sphere, not an ellipsoid: the 0.3% flattening does not
// matter for placing a marker or for a transit-time sanity check, and pretending to WGS84
// precision here would be a false promise.
inline constexpr double kEarthRadiusMetres = 6'371'000.0;

// Place a geographic point on the globe.
//
// Latitude maps to the vertical axis, longitude sweeps around it, and altitude pushes the
// point radially outward. The convention is documented rather than clever: y is up (the
// poles), the prime meridian faces +z, and +x is 90°E, so a caller reading the numbers back
// can tell where a marker will land.
[[nodiscard]] Cartesian to_cartesian(const GeoPoint& point) noexcept;

// Great-circle distance between two surface points, in metres. The haversine formula, which
// is stable for the small distances a transit check cares about, where the law of cosines
// loses precision.
[[nodiscard]] double distance_metres(const GeoPoint& a, const GeoPoint& b) noexcept;

// The fastest a person is assumed to move between two sightings, metres per second.
//
// Deliberately generous — about 250 km/h, far above walking or running or city traffic —
// because the cost of the two errors is not symmetric. Setting it too low would rule out a
// genuine same-person match (someone on a fast train) and split one person into two; setting
// it high only fails to catch a contradiction that other evidence may still catch. When in
// doubt the transit check abstains rather than falsely accuses.
inline constexpr double kMaxPlausibleSpeedMetresPerSecond = 70.0;

// Whether a person could plausibly have travelled between two sightings in the time between
// them.
//
// Returns false — "not plausible", i.e. a contradiction — only when the distance is more
// than the fastest assumed travel could cover in the elapsed time. Two sightings at the same
// instant far apart are impossible (nobody is in two places at once); two far apart with
// hours between them are fine.
//
// `elapsed_ms` is the time between the sightings; a negative value is treated as its
// magnitude, since which sighting came first does not change whether the trip was possible.
[[nodiscard]] bool transit_plausible(const GeoPoint& a, const GeoPoint& b,
                                     std::int64_t elapsed_ms) noexcept;

}  // namespace sentry::world
