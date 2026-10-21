// Wanda and the stored person graph, end to end: a person the owner knows is remembered
// across a restart, Wanda raises a tie about them, and the graph says who stands behind
// that tie at each step.
//
// The people are simulated. Encounters are given here as counts; nothing on the node yet
// produces `CoPresence` from a camera, because IRIS does not resolve who a person is.

#include "harness.hpp"

#include "sentry/identity/wanda.hpp"
#include "sentry/store/graph_store.hpp"

#include <filesystem>

using namespace sentry::identity;
using namespace sentry::identity::wanda;
using namespace sentry::store;
using sentry::policy::ConsentEvidence;
using sentry::policy::ConsentLease;
using sentry::policy::ConsentScope;
using sentry::world::EpistemicState;
using sentry::world::TimePoint;

namespace {

TimePoint at(std::int64_t ms) { return TimePoint{ms}; }

constexpr std::int64_t kDay = 24LL * 60LL * 60LL * 1000LL;

Profile profile(std::string id, SubjectKind kind) {
    Profile p;
    p.subject.subject_id = std::move(id);
    p.subject.kind = kind;
    p.subject.display_label = p.subject.subject_id;
    return p;
}

ConsentLease identifiable(std::string id) {
    ConsentLease granted;
    granted.subject_id = std::move(id);
    granted.scopes = {ConsentScope::CameraIdentity};
    granted.valid_from = at(0);
    granted.evidence = ConsentEvidence::SubjectSigned;
    return granted;
}

std::filesystem::path scratch() {
    auto path = std::filesystem::temp_directory_path() / "sentry-wanda-memory-test";
    std::filesystem::create_directories(path);
    return path / "graph.tsv";
}

void a_person_and_a_tie_through_a_restart() {
    const GraphStore store(scratch());

    CASE("a person the owner described is remembered after a restart");
    PersonGraph graph;
    graph.owner_id = "nelson";
    graph.profiles.push_back(profile("nelson", SubjectKind::Owner));
    auto pedro = profile("pedro", SubjectKind::ConsentedPerson);
    Trait plays;
    plays.key = "hobbies";
    plays.value = "fútbol";
    plays.basis = TraitBasis::OwnerDeclared;
    pedro.traits.push_back(plays);
    graph.profiles.push_back(std::move(pedro));
    CHECK(store.save(graph).has_value());
    auto loaded = store.load();
    CHECK(loaded.has_value() && loaded->clean());
    CHECK(loaded->graph.find("pedro") != nullptr);
    CHECK(loaded->graph.find("pedro")->asserted("hobbies") != nullptr);

    CASE("Wanda raises the tie as a question, never as a fact");
    WandaPolicy policy;
    policy.social_inference_enabled = true;
    CoPresence evidence;
    evidence.encounters = 4;
    evidence.distinct_places = 3;
    evidence.distinct_days = 3;
    const auto hypothesis =
        propose(policy, loaded->graph.find("nelson")->subject, loaded->graph.find("pedro")->subject,
                identifiable("nelson"), identifiable("pedro"), evidence, true, true, at(1000));
    CHECK(hypothesis.has_value());
    CHECK(hypothesis->state == EpistemicState::Inferred);
    CHECK(hypothesis->kind == RelationKind::Friend);
    CHECK(hypothesis->confidence.permille() == 30 * 4 + 90 * 3 + 80 * 3);
    CHECK(hypothesis->confidence.permille() < Confidence::kCeilingPermille);

    CASE("the store keeps the hypothesis apart from asserted ties, and it reaches nobody");
    // There is no Wanda-to-store call: the caller writes the hypothesis as an edge itself,
    // and the edge has no field for the confidence, the falsifier or the expiry. Only the
    // note can say it came from Wanda.
    RelationEdge raised;
    raised.from_id = hypothesis->from_id;
    raised.to_id = hypothesis->to_id;
    raised.kind = hypothesis->kind;
    raised.basis = EdgeBasis::ModelHypothesis;
    raised.source_note = "wanda";
    loaded->graph.hypotheses.push_back(raised);
    CHECK(store.save(loaded->graph).has_value());
    auto reloaded = store.load();
    CHECK(reloaded.has_value() && reloaded->clean());
    CHECK(reloaded->graph.hypotheses.size() == 1);
    CHECK(reloaded->graph.asserted_edges.empty());
    CHECK(reloaded->graph.hypotheses[0].basis == EdgeBasis::ModelHypothesis);
    CHECK(reloaded->graph.hypotheses[0].source_note == "wanda");
    CHECK(!reloaded->graph.to_relation_graph().path_to_owner("pedro").has_value());

    CASE("when Pedro confirms it, the tie is his declaration and Wanda is the margin note");
    const auto confirmed = promote(*hypothesis, "pedro", at(2000));
    CHECK(confirmed.has_value());
    CHECK(confirmed->basis == EdgeBasis::SubjectDeclared);
    CHECK(confirmed->declarer_id == "pedro");
    CHECK(confirmed->source_note.find("Wanda") != std::string::npos);
    reloaded->graph.asserted_edges.push_back(*confirmed);
    CHECK(store.save(reloaded->graph).has_value());
    auto after = store.load();
    CHECK(after.has_value() && after->clean());
    CHECK(after->graph.asserted_edges.size() == 1);
    CHECK(after->graph.asserted_edges[0].declarer_id == "pedro");
    CHECK(after->graph.to_relation_graph().path_to_owner("pedro").has_value());

    CASE("nobody else can confirm it, and a month later nobody can");
    CHECK(!promote(*hypothesis, "ana", at(2000)).has_value());
    CHECK(!promote(*hypothesis, "pedro", at(1000 + 31 * kDay)).has_value());

    std::filesystem::remove_all(store.path().parent_path());
}

}  // namespace

int main() {
    a_person_and_a_tie_through_a_restart();
    return sentry::test::report();
}
