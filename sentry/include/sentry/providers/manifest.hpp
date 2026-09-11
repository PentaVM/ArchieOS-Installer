// Providers — what a source promises, what it is allowed to be used for, and what it is
// currently doing.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Providers
//
// Three things get conflated whenever a system talks to more than a handful of feeds,
// and each conflation causes a different kind of damage:
//
//   the CONTRACT   what the provider promises: freshness, coverage, rate limits
//   the LICENCE    what we are permitted to do with the bytes once we have them
//   the HEALTH     what it is doing right now
//
// A provider that is rate-limited is not a provider that is broken, and neither is a
// provider whose credentials we never had. Collapsing all three into "down" produces a
// dashboard where every problem looks the same and none of them can be acted on.
//
// The licence is the one most easily forgotten and the most expensive to get wrong: some
// of the best open sources here are non-commercial, or forbid redistribution, or require
// attribution, and "we already downloaded it" is not a defence.

#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "sentry/world/time.hpp"

namespace sentry::providers {

using sentry::world::TimePoint;

// What kind of world data this provider carries. Used to route an observation to the
// right normalizer and to scope a re-probe, not for display.
enum class Category {
    Seismic,
    Fire,
    Weather,
    Orbital,
    EarthObservation,
    Aviation,
    Maritime,
    Cyber,
    Markets,
    News,
    Conflict,
    InternetHealth,
    Camera,
};

[[nodiscard]] std::string_view name(Category category) noexcept;

// What it costs to reach this provider.
enum class AccessClass {
    // No account, no key.
    Open,
    // Free, but needs a registered key.
    FreeKeyed,
    // Paid.
    Commercial,
    // Hardware or endpoints the operator owns.
    OperatorOwned,
};

// What we may do with the data.
//
// Deliberately not a free-text field. A string is unauditable, and the whole point is
// that a query can ask "which of my sources forbid redistribution" and get a real answer.
struct Licence {
    // Whether the data may be used in commercial work.
    bool commercial_use_allowed = false;
    // Whether attribution must be displayed wherever the data appears.
    bool attribution_required = true;
    // Attribution text, when required. Checked against emptiness at load: a provider
    // that requires attribution and supplies none cannot be rendered lawfully.
    std::string attribution{};
    // Whether raw provider records may be persisted at all. Some terms permit derived
    // products only, which changes what the evidence store may keep.
    bool raw_retention_allowed = true;
    // Whether the data may be passed on to another party — including to a fabric peer.
    bool redistribution_allowed = false;
    // Whether responses may be cached beyond the immediate request.
    bool caching_allowed = true;
};

// How a provider authenticates.
enum class AuthMode {
    None,
    ApiKey,
    OAuth2,
    BasicAuth,
    ClientCertificate,
};

// What a provider declares about itself. Loaded from a manifest file, never inferred
// from a successful response.
struct Manifest {
    std::string id{};
    Category category{};
    // The manifest's own version, so a change in what a provider promises is itself a
    // dated, reviewable event rather than a silent edit.
    std::string version{};
    AccessClass access = AccessClass::Open;
    AuthMode auth = AuthMode::None;
    Licence licence{};

    // How fresh the data is expected to be, in milliseconds. Zero means the provider
    // makes no freshness promise — a historical archive — and is not merely a provider
    // whose promise nobody filled in. That distinction drives the staleness rules, so it
    // is deliberately explicit rather than defaulted.
    std::int64_t freshness_expectation_ms = 0;

    // The provider's own rate limit, if it publishes one.
    std::optional<int> requests_per_minute{};

    // Capability flags. Absent capabilities are not failures — a provider without
    // streaming is polled, and one without replay simply cannot be backtested.
    bool supports_streaming = false;
    bool supports_bulk = false;
    bool supports_delta = false;
    bool supports_replay = false;

    // Where the provider has data. Empty means global.
    std::vector<std::string> geographic_coverage{};
    // How far back it reaches, in days. Zero means live-only.
    int historical_depth_days = 0;

