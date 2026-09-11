#pragma once

// How old a projection is, and what the observatory is therefore allowed to imply
// about it.
//
// Every panel in the observatory is a claim about a machine that is somewhere else —
// another daemon, another node, a provider on the far side of the internet. The claim
// was true when it was made. Whether it is true *now* is a separate question, and it is
// the question a display is most likely to get wrong, because a stale value looks
// exactly like a fresh one until someone acts on it.
//
// So freshness is not a badge in a corner. It is a first-class visual channel: the
// renderer desaturates, dims and eventually stops animating data it can no longer
// vouch for. An operator should be able to tell, without reading a number, that the
// globe in front of them stopped being current four minutes ago.
//
// This header is pure arithmetic over milliseconds. It has no clock of its own on
// purpose — a component that reads the wall clock cannot be tested against a rollback,
// and clock rollback is precisely one of the cases below.

#include <cstdint>

namespace archie::visual {

/// What the display is entitled to imply about a value of a given age.
///
/// Ordered from most to least trustworthy so comparisons read naturally and the
/// governor in `budget.hpp` can take a minimum without a lookup table.
enum class Freshness {
    /// Current. Animate it, act on it, draw it at full strength.
    Live,
    /// Older than the stream's own cadence but still within a sane window. Shown at
    /// full detail with a visible age, because the operator can still use it.
    Lagging,
    /// Too old to be treated as the present. Desaturated, animation stopped: motion on
    /// stale data is a lie, because motion is what a viewer reads as "happening now".
    Stale,
    /// So old that displaying it as data would mislead. The panel keeps its shape and
    /// says what it last knew and when, rather than going blank — a blank panel is
    /// indistinguishable from a healthy panel with nothing to report.
    Expired,
};

/// Age thresholds for one stream, in milliseconds.
///
/// These belong to the stream, not to the display: the world projection refreshes on a
/// provider's schedule measured in minutes, while kernel telemetry is a second or two
/// old at most. One global threshold would either cry wolf about the globe or stay
/// silent about a frozen kernel reader.
struct FreshnessPolicy {
    std::int64_t lagging_after_ms = 2'000;
    std::int64_t stale_after_ms = 15'000;
    std::int64_t expired_after_ms = 120'000;

    /// How far a source's clock may run ahead of ours before its timestamps stop being
    /// usable. Two daemons on one machine agree closely; a peer across a network with
    /// its own NTP drift does not, and a second or two of skew is ordinary rather than
    /// alarming.
    std::int64_t future_tolerance_ms = 2'000;
};

/// The verdict on one observation's age.
struct FreshnessReading {
    Freshness tier = Freshness::Live;

    /// The timestamp is far enough in the future that it cannot be believed.
    ///
    /// This is deliberately not folded into `tier`. The two facts are different: a
    /// stale reading is old, an implausible one is *wrong*, and an operator chasing a
    /// misconfigured clock needs to be told which they are looking at. The tier is
    /// forced to `Stale` in this case — never `Live` — because a value whose age cannot
    /// be established must not be presented as the present moment.
    bool implausible_future = false;
};

/// Classify an age. Negative ages mean the source's clock is ahead of ours.
///
/// The future case is the one worth stating plainly. Naive arithmetic makes a
/// timestamp from ten minutes in the future look *fresher* than one from now, so a node
/// with a broken clock would win every "most recent" comparison in the display and its
/// data would be animated with full confidence forever. Clamping is not enough either,
/// because clamping silently converts a wrong clock into a healthy-looking panel. So a
/// small lead is tolerated as ordinary skew, and a large one is reported as what it is.
[[nodiscard]] constexpr FreshnessReading classify_age(std::int64_t age_ms,
                                                      const FreshnessPolicy& policy) noexcept {
    FreshnessReading reading{};

    if (age_ms < 0) {
        if (-age_ms <= policy.future_tolerance_ms) {
            // Ordinary skew between two clocks. Treat it as the present.
            reading.tier = Freshness::Live;
            return reading;
        }
        reading.implausible_future = true;
        reading.tier = Freshness::Stale;
        return reading;
    }

    if (age_ms >= policy.expired_after_ms) {
        reading.tier = Freshness::Expired;
    } else if (age_ms >= policy.stale_after_ms) {
        reading.tier = Freshness::Stale;
    } else if (age_ms >= policy.lagging_after_ms) {
        reading.tier = Freshness::Lagging;
    } else {
        reading.tier = Freshness::Live;
    }
    return reading;
}

/// How strongly a value of this freshness may be drawn, as a multiplier in [0, 1].
///
/// Used for opacity and colour saturation. `Expired` is not zero: the panel must remain
/// visible and legible, because an operator needs to see that a source has gone dark.
/// It is the *data* that fades, not the frame around it.
[[nodiscard]] constexpr float confidence_of(Freshness tier) noexcept {
    switch (tier) {
        case Freshness::Live:
            return 1.0F;
        case Freshness::Lagging:
            return 0.82F;
        case Freshness::Stale:
            return 0.45F;
        case Freshness::Expired:
            return 0.22F;
    }
    return 0.22F;
}

/// Whether data of this freshness may be animated.
///
/// Motion is the strongest "this is happening now" signal a screen has. Continuing to
/// pulse a node whose source stopped reporting four minutes ago is the single most
/// misleading thing this observatory could do, so the answer is a rule rather than a
/// per-component decision.
[[nodiscard]] constexpr bool may_animate(Freshness tier) noexcept {
    return tier == Freshness::Live || tier == Freshness::Lagging;
}

/// The less trustworthy of two readings.
///
/// A composite panel is only as current as its oldest input: a card showing live
/// pressure next to a four-minute-old thermal reading is a four-minute-old card.
[[nodiscard]] constexpr Freshness worse_of(Freshness a, Freshness b) noexcept {
    return (static_cast<int>(a) >= static_cast<int>(b)) ? a : b;
}

}  // namespace archie::visual
