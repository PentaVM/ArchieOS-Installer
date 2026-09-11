// Geography: placing a point on the globe, and answering whether one person could have been
// at two sightings.
//
// The second is the safety-relevant half — it computes the transit-time contradiction the
// tracker relies on, which until now was set by hand — so its cases are about the asymmetry:
// abstain rather than falsely split one person into two, but never call an impossible trip
// possible.

#include "harness.hpp"

#include "sentry/world/geo.hpp"

#include <cmath>

using namespace sentry::world;

namespace {

bool near(double a, double b, double tol) { return std::fabs(a - b) <= tol; }

GeoPoint point(double lat, double lon, double alt = 0.0) {
    return GeoPoint{lat, lon, alt};
}

// ---------------------------------------------------------------- projection

void the_origin_faces_the_prime_meridian() {
    CASE("lat 0 lon 0 sits on +z");

    const auto c = to_cartesian(point(0.0, 0.0));
    CHECK(near(c.x, 0.0, 1e-9));
    CHECK(near(c.y, 0.0, 1e-9));
    CHECK_BECAUSE(near(c.z, 1.0, 1e-9), "the prime meridian faces +z by convention");
}

void the_poles_are_on_the_vertical_axis() {
    CASE("lat 90 is straight up");

    const auto north = to_cartesian(point(90.0, 0.0));
    CHECK(near(north.y, 1.0, 1e-9));
    CHECK(near(north.x, 0.0, 1e-9));
    CHECK(near(north.z, 0.0, 1e-9));

    const auto south = to_cartesian(point(-90.0, 123.0));  // longitude is irrelevant at a pole
    CHECK(near(south.y, -1.0, 1e-9));
    CHECK(near(south.x, 0.0, 1e-9));
}

void east_is_positive_x() {
    CASE("90 east faces +x");

    const auto c = to_cartesian(point(0.0, 90.0));
    CHECK(near(c.x, 1.0, 1e-9));
    CHECK(near(c.z, 0.0, 1e-9));
}

void altitude_lifts_the_point_radially() {
    CASE("a marker sits above the surface, not in it");

    const auto surface = to_cartesian(point(0.0, 0.0, 0.0));
    const auto lifted = to_cartesian(point(0.0, 0.0, kEarthRadiusMetres));  // one radius up

    CHECK_BECAUSE(near(lifted.z, 2.0, 1e-9),
                  "lifting by one Earth radius doubles the distance from the centre");
    CHECK(lifted.z > surface.z);
}

void every_surface_point_is_on_the_unit_sphere() {
    CASE("the projection preserves the radius");

    for (const auto& p : {point(0, 0), point(45, 45), point(-30, 170), point(60, -120),
                          point(12.5, -179.9)}) {
        const auto c = to_cartesian(p);
        const double r = std::sqrt(c.x * c.x + c.y * c.y + c.z * c.z);
        CHECK(near(r, 1.0, 1e-9));
    }
}

// ---------------------------------------------------------------- distance

void distance_is_zero_for_the_same_point() {
    CASE("a point is not far from itself");
    CHECK(near(distance_metres(point(40.7, -74.0), point(40.7, -74.0)), 0.0, 1e-6));
}

void a_known_distance_comes_out_right() {
    CASE("a degree of latitude is about 111 km");

    // One degree of latitude is ~111.2 km anywhere. A good check on the haversine without a
    // table of city pairs.
    const auto d = distance_metres(point(0.0, 0.0), point(1.0, 0.0));
    CHECK_BECAUSE(near(d, 111'195.0, 500.0),
                  "one degree of latitude is about 111.2 km");

    // New York to London, ~5570 km, as a sanity check on a real long line.
    const auto ny_london = distance_metres(point(40.71, -74.01), point(51.51, -0.13));
    CHECK(near(ny_london, 5'570'000.0, 40'000.0));
}

void the_haversine_survives_the_antimeridian() {
    CASE("two points either side of 180 are close, not half the world apart");

    const auto d = distance_metres(point(0.0, 179.9), point(0.0, -179.9));
    CHECK_BECAUSE(d < 25'000.0,
                  "0.2 degrees of longitude at the equator is about 22 km, whichever side "
                  "of the antimeridian it straddles");
}

// ---------------------------------------------------------------- transit plausibility

void an_impossible_trip_is_a_contradiction() {
    CASE("two distant cameras a minute apart did not see the same person");

    // New York and London, one minute apart. Nobody walks, drives or flies that.
    const bool plausible =
        transit_plausible(point(40.71, -74.01), point(51.51, -0.13), 60'000);
    CHECK_BECAUSE(!plausible,
                  "5500 km in a minute is impossible; this is the contradiction that stops "
                  "an appearance match from naming someone who was somewhere else");
}

void a_possible_trip_is_not_a_contradiction() {
    CASE("across town, twenty minutes apart");

    // ~2 km apart, 20 minutes. An easy walk, let alone a drive.
    const bool plausible =
        transit_plausible(point(40.710, -74.010), point(40.728, -74.010), 20 * 60'000);
    CHECK(plausible);
}

void two_places_at_the_same_instant_is_impossible() {
    CASE("nobody is in two places at once");

    CHECK_BECAUSE(!transit_plausible(point(40.71, -74.01), point(40.75, -74.01), 0),
                  "a separation at a single instant cannot be one person");
    // Unless it is effectively the same place — two overlapping crops of one body.
    CHECK(transit_plausible(point(40.7100, -74.0100), point(40.71000, -74.01000), 0));
}

void the_direction_of_time_does_not_matter() {
    CASE("which sighting came first does not change whether the trip was possible");

    const auto a = point(40.71, -74.01);
    const auto b = point(51.51, -0.13);
    CHECK_EQ(transit_plausible(a, b, 60'000), transit_plausible(a, b, -60'000));
}

void the_default_speed_errs_toward_abstaining() {
    CASE("a fast train is not split into two people");

    // ~50 km in 30 minutes is 100 km/h — a train. Below the generous ceiling, so it is
    // treated as plausibly one person rather than a contradiction. The cost of the other
    // choice is worse: splitting one traveller into two.
    const bool plausible =
        transit_plausible(point(48.0, 2.0), point(48.45, 2.0), 30 * 60'000);
    CHECK_BECAUSE(plausible,
                  "when in doubt the transit check abstains rather than falsely accusing a "
                  "match of being two people");
}

}  // namespace

int main() {
    the_origin_faces_the_prime_meridian();
    the_poles_are_on_the_vertical_axis();
    east_is_positive_x();
    altitude_lifts_the_point_radially();
    every_surface_point_is_on_the_unit_sphere();

    distance_is_zero_for_the_same_point();
    a_known_distance_comes_out_right();
    the_haversine_survives_the_antimeridian();

    an_impossible_trip_is_a_contradiction();
    a_possible_trip_is_not_a_contradiction();
    two_places_at_the_same_instant_is_impossible();
    the_direction_of_time_does_not_matter();
    the_default_speed_errs_toward_abstaining();

    return sentry::test::report();
}
