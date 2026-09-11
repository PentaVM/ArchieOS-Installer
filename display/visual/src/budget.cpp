#include "archie/visual/budget.hpp"

namespace archie::visual {
namespace {

/// Keep the austerer of two verdicts, preferring the first on a tie.
///
/// Thermal is passed first by the caller, so when both sources land on the same tier
/// the operator is told about the physical fact — a warm chassis — rather than about
/// scheduler pressure, which is usually its consequence.
TierDecision restrict_to(TierDecision current, TierDecision other) noexcept {
    if (static_cast<int>(other.tier) > static_cast<int>(current.tier)) {
        return other;
    }
    return current;
}

/// What kernel pressure alone permits.
TierDecision from_pressure(const KernelReading& reading) noexcept {
    if (!reading.pressure_available || !reading.verdict.has_value()) {
        // Either the sensor is absent, or the projection declared it available and then
        // carried no value — an inconsistent payload. Both mean the same thing here:
        // there is no evidence of headroom, so none may be assumed.
        return TierDecision{RenderTier::Normal, TierReason::TelemetryUnavailable};
    }

    switch (*reading.verdict) {
        case PressureVerdict::Idle:
            // The only path to Ultra. `PressureVerdict::Idle` is documented upstream as
            // the state in which cinematic rendering is welcome, so this is the
            // scheduler's own judgement rather than the display's opinion of itself.
            return TierDecision{RenderTier::Ultra, TierReason::PressureIdle};
        case PressureVerdict::Light:
            return TierDecision{RenderTier::Normal, TierReason::PressureLight};
        case PressureVerdict::Contended:
            // Upstream: "Real contention. Yield idle work." Post-processing on a
            // status display is idle work by any honest reading.
            return TierDecision{RenderTier::Conserve, TierReason::PressureContended};
        case PressureVerdict::Saturated:
            // Deliberately Conserve and not Headless. A saturated machine is exactly
            // when an operator most needs to see what it is doing, and an observatory
            // that blanks itself under load is useless at the only moment it matters.
            return TierDecision{RenderTier::Conserve, TierReason::PressureSaturated};
    }
    return TierDecision{RenderTier::Normal, TierReason::TelemetryUnavailable};
}

/// What the thermal reading alone permits. Thermal only ever restricts.
TierDecision from_thermal(const KernelReading& reading) noexcept {
    if (!reading.thermal_available || !reading.thermal.has_value()) {
        return TierDecision{RenderTier::Normal, TierReason::TelemetryUnavailable};
    }

    switch (*reading.thermal) {
        case ThermalState::Unknown:
            // No usable trip point, so no judgement. Not the same as Nominal, and the
            // difference is the whole reason the upstream enum carries both.
            return TierDecision{RenderTier::Normal, TierReason::TelemetryUnavailable};
        case ThermalState::Nominal:
            // Imposes no ceiling of its own; whether Ultra is reached is pressure's
            // decision.
            return TierDecision{RenderTier::Ultra, TierReason::PressureIdle};
        case ThermalState::Warm:
            return TierDecision{RenderTier::Normal, TierReason::ThermalWarm};
        case ThermalState::Throttling:
            return TierDecision{RenderTier::Conserve, TierReason::ThermalThrottling};
        case ThermalState::Critical:
            // The firmware may cut power. Everything optional stops.
            return TierDecision{RenderTier::Conserve, TierReason::ThermalCritical};
    }
    return TierDecision{RenderTier::Normal, TierReason::TelemetryUnavailable};
}

}  // namespace

TierDecision ceiling_for(const KernelReading& reading) noexcept {
    if (!reading.display_present) {
        // Not a performance judgement: there is simply nobody to show frames to.
        return TierDecision{RenderTier::Headless, TierReason::DisplayAbsent};
    }

    TierDecision decision = restrict_to(from_thermal(reading), from_pressure(reading));

    // Age is the last word. A reading can be available, internally consistent and say
    // "idle", and still be four minutes old — in which case it describes a machine that
    // no longer exists. Old evidence cannot buy a richer tier, though it is still
    // allowed to keep an austere one: if the last thing we heard was "saturated", the
    // cautious reading of silence is not "it must have recovered".
    switch (reading.freshness) {
        case Freshness::Live:
        case Freshness::Lagging:
            break;
        case Freshness::Stale:
        case Freshness::Expired:
            decision = restrict_to(
                decision, TierDecision{RenderTier::Normal, TierReason::TelemetryStale});
            break;
    }

    return decision;
}

RenderProfile profile_of(RenderTier tier) noexcept {
    switch (tier) {
        case RenderTier::Ultra:
            return RenderProfile{16, 4, 4000, 6000, true, true};
        case RenderTier::Normal:
            return RenderProfile{16, 2, 1500, 2000, false, true};
        case RenderTier::Conserve:
            // Half refresh, no multisampling, no particles, no idle motion. Everything
            // that remains is information.
            return RenderProfile{33, 0, 400, 0, false, false};
        case RenderTier::Headless:
            return RenderProfile{0, 0, 0, 0, false, false};
    }
    return RenderProfile{16, 2, 1500, 2000, false, true};
}

RenderTier BudgetGovernor::observe(const KernelReading& reading, std::int64_t now_ms) noexcept {
    const TierDecision decision = ceiling_for(reading);

    // Appearing or losing a display is a fact about the world, not a claim about
    // headroom, so it takes effect at once in both directions. Waiting five seconds
    // before drawing anything on a screen that just woke up would be a bug the operator
    // sees, and holding frames for a screen that is gone wastes the machine.
    if (decision.tier == RenderTier::Headless || m_tier == RenderTier::Headless) {
        m_tier = decision.tier;
        m_reason = decision.reason;
        m_candidate.reset();
        m_pending_ms = 0;
        return m_tier;
    }

    const int ceiling = static_cast<int>(decision.tier);
    const int held = static_cast<int>(m_tier);

    if (ceiling > held) {
        // More austere than what we are doing. Take it this frame.
        m_tier = decision.tier;
        m_reason = decision.reason;
        m_candidate.reset();
        m_pending_ms = 0;
        return m_tier;
    }

    if (ceiling == held) {
        // No change of tier, but the explanation may have changed — thermal warmth
        // giving way to scheduler pressure, say — and the operator should see the
        // current reason rather than a stale one.
        m_reason = decision.reason;
        m_candidate.reset();
        m_pending_ms = 0;
        return m_tier;
    }

    // A richer tier is on offer. It has to hold for the dwell before it is believed.
    if (!m_candidate.has_value()) {
        m_candidate = decision.tier;
        m_candidate_reason = decision.reason;
        m_candidate_since_ms = now_ms;
        m_pending_ms = 0;
        return m_tier;
    }

    // A promotion is only worth what the *worst* sample in the window justifies. If the
    // machine offered Ultra and then merely Normal, the window still supports Normal,
    // so the candidate is lowered and the clock keeps running rather than restarting.
    // Restarting on every fluctuation would mean a machine that is genuinely quiet but
    // slightly uneven never recovers its tier at all.
    if (static_cast<int>(decision.tier) > static_cast<int>(*m_candidate)) {
        m_candidate = decision.tier;
        m_candidate_reason = decision.reason;
    }

    if (now_ms < m_candidate_since_ms) {
        // The clock went backwards. The elapsed time cannot be computed, and the one
        // thing that must not happen is a promotion granted by arithmetic on a bad
        // clock. Restart the dwell from here and make the machine earn it again.
        m_candidate_since_ms = now_ms;
        m_pending_ms = 0;
        return m_tier;
    }

    m_pending_ms = now_ms - m_candidate_since_ms;
    if (m_pending_ms >= m_policy.promotion_dwell_ms) {
        m_tier = *m_candidate;
        m_reason = m_candidate_reason;
        m_candidate.reset();
        m_pending_ms = 0;
    }
    return m_tier;
}

}  // namespace archie::visual
