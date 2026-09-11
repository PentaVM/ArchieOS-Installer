// Synthetic reproducible comparator, not measured user-answer quality or GPU throughput.
#include "archie/cortex/intelligence.hpp"
#include <algorithm>
#include <chrono>
#include <iostream>
#include <vector>
using namespace archie::cortex;
int main() {
    GraphSnapshot g; g.snapshot_id="benchmark"; g.captured_at_ms=1000;
    for (int i=0;i<512;++i) {
        GraphNode n; n.id="n"+std::to_string(i); n.summary=i==0?"actuator":"archived evidence";
        n.provenance_ref="source:synthetic"; n.observed_at_ms=100; g.nodes.push_back(n);
        if(i>0) { GraphEdge e; e.id="e"+std::to_string(i); e.source_id=g.nodes[i-1].id; e.target_id=n.id;
            e.provenance_ref="source:synthetic"; e.observed_at_ms=100; g.edges.push_back(e); }
    }
    ContextRequest q; q.request_id="bench"; q.question="actuator"; q.graph_snapshot_id=g.snapshot_id;
    q.token_budget=1024; q.maximum_items=24; q.maximum_expansion_hops=3; q.requested_at_ms=1000;
    q.permitted_sensitivities={Sensitivity::LocalRestricted};
    ContextBundle b; std::vector<double> times;
    for(int i=0;i<101;++i) {
        auto start=std::chrono::steady_clock::now();
        if(DeterministicContextCompiler{}.compile(g,q,&b)) return 1;
        if(i) times.push_back(std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-start).count());
    }
    std::sort(times.begin(),times.end());
    std::cout << "{\"fixture\":\"synthetic-chain-512\",\"selector\":\"" << b.selector_version
              << "\",\"candidates\":" << b.metrics.candidate_nodes << ",\"selected\":" << b.facts.size()
              << ",\"estimated_tokens\":" << b.metrics.token_count << ",\"render_bytes\":" << b.render(q.question).size()
              << ",\"p50_ms\":" << times[49] << ",\"p95_ms\":" << times[94] << ",\"p99_ms\":" << times[98] << "}\n";
    // Distant memories have no graph edge or lexical match to the original question.
    // Explicit anticipated needs are fixtures, not generated facts or trained skill.
    g.edges.clear(); g.nodes[300].summary="soldering"; g.nodes[400].summary="ventilation";
    q.maximum_expansion_hops=0;
    const std::vector<std::string> required={"n0","n300","n400"};
    for (int variant=0;variant<2;++variant) {
        if(variant) q.retrieval_cues={{"n0","soldering"},{"n300","ventilation"}};
        auto start=std::chrono::steady_clock::now();
        if(DeterministicContextCompiler{}.compile(g,q,&b)) return 1;
        const auto ms=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-start).count();
        const auto m=evaluate_context(b,required);
        std::cout << "{\"fixture\":\"synthetic-distant-needs-512\",\"selector\":\"" << b.selector_version
            << "\",\"recall\":" << m.required_recall << ",\"irrelevant_rate\":" << m.irrelevant_rate
            << ",\"estimated_tokens\":" << m.context_tokens << ",\"latency_ms\":" << ms << "}\n";
        if(variant && (m.required_recall!=1.0 || m.irrelevant_rate!=0.0)) return 1;
    }
}
