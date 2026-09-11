// The render budget, and the rule that a rich tier needs positive evidence of headroom.

#include "archie/visual/budget.hpp"

#include "harness.hpp"

using namespace archie::visual;

namespace {

/// A machine that is measurably idle and cool, reported a moment ago. The only input
/// that should ever reach Ultra.
KernelReading idle_and_cool() {
    KernelReading r{};
    r.display_present = true;
    r.pressure_available = true;
    r.verdict = PressureVerdict::Idle;
    r.thermal_available = true;
    r.thermal = ThermalState::Nominal;
    r.freshness = Freshness::Live;
    return r;
}

void ultra_requires_evidence_from_both_sensors() {
    CASE("Ultra is reached only on a fresh, available, idle, cool reading");

    CHECK_EQ(ceiling_for(idle_and_cool()).tier, RenderTier::Ultra);
    CHECK_EQ(ceiling_for(idle_and_cool()).reason, TierReason::PressureIdle);
}

void an_unreadable_sensor_never_buys_a_richer_tier() {
    CASE("absence of evidence is not evidence of headroom");

    KernelReading no_pressure = idle_and_cool();
    no_pressure.pressure_available = false;
    no_pressure.verdict.reset();
    CHECK_BECAUSE(ceiling_for(no_pressure).tier == RenderTier::Normal,
                  "a node with no PSI support would otherwise render at full cinematic cost "
                  "forever, on the strength of a sensor it does not have");
    CHECK_EQ(ceiling_for(no_pressure).reason, TierReason::TelemetryUnavailable);

    KernelReading no_thermal = idle_and_cool();
    no_thermal.thermal_available = false;
    no_thermal.thermal.reset();
    CHECK_BECAUSE(ceiling_for(no_thermal).tier == RenderTier::Normal,
                  "a machine that cannot report its temperature must not be assumed cool");
}

void an_inconsistent_payload_is_treated_as_no_reading() {
    CASE("available with no value is a defect, and defects resolve cautiously");

    KernelReading claimed = idle_and_cool();
    claimed.pressure_available = true;
    claimed.verdict.reset();
    CHECK_BECAUSE(ceiling_for(claimed).tier == RenderTier::Normal,
                  "the projection declared the sensor readable and then carried nothing; "
                  "believing the flag over the missing value would spend a GPU on a payload "
                  "we already know is malformed");
    CHECK_EQ(ceiling_for(claimed).reason, TierReason::TelemetryUnavailable);

    KernelReading thermal_claimed = idle_and_cool();
    thermal_claimed.thermal_available = true;
    thermal_claimed.thermal.reset();
    CHECK_EQ(ceiling_for(thermal_claimed).tier, RenderTier::Normal);
}

void thermal_unknown_is_not_thermal_nominal() {
    CASE("no usable trip point means no judgement, not a clean bill of health");

    KernelReading unknown = idle_and_cool();
    unknown.thermal = ThermalState::Unknown;
    CHECK_BECAUSE(ceiling_for(unknown).tier == RenderTier::Normal,
                  "the upstream enum carries Unknown and Nominal separately precisely because "
                  "they differ; collapsing them here would put the difference back");

    KernelReading nominal = idle_and_cool();
    nominal.thermal = ThermalState::Nominal;
    CHECK_EQ(ceiling_for(nominal).tier, RenderTier::Ultra);
}

void heat_overrides_idleness() {
    CASE("a hot machine is austere however quiet its scheduler looks");

    KernelReading warm = idle_and_cool();
    warm.thermal = ThermalState::Warm;
    CHECK_EQ(ceiling_for(warm).tier, RenderTier::Normal);
    CHECK_EQ(ceiling_for(warm).reason, TierReason::ThermalWarm);

    KernelReading throttling = idle_and_cool();
    throttling.thermal = ThermalState::Throttling;
    CHECK_BECAUSE(ceiling_for(throttling).tier == RenderTier::Conserve,
                  "PSI reads idle while the kernel is cutting clocks, because nothing is "
                  "waiting on a CPU that has simply been slowed down; thermal is the only "
                  "sensor that sees this");
    CHECK_EQ(ceiling_for(throttling).reason, TierReason::ThermalThrottling);

    KernelReading critical = idle_and_cool();
    critical.thermal = ThermalState::Critical;
    CHECK_EQ(ceiling_for(critical).tier, RenderTier::Conserve);
    CHECK_EQ(ceiling_for(critical).reason, TierReason::ThermalCritical);
}

void pressure_maps_onto_the_ladder() {
    CASE("each pressure verdict lands where the scheduler's own documentation puts it");

    KernelReading r = idle_and_cool();

    r.verdict = PressureVerdict::Light;
    CHECK_EQ(ceiling_for(r).tier, RenderTier::Normal);
    CHECK_EQ(ceiling_for(r).reason, TierReason::PressureLight);

    r.verdict = PressureVerdict::Contended;
    CHECK_BECAUSE(ceiling_for(r).tier == RenderTier::Conserve,
                  "upstream says Contended means yield idle work, and post-processing on a "
                  "status display is idle work by any honest reading");

    r.verdict = PressureVerdict::Saturated;
    CHECK_EQ(ceiling_for(r).tier, RenderTier::Conserve);
    CHECK_EQ(ceiling_for(r).reason, TierReason::PressureSaturated);
}

void a_saturated_machine_is_never_blanked() {
    CASE("the observatory does not go dark at the moment it is most needed");

    KernelReading r = idle_and_cool();
    r.verdict = PressureVerdict::Saturated;
    r.thermal = ThermalState::Critical;
    CHECK_BECAUSE(ceiling_for(r).tier != RenderTier::Headless,
                  "an operator watching a machine melt needs the screen; Headless is reserved "
                  "for there being nobody to show frames to, and must never be reachable as a "
                  "performance decision");
    CHECK_EQ(ceiling_for(r).tier, RenderTier::Conserve);
}

void no_display_means_no_frames() {
    CASE("Headless is a fact about the world, not a judgement about load");

    KernelReading r = idle_and_cool();
    r.display_present = false;
    CHECK_EQ(ceiling_for(r).tier, RenderTier::Headless);
    CHECK_EQ(ceiling_for(r).reason, TierReason::DisplayAbsent);
    CHECK_EQ(profile_of(RenderTier::Headless).frame_interval_ms, 0);
}

void old_telemetry_cannot_buy_a_promotion() {
    CASE("a stale reading describes a machine that no longer exists");

    KernelReading stale = idle_and_cool();
    stale.freshness = Freshness::Stale;
    CHECK_EQ(ceiling_for(stale).tier, RenderTier::Normal);
    CHECK_EQ(ceiling_for(stale).reason, TierReason::TelemetryStale);

    KernelReading expired = idle_and_cool();
    expired.freshness = Freshness::Expired;
    CHECK_EQ(ceiling_for(expired).tier, RenderTier::Normal);

    KernelReading lagging = idle_and_cool();
    lagging.freshness = Freshness::Lagging;
    CHECK_BECAUSE(ceiling_for(lagging).tier == RenderTier::Ultra,
                  "lagging is still within a sane window; treating it as unusable would drop "
                  "every machine to Normal permanently on any slightly slow poll");
}

void old_telemetry_does_not_rescue_an_austere_tier() {
    CASE("silence after bad news is not good news");

    KernelReading r = idle_and_cool();
    r.verdict = PressureVerdict::Saturated;
    r.freshness = Freshness::Expired;
    CHECK_BECAUSE(ceiling_for(r).tier == RenderTier::Conserve,
                  "the staleness rule caps how rich a tier evidence can buy; if it also raised "
                  "austere tiers back to Normal, then losing contact with a saturated machine "
                  "would make the display spend more on it, not less");
    CHECK_EQ(ceiling_for(r).reason, TierReason::PressureSaturated);
}

void austerity_costs_monotonically_less() {
    CASE("every step down the ladder is cheaper in every dimension");

    const RenderTier ladder[] = {RenderTier::Ultra, RenderTier::Normal, RenderTier::Conserve,
                                 RenderTier::Headless};
    for (int i = 1; i < 4; ++i) {
        const RenderProfile richer = profile_of(ladder[i - 1]);
        const RenderProfile poorer = profile_of(ladder[i]);
        CHECK_BECAUSE(poorer.max_visible_nodes <= richer.max_visible_nodes,
                      "a tier that drew more geometry than the one above it would make backing "
                      "off cost more than not backing off");
        CHECK(poorer.max_particles <= richer.max_particles);
        CHECK(poorer.msaa_samples <= richer.msaa_samples);
        CHECK(static_cast<int>(poorer.post_processing) <= static_cast<int>(richer.post_processing));
    }
    CHECK(profile_of(RenderTier::Conserve).frame_interval_ms >
          profile_of(RenderTier::Normal).frame_interval_ms);
    CHECK(!profile_of(RenderTier::Conserve).ambient_motion);
}

// ---------------------------------------------------------------------------
// Hysteresis.
// ---------------------------------------------------------------------------

void demotion_takes_effect_immediately() {
    CASE("a machine that has just become busy gets its GPU back this frame");

    BudgetGovernor governor{};
    // Earn Ultra first.
    governor.observe(idle_and_cool(), 0);
    governor.observe(idle_and_cool(), 10'000);
    CHECK_EQ(governor.tier(), RenderTier::Ultra);

    KernelReading busy = idle_and_cool();
    busy.verdict = PressureVerdict::Saturated;
    CHECK_BECAUSE(governor.observe(busy, 10'001) == RenderTier::Conserve,
                  "waiting out a dwell before backing off would keep the display competing "
                  "with the work it is meant to be reporting on, for exactly as long as the "
                  "dwell lasts");
    CHECK_EQ(governor.reason(), TierReason::PressureSaturated);
}

void promotion_waits_out_the_dwell() {
    CASE("a richer tier must be continuously justified before it is believed");

    BudgetPolicy policy{};
    policy.promotion_dwell_ms = 5'000;
    BudgetGovernor governor{policy};

    KernelReading busy = idle_and_cool();
    busy.verdict = PressureVerdict::Saturated;
    governor.observe(busy, 0);
    CHECK_EQ(governor.tier(), RenderTier::Conserve);

    CHECK_EQ(governor.observe(idle_and_cool(), 1'000), RenderTier::Conserve);
    CHECK_EQ(governor.observe(idle_and_cool(), 4'000), RenderTier::Conserve);
    CHECK_BECAUSE(governor.observe(idle_and_cool(), 5'999) == RenderTier::Conserve,
                  "the dwell runs from the first qualifying sample, not from the previous one");
    CHECK_BECAUSE(governor.observe(idle_and_cool(), 6'000) == RenderTier::Ultra,
                  "five seconds after the first idle sample at t=1000, the promotion is due");
}

void a_pending_promotion_is_visible() {
    CASE("the display can say a richer tier is on its way");

    BudgetGovernor governor{};
    KernelReading busy = idle_and_cool();
    busy.verdict = PressureVerdict::Saturated;
    governor.observe(busy, 0);

    governor.observe(idle_and_cool(), 1'000);
    CHECK_EQ(governor.promotion_pending_ms(), 0);
    governor.observe(idle_and_cool(), 3'000);
    CHECK_BECAUSE(governor.promotion_pending_ms() == 2'000,
                  "an operator who has just watched their machine go quiet needs to see that "
                  "the observatory noticed, rather than a display that appears to ignore it");
}

void a_fluctuating_machine_still_recovers() {
    CASE("the window is worth what its worst sample justifies, not nothing at all");

    BudgetPolicy policy{};
    policy.promotion_dwell_ms = 5'000;
    BudgetGovernor governor{policy};

    KernelReading busy = idle_and_cool();
    busy.verdict = PressureVerdict::Saturated;
    governor.observe(busy, 0);
    CHECK_EQ(governor.tier(), RenderTier::Conserve);

    // Idle, then merely light, then idle again. Never austere enough to demote, never
    // uniformly idle enough to deserve Ultra.
    KernelReading light = idle_and_cool();
    light.verdict = PressureVerdict::Light;

    governor.observe(idle_and_cool(), 1'000);
    governor.observe(light, 2'000);
    governor.observe(idle_and_cool(), 3'000);
    governor.observe(light, 4'000);
    const RenderTier settled = governor.observe(idle_and_cool(), 6'000);

    CHECK_BECAUSE(settled == RenderTier::Normal,
                  "restarting the dwell on every fluctuation would mean a machine that is "
                  "genuinely quiet but slightly uneven never recovers its tier at all");
    CHECK_BECAUSE(settled != RenderTier::Ultra,
                  "the window contained samples that only justified Normal, and a promotion is "
                  "only worth what the worst sample in it supports");
}

void a_clock_that_goes_backwards_never_grants_a_promotion() {
    CASE("elapsed time on a bad clock is not evidence");

    BudgetPolicy policy{};
    policy.promotion_dwell_ms = 5'000;
    BudgetGovernor governor{policy};

    KernelReading busy = idle_and_cool();
    busy.verdict = PressureVerdict::Saturated;
    governor.observe(busy, 1'000'000);
    CHECK_EQ(governor.tier(), RenderTier::Conserve);

    governor.observe(idle_and_cool(), 1'000'100);

    // The clock jumps back well past the dwell. Naive subtraction would compute a huge
    // negative elapsed time, or, with unsigned arithmetic, a huge positive one.
    CHECK_BECAUSE(governor.observe(idle_and_cool(), 900'000) == RenderTier::Conserve,
                  "a promotion granted by arithmetic on a rolled-back clock is a promotion "
                  "nothing on the machine actually justified");
    CHECK_EQ(governor.promotion_pending_ms(), 0);

    // And the dwell restarts from the new, earlier now — it does not carry the old start.
    CHECK_EQ(governor.observe(idle_and_cool(), 903'000), RenderTier::Conserve);
    CHECK_BECAUSE(governor.observe(idle_and_cool(), 905'000) == RenderTier::Ultra,
                  "having restarted, the dwell must still be able to complete; a rollback that "
                  "wedged the governor permanently would be its own bug");
}

void losing_and_regaining_a_screen_is_immediate_in_both_directions() {
    CASE("headless transitions bypass the dwell");

    BudgetGovernor governor{};
    governor.observe(idle_and_cool(), 0);
    governor.observe(idle_and_cool(), 10'000);
    CHECK_EQ(governor.tier(), RenderTier::Ultra);

    KernelReading dark = idle_and_cool();
    dark.display_present = false;
    CHECK_EQ(governor.observe(dark, 10'001), RenderTier::Headless);

    CHECK_BECAUSE(governor.observe(idle_and_cool(), 10'002) == RenderTier::Ultra,
                  "holding a blank screen for five seconds after the display returns is a bug "
                  "the operator sees; the ceiling already accounts for headroom, so there is "
                  "nothing left for a dwell to protect against");
}

void the_reason_tracks_the_current_evidence() {
    CASE("a tier that has not changed still explains itself with current evidence");

    BudgetGovernor governor{};
    KernelReading warm = idle_and_cool();
    warm.thermal = ThermalState::Warm;
    governor.observe(warm, 0);
    CHECK_EQ(governor.tier(), RenderTier::Normal);
    CHECK_EQ(governor.reason(), TierReason::ThermalWarm);

    KernelReading light = idle_and_cool();
    light.verdict = PressureVerdict::Light;
    governor.observe(light, 1'000);
    CHECK_EQ(governor.tier(), RenderTier::Normal);
    CHECK_BECAUSE(governor.reason() == TierReason::PressureLight,
                  "the chassis cooled and the scheduler got busier; showing 'warm' would send "
                  "the operator to look at a fan that is now fine");
}

}  // namespace

int main() {
    ultra_requires_evidence_from_both_sensors();
    an_unreadable_sensor_never_buys_a_richer_tier();
    an_inconsistent_payload_is_treated_as_no_reading();
    thermal_unknown_is_not_thermal_nominal();
    heat_overrides_idleness();
    pressure_maps_onto_the_ladder();
    a_saturated_machine_is_never_blanked();
    no_display_means_no_frames();
    old_telemetry_cannot_buy_a_promotion();
    old_telemetry_does_not_rescue_an_austere_tier();
    austerity_costs_monotonically_less();

    demotion_takes_effect_immediately();
    promotion_waits_out_the_dwell();
    a_pending_promotion_is_visible();
    a_fluctuating_machine_still_recovers();
    a_clock_that_goes_backwards_never_grants_a_promotion();
    losing_and_regaining_a_screen_is_immediate_in_both_directions();
    the_reason_tracks_the_current_evidence();
    return archie::visual::test::report();
}
