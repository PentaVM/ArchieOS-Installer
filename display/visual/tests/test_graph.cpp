// Node identity across refreshes: which thing on screen is which, and for how long.

#include "archie/visual/graph.hpp"

#include <algorithm>
#include <set>
#include <string>

#include "harness.hpp"

using namespace archie::visual;

namespace {

NodeSpec module_spec(std::string id, int severity = 0) {
    NodeSpec s{};
    s.id = std::move(id);
    s.kind = NodeKind::Module;
    s.label = s.id;
    s.severity = severity;
    s.magnitude = 1.0F;
    return s;
}

bool contains(const std::vector<std::string>& v, const std::string& s) {
    return std::find(v.begin(), v.end(), s) != v.end();
}

bool changed(const std::vector<NodeChange>& v, const std::string& id) {
    return std::any_of(v.begin(), v.end(), [&](const NodeChange& c) { return c.id == id; });
}

void a_repeated_payload_changes_nothing() {
    CASE("applying identical state twice produces no work");

    SceneGraph graph{};
    const std::vector<NodeSpec> specs{module_spec("archie.core"), module_spec("archie.vision")};

    const GraphDelta first = graph.apply(specs, 0);
    CHECK_EQ(first.added.size(), 2U);

    const GraphDelta second = graph.apply(specs, 1'000);
    CHECK_BECAUSE(second.empty(),
                  "a poll that reports the same state is not an event; if it produced a delta "
                  "the renderer would restart every transition on every refresh, which is what "
                  "makes a scene look like it is stuttering rather than living");

    const GraphDelta third = graph.apply(specs, 2'000);
    CHECK(third.empty());
}

void payload_order_does_not_move_anything() {
    CASE("a provider reordering its results is not an event");

    SceneGraph graph{};
    graph.apply({module_spec("a"), module_spec("b"), module_spec("c")}, 0);

    const int slot_a = graph.find("a")->slot;
    const int slot_b = graph.find("b")->slot;
    const int slot_c = graph.find("c")->slot;

    const GraphDelta delta =
        graph.apply({module_spec("c"), module_spec("a"), module_spec("b")}, 1'000);

    CHECK_BECAUSE(delta.empty(),
                  "identity is the id, not the index; a payload sorted differently describes "
                  "exactly the same scene");
    CHECK_EQ(graph.find("a")->slot, slot_a);
    CHECK_EQ(graph.find("b")->slot, slot_b);
    CHECK_EQ(graph.find("c")->slot, slot_c);
}

void a_node_keeps_its_seat_and_its_history() {
    CASE("a surviving node keeps slot, first sighting and revision");

    SceneGraph graph{};
    graph.apply({module_spec("keeper"), module_spec("other")}, 500);

    const SceneNode before = *graph.find("keeper");
    graph.apply({module_spec("keeper"), module_spec("other"), module_spec("newcomer")}, 1'500);
    const SceneNode after = *graph.find("keeper");

    CHECK_EQ(after.slot, before.slot);
    CHECK_BECAUSE(after.first_seen_ms == before.first_seen_ms,
                  "first_seen is how long the operator has been watching this thing; resetting "
                  "it because an unrelated node appeared would erase that");
    CHECK_BECAUSE(after.revision == before.revision,
                  "nothing about this node changed, so nothing downstream should re-run");
    CHECK_EQ(after.last_seen_ms, 1'500);
}

void an_arrival_finishes_on_the_clock_not_on_the_next_payload() {
    CASE("a node stops arriving when its entry has played, whatever the projection does");

    GraphPolicy policy{};
    policy.entry_duration_ms = 600;
    SceneGraph graph{policy};

    graph.apply({module_spec("newcomer")}, 0);
    CHECK_EQ(graph.find("newcomer")->presence, NodePresence::Entering);

    graph.collect(599);
    CHECK_BECAUSE(graph.find("newcomer")->presence == NodePresence::Entering,
                  "the entry animation is still running");

    graph.collect(600);
    CHECK_BECAUSE(graph.find("newcomer")->presence == NodePresence::Present,
                  "tying this to the next payload instead means a slow projection holds nodes "
                  "in their entry state for seconds, and a projection that stops entirely "
                  "leaves the whole scene permanently arriving and replaying its entrances");
}

void an_arrival_settles_even_if_no_payload_ever_comes_again() {
    CASE("the projection going silent must not freeze the scene mid-entry");

    SceneGraph graph{};
    graph.apply({module_spec("alone")}, 0);

    // Nothing but time passing. No second apply, ever.
    for (std::int64_t t = 0; t <= 5'000; t += 16) {
        graph.collect(t);
    }
    CHECK_EQ(graph.find("alone")->presence, NodePresence::Present);
    CHECK_BECAUSE(graph.size() == 1U,
                  "and settling is not removal: the node is still there, it has just finished "
                  "arriving");
}

void settling_survives_a_clock_rollback() {
    CASE("an entry does not complete because the clock moved backwards");

    GraphPolicy policy{};
    policy.entry_duration_ms = 600;
    SceneGraph graph{policy};

    graph.apply({module_spec("n")}, 1'000'000);
    graph.collect(900'000);
    CHECK_BECAUSE(graph.find("n")->presence == NodePresence::Entering,
                  "a negative elapsed time is not a completed animation, and unsigned "
                  "arithmetic on it would be a very large one");
    CHECK(graph.find("n") != nullptr);
}

void a_missing_node_is_missing_not_gone() {
    CASE("one absent poll starts a grace period rather than a death");

    GraphPolicy policy{};
    policy.departure_grace_ms = 3'000;
    SceneGraph graph{policy};

    graph.apply({module_spec("flapper"), module_spec("steady")}, 0);
    const GraphDelta delta = graph.apply({module_spec("steady")}, 1'000);

    CHECK(contains(delta.departing, "flapper"));
    CHECK(delta.removed.empty());
    CHECK_BECAUSE(graph.find("flapper") != nullptr,
                  "projections have gaps and providers flap; forgetting a node the first time "
                  "it is missing means a module that restarts between two polls visibly dies "
                  "and is reborn, and the operator sees an event that did not happen");
    CHECK_EQ(graph.find("flapper")->presence, NodePresence::Departing);

    CHECK_BECAUSE(graph.collect(2'000).removed.empty(),
                  "still inside the grace window");
}

void a_node_that_comes_back_is_the_same_node() {
    CASE("returning within grace preserves identity, seat and history");

    GraphPolicy policy{};
    policy.departure_grace_ms = 3'000;
    SceneGraph graph{policy};

    graph.apply({module_spec("flapper"), module_spec("steady")}, 0);
    const SceneNode before = *graph.find("flapper");

    graph.apply({module_spec("steady")}, 1'000);
    graph.collect(1'000);

    const GraphDelta back = graph.apply({module_spec("flapper"), module_spec("steady")}, 2'000);

    CHECK(contains(back.returned, "flapper"));
    CHECK_BECAUSE(!contains(back.added, "flapper"),
                  "it never left, so it must not be reported as new; announcing an arrival "
                  "would play an entry animation for something that was on screen the whole "
                  "time");

    const SceneNode after = *graph.find("flapper");
    CHECK_EQ(after.slot, before.slot);
    CHECK_EQ(after.first_seen_ms, before.first_seen_ms);
    CHECK_EQ(after.revision, before.revision);
    CHECK_EQ(after.presence, NodePresence::Present);
}

void grace_eventually_runs_out() {
    CASE("a node absent past its grace is forgotten and frees its seat");

    GraphPolicy policy{};
    policy.departure_grace_ms = 3'000;
    SceneGraph graph{policy};

    graph.apply({module_spec("a"), module_spec("gone"), module_spec("c")}, 0);
    const int freed = graph.find("gone")->slot;

    graph.apply({module_spec("a"), module_spec("c")}, 1'000);

    CHECK_BECAUSE(graph.collect(2'999).removed.empty(),
                  "the grace runs from the last sighting at t=0, not from the poll at t=1000 "
                  "that noticed the absence -- otherwise a slow poll would silently extend "
                  "every node's life by the length of the polling interval");
    const GraphDelta collected = graph.collect(3'000);
    CHECK(contains(collected.removed, "gone"));
    CHECK_EQ(graph.find("gone"), nullptr);
    CHECK_EQ(graph.size(), 2U);

    // The seat is reusable, and deterministically so.
    graph.apply({module_spec("a"), module_spec("c"), module_spec("newcomer")}, 5'000);
    CHECK_BECAUSE(graph.find("newcomer")->slot == freed,
                  "slots are handed out lowest-free-first so that the same sequence of events "
                  "always produces the same layout; a scene that arranges itself differently "
                  "on each run cannot be reasoned about or screenshotted for comparison");
}

void collection_is_separate_from_reconciliation() {
    CASE("the renderer gets to finish an exit before the node stops existing");

    SceneGraph graph{};
    graph.apply({module_spec("leaving")}, 0);
    graph.apply({}, 1'000);

    CHECK_BECAUSE(graph.find("leaving") != nullptr,
                  "apply must never delete: if reconciliation removed nodes outright there "
                  "would be no frame in which to draw the departure");
    graph.collect(10'000);
    CHECK_EQ(graph.find("leaving"), nullptr);
}

void a_clock_that_goes_backwards_neither_forgets_nor_freezes() {
    CASE("grace survives a clock rollback");

    GraphPolicy policy{};
    policy.departure_grace_ms = 3'000;
    SceneGraph graph{policy};

    graph.apply({module_spec("x")}, 1'000'000);
    graph.apply({}, 1'000'000);

    // The clock jumps back a long way. Naive subtraction gives a large negative elapsed
    // time here, and a large positive one if anything downstream treats it as unsigned.
    CHECK_BECAUSE(graph.collect(900'000).removed.empty(),
                  "a node must not be forgotten because of arithmetic on a bad clock");

    // And it must not be stuck waiting out a grace that can never expire.
    CHECK_BECAUSE(graph.collect(903'000).removed.size() == 1,
                  "restarting the grace from the rolled-back now keeps it finite; leaving "
                  "last_seen in the future would strand the node on screen for the length of "
                  "the jump");
}

void a_changed_node_is_reported_once() {
    CASE("a visible change bumps the revision, and an invisible one does not");

    SceneGraph graph{};
    graph.apply({module_spec("m", 0)}, 0);
    const std::uint64_t before = graph.find("m")->revision;

    const GraphDelta delta_changed = graph.apply({module_spec("m", 3)}, 1'000);
    CHECK(changed(delta_changed.updated, "m"));
    CHECK(graph.find("m")->revision != before);
    CHECK_EQ(graph.find("m")->spec.severity, 3);

    const GraphDelta again = graph.apply({module_spec("m", 3)}, 2'000);
    CHECK_BECAUSE(again.updated.empty(),
                  "the same value reported twice is not a change");

    // Both sides of the change travel with it, so the event layer can describe what
    // happened without running a second comparison that might disagree.
    CHECK_EQ(delta_changed.updated.size(), 1U);
    CHECK_EQ(delta_changed.updated[0].severity_from, 0);
    CHECK_BECAUSE(delta_changed.updated[0].severity_to == 3,
                  "a pulse that says only 'this node changed' cannot tell an operator "
                  "whether the machine got better or worse");
    CHECK_EQ(delta_changed.updated[0].freshness_from, Freshness::Live);
    CHECK_EQ(delta_changed.updated[0].freshness_to, Freshness::Live);
}

void float_jitter_is_not_a_change() {
    CASE("a provider's low-decimal noise does not re-run every transition");

    SceneGraph graph{};
    NodeSpec spec = module_spec("obs");
    spec.magnitude = 4.2F;
    graph.apply({spec}, 0);
    const std::uint64_t before = graph.find("obs")->revision;

    spec.magnitude = 4.2F + 1e-6F;
    const GraphDelta delta = graph.apply({spec}, 1'000);
    CHECK_BECAUSE(delta.updated.empty(),
                  "comparing floats exactly would bump the revision on most polls, and the "
                  "renderer would restart every transition forever over a difference no "
                  "viewer could see");
    CHECK_EQ(graph.find("obs")->revision, before);

    // A change large enough to see is still a change.
    spec.magnitude = 4.9F;
    CHECK(!graph.apply({spec}, 2'000).updated.empty());
}

void a_freshness_change_is_visible() {
    CASE("data going stale is a change worth redrawing");

    SceneGraph graph{};
    NodeSpec spec = module_spec("n");
    graph.apply({spec}, 0);

    spec.freshness = Freshness::Stale;
    CHECK_BECAUSE(changed(graph.apply({spec}, 1'000).updated, "n"),
                  "freshness drives opacity and whether the node may animate at all, so a "
                  "change in it must reach the renderer");
}

void a_node_without_an_id_is_dropped() {
    CASE("identity is not optional");

    SceneGraph graph{};
    NodeSpec anonymous{};
    anonymous.kind = NodeKind::Observation;
    anonymous.label = "unnamed";

    const GraphDelta delta = graph.apply({anonymous, module_spec("real")}, 0);
    CHECK_EQ(delta.anonymous_dropped, 1);
    CHECK_EQ(graph.size(), 1U);
    CHECK_BECAUSE(graph.find("real") != nullptr,
                  "one unusable entry must not cost the rest of the payload");
}

void a_duplicate_id_does_not_become_two_nodes() {
    CASE("the first occurrence wins and the collision is reported");

    SceneGraph graph{};
    const GraphDelta delta =
        graph.apply({module_spec("dup", 1), module_spec("dup", 9), module_spec("other")}, 0);

    CHECK(contains(delta.duplicate_ids, "dup"));
    CHECK_EQ(graph.size(), 2U);
    CHECK_BECAUSE(graph.find("dup")->spec.severity == 1,
                  "letting the last occurrence win would make the node's value depend on "
                  "payload order, and two nodes sharing an identity would make 'which one did "
                  "I click' unanswerable");
}

void edges_come_from_declarations_not_from_proximity() {
    CASE("an edge is drawn only when both ends are on screen");

    SceneGraph graph{};
    NodeSpec a = module_spec("a");
    a.depends_on = {"b", "ghost"};
    NodeSpec b = module_spec("b");

    graph.apply({a, b}, 0);
    const auto edges = graph.edges();

    CHECK_EQ(edges.size(), 1U);
    CHECK(edges[0].first == "a" && edges[0].second == "b");

    NodeSpec loop = module_spec("loop");
    loop.depends_on = {"loop"};
    SceneGraph other{};
    other.apply({loop}, 0);
    CHECK_BECAUSE(other.edges().empty(),
                  "a self-dependency is an upstream declaration bug, and drawing it as a loop "
                  "would present it to the operator as a relationship");
}

void a_dependency_change_is_a_change() {
    CASE("gaining a dependency redraws the edge");

    SceneGraph graph{};
    NodeSpec a = module_spec("a");
    graph.apply({a, module_spec("b")}, 0);
    CHECK(graph.edges().empty());

    a.depends_on = {"b"};
    CHECK(changed(graph.apply({a, module_spec("b")}, 1'000).updated, "a"));
    CHECK_EQ(graph.edges().size(), 1U);
}

void the_shape_grammar_is_total_and_distinct() {
    CASE("every kind has a shape, and no two kinds share one");

    const NodeKind kinds[] = {NodeKind::Module, NodeKind::Peer,   NodeKind::Model,
                             NodeKind::Observation, NodeKind::Sensor, NodeKind::Task,
                             NodeKind::Unknown};

    std::set<NodeShape> shapes;
    for (NodeKind kind : kinds) {
        shapes.insert(shape_of(kind));
    }
    CHECK_BECAUSE(shapes.size() == 7U,
                  "colour is already carrying severity and freshness, so kind has to be "
                  "readable from form alone; two kinds sharing a shape would be "
                  "indistinguishable on a desaturated stale node");
    CHECK_EQ(shape_of(NodeKind::Unknown), NodeShape::Placeholder);
}

void an_unknown_kind_is_drawn_anyway() {
    CASE("a subsystem this build predates is shown, not silently omitted");

    SceneGraph graph{};
    NodeSpec future{};
    future.id = "quantum.thing";
    future.kind = NodeKind::Unknown;
    future.label = "quantum.thing";

    graph.apply({future}, 0);
    CHECK_BECAUSE(graph.find("quantum.thing") != nullptr,
                  "showing that something is there and admitting we do not know what it is "
                  "beats leaving a gap the operator cannot see");
    CHECK_EQ(shape_of(graph.find("quantum.thing")->spec.kind), NodeShape::Placeholder);
}

}  // namespace

int main() {
    a_repeated_payload_changes_nothing();
    payload_order_does_not_move_anything();
    a_node_keeps_its_seat_and_its_history();
    an_arrival_finishes_on_the_clock_not_on_the_next_payload();
    an_arrival_settles_even_if_no_payload_ever_comes_again();
    settling_survives_a_clock_rollback();
    a_missing_node_is_missing_not_gone();
    a_node_that_comes_back_is_the_same_node();
    grace_eventually_runs_out();
    collection_is_separate_from_reconciliation();
    a_clock_that_goes_backwards_neither_forgets_nor_freezes();
    a_changed_node_is_reported_once();
    float_jitter_is_not_a_change();
    a_freshness_change_is_visible();
    a_node_without_an_id_is_dropped();
    a_duplicate_id_does_not_become_two_nodes();
    edges_come_from_declarations_not_from_proximity();
    a_dependency_change_is_a_change();
    the_shape_grammar_is_total_and_distinct();
    an_unknown_kind_is_drawn_anyway();
    return archie::visual::test::report();
}
