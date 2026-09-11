// Sentry epistemic model — how sure we are, and who made us that sure.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Epistemic
//
// The temptation in a fusion system is to attach a confidence float to everything and
// call the epistemology solved. It is not solved: 0.87 does not say whether four
// independent sensors agreed, whether one wire service was copied four times, or
// whether a language model found the story plausible. Those are different situations
// and they demand different actions.
//
// So state is a state, transitions are explicit, and the actor causing each transition
// is recorded — because the single most important rule in this system is about *who* is
// allowed to move a fact, not how confident anyone is:
//
//     NO GENERATIVE OUTPUT MAY DIRECTLY BECOME VERIFIED WORLD STATE.
//
// That rule is enforced here, mechanically, rather than being written in a design
// document and hoped for.

#pragma once

#include <optional>
#include <string>
#include <string_view>

#include "sentry/world/time.hpp"

namespace sentry::world {

// What we currently believe about a statement, and how it got there.
enum class EpistemicState {
    // Bytes as the provider sent them. No interpretation has been applied.
    Raw,
    // Structurally decoded. Says nothing about whether the content makes sense.
    Parsed,
    // Mapped onto Sentry's own types, units and coordinate frames.
    Normalized,
    // At least two *independent* sources agree. Independence is a property of the
    // provenance chain, not of the number of documents: four papers reprinting one wire
    // story are one source.
    Corroborated,
    // Derived by a deterministic or specialist-model analytic from other observations.
    // Real, useful, and explicitly not the same as observed.
    Inferred,
    // Corroborated and survived contradiction checks. The strongest state Sentry
    // asserts on its own.
    Verified,
    // Sources disagree and the disagreement is unresolved. Not an error — a disputed
    // fact is a finding, and collapsing it to one side is how a fusion system starts
    // lying.
    Disputed,
    // A newer statement replaced this one. Kept, because replay needs what we used to
    // believe.
    Superseded,
    // The source withdrew it.
    Retracted,
    // Was believed, but is now older than its provider's freshness expectation. Stale
    // data is shown as stale, never hidden and never presented as current.
    Stale,
    // A generative model proposed this. It is a research lead. It is never world state,
    // and no amount of model confidence changes that.
    ArchieHypothesis,
};

[[nodiscard]] std::string_view name(EpistemicState state) noexcept;

// Whether this state may be rendered as something Sentry asserts about the world.
//
// `ArchieHypothesis` is deliberately excluded: it may be *shown*, but it must never be
// drawn in the same visual register as an observed fact.
[[nodiscard]] constexpr bool is_asserted(EpistemicState state) noexcept {
    return state == EpistemicState::Verified || state == EpistemicState::Corroborated ||
           state == EpistemicState::Inferred;
}

// Whether a statement in this state should still be acted on at all.
[[nodiscard]] constexpr bool is_actionable(EpistemicState state) noexcept {
    return is_asserted(state) || state == EpistemicState::Normalized ||
           state == EpistemicState::Disputed;
}

// Who or what is asking for a state change.
//
// This is not logging. It is the input to the rule that keeps generated text out of the
// world model, so it has to travel with every transition rather than being reconstructed
// afterwards from a log line.
enum class Actor {
    // The ingest path, writing raw provider bytes.
    Ingest,
    // A normalizer mapping raw records onto Sentry types.
    Normalizer,
    // A versioned deterministic or statistical detector.
    Detector,
    // The fusion engine, comparing independent sources.
    Fusion,
    // A specialist (non-generative) model: a detector network, a tracker, a classifier.
    SpecialistModel,
    // A human with authority over this node.
    Operator,
    // A large language model or other generative system.
    GenerativeModel,
};

[[nodiscard]] std::string_view name(Actor actor) noexcept;

// Whether this actor produces evidence, or merely produces text.
//
// The distinction is the whole architecture. A specialist model that outputs a bounding
// box with a calibrated score has made a measurement. A language model that outputs a
// paragraph has made a suggestion.
[[nodiscard]] constexpr bool produces_evidence(Actor actor) noexcept {
    return actor != Actor::GenerativeModel;
}

// Why a proposed transition was refused.
enum class TransitionRefusal {
    // The constitutional rule: a generative model tried to move a statement into a state
    // Sentry asserts about the world.
    GenerativeCannotAssert,
    // Nothing may leave a terminal state except by explicit operator action.
    TerminalState,
    // The target state does not follow from the current one.
    NotReachable,
    // Corroboration requires independent sources, and this transition did not carry
    // enough of them.
    InsufficientIndependence,
};

[[nodiscard]] std::string_view describe(TransitionRefusal refusal) noexcept;

// A proposed change of belief.
struct Transition {
    EpistemicState from{};
    EpistemicState to{};
    Actor actor{};
    // How many *independent* sources back the target state. Independence is computed by
    // walking provenance to its roots, not by counting documents.
    int independent_sources = 0;
    TimePoint at{};
    // Which detector/model version made this call, so a regression can be traced to it.
    std::string agent_version{};
};

// Whether a transition is permitted, and if not, why.
//
// Returning the refusal rather than a bool is deliberate: an operator seeing
// "transition refused" learns nothing, and one seeing "a generative model tried to mark
// this verified" learns exactly what happened.
[[nodiscard]] std::optional<TransitionRefusal> refuse(const Transition& t) noexcept;

[[nodiscard]] inline bool permitted(const Transition& t) noexcept {
    return !refuse(t).has_value();
}

// The number of independent sources below which nothing may be called corroborated.
//
// Two, because one source agreeing with itself is not agreement. This is a floor, not a
// sufficient condition — contradiction checks still apply above it.
inline constexpr int kMinimumIndependentSources = 2;

}  // namespace sentry::world
