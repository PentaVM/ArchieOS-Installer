#pragma once

// What the observatory may spend on drawing, decided from what the machine actually
// reports rather than from a frame-rate counter.
//
// The usual approach is to render, measure the resulting frame time and back off when
// it gets bad. That reacts *after* the damage: by the time frames are late the machine
// is already contended, and on a node that is also the thing being observed, the
// observatory has become part of the problem it is displaying.
//
// ArchieOS already reads pressure and thermal telemetry from the kernel for scheduling
// decisions. The same evidence decides the render budget here, which means the display
// yields for the same reason and at the same moment the scheduler does — and it means
// the tier shown on screen is a real statement about the machine, not a guess.
//
// The governing rule is that a rich tier requires *positive evidence of headroom*.
// Absence of evidence is not evidence of headroom: an unreadable sensor, a stale
// reading and a saturated machine all land on the cautious side, because the cost of
// being wrong is asymmetric. Spending a GPU the machine needed is a real harm; drawing
// two fewer bloom passes on a machine that could have afforded them is not.

#include <cstdint>
#include <optional>

#include "archie/visual/freshness.hpp"

namespace archie::visual {

/// Kernel pressure, mirroring `archie_observability::kernel::psi::PressureVerdict`.
///
/// Ordered by increasing contention, matching the Rust definition, so the two can be
/// compared without a translation table when the projection is parsed.
enum class PressureVerdict {
    Idle,
    Light,
    Contended,
    Saturated,
};

/// Thermal state, mirroring `archie_observability::kernel::thermal::ThermalState`.
///
/// `Unknown` is first and is genuinely distinct from `Nominal`: it means no usable trip
/// point was found, so no judgement is available. It must never be read as "fine".
enum class ThermalState {
    Unknown,
    Nominal,
    Warm,
    Throttling,
    Critical,
};

/// How much the observatory is allowed to draw.
///
/// Ordered by increasing austerity so `austerer_of` is a comparison rather than a
/// table, and so a tier inserted in the middle keeps the ordering meaningful.
enum class RenderTier {
    /// The machine is measurably idle. Full cinematic treatment: depth of field,
    /// volumetric light, high particle counts, 4x multisampling.
    Ultra,
    /// The default, and the tier used whenever the truth is unknown. A complete,
    /// good-looking scene with the expensive optional passes switched off.
    Normal,
    /// The machine needs its GPU back. Flat shading, no post-processing, halved
    /// refresh, and a hard cap on visible geometry.
    Conserve,
    /// Nothing is being presented — no display, or the compositor is away. The scene
    /// graph is still maintained so that state is correct the moment a screen returns,
    /// but no frames are produced.
    Headless,
};

/// Why the governor settled where it did.
///
/// Carried so the observatory can say "Conserve — CPU saturated" instead of showing a
/// degraded scene with no explanation. A display that quietly gets worse teaches its
/// operator to distrust it.
enum class TierReason {
    DisplayAbsent,
    ThermalCritical,
    ThermalThrottling,
    ThermalWarm,
    PressureSaturated,
    PressureContended,
    PressureLight,
    PressureIdle,
    /// The sensor could not be read at all. Distinct from a bad reading.
    TelemetryUnavailable,
    /// The reading exists but is too old to justify spending on.
    TelemetryStale,
};

/// One kernel telemetry sample, as the display receives it.
///
/// Mirrors the fields of `archie_displayd::kernel::KernelSnapshot` that bear on cost.
/// The availability flags are kept separate from the values because the projection
/// keeps them separate, and collapsing them would lose the distinction between "the
/// machine is idle" and "we cannot tell whether the machine is idle".
struct KernelReading {
    bool display_present = true;

    bool pressure_available = false;
    std::optional<PressureVerdict> verdict{};

    bool thermal_available = false;
    std::optional<ThermalState> thermal{};

