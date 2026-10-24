#include "harness.hpp"
#include "sentry/store/wanda_projection.hpp"

using namespace sentry::store;
using namespace sentry::identity;

namespace {
PersonGraph root() {
    PersonGraph graph;
    graph.owner_id = "owner";
    graph.profiles.push_back(Profile{SubjectRecord{"owner", SubjectKind::Owner, "Owner"}, {}});
    return graph;
}

void malformed_projection_graphs_are_refused() {
    CASE("a wire projection cannot contain duplicate IDs or dangling endpoints");
    auto graph = root();
    CHECK(project_wanda_json(graph).has_value());
    graph.profiles.push_back(graph.profiles[0]);
    auto duplicate = project_wanda_json(graph);
    CHECK(!duplicate);
    if (!duplicate) { CHECK_EQ(duplicate.error(), ProjectionError::InvalidGraph); }
    graph.profiles.pop_back();
    graph.asserted_edges.push_back(RelationEdge{"owner", "missing", RelationKind::Friend,
                                                EdgeBasis::OwnerDeclared, "owner", ""});
    auto dangling = project_wanda_json(graph);
    CHECK(!dangling);
    if (!dangling) { CHECK_EQ(dangling.error(), ProjectionError::InvalidGraph); }
    graph.asserted_edges.clear();
    graph.owner_id = "missing-owner";
    CHECK(!project_wanda_json(graph));
}

void node_and_edge_budgets_are_applied_before_projection() {
    CASE("bounded records are checked before traversing or emitting a graph");
    auto graph = root();
    graph.profiles.resize(kMaxProjectionNodes + 1);
    auto nodes = project_wanda_json(graph);
    CHECK(!nodes);
    if (!nodes) { CHECK_EQ(nodes.error(), ProjectionError::TooLarge); }
    graph = root();
    graph.hypotheses.resize(kMaxProjectionEdges + 1);
    auto edges = project_wanda_json(graph);
    CHECK(!edges);
    if (!edges) { CHECK_EQ(edges.error(), ProjectionError::TooLarge); }
}

void utf8_is_validated_without_replacing_someones_data() {
    CASE("invalid UTF-8 is refused instead of returning invalid or silently repaired JSON");
    const std::vector<std::string> invalid{
        std::string{"\xc0\xaf", 2}, std::string{"\xe2\x82", 2},
        std::string{"\xed\xa0\x80", 3}, std::string{"\xf4\x90\x80\x80", 4},
        std::string{"\x80", 1}};
    for (const auto& label : invalid) {
        auto graph = root();
        graph.profiles[0].subject.display_label = label;
        const auto json = project_wanda_json(graph);
        CHECK(!json);
        if (!json) { CHECK_EQ(json.error(), ProjectionError::InvalidUtf8); }
    }
}

void partner_and_manager_keep_existing_refusal_boundaries() {
    CASE("partner is a personal declaration; manager is a professional tie");
    const SubjectRecord owner{"owner", SubjectKind::Owner, "Owner"};
    const SubjectRecord person{"person", SubjectKind::ConsentedPerson, "Person"};
    RelationEdge partner{"owner", "person", RelationKind::Partner,
                          EdgeBasis::OwnerDeclared, "owner", ""};
    CHECK(is_personal(RelationKind::Partner));
    CHECK(!is_personal(RelationKind::Manager));
    CHECK(!refuse_edge(partner, owner, person, "owner"));
    CHECK_EQ(refuse_edge(partner, owner, person, "other"),
             std::optional{EdgeRefusal::NotDeclaredByAParticipant});
    partner.basis = EdgeBasis::PublicRecord;
    CHECK_EQ(refuse_edge(partner, owner, person, ""),
             std::optional{EdgeRefusal::PersonalTieFromPublicRecord});
    partner.basis = EdgeBasis::CoPresenceInference;
    CHECK_EQ(refuse_edge(partner, owner, person, "owner"),
             std::optional{EdgeRefusal::InferredFromCoPresence});
    partner.kind = RelationKind::Manager;
    partner.basis = EdgeBasis::ModelHypothesis;
    CHECK_EQ(refuse_edge(partner, owner, person, ""),
             std::optional{EdgeRefusal::GenerativeCannotAssert});
    partner.basis = EdgeBasis::PublicRecord;
    CHECK(!refuse_edge(partner, owner, person, ""));
    CHECK_EQ(name(RelationKind::Partner), std::string_view{"partner"});
    CHECK_EQ(name(RelationKind::Manager), std::string_view{"manager"});
}
}  // namespace

int main() {
    malformed_projection_graphs_are_refused();
    node_and_edge_budgets_are_applied_before_projection();
    utf8_is_validated_without_replacing_someones_data();
    partner_and_manager_keep_existing_refusal_boundaries();
    return sentry::test::report();
}
