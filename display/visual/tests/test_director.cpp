// Where the camera looks, and what is drawn, while the agent works any application.

#include <cmath>

#include "archie/visual/director.hpp"

#include "harness.hpp"

using namespace archie::visual;

namespace {

constexpr std::int64_t T0 = 9'000'000;

struct Fixture {
    SurfaceTable table;
    InputAuthority authority;
    PlaybackPolicy playback_policy;
    StageDirector director;
    std::uint64_t sequence = 0;

    Fixture() {
        table.map(1, "org.freecad.FreeCAD", IRect{0, 0, 1920, 1080});
        table.map(2, "org.codeberg.dnkl.foot", IRect{1200, 600, 640, 400});
    }

    StageCommand command(SurfaceId id, StageAction action, int x, int y) {
        const Surface* surface = table.find(id);
        StageCommand c;
        c.sequence = ++sequence;
        c.expires_at_ms = T0 + 3'600'000;
        c.action = action;
        c.surface = id;
        c.app_id = surface->app_id;
        c.geometry_revision = surface->geometry_revision;
        c.local_x = x;
        c.local_y = y;
        return c;
    }

    Playback run(const StageCommand& c, PointF pointer, std::int64_t now) {
        Playback playback = Playback::start(c, *table.find(c.surface), pointer, now, playback_policy);
        director.command_started(c, *table.find(c.surface), playback, now);
        return playback;
    }
};

bool inside(const VisibleRegion& region, PointF point) {
    return point.x >= region.x - 0.01F && point.y >= region.y - 0.01F &&
           point.x <= region.x + region.w + 0.01F && point.y <= region.y + region.h + 0.01F;
}

bool within_stage(const VisibleRegion& region, int w, int h) {
    return region.x >= -0.01F && region.y >= -0.01F && region.x + region.w <= static_cast<float>(w) + 0.01F &&
           region.y + region.h <= static_cast<float>(h) + 0.01F;
}

void a_still_stage_costs_nothing() {
    CASE("with nothing happening the whole stage is shown and no frame is requested");

    const StageDirector director;
    const StageFrame frame = director.frame(T0, RenderTier::Normal);
    CHECK(frame.view == (ViewRect{960.0F, 540.0F, 1.0F}));
    CHECK(!frame.animate);
    CHECK(!frame.cursor_visible);
    CHECK(!frame.caption.has_value());
    CHECK_BECAUSE(!frame.next_frame_at_ms.has_value(), "a stage where nothing will change asks for nothing");
}

void a_precise_action_gets_a_closer_shot_than_a_long_one() {
    CASE("framing follows what the action spans: short and precise is close, long is wide");

    Fixture precise;
    const auto click = precise.command(1, StageAction::Click, 700, 400);
    const Playback nearby = precise.run(click, PointF{640.0F, 380.0F}, T0);
    const ViewRect close = precise.director.camera_target();
    CHECK(close.zoom > 1.2F);
    CHECK(close.zoom <= precise.director.policy().max_zoom);

    Fixture sweeping;
    const auto far_click = sweeping.command(1, StageAction::Click, 1800, 1000);
    (void)sweeping.run(far_click, PointF{60.0F, 60.0F}, T0);
    const ViewRect wide = sweeping.director.camera_target();
    CHECK_BECAUSE(wide.zoom < close.zoom, "a move across the stage needs the stage in frame");

    // Wherever the camera settles, the press is in the shot.
    const PointF target = to_output(*precise.table.find(1), 700, 400);
    const VisibleRegion settled = visible_region(close, 1920, 1080);
    CHECK(inside(settled, target));
    CHECK(inside(settled, PointF{640.0F, 380.0F}));
    CHECK(within_stage(settled, 1920, 1080));

    const StageFrame moving = precise.director.frame(T0 + 50, RenderTier::Normal);
    CHECK(moving.camera_moving);
    CHECK(moving.animate);
    CHECK(moving.next_frame_at_ms == std::optional<std::int64_t>{T0 + 50 + 16});
    CHECK(moving.cursor_visible);
    CHECK_EQ(moving.caption->verb, CaptionVerb::Clicking);
    CHECK(moving.caption->app_id == "org.freecad.FreeCAD");
    (void)nearby;
}

void the_camera_does_not_chase_the_cursor() {
    CASE("an action that already fits in the frame keeps the framing; one that leaves it re-frames");

    Fixture f;
    const auto first = f.command(1, StageAction::Click, 700, 400);
    Playback one = f.run(first, PointF{640.0F, 380.0F}, T0);
    (void)one.due(T0 + 5'000);
    f.director.command_ended(PlaybackState::Finished, T0 + 1'000);
    const ViewRect held = f.director.camera_target();

    const auto nudge = f.command(1, StageAction::Click, 720, 410);
    (void)f.run(nudge, to_output(*f.table.find(1), 700, 400), T0 + 1'200);
    CHECK_BECAUSE(f.director.camera_target() == held,
                  "a small follow-up inside the frame must not shake the camera");

    f.director.command_ended(PlaybackState::Finished, T0 + 1'900);
    const auto away = f.command(1, StageAction::Click, 1700, 150);
    const ViewRect before = f.director.camera_at(T0 + 2'000);
    (void)f.run(away, to_output(*f.table.find(1), 720, 410), T0 + 2'000);
    CHECK(!(f.director.camera_target() == held));
    CHECK_BECAUSE(f.director.camera_at(T0 + 2'000) == before,
                  "a new framing starts from wherever the camera actually is");
}

void an_interrupted_move_continues_from_where_it_is() {
    CASE("re-aiming mid-move begins at the camera's current pose, not the old origin");

    Fixture f;
    const auto first = f.command(1, StageAction::Click, 300, 300);
    (void)f.run(first, PointF{280.0F, 280.0F}, T0);
    const ViewRect midway = f.director.camera_at(T0 + 300);
    f.director.command_ended(PlaybackState::Finished, T0 + 300);
    const auto second = f.command(1, StageAction::Click, 1650, 900);
    (void)f.run(second, to_output(*f.table.find(1), 300, 300), T0 + 300);
    CHECK(f.director.camera_at(T0 + 300) == midway);
    const ViewRect a_bit_later = f.director.camera_at(T0 + 310);
    CHECK(std::fabs(a_bit_later.cx - midway.cx) < 5.0F);
}

void framing_never_shows_past_the_stage() {
    CASE("an action in a corner is framed without showing space beyond the stage");

    Fixture f;
    const auto corner = f.command(1, StageAction::Click, 4, 4);
    (void)f.run(corner, PointF{30.0F, 20.0F}, T0);
    const ViewRect view = f.director.camera_target();
    CHECK(within_stage(visible_region(view, 1920, 1080), 1920, 1080));
    CHECK(inside(visible_region(view, 1920, 1080), to_output(*f.table.find(1), 4, 4)));
}

void the_camera_goes_home_after_a_quiet_spell() {
    CASE("after the last action and the settle time the camera returns to the whole stage");

    Fixture f;
    const auto click = f.command(2, StageAction::Click, 300, 200);
    (void)f.run(click, PointF{1500.0F, 700.0F}, T0);
    f.director.command_ended(PlaybackState::Finished, T0 + 600);
    CHECK(f.director.camera_target().zoom > 1.0F);

    const StageFrame waiting = f.director.frame(T0 + 1'700, RenderTier::Normal);
    const std::int64_t settle_at = T0 + 600 + f.director.policy().settle_ms;
    CHECK(waiting.next_frame_at_ms.has_value() && *waiting.next_frame_at_ms <= settle_at);
    // After the caption is gone and before the cursor starts fading, going home is the
    // only thing left to wake for.
    const StageFrame only_home = f.director.frame(T0 + 2'100, RenderTier::Normal);
    CHECK_BECAUSE(only_home.next_frame_at_ms == std::optional<std::int64_t>{settle_at},
                  "the return home is a deadline, and it needs a frame even though nothing moves");

    f.director.tick(settle_at - 1);
    CHECK(f.director.camera_target().zoom > 1.0F);
    f.director.tick(settle_at);
    CHECK(f.director.camera_target() == (ViewRect{960.0F, 540.0F, 1.0F}));

    // Once home and faded out, the stage asks for nothing again.
    const std::int64_t later = settle_at + 60'000;
    f.director.tick(later);
    const StageFrame quiet = f.director.frame(later, RenderTier::Normal);
    CHECK(!quiet.animate);
    CHECK(!quiet.next_frame_at_ms.has_value());
    CHECK(!quiet.cursor_visible);
    CHECK(!quiet.caption.has_value());
}

void a_person_taking_over_gets_the_whole_stage_and_the_only_cursor() {
    CASE("operator takeover: whole stage at once, banner shown, agent cursor hidden, no following");

    Fixture f;
    const auto click = f.command(1, StageAction::Click, 500, 500);
    (void)f.run(click, PointF{450.0F, 470.0F}, T0);
    CHECK(f.director.camera_target().zoom > 1.0F);

    f.director.authority_changed(AuthorityMode::Operator, T0 + 200);
    f.director.command_ended(PlaybackState::Cancelled, T0 + 200);
    CHECK(f.director.camera_target() == (ViewRect{960.0F, 540.0F, 1.0F}));
    CHECK(f.director.camera_at(T0 + 200 + f.director.policy().release_ms) ==
          (ViewRect{960.0F, 540.0F, 1.0F}));

    const StageFrame frame = f.director.frame(T0 + 300, RenderTier::Normal);
    CHECK(frame.banner == std::optional<AuthorityMode>{AuthorityMode::Operator});
    CHECK_BECAUSE(!frame.cursor_visible, "under a person's hand there is exactly one pointer, theirs");
    CHECK(frame.caption.has_value() && frame.caption->verb == CaptionVerb::Interrupted);

    f.director.tick(T0 + 600'000);
    CHECK(f.director.camera_target() == (ViewRect{960.0F, 540.0F, 1.0F}));

    f.director.authority_changed(AuthorityMode::Paused, T0 + 700'000);
    const StageFrame paused = f.director.frame(T0 + 700'100, RenderTier::Normal);
    CHECK(paused.banner == std::optional<AuthorityMode>{AuthorityMode::Paused});
}

void clicks_ripple_once_and_cancelled_clicks_never_do() {
    CASE("a press ripples once, expires, is bounded, and a press that never happened does not ripple");

    Fixture f;
    const auto click = f.command(1, StageAction::Click, 900, 500);
    const Playback playback = f.run(click, PointF{880.0F, 480.0F}, T0);
    std::int64_t press_at = 0;
    for (const auto& step : playback.schedule()) {
        if (step.kind == StepKind::Press) press_at = step.at_ms;
    }
    CHECK(f.director.frame(press_at - 1, RenderTier::Normal).ripples.empty());
    const StageFrame rippling = f.director.frame(press_at + 100, RenderTier::Normal);
    CHECK_EQ(rippling.ripples.size(), std::size_t{1});
    CHECK(rippling.ripples[0].radius > 0.0F && rippling.ripples[0].opacity < 1.0F);
    CHECK(f.director.frame(press_at + f.director.policy().ripple_ms, RenderTier::Normal).ripples.empty());

    CHECK_BECAUSE(f.director.frame(press_at + 100, RenderTier::Conserve).ripples.empty(),
                  "decoration is the first thing a machine that needs its GPU back loses");

    Fixture cancelled;
    const auto doomed = cancelled.command(1, StageAction::Click, 900, 500);
    const Playback never = cancelled.run(doomed, PointF{100.0F, 100.0F}, T0);
    std::int64_t never_at = 0;
    for (const auto& step : never.schedule()) {
        if (step.kind == StepKind::Press) never_at = step.at_ms;
    }
    cancelled.director.command_ended(PlaybackState::Cancelled, T0 + 10);
    CHECK(cancelled.director.frame(never_at + 50, RenderTier::Normal).ripples.empty());

    // Twelve presses at once, from a pointer already on each target.
    Fixture storm;
    for (int i = 0; i < 12; ++i) {
        const auto c = storm.command(1, StageAction::Click, 900 + i * 30, 500);
        (void)storm.run(c, to_output(*storm.table.find(1), 900 + i * 30, 500), T0);
    }
    const StageFrame stormy = storm.director.frame(T0 + 10, RenderTier::Normal);
    CHECK_BECAUSE(stormy.ripples.size() == kMaxRipples,
                  "a burst of clicks draws a bounded number of ripples, never one per click");
    CHECK(!stormy.drag_from.has_value());
}

void captions_say_what_without_saying_the_text() {
    CASE("a typing caption carries a count and nothing else; a refusal carries its code");

    Fixture f;
    auto text = f.command(2, StageAction::Text, 0, 0);
    text.text_chars = 27;
    (void)f.run(text, PointF{1500.0F, 800.0F}, T0);
    const StageFrame typing = f.director.frame(T0 + 100, RenderTier::Normal);
    CHECK(typing.caption.has_value());
    CHECK_EQ(typing.caption->verb, CaptionVerb::Typing);
    CHECK_EQ(typing.caption->text_chars, std::size_t{27});
    CHECK(typing.caption->app_id == "org.codeberg.dnkl.foot");

    // Typing frames the whole window the characters go into — checked on a window in the
    // middle of the stage, where the edge of the stage cannot rescue a framing that is off.
    Fixture central;
    central.table.map(5, "org.gnome.TextEditor", IRect{560, 240, 800, 600});
    auto note = central.command(5, StageAction::Text, 0, 0);
    note.text_chars = 3;
    (void)central.run(note, PointF{960.0F, 540.0F}, T0);
    const VisibleRegion region = visible_region(central.director.camera_target(), 1920, 1080);
    CHECK_BECAUSE(inside(region, PointF{560.0F, 240.0F}) && inside(region, PointF{1360.0F, 840.0F}),
                  "the caret is somewhere this code cannot see, so the whole window is in frame");

    // Looking at a window says so, and shows the whole window being looked at.
    Fixture watched;
    watched.table.map(6, "org.gnome.TextEditor", IRect{560, 240, 800, 600});
    const auto look = watched.command(6, StageAction::Snapshot, 0, 0);
    (void)watched.run(look, PointF{960.0F, 540.0F}, T0);
    CHECK_EQ(watched.director.frame(T0 + 50, RenderTier::Normal).caption->verb, CaptionVerb::Observing);
    const VisibleRegion seen = visible_region(watched.director.camera_target(), 1920, 1080);
    CHECK(inside(seen, PointF{560.0F, 240.0F}) && inside(seen, PointF{1360.0F, 840.0F}));

    auto refused = f.command(2, StageAction::Click, 10, 10);
    f.director.command_refused(refused, CommandRefusal::SurfaceChanged, T0 + 200);
    const StageFrame said = f.director.frame(T0 + 300, RenderTier::Normal);
    CHECK(said.caption->verb == CaptionVerb::Refused);
    CHECK(said.caption->refusal == std::optional<CommandRefusal>{CommandRefusal::SurfaceChanged});

    // A playback the compositor could not finish says why, not that a person took over.
    Fixture stuck;
    auto accent = stuck.command(2, StageAction::Text, 0, 0);
    accent.text_chars = 5;
    (void)stuck.run(accent, PointF{1500.0F, 800.0F}, T0);
    stuck.director.command_failed(CommandRefusal::NotTypeable, T0 + 40);
    const StageFrame failed = stuck.director.frame(T0 + 60, RenderTier::Normal);
    CHECK_EQ(failed.caption->verb, CaptionVerb::Refused);
    CHECK(failed.caption->refusal == std::optional<CommandRefusal>{CommandRefusal::NotTypeable});
    CHECK(failed.caption->app_id == "org.codeberg.dnkl.foot");
    stuck.director.command_failed(CommandRefusal::NotTypeable, T0 + 80);
    CHECK_BECAUSE(stuck.director.frame(T0 + 90, RenderTier::Normal).caption->verb == CaptionVerb::Refused,
                  "a second failure report with nothing running changes nothing");
}

void reduced_motion_cuts_and_headless_draws_nothing() {
    CASE("reduced motion cuts the camera and drops ripples; headless requests no frames");

    DirectorPolicy policy;
    policy.reduced_motion = true;
    Fixture f;
    f.director = StageDirector(policy);
    const auto click = f.command(1, StageAction::Click, 700, 400);
    const Playback playback = f.run(click, PointF{640.0F, 380.0F}, T0);
    CHECK_BECAUSE(f.director.camera_at(T0) == f.director.camera_target(),
                  "under reduced motion a new framing is a cut");
    std::int64_t press_at = 0;
    for (const auto& step : playback.schedule()) {
        if (step.kind == StepKind::Press) press_at = step.at_ms;
    }
    const StageFrame frame = f.director.frame(press_at + 50, RenderTier::Normal);
    CHECK(frame.ripples.empty());
    CHECK(frame.cursor_visible);
    CHECK(frame.caption->opacity == 1.0F);

    Fixture headless;
    (void)headless.run(headless.command(1, StageAction::Click, 700, 400), PointF{}, T0);
    const StageFrame none = headless.director.frame(T0 + 50, RenderTier::Headless);
    CHECK(!none.next_frame_at_ms.has_value());
    CHECK(!none.animate);

    Fixture conserve;
    (void)conserve.run(conserve.command(1, StageAction::Click, 700, 400), PointF{}, T0);
    const StageFrame slow = conserve.director.frame(T0 + 50, RenderTier::Conserve);
    CHECK(slow.next_frame_at_ms == std::optional<std::int64_t>{T0 + 50 + 33});
}

void a_carried_drag_is_drawn_as_carried() {
    CASE("while a drag holds the button, the frame knows where it was picked up");

    Fixture f;
    auto drag = f.command(1, StageAction::Drag, 400, 400);
    drag.to_local_x = 1200;
    drag.to_local_y = 700;
    const Playback playback = f.run(drag, PointF{380.0F, 380.0F}, T0);
    std::int64_t press_at = 0;
    for (const auto& step : playback.schedule()) {
        if (step.kind == StepKind::Press) press_at = step.at_ms;
    }
    const StageFrame carrying = f.director.frame(press_at + 200, RenderTier::Normal);
    CHECK(carrying.cursor_pressed);
    CHECK(carrying.drag_from == std::optional<PointF>{to_output(*f.table.find(1), 400, 400)});
    const StageFrame before = f.director.frame(press_at - 10, RenderTier::Normal);
    CHECK(!before.cursor_pressed && !before.drag_from.has_value());
}

void a_new_output_size_cuts_to_the_whole_stage() {
    CASE("changing the output size cuts to the overview of the new size");

    Fixture f;
    (void)f.run(f.command(1, StageAction::Click, 700, 400), PointF{640.0F, 380.0F}, T0);
    f.director.set_output(2560, 1440, T0 + 100);
    CHECK(f.director.camera_at(T0 + 100) == (ViewRect{1280.0F, 720.0F, 1.0F}));
    CHECK(f.director.camera_target() == (ViewRect{1280.0F, 720.0F, 1.0F}));
}

void the_activity_strip_remembers_what_was_done() {
    CASE("the strip keeps the last few actions, newest first, as captions, and then lets go");

    Fixture f;
    const std::int64_t step_ms = 1'000;

    // A click that finished.
    (void)f.run(f.command(1, StageAction::Click, 700, 400), PointF{640.0F, 380.0F}, T0);
    f.director.command_ended(PlaybackState::Finished, T0 + step_ms);
    // Text into the terminal that a person interrupted.
    auto text = f.command(2, StageAction::Text, 0, 0);
    text.text_chars = 12;
    (void)f.run(text, PointF{1500.0F, 800.0F}, T0 + 2 * step_ms);
    f.director.command_ended(PlaybackState::Cancelled, T0 + 3 * step_ms);
    // A refusal.
    f.director.command_refused(f.command(2, StageAction::Click, 5, 5), CommandRefusal::SurfaceChanged,
                               T0 + 4 * step_ms);

    const StageFrame now = f.director.frame(T0 + 4 * step_ms + 10, RenderTier::Normal);
    CHECK_EQ(now.activity.size(), std::size_t{3});
    CHECK_EQ(now.activity[0].verb, CaptionVerb::Refused);
    CHECK(now.activity[0].refusal == std::optional<CommandRefusal>{CommandRefusal::SurfaceChanged});
    CHECK_EQ(now.activity[1].verb, CaptionVerb::Interrupted);
    CHECK_BECAUSE(now.activity[1].text_chars == 12 && now.activity[1].app_id == "org.codeberg.dnkl.foot",
                  "an entry is the caption as it stood: a count and an application, no text");
    CHECK_EQ(now.activity[2].verb, CaptionVerb::Clicking);
    CHECK_EQ(now.activity[2].app_id, std::string("org.freecad.FreeCAD"));

    // A failure ends as a refusal in the strip too, not as an interruption.
    Fixture failing;
    auto accent = failing.command(2, StageAction::Text, 0, 0);
    accent.text_chars = 2;
    (void)failing.run(accent, PointF{1500.0F, 800.0F}, T0);
    failing.director.command_failed(CommandRefusal::NotTypeable, T0 + 50);
    const StageFrame failed = failing.director.frame(T0 + 60, RenderTier::Normal);
    CHECK(failed.activity.size() == 1 && failed.activity[0].verb == CaptionVerb::Refused &&
          failed.activity[0].refusal == std::optional<CommandRefusal>{CommandRefusal::NotTypeable});

    // Bounded: a burst of refusals keeps only the newest few.
    Fixture burst;
    for (int i = 0; i < 9; ++i) {
        burst.director.command_refused(burst.command(1, StageAction::Click, i, i), CommandRefusal::Expired,
                                       T0 + i);
    }
    CHECK_EQ(burst.director.frame(T0 + 20, RenderTier::Normal).activity.size(), kMaxActivity);

    // It fades out after its time, asks for a frame for that, and then asks for nothing.
    Fixture fading;
    fading.director.command_refused(fading.command(1, StageAction::Click, 1, 1), CommandRefusal::Expired, T0);
    const DirectorPolicy& policy = fading.director.policy();
    const std::int64_t until = T0 + policy.activity_ms;
    const StageFrame steady = fading.director.frame(T0 + policy.caption_hold_ms + 500, RenderTier::Normal);
    CHECK_BECAUSE(steady.next_frame_at_ms == std::optional<std::int64_t>{until - policy.fade_ms},
                  "a still strip wakes once, when its entry starts to fade");
    const StageFrame going = fading.director.frame(until - policy.fade_ms / 2, RenderTier::Normal);
    CHECK(going.activity.size() == 1 && going.activity[0].opacity > 0.4F && going.activity[0].opacity < 0.6F);
    const StageFrame gone = fading.director.frame(until, RenderTier::Normal);
    CHECK(gone.activity.empty());
    CHECK(!gone.next_frame_at_ms.has_value());

    // A person who has just taken over still sees what the agent did.
    Fixture handed;
    (void)handed.run(handed.command(1, StageAction::Click, 700, 400), PointF{640.0F, 380.0F}, T0);
    handed.director.command_ended(PlaybackState::Finished, T0 + 500);
    handed.director.authority_changed(AuthorityMode::Operator, T0 + 600);
    CHECK_EQ(handed.director.frame(T0 + 700, RenderTier::Normal).activity.size(), std::size_t{1});
}

void frames_are_pure() {
    CASE("the same director and time give the same frame");

    Fixture f;
    (void)f.run(f.command(1, StageAction::Click, 700, 400), PointF{640.0F, 380.0F}, T0);
    const StageFrame a = f.director.frame(T0 + 321, RenderTier::Normal);
    const StageFrame b = f.director.frame(T0 + 321, RenderTier::Normal);
    CHECK(a.view == b.view && a.cursor == b.cursor && a.next_frame_at_ms == b.next_frame_at_ms);
    CHECK(a.ripples.size() == b.ripples.size() && a.animate == b.animate);
}

}  // namespace

int main() {
    a_still_stage_costs_nothing();
    a_precise_action_gets_a_closer_shot_than_a_long_one();
    the_camera_does_not_chase_the_cursor();
    an_interrupted_move_continues_from_where_it_is();
    framing_never_shows_past_the_stage();
    the_camera_goes_home_after_a_quiet_spell();
    a_person_taking_over_gets_the_whole_stage_and_the_only_cursor();
    clicks_ripple_once_and_cancelled_clicks_never_do();
    captions_say_what_without_saying_the_text();
    reduced_motion_cuts_and_headless_draws_nothing();
    a_carried_drag_is_drawn_as_carried();
    a_new_output_size_cuts_to_the_whole_stage();
    the_activity_strip_remembers_what_was_done();
    frames_are_pure();
    return archie::visual::test::report();
}
