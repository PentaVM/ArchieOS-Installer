// What the display is allowed to imply about data of a given age.

#include "archie/visual/freshness.hpp"

#include "harness.hpp"

using namespace archie::visual;

namespace {

constexpr FreshnessPolicy kPolicy{};

void thresholds_are_half_open_at_the_lower_bound() {
    CASE("age classification is exact at every boundary");

    CHECK_EQ(classify_age(0, kPolicy).tier, Freshness::Live);
    CHECK_EQ(classify_age(kPolicy.lagging_after_ms - 1, kPolicy).tier, Freshness::Live);

    CHECK_BECAUSE(classify_age(kPolicy.lagging_after_ms, kPolicy).tier == Freshness::Lagging,
                  "the threshold is the first age that qualifies, not the last that does not; "
                  "an off-by-one here shows up as a panel that never admits to lagging");
    CHECK_EQ(classify_age(kPolicy.stale_after_ms - 1, kPolicy).tier, Freshness::Lagging);

    CHECK_EQ(classify_age(kPolicy.stale_after_ms, kPolicy).tier, Freshness::Stale);
    CHECK_EQ(classify_age(kPolicy.expired_after_ms - 1, kPolicy).tier, Freshness::Stale);

    CHECK_EQ(classify_age(kPolicy.expired_after_ms, kPolicy).tier, Freshness::Expired);
    CHECK_EQ(classify_age(kPolicy.expired_after_ms * 1000, kPolicy).tier, Freshness::Expired);
}

void ordinary_clock_skew_is_tolerated() {
    CASE("a source whose clock is slightly ahead is still current");

    const auto reading = classify_age(-(kPolicy.future_tolerance_ms - 1), kPolicy);
    CHECK_EQ(reading.tier, Freshness::Live);
    CHECK_BECAUSE(!reading.implausible_future,
                  "two daemons on one machine, or a peer with its own NTP drift, routinely "
                  "disagree by a few hundred milliseconds; flagging that as a broken clock "
                  "would make the warning meaningless");

    const auto boundary = classify_age(-kPolicy.future_tolerance_ms, kPolicy);
    CHECK_EQ(boundary.tier, Freshness::Live);
    CHECK(!boundary.implausible_future);
}

void a_timestamp_from_the_future_is_never_the_freshest_thing_on_screen() {
    CASE("an implausible future timestamp is reported, not rewarded");

    const auto reading = classify_age(-(kPolicy.future_tolerance_ms + 1), kPolicy);

    CHECK_BECAUSE(reading.implausible_future,
                  "the operator is chasing a misconfigured clock and needs to be told which "
                  "of the two problems they have: old data, or wrong data");
    CHECK_BECAUSE(reading.tier != Freshness::Live,
                  "naive age arithmetic makes a timestamp ten minutes in the future look "
                  "fresher than one from now, so a node with a broken clock would win every "
                  "most-recent comparison in the display and be animated at full confidence "
                  "forever");
    CHECK_EQ(reading.tier, Freshness::Stale);

    // Far into the future is no better than slightly into it.
    const auto absurd = classify_age(-86'400'000, kPolicy);
    CHECK(absurd.implausible_future);
    CHECK_EQ(absurd.tier, Freshness::Stale);
}

void confidence_falls_monotonically() {
    CASE("older data is never drawn more strongly than newer data");

    CHECK(confidence_of(Freshness::Live) > confidence_of(Freshness::Lagging));
    CHECK(confidence_of(Freshness::Lagging) > confidence_of(Freshness::Stale));
    CHECK(confidence_of(Freshness::Stale) > confidence_of(Freshness::Expired));

    CHECK_BECAUSE(confidence_of(Freshness::Expired) > 0.0F,
                  "a dead source must stay visible and legible; a panel that fades to nothing "
                  "is indistinguishable from a healthy panel with nothing to report");
    CHECK(confidence_of(Freshness::Live) <= 1.0F);
}

void motion_stops_before_the_data_disappears() {
    CASE("stale data is never animated");

    CHECK(may_animate(Freshness::Live));
    CHECK(may_animate(Freshness::Lagging));
    CHECK_BECAUSE(!may_animate(Freshness::Stale),
                  "motion is the strongest 'this is happening now' signal a screen has, so "
                  "pulsing a node whose source stopped reporting is the most misleading thing "
                  "the observatory could do");
    CHECK(!may_animate(Freshness::Expired));
}

void a_composite_is_as_old_as_its_oldest_input() {
    CASE("worse_of picks the less trustworthy reading");

    CHECK_EQ(worse_of(Freshness::Live, Freshness::Stale), Freshness::Stale);
    CHECK_EQ(worse_of(Freshness::Stale, Freshness::Live), Freshness::Stale);
    CHECK_EQ(worse_of(Freshness::Live, Freshness::Live), Freshness::Live);
    CHECK_EQ(worse_of(Freshness::Expired, Freshness::Lagging), Freshness::Expired);
    CHECK_BECAUSE(worse_of(Freshness::Lagging, Freshness::Expired) == Freshness::Expired,
                  "a card showing live pressure beside a four-minute-old thermal reading is a "
                  "four-minute-old card, whichever order the fields happen to be in");
}

void a_stream_may_carry_its_own_thresholds() {
    CASE("per-stream policy, because one global threshold fits neither the globe nor the kernel");

    // The world projection refreshes on a provider's schedule measured in minutes.
    constexpr FreshnessPolicy world{60'000, 600'000, 3'600'000, 2'000};
    CHECK_BECAUSE(classify_age(30'000, world).tier == Freshness::Live,
                  "thirty seconds is current for a feed that updates every few minutes; the "
                  "kernel's two-second threshold would cry wolf about the globe permanently");

    // Kernel telemetry is a second or two old at most.
    constexpr FreshnessPolicy kernel{1'500, 5'000, 30'000, 2'000};
    CHECK_BECAUSE(classify_age(30'000, kernel).tier == Freshness::Expired,
                  "thirty seconds without a reading means the kernel reader has stopped, and "
                  "the world projection's threshold would stay silent about it");
}

}  // namespace

int main() {
    thresholds_are_half_open_at_the_lower_bound();
    ordinary_clock_skew_is_tolerated();
    a_timestamp_from_the_future_is_never_the_freshest_thing_on_screen();
    confidence_falls_monotonically();
    motion_stops_before_the_data_disappears();
    a_composite_is_as_old_as_its_oldest_input();
    a_stream_may_carry_its_own_thresholds();
    return archie::visual::test::report();
}
