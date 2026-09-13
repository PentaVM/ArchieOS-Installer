#pragma once

// What Archie is doing, drawn at the cost of what actually changed.
//
// Cerebra already records every pass through its loop as a `CognitiveTrace`, and the
// Rust side projects that into a `HudProjection`: which stage the pass reached, under
// which privacy grade, whether it is still running, whether it chose to stay silent,
// what policy said. This header turns that projection into motion a person can read,
// and it follows the same rules as the rest of this library, because the ways a status
// display lies do not change with the thing being displayed.
//
// Three lies in particular shape it.
//
// The first is synthetic motion. A spinner that turns whether or not anything is
// happening teaches the viewer that motion means nothing. So every transition here is
// derived from a difference between two readings — observing the same projection twice
// starts nothing — and a pass that is genuinely still running is allowed a slow breath
// only while its reading is fresh. When the source stops reporting, the breath stops
// with it, exactly as `freshness.hpp` requires of every other animated thing.
//
// The second is cost that has nothing to do with change. A render loop that redraws at
// sixty frames a second to show a screen where nothing moved spends the machine it is
// reporting on. `CognitionFrame::next_frame_at_ms` is the answer to "when does anything
// on this panel next look different": the tier's refresh interval while something is
// travelling, a slow ambient interval while a pass breathes, a single frame at the next
// freshness boundary when the panel is still, and nothing at all otherwise. An idle
// panel costs one frame per boundary a reading crosses, not one per sixteen
// milliseconds.
//
// The third is the empty panel that looks healthy. A panel that has never received a
// projection is `Activity::Unmeasured` and draws differently from any concluded pass,
// because "we do not know what Archie is doing" and "Archie finished and said nothing"
// call for opposite responses from whoever is watching.
//
// Like everything else here: no clock, no Qt, no socket. Time is a parameter.

#include <array>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>

#include "archie/visual/budget.hpp"
#include "archie/visual/freshness.hpp"
#include "archie/visual/value.hpp"

namespace archie::visual {

/// Mirrors `archie_observability::cognition::Stage`, in declaration order.
enum class LoopStage {
    Observe,
    Contextualize,
    Interpret,
    Reason,
    Plan,
    SelectCapability,
    Authorize,
    Act,
    Verify,
    UpdateState,
    Remember,
};

inline constexpr std::size_t kLoopStageCount = 11;

/// The order the loop actually runs in, which is not the enum's declaration order.
///
/// Cerebra consults the floor (`Interpret`) *before* it assembles context, because
/// deciding whether to speak touches nothing private and deciding what to say means
/// opening the vault. A ring drawn in declaration order would show context being
/// gathered before the decision that is supposed to gate it, and so would draw the one
/// ordering the architecture exists to prevent.
inline constexpr std::array<LoopStage, kLoopStageCount> kExecutionOrder{
    LoopStage::Observe,     LoopStage::Interpret,        LoopStage::Contextualize,
    LoopStage::Reason,      LoopStage::Plan,             LoopStage::SelectCapability,
    LoopStage::Authorize,   LoopStage::Act,              LoopStage::Verify,
    LoopStage::UpdateState, LoopStage::Remember,
};

/// Position of a stage on the ring, in execution order.
[[nodiscard]] std::size_t ring_index(LoopStage stage) noexcept;

/// Mirrors the tag of `archie_observability::cognition::StageOutcome`.
enum class OutcomeKind {
    Proceeded,
    Refused,
    AwaitingApproval,
    Silent,
    Failed,
};

/// Mirrors `archie_types::PrivacyGrade`, least to most restrictive.
enum class PrivacyGrade {
    Public,
    Internal,
    Sensitive,
    Restricted,
    Secret,
};

/// The part of a `HudProjection` this panel draws.
///
/// Deliberately smaller than the projection. The goal label, the model identifier and
/// the reason codes are left out: they are safe to show as text elsewhere, but nothing
/// about *motion* depends on them, and a module that never holds a string cannot be the
/// place one leaks from.
struct CognitionReading {
    std::string trace_id;
    std::optional<LoopStage> stage{};
    std::optional<PrivacyGrade> privacy{};
    std::optional<OutcomeKind> authorization{};
    std::optional<OutcomeKind> execution{};
    std::optional<OutcomeKind> verification{};
    std::optional<OutcomeKind> memory{};
    bool silent = false;
    bool in_progress = false;
    int context_source_count = 0;

