// What gets drawn, and the rule that the scene holds still while it decides.

#include "archie/visual/lod.hpp"

#include <algorithm>
#include <string>

#include "harness.hpp"

using namespace archie::visual;

namespace {

LodCandidate node(std::string id, int severity, float magnitude = 0.0F, float distance = 10.0F) {
    LodCandidate c{};
    c.id = std::move(id);
    c.severity = severity;
    c.magnitude = magnitude;
    c.distance = distance;
    return c;
}

bool drawn(const LodPlan& plan, const std::string& id) {
    return std::any_of(plan.visible.begin(), plan.visible.end(),
                       [&](const VisibleNode& v) { return v.id == id; });
}

// ---------------------------------------------------------------------------
// Semantic zoom.
// ---------------------------------------------------------------------------

void detail_falls_away_in_a_predictable_order() {
    CASE("each step removes a kind of information, not a polygon count");

    const LodPolicy policy{};
    CHECK_EQ(detail_for(1.0F, false, policy), DetailLevel::Inspect);
    CHECK_EQ(detail_for(policy.inspect_within, false, policy), DetailLevel::Inspect);
    CHECK_EQ(detail_for(policy.inspect_within + 0.1F, false, policy), DetailLevel::Full);
    CHECK_EQ(detail_for(policy.full_within, false, policy), DetailLevel::Full);
    CHECK_EQ(detail_for(policy.full_within + 0.1F, false, policy), DetailLevel::Simplified);
    CHECK_EQ(detail_for(policy.simplified_within, false, policy), DetailLevel::Simplified);
    CHECK_EQ(detail_for(policy.simplified_within + 0.1F, false, policy), DetailLevel::Silhouette);
    CHECK_EQ(detail_for(100'000.0F, false, policy), DetailLevel::Silhouette);
}

void the_inspected_node_is_detailed_wherever_it_is() {
    CASE("a focused node gets full detail before the camera arrives");

    const LodPolicy policy{};
    CHECK_BECAUSE(detail_for(5'000.0F, true, policy) == DetailLevel::Inspect,
                  "the panel beside it is already filling in while the camera dives; drawing "
                  "the node itself as a distant blob until arrival makes the interface look "
                  "like it registered the click late");
}

// ---------------------------------------------------------------------------
// Selection.
// ---------------------------------------------------------------------------

void the_budget_is_never_exceeded() {
    CASE("the tier's cap is a cap");

    LodPlanner planner{};
    std::vector<LodCandidate> many;
    for (int i = 0; i < 500; ++i) {
        many.push_back(node("n" + std::to_string(i), i % 4));
    }

    const LodPlan plan = planner.plan(many, 50, {});
    CHECK_EQ(plan.drawn(), 50U);
    CHECK_BECAUSE(plan.aggregated == 450,
                  "a display that shows 50 of 500 nodes without saying so is telling the "
                  "operator the machine is quieter than it is");
    CHECK_EQ(plan.drawn() + static_cast<std::size_t>(plan.aggregated), many.size());
}

void nothing_is_lost_when_everything_fits() {
    CASE("a scene inside its budget aggregates nothing");

    LodPlanner planner{};
    const LodPlan plan = planner.plan({node("a", 1), node("b", 2)}, 100, {});
    CHECK_EQ(plan.drawn(), 2U);
    CHECK_EQ(plan.aggregated, 0);
}

void severity_outranks_magnitude() {
    CASE("a large quiet reading never pushes out a small critical one");

    LodPlanner planner{};
    const std::vector<LodCandidate> candidates{
        node("quiet.but.huge", 0, 9'999.0F),
        node("small.but.critical", 3, 0.001F),
    };

    const LodPlan plan = planner.plan(candidates, 1, {});
    CHECK_EQ(plan.drawn(), 1U);
    CHECK_BECAUSE(drawn(plan, "small.but.critical"),
                  "magnitude is an unbounded provider number and severity is a judgement; "
                  "adding them raw would let any large reading outrank any alarm");
}

void magnitude_orders_nodes_of_equal_severity() {
    CASE("magnitude is the tie-break it was meant to be");

    LodPlanner planner{};
    const std::vector<LodCandidate> candidates{
        node("small", 2, 1.0F),
        node("large", 2, 50.0F),
    };
    const LodPlan plan = planner.plan(candidates, 1, {});
    CHECK(drawn(plan, "large"));
}

void selection_is_deterministic() {
    CASE("identical input produces an identical plan, whatever order it arrives in");

    std::vector<LodCandidate> candidates{node("c", 1), node("a", 1), node("b", 1)};

    LodPlanner first{};
    const LodPlan plan_a = first.plan(candidates, 2, {});

    std::reverse(candidates.begin(), candidates.end());
    LodPlanner second{};
    const LodPlan plan_b = second.plan(candidates, 2, {});

    CHECK_EQ(plan_a.drawn(), plan_b.drawn());
    bool same = plan_a.visible.size() == plan_b.visible.size();
    for (std::size_t i = 0; i < plan_a.visible.size() && i < plan_b.visible.size(); ++i) {
        if (plan_a.visible[i].id != plan_b.visible[i].id) {
            same = false;
        }
    }
    CHECK_BECAUSE(same,
                  "breaking ties by insertion order, a hash or a pointer would make the "
                  "visible set depend on payload ordering, which is the same flicker in a "
                  "disguise");
}

// ---------------------------------------------------------------------------
// Stability. The reason this class is stateful.
// ---------------------------------------------------------------------------

void a_node_hovering_at_the_cutoff_does_not_blink() {
    CASE("incumbency: a marginal challenger does not displace what is on screen");

    LodPlanner planner{};

    // Two nodes of equal severity, one slightly larger, and room for exactly one.
    LodPlan plan = planner.plan({node("incumbent", 1, 5.00F), node("rival", 1, 4.99F)}, 1, {});
    CHECK(drawn(plan, "incumbent"));

    // The rival edges ahead by a hair, as provider numbers do every poll.
    for (int i = 0; i < 10; ++i) {
        plan = planner.plan({node("incumbent", 1, 5.00F), node("rival", 1, 5.01F)}, 1, {});
        CHECK_BECAUSE(drawn(plan, "incumbent"),
                      "swapping on a rounding error makes nodes blink in and out several "
                      "times a second, and the eye goes to whatever is flickering rather "
                      "than to whatever matters");
        plan = planner.plan({node("incumbent", 1, 5.00F), node("rival", 1, 4.98F)}, 1, {});
        CHECK(drawn(plan, "incumbent"));
    }
}

void a_genuinely_worse_situation_still_gets_through() {
    CASE("incumbency is a margin, not a lock");

    LodPlanner planner{};
    LodPlan plan = planner.plan({node("incumbent", 1, 5.0F), node("rival", 1, 4.0F)}, 1, {});
    CHECK(drawn(plan, "incumbent"));

    // The rival gains a whole severity level. That is not a rounding error.
    plan = planner.plan({node("incumbent", 1, 5.0F), node("rival", 2, 4.0F)}, 1, {});
    CHECK_BECAUSE(drawn(plan, "rival"),
                  "holding a node's place against something that has become clearly more "
                  "important would hide exactly the event the operator is watching for");
    CHECK(!drawn(plan, "incumbent"));
}

void the_inspected_node_is_never_evicted() {
    CASE("the display does not decide what the operator should be looking at");

    LodPlanner planner{};
    std::vector<LodCandidate> candidates{node("boring", 0, 0.0F)};
    for (int i = 0; i < 100; ++i) {
        candidates.push_back(node("crisis" + std::to_string(i), 9, 100.0F));
    }

    const LodPlan plan = planner.plan(candidates, 5, "boring");
    CHECK_BECAUSE(drawn(plan, "boring"),
                  "dropping the node someone is reading because a hundred worse things "
                  "appeared elsewhere loses their place and their context at once");
    CHECK_EQ(plan.drawn(), 5U);
    CHECK_BECAUSE(plan.aggregated == 96,
                  "the guarantee costs a slot, and the count must still add up");
}

void a_focused_node_ranks_first() {
    CASE("the inspected node leads the draw order");

    LodPlanner planner{};
    const LodPlan plan =
        planner.plan({node("severe", 9), node("mine", 0), node("also.severe", 8)}, 3, "mine");
    CHECK_EQ(plan.visible.front().id, std::string{"mine"});
    CHECK_EQ(plan.visible.front().detail, DetailLevel::Inspect);
}

void an_empty_budget_draws_nothing_and_forgets() {
    CASE("Headless aggregates everything and drops its incumbency record");

    LodPlanner planner{};
    planner.plan({node("a", 5), node("b", 1)}, 2, {});
    CHECK_EQ(planner.visible_set().size(), 2U);

    const LodPlan dark = planner.plan({node("a", 5), node("b", 1)}, 0, {});
    CHECK_EQ(dark.drawn(), 0U);
    CHECK_EQ(dark.aggregated, 2);
    CHECK_BECAUSE(planner.visible_set().empty(),
                  "a node cannot claim incumbency from a period when nothing was drawn; "
                  "whatever comes back when the screen returns should do so on merit");

    // A negative budget is a caller bug, not a licence to draw everything.
    const LodPlan negative = planner.plan({node("a", 5)}, -10, {});
    CHECK_EQ(negative.drawn(), 0U);
}

void a_mode_change_clears_incumbency() {
    CASE("reset: importance in the view you left is not importance in the one you entered");

    LodPlanner planner{};
    planner.plan({node("a", 1, 5.0F), node("b", 1, 4.0F)}, 1, {});
    CHECK_EQ(planner.visible_set().count("a"), 1U);

    planner.reset();
    CHECK(planner.visible_set().empty());

    // With no incumbency, the better candidate wins outright.
    const LodPlan plan = planner.plan({node("a", 1, 4.0F), node("b", 1, 5.0F)}, 1, {});
    CHECK_BECAUSE(drawn(plan, "b"),
                  "carrying a claim across a mode change privileges nodes that mattered in a "
                  "view the operator has left");
}

void a_departed_node_frees_its_place() {
    CASE("incumbency does not outlive the node");

    LodPlanner planner{};
    planner.plan({node("old", 5), node("waiting", 1)}, 1, {});
    CHECK(planner.visible_set().count("old") == 1U);

    const LodPlan plan = planner.plan({node("waiting", 1)}, 1, {});
    CHECK(drawn(plan, "waiting"));
    CHECK_BECAUSE(planner.visible_set().count("old") == 0U,
                  "a node that is no longer in the payload must not keep holding a slot "
                  "against the nodes that are");
}

}  // namespace

int main() {
    detail_falls_away_in_a_predictable_order();
    the_inspected_node_is_detailed_wherever_it_is();
    the_budget_is_never_exceeded();
    nothing_is_lost_when_everything_fits();
    severity_outranks_magnitude();
    magnitude_orders_nodes_of_equal_severity();
    selection_is_deterministic();
    a_node_hovering_at_the_cutoff_does_not_blink();
    a_genuinely_worse_situation_still_gets_through();
    the_inspected_node_is_never_evicted();
    a_focused_node_ranks_first();
    an_empty_budget_draws_nothing_and_forgets();
    a_mode_change_clears_incumbency();
    a_departed_node_frees_its_place();
    return archie::visual::test::report();
}
