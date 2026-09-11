#include "archie/visual/geo.hpp"

#include <cmath>

namespace archie::visual {
namespace {

constexpr double kPi = 3.14159265358979323846;
constexpr double kDegreesToRadians = kPi / 180.0;

}  // namespace

bool GeoPoint::same_place_as(const GeoPoint& other) const noexcept {
    // Roughly a centimetre at the equator. Far below anything a viewer could perceive as
    // the marker moving.
    constexpr double kTolerance = 1e-7;
    return std::fabs(latitude - other.latitude) <= kTolerance &&
           std::fabs(wrap_longitude(longitude) - wrap_longitude(other.longitude)) <= kTolerance;
}

double wrap_longitude(double degrees) noexcept {
    if (!std::isfinite(degrees)) {
        // A payload can carry a NaN through a JSON number. Placing a marker at an
        // undefined coordinate puts it somewhere the camera can never reach and the
        // operator can never click, so it resolves to the prime meridian and the ingest
        // layer reports the record as defective.
        return 0.0;
    }
    double wrapped = std::fmod(degrees + 180.0, 360.0);
    if (wrapped < 0.0) {
        wrapped += 360.0;
    }
    return wrapped - 180.0;
}

double clamp_latitude(double degrees) noexcept {
    if (!std::isfinite(degrees)) {
        return 0.0;
    }
    if (degrees > 90.0) {
        return 90.0;
    }
    if (degrees < -90.0) {
        return -90.0;
    }
    return degrees;
}

Vec3 globe_position(const GeoPoint& point, float magnitude, const GlobePolicy& policy) noexcept {
    const double latitude = clamp_latitude(point.latitude) * kDegreesToRadians;
    const double longitude = wrap_longitude(point.longitude) * kDegreesToRadians;

    const double clean_magnitude =
        (std::isfinite(magnitude) && magnitude > 0.0F) ? static_cast<double>(magnitude) : 0.0;
    const double radius = static_cast<double>(policy.radius) +
                          static_cast<double>(policy.base_elevation) +
                          clean_magnitude * static_cast<double>(policy.elevation_per_magnitude);

    const double ring = std::cos(latitude) * radius;

    Vec3 position{};
    position.x = static_cast<float>(ring * std::sin(longitude));
    position.y = static_cast<float>(std::sin(latitude) * radius);
    position.z = static_cast<float>(ring * std::cos(longitude));
    return position;
}

}  // namespace archie::visual