    /// How current this sample is. A perfectly readable sensor whose last successful
    /// read was four minutes ago is not evidence about the machine right now.
    Freshness freshness = Freshness::Live;
};

/// The instantaneous verdict, before hysteresis.
struct TierDecision {
    RenderTier tier = RenderTier::Normal;
    TierReason reason = TierReason::TelemetryUnavailable;
};

/// The more austere of two tiers.
[[nodiscard]] constexpr RenderTier austerer_of(RenderTier a, RenderTier b) noexcept {
    return (static_cast<int>(a) >= static_cast<int>(b)) ? a : b;
}

/// The instantaneous ceiling implied by one sample, with no memory.
///
/// Pure, so the whole policy can be checked exhaustively without constructing a
/// governor or advancing a clock.
[[nodiscard]] TierDecision ceiling_for(const KernelReading& reading) noexcept;

/// Concrete numbers a renderer can act on.
struct RenderProfile {
    /// Target interval between frames. 0 means no frames at all.
    int frame_interval_ms = 16;
    /// Multisample count; 0 disables multisampling.
    int msaa_samples = 2;
    /// Hard cap on nodes submitted for drawing. Above this the scene must aggregate
    /// rather than draw more, which is what the level-of-detail rules are for.
    ///
    /// These numbers are not yet validated end to end, and the honest statement of what
    /// is known is worth more than a confident one. `archie-visual-bench` measures what
    /// deciding a frame costs before anything is drawn: roughly 0.7ms at 400 nodes, 3ms
    /// at 1500 and 9ms at 4000 on a loaded development machine. So at the top cap the
    /// model alone can take over half a 16ms frame, leaving the rest for the renderer.
    ///
    /// The caps are deliberately left where they are rather than tuned down to fit that
    /// figure. The measurement comes from one Windows laptop under heavy disk pressure,
    /// not from the Arch node the observatory actually runs on, and the cost of drawing
    /// those nodes has not been measured at all — so a cap chosen to fit half of an
    /// unmeasured budget would be exactly the guess this file exists to avoid. What the
    /// numbers do establish is that the model's own cost is not free at the top end, and
    /// that the pair needs measuring together on real hardware before either is trusted.
    int max_visible_nodes = 1500;
    /// Cap on simultaneously live particles across every emitter.
    int max_particles = 2000;
    /// Whether the expensive post-processing chain runs.
    bool post_processing = false;
    /// Whether ambient idle motion (drift, breathing, orbit) runs. Kept separate from
    /// post-processing because it costs CPU rather than GPU, and the two run out at
    /// different times.
    bool ambient_motion = true;
};

[[nodiscard]] RenderProfile profile_of(RenderTier tier) noexcept;

/// How the governor is allowed to move between tiers.
struct BudgetPolicy {
    /// How long a richer tier must be continuously justified before it is adopted.
    ///
    /// Only promotion waits. Demotion is immediate, and the asymmetry is the point: a
    /// machine that has just become busy needs its resources back this frame, whereas a
    /// machine that has just gone quiet loses nothing by staying austere for another
    /// few seconds. Without the delay, a workload that alternates between bursts makes
    /// the display oscillate between tiers, which is both expensive and — because the
    /// tier is shown to the operator — actively confusing.
    std::int64_t promotion_dwell_ms = 5000;
};

/// Applies hysteresis to the instantaneous ceiling.
///
/// Holds no clock: every method takes the current time, so a test can drive it across
/// hours, or backwards, in a few microseconds.
class BudgetGovernor final {
public:
    BudgetGovernor() = default;
    explicit BudgetGovernor(BudgetPolicy policy) : m_policy(policy) {}

    /// Feed one sample. Returns the tier in force after it.
    RenderTier observe(const KernelReading& reading, std::int64_t now_ms) noexcept;

    [[nodiscard]] RenderTier tier() const noexcept { return m_tier; }
    [[nodiscard]] TierReason reason() const noexcept { return m_reason; }
    [[nodiscard]] RenderProfile profile() const noexcept { return profile_of(m_tier); }

    /// How long the pending promotion has been waiting, or 0 when none is pending.
    ///
    /// Exposed so the interface can show that a richer tier is on its way, rather than
    /// appearing to ignore a machine that has visibly gone quiet.
    [[nodiscard]] std::int64_t promotion_pending_ms() const noexcept { return m_pending_ms; }

private:
    BudgetPolicy m_policy{};
    RenderTier m_tier = RenderTier::Normal;
    TierReason m_reason = TierReason::TelemetryUnavailable;

    /// The richer tier currently being waited out, if any.
    std::optional<RenderTier> m_candidate{};
    TierReason m_candidate_reason = TierReason::TelemetryUnavailable;
    std::int64_t m_candidate_since_ms = 0;
    std::int64_t m_pending_ms = 0;
};

}  // namespace archie::visual
