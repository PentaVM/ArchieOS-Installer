// Events: derived from real change, bounded, and honest about what they dropped.

#include "archie/visual/events.hpp"

#include <algorithm>
#include <string>

#include "harness.hpp"

using namespace archie::visual;

namespace {

NodeChange severity_move(std::string id, int from, int to) {
    NodeChange c{};
    c.id = std::move(id);
    c.severity_from = from;
    c.severity_to = to;
    return c;
}

NodeChange freshness_move(std::string id, Freshness from, Freshness to) {
    NodeChange c{};
    c.id = std::move(id);
    c.freshness_from = from;
    c.freshness_to = to;
    return c;
}

int count_of(const EventBatch& batch, EventKind kind) {
    return static_cast<int>(std::count_if(batch.events.begin(), batch.events.end(),
                                          [&](const VisualEvent& e) { return e.kind == kind; }));
}

const VisualEvent* first_of(const EventBatch& batch, EventKind kind) {
    const auto it = std::find_if(batch.events.begin(), batch.events.end(),
                                 [&](const VisualEvent& e) { return e.kind == kind; });
    return it == batch.events.end() ? nullptr : &*it;
}

void nothing_happens_unless_something_changed() {
    CASE("an empty delta produces an empty queue");

    EventQueue queue{};
    queue.observe(GraphDelta{}, 1'000);
    queue.observe(GraphDelta{}, 2'000);
    queue.observe(GraphDelta{}, 3'000);

    CHECK_BECAUSE(queue.drain().empty(),
                  "nothing in this layer may manufacture a pulse for which no state change "
                  "exists; a display that invents events teaches its operator that motion on "
                  "screen means nothing");
}

void every_kind_of_change_becomes_its_own_event() {
    CASE("presence and value changes are derived from the delta");

    EventQueue queue{};
    GraphDelta delta{};
    delta.added = {"fresh.node"};
    delta.returned = {"came.back"};
    delta.departing = {"going"};
    delta.removed = {"gone"};
    delta.updated = {severity_move("worse", 1, 4)};

    queue.observe(delta, 5'000);
    const EventBatch batch = queue.drain();

    CHECK_EQ(count_of(batch, EventKind::Appeared), 1);
    CHECK_EQ(count_of(batch, EventKind::Returned), 1);
    CHECK_EQ(count_of(batch, EventKind::Departed), 1);
    CHECK_EQ(count_of(batch, EventKind::Removed), 1);
    CHECK_EQ(count_of(batch, EventKind::SeverityChanged), 1);
    CHECK_EQ(batch.events.size(), 5U);
    CHECK_EQ(batch.dropped, 0);

    const VisualEvent* severity = first_of(batch, EventKind::SeverityChanged);
    CHECK(severity != nullptr && severity->severity_from == 1 && severity->severity_to == 4);
    CHECK_EQ(severity->at_ms, 5'000);
}

void a_return_is_not_an_arrival() {
    CASE("the two are separate kinds so the renderer can play different things");

    EventQueue queue{};
    GraphDelta delta{};
    delta.returned = {"flapper"};
    queue.observe(delta, 100);

    const EventBatch batch = queue.drain();
    CHECK_EQ(count_of(batch, EventKind::Appeared), 0);
    CHECK_BECAUSE(count_of(batch, EventKind::Returned) == 1,
                  "a node that never left must not play an entry; the whole point of the "
                  "grace period is that the operator is not shown a death and a birth that "
                  "did not happen");
}

void a_cosmetic_change_earns_no_pulse() {
    CASE("not every change deserves the operator's attention");

    EventQueue queue{};
    GraphDelta delta{};
    // Severity and freshness both unmoved: a label or dependency edit reached the graph.
    delta.updated = {severity_move("relabelled", 2, 2)};
    queue.observe(delta, 100);

    CHECK_BECAUSE(queue.drain().empty(),
                  "the node will be redrawn, but flashing the screen for a renamed label "
                  "spends attention the operator needs for things that matter");
}

void a_walk_is_summarised_not_replayed() {
    CASE("severity 1 -> 2 -> 5 between frames is one event, from 1 to 5");

    EventQueue queue{};
    GraphDelta first{};
    first.updated = {severity_move("climber", 1, 2)};
    queue.observe(first, 100);

    GraphDelta second{};
    second.updated = {severity_move("climber", 2, 5)};
    queue.observe(second, 200);

    const EventBatch batch = queue.drain();
    CHECK_EQ(batch.events.size(), 1U);

    const VisualEvent& event = batch.events.front();
    CHECK_BECAUSE(event.severity_from == 1,
                  "keeping the original starting point is what makes the merged event a true "
                  "summary of the whole run rather than of its last step");
    CHECK_EQ(event.severity_to, 5);
    CHECK_BECAUSE(event.at_ms == 200, "the time of the most recent movement");
    CHECK(event.is_deterioration());
}

void a_value_that_returns_home_did_not_change() {
    CASE("1 -> 3 -> 1 within one frame annihilates");

    EventQueue queue{};
    GraphDelta up{};
    up.updated = {severity_move("jitter", 1, 3)};
    queue.observe(up, 100);
    CHECK_EQ(queue.size(), 1U);

    GraphDelta down{};
    down.updated = {severity_move("jitter", 3, 1)};
    queue.observe(down, 200);

    CHECK_BECAUSE(queue.size() == 0U,
                  "the value on screen is identical to the value that was there before, so "
                  "pulsing would be the display reacting to nothing");
    CHECK(queue.drain().empty());
}

void freshness_and_severity_are_separate_events() {
    CASE("two kinds of change on one node do not merge into each other");

    EventQueue queue{};
    GraphDelta delta{};
    NodeChange both = severity_move("node", 1, 2);
    both.freshness_from = Freshness::Live;
    both.freshness_to = Freshness::Stale;
    delta.updated = {both};

    queue.observe(delta, 100);
    const EventBatch batch = queue.drain();

    CHECK_EQ(batch.events.size(), 2U);
    CHECK_EQ(count_of(batch, EventKind::SeverityChanged), 1);
    CHECK_EQ(count_of(batch, EventKind::FreshnessChanged), 1);
}

void freshness_movement_knows_which_way_is_worse() {
    CASE("going stale is a deterioration; recovering is not");

    EventQueue queue{};
    GraphDelta worse{};
    worse.updated = {freshness_move("a", Freshness::Live, Freshness::Expired)};
    queue.observe(worse, 100);

    GraphDelta better{};
    better.updated = {freshness_move("b", Freshness::Stale, Freshness::Live)};
    queue.observe(better, 200);

    const EventBatch batch = queue.drain();
    CHECK_EQ(batch.events.size(), 2U);
    for (const VisualEvent& event : batch.events) {
        if (event.node_id == "a") {
            CHECK(event.is_deterioration());
        } else {
            CHECK_BECAUSE(!event.is_deterioration(),
                          "a source coming back must not be drawn in the colour reserved for "
                          "something going wrong");
        }
    }
}

void the_queue_cannot_grow_without_bound() {
    CASE("a storm costs a fixed amount of memory");

    EventQueue queue{8};
    for (int i = 0; i < 100; ++i) {
        GraphDelta delta{};
        delta.added = {"node" + std::to_string(i)};
        queue.observe(delta, i);
        CHECK(queue.size() <= 8U);
    }

    CHECK_EQ(queue.size(), 8U);
    CHECK_BECAUSE(queue.dropped() == 92,
                  "a node that loses its provider changes state on every poll, and a hundred "
                  "of them would otherwise fill memory with pulses nobody will ever watch");
}

void the_newest_events_are_the_ones_kept() {
    CASE("overflow drops the oldest, not the most recent");

    EventQueue queue{3};
    for (int i = 0; i < 6; ++i) {
        GraphDelta delta{};
        delta.added = {"n" + std::to_string(i)};
        queue.observe(delta, i);
    }

    const EventBatch batch = queue.drain();
    CHECK_EQ(batch.events.size(), 3U);
    CHECK_BECAUSE(batch.events[0].node_id == "n3" && batch.events[2].node_id == "n5",
                  "an operator looking at a busy machine wants what just happened; a queue "
                  "that kept the oldest would go blind exactly when things got interesting");
}

void a_lossy_batch_says_so() {
    CASE("the interface can admit to a gap rather than implying a complete history");

    EventQueue queue{2};
    for (int i = 0; i < 5; ++i) {
        GraphDelta delta{};
        delta.added = {"n" + std::to_string(i)};
        queue.observe(delta, i);
    }

    const EventBatch batch = queue.drain();
    CHECK_EQ(batch.dropped, 3);
    CHECK_BECAUSE(!batch.empty(),
                  "a batch that lost events is not an empty one, even if it carried none");
}

void draining_clears_the_queue_and_the_counter() {
    CASE("events are delivered once");

    EventQueue queue{2};
    for (int i = 0; i < 5; ++i) {
        GraphDelta delta{};
        delta.added = {"n" + std::to_string(i)};
        queue.observe(delta, i);
    }
    CHECK_EQ(queue.drain().dropped, 3);

    CHECK_EQ(queue.size(), 0U);
    CHECK_EQ(queue.dropped(), 0);
    const EventBatch second = queue.drain();
    CHECK_BECAUSE(second.empty(),
                  "a drained event replayed on the next frame would pulse twice for one thing "
                  "happening once");
}

void coalescing_does_not_consume_capacity() {
    CASE("one flapping node costs one slot, not one per poll");

    EventQueue queue{4};
    for (int i = 0; i < 500; ++i) {
        GraphDelta delta{};
        delta.updated = {severity_move("flapper", i % 2 == 0 ? 1 : 2, i % 2 == 0 ? 2 : 1)};
        queue.observe(delta, i);
    }

    CHECK_BECAUSE(queue.dropped() == 0,
                  "coalescing has to happen before the capacity check, or a single noisy node "
                  "would evict every other event on the machine");
    CHECK(queue.size() <= 1U);
}

void a_zero_capacity_queue_still_works() {
    CASE("a degenerate capacity is corrected rather than dividing the queue by zero");

    EventQueue queue{0};
    CHECK_EQ(queue.capacity(), 1U);

    GraphDelta delta{};
    delta.added = {"a"};
    queue.observe(delta, 1);
    delta.added = {"b"};
    queue.observe(delta, 2);

    CHECK_EQ(queue.size(), 1U);
    CHECK_EQ(queue.dropped(), 1);
}

}  // namespace

int main() {
    nothing_happens_unless_something_changed();
    every_kind_of_change_becomes_its_own_event();
    a_return_is_not_an_arrival();
    a_cosmetic_change_earns_no_pulse();
    a_walk_is_summarised_not_replayed();
    a_value_that_returns_home_did_not_change();
    freshness_and_severity_are_separate_events();
    freshness_movement_knows_which_way_is_worse();
    the_queue_cannot_grow_without_bound();
    the_newest_events_are_the_ones_kept();
    a_lossy_batch_says_so();
    draining_clears_the_queue_and_the_counter();
    coalescing_does_not_consume_capacity();
    a_zero_capacity_queue_still_works();
    return archie::visual::test::report();
}
