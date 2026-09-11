// Mode changes: the operator's alone, and never abandoned mid-air.

#include "archie/visual/scene.hpp"

#include "harness.hpp"

using namespace archie::visual;

namespace {

void the_default_view_answers_the_first_question() {
    CASE("the observatory opens on Cerebra");

    ModeDirector modes{};
    CHECK_EQ(modes.current(), SceneMode::Cerebra);
    CHECK(!modes.morphing());
    CHECK_BECAUSE(modes.progress() == 1.0F,
                  "a settled view is not partway through anything");
}

void re_entering_the_current_view_does_nothing() {
    CASE("asking for the mode you are in is not a transition");

    ModeDirector modes{};
    modes.request(SceneMode::Cerebra);
    CHECK_BECAUSE(!modes.morphing(),
                  "replaying an arrival for a view that never went anywhere makes the "
                  "interface feel like it misheard the operator");
}

void a_morph_runs_and_completes() {
    CASE("a mode change moves and then settles");

    ModePolicy policy{};
    policy.morph_ms = 1'000;
    ModeDirector modes{policy};

    modes.request(SceneMode::Watcher);
    CHECK(modes.morphing());
    CHECK_EQ(modes.current(), SceneMode::Cerebra);
    CHECK_EQ(modes.target(), SceneMode::Watcher);
    CHECK_EQ(modes.progress(), 0.0F);

    modes.advance(500);
    CHECK_EQ(modes.progress(), 0.5F);
    CHECK(modes.morphing());

    modes.advance(500);
    CHECK(!modes.morphing());
    CHECK_EQ(modes.current(), SceneMode::Watcher);
    CHECK_EQ(modes.target(), SceneMode::Watcher);
    CHECK_EQ(modes.progress(), 1.0F);
}

void pressing_harder_does_not_slow_you_down() {
    CASE("re-requesting the mode already being entered does not restart it");

    ModePolicy policy{};
    policy.morph_ms = 1'000;
    ModeDirector modes{policy};

    modes.request(SceneMode::Fabric);
    modes.advance(600);
    modes.request(SceneMode::Fabric);

    CHECK_BECAUSE(modes.progress() == 0.6F,
                  "restarting on a second keypress means the harder an impatient operator "
                  "asks, the slower they arrive");
    CHECK(!modes.queued().has_value());
}

void only_the_last_request_survives() {
    CASE("mashing keys lands on the last view, not a tour of all of them");

    ModePolicy policy{};
    policy.morph_ms = 1'000;
    ModeDirector modes{policy};

    modes.request(SceneMode::Watcher);
    modes.advance(100);
    modes.request(SceneMode::Brain);
    modes.request(SceneMode::Fabric);

    CHECK_BECAUSE(modes.queued().has_value() && *modes.queued() == SceneMode::Fabric,
                  "the newest request replaces the waiting one; a queue that grew would fly "
                  "the operator through every view they passed through on the way to changing "
                  "their mind, and a queue that kept the first would strand them in the view "
                  "they had already abandoned");
    CHECK_EQ(modes.target(), SceneMode::Watcher);

    // The morph in flight finishes, then the queued one begins.
    modes.advance(900);
    CHECK_EQ(modes.current(), SceneMode::Watcher);
    CHECK_EQ(modes.target(), SceneMode::Fabric);
    CHECK(!modes.queued().has_value());

    modes.advance(1'000);
    CHECK_EQ(modes.current(), SceneMode::Fabric);
    CHECK(!modes.morphing());
}

void turning_back_reverses_in_place() {
    CASE("asking for the mode being left retraces the path instead of completing it");

    ModePolicy policy{};
    policy.morph_ms = 1'000;
    ModeDirector modes{policy};

    modes.request(SceneMode::Watcher);
    modes.advance(300);
    CHECK_EQ(modes.progress(), 0.3F);

    modes.request(SceneMode::Cerebra);

    CHECK_EQ(modes.current(), SceneMode::Watcher);
    CHECK_EQ(modes.target(), SceneMode::Cerebra);
    CHECK_BECAUSE(modes.progress() == 0.7F,
                  "the scene is 30% of the way to Watcher, which is 70% of the way back from "
                  "Watcher; starting the reversal at zero would teleport it the rest of the "
                  "way out before coming back");
    CHECK_BECAUSE(!modes.queued().has_value(),
                  "turning back is a change of mind, not a stop on the way to somewhere else");

    modes.advance(300);
    CHECK_BECAUSE(!modes.morphing(),
                  "the way back is as long as the distance actually travelled, so a change of "
                  "mind three tenths in costs three tenths to undo, not a full morph");
    CHECK_EQ(modes.current(), SceneMode::Cerebra);
}

void turning_back_discards_anything_queued() {
    CASE("changing your mind entirely clears what was waiting");

    ModePolicy policy{};
    policy.morph_ms = 1'000;
    ModeDirector modes{policy};

    modes.request(SceneMode::Watcher);
    modes.advance(200);
    modes.request(SceneMode::Fabric);
    CHECK(modes.queued().has_value());

    modes.request(SceneMode::Cerebra);
    CHECK_BECAUSE(!modes.queued().has_value(),
                  "an operator who has gone back to where they started is not still on their "
                  "way to Fabric, and arriving there a second later would be baffling");
    CHECK_EQ(modes.target(), SceneMode::Cerebra);

    for (int i = 0; i < 20; ++i) {
        modes.advance(100);
    }
    CHECK_EQ(modes.current(), SceneMode::Cerebra);
    CHECK(!modes.morphing());
}

void a_queued_request_for_the_arriving_mode_is_dropped() {
    CASE("a queued view that turns out to be the one we land on is not re-entered");

    ModePolicy policy{};
    policy.morph_ms = 1'000;
    ModeDirector modes{policy};

    modes.request(SceneMode::Watcher);
    modes.advance(100);
    modes.request(SceneMode::Brain);
    // Then back to the one already in flight: this clears the queue rather than leaving
    // Brain waiting behind a morph that is now heading to Watcher anyway.
    modes.request(SceneMode::Watcher);
    CHECK(!modes.queued().has_value());

    modes.advance(1'000);
    CHECK_EQ(modes.current(), SceneMode::Watcher);
    CHECK_BECAUSE(!modes.morphing(),
                  "landing and immediately setting off again would give the operator a second "
                  "transition they never asked for");
}

void time_never_runs_backwards() {
    CASE("a zero or negative frame delta does not move the morph");

    ModePolicy policy{};
    policy.morph_ms = 1'000;
    ModeDirector modes{policy};

    modes.request(SceneMode::Fabric);
    modes.advance(400);
    const float held = modes.progress();

    modes.advance(0);
    CHECK_EQ(modes.progress(), held);
    modes.advance(-10'000);
    CHECK_BECAUSE(modes.progress() == held,
                  "a clock that steps backwards between frames must not unwind a transition "
                  "the viewer has already watched");
    CHECK(modes.morphing());
}

void nothing_but_the_operator_changes_the_mode() {
    CASE("time alone never switches views");

    ModeDirector modes{};
    for (int i = 0; i < 10'000; ++i) {
        modes.advance(16);
    }
    CHECK_BECAUSE(modes.current() == SceneMode::Cerebra && !modes.morphing(),
                  "a display that reorganises itself because something happened elsewhere "
                  "takes the screen away from whoever was reading it, at the moment they were "
                  "most likely reading it for a reason");
}

void an_overshooting_frame_settles_exactly() {
    CASE("a long frame lands on the destination rather than past it");

    ModePolicy policy{};
    policy.morph_ms = 1'000;
    ModeDirector modes{policy};

    modes.request(SceneMode::Brain);
    modes.advance(50'000);

    CHECK(!modes.morphing());
    CHECK_EQ(modes.current(), SceneMode::Brain);
    CHECK_BECAUSE(modes.progress() == 1.0F,
                  "a dropped frame or a stalled thread must not leave the scene reporting a "
                  "progress of fifty");
}

void a_cut_is_honoured_when_asked_for() {
    CASE("a zero-length morph settles immediately rather than dividing by zero");

    ModePolicy instant{};
    instant.morph_ms = 0;
    ModeDirector modes{instant};

    modes.request(SceneMode::Watcher);
    CHECK(!modes.morphing());
    CHECK_EQ(modes.current(), SceneMode::Watcher);
    CHECK_EQ(modes.progress(), 1.0F);
}

}  // namespace

int main() {
    the_default_view_answers_the_first_question();
    re_entering_the_current_view_does_nothing();
    a_morph_runs_and_completes();
    pressing_harder_does_not_slow_you_down();
    only_the_last_request_survives();
    turning_back_reverses_in_place();
    turning_back_discards_anything_queued();
    a_queued_request_for_the_arriving_mode_is_dropped();
    time_never_runs_backwards();
    nothing_but_the_operator_changes_the_mode();
    an_overshooting_frame_settles_exactly();
    a_cut_is_honoured_when_asked_for();
    return archie::visual::test::report();
}
