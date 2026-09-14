#include "archie/visual/cognition.hpp"

#include <algorithm>
#include <string_view>

namespace archie::visual {

namespace {

constexpr float kVisited = 0.35F;
constexpr float kUnreached = 0.08F;

[[nodiscard]] constexpr float out_cubic(float t) noexcept {
    const float u = 1.0F - t;
    return 1.0F - u * u * u;
}

/// How far through an animation `now_ms` is, in [0, 1].
///
/// A clock that reads *earlier* than the animation's start returns 1, not 0. The two
/// look similar and mean opposite things: 0 would hold the animation at its first frame
/// and keep asking for frames until the clock caught up — hours, after a rollback — which
/// is motion with no trigger. Landing it is the only answer that does not invent time.
[[nodiscard]] constexpr float progress(std::int64_t now_ms, std::int64_t since_ms,
                                       std::int64_t duration_ms) noexcept {
    if (duration_ms <= 0 || now_ms < since_ms) {
        return 1.0F;
    }
    const std::int64_t elapsed = now_ms - since_ms;
    if (elapsed >= duration_ms) {
        return 1.0F;
    }
    return static_cast<float>(elapsed) / static_cast<float>(duration_ms);
}

[[nodiscard]] std::optional<LoopStage> stage_named(std::string_view name) noexcept {
    if (name == "observe") return LoopStage::Observe;
    if (name == "contextualize") return LoopStage::Contextualize;
    if (name == "interpret") return LoopStage::Interpret;
    if (name == "reason") return LoopStage::Reason;
    if (name == "plan") return LoopStage::Plan;
    if (name == "select-capability") return LoopStage::SelectCapability;
    if (name == "authorize") return LoopStage::Authorize;
    if (name == "act") return LoopStage::Act;
    if (name == "verify") return LoopStage::Verify;
    if (name == "update-state") return LoopStage::UpdateState;
    if (name == "remember") return LoopStage::Remember;
    return std::nullopt;
}

[[nodiscard]] std::optional<OutcomeKind> outcome_named(std::string_view name) noexcept {
    if (name == "proceeded") return OutcomeKind::Proceeded;
    if (name == "refused") return OutcomeKind::Refused;
    if (name == "awaiting-approval") return OutcomeKind::AwaitingApproval;
    if (name == "silent") return OutcomeKind::Silent;
    if (name == "failed") return OutcomeKind::Failed;
    return std::nullopt;
}

[[nodiscard]] std::optional<PrivacyGrade> grade_named(std::string_view name) noexcept {
    if (name == "public") return PrivacyGrade::Public;
    if (name == "internal") return PrivacyGrade::Internal;
    if (name == "sensitive") return PrivacyGrade::Sensitive;
    if (name == "restricted") return PrivacyGrade::Restricted;
    if (name == "secret") return PrivacyGrade::Secret;
    return std::nullopt;
}

/// A field that serde omits for `None` may also arrive as an explicit null.
[[nodiscard]] const Value* present(const Value& object, std::string_view key) noexcept {
    const Value* value = object.find(key);
    return (value == nullptr || value->is_null()) ? nullptr : value;
}

struct FieldError {
    std::string message;
};

[[nodiscard]] std::optional<FieldError> read_outcome(const Value& object, std::string_view key,
                                                     std::optional<OutcomeKind>& out) {
    const Value* value = present(object, key);
    if (value == nullptr) {
        return std::nullopt;
    }
    if (value->kind() != Value::Kind::Object) {
        return FieldError{std::string{key} + " is not an outcome object"};
    }
    if (value->count("outcome") != 1) {
        return FieldError{std::string{key} + " has no single outcome tag"};
    }
    const std::string* tag = value->find("outcome")->as_string();
    if (tag == nullptr) {
        return FieldError{std::string{key} + " outcome tag is not a string"};
    }
    const std::optional<OutcomeKind> kind = outcome_named(*tag);
    if (!kind) {
        return FieldError{std::string{key} + " has an unknown outcome: " + *tag};
    }
    out = kind;
    return std::nullopt;
}

/// The smallest age threshold, strictly above `age_ms`, at which the tier changes.
[[nodiscard]] std::optional<std::int64_t> next_boundary(std::int64_t age_ms,
                                                        const FreshnessPolicy& policy) noexcept {
    const std::int64_t candidates[] = {-policy.future_tolerance_ms, policy.lagging_after_ms,
                                       policy.stale_after_ms, policy.expired_after_ms};
    for (const std::int64_t threshold : candidates) {
        if (threshold > age_ms) {
            return threshold;
        }
    }
    return std::nullopt;
}

}  // namespace

std::size_t ring_index(LoopStage stage) noexcept {
    for (std::size_t i = 0; i < kExecutionOrder.size(); ++i) {
        if (kExecutionOrder[i] == stage) {
            return i;
        }
    }
    return 0;
}

CognitionParse parse_cognition(const Value& projection) {
    // Each failure builds its own empty result. Copying one shared result out of a lambda
    // left GCC 16's optimiser unable to prove the optional reading was never engaged, and
    // under -O2 with -Werror that is a build failure on every optimised Linux build.
    const auto fail = [](std::string message) {
        CognitionParse failed;
        failed.error = std::move(message);
        return failed;
    };

    const std::vector<Member>* members = projection.as_object();
    if (members == nullptr) {
        return fail("projection is not an object");
    }
    for (const Member& m : *members) {
        if (projection.count(m.key) > 1) {
            return fail("projection repeats the key " + m.key);
        }
    }

    CognitionReading reading;

    const Value* trace = projection.find("trace_id");
    if (trace == nullptr || trace->as_string() == nullptr || trace->as_string()->empty()) {
        return fail("projection has no trace_id");
    }
    reading.trace_id = *trace->as_string();

    const Value* silent = projection.find("silent");
    const Value* in_progress = projection.find("in_progress");
    if (silent == nullptr || !silent->as_boolean() || in_progress == nullptr ||
        !in_progress->as_boolean()) {
        // Not defaulted to false. A projection that does not say whether the pass is
        // running is one this build does not understand, and `false` would draw it as
        // concluded.
        return fail("projection does not state silent and in_progress");
    }
    reading.silent = *silent->as_boolean();
    reading.in_progress = *in_progress->as_boolean();
    if (reading.silent && reading.in_progress) {
        return fail("projection claims a pass is both silent and in progress");
    }

    if (const Value* stage = present(projection, "stage")) {
        const std::string* name = stage->as_string();
        const std::optional<LoopStage> parsed = name ? stage_named(*name) : std::nullopt;
        if (!parsed) {
            return fail("projection has an unknown stage");
        }
        reading.stage = parsed;
    }

    if (const Value* privacy = present(projection, "privacy")) {
        const std::string* name = privacy->as_string();
        const std::optional<PrivacyGrade> parsed = name ? grade_named(*name) : std::nullopt;
        if (!parsed) {
            return fail("projection has an unknown privacy grade");
        }
        reading.privacy = parsed;
    }

    const std::array<std::pair<std::string_view, std::optional<OutcomeKind>*>, 4> outcomes{{
        {"authorization", &reading.authorization},
        {"execution", &reading.execution},
        {"verification", &reading.verification},
        {"memory", &reading.memory},
    }};
    for (const auto& [key, slot] : outcomes) {
        if (const std::optional<FieldError> error = read_outcome(projection, key, *slot)) {
            return fail(error->message);
        }
    }

    if (const Value* count = present(projection, "context_source_count")) {
        const std::optional<double> number = count->as_number();
        if (!number || *number < 0.0 || *number > 1'000'000.0 ||
            *number != static_cast<double>(static_cast<int>(*number))) {
            return fail("projection has an invalid context_source_count");
        }
        reading.context_source_count = static_cast<int>(*number);
    }

    CognitionParse result;
    result.reading = std::move(reading);
    return result;
}

Activity activity_of(const CognitionReading& reading) noexcept {
    if (reading.in_progress) {
        return Activity::Thinking;
    }
    if (reading.authorization == OutcomeKind::AwaitingApproval) {
        return Activity::AwaitingApproval;
    }
    if (reading.authorization == OutcomeKind::Refused ||
        reading.execution == OutcomeKind::Refused) {
        return Activity::Refused;
    }
    if (reading.execution == OutcomeKind::Failed || reading.verification == OutcomeKind::Failed) {
        return Activity::Failed;
    }
    if (reading.execution == OutcomeKind::Proceeded) {
        return Activity::Acted;
    }
    return Activity::Silent;
}

bool is_adverse(Activity activity) noexcept {
    switch (activity) {
        case Activity::Refused:
        case Activity::Failed:
            return true;
        case Activity::Unmeasured:
        case Activity::Thinking:
        case Activity::Silent:
        case Activity::Acted:
        case Activity::AwaitingApproval:
            return false;
    }
    return false;
}

void CognitionMotion::observe(const CognitionReading& reading, std::int64_t age_ms,
                              std::int64_t now_ms) {
    const std::int64_t produced_at = now_ms - age_ms;
    const bool same_trace = m_reading.has_value() && m_reading->trace_id == reading.trace_id;

    if (same_trace && produced_at < m_produced_at_ms) {
        // An older state of this pass arriving late. Drawing it would walk the ring
        // backwards through stages the pass has already left.
        return;
    }

    const bool stale_on_arrival = !may_animate(classify_age(age_ms, m_policy.freshness).tier);
    const Activity activity = activity_of(reading);

    // Motion starts only from a difference. The same projection published again changes
    // no stage and no activity, so it moves nothing — it only refreshes the production
    // time below, because the source is vouching for the same state again.
    const bool stage_changed = !same_trace || m_reading->stage != reading.stage;
    if (reading.stage.has_value() && stage_changed) {
        if (same_trace && m_reading->stage.has_value()) {
            const bool mid_flight =
                m_travelling && !m_snap &&
                progress(now_ms, m_travel_since_ms, m_policy.transition_ms) < 1.0F;
            if (mid_flight) {
                // Obey the new instruction from wherever the marker actually is, the
                // same rule the camera follows. Snapping back to the old start would
                // draw a jump that never happened.
                m_from = marker_at(now_ms);
                ++m_superseded;
            } else {
                m_from = static_cast<float>(ring_index(*m_reading->stage));
            }
            m_to = static_cast<float>(ring_index(*reading.stage));
            m_travelling = true;
            m_travel_since_ms = now_ms;
            m_snap = stale_on_arrival || m_policy.reduced_motion;
        } else {
            // A new pass did not travel from where the previous one stopped.
            m_travelling = false;
        }
    } else if (!reading.stage.has_value()) {
        m_travelling = false;
    }

    if (!same_trace || activity != m_activity) {
        m_activity_since_ms = now_ms;
    }

    m_reading = reading;
    m_activity = activity;
    m_produced_at_ms = produced_at;
}

void CognitionMotion::observe_unmeasured(std::int64_t now_ms) {
    if (!m_reading.has_value() && m_activity == Activity::Unmeasured) {
        return;
    }
    m_reading.reset();
    m_activity = Activity::Unmeasured;
    m_travelling = false;
    m_activity_since_ms = now_ms;
}

float CognitionMotion::marker_at(std::int64_t now_ms) const noexcept {
    const float t = progress(now_ms, m_travel_since_ms, m_policy.transition_ms);
    return m_from + (m_to - m_from) * out_cubic(t);
}

CognitionFrame CognitionMotion::frame(std::int64_t now_ms, RenderTier tier) const {
    const RenderProfile profile = profile_of(tier);
    const bool presenting = profile.frame_interval_ms > 0;

    CognitionFrame frame;
    frame.superseded = m_superseded;
    frame.activity = m_activity;

    const auto schedule = [&](bool transitioning, bool breathing,
                              std::optional<std::int64_t> boundary_at) {
        if (!presenting) {
            // Headless: the model stays correct for the moment a screen returns, and
            // not one frame is produced for a screen that is not there.
            frame.next_frame_at_ms.reset();
            return;
        }
        std::optional<std::int64_t> next{};
        if (transitioning) {
            next = now_ms + profile.frame_interval_ms;
        } else if (breathing) {
            next = now_ms + std::max(profile.frame_interval_ms, m_policy.ambient_frame_interval_ms);
        }
        if (boundary_at.has_value()) {
            next = next.has_value() ? std::min(*next, *boundary_at) : boundary_at;
        }
        frame.next_frame_at_ms = next;
    };

    if (!m_reading.has_value()) {
        frame.freshness = Freshness::Expired;
        frame.confidence = confidence_of(Freshness::Expired);
        const bool may_move = presenting && !m_policy.reduced_motion;
        const float t = may_move ? progress(now_ms, m_activity_since_ms, m_policy.glyph_fade_ms)
                                 : 1.0F;
        frame.glyph_opacity = out_cubic(t);
        frame.animate = t < 1.0F;
        schedule(frame.animate, false, std::nullopt);
        return frame;
    }

    const std::int64_t age_ms = now_ms - m_produced_at_ms;
    const FreshnessReading freshness = classify_age(age_ms, m_policy.freshness);
    frame.freshness = freshness.tier;
    frame.confidence = confidence_of(freshness.tier);
    frame.privacy = m_reading->privacy;

    const bool motion_allowed =
        presenting && may_animate(freshness.tier) && !m_policy.reduced_motion;

    bool travelling = false;
    float rise = 1.0F;
    if (m_travelling && !m_snap && motion_allowed) {
        const float t = progress(now_ms, m_travel_since_ms, m_policy.transition_ms);
        if (t < 1.0F) {
            travelling = true;
            rise = out_cubic(t);
            frame.marker_position = m_from + (m_to - m_from) * rise;
        }
    }

    if (m_reading->stage.has_value()) {
        const std::size_t current = ring_index(*m_reading->stage);
        frame.current_index = current;
        for (std::size_t i = 0; i < kLoopStageCount; ++i) {
            if (i < current) {
                frame.stage_intensity[i] = kVisited;
            } else if (i == current) {
                frame.stage_intensity[i] = kVisited + (1.0F - kVisited) * rise;
            } else {
                frame.stage_intensity[i] = kUnreached;
            }
        }
    }

    bool fading = false;
    if (motion_allowed) {
        const float t = progress(now_ms, m_activity_since_ms, m_policy.glyph_fade_ms);
        frame.glyph_opacity = out_cubic(t);
        fading = t < 1.0F;
    }

    bool breathing = false;
    if (m_activity == Activity::Thinking && motion_allowed && profile.ambient_motion &&
        now_ms >= m_activity_since_ms) {
        const std::int64_t period = std::max<std::int64_t>(1, m_policy.breath_period_ms);
        const std::int64_t phase_ms = (now_ms - m_activity_since_ms) % period;
        const float phase = static_cast<float>(phase_ms) / static_cast<float>(period);
        const float triangle = phase < 0.5F ? phase * 2.0F : (1.0F - phase) * 2.0F;
        frame.breath = triangle * triangle * (3.0F - 2.0F * triangle);
        breathing = true;
    }

    frame.animate = travelling || fading || breathing;

    std::optional<std::int64_t> boundary_at{};
    if (const std::optional<std::int64_t> threshold = next_boundary(age_ms, m_policy.freshness)) {
        boundary_at = m_produced_at_ms + *threshold;
    }
    schedule(travelling || fading, breathing, boundary_at);
    return frame;
}

}  // namespace archie::visual
