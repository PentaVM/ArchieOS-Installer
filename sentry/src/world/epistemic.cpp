#include "sentry/world/epistemic.hpp"

namespace sentry::world {

std::string_view name(EpistemicState state) noexcept {
    switch (state) {
        case EpistemicState::Raw:              return "raw";
        case EpistemicState::Parsed:           return "parsed";
        case EpistemicState::Normalized:       return "normalized";
        case EpistemicState::Corroborated:     return "corroborated";
        case EpistemicState::Inferred:         return "inferred";
        case EpistemicState::Verified:         return "verified";
        case EpistemicState::Disputed:         return "disputed";
        case EpistemicState::Superseded:       return "superseded";
        case EpistemicState::Retracted:        return "retracted";
        case EpistemicState::Stale:            return "stale";
        case EpistemicState::ArchieHypothesis: return "archie_hypothesis";
    }
    return "unknown";
}

std::string_view name(Actor actor) noexcept {
    switch (actor) {
        case Actor::Ingest:          return "ingest";
        case Actor::Normalizer:      return "normalizer";
        case Actor::Detector:        return "detector";
        case Actor::Fusion:          return "fusion";
        case Actor::SpecialistModel: return "specialist_model";
        case Actor::Operator:        return "operator";
        case Actor::GenerativeModel: return "generative_model";
    }
    return "unknown";
}

std::string_view describe(TransitionRefusal refusal) noexcept {
    switch (refusal) {
        case TransitionRefusal::GenerativeCannotAssert:
            return "a generative model may propose a hypothesis but may never move a "
                   "statement into world state Sentry asserts";
        case TransitionRefusal::TerminalState:
            return "the statement is retracted or superseded; only an operator may move "
                   "it again";
        case TransitionRefusal::NotReachable:
            return "the target state does not follow from the current one";
        case TransitionRefusal::InsufficientIndependence:
            return "corroboration needs at least two sources that are independent at the "
                   "root of their provenance, not two copies of one";
    }
    return "unknown refusal";
}

namespace {

constexpr bool is_terminal(EpistemicState state) noexcept {
    return state == EpistemicState::Retracted || state == EpistemicState::Superseded;
}

// Whether `to` can follow `from` at all, ignoring who is asking.
//
// The graph is deliberately narrow. Everything reachable from anywhere would make the
// state field decorative, and the point of it is that a jump from Raw straight to
// Verified is impossible to express, not merely discouraged.
constexpr bool reachable(EpistemicState from, EpistemicState to) noexcept {
    using S = EpistemicState;

    // Any live statement can be withdrawn, superseded, disputed or go stale. These are
    // things the world does to a fact, not steps in a ladder.
    if (to == S::Retracted || to == S::Superseded || to == S::Disputed || to == S::Stale) {
        return true;
    }
    // Staleness and dispute are recoverable: fresher data arrives, contradictions
    // resolve. A stale statement re-enters at Normalized and must earn its way up again
    // rather than resuming the standing it had before it aged out.
    if (from == S::Stale) {
        return to == S::Normalized;
    }
    if (from == S::Disputed) {
        return to == S::Normalized || to == S::Corroborated || to == S::Verified;
    }

    switch (from) {
        case S::Raw:
            return to == S::Parsed;
        case S::Parsed:
            return to == S::Normalized;
        case S::Normalized:
            // Inference and corroboration are different routes out of the same place:
            // one derives a new statement, the other finds agreement for this one.
            return to == S::Corroborated || to == S::Inferred;
        case S::Inferred:
            return to == S::Corroborated;
        case S::Corroborated:
            return to == S::Verified;
        case S::ArchieHypothesis:
            // A hypothesis is a research lead. It leaves this state only by being
            // withdrawn or superseded, both handled above. It cannot be promoted: the
            // way a hypothesis becomes world state is that someone goes and observes
            // the thing, producing a *new* statement with its own provenance.
            return false;
        case S::Verified:
            return false;
        case S::Retracted:
        case S::Superseded:
            return false;
        case S::Disputed:
        case S::Stale:
            // Both are answered by the early returns above, before this switch. Listed
            // so the enumeration stays exhaustive and a state added later cannot fall
            // silently through to `false`.
            return false;
    }
    return false;
}

}  // namespace

std::optional<TransitionRefusal> refuse(const Transition& t) noexcept {
    // The constitutional rule, checked before anything else so that no combination of
    // other conditions can route around it.
    //
    // A generative model may write exactly one state: ArchieHypothesis. It may not mark
    // something verified, corroborated or inferred; it may not retract a fact it
    // disagrees with; it may not resolve a dispute. Its output is a lead for the
    // deterministic machinery to chase, and the moment it can promote its own output the
    // evidence graph is worth nothing.
    if (t.actor == Actor::GenerativeModel && t.to != EpistemicState::ArchieHypothesis) {
        return TransitionRefusal::GenerativeCannotAssert;
    }

    // Retracted and superseded are terminal for automation. An operator can still act —
    // a source that retracts in error is a real situation — but no detector, fusion pass
    // or model may quietly resurrect a withdrawn fact.
    if (is_terminal(t.from) && t.actor != Actor::Operator) {
        return TransitionRefusal::TerminalState;
    }

    // `ArchieHypothesis` means precisely "a generative model suggested this", so the
    // actor decides reachability rather than the source state. A model may hypothesise
    // about anything still live; nothing else may put a statement there, because a
    // detector demoting real evidence to a guess would be a loss of standing disguised
    // as a transition.
    if (t.to == EpistemicState::ArchieHypothesis) {
        return t.actor == Actor::GenerativeModel
                   ? std::nullopt
                   : std::optional{TransitionRefusal::NotReachable};
    }

    // An operator may place a statement anywhere else.
    if (t.actor == Actor::Operator) {
        return std::nullopt;
    }

    if (!reachable(t.from, t.to)) {
        return TransitionRefusal::NotReachable;
    }

    // Corroboration and verification are claims about agreement between sources, so they
    // carry an evidential floor. Independence is counted at the root of the provenance
    // chain: four outlets running one wire story are one source, and this is the check
    // that stops them being counted as four.
    if (t.to == EpistemicState::Corroborated || t.to == EpistemicState::Verified) {
        if (t.independent_sources < kMinimumIndependentSources) {
            return TransitionRefusal::InsufficientIndependence;
        }
    }

    return std::nullopt;
}

}  // namespace sentry::world
