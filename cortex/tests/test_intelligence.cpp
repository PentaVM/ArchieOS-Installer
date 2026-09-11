#include "archie/cortex/intelligence.hpp"
#include "archie/cortex/ipc.hpp"
#include <algorithm>
#include <cstdlib>
#include <iostream>
#include <fstream>
#include <limits>

using namespace archie::cortex;
namespace {
int failures = 0, checks = 0;
#define CHECK(x) do { ++checks; if (!(x)) { ++failures; std::cerr << __LINE__ << ": " #x "\n"; } } while(false)
GraphSnapshot chain() {
    GraphSnapshot g{"snapshot", 1000};
    for (int i = 0; i < 6; ++i) {
        GraphNode n; n.id = "node" + std::to_string(i); n.summary = i == 0 ? "actuator" : "distant evidence";
        n.provenance_ref = "source:fixture"; n.observed_at_ms = 100; n.version = 1;
        g.nodes.push_back(n);
        if (i > 0) {
            GraphEdge e; e.id = "edge" + std::to_string(i); e.source_id = g.nodes[i-1].id; e.target_id = n.id;
            e.provenance_ref = "source:fixture"; e.observed_at_ms = 100; g.edges.push_back(e);
        }
    }
    return g;
}
ContextRequest query() {
    ContextRequest q; q.request_id = "query"; q.question = "actuator"; q.graph_snapshot_id = "snapshot";
    q.permitted_sensitivities = {Sensitivity::LocalRestricted}; q.token_budget = 1024; q.requested_at_ms = 1000;
    return q;
}
ContextBundle compile(const GraphSnapshot& g, const ContextRequest& q) {
    ContextBundle b; CHECK(!DeterministicContextCompiler{}.compile(g, q, &b)); return b;
}
bool has(const ContextBundle& b, const std::string& id) {
    return std::any_of(b.facts.begin(), b.facts.end(), [&](const auto& n) { return n.node_id == id; });
}
void retrieval() {
    auto g = chain(); auto q = query();
    for (std::uint8_t hops = 0; hops <= 3; ++hops) {
        q.maximum_expansion_hops = hops;
        auto b = compile(g,q);
        CHECK(b.facts.size() == hops + 1U);
        CHECK(has(b,"node" + std::to_string(hops)));
        CHECK(!has(b,"node" + std::to_string(hops+1)));
        std::reverse(g.edges.begin(),g.edges.end());
        CHECK(b.render(q.question) == compile(g,q).render(q.question));
    }
    q.maximum_expansion_hops = 3;
    g.nodes[1].sensitivity = Sensitivity::Secret;
    CHECK(compile(g,q).facts.size() == 1);
    g = chain(); g.edges[0].observed_at_ms = 900; q.requested_at_ms = 500;
    ContextBundle rejected;
    CHECK(DeterministicContextCompiler{}.compile(g,q,&rejected) == ContractRefusal::InvalidTimestamp);
    g = chain(); g.nodes[1].observed_at_ms = 900;
    CHECK(DeterministicContextCompiler{}.compile(g,q,&rejected) == ContractRefusal::InvalidTimestamp);
    g.nodes[1].observed_at_ms = 1001;
    CHECK(validate(g) == ContractRefusal::InvalidTimestamp);
    g = chain(); q = query(); q.token_budget = 128;
    for (auto& n : g.nodes) n.provenance_ref = std::string(180,'s');
    auto b = compile(g,q);
    CHECK((b.render(q.question).size()+3)/4 <= q.token_budget);
    CHECK(b.metrics.token_count <= q.token_budget);
    q.question = std::string(2000,'a');
    CHECK(DeterministicContextCompiler{}.compile(g,q,&b) == ContractRefusal::InvalidTokenBudget);
    g = chain(); q = query(); g.edges[0].kind = EdgeKind::Contradicts;
    b = compile(g,q); CHECK(has(b,"node1")); CHECK(!b.uncertainties.empty());
    q.maximum_items = 1; b = compile(g,q); CHECK(b.facts.empty());
}
void prospection() {
    auto g=chain(); auto q=query(); g.edges.clear(); q.maximum_expansion_hops=0;
    g.nodes[4].summary="soldering"; g.nodes[5].summary="ventilation"; g.nodes[3].summary="thirdround";
    CHECK(!has(compile(g,q),"node5"));
    q.retrieval_cues={{"node0","soldering"},{"node4","ventilation"},{"node5","thirdround"}};
    auto b=compile(g,q); CHECK(has(b,"node4")); CHECK(has(b,"node5")); CHECK(!has(b,"node3"));
    CHECK(b.selector_version=="prospective-v1"); CHECK(b.metrics.token_count<=q.token_budget);
    CHECK(b.facts.size()==3); CHECK(b.relationships.empty());
    std::reverse(q.retrieval_cues.begin(),q.retrieval_cues.end()); std::reverse(g.nodes.begin(),g.nodes.end());
    CHECK(compile(g,q).render(q.question)==b.render(q.question));
    auto bytes=ipc::encode(q); CHECK(bytes.front()==3);
    auto decoded=ipc::decode_context_request(bytes); CHECK(decoded.has_value());
    if(decoded) CHECK(compile(g,*decoded).render(q.question)==b.render(q.question));
    bytes.pop_back(); CHECK(!ipc::decode_context_request(bytes));
    q.focus=FocusState{"focus:pgr",false,100,{{"node0",1,1000,true}}};
    bytes=ipc::encode(q); decoded=ipc::decode_context_request(bytes); CHECK(decoded && decoded->focus);
    q.focus.reset(); q.maximum_items=2; CHECK(!has(compile(g,q),"node5")); // grounding closure must fit
    q.maximum_items=24;
    for(auto& n:g.nodes) if(n.id=="node4") n.sensitivity=Sensitivity::Secret;
    CHECK(!has(compile(g,q),"node5")); // private grounding cannot bridge to public result
    q.retrieval_cues.push_back(q.retrieval_cues[0]); CHECK(validate(q).has_value());
    q.retrieval_cues.resize(9); CHECK(validate(q).has_value());
}
void focus() {
    auto g = chain(); auto q = query(); q.question = "unrelated"; q.maximum_expansion_hops = 0;
    FocusState f; f.focus_id = "focus:robot"; f.half_life_ms = 100; f.activations = {{"node4",1.0,1000,false}};
    q.focus = f;
    auto b = compile(g,q); CHECK(has(b,"node4")); CHECK(b.facts.size() == 1);
    auto encoded = ipc::encode(q); CHECK(encoded.front() == 2);
    auto decoded = ipc::decode_context_request(encoded); CHECK(decoded.has_value());
    if (decoded) CHECK(compile(g,*decoded).render(q.question) == b.render(q.question));
    encoded.pop_back(); CHECK(!ipc::decode_context_request(encoded));
    q.focus->suspended = true; CHECK(compile(g,q).facts.empty());
    q.focus->suspended = false; q.requested_at_ms = 5000;
    CHECK(compile(g,q).facts.empty());
    q.focus->activations[0].pinned = true; CHECK(has(compile(g,q),"node4"));
    g.nodes[4].sensitivity = Sensitivity::Secret; CHECK(compile(g,q).facts.empty());
    q.focus->activations[0].strength = std::numeric_limits<double>::quiet_NaN();
    CHECK(validate(q).has_value()); CHECK(!ipc::decode_context_request(ipc::encode(q)));
    q.focus.reset(); CHECK(ipc::encode(q).front() == 1);
}
void registry() {
    NeuronRegistry r;
    CapabilityDescriptor d; d.id="module:new-tool"; d.graph_node_id="process:new-tool";
    d.authority_boundary_ref="policy:module"; d.provenance_ref="manifest:new-tool";
    d.telemetry_fields={"latency_ms","task_success"}; d.capabilities={"simulation"};
    CHECK(r.upsert(d)); CHECK(r.neurons().at(d.id).heads.size() == 4);
    CHECK(!r.upsert(d)); ++d.version; CHECK(r.upsert(d));
    CHECK(r.attach_graph(chain())); CHECK(r.neurons().size() == 7);
    auto g = chain(); g.nodes.pop_back(); g.edges.pop_back();
    CHECK(r.attach_graph(g)); CHECK(r.neurons().size() == 6);
    auto bad = d; bad.id="bad"; bad.authority_boundary_ref.clear(); CHECK(!r.upsert(bad));
}
void outcomes() {
    std::vector<TrainingEvent> events;
    for (int i=0; i<6; ++i) {
        TrainingEvent e; e.event_id="event"+std::to_string(i); e.subject_id="process:new-tool";
        e.snapshot_id="snapshot"; e.evidence_ref="audit:outcome"; e.recorded_at_ms=100+i;
        e.signal=LearningSignal::TaskOutcome; e.observed.task_success=i!=0;
        e.observed.latency_ms=10; e.observed.memory_bytes=1000; events.push_back(e);
    }
    auto p=predict_process(events,"process:new-tool",104); CHECK(p.has_value());
    CHECK(p->sufficient_data); CHECK(*p->latency_ms==10); CHECK(*p->failure_probability==0.2);
    p=predict_process(events,"process:new-tool",103); CHECK(!p->sufficient_data); CHECK(!p->failure_probability);
    events[0].observed.task_success.reset(); CHECK(!valid(events[0]));
    events[0].signal=LearningSignal::ProcessSample; CHECK(valid(events[0]));
    p=predict_process(events,"process:new-tool",105); CHECK(*p->failure_probability==0);
    events.push_back(events.back()); CHECK(!predict_process(events,"process:new-tool",105));
}
void consolidation_and_projection() {
    auto g=chain(); auto q=query();
    auto groups=consolidation_candidates(g,q); CHECK(groups.size()==1); CHECK(groups[0].redundant_ids.size()==4);
    g.edges[1].kind=EdgeKind::Contradicts;
    groups=consolidation_candidates(g,q); CHECK(groups[0].redundant_ids.size()==2);
    CHECK(g.nodes.size()==6); // suggestions never destroy evidence
    auto historical=q; historical.requested_at_ms=g.captured_at_ms-1;
    CHECK(consolidation_candidates(g,historical).empty());
    auto b=compile(g,q);
    auto metrics=evaluate_context(b,{"node0","node1"}); CHECK(metrics.required_recall==1);
    b.facts[0].summary="<script> [[private]] #tag";
    auto md=cognitive_projection(b); CHECK(md.find("<script>")==std::string::npos);
    CHECK(md.find("[[private]]")==std::string::npos);
}
std::vector<std::uint8_t> fixture(std::string name) {
    std::ifstream stream(std::string(ARCHIE_CORTEX_FIXTURES) + "/" + name);
    std::string hex; stream >> hex;
    CHECK(!hex.empty());
    std::vector<std::uint8_t> bytes;
    for (std::size_t i=0;i+1<hex.size();i+=2) bytes.push_back(static_cast<std::uint8_t>(std::stoul(hex.substr(i,2),nullptr,16)));
    return bytes;
}
void cross_language_wire() {
    auto graph_bytes=fixture("cortex-graph-v1.hex");
    auto request_bytes=fixture("cortex-focus-request-v2.hex");
    auto graph=ipc::decode_graph_snapshot(graph_bytes); auto request=ipc::decode_context_request(request_bytes);
    CHECK(graph.has_value()); CHECK(request.has_value());
    if (graph && request) {
        CHECK(ipc::encode(*graph)==graph_bytes); CHECK(ipc::encode(*request)==request_bytes);
        CHECK(has(compile(*graph,*request),"node:archive"));
    }
    auto prospective_bytes=fixture("cortex-prospective-request-v3.hex");
    auto prospective=ipc::decode_context_request(prospective_bytes); CHECK(prospective.has_value());
    if(graph && prospective) {
        CHECK(ipc::encode(*prospective)==prospective_bytes);
        CHECK(has(compile(*graph,*prospective),"node:archive"));
    }
}
}
int main() {
    retrieval(); prospection(); focus(); registry(); outcomes(); consolidation_and_projection(); cross_language_wire();
    std::cout << checks << " checks, " << failures << " failures\n";
    return failures ? EXIT_FAILURE : EXIT_SUCCESS;
}
