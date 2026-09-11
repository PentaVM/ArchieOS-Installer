// Sentry temporal model — event time and knowledge time are different questions.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Temporal
//
// A world model that stores one timestamp per fact cannot answer the question that
// matters most when something goes wrong: *what did Archie know at time T?* An
// earthquake that happened at 03:00, was published at 03:14 and was fetched at 03:31 is
// one event with three different answers depending on which clock is being asked, and a
// replay that uses the wrong one silently rewrites history in Archie's favour — every
// detector looks prescient because it is being fed data it could not have had.
//
// So every temporal fact here carries both axes, and the type will not let you collapse
// them by accident.

#pragma once

#include <compare>
#include <cstdint>
#include <optional>
#include <string>

namespace sentry::world {

// Milliseconds since the Unix epoch, UTC.
//
// Signed because pre-1970 timestamps are real in historical corpora — a 1963 conflict
// record from UCDP is not corrupt data — and an unsigned type would wrap them into the
// far future rather than rejecting them.
using UnixMillis = std::int64_t;

// A point in time, distinct from a plain integer so it cannot be confused with a
// duration, an identifier, or a count.
class TimePoint {
public:
    constexpr TimePoint() noexcept = default;
    constexpr explicit TimePoint(UnixMillis millis) noexcept : millis_(millis) {}

    [[nodiscard]] constexpr UnixMillis millis() const noexcept { return millis_; }

    [[nodiscard]] constexpr auto operator<=>(const TimePoint&) const noexcept = default;

    // Milliseconds from this point to `later`. Negative when `later` precedes it, which
    // is a fact worth seeing rather than clamping: a provider whose timestamps run
    // backwards is broken, and hiding that behind a zero makes it undiagnosable.
    [[nodiscard]] constexpr std::int64_t to(TimePoint later) const noexcept {
        return later.millis_ - millis_;
    }

    // ISO-8601 in UTC, for logs and provenance records.
    [[nodiscard]] std::string iso8601() const;

private:
    UnixMillis millis_ = 0;
};

// Why a temporal extent was rejected.
enum class TemporalFault {
    // observed_at is after fetched_at: the provider claims we retrieved a record before
    // the thing it describes happened.
    ObservedAfterFetched,
    // valid_until precedes valid_from.
    ValidityInverted,
    // A timestamp lies outside the range any real observation can occupy.
    OutOfPlausibleRange,
};

[[nodiscard]] std::string_view describe(TemporalFault fault) noexcept;

// The two clocks every observation carries.
//
// **Event time** is when the world did something. **Knowledge time** is when Archie
// found out. They are independent, they routinely disagree by hours, and conflating
// them is the single most common way a replay harness lies to its owner.
struct TemporalExtent {
    // ---- event time: when the world did the thing -------------------------------
    //
    // When the observation is about. Absent for records that describe a state rather
    // than a moment — a provider catalogue entry, an orbital element set at its epoch.
    std::optional<TimePoint> observed_at{};

    // What the provider itself claimed, preserved verbatim even when it disagrees with
    // `observed_at` after normalization. A provider that timestamps in local time
    // without an offset is a fact about that provider worth keeping.
    std::optional<TimePoint> provider_timestamp{};

    // When the source made it public. For news this is the story's publication; for a
    // satellite product, when it landed in the catalogue.
    std::optional<TimePoint> published_at{};

    // ---- knowledge time: when Archie found out ----------------------------------
    //
    // When this node retrieved the bytes. Never optional: if we hold a record, we
    // fetched it, and a record that cannot say when it arrived cannot be replayed.
    TimePoint fetched_at{};

    // When the raw bytes became a typed observation. Distinct from `fetched_at` because
    // a normalizer can be re-run over preserved raw records years later, and the
    // difference is exactly what distinguishes "we knew this then" from "we could have
    // known this then".
    TimePoint normalized_at{};

    // ---- validity: the window in which the statement holds -----------------------
    std::optional<TimePoint> valid_from{};
    std::optional<TimePoint> valid_until{};

    // Whether this extent could describe a real observation.
    //
    // Deliberately returns the fault rather than a bool. "Invalid" is not actionable;
    // "this provider's timestamps precede our own fetch" tells an operator which
    // provider to go and look at.
    [[nodiscard]] std::optional<TemporalFault> fault() const noexcept;

    [[nodiscard]] bool plausible() const noexcept { return !fault().has_value(); }

    // Whether Archie could have known this at `t`.
    //
    // This is the whole reason knowledge time is stored separately, and it is the
    // predicate a replay harness must filter on. Filtering on `observed_at` instead
    // makes every detector look prescient, because it is handed observations that had
    // not yet been fetched when the decision was made.
    [[nodiscard]] bool known_by(TimePoint t) const noexcept { return fetched_at <= t; }

    // Whether the statement applies at `t`, independent of when we learned it.
    //
    // An absent bound is open, not zero: an orbital element set with no `valid_until`
    // has not expired, it simply has no declared end.
    [[nodiscard]] bool valid_at(TimePoint t) const noexcept {
        if (valid_from && t < *valid_from) {
            return false;
        }
        if (valid_until && t >= *valid_until) {
            return false;
        }
        return true;
    }

    // How stale this observation is at `t`, in milliseconds of *event* time.
    //
    // Measured from the most specific event-time field available, because that is what
    // "how old is this information" means to an operator. Returns nothing when the
    // record carries no event time at all — a provider catalogue entry is not stale, it
    // is simply not about a moment, and reporting zero would claim it is fresh.
    [[nodiscard]] std::optional<std::int64_t> staleness_at(TimePoint t) const noexcept;

    // The event-time anchor: the most specific one the record actually has.
    [[nodiscard]] std::optional<TimePoint> event_time() const noexcept;

    // Whether this record is impossibly old for a provider that promises fresh data.
    //
    // This exists because the global plausibility window cannot catch the most common
    // ingestion bug there is: seconds handed to a field expecting milliseconds. A 2026
    // timestamp divided by a thousand lands in January 1970, which is a perfectly
    // plausible date — and the window cannot be narrowed to exclude it, because UCDP
    // conflict records legitimately reach back to 1946 and orbital elements legitimately
    // reach forward.
    //
    // What distinguishes the bug from the history is the *provider*. An earthquake feed
    // that promises data minutes old has not just returned a record from 1970; a
    // historical archive has. So the check needs the provider's own freshness
    // expectation, and belongs here rather than in the global range test.
    //
    // `tolerance_multiple` is how far past its promise a provider may drift before this
    // stops being lateness and starts being a decoding error.
    [[nodiscard]] bool implausibly_stale_for(TimePoint now,
                                             std::int64_t expected_freshness_ms,
                                             int tolerance_multiple = 100) const noexcept;
};

}  // namespace sentry::world
