#include "sentry/providers/manifest.hpp"

#include <algorithm>

namespace sentry::providers {

std::string_view name(Category category) noexcept {
    switch (category) {
        case Category::Seismic:          return "seismic";
        case Category::Fire:             return "fire";
        case Category::Weather:          return "weather";
        case Category::Orbital:          return "orbital";
        case Category::EarthObservation: return "earth_observation";
        case Category::Aviation:         return "aviation";
        case Category::Maritime:         return "maritime";
        case Category::Cyber:            return "cyber";
        case Category::Markets:          return "markets";
        case Category::News:             return "news";
        case Category::Conflict:         return "conflict";
        case Category::InternetHealth:   return "internet_health";
        case Category::Camera:           return "camera";
    }
    return "unknown";
}

std::string_view name(Health health) noexcept {
    switch (health) {
        case Health::Available:          return "available";
        case Health::NoCredentials:      return "no_credentials";
        case Health::RateLimited:        return "rate_limited";
        case Health::Stale:              return "stale";
        case Health::Degraded:           return "degraded";
        case Health::Failed:             return "failed";
        case Health::DisabledByOperator: return "disabled_by_operator";
        case Health::TermsBlocked:       return "terms_blocked";
    }
    return "unknown";
}

std::string_view describe(LicenceRefusal refusal) noexcept {
    switch (refusal) {
        case LicenceRefusal::CommercialUseForbidden:
            return "this provider's terms do not permit commercial use";
        case LicenceRefusal::RedistributionForbidden:
            return "this provider's data may not be passed on, including to a fabric peer";
        case LicenceRefusal::RawRetentionForbidden:
            return "this provider permits derived products only; the raw record may not "
                   "be persisted";
        case LicenceRefusal::CachingForbidden:
            return "this provider's terms do not permit caching beyond the request";
        case LicenceRefusal::AttributionMissing:
            return "this provider requires attribution and the manifest supplies none, so "
                   "it cannot be displayed lawfully";
    }
    return "unknown refusal";
}

std::vector<std::string> Manifest::problems() const {
    std::vector<std::string> found;

    if (id.empty()) {
        found.emplace_back("provider id must not be empty");
    }
    if (version.empty()) {
        found.emplace_back(
            "manifest version must not be empty; a change in what a provider promises "
            "has to be a dated, reviewable event");
    }

    // A provider requiring attribution that supplies none cannot be drawn at all, so it
    // is a manifest error rather than a render-time surprise.
    if (licence.attribution_required && licence.attribution.empty()) {
        found.emplace_back("licence requires attribution but none is supplied");
    }

    // An authenticated provider that declares itself open is a configuration mistake
    // that only shows up as a mysterious 401 much later.
    if (auth != AuthMode::None && access == AccessClass::Open) {
        found.emplace_back(
            "provider declares open access but also an authentication mode");
    }

    if (freshness_expectation_ms < 0) {
        found.emplace_back("freshness expectation cannot be negative");
    }
    if (historical_depth_days < 0) {
        found.emplace_back("historical depth cannot be negative");
    }
    if (requests_per_minute && *requests_per_minute <= 0) {
        found.emplace_back("a declared rate limit must be positive");
    }

    // Replay needs history to replay. A provider claiming replay support with no
    // historical depth cannot back a "what did Archie know at T" query, and finding that
    // out during an incident review is too late.
    if (supports_replay && historical_depth_days == 0) {
        found.emplace_back(
            "provider claims replay support but declares no historical depth");
    }

    return found;
}

bool Status::is_stale_at(const Manifest& manifest, TimePoint now) const noexcept {
    // No promise, no staleness. A historical archive is not stale; being old is its
    // purpose, and marking it stale would put a permanent warning on a healthy source.
    if (manifest.freshness_expectation_ms <= 0) {
        return false;
    }
    if (!last_success) {
        return false;
    }
    return last_success->to(now) > manifest.freshness_expectation_ms;
}

Health Status::effective_health(const Manifest& manifest, TimePoint now) const noexcept {
    // States that describe a decision rather than a condition are returned untouched.
    // An operator's switch and a terms conflict are not made less true by the clock, and
    // a missing credential does not become staleness just because time passed.
    switch (health) {
        case Health::DisabledByOperator:
        case Health::TermsBlocked:
        case Health::NoCredentials:
        case Health::Failed:
        case Health::RateLimited:
            return health;
        case Health::Available:
        case Health::Degraded:
        case Health::Stale:
            break;
    }

    // Staleness is a function of the clock, so it is computed here rather than stored. A
    // status written as Available five hours ago describes a provider that is stale now,
    // and a stored verdict would have the display asserting freshness that expired while
    // nobody was looking.
    if (is_stale_at(manifest, now)) {
        return Health::Stale;
    }
    // A provider that has recovered stops being stale. Degraded is preserved, since
    // partial answers are not fixed by freshness.
    return health == Health::Stale ? Health::Available : health;
}

std::int64_t backoff_ms(int consecutive_failures,
                        std::int64_t base_ms,
                        std::int64_t cap_ms) noexcept {
    if (consecutive_failures <= 0) {
        return 0;
    }
    if (base_ms <= 0 || cap_ms <= 0) {
        return 0;
    }

    // Doubling, but computed by stepping rather than shifting: a large failure count
    // would overflow a shift long before it reached the cap, and an overflowed backoff
    // is either instant retry or effectively never.
    std::int64_t delay = base_ms;
    for (int i = 1; i < consecutive_failures; ++i) {
        if (delay >= cap_ms / 2) {
            return cap_ms;
        }
        delay *= 2;
    }
    return std::min(delay, cap_ms);
}

std::optional<LicenceRefusal> refuse(const Licence& licence, Use use) noexcept {
    switch (use) {
        case Use::RetainRaw:
            return licence.raw_retention_allowed
                       ? std::nullopt
                       : std::optional{LicenceRefusal::RawRetentionForbidden};
        case Use::Cache:
            return licence.caching_allowed
                       ? std::nullopt
                       : std::optional{LicenceRefusal::CachingForbidden};
        case Use::Redistribute:
            return licence.redistribution_allowed
                       ? std::nullopt
                       : std::optional{LicenceRefusal::RedistributionForbidden};
        case Use::Commercial:
            return licence.commercial_use_allowed
                       ? std::nullopt
                       : std::optional{LicenceRefusal::CommercialUseForbidden};
        case Use::Display:
            // Displaying is permitted by every licence here, provided the attribution it
            // demands actually exists to be shown.
            if (licence.attribution_required && licence.attribution.empty()) {
                return LicenceRefusal::AttributionMissing;
            }
            return std::nullopt;
    }
    return std::nullopt;
}

}  // namespace sentry::providers
