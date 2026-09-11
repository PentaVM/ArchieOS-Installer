// Camera movement: deterministic, continuous, and never left staring at nothing.

#include "archie/visual/camera.hpp"

#include <vector>

#include "harness.hpp"

using namespace archie::visual;

namespace {

CameraPose pose_at(float x, float fov = 45.0F) {
    CameraPose p{};
    p.eye = Vec3{x, x * 2.0F, x * 3.0F};
    p.target = Vec3{0.0F, 0.0F, 0.0F};
    p.fov_degrees = fov;
    return p;
}

/// Drive a director through a fixed script, sampling the pose at every step.
std::vector<CameraPose> run_script(CameraDirector& camera) {
    std::vector<CameraPose> samples;
    camera.set_home(pose_at(10.0F), false);
    for (int i = 0; i < 5; ++i) {
        camera.advance(16);
        samples.push_back(camera.pose());
    }
    camera.focus("node.a", pose_at(2.0F, 30.0F));
    for (int i = 0; i < 20; ++i) {
        camera.advance(16);
        samples.push_back(camera.pose());
    }
    camera.release_focus();
    for (int i = 0; i < 30; ++i) {
        camera.advance(16);
        samples.push_back(camera.pose());
    }
    return samples;
}

void the_same_script_produces_the_same_path() {
    CASE("camera movement is deterministic to the bit");

    CameraDirector first{};
    CameraDirector second{};
    const std::vector<CameraPose> a = run_script(first);
    const std::vector<CameraPose> b = run_script(second);

    CHECK_EQ(a.size(), b.size());
    bool identical = a.size() == b.size();
    for (std::size_t i = 0; i < a.size() && i < b.size(); ++i) {
        if (!(a[i] == b[i])) {
            identical = false;
        }
    }
    CHECK_BECAUSE(identical,
                  "a camera path that differs between runs cannot be asserted on, compared "
                  "across screenshots, or replayed; every source of variation here is a "
                  "source of a bug that only appears sometimes");
    CHECK(first.pose() == second.pose());
    CHECK_EQ(first.state(), second.state());
}

void an_interrupted_move_continues_from_where_the_camera_is() {
    CASE("a new instruction never snaps the camera back to the old origin");

    CameraDirector camera{};
    camera.set_home(pose_at(0.0F), false);
    camera.focus("far", pose_at(100.0F));

    // Halfway through the dive.
    camera.advance(350);
    const CameraPose midway = camera.pose();
    CHECK(camera.moving());

    // The operator changes their mind and picks a different node.
    camera.focus("other", pose_at(-50.0F));
    const CameraPose after = camera.pose();

    CHECK_BECAUSE(after == midway,
                  "the new move must begin from the pose on screen; starting it from where "
                  "the previous move began teleports the camera backwards for one frame, "
                  "which reads as a glitch rather than a change of mind");
    CHECK_EQ(camera.focused_node(), std::string{"other"});
}

void a_retargeted_move_still_arrives() {
    CASE("interruption does not wedge the camera");

    CameraDirector camera{};
    camera.set_home(pose_at(0.0F), false);

    camera.focus("a", pose_at(10.0F));
    camera.advance(200);
    camera.focus("b", pose_at(20.0F));
    camera.advance(200);
    camera.focus("a", pose_at(10.0F));

    for (int i = 0; i < 100; ++i) {
        camera.advance(16);
    }
    CHECK_EQ(camera.state(), CameraState::Focused);
    CHECK_BECAUSE(camera.pose() == pose_at(10.0F),
                  "however many times the destination changed, the camera must end up at the "
                  "last one it was given");
    CHECK_EQ(camera.progress(), 1.0F);
}

void easing_starts_and_stops_at_rest() {
    CASE("smoothstep: no jerk into motion, no dead stop");

    CHECK_EQ(ease(0.0F), 0.0F);
    CHECK_EQ(ease(1.0F), 1.0F);
    CHECK_BECAUSE(ease(0.5F) == 0.5F, "symmetric about the midpoint");

    // Monotone, and slower at the ends than in the middle.
    const float early = ease(0.1F) - ease(0.0F);
    const float middle = ease(0.55F) - ease(0.45F);
    CHECK_BECAUSE(middle > early * 2.0F,
                  "a move at constant velocity starts and stops abruptly, which viewers read "
                  "as the software stuttering rather than the camera moving");

    // Out-of-range input is clamped, not extrapolated past the destination.
    CHECK_EQ(ease(-1.0F), 0.0F);
    CHECK_EQ(ease(5.0F), 1.0F);
}

void a_move_ends_exactly_on_its_destination() {
    CASE("no residual offset after arrival");

    CameraDirector camera{};
    camera.set_home(pose_at(0.0F), false);
    camera.focus("n", pose_at(7.5F, 33.0F));

    // Deliberately overshoot the duration with an uneven step.
    for (int i = 0; i < 10; ++i) {
        camera.advance(97);
    }

    CHECK_BECAUSE(camera.pose() == pose_at(7.5F, 33.0F),
                  "interpolating to 0.999 and stopping leaves the camera permanently just "
                  "short of every destination, and the error accumulates across moves");
    CHECK(!camera.moving());
    CHECK_EQ(camera.state(), CameraState::Focused);
}

void time_never_runs_backwards() {
    CASE("a zero or negative frame delta moves nothing");

    CameraDirector camera{};
    camera.set_home(pose_at(0.0F), false);
    camera.focus("n", pose_at(10.0F));
    camera.advance(300);

    const CameraPose settled = camera.pose();
    camera.advance(0);
    CHECK(camera.pose() == settled);
    camera.advance(-5'000);
    CHECK_BECAUSE(camera.pose() == settled,
                  "a clock that steps backwards between two frames must not rewind the camera "
                  "along a path the viewer has already watched");
}

void focus_is_committed_the_moment_it_is_asked_for() {
    CASE("the inspector populates during the dive, not after it");

    CameraDirector camera{};
    camera.set_home(pose_at(0.0F), false);
    camera.focus("chosen", pose_at(5.0F));

    CHECK_BECAUSE(camera.focused_node() == std::string{"chosen"},
                  "an operator who has clicked a node has already committed; making them "
                  "wait 700ms for the panel to admit which node they picked is latency they "
                  "will read as the interface not registering the click");
    CHECK_EQ(camera.state(), CameraState::Transitioning);

    for (int i = 0; i < 60; ++i) {
        camera.advance(16);
    }
    CHECK_EQ(camera.state(), CameraState::Focused);
}

void releasing_a_focus_goes_home() {
    CASE("release returns to the mode's home and forgets the node");

    CameraDirector camera{};
    const CameraPose home = pose_at(12.0F);
    camera.set_home(home, false);
    camera.focus("n", pose_at(1.0F));
    for (int i = 0; i < 60; ++i) {
        camera.advance(16);
    }

    camera.release_focus();
    CHECK_EQ(camera.state(), CameraState::Returning);
    CHECK(camera.focused_node().empty());

    for (int i = 0; i < 80; ++i) {
        camera.advance(16);
    }
    CHECK_EQ(camera.state(), CameraState::Resting);
    CHECK(camera.pose() == home);
}

void a_vanished_node_does_not_hold_the_camera() {
    CASE("the camera leaves a node that has been removed");

    CameraDirector camera{};
    const CameraPose home = pose_at(20.0F);
    camera.set_home(home, false);
    camera.focus("doomed", pose_at(3.0F));
    for (int i = 0; i < 60; ++i) {
        camera.advance(16);
    }
    CHECK_EQ(camera.state(), CameraState::Focused);

    camera.note_node_removed("doomed");
    CHECK_BECAUSE(camera.state() == CameraState::Returning,
                  "a camera left on a node that no longer exists shows an empty patch of space "
                  "beside an inspector of fields that will never update again, and nothing "
                  "tells the operator that from a node which has merely gone quiet");
    CHECK(camera.focused_node().empty());

    for (int i = 0; i < 80; ++i) {
        camera.advance(16);
    }
    CHECK(camera.pose() == home);
}

void an_unrelated_removal_is_ignored() {
    CASE("another node disappearing does not interrupt an inspection");

    CameraDirector camera{};
    camera.set_home(pose_at(20.0F), false);
    camera.focus("watched", pose_at(3.0F));
    for (int i = 0; i < 60; ++i) {
        camera.advance(16);
    }

    camera.note_node_removed("somebody.else");
    CHECK_BECAUSE(camera.state() == CameraState::Focused,
                  "nodes come and go constantly; yanking the operator out of what they are "
                  "reading every time an unrelated one departs would make inspection useless");
    CHECK_EQ(camera.focused_node(), std::string{"watched"});
}

void rehoming_does_not_evict_an_operator() {
    CASE("a mode change while inspecting does not steal the camera");

    CameraDirector camera{};
    camera.set_home(pose_at(0.0F), false);
    camera.focus("reading", pose_at(4.0F));
    for (int i = 0; i < 60; ++i) {
        camera.advance(16);
    }
    const CameraPose held = camera.pose();

    camera.set_home(pose_at(99.0F));
    CHECK_BECAUSE(camera.pose() == held && camera.state() == CameraState::Focused,
                  "re-homing underneath an operator ends their inspection without them asking; "
                  "the new home waits until they release it");

    // The same protection has to hold while the camera is still diving. The operator has
    // committed to the node; the fact that the dive has not landed yet does not make it
    // acceptable to redirect them somewhere they did not ask to go.
    CameraDirector diving{};
    diving.set_home(pose_at(0.0F), false);
    diving.focus("mid-dive", pose_at(4.0F));
    diving.advance(200);
    const CameraPose in_flight = diving.pose();
    diving.set_home(pose_at(77.0F));
    CHECK_BECAUSE(diving.pose() == in_flight,
                  "a mode change landing between the click and the arrival must not divert a "
                  "dive that is already in the air");
    for (int i = 0; i < 60; ++i) {
        diving.advance(16);
    }
    CHECK_BECAUSE(diving.state() == CameraState::Focused && diving.pose() == pose_at(4.0F),
                  "and the dive still finishes on the node the operator picked");

    // And once released, it goes to the new home, not the old one.
    camera.release_focus();
    for (int i = 0; i < 80; ++i) {
        camera.advance(16);
    }
    CHECK(camera.pose() == pose_at(99.0F));
}

void a_cut_is_honoured_when_asked_for() {
    CASE("a zero-length move settles immediately rather than dividing by zero");

    CameraPolicy instant{};
    instant.focus_transition_ms = 0;
    CameraDirector camera{instant};
    camera.set_home(pose_at(0.0F), false);

    camera.focus("n", pose_at(6.0F));
    CHECK(camera.pose() == pose_at(6.0F));
    CHECK_EQ(camera.state(), CameraState::Focused);
    CHECK(!camera.moving());
    CHECK_EQ(camera.progress(), 1.0F);
}

void the_initial_home_can_be_set_without_a_move() {
    CASE("startup places the camera rather than flying it in from the origin");

    CameraDirector camera{};
    camera.set_home(pose_at(15.0F), false);
    CHECK_BECAUSE(camera.pose() == pose_at(15.0F),
                  "animating the very first placement means the observatory opens with a swoop "
                  "from a pose that was never meaningful");
    CHECK_EQ(camera.state(), CameraState::Resting);
}

}  // namespace

int main() {
    the_same_script_produces_the_same_path();
    an_interrupted_move_continues_from_where_the_camera_is();
    a_retargeted_move_still_arrives();
    easing_starts_and_stops_at_rest();
    a_move_ends_exactly_on_its_destination();
    time_never_runs_backwards();
    focus_is_committed_the_moment_it_is_asked_for();
    releasing_a_focus_goes_home();
    a_vanished_node_does_not_hold_the_camera();
    an_unrelated_removal_is_ignored();
    rehoming_does_not_evict_an_operator();
    a_cut_is_honoured_when_asked_for();
    the_initial_home_can_be_set_without_a_move();
    return archie::visual::test::report();
}
