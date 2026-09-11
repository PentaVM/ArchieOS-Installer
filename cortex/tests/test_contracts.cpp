#include "archie/cortex/contracts.hpp"
#include "archie/cortex/ipc.hpp"

#include <cstdlib>
#include <iostream>
#include <string>

namespace {

int checks = 0;
int failures = 0;

#define CHECK(expression)                                                                    \
    do {                                                                                     \
        ++checks;                                                                            \
        if (!(expression)) {                                                                 \
            ++failures;                                                                      \
            std::cerr << __FILE__ << ':' << __LINE__ << ": check failed: " #expression     \
                      << '\n';                                                              \
        }                                                                                    \
    } while (false)

using namespace archie::cortex;

GraphNode node(std::string id, NodeKind kind, std::string summary, Sensitivity sensitivity,
               std::int64_t observed_at = 1'700'000'000'000) {
    return GraphNode{.id = std::move(id),
                     .kind = kind,
                     .summary = std::move(summary),
                     .aliases = {},
                     .provenance_ref = "source:test",
                     .sensitivity = sensitivity,
                     .observed_at_ms = observed_at,
                     .version = 1};
}

GraphEdge edge(std::string id, std::string source, std::string target, EdgeKind kind) {
    return GraphEdge{.id = std::move(id),
                     .source_id = std::move(source),
                     .target_id = std::move(target),
                     .kind = kind,
                     .provenance_ref = "source:test",
                     .observed_at_ms = 1'700'000'000'000,
                     .version = 1};
}

GraphSnapshot graph() {
    GraphSnapshot snapshot;
    snapshot.snapshot_id = "snapshot-1";
    snapshot.captured_at_ms = 1'700'000'000'000;
    snapshot.nodes = {
        node("person:rafita", NodeKind::Person, "Rafita discussed the robot arm project.",
             Sensitivity::LocalRestricted),
        node("project:robot-arm", NodeKind::Project,
             "Robot arm engineering project with Motor X.", Sensitivity::LocalRestricted),
        node("simulation:motor-x", NodeKind::Simulation,
             "Failed Motor X thermal simulation and follow-up decision.",
             Sensitivity::LocalRestricted),
        node("secret:credential", NodeKind::Document, "Rafita password credential.",
             Sensitivity::Secret),
    };
    snapshot.edges = {
        edge("edge:person-project", "person:rafita", "project:robot-arm", EdgeKind::Mentions),
        edge("edge:project-simulation", "project:robot-arm", "simulation:motor-x",
             EdgeKind::DependsOn),
    };
    return snapshot;
}

ContextRequest request() {
    ContextRequest value;
    value.request_id = "request-1";
    value.question = "What did Rafita say about the robot arm?";
    value.graph_snapshot_id = "snapshot-1";
    value.permitted_sensitivities = {Sensitivity::LocalRestricted};
    value.token_budget = 512;
    value.maximum_items = 8;
    value.maximum_expansion_hops = 2;
    value.requested_at_ms = 1'700'000'001'000;
    return value;
}

bool selected(const ContextBundle& bundle, std::string_view id) {
    for (const auto& item : bundle.facts) {
        if (item.node_id == id) {
            return true;
        }
    }
    return false;
}

void graph_retrieval_is_bounded_provenanced_and_deterministic() {
    DeterministicContextCompiler compiler;
    ContextBundle first;
    ContextBundle second;
    const auto snapshot = graph();
    const auto context_request = request();
    CHECK(!compiler.compile(snapshot, context_request, &first).has_value());
    CHECK(!compiler.compile(snapshot, context_request, &second).has_value());
    CHECK(selected(first, "person:rafita"));
    CHECK(selected(first, "project:robot-arm"));
    CHECK(selected(first, "simulation:motor-x"));
    CHECK(!selected(first, "secret:credential"));
    CHECK(first.metrics.token_count <= context_request.token_budget);
    CHECK(first.metrics.selected_edges == 2U);
    CHECK(first.render(context_request.question) == second.render(context_request.question));
    CHECK(first.render(context_request.question).find("source:test") != std::string::npos);
}

void strict_sensitivity_and_budget_are_hard_gates() {
    DeterministicContextCompiler compiler;
    auto snapshot = graph();
    auto context_request = request();
    context_request.question = "Rafita credential";
    context_request.maximum_expansion_hops = 0;
    context_request.token_budget = 64;
    ContextBundle bundle;
    CHECK(!compiler.compile(snapshot, context_request, &bundle).has_value());
    CHECK(!selected(bundle, "secret:credential"));
    CHECK(bundle.metrics.token_count <= 64U);

    context_request.permitted_sensitivities.clear();
    CHECK(compiler.compile(snapshot, context_request, &bundle) ==
          ContractRefusal::MissingSensitivityPolicy);
}

void malformed_graphs_and_requests_fail_before_scoring() {
    auto snapshot = graph();
    snapshot.edges.front().target_id = "missing";
    CHECK(validate(snapshot) == ContractRefusal::UnknownEdgeEndpoint);

    auto context_request = request();
    context_request.graph_snapshot_id = "another-snapshot";
    ContextBundle bundle;
    CHECK(DeterministicContextCompiler{}.compile(graph(), context_request, &bundle) ==
          ContractRefusal::SnapshotVersionMismatch);

    context_request = request();
    context_request.token_budget = 63;
    CHECK(validate(context_request) == ContractRefusal::InvalidTokenBudget);
    context_request = request();
    context_request.maximum_expansion_hops = 4;
    CHECK(validate(context_request) == ContractRefusal::InvalidExpansionDepth);
}

void corrections_and_models_carry_auditable_promotion_boundary() {
    CorrectionEvent correction{
        .correction_id = "correction-1",
        .kind = CorrectionKind::MissingContextLabel,
        .operator_id = "operator:nelson",
        .graph_snapshot_id = "snapshot-1",
        .query_digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        .raw_correction_ref = "vault:corrections/1",
        .rationale_ref = "knowledge:correction-rationales/1",
        .target_ids = {"simulation:motor-x"},
        .recorded_at_ms = 1'700'000'002'000,
    };
    CHECK(!validate(correction).has_value());
    correction.target_ids.clear();
    CHECK(validate(correction) == ContractRefusal::InvalidCorrection);
    correction.target_ids = {"simulation:motor-x"};
    correction.rationale_ref.clear();
    CHECK(validate(correction) == ContractRefusal::InvalidCorrection);

    correction.rationale_ref = "knowledge:correction-rationales/1";
    GraphCorrectionReceipt receipt{
        .correction_id = correction.correction_id,
        .source_graph_snapshot_id = correction.graph_snapshot_id,
        .resulting_graph_snapshot_id = "snapshot-2",
        .graph_update_ref = "knowledge:updates/1",
        .applied_at_ms = 1'700'000'002'001,
    };
    CorrectionTrajectory trajectory{
        .trajectory_id = "trajectory-1",
        .correction = correction,
        .graph_update = receipt,
        .selected_context_ids = {"project:robot-arm"},
        .omitted_candidate_ids = {"simulation:motor-x"},
        .response_ref = "vault:responses/1",
        .response_digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        .corrected_interpretation_ref = "knowledge:interpretations/1",
        .model_id = "deterministic-v1",
        .model_hash = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        .captured_at_ms = 1'700'000'002'002,
    };
    CHECK(!validate(trajectory).has_value());
    trajectory.graph_update.source_graph_snapshot_id = "different";
    CHECK(validate(trajectory) == ContractRefusal::InvalidCorrection);
    trajectory.graph_update.source_graph_snapshot_id = correction.graph_snapshot_id;
    trajectory.graph_update.resulting_graph_snapshot_id = correction.graph_snapshot_id;
    CHECK(validate(trajectory) == ContractRefusal::InvalidCorrection);

    ModelManifest candidate{
        .model_id = "archie-cortex-r2",
        .model_hash = "sha256:candidate",
        .training_dataset_version = "dataset-2",
        .graph_schema_version = "graph-v1",
        .backend = ModelBackend::OnnxRuntime,
        .architecture = "shared-temporal-graph-encoder",
        .rollback_model_id = "archie-cortex-r1",
        .promoted = false,
        .created_at_ms = 1'700'000'002'000,
        .promoted_at_ms = 1'700'000'003'000,
    };
    CHECK(validate(candidate) == ContractRefusal::UnpromotedModel);
    candidate.promoted = true;
    CHECK(!validate(candidate).has_value());
}

void local_ipc_is_typed_bounded_and_round_trips() {
    const auto snapshot = graph();
    const auto decoded_snapshot = ipc::decode_graph_snapshot(ipc::encode(snapshot));
    CHECK(decoded_snapshot.has_value());
    CHECK(decoded_snapshot->snapshot_id == snapshot.snapshot_id);
    CHECK(decoded_snapshot->nodes.size() == snapshot.nodes.size());

    const auto context_request = request();
    const auto decoded_request = ipc::decode_context_request(ipc::encode(context_request));
    CHECK(decoded_request.has_value());
    CHECK(decoded_request->question == context_request.question);
    CHECK(decoded_request->permitted_sensitivities == context_request.permitted_sensitivities);

    ContextBundle bundle;
    CHECK(!DeterministicContextCompiler{}.compile(snapshot, context_request, &bundle).has_value());
    const auto decoded_bundle = ipc::decode_context_response(ipc::encode(bundle));
    CHECK(decoded_bundle.has_value());
    CHECK(decoded_bundle->metrics.token_count == bundle.metrics.token_count);
    CHECK(decoded_bundle->facts.size() == bundle.facts.size());

    const auto refusal = ipc::decode_refusal(ipc::encode(ipc::Refusal{
        .code = ContractRefusal::SnapshotUnavailable}));
    CHECK(refusal.has_value());
    CHECK(refusal->code == ContractRefusal::SnapshotUnavailable);

    auto malformed = ipc::encode(context_request);
    malformed.front() = 99U;
    CHECK(!ipc::decode_context_request(malformed).has_value());
}

}  // namespace

int main() {
    graph_retrieval_is_bounded_provenanced_and_deterministic();
    strict_sensitivity_and_budget_are_hard_gates();
    malformed_graphs_and_requests_fail_before_scoring();
    corrections_and_models_carry_auditable_promotion_boundary();
    local_ipc_is_typed_bounded_and_round_trips();
    if (failures == 0) {
        std::cout << "ok — " << checks << " checks passed\n";
    }
    return failures == 0 ? EXIT_SUCCESS : EXIT_FAILURE;
}
