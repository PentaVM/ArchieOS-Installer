#include "archie/cortex/resources.hpp"

#include <algorithm>
#include <cmath>
#include <fstream>
#include <iostream>
#include <sstream>

using namespace archie::cortex;
namespace {
int failures = 0, checks = 0;
#define CHECK(x) do { ++checks; if (!(x)) { ++failures; std::cerr << __LINE__ << ": " #x "\n"; } } while(false)

FocusSource source(std::string id, FocusSourceKind kind, std::vector<std::string> phrases, bool text, double fps) {
    FocusSource s; s.node_id = std::move(id); s.kind = kind; s.phrases = std::move(phrases);
    s.has_text = text; s.max_fps = fps; return s;
}

FocusState state() {
    FocusState f; f.focus_id = "focus:workday"; f.half_life_ms = 1000;
    f.activations = {{"camera:desk", 1.0, 3000, false}, {"screen:0", 1.0, 2000, false}};
    return f;
}

std::vector<FocusSource> sources() {
    return {source("camera:desk", FocusSourceKind::Camera, {"taza \"roja\"", "llaves"}, false, 30),
            source("screen:0", FocusSourceKind::Screen, {"botón enviar"}, true, 15),
            source("camera:door", FocusSourceKind::Camera, {}, false, 30)};
}

void shares_follow_decayed_activation() {
    auto advice = advise_resources(state(), sources(), {}, {"iris.detect"}, 3000);
    CHECK(advice.has_value());
    if (!advice) return;
    // The screen was activated one half-life earlier: half the camera's weight.
    CHECK(std::abs(advice->foci[0].share - 2.0 / 3.0) < 1e-12);
    CHECK(std::abs(advice->foci[1].share - 1.0 / 3.0) < 1e-12);
    CHECK(advice->foci[2].share == 0.0 && advice->foci[2].mode == FocusMode::Off && advice->foci[2].max_fps == 0.0);
    CHECK(advice->foci[0].mode == FocusMode::Focused);
    CHECK(advice->foci[1].mode == FocusMode::FocusedAndOpen && advice->foci[1].read_text);
    CHECK(!validate(*advice));
    auto pinned = state(); pinned.activations[1].pinned = true;
    auto even = advise_resources(pinned, sources(), {}, {}, 3000);
    CHECK(even && std::abs(even->foci[0].share - 0.5) < 1e-12);
    auto suspended = state(); suspended.suspended = true;
    auto off = advise_resources(suspended, sources(), {}, {}, 3000);
    CHECK(off && off->foci[0].mode == FocusMode::Off && off->foci[1].share == 0.0);
}

void predictions_come_from_measured_jobs() {
    std::vector<TrainingEvent> history;
    for (int i = 0; i < 5; ++i) {
        TrainingEvent e; e.event_id = "job" + std::to_string(i); e.subject_id = "iris.detect";
        e.snapshot_id = "s"; e.evidence_ref = "accel:job"; e.signal = LearningSignal::ProcessSample;
        e.observed.latency_ms = 40.0 + i; e.observed.memory_bytes = 3.0e9; e.recorded_at_ms = 100 + i;
        history.push_back(e);
    }
    auto advice = advise_resources(state(), sources(), history, {"iris.detect", "wanda.lncm"}, 3000);
    CHECK(advice && advice->models.size() == 2);
    if (!advice || advice->models.size() != 2) return;
    CHECK(advice->models[0].latency_ms && std::abs(*advice->models[0].latency_ms - 42.0) < 1e-9);
    CHECK(!advice->models[1].latency_ms);
}

void advice_is_validated_and_only_proposed() {
    auto advice = *advise_resources(state(), sources(), {}, {}, 3000);
    auto proposal = propose(advice);
    CHECK(proposal.kind == ProposalKind::CandidateSchedulerChange);
    CHECK(proposal.rationale.find("Policy envelope") != std::string::npos);
    auto bad = advice; bad.foci[1].focus_id = bad.foci[0].focus_id;
    CHECK(validate(bad) == ContractRefusal::DuplicateNode);
    bad = advice; bad.foci[0].share = 0.9; bad.foci[1].share = 0.9;
    CHECK(validate(bad) == ContractRefusal::InvalidCorrection);
    bad = advice; bad.foci[0].max_fps = std::nan("");
    CHECK(validate(bad) == ContractRefusal::InvalidCorrection);
    bad = advice; bad.foci[0].phrases.assign(17, "x");
    CHECK(validate(bad) == ContractRefusal::SummaryTooLong);
    bad = advice; bad.foci[0].phrases = {std::string(64, 'a')};
    CHECK(!validate(bad));
    bad.foci[0].phrases = {std::string(65, 'a')};
    CHECK(validate(bad) == ContractRefusal::SummaryTooLong);
    bad.foci[0].phrases = {"ñandú ñandú ñandú ñandú ñandú ñandú ñandú ñandú ñandú ñandú ñan"};  // 64 characters, 87 bytes
    CHECK(!validate(bad));
    bad = advice; bad.advised_at_ms = 0;
    CHECK(validate(bad) == ContractRefusal::InvalidTimestamp);
}

void focus_plan_matches_the_shared_fixture() {
    ResourceAdvice advice;
    advice.advice_id = "a"; advice.focus_state_id = "f"; advice.advised_at_ms = 1;
    FocusAdvice camera; camera.focus_id = "camera:desk"; camera.share = 0.75; camera.mode = FocusMode::Focused;
    camera.phrases = {"taza \"roja\"", "llaves"}; camera.max_fps = 30;
    FocusAdvice screen; screen.focus_id = "screen:0"; screen.share = 0.25; screen.mode = FocusMode::FocusedAndOpen;
    screen.phrases = {"botón enviar"}; screen.read_text = true; screen.max_fps = 15;
    FocusAdvice door; door.focus_id = "camera:door";
    advice.foci = {camera, screen, door};
    CHECK(!validate(advice));
    std::ifstream file(ARCHIE_CORTEX_FIXTURES "/focus-plan.json");
    std::stringstream expected; expected << file.rdbuf();
    std::string fixture = expected.str();
    while (!fixture.empty() && (fixture.back() == '\n' || fixture.back() == '\r')) fixture.pop_back();
    CHECK(focus_plan_json(advice) == fixture);
}

void descriptors_propose_only() {
    auto descriptors = model_capability_descriptors();
    CHECK(descriptors.size() == 4);
    NeuronRegistry registry;
    for (const auto& d : descriptors) {
        CHECK(d.authority_boundary_ref == "authority:proposes-only");
        CHECK(std::find(d.telemetry_fields.begin(), d.telemetry_fields.end(), "latency_ms") != d.telemetry_fields.end());
        CHECK(registry.upsert(d));
    }
}
}  // namespace

int main() {
    shares_follow_decayed_activation();
    predictions_come_from_measured_jobs();
    advice_is_validated_and_only_proposed();
    focus_plan_matches_the_shared_fixture();
    descriptors_propose_only();
    std::cout << checks - failures << "/" << checks << " resource checks passed\n";
    return failures == 0 ? 0 : 1;
}
