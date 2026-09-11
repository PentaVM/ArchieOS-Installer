#include "sentry/world/time.hpp"

#include <algorithm>
#include <array>
#include <chrono>
#include <format>

namespace sentry::world {
namespace {

// The window a real observation can occupy. Wide enough for historical conflict corpora
// (UCDP reaches back to 1946) and for orbital elements with far-future validity, narrow
// enough that a provider emitting nanoseconds-as-milliseconds, or a zeroed struct, is
// caught rather than plotted somewhere absurd on the timeline.
constexpr UnixMillis kEarliestPlausible = -3'155'760'000'000;  // 1870-01-01
constexpr UnixMillis kLatestPlausible = 4'102'444'800'000;     // 2100-01-01

constexpr bool in_plausible_range(TimePoint t) noexcept {
    return t.millis() >= kEarliestPlausible && t.millis() <= kLatestPlausible;
}

}  // namespace

std::string TimePoint::iso8601() const {
    const std::chrono::sys_time<std::chrono::milliseconds> tp{
        std::chrono::milliseconds{millis_}};
    return std::format("{:%FT%T}Z", tp);
}

std::string_view describe(TemporalFault fault) noexcept {
    switch (fault) {
        case TemporalFault::ObservedAfterFetched:
            return "observation is timestamped after we fetched it; the provider's clock "
                   "or timezone handling is wrong";
        case TemporalFault::ValidityInverted:
            return "valid_until precedes valid_from";
        case TemporalFault::OutOfPlausibleRange:
            return "a timestamp falls outside 1870-2100 and is almost certainly a unit or "
                   "encoding error";
    }
    return "unknown temporal fault";
}

std::optional<TemporalFault> TemporalExtent::fault() const noexcept {
    const std::array<std::optional<TimePoint>, 6> all{observed_at,  provider_timestamp,
                                                      published_at, valid_from,
                                                      valid_until,  fetched_at};
    for (const auto& t : all) {
        if (t && !in_plausible_range(*t)) {
            return TemporalFault::OutOfPlausibleRange;
        }
    }
    if (!in_plausible_range(normalized_at)) {
        return TemporalFault::OutOfPlausibleRange;
    }

    // An observation timestamped after we fetched it means the provider's clock is
    // ahead, its timezone handling is broken, or the field was misread. Any of those
    // makes every downstream time-alignment wrong, so it is caught at the boundary
    // rather than propagated. Compared against fetch rather than normalization, since
    // a normalizer may legitimately be re-run long afterwards.
    if (observed_at && *observed_at > fetched_at) {
        return TemporalFault::ObservedAfterFetched;
    }

    if (valid_from && valid_until && *valid_until < *valid_from) {
        return TemporalFault::ValidityInverted;
    }
    return std::nullopt;
}

std::optional<TimePoint> TemporalExtent::event_time() const noexcept {
    // Most specific first. `published_at` is the weakest of the three — a story about
    // yesterday published today is about yesterday — so it is the last resort.
    if (observed_at) {
        return observed_at;
    }
    if (provider_timestamp) {
        return provider_timestamp;
    }
    return published_at;
}

bool TemporalExtent::implausibly_stale_for(TimePoint now,
                                           std::int64_t expected_freshness_ms,
                                           int tolerance_multiple) const noexcept {
    // A provider with no freshness promise — a historical archive — cannot produce an
    // implausibly old record by definition. Old is what it is for.
    if (expected_freshness_ms <= 0 || tolerance_multiple <= 0) {
        return false;
    }
    const auto age = staleness_at(now);
    if (!age) {
        return false;
    }
    return *age > expected_freshness_ms * tolerance_multiple;
}

std::optional<std::int64_t> TemporalExtent::staleness_at(TimePoint t) const noexcept {
    const auto anchor = event_time();
    if (!anchor) {
        return std::nullopt;
    }
    return anchor->to(t);
}

}  // namespace sentry::world
