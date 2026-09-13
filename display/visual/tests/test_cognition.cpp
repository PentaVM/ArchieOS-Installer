// What Archie is doing, and what drawing it is allowed to cost.

#include "archie/visual/cognition.hpp"

#include "harness.hpp"

using namespace archie::visual;

namespace {

constexpr std::int64_t T0 = 1'000'000;
const CognitionMotionPolicy kPolicy{};

CognitionReading reading(std::string trace, LoopStage stage, bool in_progress, bool silent) {
    CognitionReading r;
    r.trace_id = std::move(trace);
    r.stage = stage;
    r.in_progress = in_progress;
    r.silent = silent;
    r.privacy = PrivacyGrade::Internal;
    return r;
}

CognitionReading thinking(std::string trace, LoopStage stage) {
    return reading(std::move(trace), stage, true, false);
}

bool frames_equal(const CognitionFrame& a, const CognitionFrame& b) {
    return a.activity == b.activity && a.freshness == b.freshness && a.privacy == b.privacy &&
           a.stage_intensity == b.stage_intensity && a.current_index == b.current_index &&
           a.marker_position == b.marker_position && a.glyph_opacity == b.glyph_opacity &&
           a.breath == b.breath && a.confidence == b.confidence && a.animate == b.animate &&
           a.next_frame_at_ms == b.next_frame_at_ms && a.superseded == b.superseded;
}

void a_panel_that_has_heard_nothing_asks_for_nothing() {
    CASE("nothing observed draws nothing and requests no frame");

    const CognitionMotion motion;
    const CognitionFrame frame = motion.frame(T0, RenderTier::Normal);

    CHECK_EQ(frame.activity, Activity::Unmeasured);
    CHECK(!frame.animate);
    CHECK_BECAUSE(!frame.next_frame_at_ms.has_value(),
                  "an empty panel has no reason to wake the renderer, ever");
    CHECK(!frame.marker_position.has_value());
}

void unmeasured_never_renders_like_a_concluded_pass() {
    CASE("unmeasured and a finished silent pass are visibly different");

    const CognitionMotion empty;
    CognitionMotion concluded;
    concluded.observe(reading("trace-a", LoopStage::Interpret, false, true), 0, T0);

    const CognitionFrame a = empty.frame(T0 + 5'000, RenderTier::Normal);
    const CognitionFrame b = concluded.frame(T0 + 5'000, RenderTier::Normal);

    CHECK(a.activity != b.activity);
    CHECK_BECAUSE(a.stage_intensity != b.stage_intensity,
                  "'we do not know' and 'it finished and said nothing' call for opposite "
                  "responses, so they may not share a drawing");
    CHECK(a.confidence < b.confidence);
}

void only_a_change_starts_motion() {
    CASE("observing the same projection twice starts nothing");

    CognitionMotion motion;
    motion.observe(thinking("trace-a", LoopStage::Observe), 0, T0);
    motion.observe(thinking("trace-a", LoopStage::Interpret), 0, T0 + 100);

    const std::int64_t landed = T0 + 100 + kPolicy.transition_ms + kPolicy.glyph_fade_ms;
    CHECK(!motion.frame(landed, RenderTier::Conserve).animate);

    motion.observe(thinking("trace-a", LoopStage::Interpret), 0, landed);
    const CognitionFrame frame = motion.frame(landed, RenderTier::Conserve);
    CHECK_BECAUSE(!frame.marker_position.has_value(),
                  "a repeated reading is not an event; a marker here would be a pulse for "
                  "something that did not happen");
    CHECK_EQ(frame.superseded, 0);
}

void a_transition_asks_for_frames_at_the_tier_rate_until_it_lands() {
    CASE("travel requests the tier's refresh and stops asking when it lands");

    CognitionMotion motion;
    motion.observe(thinking("trace-a", LoopStage::Observe), 0, T0);
    motion.observe(thinking("trace-a", LoopStage::Contextualize), 0, T0 + 1'000);

    const std::int64_t mid = T0 + 1'000 + kPolicy.transition_ms / 2;
    const CognitionFrame travelling = motion.frame(mid, RenderTier::Normal);
    CHECK(travelling.animate);
    CHECK(travelling.marker_position.has_value());
    CHECK_EQ(travelling.next_frame_at_ms, std::optional<std::int64_t>{mid + 16});

    const CognitionFrame conserve = motion.frame(mid, RenderTier::Conserve);
    CHECK_BECAUSE(conserve.next_frame_at_ms == std::optional<std::int64_t>{mid + 33},
                  "a machine that needs its GPU back gets half the refresh, not a label");

    const std::int64_t after = T0 + 1'000 + kPolicy.transition_ms;
    CHECK(!motion.frame(after, RenderTier::Normal).marker_position.has_value());
}

void an_idle_panel_wakes_only_at_freshness_boundaries() {
    CASE("a still panel costs one frame per boundary a reading crosses");

    CognitionMotion motion;
    motion.observe(reading("trace-a", LoopStage::Remember, false, true), 0, T0);

    const std::int64_t settled = T0 + 1'000;
    const CognitionFrame still = motion.frame(settled, RenderTier::Normal);
    CHECK(!still.animate);
    CHECK_BECAUSE(still.next_frame_at_ms ==
                      std::optional<std::int64_t>{T0 + kPolicy.freshness.lagging_after_ms},
                  "the next time this panel looks different is when its reading starts to lag");

    const CognitionFrame lagging =
        motion.frame(T0 + kPolicy.freshness.lagging_after_ms, RenderTier::Normal);
    CHECK_EQ(lagging.freshness, Freshness::Lagging);
    CHECK_EQ(lagging.next_frame_at_ms,
             std::optional<std::int64_t>{T0 + kPolicy.freshness.stale_after_ms});

    const CognitionFrame expired =
        motion.frame(T0 + kPolicy.freshness.expired_after_ms, RenderTier::Normal);
    CHECK_EQ(expired.freshness, Freshness::Expired);
    CHECK_BECAUSE(!expired.next_frame_at_ms.has_value(),
                  "past the last boundary nothing will ever change without new input");
}

void a_running_pass_breathes_only_while_its_reading_is_fresh() {
    CASE("the thinking breath stops when the source stops vouching for it");

    CognitionMotion motion;
    motion.observe(thinking("trace-a", LoopStage::Reason), 0, T0);

    const std::int64_t fresh = T0 + kPolicy.glyph_fade_ms + 400;
    const CognitionFrame breathing = motion.frame(fresh, RenderTier::Normal);
    CHECK(breathing.animate);
    CHECK(breathing.breath > 0.0F);
    CHECK_BECAUSE(breathing.next_frame_at_ms ==
                      std::optional<std::int64_t>{fresh + kPolicy.ambient_frame_interval_ms},
                  "a slow swell reads the same at fifteen frames a second as at sixty");

    const CognitionFrame stale =
        motion.frame(T0 + kPolicy.freshness.stale_after_ms, RenderTier::Normal);
    CHECK_EQ(stale.activity, Activity::Thinking);
    CHECK_BECAUSE(!stale.animate && stale.breath == 0.0F,
                  "motion on stale data is a lie: a pass nobody has reported on for fifteen "
                  "seconds may not look busy");
}

void a_reading_stale_on_arrival_lands_without_travel() {
    CASE("a stale reading changes the panel but never moves on it");

    // Both readings are old, and the second was produced after the first — a backlog
    // delivered late, in order. (A second reading produced *before* the first would be
    // the out-of-order case, which is ignored outright and tested separately.)
    CognitionMotion motion;
    motion.observe(thinking("trace-a", LoopStage::Observe), kPolicy.freshness.stale_after_ms + 5'000,
                   T0);
    motion.observe(thinking("trace-a", LoopStage::Act), kPolicy.freshness.stale_after_ms,
                   T0 + 100);

    const CognitionFrame frame = motion.frame(T0 + 100, RenderTier::Normal);
    CHECK(!frame.marker_position.has_value());
    CHECK_EQ(frame.current_index, std::optional<std::size_t>{ring_index(LoopStage::Act)});
    CHECK_EQ(frame.stage_intensity[ring_index(LoopStage::Act)], 1.0F);
}

void headless_produces_no_frames_even_mid_transition() {
    CASE("no display, no frames");

    CognitionMotion motion;
    motion.observe(thinking("trace-a", LoopStage::Observe), 0, T0);
    motion.observe(thinking("trace-a", LoopStage::Reason), 0, T0 + 10);

    const CognitionFrame frame = motion.frame(T0 + 50, RenderTier::Headless);
    CHECK(!frame.next_frame_at_ms.has_value());
    CHECK(!frame.animate);
    CHECK_EQ(frame.activity, Activity::Thinking);
}

void conserve_keeps_the_state_and_drops_the_breath() {
    CASE("under pressure the drawing loses decoration, not information");

    CognitionMotion motion;
    motion.observe(thinking("trace-a", LoopStage::Reason), 0, T0);
    const std::int64_t later = T0 + kPolicy.glyph_fade_ms + 400;

    const CognitionFrame frame = motion.frame(later, RenderTier::Conserve);
    CHECK_EQ(frame.activity, Activity::Thinking);
    CHECK_EQ(frame.current_index, std::optional<std::size_t>{ring_index(LoopStage::Reason)});
    CHECK_BECAUSE(frame.breath == 0.0F && !frame.animate,
                  "the breath is ambient motion, which Conserve switches off by profile");
}

void reduced_motion_changes_the_drawing_not_a_label() {
    CASE("reduced motion lands every change at once");

    CognitionMotionPolicy policy;
    policy.reduced_motion = true;
    CognitionMotion motion{policy};
    motion.observe(thinking("trace-a", LoopStage::Observe), 0, T0);
    motion.observe(thinking("trace-a", LoopStage::Verify), 0, T0 + 10);

    const CognitionFrame frame = motion.frame(T0 + 10, RenderTier::Ultra);
    CHECK(!frame.marker_position.has_value());
    CHECK_EQ(frame.stage_intensity[ring_index(LoopStage::Verify)], 1.0F);
    CHECK_EQ(frame.glyph_opacity, 1.0F);
    CHECK_EQ(frame.breath, 0.0F);
    CHECK(!frame.animate);
}

void a_new_instruction_starts_from_where_the_marker_is() {
    CASE("a transition overtaken mid-flight continues from its actual position");

    CognitionMotion motion;
    motion.observe(thinking("trace-a", LoopStage::Observe), 0, T0);
    motion.observe(thinking("trace-a", LoopStage::Reason), 0, T0 + 10);

    const std::int64_t mid = T0 + 10 + kPolicy.transition_ms / 2;
    const float where = *motion.frame(mid, RenderTier::Normal).marker_position;

    motion.observe(thinking("trace-a", LoopStage::Act), 0, mid);
    const CognitionFrame frame = motion.frame(mid, RenderTier::Normal);

    CHECK_BECAUSE(frame.marker_position == std::optional<float>{where},
                  "snapping back to the old start would draw a jump that never happened");
    CHECK_EQ(frame.superseded, 1);
}

void an_older_state_arriving_late_does_not_walk_backwards() {
    CASE("a stale message from the same pass is ignored");

    CognitionMotion motion;
    motion.observe(thinking("trace-a", LoopStage::Act), 0, T0);
    motion.observe(thinking("trace-a", LoopStage::Observe), 500, T0 + 100);

    CHECK_EQ(motion.frame(T0 + 100, RenderTier::Normal).current_index,
             std::optional<std::size_t>{ring_index(LoopStage::Act)});
}

void a_new_pass_restarts_without_travel() {
    CASE("a different trace does not travel from where the last pass stopped");

    CognitionMotion motion;
    motion.observe(reading("trace-a", LoopStage::Remember, false, true), 0, T0);
    motion.observe(thinking("trace-b", LoopStage::Observe), 0, T0 + 5'000);

    const CognitionFrame frame = motion.frame(T0 + 5'010, RenderTier::Normal);
    CHECK(!frame.marker_position.has_value());
    CHECK_EQ(frame.current_index, std::optional<std::size_t>{0});
}

void a_clock_that_runs_backwards_lands_animations() {
    CASE("clock rollback never replays or holds an animation");

    // Half a second back: inside the freshness policy's skew tolerance, so the reading is
    // still Live and motion is otherwise allowed. A large rollback would be caught by the
    // implausible-future rule first and would not exercise this one at all.
    CognitionMotion motion;
    motion.observe(thinking("trace-a", LoopStage::Observe), 0, T0);
    motion.observe(thinking("trace-a", LoopStage::Reason), 0, T0 + 1'000);

    const CognitionFrame frame = motion.frame(T0 + 500, RenderTier::Normal);
    CHECK_EQ(frame.freshness, Freshness::Live);
    CHECK_BECAUSE(!frame.marker_position.has_value() && frame.glyph_opacity == 1.0F,
                  "a clock behind the transition's start would otherwise hold the first frame "
                  "and keep requesting frames until real time caught up");
}

void the_ring_is_drawn_in_execution_order() {
    CASE("the floor is drawn before context, as Cerebra runs it");

    CHECK_BECAUSE(ring_index(LoopStage::Interpret) < ring_index(LoopStage::Contextualize),
                  "drawing context before the floor would draw the ordering the architecture "
                  "exists to prevent");
    CHECK_EQ(ring_index(LoopStage::Observe), std::size_t{0});
    CHECK_EQ(ring_index(LoopStage::Remember), kLoopStageCount - 1);

    for (std::size_t i = 0; i < kLoopStageCount; ++i) {
        CHECK_EQ(ring_index(kExecutionOrder[i]), i);
    }
}

void activity_precedence_follows_the_trace_invariants() {
    CASE("refusal outranks silence, and an approval hold outranks refusal");

    CognitionReading refused = reading("t", LoopStage::Authorize, false, true);
    refused.authorization = OutcomeKind::Refused;
    CHECK_BECAUSE(activity_of(refused) == Activity::Refused,
                  "the trace reports a refused pass as silent too; drawing it quiet hides it");
    CHECK(is_adverse(activity_of(refused)));

    CognitionReading held = reading("t", LoopStage::Authorize, false, true);
    held.authorization = OutcomeKind::AwaitingApproval;
    CHECK_EQ(activity_of(held), Activity::AwaitingApproval);
    CHECK(!is_adverse(Activity::AwaitingApproval));

    CognitionReading acted = reading("t", LoopStage::Remember, false, false);
    acted.execution = OutcomeKind::Proceeded;
    acted.memory = OutcomeKind::Silent;
    CHECK_BECAUSE(activity_of(acted) == Activity::Acted,
                  "memory declining to keep a result is not Archie having done nothing");

    CognitionReading failed = reading("t", LoopStage::Verify, false, false);
    failed.execution = OutcomeKind::Proceeded;
    failed.verification = OutcomeKind::Failed;
    CHECK_EQ(activity_of(failed), Activity::Failed);

    CHECK_EQ(activity_of(thinking("t", LoopStage::Reason)), Activity::Thinking);
    CHECK_EQ(activity_of(reading("t", LoopStage::Interpret, false, true)), Activity::Silent);
}

void frames_are_pure() {
    CASE("the same inputs give the same frame");

    CognitionMotion motion;
    motion.observe(thinking("trace-a", LoopStage::Observe), 0, T0);
    motion.observe(thinking("trace-a", LoopStage::Plan), 0, T0 + 20);

    for (std::int64_t t = T0; t < T0 + 3'000; t += 37) {
        CHECK(frames_equal(motion.frame(t, RenderTier::Normal),
                           motion.frame(t, RenderTier::Normal)));
    }
}

Value projection(std::vector<Member> members) { return Value::object(std::move(members)); }

void a_projection_parses_as_the_rust_side_writes_it() {
    CASE("the HudProjection shape round-trips into a reading");

    const Value value = projection({
        member("trace_id", Value{"8f1c2a"}),
        member("goal_label", Value{"draft-reply"}),
        member("stage", Value{"select-capability"}),
        member("privacy", Value{"restricted"}),
        member("context_source_count", Value{3}),
        member("authorization", projection({member("outcome", Value{"awaiting-approval"}),
                                            member("reason", Value{"operator_approval_required"})})),
        member("execution", Value{}),
        member("silent", Value{true}),
        member("in_progress", Value{false}),
        member("total_latency_ms", Value{620}),
        member("total_cost_micros", Value{4200}),
    });

    const CognitionParse parsed = parse_cognition(value);
    CHECK(parsed.error.empty());
    CHECK(parsed.reading.has_value());
    if (parsed.reading) {
        CHECK_EQ(parsed.reading->stage, std::optional<LoopStage>{LoopStage::SelectCapability});
        CHECK_EQ(parsed.reading->privacy, std::optional<PrivacyGrade>{PrivacyGrade::Restricted});
        CHECK_EQ(parsed.reading->authorization,
                 std::optional<OutcomeKind>{OutcomeKind::AwaitingApproval});
        CHECK_BECAUSE(!parsed.reading->execution.has_value(),
                      "an explicit null is the same as serde omitting the field");
        CHECK_EQ(parsed.reading->context_source_count, 3);
        CHECK_EQ(activity_of(*parsed.reading), Activity::AwaitingApproval);
    }
}

void a_projection_this_build_does_not_understand_is_refused() {
    CASE("unknown enums, missing flags and repeated keys are errors, not guesses");

    const auto base = [](std::vector<Member> extra) {
        std::vector<Member> members{member("trace_id", Value{"t"}),
                                    member("silent", Value{false}),
                                    member("in_progress", Value{true})};
        for (Member& m : extra) {
            members.push_back(std::move(m));
        }
        return Value::object(std::move(members));
    };

    CHECK(parse_cognition(base({})).reading.has_value());
    CHECK(!parse_cognition(base({member("stage", Value{"daydream"})})).reading.has_value());
    CHECK(!parse_cognition(base({member("privacy", Value{"top-secret"})})).reading.has_value());
    CHECK(!parse_cognition(base({member("execution", projection({member("outcome", Value{"maybe"})}))}))
               .reading.has_value());
    CHECK(!parse_cognition(base({member("silent", Value{true})})).reading.has_value());

    const Value missing = Value::object({member("trace_id", Value{"t"}), member("silent", Value{false})});
    CHECK_BECAUSE(!parse_cognition(missing).reading.has_value(),
                  "defaulting in_progress to false would draw an unknown pass as concluded");

    const Value wrong_type = Value::object({member("trace_id", Value{"t"}),
                                            member("silent", Value{false}),
                                            member("in_progress", Value{"true"})});
    CHECK_BECAUSE(!parse_cognition(wrong_type).reading.has_value(),
                  "the string \"true\" is not a boolean, and reading it as one is a guess");

    const Value both = Value::object({member("trace_id", Value{"t"}), member("silent", Value{true}),
                                      member("in_progress", Value{true})});
    CHECK(!parse_cognition(both).reading.has_value());

    CHECK(!parse_cognition(Value{"not an object"}).reading.has_value());
    CHECK(!parse_cognition(base({member("context_source_count", Value{-1})})).reading.has_value());
}

void losing_the_source_admits_it() {
    CASE("a source that goes away turns the panel unmeasured");

    CognitionMotion motion;
    motion.observe(thinking("trace-a", LoopStage::Reason), 0, T0);
    motion.observe_unmeasured(T0 + 1'000);

    const CognitionFrame frame = motion.frame(T0 + 1'000 + kPolicy.glyph_fade_ms, RenderTier::Normal);
    CHECK_EQ(frame.activity, Activity::Unmeasured);
    CHECK(!frame.current_index.has_value());
    CHECK(!frame.next_frame_at_ms.has_value());
}

}  // namespace

int main() {
    a_panel_that_has_heard_nothing_asks_for_nothing();
    unmeasured_never_renders_like_a_concluded_pass();
    only_a_change_starts_motion();
    a_transition_asks_for_frames_at_the_tier_rate_until_it_lands();
    an_idle_panel_wakes_only_at_freshness_boundaries();
    a_running_pass_breathes_only_while_its_reading_is_fresh();
    a_reading_stale_on_arrival_lands_without_travel();
    headless_produces_no_frames_even_mid_transition();
    conserve_keeps_the_state_and_drops_the_breath();
    reduced_motion_changes_the_drawing_not_a_label();
    a_new_instruction_starts_from_where_the_marker_is();
    an_older_state_arriving_late_does_not_walk_backwards();
    a_new_pass_restarts_without_travel();
    a_clock_that_runs_backwards_lands_animations();
    the_ring_is_drawn_in_execution_order();
    activity_precedence_follows_the_trace_invariants();
    frames_are_pure();
    a_projection_parses_as_the_rust_side_writes_it();
    a_projection_this_build_does_not_understand_is_refused();
    losing_the_source_admits_it();
    return archie::visual::test::report();
}
