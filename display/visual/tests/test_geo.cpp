// Putting a real place in the scene, checked against coordinates whose answers are known.

#include "archie/visual/geo.hpp"

#include <cmath>
#include <limits>

#include "harness.hpp"

using namespace archie::visual;

namespace {

constexpr GlobePolicy kPolicy{};

float radius_of(const Vec3& v) { return std::sqrt(v.x * v.x + v.y * v.y + v.z * v.z); }

bool near(float a, float b, float tolerance = 0.01F) { return std::fabs(a - b) <= tolerance; }

void the_landmarks_land_where_they_should() {
    CASE("poles and the prime meridian, whose answers need no formula");

    const float surface = kPolicy.radius + kPolicy.base_elevation;

    // 0,0 is in the Gulf of Guinea, and faces +Z on Qt's sphere texture.
    const Vec3 origin = globe_position(GeoPoint{0.0, 0.0}, 0.0F, kPolicy);
    CHECK(near(origin.x, 0.0F));
    CHECK(near(origin.y, 0.0F));
    CHECK(near(origin.z, surface));

    // North pole: straight up, and nowhere else.
    const Vec3 north = globe_position(GeoPoint{90.0, 0.0}, 0.0F, kPolicy);
    CHECK_BECAUSE(near(north.y, surface),
                  "Y is north, matching the rest of the scene where severity lifts a node "
                  "upward; swapping the axes draws the northern hemisphere in the south and "
                  "nothing about the picture says so");
    CHECK(near(north.x, 0.0F) && near(north.z, 0.0F));

    const Vec3 south = globe_position(GeoPoint{-90.0, 0.0}, 0.0F, kPolicy);
    CHECK(near(south.y, -surface));

    // The pole is the pole whatever longitude arrives with it.
    const Vec3 north_again = globe_position(GeoPoint{90.0, 137.0}, 0.0F, kPolicy);
    CHECK(near(north_again.x, north.x) && near(north_again.z, north.z));

    // Handedness, pinned explicitly. A sign flip on Z mirrors the whole globe east for
    // west, and every relative check -- antipodes, wrapping, standoff -- still passes,
    // because both sides of each comparison are mirrored together. Only an absolute
    // statement about one known meridian catches it.
    const Vec3 ninety_east = globe_position(GeoPoint{0.0, 90.0}, 0.0F, kPolicy);
    CHECK_BECAUSE(near(ninety_east.x, surface) && near(ninety_east.z, 0.0F),
                  "Qt's unrotated sphere puts 90 east on +X; viewed from Greenwich +Z, "
                  "east is right and the native texture is not mirrored");
    const Vec3 ninety_west = globe_position(GeoPoint{0.0, -90.0}, 0.0F, kPolicy);
    CHECK(near(ninety_west.x, -surface));
}

void antipodes_come_out_opposite() {
    CASE("a point and its antipode are exactly across the sphere");

    const GeoPoint here{35.68, 139.69};
    const GeoPoint there{-35.68, -40.31};

    const Vec3 a = globe_position(here, 0.0F, kPolicy);
    const Vec3 b = globe_position(there, 0.0F, kPolicy);

    CHECK_BECAUSE(near(a.x, -b.x) && near(a.y, -b.y) && near(a.z, -b.z),
                  "this is the check that catches a sign error or a swapped sine and cosine, "
                  "either of which produces a globe that looks entirely plausible and is "
                  "wrong");
}

void longitude_wraps_because_the_meridian_is_the_same() {
    CASE("181 east and 179 west are one place");

    CHECK(near(static_cast<float>(wrap_longitude(181.0)), -179.0F));
    CHECK(near(static_cast<float>(wrap_longitude(-181.0)), 179.0F));
    CHECK(near(static_cast<float>(wrap_longitude(360.0)), 0.0F));
    CHECK(near(static_cast<float>(wrap_longitude(540.0)), -180.0F));
    CHECK(near(static_cast<float>(wrap_longitude(45.0)), 45.0F));

    const Vec3 east = globe_position(GeoPoint{10.0, 181.0}, 0.0F, kPolicy);
    const Vec3 west = globe_position(GeoPoint{10.0, -179.0}, 0.0F, kPolicy);
    CHECK_BECAUSE(near(east.x, west.x) && near(east.z, west.z),
                  "wrapping the wrong way puts an event off Fiji somewhere off Somalia, and "
                  "the globe still looks entirely reasonable");
}

void latitude_clamps_because_there_is_no_such_place() {
    CASE("100 degrees north is a broken record, not a coordinate");

    CHECK(near(static_cast<float>(clamp_latitude(100.0)), 90.0F));
    CHECK(near(static_cast<float>(clamp_latitude(-100.0)), -90.0F));
    CHECK(near(static_cast<float>(clamp_latitude(45.0)), 45.0F));

    const Vec3 impossible = globe_position(GeoPoint{100.0, 0.0}, 0.0F, kPolicy);
    CHECK_BECAUSE(impossible.y > 0.0F,
                  "wrapping it instead would carry the marker over the pole and down the far "
                  "side, silently moving a broken record to the opposite hemisphere");
    CHECK(near(impossible.y, kPolicy.radius + kPolicy.base_elevation));
}

void a_marker_stands_off_the_surface() {
    CASE("nothing is drawn inside the planet");

    for (const double latitude : {-90.0, -45.0, 0.0, 12.5, 60.0, 90.0}) {
        for (const double longitude : {-180.0, -90.0, 0.0, 77.0, 179.9}) {
            const Vec3 point = globe_position(GeoPoint{latitude, longitude}, 0.0F, kPolicy);
            CHECK(radius_of(point) > kPolicy.radius);
        }
    }
}

void a_larger_event_stands_further_off() {
    CASE("magnitude is legible on the limb");

    const Vec3 small = globe_position(GeoPoint{0.0, 0.0}, 1.0F, kPolicy);
    const Vec3 large = globe_position(GeoPoint{0.0, 0.0}, 8.0F, kPolicy);
    CHECK(radius_of(large) > radius_of(small));
    CHECK_BECAUSE(
        near(radius_of(large) - radius_of(small), 7.0F * kPolicy.elevation_per_magnitude),
        "elevation is linear in magnitude, so two events are comparable by eye rather than "
        "only by reading their labels");
}

void a_broken_coordinate_does_not_place_a_marker_nowhere() {
    CASE("a non-finite coordinate resolves to a real point");

    const double nan = std::nan("");
    const double infinity = std::numeric_limits<double>::infinity();

    const Vec3 from_nan = globe_position(GeoPoint{nan, nan}, 0.0F, kPolicy);
    CHECK_BECAUSE(std::isfinite(from_nan.x) && std::isfinite(from_nan.y) &&
                      std::isfinite(from_nan.z),
                  "a NaN can travel through a JSON number, and a marker at an undefined "
                  "coordinate is one the camera can never reach and the operator can never "
                  "click");
    const Vec3 from_infinity = globe_position(GeoPoint{infinity, -infinity}, 0.0F, kPolicy);
    CHECK(std::isfinite(from_infinity.x) && std::isfinite(from_infinity.z));

    // And a non-finite magnitude does not send the marker to the edge of the universe.
    const Vec3 huge = globe_position(GeoPoint{0.0, 0.0}, infinity, kPolicy);
    CHECK(std::isfinite(radius_of(huge)));
}

void placement_is_deterministic() {
    CASE("the same coordinate is always the same point");

    const GeoPoint point{-33.87, 151.21};
    CHECK(globe_position(point, 5.5F, kPolicy) == globe_position(point, 5.5F, kPolicy));
}

}  // namespace

int main() {
    the_landmarks_land_where_they_should();
    antipodes_come_out_opposite();
    longitude_wraps_because_the_meridian_is_the_same();
    latitude_clamps_because_there_is_no_such_place();
    a_marker_stands_off_the_surface();
    a_larger_event_stands_further_off();
    a_broken_coordinate_does_not_place_a_marker_nowhere();
    placement_is_deterministic();
    return archie::visual::test::report();
}
