//! Web search through the loop: gated like a tool call, and traced without its words.

use archie_cerebra::{Cerebra, CerebraConfig};
use archie_observability::{CognitiveTrace, Stage, StageOutcome, Subject};
use archie_router::Router;
use archie_search::{
    SearchAdmission, SearchAuthority, SearchProposal, SearchProviderManifest, SearchRegistry,
};
use archie_types::capability::well_known;
use archie_types::{CapabilitySet, PrivacyGrade, TraceId};
use archie_vault::{ContextGateway, VaultRef};

const T0: i64 = 1_760_000_000_000;
const PRIVATE_QUERY: &str = "the diagnosis written in my private notes";

fn cerebra() -> Cerebra {
    Cerebra::new(
        Router::default(),
        ContextGateway::new(),
        CerebraConfig::default(),
    )
}

fn providers() -> SearchRegistry {
    let mut registry = SearchRegistry::new();
    registry
        .register(SearchProviderManifest::tavily(
            VaultRef::parse("vault:tavily/main").unwrap(),
        ))
        .unwrap();
    registry
}

fn internet() -> SearchAuthority {
    SearchAuthority::new([well_known::network_internet()].into_iter().collect())
}

fn trace() -> CognitiveTrace {
    CognitiveTrace::new(TraceId::new(), "look something up", T0)
}

#[test]
fn a_public_search_proceeds_and_is_traced_at_select_capability() {
    let mut trace = trace();
    let proposal = SearchProposal::new(
        "tavily",
        "Nemotron speech streaming languages",
        PrivacyGrade::Public,
        "task-1",
        "trace-1",
    );
    let admission = cerebra()
        .propose_search(&mut trace, &providers(), &internet(), &proposal, T0 + 10)
        .unwrap();
    assert!(matches!(admission, SearchAdmission::Allowed(_)));

    let record = trace
        .last(Stage::SelectCapability)
        .expect("the search is recorded");
    assert_eq!(record.outcome, StageOutcome::Proceeded);
    assert_eq!(
        record.subject,
        Some(Subject::Tool {
            server: "tavily".into(),
            tool: "search".into()
        })
    );
    assert_eq!(record.privacy, Some(PrivacyGrade::Public));
}

#[test]
fn a_private_query_is_refused_and_the_trace_never_holds_its_words() {
    let mut trace = trace();
    let proposal = SearchProposal::new(
        "tavily",
        PRIVATE_QUERY,
        PrivacyGrade::Internal,
        "task-1",
        "trace-1",
    );
    let admission = cerebra()
        .propose_search(&mut trace, &providers(), &internet(), &proposal, T0 + 10)
        .unwrap();
    assert!(admission.refusal().is_some());

    let record = trace.last(Stage::SelectCapability).unwrap();
    assert_eq!(
        record.outcome,
        StageOutcome::Refused {
            reason: "query_exceeds_provider_grade".into()
        }
    );
    // The display learns that something internal was held back, and nothing else.
    assert_eq!(trace.hud().privacy, Some(PrivacyGrade::Internal));
    let written = serde_json::to_string(&trace).unwrap();
    assert!(!written.contains("diagnosis"), "{written}");
}

#[test]
fn a_loop_without_the_internet_grant_cannot_search() {
    let mut trace = trace();
    let proposal = SearchProposal::new(
        "tavily",
        "weather tomorrow",
        PrivacyGrade::Public,
        "task-1",
        "trace-1",
    );
    let admission = cerebra()
        .propose_search(
            &mut trace,
            &providers(),
            &SearchAuthority::new(CapabilitySet::new()),
            &proposal,
            T0 + 10,
        )
        .unwrap();
    assert!(admission.allowed().is_none());
    assert_eq!(
        trace.last(Stage::SelectCapability).unwrap().outcome,
        StageOutcome::Refused {
            reason: "capability_not_granted".into()
        }
    );
}
