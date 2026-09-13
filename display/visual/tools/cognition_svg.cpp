// `archie-visual-cognition` — watch what Archie is doing, and count what it cost.
//
// Runs a scripted pass through Cerebra against `CognitionMotion` the way a renderer is
// meant to drive it: draw when a projection arrives, draw again at `next_frame_at_ms`,
// and never on a timer of its own. Every frame it draws is sampled into an SVG whose
// SMIL animation replays exactly those frames — so the file shows the motion the engine
// decided and, by construction, nothing in between.
//
// It then prints the number that matters: how many frames that took, beside what a
// fixed sixty-frames-a-second loop would have drawn over the same fifteen seconds.
//
// What it is not: the native renderer. No materials, no shaders, no Qt. The positions,
// intensities, breath, marker and timing all come from the engine; the colours and the
// ring geometry are chosen here and stated here.
//
// Usage:  archie-visual-cognition [out.svg] [normal|conserve|ultra] [--reduced-motion]

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <optional>
#include <string>
#include <vector>

#include "archie/visual/cognition.hpp"

using namespace archie::visual;

namespace {

constexpr double kWidth = 980.0;
constexpr double kHeight = 600.0;
constexpr double kCx = 300.0;
constexpr double kCy = 290.0;
constexpr double kRing = 190.0;
constexpr double kPi = 3.14159265358979323846;
constexpr std::int64_t kEndMs = 15'000;

struct Step {
    std::int64_t at_ms;
    std::optional<CognitionReading> reading;
    const char* caption;
};

CognitionReading pass(const char* trace, LoopStage stage, bool in_progress, bool silent) {
    CognitionReading r;
    r.trace_id = trace;
    r.stage = stage;
    r.in_progress = in_progress;
    r.silent = silent;
    r.privacy = PrivacyGrade::Internal;
    return r;
}

std::vector<Step> scenario() {
    std::vector<Step> steps;
    steps.push_back({0, std::nullopt, "No projection yet: the panel says it does not know"});

    steps.push_back({900, pass("a", LoopStage::Observe, true, false),
                     "Someone starts speaking"});
    steps.push_back({1'150, pass("a", LoopStage::Interpret, false, true),
                     "They pause mid-sentence: the floor says wait. The vault is never opened"});
    steps.push_back({4'900, pass("b", LoopStage::Observe, true, false),
                     "They finish the question: a new pass begins"});
    steps.push_back({5'150, pass("b", LoopStage::Interpret, true, false),
                     "The floor clears Archie to answer"});
    CognitionReading context = pass("b", LoopStage::Contextualize, true, false);
    context.privacy = PrivacyGrade::Restricted;
    steps.push_back({5'400, context, "Context capsule assembled from the vault (restricted)"});
    CognitionReading reason = pass("b", LoopStage::Reason, true, false);
    reason.privacy = PrivacyGrade::Restricted;
    steps.push_back({5'700, reason, "A local model is reasoning: a slow breath, fresh data only"});
    CognitionReading select = pass("b", LoopStage::SelectCapability, true, false);
    select.privacy = PrivacyGrade::Restricted;
    steps.push_back({8'300, select, "It proposes a tool call"});
    CognitionReading held = pass("b", LoopStage::Authorize, false, true);
    held.privacy = PrivacyGrade::Restricted;
    held.authorization = OutcomeKind::AwaitingApproval;
    steps.push_back({8'550, held, "The gate holds it for the operator's approval"});
    CognitionReading acted = pass("b", LoopStage::Act, false, false);
    acted.privacy = PrivacyGrade::Restricted;
    acted.authorization = OutcomeKind::Proceeded;
    acted.execution = OutcomeKind::Proceeded;
    steps.push_back({11'200, acted, "Approved: the tool runs"});
    CognitionReading verified = acted;
    verified.stage = LoopStage::Verify;
    verified.verification = OutcomeKind::Proceeded;
    steps.push_back({11'450, verified, "The result is checked against the expected state"});
    CognitionReading remembered = verified;
    remembered.stage = LoopStage::Remember;
    remembered.memory = OutcomeKind::Silent;
    steps.push_back({11'700, remembered,
                     "Memory declines to keep it. The panel settles and stops drawing"});
    return steps;
}

struct Sample {
    std::int64_t at_ms;
    CognitionFrame frame;
    std::size_t caption;
};

struct Run {
    std::vector<Sample> samples;
    int frames = 0;
};

/// Drive the engine the way a renderer must: on input, and when asked.
Run simulate(const std::vector<Step>& steps, const CognitionMotionPolicy& policy, RenderTier tier) {
    CognitionMotion motion{policy};
    Run run;
    std::size_t next_step = 0;
    std::size_t caption = 0;
    std::optional<std::int64_t> requested{};

    std::int64_t now = 0;
    while (true) {
        std::optional<std::int64_t> candidate{};
        if (next_step < steps.size()) {
            candidate = steps[next_step].at_ms;
        }
        if (requested.has_value()) {
            candidate = candidate.has_value() ? std::min(*candidate, *requested) : requested;
        }
        if (!candidate.has_value() || *candidate > kEndMs) {
            break;
        }
        now = *candidate;
        while (next_step < steps.size() && steps[next_step].at_ms == now) {
            if (steps[next_step].reading.has_value()) {
                motion.observe(*steps[next_step].reading, 0, now);
            } else {
                motion.observe_unmeasured(now);
            }
            caption = next_step;
            ++next_step;
        }
        const CognitionFrame frame = motion.frame(now, tier);
        run.samples.push_back({now, frame, caption});
        ++run.frames;
        requested = frame.next_frame_at_ms;
    }
    return run;
}

const char* activity_name(Activity a) {
    switch (a) {
        case Activity::Unmeasured: return "NO DATA";
        case Activity::Thinking: return "THINKING";
        case Activity::Silent: return "STAYING SILENT";
        case Activity::Acted: return "ACTED";
        case Activity::AwaitingApproval: return "AWAITING APPROVAL";
        case Activity::Refused: return "REFUSED";
        case Activity::Failed: return "FAILED";
    }
    return "NO DATA";
}

const char* activity_colour(Activity a) {
    switch (a) {
        case Activity::Unmeasured: return "#6b7280";
        case Activity::Thinking: return "#60a5fa";
        case Activity::Silent: return "#cbd5e1";
        case Activity::Acted: return "#34d399";
        case Activity::AwaitingApproval: return "#fbbf24";
        case Activity::Refused: return "#f87171";
        case Activity::Failed: return "#fb7185";
    }
    return "#6b7280";
}

const char* grade_name(std::optional<PrivacyGrade> g) {
    if (!g.has_value()) return "privacy: none";
    switch (*g) {
        case PrivacyGrade::Public: return "privacy: public";
        case PrivacyGrade::Internal: return "privacy: internal";
        case PrivacyGrade::Sensitive: return "privacy: sensitive";
        case PrivacyGrade::Restricted: return "privacy: restricted (stays on this machine)";
        case PrivacyGrade::Secret: return "privacy: secret";
    }
    return "privacy: none";
}

const char* stage_label(LoopStage s) {
    switch (s) {
        case LoopStage::Observe: return "observe";
        case LoopStage::Contextualize: return "context";
        case LoopStage::Interpret: return "floor";
        case LoopStage::Reason: return "reason";
        case LoopStage::Plan: return "plan";
        case LoopStage::SelectCapability: return "capability";
        case LoopStage::Authorize: return "authorize";
        case LoopStage::Act: return "act";
        case LoopStage::Verify: return "verify";
        case LoopStage::UpdateState: return "state";
        case LoopStage::Remember: return "remember";
    }
    return "";
}

double angle_of(double ring_position) {
    return -kPi / 2.0 + ring_position * (2.0 * kPi / static_cast<double>(kLoopStageCount));
}

std::string key_times(const std::vector<Sample>& samples) {
    std::string out;
    char buf[32];
    for (std::size_t i = 0; i < samples.size(); ++i) {
        std::snprintf(buf, sizeof buf, "%s%.5f", i == 0 ? "" : ";",
                      static_cast<double>(samples[i].at_ms) / static_cast<double>(kEndMs));
        out += buf;
    }
    return out;
}

template <typename F>
std::string values(const std::vector<Sample>& samples, F&& value_of) {
    std::string out;
    for (std::size_t i = 0; i < samples.size(); ++i) {
        if (i != 0) out += ';';
        out += value_of(samples[i]);
    }
    return out;
}

std::string num(double v) {
    char buf[32];
    std::snprintf(buf, sizeof buf, "%.3f", v);
    return buf;
}

void animate(std::FILE* f, const char* attribute, const std::string& times, const std::string& vals) {
    std::fprintf(f,
                 "<animate attributeName=\"%s\" dur=\"%.3fs\" repeatCount=\"indefinite\" "
                 "calcMode=\"discrete\" keyTimes=\"%s\" values=\"%s\"/>",
                 attribute, static_cast<double>(kEndMs) / 1000.0, times.c_str(), vals.c_str());
}

void write_svg(std::FILE* f, const Run& run, const std::vector<Step>& steps, const Run& conserve,
               const Run& reduced) {
    const std::string times = key_times(run.samples);
    std::fprintf(f,
                 "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 %.0f %.0f\" "
                 "font-family=\"Segoe UI, Helvetica, Arial, sans-serif\">\n",
                 kWidth, kHeight);
    std::fprintf(f, "<rect width=\"100%%\" height=\"100%%\" fill=\"#0b0f14\"/>\n");

    // Ring guide.
    std::fprintf(f,
                 "<circle cx=\"%.1f\" cy=\"%.1f\" r=\"%.1f\" fill=\"none\" stroke=\"#1f2937\" "
                 "stroke-width=\"2\"/>\n",
                 kCx, kCy, kRing);

    // Breath and activity colour at the centre.
    std::fprintf(f, "<circle cx=\"%.1f\" cy=\"%.1f\" r=\"46\" fill=\"#60a5fa\" fill-opacity=\"0.18\">", kCx, kCy);
    animate(f, "r", times, values(run.samples, [](const Sample& s) { return num(46.0 + 16.0 * s.frame.breath); }));
    animate(f, "fill", times, values(run.samples, [](const Sample& s) { return std::string{activity_colour(s.frame.activity)}; }));
    std::fprintf(f, "</circle>\n");

    // Stage nodes, in execution order.
    for (std::size_t i = 0; i < kLoopStageCount; ++i) {
        const double a = angle_of(static_cast<double>(i));
        const double x = kCx + kRing * std::cos(a);
        const double y = kCy + kRing * std::sin(a);
        std::fprintf(f, "<circle cx=\"%.1f\" cy=\"%.1f\" r=\"17\" fill=\"#60a5fa\">", x, y);
        animate(f, "fill-opacity", times, values(run.samples, [i](const Sample& s) {
                    return num(static_cast<double>(s.frame.stage_intensity[i] * s.frame.confidence));
                }));
        animate(f, "fill", times, values(run.samples, [](const Sample& s) {
                    return std::string{activity_colour(s.frame.activity)};
                }));
        std::fprintf(f, "</circle>\n");
        const double lx = kCx + (kRing + 36.0) * std::cos(a);
        const double ly = kCy + (kRing + 36.0) * std::sin(a) + 4.0;
        std::fprintf(f,
                     "<text x=\"%.1f\" y=\"%.1f\" fill=\"#9ca3af\" font-size=\"12\" "
                     "text-anchor=\"middle\">%s</text>\n",
                     lx, ly, stage_label(kExecutionOrder[i]));
    }

    // The travelling marker: present only while the engine says it travels.
    std::fprintf(f, "<circle r=\"8\" fill=\"#f9fafb\" cx=\"%.1f\" cy=\"%.1f\">", kCx, kCy - kRing);
    animate(f, "cx", times, values(run.samples, [](const Sample& s) {
                const double p = s.frame.marker_position.value_or(0.0F);
                return num(kCx + kRing * std::cos(angle_of(p)));
            }));
    animate(f, "cy", times, values(run.samples, [](const Sample& s) {
                const double p = s.frame.marker_position.value_or(0.0F);
                return num(kCy + kRing * std::sin(angle_of(p)));
            }));
    animate(f, "opacity", times, values(run.samples, [](const Sample& s) {
                return std::string{s.frame.marker_position.has_value() ? "1" : "0"};
            }));
    std::fprintf(f, "</circle>\n");

    // Activity label at the centre, one element per activity.
    for (const Activity a : {Activity::Unmeasured, Activity::Thinking, Activity::Silent,
                             Activity::Acted, Activity::AwaitingApproval, Activity::Refused,
                             Activity::Failed}) {
        std::fprintf(f,
                     "<text x=\"%.1f\" y=\"%.1f\" fill=\"%s\" font-size=\"15\" font-weight=\"600\" "
                     "text-anchor=\"middle\" opacity=\"0\">",
                     kCx, kCy + 5.0, activity_colour(a));
        animate(f, "opacity", times, values(run.samples, [a](const Sample& s) {
                    return num(s.frame.activity == a
                                   ? static_cast<double>(s.frame.glyph_opacity) * (0.4 + 0.6 * static_cast<double>(s.frame.confidence))
                                   : 0.0);
                }));
        std::fprintf(f, "%s</text>\n", activity_name(a));
    }

    // Right-hand panel.
    std::fprintf(f, "<text x=\"580\" y=\"70\" fill=\"#f3f4f6\" font-size=\"22\" font-weight=\"600\">Cerebra</text>\n");
    std::fprintf(f, "<text x=\"580\" y=\"94\" fill=\"#9ca3af\" font-size=\"13\">what Archie is doing, drawn only when it changes</text>\n");

    std::vector<std::optional<PrivacyGrade>> grades{std::nullopt, PrivacyGrade::Public, PrivacyGrade::Internal,
                                                    PrivacyGrade::Sensitive, PrivacyGrade::Restricted, PrivacyGrade::Secret};
    for (const auto& g : grades) {
        std::fprintf(f, "<text x=\"580\" y=\"140\" fill=\"#e5e7eb\" font-size=\"14\" opacity=\"0\">");
        animate(f, "opacity", times, values(run.samples, [g](const Sample& s) {
                    return std::string{s.frame.privacy == g ? "1" : "0"};
                }));
        std::fprintf(f, "%s</text>\n", grade_name(g));
    }

    for (std::size_t c = 0; c < steps.size(); ++c) {
        std::fprintf(f, "<text x=\"580\" y=\"190\" fill=\"#d1d5db\" font-size=\"14\" opacity=\"0\">");
        animate(f, "opacity", times, values(run.samples, [c](const Sample& s) {
                    return std::string{s.caption == c ? "1" : "0"};
                }));
        std::fprintf(f, "%s</text>\n", steps[c].caption);
    }

    const int naive = static_cast<int>(kEndMs / 16);
    std::fprintf(f, "<text x=\"580\" y=\"270\" fill=\"#f3f4f6\" font-size=\"16\" font-weight=\"600\">Frames drawn over 15 s</text>\n");
    std::fprintf(f, "<text x=\"580\" y=\"300\" fill=\"#34d399\" font-size=\"14\">normal tier: %d</text>\n", run.frames);
    std::fprintf(f, "<text x=\"580\" y=\"324\" fill=\"#fbbf24\" font-size=\"14\">conserve tier: %d</text>\n", conserve.frames);
    std::fprintf(f, "<text x=\"580\" y=\"348\" fill=\"#cbd5e1\" font-size=\"14\">reduced motion: %d</text>\n", reduced.frames);
    std::fprintf(f, "<text x=\"580\" y=\"372\" fill=\"#f87171\" font-size=\"14\">fixed 60 fps loop: %d</text>\n", naive);
    std::fprintf(f, "<text x=\"580\" y=\"410\" fill=\"#6b7280\" font-size=\"12\">Each animation step below is a frame the engine asked for.</text>\n");
    std::fprintf(f, "<text x=\"580\" y=\"428\" fill=\"#6b7280\" font-size=\"12\">Between them the renderer is asleep.</text>\n");

    // Playhead.
    std::fprintf(f, "<rect x=\"40\" y=\"560\" width=\"900\" height=\"4\" fill=\"#1f2937\"/>\n");
    std::fprintf(f, "<rect x=\"40\" y=\"560\" width=\"0\" height=\"4\" fill=\"#60a5fa\">"
                    "<animate attributeName=\"width\" dur=\"%.3fs\" repeatCount=\"indefinite\" values=\"0;900\"/></rect>\n",
                 static_cast<double>(kEndMs) / 1000.0);
    // Tick marks where the engine drew.
    for (const Sample& s : run.samples) {
        const double x = 40.0 + 900.0 * static_cast<double>(s.at_ms) / static_cast<double>(kEndMs);
        std::fprintf(f, "<rect x=\"%.2f\" y=\"572\" width=\"1\" height=\"10\" fill=\"%s\"/>\n", x,
                     s.frame.animate ? "#60a5fa" : "#f9fafb");
    }
    std::fprintf(f, "<text x=\"40\" y=\"552\" fill=\"#6b7280\" font-size=\"11\">frames requested (blue: in motion, white: a state or freshness change drawn once)</text>\n");
    std::fprintf(f, "</svg>\n");
}

}  // namespace

int main(int argc, char** argv) {
    const char* out_path = argc > 1 ? argv[1] : nullptr;
    RenderTier tier = RenderTier::Normal;
    CognitionMotionPolicy policy;
    for (int i = 2; i < argc; ++i) {
        if (std::strcmp(argv[i], "conserve") == 0) tier = RenderTier::Conserve;
        if (std::strcmp(argv[i], "ultra") == 0) tier = RenderTier::Ultra;
        if (std::strcmp(argv[i], "--reduced-motion") == 0) policy.reduced_motion = true;
    }

    const std::vector<Step> steps = scenario();
    const Run run = simulate(steps, policy, tier);
    const Run conserve = simulate(steps, policy, RenderTier::Conserve);
    CognitionMotionPolicy reduced_policy = policy;
    reduced_policy.reduced_motion = true;
    const Run reduced = simulate(steps, reduced_policy, tier);

    std::FILE* f = out_path != nullptr ? std::fopen(out_path, "wb") : stdout;
    if (f == nullptr) {
        std::fprintf(stderr, "cannot open %s\n", out_path);
        return 1;
    }
    write_svg(f, run, steps, conserve, reduced);
    if (f != stdout) {
        std::fclose(f);
    }

    int in_motion = 0;
    for (const Sample& s : run.samples) {
        in_motion += s.frame.animate ? 1 : 0;
    }
    std::fprintf(stderr,
                 "scenario %lld ms: %d frames (%d in motion, %d state/boundary) | conserve %d | "
                 "reduced motion %d | fixed 60 fps would draw %lld\n",
                 static_cast<long long>(kEndMs), run.frames, in_motion, run.frames - in_motion,
                 conserve.frames, reduced.frames, static_cast<long long>(kEndMs / 16));
    return 0;
}