    friend bool operator==(const CognitionReading&, const CognitionReading&) = default;
};

/// The result of reading a decoded projection.
///
/// Carries a reason on failure rather than a bare empty optional: an operator looking at
/// an `Unmeasured` panel needs to know whether the daemon is silent or speaking a
/// version of the protocol this build does not understand.
struct CognitionParse {
    std::optional<CognitionReading> reading{};
    std::string error{};
};

/// Read one `HudProjection` as the bridge decoded it.
///
/// Strict in the way `ingest.hpp` is strict. An unknown stage, outcome or grade is an
/// error rather than a best guess, a missing `silent` or `in_progress` is an error rather
/// than `false`, and a repeated key is an error rather than first-one-wins. A panel that
/// guesses draws a confident state for a projection it did not understand.
[[nodiscard]] CognitionParse parse_cognition(const Value& projection);

/// What a person should read from the panel.
enum class Activity {
    /// No usable projection has ever arrived. Never drawn like a concluded pass.
    Unmeasured,
    /// The pass is still running.
    Thinking,
    /// The pass concluded and chose to do nothing outward.
    Silent,
    /// The pass did something.
    Acted,
    /// A legitimate action is waiting on a person.
    AwaitingApproval,
    /// A rule refused the pass.
    Refused,
    /// The pass tried and failed for a reason that was not a policy.
    Failed,
};

/// The single activity a reading implies.
///
/// Precedence follows the Rust invariants rather than field order. A refused pass is
/// also reported `silent` by the trace, because nothing outward happened; drawing it as
/// a quiet pass would hide the refusal, so refusal outranks silence. An approval hold
/// outranks refusal because it is the one state that is waiting on the viewer.
[[nodiscard]] Activity activity_of(const CognitionReading& reading) noexcept;

/// Whether this activity reads as something having gone wrong.
[[nodiscard]] bool is_adverse(Activity activity) noexcept;

struct CognitionMotionPolicy {
    /// How long the marker takes to travel between stages.
    std::int64_t transition_ms = 420;
    /// How long a change of activity takes to fade in.
    std::int64_t glyph_fade_ms = 260;
    /// One full breath of a running pass.
    std::int64_t breath_period_ms = 1600;
    /// Refresh used for the breath. A slow swell reads identically at fifteen frames a
    /// second, and ambient motion is exactly the spending `budget.hpp` separates out.
    int ambient_frame_interval_ms = 66;
    /// Accessibility: state changes land at once and nothing breathes. The drawing
    /// changes — no travel, no swell — rather than a label announcing reduced motion.
    bool reduced_motion = false;
    /// The HUD is a local projection refreshed on every stage, so its lag window is
    /// short; the stale window is long enough for a real inference call to finish.
    FreshnessPolicy freshness{2'000, 15'000, 120'000, 2'000};
};

/// Everything the renderer needs, already decided.
struct CognitionFrame {
    Activity activity = Activity::Unmeasured;
    Freshness freshness = Freshness::Expired;
    std::optional<PrivacyGrade> privacy{};

    /// Brightness of each stage on the ring, indexed by `ring_index`. Stages the pass has
    /// passed through are dim, the current one is full, the rest are barely there.
    std::array<float, kLoopStageCount> stage_intensity{};
    /// Ring index of the stage the pass has reached, if any.
    std::optional<std::size_t> current_index{};

    /// Where the travelling marker is, as a fractional ring index. Present only while it
    /// travels; a panel at rest has no marker to draw.
    std::optional<float> marker_position{};
    /// Fade-in of the current activity glyph, in [0, 1].
    float glyph_opacity = 1.0F;
    /// Swell of a running pass, in [0, 1]. Zero whenever it may not animate.
    float breath = 0.0F;

    /// Multiplier for opacity and saturation, from `confidence_of`.
    float confidence = 0.0F;
    /// Whether anything in this frame is in motion.
    bool animate = false;

    /// When the panel next looks different. Absent means: draw nothing until new input.
    std::optional<std::int64_t> next_frame_at_ms{};

    /// How many transitions were overtaken by a newer reading before landing. Non-zero
    /// means the panel skipped stages it could not show in time, and may say so.
    int superseded = 0;
};

/// Holds the last reading and when it changed. Derives every frame from those alone.
class CognitionMotion final {
public:
    CognitionMotion() = default;
    explicit CognitionMotion(CognitionMotionPolicy policy) : m_policy(policy) {}

    /// Feed one projection. `age_ms` is how old it already was on arrival.
    ///
    /// The only way to start motion. A reading identical to the last one changes
    /// nothing; a reading from a different trace restarts the ring without travel,
    /// because a new pass did not move from where the previous one stopped.
    void observe(const CognitionReading& reading, std::int64_t age_ms, std::int64_t now_ms);

    /// The source went away or spoke something unparseable. The panel admits it.
    void observe_unmeasured(std::int64_t now_ms);

    /// The frame for `now_ms` under `tier`. Pure: the same inputs give the same frame.
    [[nodiscard]] CognitionFrame frame(std::int64_t now_ms, RenderTier tier) const;

    [[nodiscard]] const CognitionMotionPolicy& policy() const noexcept { return m_policy; }

private:
    [[nodiscard]] float marker_at(std::int64_t now_ms) const noexcept;

    CognitionMotionPolicy m_policy{};

    std::optional<CognitionReading> m_reading{};
    Activity m_activity = Activity::Unmeasured;
    /// When the source produced the current reading, on our clock.
    std::int64_t m_produced_at_ms = 0;

    /// When the current activity began.
    std::int64_t m_activity_since_ms = 0;

    /// Marker travel: from a fractional position to a ring index.
    bool m_travelling = false;
    float m_from = 0.0F;
    float m_to = 0.0F;
    std::int64_t m_travel_since_ms = 0;
    /// The travel was started from a stale reading and must not move.
    bool m_snap = false;

    int m_superseded = 0;
};

}  // namespace archie::visual
