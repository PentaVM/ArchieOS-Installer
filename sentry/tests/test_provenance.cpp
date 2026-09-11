// Independence is counted at the root, not at the row.
//
// This suite exists because the corroboration floor in epistemic.hpp is only as good as
// the number handed to it. Until the graph existed, that number was a caller's promise.

#include "harness.hpp"

#include "sentry/world/epistemic.hpp"
#include "sentry/world/provenance.hpp"

using namespace sentry::world;

namespace {

constexpr UnixMillis kNow = 1'767'225'600'000;

Provenance fetched(std::string id, std::string provider) {
    Provenance p;
    p.record_id = std::move(id);
    p.provider_id = std::move(provider);
    p.derivation = Derivation::Fetched;
    p.time.fetched_at = TimePoint{kNow};
    p.time.normalized_at = TimePoint{kNow};
    return p;
}

Provenance syndicated(std::string id, std::string outlet, std::string credits) {
    Provenance p = fetched(std::move(id), std::move(outlet));
    p.derivation = Derivation::Syndicated;
    p.attributed_origin = std::move(credits);
    return p;
}

Provenance derived(std::string id, std::string agent, std::vector<RecordId> parents) {
    Provenance p;
    p.record_id = std::move(id);
    p.derivation = Derivation::Derived;
    p.parents = std::move(parents);
    p.agent_version = std::move(agent);
    p.time.fetched_at = TimePoint{kNow};
    p.time.normalized_at = TimePoint{kNow};
    return p;
}

void a_wire_story_carried_by_four_outlets_is_one_source() {
    CASE("syndication collapses to the wire service that wrote it");

    ProvenanceGraph g;
    g.add(syndicated("story-a", "outlet-a", "reuters"));
    g.add(syndicated("story-b", "outlet-b", "reuters"));
    g.add(syndicated("story-c", "outlet-c", "reuters"));
    g.add(syndicated("story-d", "outlet-d", "reuters"));

    const std::vector<RecordId> all{"story-a", "story-b", "story-c", "story-d"};
    CHECK_EQ(g.size(), std::size_t{4});
    CHECK_BECAUSE(g.independent_source_count(all) == 1,
                  "four records, one source; counting rows would have found four and "
                  "cleared the corroboration floor on a single story");
    CHECK_EQ(g.independent_sources(all).front(), std::string{"reuters"});

    // And the floor now refuses what it would have accepted.
    const Transition t{
        .from = EpistemicState::Normalized,
        .to = EpistemicState::Corroborated,
        .actor = Actor::Fusion,
        .independent_sources = g.independent_source_count(all),
        .at = TimePoint{kNow},
        .agent_version = "fusion-v1",
    };
    CHECK_BECAUSE(refuse(t) == TransitionRefusal::InsufficientIndependence,
                  "this is the whole reason the graph exists");
}

void genuinely_different_providers_count_separately() {
    CASE("independent observations of the same thing do corroborate");

    ProvenanceGraph g;
    g.add(fetched("quake-usgs", "usgs"));
    g.add(fetched("quake-emsc", "emsc"));

    const std::vector<RecordId> both{"quake-usgs", "quake-emsc"};
    CHECK_EQ(g.independent_source_count(both), 2);

    const Transition t{
        .from = EpistemicState::Normalized,
        .to = EpistemicState::Corroborated,
        .actor = Actor::Fusion,
        .independent_sources = g.independent_source_count(both),
        .at = TimePoint{kNow},
        .agent_version = "fusion-v1",
    };
    CHECK(permitted(t));
}

void an_analysis_and_its_input_are_one_source() {
    CASE("deriving from a record does not create a second witness");

    ProvenanceGraph g;
    g.add(fetched("s2-product", "copernicus"));
    g.add(derived("change-detection", "change-v2.4", {"s2-product"}));

    CHECK_BECAUSE(g.independent_source_count("change-detection") == 1,
                  "running a detector over an image does not make the image agree with "
                  "itself");

    // Counting the product and the analysis together is still one source — the shape
    // fusion actually asks, and the one where naive counting looks most convincing.
    const std::vector<RecordId> both{"s2-product", "change-detection"};
    CHECK_EQ(g.independent_source_count(both), 1);
}

void a_shared_ancestor_is_counted_once_across_a_cluster() {
    CASE("two derivations of one observation are not two sources");

    ProvenanceGraph g;
    g.add(fetched("adsb-raw", "adsb.lol"));
    g.add(derived("track-a", "tracker-v1", {"adsb-raw"}));
    g.add(derived("track-b", "tracker-v2", {"adsb-raw"}));
    g.add(fetched("sar-raw", "copernicus"));
    g.add(derived("vessel", "sar-v1", {"sar-raw"}));

    const std::vector<RecordId> cluster{"track-a", "track-b", "vessel"};
    const auto sources = g.independent_sources(cluster);
    CHECK_BECAUSE(sources.size() == 2,
                  "three derived records, two real observations behind them");
    CHECK_EQ(sources[0], std::string{"adsb.lol"});
    CHECK_EQ(sources[1], std::string{"copernicus"});
}

void an_uncredited_copy_still_counts_as_itself() {
    CASE("an uncredited republication might be original reporting");

    ProvenanceGraph g;
    g.add(syndicated("uncredited", "outlet-x", ""));
    CHECK_BECAUSE(g.independent_source_count("uncredited") == 1,
                  "refusing to count it would discard evidence that may be genuine; "
                  "counting it as the outlet is the honest reading of what we know");
    CHECK_EQ(g.independent_sources("uncredited").front(), std::string{"outlet-x"});
}

void a_relaying_root_resolves_to_what_it_relays() {
    CASE("an aggregator is not a source");

    ProvenanceGraph g;
    // A root fetch that names the upstream it is relaying — an aggregator mirroring
    // someone else's dataset.
    auto mirror = fetched("mirror-1", "some-aggregator");
    mirror.attributed_origin = "noaa";
    g.add(mirror);
    g.add(fetched("direct", "noaa"));

    const std::vector<RecordId> both{"mirror-1", "direct"};
    CHECK_BECAUSE(g.independent_source_count(both) == 1,
                  "fetching NOAA's data from a mirror and from NOAA is one source read "
                  "twice");
}

void a_cycle_in_provider_metadata_does_not_hang_fusion() {
    CASE("provenance is assembled from provider claims and cannot be trusted acyclic");

    ProvenanceGraph g;
    g.add(derived("a", "v1", {"b"}));
    g.add(derived("b", "v1", {"a"}));

    // The guarantee is that this returns at all.
    const auto sources = g.independent_sources("a");
    CHECK_BECAUSE(sources.empty(),
                  "a cycle contains no root; reporting none is correct and, at the "
                  "corroboration floor, safely refuses rather than accepting");

    const Transition t{
        .from = EpistemicState::Normalized,
        .to = EpistemicState::Corroborated,
        .actor = Actor::Fusion,
        .independent_sources = g.independent_source_count("a"),
        .at = TimePoint{kNow},
        .agent_version = "fusion-v1",
    };
    CHECK(refuse(t) == TransitionRefusal::InsufficientIndependence);
}

void an_unknown_parent_is_counted_as_its_own_origin() {
    CASE("a parent we hold no record for is still a distinct origin");

    ProvenanceGraph g;
    g.add(derived("analysis", "v1", {"missing-upstream"}));
    CHECK(g.independent_source_count("analysis") == 1);
    CHECK_EQ(g.independent_sources("analysis").front(), std::string{"missing-upstream"});
}

void re_normalizing_a_record_does_not_invent_a_second_source() {
    CASE("adding the same id twice replaces rather than duplicates");

    ProvenanceGraph g;
    g.add(fetched("obs-1", "usgs"));
    g.add(fetched("obs-1", "usgs"));  // re-run of the normalizer
    CHECK_EQ(g.size(), std::size_t{1});
    CHECK_EQ(g.independent_source_count("obs-1"), 1);
}

void the_source_list_is_stable_between_calls() {
    CASE("a provenance explanation must not reorder itself between refreshes");

    ProvenanceGraph g;
    g.add(fetched("a", "zulu"));
    g.add(fetched("b", "alpha"));
    g.add(fetched("c", "mike"));

    const std::vector<RecordId> all{"a", "b", "c"};
    const auto first = g.independent_sources(all);
    const auto second = g.independent_sources(all);
    CHECK_EQ(first, second);
    CHECK_BECAUSE(first[0] == std::string{"alpha"} && first[1] == std::string{"mike"} &&
                      first[2] == std::string{"zulu"},
                  "sorted, because a hash set's order is an implementation detail and a "
                  "shuffling explanation looks to an operator like the data changed");
}

void a_missing_record_reports_nothing_rather_than_guessing() {
    CASE("asking about a record we do not have");

    ProvenanceGraph g;
    CHECK(g.find("nope") == nullptr);
    // The id itself is the only thing known about it, so that is what is reported.
    CHECK_EQ(g.independent_source_count("nope"), 1);
}

void a_root_knows_it_is_one() {
    CASE("Provenance::is_root");

    CHECK(fetched("a", "usgs").is_root());
    CHECK(!syndicated("b", "outlet", "reuters").is_root());
    CHECK(!derived("c", "v1", {"a"}).is_root());
}

}  // namespace

int main() {
    a_wire_story_carried_by_four_outlets_is_one_source();
    genuinely_different_providers_count_separately();
    an_analysis_and_its_input_are_one_source();
    a_shared_ancestor_is_counted_once_across_a_cluster();
    an_uncredited_copy_still_counts_as_itself();
    a_relaying_root_resolves_to_what_it_relays();
    a_cycle_in_provider_metadata_does_not_hang_fusion();
    an_unknown_parent_is_counted_as_its_own_origin();
    re_normalizing_a_record_does_not_invent_a_second_source();
    the_source_list_is_stable_between_calls();
    a_missing_record_reports_nothing_rather_than_guessing();
    a_root_knows_it_is_one();

    return sentry::test::report();
}