    // Whether this manifest is internally coherent. Returns the problems rather than a
    // bool, because a manifest with three faults should report three.
    [[nodiscard]] std::vector<std::string> problems() const;

    [[nodiscard]] bool valid() const { return problems().empty(); }
};

// What the provider is doing right now. Entirely separate from the manifest, and from
// the licence.
enum class Health {
    // Answering, fresh, within its limits.
    Available,
    // We have no key for it. Not a failure — a provider we were never configured to
    // reach has not broken, and reporting it as failed sends an operator hunting for an
    // outage that does not exist.
    NoCredentials,
    // We are being throttled. The data is fine; we are asking too often.
    RateLimited,
    // Answering, but with data older than it promised.
    Stale,
    // Answering partially, or intermittently.
    Degraded,
    // Not answering.
    Failed,
    // The operator switched it off.
    DisabledByOperator,
    // Its terms forbid what we would be using it for. A deliberate refusal on our side,
    // and never something to retry.
    TermsBlocked,
};

[[nodiscard]] std::string_view name(Health health) noexcept;

// Whether this health state is worth retrying on a timer.
//
// Three of these are not: a missing key, an operator's decision and a terms conflict all
// need a human, and retrying them produces a permanent stream of failures that buries
// the real outages.
[[nodiscard]] constexpr bool warrants_retry(Health health) noexcept {
    return health != Health::NoCredentials && health != Health::DisabledByOperator &&
           health != Health::TermsBlocked;
}

// Whether Sentry may currently ask this provider for anything.
[[nodiscard]] constexpr bool may_query(Health health) noexcept {
    return health == Health::Available || health == Health::Degraded ||
           health == Health::Stale;
}

// Live state for one provider.
struct Status {
    std::string provider_id{};
    Health health = Health::NoCredentials;
    // When we last got a usable response. Absent if we never have.
    std::optional<TimePoint> last_success{};
    std::optional<TimePoint> last_attempt{};
    // Consecutive failures, for backoff.
    int consecutive_failures = 0;
    // The provider's own message, when it gave one. Kept verbatim: a 429's Retry-After
    // or a 403's explanation is the most useful thing in the whole record.
    std::string detail{};

    // Whether the last success is older than the provider promised.
    //
    // A provider with no freshness promise is never stale — that is what an archive is.
    [[nodiscard]] bool is_stale_at(const Manifest& manifest, TimePoint now) const noexcept;

    // The health this status implies, given the manifest.
    //
    // Staleness is derived rather than stored, because it is a function of the clock: a
    // status written as Available five hours ago describes a provider that is now stale,
    // and storing the verdict would leave the display asserting freshness that expired
    // while nobody was looking.
    [[nodiscard]] Health effective_health(const Manifest& manifest,
                                          TimePoint now) const noexcept;
};

// How long to wait before retrying, given consecutive failures.
//
// Capped exponential. The cap matters more than the curve: an uncapped backoff on a
// provider that comes back after an hour means Sentry notices some time the next day.
[[nodiscard]] std::int64_t backoff_ms(int consecutive_failures,
                                      std::int64_t base_ms = 1'000,
                                      std::int64_t cap_ms = 300'000) noexcept;

// Why a proposed use of a provider's data was refused on licence grounds.
enum class LicenceRefusal {
    CommercialUseForbidden,
    RedistributionForbidden,
    RawRetentionForbidden,
    CachingForbidden,
    AttributionMissing,
};

[[nodiscard]] std::string_view describe(LicenceRefusal refusal) noexcept;

// What we want to do with a provider's bytes.
enum class Use {
    // Hold the raw record in the evidence store.
    RetainRaw,
    // Keep a response for longer than the request.
    Cache,
    // Pass it to a fabric peer.
    Redistribute,
    // Use it in commercial work.
    Commercial,
    // Draw it on the display.
    Display,
};

// Whether the licence permits this use.
[[nodiscard]] std::optional<LicenceRefusal> refuse(const Licence& licence,
                                                   Use use) noexcept;

}  // namespace sentry::providers
