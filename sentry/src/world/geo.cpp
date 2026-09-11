#include "sentry/world/geo.hpp"

#include <cmath>

namespace sentry::world {

namespace {

constexpr double kPi = 3.14159265358979323846;

constexpr double radians(double degrees) noexcept { return degrees * kPi / 180.0; }

}  // namespace

Cartesian to_cartesian(const GeoPoint& point) noexcept {
    const double lat = radians(point.latitude_deg);
    const double lon = radians(point.longitude_deg);
    // A unit sphere plus the altitude expressed as a fraction of the Earth's radius, so a
    // marker's lift is to the same scale as its position and the renderer can multiply the
    // whole vector by one scene radius.
    const double r = 1.0 + point.altitude_m / kEarthRadiusMetres;

    // y is up (the poles); the prime meridian (lon 0) faces +z; +x is 90°E.
    const double cos_lat = std::cos(lat);
    return Cartesian{
        r * cos_lat * std::sin(lon),
        r * std::sin(lat),
        r * cos_lat * std::cos(lon),
    };
}

double distance_metres(const GeoPoint& a, const GeoPoint& b) noexcept {
    const double lat1 = radians(a.latitude_deg);
    const double lat2 = radians(b.latitude_deg);
    const double dlat = radians(b.latitude_deg - a.latitude_deg);
    const double dlon = radians(b.longitude_deg - a.longitude_deg);

    // Haversine. Stable at the small separations a transit check cares about, where the
    // spherical law of cosines rounds two nearby points to zero distance.
    const double h = std::sin(dlat / 2.0) * std::sin(dlat / 2.0) +
                     std::cos(lat1) * std::cos(lat2) * std::sin(dlon / 2.0) *
                         std::sin(dlon / 2.0);
    const double c = 2.0 * std::atan2(std::sqrt(h), std::sqrt(std::max(0.0, 1.0 - h)));
    return kEarthRadiusMetres * c;
}

bool transit_plausible(const GeoPoint& a, const GeoPoint& b, std::int64_t elapsed_ms) noexcept {
    const double distance = distance_metres(a, b);

    // Which sighting came first does not change whether the trip was possible.
    const double seconds = static_cast<double>(elapsed_ms < 0 ? -elapsed_ms : elapsed_ms) / 1000.0;

    // Same instant, two places. Only plausible if they are effectively the same place — one
    // camera's two overlapping crops of one person, say. A metre of slack absorbs the
    // rounding without admitting a real separation.
    if (seconds == 0.0) {
        return distance <= 1.0;
    }

    const double reachable = kMaxPlausibleSpeedMetresPerSecond * seconds;
    return distance <= reachable;
}

}  // namespace sentry::world
