// Who holds the stage's input, which commands may run, and how the pointer moves.

#include <cmath>
#include <cstdlib>

#include "archie/visual/stage.hpp"

#include "harness.hpp"

using namespace archie::visual;

namespace {

constexpr std::int64_t T0 = 5'000'000;

SurfaceTable table_with_terminal() {
    SurfaceTable table;
    table.map(7, "org.codeberg.dnkl.foot", IRect{100, 80, 800, 600});
    return table;
}

StageCommand click_on(const SurfaceTable& table, SurfaceId id, int x, int y) {
    const Surface* surface = table.find(id);
    StageCommand command;
    command.sequence = 1;
    command.expires_at_ms = T0 + 2'000;
    command.action = StageAction::Click;
    command.surface = id;
    command.app_id = surface != nullptr ? surface->app_id : "";
    command.geometry_revision = surface != nullptr ? surface->geometry_revision : 0;
    command.local_x = x;
    command.local_y = y;
    return command;
}

float distance(PointF a, PointF b) { return std::hypot(a.x - b.x, a.y - b.y); }

void surfaces_are_identified_by_revision_not_by_hope() {
    CASE("a surface's revision moves only when its geometry does, and never repeats");

    SurfaceTable table;
    CHECK(table.map(1, "a", IRect{0, 0, 100, 100}));
    CHECK_BECAUSE(!table.map(1, "b", IRect{0, 0, 50, 50}),
                  "two surfaces under one id would let a command meant for one reach the other");
    CHECK(!table.map(2, "c", IRect{0, 0, 0, 10}));

    const auto first = table.find(1)->geometry_revision;
    CHECK(table.move(1, IRect{0, 0, 100, 100}));
    CHECK_BECAUSE(table.find(1)->geometry_revision == first,
                  "re-reporting the same geometry must not invalidate commands");
    CHECK(table.move(1, IRect{10, 0, 100, 100}));
    const auto moved = table.find(1)->geometry_revision;
    CHECK(moved != first);

    CHECK(table.focus(1));
    CHECK(table.unmap(1));
    CHECK_BECAUSE(!table.focused().has_value(), "focus cannot outlive the surface it was on");
    CHECK(table.map(1, "a", IRect{10, 0, 100, 100}));
    CHECK_BECAUSE(table.find(1)->geometry_revision != moved && table.find(1)->geometry_revision != first,
                  "a reopened window at the same place is still not the window a command was aimed at");
    CHECK(!table.focus(99));
    CHECK(!table.move(99, IRect{0, 0, 1, 1}));
}

void a_bumped_desk_does_not_stop_the_agent() {
    CASE("jitter and slow drift below the takeover threshold leave the agent working");

    InputAuthority authority;
    for (int i = 0; i < 10; ++i) {
        authority.physical_motion(2.0F, 1.0F, T0 + i * 10);
    }
    CHECK_EQ(authority.mode(), AuthorityMode::Autonomous);

    // The same distance as a deliberate reach, spread over far longer than the window.
    InputAuthority drifting;
    for (int i = 0; i < 20; ++i) {
        drifting.physical_motion(5.0F, 0.0F, T0 + i * 1'000);
    }
    CHECK_BECAUSE(drifting.mode() == AuthorityMode::Autonomous,
                  "a hundred pixels over twenty seconds is a cable settling, not a person");
}

void a_person_reaching_for_the_mouse_takes_the_stage_at_once() {
    CASE("deliberate motion, any button or any key hands the stage to the operator");

    InputAuthority reach;
    reach.physical_motion(20.0F, 0.0F, T0);
    reach.physical_motion(20.0F, 0.0F, T0 + 100);
    CHECK_EQ(reach.mode(), AuthorityMode::Autonomous);
    reach.physical_motion(20.0F, 0.0F, T0 + 200);
    CHECK_EQ(reach.mode(), AuthorityMode::Operator);
    CHECK_EQ(reach.reason(), AuthorityReason::OperatorMotion);
    CHECK(!reach.accepts_agent_input());
    CHECK(reach.forwards_physical_input());

    // Straddling where a tumbling window would have reset.
    InputAuthority straddle;
    straddle.physical_motion(30.0F, 0.0F, T0 + 350);
    straddle.physical_motion(30.0F, 0.0F, T0 + 450);
    CHECK_BECAUSE(straddle.mode() == AuthorityMode::Operator,
                  "a reach split across an arbitrary window boundary is still one reach");

    InputAuthority button;
    button.physical_button(T0);
    CHECK_EQ(button.mode(), AuthorityMode::Operator);
    CHECK_EQ(button.reason(), AuthorityReason::OperatorButton);

    InputAuthority key;
    key.physical_key(T0);
    CHECK_EQ(key.mode(), AuthorityMode::Operator);
    CHECK_EQ(key.reason(), AuthorityReason::OperatorKey);

    InputAuthority broken;
    broken.physical_motion(INFINITY, 0.0F, T0);
    CHECK_BECAUSE(broken.mode() == AuthorityMode::Autonomous,
                  "a device reporting nonsense is not a person");

    // Worse than a false takeover is a missed one: one garbage sample must not poison the
    // window so that a real reach right after it goes unnoticed.
    InputAuthority poisoned;
    poisoned.physical_motion(NAN, 0.0F, T0);
    poisoned.physical_motion(30.0F, 0.0F, T0 + 100);
    poisoned.physical_motion(30.0F, 0.0F, T0 + 150);
    CHECK_BECAUSE(poisoned.mode() == AuthorityMode::Operator,
                  "a person reaching for the mouse is never masked by a glitch before it");
}

void an_operator_takeover_expires_only_after_real_quiet() {
    CASE("the agent resumes after the operator has been quiet, and only then");

    AuthorityPolicy policy;
    policy.operator_idle_resume_ms = 8'000;
    InputAuthority authority(policy);
    authority.physical_button(T0);
    CHECK(authority.resumes_at_ms() == std::optional<std::int64_t>{T0 + 8'000});

    authority.tick(T0 + 5'000);
    CHECK_EQ(authority.mode(), AuthorityMode::Operator);
    // Still using it.
    authority.physical_motion(3.0F, 0.0F, T0 + 6'000);
    authority.tick(T0 + 9'000);
    CHECK_BECAUSE(authority.mode() == AuthorityMode::Operator,
                  "every movement by the operator restarts their quiet period");
    authority.tick(T0 + 14'000);
    CHECK_EQ(authority.mode(), AuthorityMode::Autonomous);
    CHECK_EQ(authority.reason(), AuthorityReason::OperatorIdle);

    InputAuthority rollback(policy);
    rollback.physical_key(T0);
    rollback.tick(T0 - 60'000);
    CHECK_BECAUSE(rollback.mode() == AuthorityMode::Operator,
                  "a clock that went backwards has not produced a quiet period");

    AuthorityPolicy never;
    never.operator_idle_resume_ms = 0;
    InputAuthority sticky(never);
    sticky.physical_key(T0);
    sticky.tick(T0 + 86'400'000);
    CHECK_EQ(sticky.mode(), AuthorityMode::Operator);
    CHECK(!sticky.resumes_at_ms().has_value());
}

void a_pause_holds_until_a_person_resumes() {
    CASE("the pause chord stops the agent until explicitly resumed, whatever else happens");

    InputAuthority authority;
    authority.pause_chord(T0);
    CHECK_EQ(authority.mode(), AuthorityMode::Paused);
    authority.physical_motion(500.0F, 0.0F, T0 + 10);
    authority.physical_button(T0 + 20);
    authority.physical_key(T0 + 30);
    authority.tick(T0 + 86'400'000);
    CHECK_BECAUSE(authority.mode() == AuthorityMode::Paused,
                  "quiet is not consent: a pause ends only when a person says so");
    CHECK(!authority.resumes_at_ms().has_value());
    CHECK(authority.forwards_physical_input());

    authority.resume(T0 + 90'000'000);
    CHECK_EQ(authority.mode(), AuthorityMode::Autonomous);
    CHECK_EQ(authority.reason(), AuthorityReason::Resumed);

    InputAuthority from_operator;
    from_operator.physical_key(T0);
    from_operator.pause_chord(T0 + 1);
    CHECK_EQ(from_operator.mode(), AuthorityMode::Paused);
}

void commands_are_rechecked_against_the_stage_as_it_is_now() {
    CASE("every refusal the compositor can make, in the order it makes them");

    SurfaceTable table = table_with_terminal();
    InputAuthority authority;
    const StageCommand good = click_on(table, 7, 40, 30);
    CHECK(!check_command(good, table, authority, 0, T0).has_value());

    InputAuthority paused;
    paused.pause_chord(T0);
    StageCommand ghost = good;
    ghost.surface = 12345;
    CHECK_BECAUSE(check_command(ghost, table, paused, 0, T0) == CommandRefusal::Paused,
                  "a paused stage refuses before revealing which surfaces exist");
    InputAuthority held;
    held.physical_button(T0);
    CHECK(check_command(good, table, held, 0, T0) == CommandRefusal::OperatorHasControl);

    CHECK(check_command(good, table, authority, 0, T0 + 2'000) == CommandRefusal::Expired);
    CHECK(check_command(good, table, authority, 1, T0) == CommandRefusal::Replayed);
    StageCommand zero = good;
    zero.sequence = 0;
    CHECK(check_command(zero, table, authority, 0, T0) == CommandRefusal::Replayed);

    CHECK(check_command(ghost, table, authority, 0, T0) == CommandRefusal::UnknownSurface);

    SurfaceTable guarded;
    guarded.map(3, "archie.approval", IRect{0, 0, 400, 200}, true);
    StageCommand approve = click_on(guarded, 3, 200, 100);
    CHECK_BECAUSE(check_command(approve, guarded, authority, 0, T0) == CommandRefusal::SurfaceProtected,
                  "a prompt the agent can click is not a prompt");

    StageCommand other_app = good;
    other_app.app_id = "org.mozilla.firefox";
    CHECK(check_command(other_app, table, authority, 0, T0) == CommandRefusal::AppMismatch);
    SurfaceTable anonymous;
    anonymous.map(9, "", IRect{0, 0, 100, 100});
    StageCommand nameless = click_on(anonymous, 9, 5, 5);
    CHECK_BECAUSE(check_command(nameless, anonymous, authority, 0, T0) == CommandRefusal::AppMismatch,
                  "a window that never said what it is cannot be authorised by name");

    SurfaceTable moved = table_with_terminal();
    StageCommand stale = click_on(moved, 7, 40, 30);
    moved.move(7, IRect{140, 80, 800, 600});
    CHECK_BECAUSE(check_command(stale, moved, authority, 0, T0) == CommandRefusal::SurfaceChanged,
                  "a window that moved is refused rather than re-aimed");

    StageCommand outside = click_on(table, 7, 800, 30);
    CHECK(check_command(outside, table, authority, 0, T0) == CommandRefusal::OutsideSurface);
    StageCommand negative = click_on(table, 7, -1, 30);
    CHECK(check_command(negative, table, authority, 0, T0) == CommandRefusal::OutsideSurface);

    StageCommand drag = click_on(table, 7, 10, 10);
    drag.action = StageAction::Drag;
    drag.to_local_x = 799;
    drag.to_local_y = 600;
    CHECK(check_command(drag, table, authority, 0, T0) == CommandRefusal::OutsideSurface);
    drag.to_local_y = 599;
    CHECK(!check_command(drag, table, authority, 0, T0).has_value());

    StageCommand scroll = click_on(table, 7, 10, 10);
    scroll.action = StageAction::Scroll;
    CHECK(check_command(scroll, table, authority, 0, T0) == CommandRefusal::ScrollOutOfBounds);
    scroll.scroll_steps_y = -(kMaxScrollSteps + 1);
    CHECK(check_command(scroll, table, authority, 0, T0) == CommandRefusal::ScrollOutOfBounds);
    scroll.scroll_steps_y = -3;
    CHECK(!check_command(scroll, table, authority, 0, T0).has_value());

    StageCommand text = click_on(table, 7, 0, 0);
    text.action = StageAction::Text;
    CHECK(check_command(text, table, authority, 0, T0) == CommandRefusal::TextOutOfBounds);
    text.text_chars = kMaxTextChars + 1;
    CHECK(check_command(text, table, authority, 0, T0) == CommandRefusal::TextOutOfBounds);
    text.text_chars = 12;
    CHECK(!check_command(text, table, authority, 0, T0).has_value());

    StageCommand key = click_on(table, 7, 0, 0);
    key.action = StageAction::Key;
    CHECK(check_command(key, table, authority, 0, T0) == CommandRefusal::ChordOutOfBounds);
    key.key_count = kMaxChordKeys + 1;
    CHECK(check_command(key, table, authority, 0, T0) == CommandRefusal::ChordOutOfBounds);
    key.key_count = 2;
    CHECK(!check_command(key, table, authority, 0, T0).has_value());

    CHECK(std::string(refusal_code(CommandRefusal::SurfaceProtected)) == "surface_protected");
    CHECK(std::string(refusal_code(CommandRefusal::NotTypeable)) == "not_typeable");
    CHECK(std::string(refusal_code(CommandRefusal::CaptureFailed)) == "capture_failed");
}

void the_pointer_moves_like_a_hand_and_always_arrives() {
    CASE("trajectories arrive exactly, take Fitts time, bow a little and start gently");

    const TrajectoryPolicy policy;
    const PointF from{100.0F, 100.0F};
    const PointF to{900.0F, 500.0F};
    const auto path = PointerTrajectory::plan(from, to, T0, policy);

    CHECK(path.at(T0 - 50) == from);
    CHECK_BECAUSE(path.at(path.end_ms()) == to && path.at(path.end_ms() + 5'000) == to,
                  "a press must land exactly where the command said, not near it");
    CHECK(path.duration_ms() >= policy.min_duration_ms && path.duration_ms() <= policy.max_duration_ms);

    const float span = distance(from, to);
    // Minimum jerk: it barely moves in the first twentieth of the time.
    const PointF early = path.at(T0 + path.duration_ms() / 20);
    CHECK_BECAUSE(distance(from, early) < 0.02F * span,
                  "motion eases out of rest instead of jerking into it");
    const PointF late = path.at(path.end_ms() - path.duration_ms() / 20);
    CHECK(distance(late, to) < 0.02F * span);

    // The bow stays small.
    float widest = 0.0F;
    for (std::int64_t t = T0; t <= path.end_ms(); t += 10) {
        const PointF p = path.at(t);
        const float cross = std::fabs((to.x - from.x) * (from.y - p.y) - (from.x - p.x) * (to.y - from.y)) / span;
        widest = std::max(widest, cross);
    }
    CHECK_BECAUSE(widest <= policy.bow * span && widest <= 120.0F,
                  "a hand-like arc, not a detour");
    CHECK(widest > 1.0F);

    const auto again = PointerTrajectory::plan(from, to, T0, policy);
    CHECK_BECAUSE(again.at(T0 + 123) == path.at(T0 + 123), "the same command always moves the same way");

    const auto short_hop = PointerTrajectory::plan(from, PointF{130.0F, 100.0F}, T0, policy);
    const auto long_haul = PointerTrajectory::plan(from, PointF{1800.0F, 1000.0F}, T0, policy);
    CHECK(short_hop.duration_ms() < path.duration_ms());
    CHECK(long_haul.duration_ms() >= path.duration_ms());
    CHECK_EQ(long_haul.duration_ms(), policy.max_duration_ms);

    const auto still = PointerTrajectory::plan(from, from, T0, policy);
    CHECK_EQ(still.duration_ms(), 0);
    CHECK(still.at(T0) == from);

    TrajectoryPolicy slow = policy;
    slow.pace = 2.0F;
    const auto presented = PointerTrajectory::plan(from, to, T0, slow);
    CHECK_EQ(presented.duration_ms(), path.duration_ms() * 2);
}

void a_click_arrives_before_it_presses_even_on_a_late_frame() {
    CASE("playback orders arrival, press and release, whenever the frame comes");

    const SurfaceTable table = table_with_terminal();
    const PlaybackPolicy policy;
    const StageCommand command = click_on(table, 7, 40, 30);
    Playback playback = Playback::start(command, *table.find(7), PointF{0.0F, 0.0F}, T0, policy);

    const PointF target = to_output(*table.find(7), 40, 30);
    CHECK_BECAUSE(target == (PointF{140.5F, 110.5F}),
                  "a point is the centre of its pixel: on an edge it belongs to whichever neighbour rounding picks");
    const auto& schedule = playback.schedule();
    CHECK_EQ(schedule.size(), std::size_t{3});
    CHECK(schedule[0].kind == StepKind::Motion && schedule[0].point == target);
    CHECK(schedule[1].kind == StepKind::Press && schedule[1].at_ms == schedule[0].at_ms);
    CHECK(schedule[2].kind == StepKind::Release &&
          schedule[2].at_ms == schedule[1].at_ms + policy.press_dwell_ms);

    // One frame in the middle of the flight: a sampled motion, nothing else.
    const auto middle = playback.due(T0 + (schedule[0].at_ms - T0) / 2);
    CHECK_EQ(middle.size(), std::size_t{1});
    CHECK(middle[0].kind == StepKind::Motion && !(middle[0].point == target));

    // A frame long after the end still delivers everything, in order.
    const auto late = playback.due(T0 + 10'000);
    CHECK_EQ(late.size(), std::size_t{3});
    CHECK(late[0].kind == StepKind::Motion && late[0].point == target);
    CHECK(late[1].kind == StepKind::Press);
    CHECK(late[2].kind == StepKind::Release);
    CHECK_EQ(playback.state(), PlaybackState::Finished);
    CHECK(playback.due(T0 + 20'000).empty());
    CHECK(playback.cancel(T0 + 20'000).empty());
}

void an_interrupted_drag_lets_go() {
    CASE("cancelling mid-drag releases the button; cancelling before the press releases nothing");

    const SurfaceTable table = table_with_terminal();
    const PlaybackPolicy policy;
    StageCommand drag = click_on(table, 7, 10, 10);
    drag.action = StageAction::Drag;
    drag.to_local_x = 700;
    drag.to_local_y = 500;

    Playback carried = Playback::start(drag, *table.find(7), PointF{0.0F, 0.0F}, T0, policy);
    std::int64_t press_at = 0;
    for (const auto& step : carried.schedule()) {
        if (step.kind == StepKind::Press) {
            press_at = step.at_ms;
        }
    }
    const auto mid_haul = carried.due(press_at + 100);
    CHECK_BECAUSE(mid_haul.size() == 3 && mid_haul[0].kind == StepKind::Motion &&
                      mid_haul[1].kind == StepKind::Press && mid_haul[2].kind == StepKind::Motion &&
                      mid_haul[0].point == to_output(*table.find(7), 10, 10) &&
                      !(mid_haul[2].point == mid_haul[0].point),
                  "a late frame presses where the pointer arrived before carrying it anywhere");
    CHECK(carried.button_held());
    const auto cleanup = carried.cancel(press_at + 100);
    CHECK_BECAUSE(cleanup.size() == 1 && cleanup[0].kind == StepKind::Release,
                  "an application left with a button held is in a drag nobody is performing");
    CHECK_EQ(carried.state(), PlaybackState::Cancelled);
    CHECK(carried.due(press_at + 10'000).empty());

    Playback early = Playback::start(drag, *table.find(7), PointF{0.0F, 0.0F}, T0, policy);
    (void)early.due(T0 + 10);
    CHECK(early.cancel(T0 + 10).empty());

    // The release is at the drop point, after a carry of at least a quarter second.
    const auto& schedule = carried.schedule();
    const InputStep& release = schedule.back();
    CHECK(release.kind == StepKind::Release);
    CHECK(release.point == to_output(*table.find(7), 700, 500));
    CHECK(release.at_ms - press_at >= 250 + policy.drag_hold_ms);

    // Even a drag of a few pixels is carried, not flicked.
    StageCommand nudge = click_on(table, 7, 10, 10);
    nudge.action = StageAction::Drag;
    nudge.to_local_x = 14;
    nudge.to_local_y = 10;
    const Playback small = Playback::start(nudge, *table.find(7), to_output(*table.find(7), 10, 10), T0, policy);
    std::int64_t small_press = 0;
    for (const auto& step : small.schedule()) {
        if (step.kind == StepKind::Press) {
            small_press = step.at_ms;
        }
    }
    const std::int64_t carry = small.schedule().back().at_ms - small_press - policy.drag_hold_ms - policy.press_dwell_ms;
    CHECK_BECAUSE(carry >= 250,
                  "an application needs time between a press and a release to see a drag at all");
}

void typing_and_chords_are_counted_not_carried() {
    CASE("text is scheduled by index, chords release in reverse, scroll steps carry direction");

    const SurfaceTable table = table_with_terminal();
    const PlaybackPolicy policy;

    StageCommand text = click_on(table, 7, 0, 0);
    text.action = StageAction::Text;
    text.text_chars = 4;
    Playback typing = Playback::start(text, *table.find(7), PointF{5.0F, 5.0F}, T0, policy);
    const auto& typed = typing.schedule();
    CHECK_EQ(typed.size(), std::size_t{5});
    CHECK(typed[0].kind == StepKind::FocusSurface && typed[0].at_ms == T0);
    CHECK_BECAUSE(typed[1].at_ms - typed[0].at_ms == policy.focus_settle_ms,
                  "the first character waits for the application to take focus");
    for (std::size_t i = 0; i < 4; ++i) {
        CHECK(typed[i + 1].kind == StepKind::TextUnit && typed[i + 1].index == i);
        CHECK_EQ(typed[i + 1].at_ms,
                 T0 + policy.focus_settle_ms + static_cast<std::int64_t>(i) * policy.key_interval_ms);
    }

    StageCommand chord = click_on(table, 7, 0, 0);
    chord.action = StageAction::Key;
    chord.key_count = 3;
    Playback keys = Playback::start(chord, *table.find(7), PointF{}, T0, policy);
    std::vector<std::size_t> presses;
    std::vector<std::size_t> releases;
    for (const auto& step : keys.schedule()) {
        if (step.kind == StepKind::KeyPress) presses.push_back(step.index);
        if (step.kind == StepKind::KeyRelease) releases.push_back(step.index);
    }
    CHECK(presses == (std::vector<std::size_t>{0, 1, 2}));
    CHECK(releases == (std::vector<std::size_t>{2, 1, 0}));
    CHECK_BECAUSE(keys.schedule()[1].at_ms == T0 + policy.focus_settle_ms,
                  "a chord's first key waits for focus the way text does");
    // Interrupted with two keys down: both come back up, last pressed first.
    const std::int64_t two_down = T0 + policy.focus_settle_ms + policy.key_interval_ms;
    (void)keys.due(two_down);
    const auto up = keys.cancel(two_down);
    CHECK(up.size() == 2 && up[0].index == 1 && up[1].index == 0);

    StageCommand scroll = click_on(table, 7, 100, 100);
    scroll.action = StageAction::Scroll;
    scroll.scroll_steps_x = 1;
    scroll.scroll_steps_y = -3;
    Playback wheel = Playback::start(scroll, *table.find(7), PointF{}, T0, policy);
    int wheels = 0;
    for (const auto& step : wheel.schedule()) {
        if (step.kind != StepKind::Wheel) continue;
        CHECK_EQ(step.wheel_y, -1);
        CHECK_EQ(step.wheel_x, wheels == 0 ? 1 : 0);
        ++wheels;
    }
    CHECK_EQ(wheels, 3);

    StageCommand focus = click_on(table, 7, 0, 0);
    focus.action = StageAction::Focus;
    Playback focused = Playback::start(focus, *table.find(7), PointF{}, T0, policy);
    CHECK_EQ(focused.due(T0).size(), std::size_t{1});
    CHECK_EQ(focused.state(), PlaybackState::Finished);
}

void a_capture_is_checked_like_a_touch_and_held_on_screen() {
    CASE("a snapshot is one capture step, re-checked like any command, and visible for a moment");

    SurfaceTable table = table_with_terminal();
    InputAuthority authority;
    const PlaybackPolicy policy;

    StageCommand look = click_on(table, 7, 0, 0);
    look.action = StageAction::Snapshot;
    CHECK_BECAUSE(!check_command(look, table, authority, 0, T0).has_value(),
                  "a capture names no point, so no point can be outside the surface");

    InputAuthority paused;
    paused.pause_chord(T0);
    CHECK_BECAUSE(check_command(look, table, paused, 0, T0) == CommandRefusal::Paused,
                  "a paused stage is not looked at either");
    SurfaceTable guarded;
    guarded.map(7, "org.codeberg.dnkl.foot", IRect{100, 80, 800, 600}, true);
    CHECK_BECAUSE(check_command(look, guarded, authority, 0, T0) == CommandRefusal::SurfaceProtected,
                  "a protected surface is never captured, whatever the policy allowed");
    SurfaceTable moved = table_with_terminal();
    moved.move(7, IRect{140, 80, 800, 600});
    CHECK(check_command(look, moved, authority, 0, T0) == CommandRefusal::SurfaceChanged);

    Playback capture = Playback::start(look, *table.find(7), PointF{5.0F, 5.0F}, T0, policy);
    CHECK_EQ(capture.schedule().size(), std::size_t{1});
    CHECK(capture.schedule()[0].kind == StepKind::Capture && capture.schedule()[0].at_ms == T0);
    CHECK_EQ(capture.ends_ms(), T0 + policy.capture_hold_ms);
    CHECK_EQ(capture.due(T0).size(), std::size_t{1});
    CHECK_BECAUSE(capture.state() == PlaybackState::Running,
                  "the receipt waits for the hold, so the capture has been rendered and seen");
    (void)capture.due(T0 + policy.capture_hold_ms);
    CHECK_EQ(capture.state(), PlaybackState::Finished);
    Playback interrupted = Playback::start(look, *table.find(7), PointF{}, T0, policy);
    (void)interrupted.due(T0);
    CHECK_BECAUSE(interrupted.cancel(T0 + 10).empty(), "a capture holds nothing that needs letting go");
}


void a_filming_tempo_still_draws_slower_than_an_application_can_look() {
    CASE("video tempo leaves an application enough pointer reports to see a line by");

    // An application learns where the pointer went by being told where it is, once a frame.
    // At sixty frames a second, a line drawn faster than about a thousand pixels a second
    // reaches it as a polygon of the few positions that fitted in, whatever the command
    // carried. That is not something the stage can fix by sending more points, so the tempo
    // meant for filming has to stay slow enough to be worth watching.
    const PlaybackPolicy filming = PlaybackPolicy::video_tempo();
    const float between_reports = filming.stroke_speed_px_per_s / 60.0F;
    CHECK_BECAUSE(between_reports <= 20.0F,
                  "no more than twenty pixels between one report and the next");
    // It is barely faster than a person's hand, and cannot be much faster: the limit is
    // the application looking, not the stage drawing.
    CHECK_BECAUSE(filming.stroke_speed_px_per_s >= PlaybackPolicy{}.stroke_speed_px_per_s,
                  "and no slower than a person's hand");
}

void an_agent_tempo_is_the_same_playback_on_a_faster_clock() {

    CASE("at agent tempo the same steps happen sooner, and looking holds nothing");

    SurfaceTable table = table_with_terminal();
    const PlaybackPolicy person;
    const PlaybackPolicy agent = PlaybackPolicy::agent_tempo();

    const StageCommand click = click_on(table, 7, 400, 300);
    Playback watched = Playback::start(click, *table.find(7), PointF{100.0F, 100.0F}, T0, person);
    Playback working = Playback::start(click, *table.find(7), PointF{100.0F, 100.0F}, T0, agent);
    CHECK_EQ(working.schedule().size(), watched.schedule().size());
    for (std::size_t i = 0; i < working.schedule().size(); ++i) {
        CHECK_BECAUSE(working.schedule()[i].kind == watched.schedule()[i].kind,
                      "the same steps in the same order: only the clock changes");
    }
    CHECK_BECAUSE((working.ends_ms() - T0) * 3 < watched.ends_ms() - T0,
                  "a click that took over half a second takes under two hundred milliseconds");
    CHECK_BECAUSE(working.due(working.ends_ms()).back().kind == StepKind::Release,
                  "and it still ends by letting go");

    StageCommand look = click_on(table, 7, 0, 0);
    look.action = StageAction::Snapshot;
    Playback capture = Playback::start(look, *table.find(7), PointF{}, T0, agent);
    CHECK_EQ(capture.ends_ms(), T0);
    (void)capture.due(T0);
    CHECK_BECAUSE(capture.state() == PlaybackState::Finished,
                  "an agent at work does not wait 450 ms for anyone to see that it looked");
}
void a_capture_can_ask_for_one_box_instead_of_the_whole_window() {
    CASE("a snapshot may name a part of the surface, and only a real part of it");

    SurfaceTable table = table_with_terminal();  // 800x600 at 100,80
    InputAuthority authority;

    StageCommand whole = click_on(table, 7, 0, 0);
    whole.action = StageAction::Snapshot;
    CHECK_BECAUSE(!check_command(whole, table, authority, 0, T0).has_value(),
                  "naming no part is asking for the whole surface, as before");
    CHECK_EQ(snapshot_region(whole, *table.find(7)), (IRect{0, 0, 800, 600}));

    StageCommand box = whole;
    box.local_x = 120;
    box.local_y = 40;
    box.to_local_x = 200;
    box.to_local_y = 67;
    CHECK_BECAUSE(!check_command(box, table, authority, 0, T0).has_value(),
                  "a box inside the window is a box the window has");
    CHECK_EQ(snapshot_region(box, *table.find(7)), (IRect{120, 40, 80, 27}));

    StageCommand past = box;
    past.to_local_x = 801;
    CHECK_BECAUSE(check_command(past, table, authority, 0, T0) == CommandRefusal::OutsideSurface,
                  "a capture cannot ask for pixels the window does not have");
    StageCommand behind = box;
    behind.local_y = -1;
    CHECK(check_command(behind, table, authority, 0, T0) == CommandRefusal::OutsideSurface);
    StageCommand inside_out = box;
    inside_out.to_local_x = box.local_x;
    CHECK_BECAUSE(check_command(inside_out, table, authority, 0, T0) == CommandRefusal::OutsideSurface,
                  "an empty box is not the whole window by accident");

    // The command was checked against the surface as it was; by the time it is rendered
    // the window may be smaller. What is left of the box is captured, not what is past it.
    SurfaceTable shrunk = table_with_terminal();
    shrunk.move(7, IRect{100, 80, 150, 600});
    CHECK_EQ(snapshot_region(box, *shrunk.find(7)), (IRect{120, 40, 30, 27}));
    StageCommand gone = box;
    gone.local_x = 400;
    gone.to_local_x = 480;
    CHECK_BECAUSE(snapshot_region(gone, *shrunk.find(7)) == (IRect{0, 0, 150, 600}),
                  "nothing left of the box is the whole surface, never a negative rectangle");
}
void a_drawing_of_several_lines_lifts_between_them() {
    CASE("several lines in one command press, draw, let go, travel and press again");

    SurfaceTable table = table_with_terminal();
    StageCommand drawing = click_on(table, 7, 0, 0);
    drawing.action = StageAction::Stroke;
    // Two lines far apart, joined in one command by a lift at the third point.
    drawing.path = {IPoint{100, 100}, IPoint{160, 120}, IPoint{400, 300}, IPoint{460, 330}};
    drawing.lifts = {2};
    CHECK_BECAUSE(!check_command(drawing, table, InputAuthority{}, 0, T0).has_value(),
                  "a drawing of two real lines is allowed");

    Playback run = Playback::start(drawing, *table.find(7), PointF{}, T0, PlaybackPolicy{});
    const auto steps = run.schedule();
    int presses = 0;
    int releases = 0;
    for (const InputStep& one : steps) {
        if (one.kind == StepKind::Press) ++presses;
        if (one.kind == StepKind::Release) ++releases;
    }
    CHECK_BECAUSE(presses == 2, "one press for each line");
    CHECK_BECAUSE(releases == 2, "and one release for each");
    CHECK_BECAUSE(steps.back().kind == StepKind::Release, "it ends by letting go");

    // A lift that would leave a line of one point is not a drawing.
    StageCommand bad = drawing;
    bad.lifts = {1};
    CHECK_BECAUSE(check_command(bad, table, InputAuthority{}, 0, T0) == CommandRefusal::PathOutOfBounds,
                  "a line has to have two points on both sides of a lift");
    bad.lifts = {2, 2};
    CHECK_BECAUSE(check_command(bad, table, InputAuthority{}, 0, T0) == CommandRefusal::PathOutOfBounds,
                  "and the lifts have to be in order");
    // A second line that never leaves its first point is a click wearing a line's name.
    StageCommand still = drawing;
    still.path = {IPoint{100, 100}, IPoint{160, 120}, IPoint{400, 300}, IPoint{400, 300}};
    CHECK_BECAUSE(check_command(still, table, InputAuthority{}, 0, T0) == CommandRefusal::PathOutOfBounds,
                  "every line of a drawing has to go somewhere, not only the first");
}

void a_stroke_is_pressed_followed_and_let_go() {

    CASE("a stroke goes down at its first point, through every one of them, and up at the last");

    SurfaceTable table = table_with_terminal();  // 800x600 at 100,80
    InputAuthority authority;
    const PlaybackPolicy policy;

    StageCommand line = click_on(table, 7, 0, 0);
    line.action = StageAction::Stroke;
    line.path = {IPoint{100, 100}, IPoint{300, 100}, IPoint{300, 260}};
    CHECK(!check_command(line, table, authority, 0, T0).has_value());

    Playback drawing = Playback::start(line, *table.find(7), PointF{5.0F, 5.0F}, T0, policy);
    const auto& steps = drawing.schedule();
    std::size_t presses = 0;
    std::size_t releases = 0;
    for (const InputStep& step : steps) {
        presses += step.kind == StepKind::Press ? 1 : 0;
        releases += step.kind == StepKind::Release ? 1 : 0;
    }
    CHECK_BECAUSE(presses == 1 && releases == 1, "one line is one press and one release");
    const InputStep* press = nullptr;
    const InputStep* release = nullptr;
    for (const InputStep& step : steps) {
        if (step.kind == StepKind::Press) press = &step;
        if (step.kind == StepKind::Release) release = &step;
    }
    // The surface sits at 100,80 and a point is its centre, so the window's 100,100 is the
    // output's 200.5,180.5.
    const auto at_point = [](PointF point, float x, float y) {
        return std::abs(point.x - x) < 0.01F && std::abs(point.y - y) < 0.01F;
    };
    CHECK(press != nullptr && release != nullptr);
    if (press && release) {
        CHECK(at_point(press->point, 200.5F, 180.5F));
        CHECK(at_point(release->point, 400.5F, 340.5F));
        CHECK_BECAUSE(press->at_ms < release->at_ms, "the button is down for the whole line");
    }

    // Every point named is arrived at, in order, and nothing is drawn before the press.
    std::vector<PointF> arrivals;
    for (const InputStep& step : steps) {
        if (step.kind == StepKind::Motion && press && step.at_ms > press->at_ms) {
            arrivals.push_back(step.point);
        }
    }
    CHECK_EQ(arrivals.size(), std::size_t{2});
    if (arrivals.size() == 2) {
        CHECK(at_point(arrivals[0], 400.5F, 180.5F));
        CHECK(at_point(arrivals[1], 400.5F, 340.5F));
    }
    // And it is the line it was asked for: halfway along the first segment the pointer is
    // on that segment, not bowed off it the way a reach for a target is.
    const auto first = drawing.trajectory_at(press->at_ms + policy.drag_hold_ms + 1);
    CHECK(first.has_value());
    if (first) {
        const PointF half = first->at((first->start_ms() + first->end_ms()) / 2);
        CHECK_BECAUSE(std::abs(half.y - 180.5F) < 0.01F,
                      "a line drawn across the window does not sag in the middle");
    }

    StageCommand alone = line;
    alone.path = {IPoint{100, 100}};
    CHECK(check_command(alone, table, authority, 0, T0) == CommandRefusal::PathOutOfBounds);
    StageCommand still = line;
    still.path = {IPoint{100, 100}, IPoint{100, 100}};
    CHECK_BECAUSE(check_command(still, table, authority, 0, T0) == CommandRefusal::PathOutOfBounds,
                  "a stroke that never leaves its first point is a click wearing a line's name");
    StageCommand long_line = line;
    long_line.path.assign(kMaxStrokePoints + 1, IPoint{100, 100});
    long_line.path[1] = IPoint{101, 100};
    CHECK(check_command(long_line, table, authority, 0, T0) == CommandRefusal::PathOutOfBounds);
    StageCommand past = line;
    past.path = {IPoint{100, 100}, IPoint{800, 100}};
    CHECK_BECAUSE(check_command(past, table, authority, 0, T0) == CommandRefusal::OutsideSurface,
                  "a line cannot be drawn past the edge of the window it is drawn in");

    // At an agent's tempo the same line is drawn faster, and it is still the same line.
    Playback quick = Playback::start(line, *table.find(7), PointF{5.0F, 5.0F}, T0,
                                     PlaybackPolicy::agent_tempo());
    CHECK_BECAUSE(quick.ends_ms() - T0 < drawing.ends_ms() - T0,
                  "the pointer covers the same way in less time");
    std::size_t quick_motions = 0;
    for (const InputStep& step : quick.schedule()) {
        quick_motions += step.kind == StepKind::Motion ? 1 : 0;
    }
    std::size_t motions = 0;
    for (const InputStep& step : steps) {
        motions += step.kind == StepKind::Motion ? 1 : 0;
    }
    CHECK_EQ(quick_motions, motions);
}
}  // namespace

int main() {
    surfaces_are_identified_by_revision_not_by_hope();
    a_bumped_desk_does_not_stop_the_agent();
    a_person_reaching_for_the_mouse_takes_the_stage_at_once();
    an_operator_takeover_expires_only_after_real_quiet();
    a_pause_holds_until_a_person_resumes();
    commands_are_rechecked_against_the_stage_as_it_is_now();
    the_pointer_moves_like_a_hand_and_always_arrives();
    a_click_arrives_before_it_presses_even_on_a_late_frame();
    an_interrupted_drag_lets_go();
    typing_and_chords_are_counted_not_carried();
    a_capture_is_checked_like_a_touch_and_held_on_screen();
    an_agent_tempo_is_the_same_playback_on_a_faster_clock();
    a_filming_tempo_still_draws_slower_than_an_application_can_look();
    a_capture_can_ask_for_one_box_instead_of_the_whole_window();
    a_stroke_is_pressed_followed_and_let_go();
    a_drawing_of_several_lines_lifts_between_them();
    return archie::visual::test::report();
}
