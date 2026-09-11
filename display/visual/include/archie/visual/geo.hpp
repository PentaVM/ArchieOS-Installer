#pragma once

// Putting a real place in the scene.
//
// The observatory's globe plots observations at the coordinates a provider reported.
// That is the whole claim it makes, and the ways to get it quietly wrong are all
// arithmetic: swapping the axes so the northern hemisphere is drawn in the south,
// wrapping a longitude the wrong way so an event off Fiji appears off Somalia, or
// accepting a latitude of 200 and placing something inside the planet.
//
// So the conversion is here, on its own, checked against the handful of coordinates
// whose answers are known without trusting the formula: the poles, the prime meridian,
// the antimeridian, and pairs that must come out exactly opposite each other.
//
// Longitude wraps, because 181 degrees east and 179 west are the same meridian and a
// provider may legitimately report either. Latitude clamps, because there is no such
// place as 100 degrees north — that is a broken record, and bending it round to the
// far side of the pole would silently move an event to the opposite hemisphere.

#include "archie/visual/camera.hpp"

namespace archie::visual {

struct GeoPoint {
    double latitude = 0.0;
    double longitude = 0.0;

    /// The same place, within about a centimetre.
    ///
    /// Compared with a tolerance rather than exactly, for the same reason magnitude is:
    /// a provider re-reporting the same event may round its coordinates differently
    /// between fetches, and treating that as movement would restart the marker's
    /// transition on most polls.
    [[nodiscard]] bool same_place_as(const GeoPoint& other) const noexcept;
};

struct GlobePolicy {
    /// Radius of the globe in scene units.
    float radius = 20.0F;

    /// How far a marker stands off the surface at magnitude zero, and how much further
    /// per unit of magnitude. Standing off keeps a marker from being swallowed by the
    /// sphere, and scaling it means a larger event is legible on the limb.
    float base_elevation = 0.4F;
    float elevation_per_magnitude = 0.45F;
};

/// Latitude and longitude to a point in the scene.
///
/// Y is north, matching the rest of the scene where severity lifts a node upward.
/// Longitude 0 faces +Z and 90 east faces +X, matching Qt's native sphere UV map.
/// Seen from Greenwich with north up, east is right. A point and its antipode
/// come out exactly opposite; the texture and markers share this fixed frame.
[[nodiscard]] Vec3 globe_position(const GeoPoint& point, float magnitude,
                                  const GlobePolicy& policy) noexcept;

/// Wrap a longitude into [-180, 180). 181 east and 179 west are the same meridian.
[[nodiscard]] double wrap_longitude(double degrees) noexcept;

/// Clamp a latitude into [-90, 90]. There is no such place as 100 degrees north, and
/// wrapping one would move the event to the opposite hemisphere without saying so.
[[nodiscard]] double clamp_latitude(double degrees) noexcept;

}  // namespace archie::visual
