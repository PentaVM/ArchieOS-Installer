// The timeline, and the difference between "what was true then" and "what was nearest".

#include "archie/visual/history.hpp"

#include <string>

#include "harness.hpp"

using namespace archie::visual;

namespace {

HistoryEntry entry_at(std::int64_t at_ms, int severity = 0, int total = 1) {
    HistoryEntry entry{};
    entry.at_ms = at_ms;
    entry.total_nodes = total;
    entry.nodes.push_back(NodeSample{"n", severity, Freshness::Live, NodePresence::Present});
    return entry;
}

void seek_returns_what_was_in_force_not_what_was_nearest() {
    CASE("asking about 14:02 gives the 14:00 state, not the 14:05 one");

    History history{};
    history.record(entry_at(0, 1));
    history.record(entry_at(300'000, 2));

    const HistoryEntry* answer = history.at_or_before(120'000);
    CHECK(answer != nullptr);
    CHECK_BECAUSE(answer->at_ms == 0,
                  "the 300000 entry is closer in time, but it describes a state the machine "
                  "had not reached yet; returning it would show the operator a future they "
                  "are asking to look behind");
    CHECK_EQ(answer->nodes[0].severity, 1);

    // Exactly on a boundary is that entry, not the one before it.
    const HistoryEntry* boundary = history.at_or_before(300'000);
    CHECK(boundary != nullptr && boundary->at_ms == 300'000);
}

void a_time_before_the_recording_has_no_answer() {
    CASE("the display does not claim to know what it never saw");

    History history{};
    history.record(entry_at(10'000, 3));

    CHECK_BECAUSE(history.at_or_before(9'999) == nullptr,
                  "handing back the oldest entry would claim that state held for a period "
                  "nobody observed, and an operator scrubbing before the daemon started would "
                  "see a confident answer for a time nothing was running");
    CHECK(history.at_or_before(10'000) != nullptr);
}

void a_time_after_the_last_entry_is_the_last_entry() {
    CASE("the newest state is still in force until something replaces it");

    History history{};
    history.record(entry_at(1'000, 2));

    const HistoryEntry* answer = history.at_or_before(9'999'999);
    CHECK(answer != nullptr);
    CHECK_BECAUSE(answer->at_ms == 1'000,
                  "unlike the past, the future of the last entry is known: nothing has changed "
                  "since, which is exactly what an empty stretch of timeline means");
}

void an_empty_timeline_answers_nothing() {
    CASE("no entries means no claims");

    History history{};
    CHECK_EQ(history.at_or_before(0), nullptr);
    CHECK_EQ(history.newest(), nullptr);
    CHECK_EQ(history.oldest(), nullptr);
    CHECK(history.span().first == 0 && history.span().second == 0);
    CHECK_EQ(history.size(), 0U);
}

void a_still_scene_does_not_fill_the_ring() {
    CASE("sixty identical ticks a second must not cost sixty entries");

    HistoryPolicy policy{};
    policy.capacity = 600;
    policy.keyframe_interval_ms = 5'000;
    History history{policy};

    // Ten seconds of a scene in which nothing whatsoever happens.
    for (int i = 0; i < 600; ++i) {
        history.record(entry_at(i * 16 + 1, 1));
    }

    CHECK_BECAUSE(history.size() <= 4U,
                  "an idle minute that consumed the whole ring would mean the timeline could "
                  "never reach back past the last idle minute");
    CHECK(history.size() >= 2U);
}

void a_change_is_recorded_immediately() {
    CASE("a change does not have to wait out the keyframe interval");

    HistoryPolicy policy{};
    policy.keyframe_interval_ms = 60'000;
    History history{policy};

    history.record(entry_at(0, 1));
    CHECK_BECAUSE(history.record(entry_at(16, 3)),
                  "waiting a minute to record that something broke would make the timeline "
                  "useless for the one thing an operator scrubs back to find");
    CHECK_EQ(history.size(), 2U);
    CHECK_EQ(history.at_or_before(16)->nodes[0].severity, 3);
}

void a_quiet_scene_still_gets_keyframes() {
    CASE("long stretches with nothing in them make a scrub look like a jump");

    HistoryPolicy policy{};
    policy.keyframe_interval_ms = 5'000;
    History history{policy};

    history.record(entry_at(0, 1));
    CHECK(!history.record(entry_at(4'999, 1)));
    CHECK(history.record(entry_at(5'000, 1)));
    CHECK_EQ(history.size(), 2U);
}

void every_recorded_field_counts_as_a_change() {
    CASE("a change in anything an operator would read is worth an entry");

    HistoryPolicy policy{};
    policy.keyframe_interval_ms = 1'000'000;
    History history{policy};
    history.record(entry_at(0, 1));

    HistoryEntry tier_moved = entry_at(10, 1);
    tier_moved.tier = RenderTier::Conserve;
    CHECK(history.record(std::move(tier_moved)));

    HistoryEntry mode_moved = entry_at(20, 1);
    mode_moved.tier = RenderTier::Conserve;
    mode_moved.mode = SceneMode::Watcher;
    CHECK(history.record(std::move(mode_moved)));

    HistoryEntry freshness_moved = entry_at(30, 1);
    freshness_moved.tier = RenderTier::Conserve;
    freshness_moved.mode = SceneMode::Watcher;
    freshness_moved.nodes[0].freshness = Freshness::Stale;
    CHECK_BECAUSE(history.record(std::move(freshness_moved)),
                  "a node going stale is exactly the kind of thing someone scrubs back to "
                  "find the moment of");

    HistoryEntry aggregated_moved = entry_at(40, 1);
    aggregated_moved.tier = RenderTier::Conserve;
    aggregated_moved.mode = SceneMode::Watcher;
    aggregated_moved.nodes[0].freshness = Freshness::Stale;
    aggregated_moved.aggregated = 12;
    CHECK(history.record(std::move(aggregated_moved)));

    CHECK_EQ(history.size(), 5U);
}

void the_timestamp_alone_is_not_a_change() {
    CASE("differs_from ignores the clock, or nothing would ever coalesce");

    const HistoryEntry first = entry_at(0, 1);
    const HistoryEntry later = entry_at(999, 1);
    CHECK_BECAUSE(!later.differs_from(first),
                  "every entry differs in its timestamp, so counting that would make the "
                  "comparison always true and the keyframe logic dead code");
}

void the_ring_is_bounded_and_says_what_it_lost() {
    CASE("a timeline cannot grow forever, and must admit how far back it reaches");

    HistoryPolicy policy{};
    policy.capacity = 4;
    policy.keyframe_interval_ms = 1;
    History history{policy};

    for (int i = 0; i < 20; ++i) {
        history.record(entry_at(i * 100, i));
    }

    CHECK_EQ(history.size(), 4U);
    CHECK_EQ(history.evicted(), 16);
    CHECK_BECAUSE(history.oldest()->at_ms == 1'600,
                  "the oldest entries go first: a timeline is for looking at the recent past, "
                  "and dropping the newest would make it useless for the thing it is for");
    CHECK_EQ(history.newest()->at_ms, 1'900);

    CHECK_BECAUSE(history.at_or_before(500) == nullptr,
                  "once an entry is evicted the timeline genuinely no longer knows what was "
                  "true then, and must say so rather than answering from the oldest survivor");

    const auto [from, to] = history.span();
    CHECK(from == 1'600 && to == 1'900);
}

void a_clock_that_goes_backwards_does_not_corrupt_the_order() {
    CASE("an out-of-order entry is refused and counted, not inserted");

    History history{};
    history.record(entry_at(1'000'000, 1));
    history.record(entry_at(1'000'100, 2));

    CHECK_BECAUSE(!history.record(entry_at(900'000, 3)),
                  "inserting it would make the answer to 'what was true then' depend on the "
                  "order things arrived, and every seek after it would be wrong");
    CHECK_EQ(history.out_of_order(), 1);
    CHECK_EQ(history.size(), 2U);

    // The timeline is still monotonic and still answerable.
    CHECK_EQ(history.at_or_before(1'000'050)->at_ms, 1'000'000);
    CHECK_EQ(history.at_or_before(900'000), nullptr);
}

void a_duplicate_timestamp_is_refused() {
    CASE("two entries at the same instant would make a seek ambiguous");

    History history{};
    history.record(entry_at(500, 1));
    CHECK(!history.record(entry_at(500, 9)));
    CHECK_EQ(history.out_of_order(), 1);
    CHECK_EQ(history.at_or_before(500)->nodes[0].severity, 1);
}

void a_degenerate_capacity_is_corrected() {
    CASE("a zero-capacity ring would drop everything it was given");

    History history{HistoryPolicy{0, 1'000}};
    CHECK_EQ(history.capacity(), 1U);
    CHECK(history.record(entry_at(0, 1)));
    CHECK_EQ(history.size(), 1U);
}

void seeking_is_exact_across_a_long_timeline() {
    CASE("every moment resolves to the entry that was in force at it");

    HistoryPolicy policy{};
    policy.capacity = 500;
    policy.keyframe_interval_ms = 1;
    History history{policy};

    for (int i = 0; i < 200; ++i) {
        history.record(entry_at(i * 1'000, i));
    }

    bool all_correct = true;
    for (int i = 0; i < 200; ++i) {
        for (const std::int64_t offset : {std::int64_t{0}, std::int64_t{1}, std::int64_t{999}}) {
            const HistoryEntry* found = history.at_or_before(i * 1'000 + offset);
            if (found == nullptr || found->at_ms != i * 1'000) {
                all_correct = false;
            }
        }
    }
    CHECK_BECAUSE(all_correct,
                  "a binary search that lands one entry either side is a timeline that shows "
                  "the wrong state for a moment the operator is studying closely");
}

}  // namespace

int main() {
    seek_returns_what_was_in_force_not_what_was_nearest();
    a_time_before_the_recording_has_no_answer();
    a_time_after_the_last_entry_is_the_last_entry();
    an_empty_timeline_answers_nothing();
    a_still_scene_does_not_fill_the_ring();
    a_change_is_recorded_immediately();
    a_quiet_scene_still_gets_keyframes();
    every_recorded_field_counts_as_a_change();
    the_timestamp_alone_is_not_a_change();
    the_ring_is_bounded_and_says_what_it_lost();
    a_clock_that_goes_backwards_does_not_corrupt_the_order();
    a_duplicate_timestamp_is_refused();
    a_degenerate_capacity_is_corrected();
    seeking_is_exact_across_a_long_timeline();
    return archie::visual::test::report();
}
