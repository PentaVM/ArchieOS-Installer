// Contract, licence and health are three separate things.
//
// The failure this suite guards against is the one every multi-source system drifts
// into: a dashboard where a missing API key, a throttle and a dead endpoint all render
// as "down", and none of them can be acted on.

#include "harness.hpp"

#include "sentry/providers/manifest.hpp"

using namespace sentry::providers;
using sentry::world::TimePoint;

namespace {

constexpr sentry::world::UnixMillis kNow = 1'767'225'600'000;
constexpr TimePoint now{kNow};

Manifest usgs() {
    Manifest m;
    m.id = "usgs.earthquakes";
    m.category = Category::Seismic;
    m.version = "2026-01-01";
    m.access = AccessClass::Open;
    m.auth = AuthMode::None;
    m.licence.commercial_use_allowed = true;
    m.licence.attribution_required = true;
    m.licence.attribution = "U.S. Geological Survey";
    m.licence.raw_retention_allowed = true;
    m.licence.redistribution_allowed = true;
    m.licence.caching_allowed = true;
    m.freshness_expectation_ms = 5 * 60'000;
    m.supports_replay = true;
    m.historical_depth_days = 36'500;
    return m;
}

Manifest historical_archive() {
    Manifest m = usgs();
    m.id = "ucdp.ged";
    m.category = Category::Conflict;
    // No freshness promise at all: being old is what an archive is for.
    m.freshness_expectation_ms = 0;
    return m;
}

void health_states_that_need_a_human_are_never_retried() {
    CASE("a missing key is not an outage");

    CHECK_BECAUSE(!warrants_retry(Health::NoCredentials),
                  "a provider we were never configured to reach has not broken; retrying "
                  "produces a permanent stream of failures that buries the real outages");
    CHECK(!warrants_retry(Health::DisabledByOperator));
    CHECK_BECAUSE(!warrants_retry(Health::TermsBlocked),
                  "a terms conflict is our own deliberate refusal, not a transient fault");

    CHECK(warrants_retry(Health::Failed));
    CHECK(warrants_retry(Health::RateLimited));
    CHECK(warrants_retry(Health::Degraded));
    CHECK(warrants_retry(Health::Stale));
}

void a_throttled_provider_is_not_a_broken_one() {
    CASE("rate limited means we are asking too often, not that the data is bad");

    CHECK(may_query(Health::Available));
    CHECK(may_query(Health::Degraded));
    CHECK_BECAUSE(may_query(Health::Stale),
                  "stale data is still data; hiding it would be worse than labelling it");
    CHECK_BECAUSE(!may_query(Health::RateLimited),
                  "we back off rather than compounding the throttle");
    CHECK(!may_query(Health::NoCredentials));
    CHECK(!may_query(Health::TermsBlocked));
}

void staleness_is_recomputed_from_the_clock_not_stored() {
    CASE("a status written as available five hours ago describes a stale provider");

    const auto manifest = usgs();
    Status s;
    s.provider_id = manifest.id;
    s.health = Health::Available;
    s.last_success = TimePoint{kNow - 5 * 3'600'000};

    CHECK_BECAUSE(s.effective_health(manifest, now) == Health::Stale,
                  "storing the verdict would leave the display asserting freshness that "
                  "expired while nobody was looking");

    // Fresh again after a successful poll.
    s.last_success = TimePoint{kNow - 60'000};
    CHECK_EQ(s.effective_health(manifest, now), Health::Available);

    // A provider previously marked stale recovers on its own once fresh data arrives.
    s.health = Health::Stale;
    CHECK_EQ(s.effective_health(manifest, now), Health::Available);

    // Degraded is not cured by freshness: partial answers stay partial.
    s.health = Health::Degraded;
    CHECK_EQ(s.effective_health(manifest, now), Health::Degraded);
}

void an_archive_is_never_stale() {
    CASE("no freshness promise means staleness is not a concept");

    const auto archive = historical_archive();
    Status s;
    s.health = Health::Available;
    s.last_success = TimePoint{kNow - 400LL * 24 * 3'600'000};  // over a year ago

    CHECK_BECAUSE(!s.is_stale_at(archive, now),
                  "being old is the archive's purpose; a permanent staleness warning on a "
                  "healthy source trains an operator to ignore the warning");
    CHECK_EQ(s.effective_health(archive, now), Health::Available);
}

void a_provider_never_reached_is_not_reported_as_stale() {
    CASE("no successful fetch yet is not the same as old data");

    const auto manifest = usgs();
    Status s;
    s.health = Health::NoCredentials;
    CHECK(!s.last_success.has_value());
    CHECK(!s.is_stale_at(manifest, now));
    CHECK_BECAUSE(s.effective_health(manifest, now) == Health::NoCredentials,
                  "the missing key is the finding; staleness would describe a problem we "
                  "have not reached yet");
}

void decisions_are_not_overridden_by_the_clock() {
    CASE("an operator's switch does not expire");

    const auto manifest = usgs();
    for (const auto decided : {Health::DisabledByOperator, Health::TermsBlocked,
                               Health::NoCredentials, Health::Failed,
                               Health::RateLimited}) {
        Status s;
        s.health = decided;
        s.last_success = TimePoint{kNow - 10 * 3'600'000};
        CHECK_BECAUSE(s.effective_health(manifest, now) == decided,
                      "these describe a decision or a hard condition, and time passing "
                      "does not turn any of them into staleness");
    }
}

void backoff_is_capped_and_cannot_overflow() {
    CASE("an uncapped backoff means noticing a recovery some time tomorrow");

    CHECK_EQ(backoff_ms(0), std::int64_t{0});
    CHECK_EQ(backoff_ms(1), std::int64_t{1'000});
    CHECK_EQ(backoff_ms(2), std::int64_t{2'000});
    CHECK_EQ(backoff_ms(3), std::int64_t{4'000});

    // Doubling runs until it would exceed the cap: 2^8 * 1s is 256s, still under it.
    CHECK_EQ(backoff_ms(9), std::int64_t{256'000});
    CHECK_BECAUSE(backoff_ms(10) == 300'000,
                  "capped at five minutes; a provider that returns after an hour must be "
                  "noticed within five minutes of returning, not on the next doubling");
    CHECK_EQ(backoff_ms(11), std::int64_t{300'000});

    // A large failure count is what a long outage actually produces, and a shift-based
    // implementation would have overflowed into either instant retry or effectively
    // never long before here.
    CHECK_EQ(backoff_ms(1'000), std::int64_t{300'000});
    CHECK_EQ(backoff_ms(1'000'000), std::int64_t{300'000});
    CHECK(backoff_ms(50) > 0);
}

void the_licence_is_checked_per_use_not_once() {
    CASE("a provider can be free to display and forbidden to redistribute");

    Licence noncommercial;
    noncommercial.commercial_use_allowed = false;
    noncommercial.attribution_required = true;
    noncommercial.attribution = "Global Fishing Watch";
    noncommercial.raw_retention_allowed = true;
    noncommercial.redistribution_allowed = false;
    noncommercial.caching_allowed = true;

    CHECK(!refuse(noncommercial, Use::Display).has_value());
    CHECK(!refuse(noncommercial, Use::RetainRaw).has_value());
    CHECK(!refuse(noncommercial, Use::Cache).has_value());
    CHECK_BECAUSE(refuse(noncommercial, Use::Redistribute) ==
                      LicenceRefusal::RedistributionForbidden,
                  "passing it to a fabric peer is redistribution, and 'we already "
                  "downloaded it' is not a defence");
    CHECK(refuse(noncommercial, Use::Commercial) ==
          LicenceRefusal::CommercialUseForbidden);
}

void a_derived_products_only_licence_blocks_the_evidence_store() {
    CASE("some terms permit derived products only");

    Licence derived_only;
    derived_only.raw_retention_allowed = false;
    derived_only.attribution_required = false;
    CHECK_BECAUSE(refuse(derived_only, Use::RetainRaw) ==
                      LicenceRefusal::RawRetentionForbidden,
                  "the evidence store's default is to keep raw records; this is the check "
                  "that stops it doing so where terms forbid it");
    CHECK(!refuse(derived_only, Use::Display).has_value());
}

void a_provider_requiring_attribution_it_cannot_supply_is_not_displayable() {
    CASE("attribution required and none supplied is a manifest error, not a render bug");

    Licence broken;
    broken.attribution_required = true;
    broken.attribution = "";
    CHECK(refuse(broken, Use::Display) == LicenceRefusal::AttributionMissing);

    auto manifest = usgs();
    manifest.licence.attribution.clear();
    const auto problems = manifest.problems();
    CHECK(!problems.empty());
    CHECK(!manifest.valid());
}

void a_manifest_reports_every_fault_not_the_first() {
    CASE("three faults report as three");

    Manifest bad;
    bad.id.clear();
    bad.version.clear();
    bad.licence.attribution_required = true;
    bad.freshness_expectation_ms = -1;

    const auto problems = bad.problems();
    CHECK_BECAUSE(problems.size() >= 4,
                  "fixing one fault at a time across four round trips is how a config "
                  "session becomes an afternoon");
}

void claiming_replay_without_history_is_caught_before_an_incident_review() {
    CASE("replay needs history to replay");

    auto manifest = usgs();
    manifest.supports_replay = true;
    manifest.historical_depth_days = 0;
    CHECK_BECAUSE(!manifest.valid(),
                  "a provider that cannot back a 'what did Archie know at T' query must "
                  "say so at load, not during the incident review that needs it");
}

void an_authenticated_provider_cannot_also_be_open() {
    CASE("a config mistake that otherwise surfaces as a mysterious 401");

    auto manifest = usgs();
    manifest.auth = AuthMode::ApiKey;
    manifest.access = AccessClass::Open;
    CHECK(!manifest.valid());

    manifest.access = AccessClass::FreeKeyed;
    CHECK(manifest.valid());
}

void a_well_formed_manifest_has_nothing_to_say() {
    CASE("the good case is silent");

    CHECK(usgs().valid());
    CHECK(usgs().problems().empty());
    CHECK(historical_archive().valid());
}

}  // namespace

int main() {
    health_states_that_need_a_human_are_never_retried();
    a_throttled_provider_is_not_a_broken_one();
    staleness_is_recomputed_from_the_clock_not_stored();
    an_archive_is_never_stale();
    a_provider_never_reached_is_not_reported_as_stale();
    decisions_are_not_overridden_by_the_clock();
    backoff_is_capped_and_cannot_overflow();
    the_licence_is_checked_per_use_not_once();
    a_derived_products_only_licence_blocks_the_evidence_store();
    a_provider_requiring_attribution_it_cannot_supply_is_not_displayable();
    a_manifest_reports_every_fault_not_the_first();
    claiming_replay_without_history_is_caught_before_an_incident_review();
    an_authenticated_provider_cannot_also_be_open();
    a_well_formed_manifest_has_nothing_to_say();

    return sentry::test::report();
}
