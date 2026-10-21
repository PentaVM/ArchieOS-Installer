#pragma once

#include "archie/cortex/intelligence.hpp"

namespace archie::cortex {

// How IRIS should read a focus. Mirrors archie-accel's FocusMode.
enum class FocusMode { Off, Focused, Open, FocusedAndOpen };

enum class FocusSourceKind { Screen, Camera };

// A screen (a Device node) or a camera (a Sensor node) IRIS can watch.
struct FocusSource {
    std::string node_id{};
    FocusSourceKind kind = FocusSourceKind::Screen;
    std::vector<std::string> phrases{};
    // Whether the source shows text worth reading (a document, a chat).
    bool has_text = false;
    double max_fps = 30.0;
};

// Cortex's suggestion for one focus: its share of IRIS's compute, the mode and phrases.
struct FocusAdvice {
    std::string focus_id{};
    double share = 0.0;
    FocusMode mode = FocusMode::Off;
    std::vector<std::string> phrases{};
    bool read_text = false;
    double max_fps = 0.0;
};

// predict_process's view of a model's next job.
struct ModelAdvice {
    std::string subject_id{};
    std::optional<double> latency_ms{};
    std::optional<double> memory_bytes{};
};

// The payload of a CandidateSchedulerChange. Consultative only: archie-accel applies it
// inside the envelope Policy approved, and changing the envelope is a proposal Policy
// decides. Nothing here touches the GPU, plans work or grants anything.
struct ResourceAdvice {
    std::string advice_id{};
    std::string focus_state_id{};
    std::vector<FocusAdvice> foci{};
    std::vector<ModelAdvice> models{};
    std::int64_t advised_at_ms = 0;
};

inline constexpr std::size_t kMaxAdvisedFoci = 16;
inline constexpr std::size_t kMaxFocusPhrases = 16;
// Characters (code points), as archie-accel counts them.
inline constexpr std::size_t kMaxPhraseChars = 64;
inline constexpr double kMaxAdvisedFps = 240.0;

[[nodiscard]] std::optional<ContractRefusal> validate(const ResourceAdvice& advice) noexcept;

// Shares follow each source's activation in the focus state, decayed by its half-life at
// `as_of_ms` (pinned activations do not decay); a source with no activation is advised
// off. Model predictions come from `history` through predict_process.
[[nodiscard]] std::optional<ResourceAdvice> advise_resources(
    const FocusState& focus, const std::vector<FocusSource>& sources,
    const std::vector<TrainingEvent>& history, const std::vector<std::string>& model_subjects,
    std::int64_t as_of_ms);

// The proposal that carries the advice to Policy and archie-accel.
[[nodiscard]] AnalyticalProposal propose(const ResourceAdvice& advice);

// `focus.plan`'s payload: the advice per focus in the schema archie-accel accepts
// ({"advice":[{"focus","share","mode","phrases","read_text","priority","max_fps"}]}).
// The strongest focus is suggested `active_focus`, the rest `background`; the envelope
// still caps both.
[[nodiscard]] std::string focus_plan_json(const ResourceAdvice& advice);

// Capability descriptors for the models archie-accel runs and the LLM, with the telemetry
// fields their jobs report as TrainingEvents.
[[nodiscard]] std::vector<CapabilityDescriptor> model_capability_descriptors();

}  // namespace archie::cortex
