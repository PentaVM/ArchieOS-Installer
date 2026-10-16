use archie_vault::ContextDomain;
use archie_wanda::{
    config::{DomainBinding, WandaConfig},
    graph::project,
    WandaError,
};
#[tokio::test]
#[ignore = "requires the separately built C++ authority; run with WANDA_TEST_GRAPH_BINARY and --ignored"]
async fn real_graph_authority_projects_tags_without_promoting_hypotheses() {
    let binary = std::env::var_os("WANDA_TEST_GRAPH_BINARY")
        .expect("run with the built archie-sentry binary");
    let dir = tempfile::tempdir().unwrap();
    let domain = ContextDomain::personal("owner").unwrap();
    let graph = dir.path().join("graph.tsv");
    std::fs::write(&graph,"v\t1\nowner\towner\nsubject\towner\towner\tOwner\nsubject\tana\tconsented_person\tAna\ntrait\tana\twanda.tag\tfamily\towner_declared\t-\tfixture\ntrait\tana\thealth\tasthma\towner_declared\tsensitive\tclinic-note\nhyp\towner\tana\tfriend\tmodel_hypothesis\t\tguess\n").unwrap();
    let config = WandaConfig {
        state_dir: dir.path().join("state"),
        graph_binary: binary.into(),
        models_dir: dir.path().join("models"),
        operator_token_file: dir.path().join("token"),
        domains: vec![DomainBinding {
            domain: domain.clone(),
            graph_path: graph,
        }],
        sources: vec![],
        quality_tier: 50,
        deep_quality_tier: 80,
        retention_hours: 168,
    };
    let projection = project(&config, &domain).await.unwrap();
    let ana = projection.nodes.iter().find(|p| p.id == "ana").unwrap();
    assert_eq!(ana.tags[0].value.as_deref(), Some("family"));
    assert!(ana.degree.is_none());
    // Wanda never asks for sensitive values: they reach neither the view nor the model.
    let health = ana.traits.iter().find(|t| t.key == "health").unwrap();
    assert!(health.redacted);
    assert_eq!(health.value, None);
    assert!(!serde_json::to_string(&projection)
        .unwrap()
        .contains("asthma"));
    assert!(!serde_json::to_string(&projection)
        .unwrap()
        .contains("clinic-note"));
    assert_eq!(projection.edges[0].standing, "hypothesis");
    assert!(matches!(
        project(&config, &ContextDomain::personal("other").unwrap()).await,
        Err(WandaError::UnknownDomain)
    ));
}

#[test]
fn a_large_graph_is_cut_down_from_the_edge_not_dropped() {
    use archie_wanda::{runtime::bounded_graph, types::*, GRAPH_CONTEXT_BYTES};
    let trait_view = |key: &str, value: String| TraitView {
        key: key.into(),
        value: Some(value),
        basis: "owner_declared".into(),
        sensitivity: "personal".into(),
        source_note: "fixture".into(),
        redacted: false,
    };
    let person = |i: usize| {
        let id = if i == 0 {
            "owner".to_string()
        } else {
            format!("p{i}")
        };
        let tags: Vec<_> = (0..4)
            .map(|t| trait_view("wanda.tag", format!("tag-{i}-{t}")))
            .collect();
        let mut traits: Vec<_> = (0..6)
            .map(|t| trait_view("likes", format!("something-{i}-{t}")))
            .collect();
        traits.extend(tags.clone());
        PersonView {
            id: id.clone(),
            label: id,
            kind: "consented_person".into(),
            degree: if i == 0 {
                Some(0)
            } else {
                Some(1 + (64 - i) / 16)
            },
            path_to_owner: None,
            tags,
            traits,
        }
    };
    let graph = PersonProjection {
        schema_version: 1,
        owner_id: "owner".into(),
        nodes: (0..64).rev().map(person).collect(),
        edges: vec![],
    };
    assert!(serde_json::to_string(&graph).unwrap().len() > GRAPH_CONTEXT_BYTES);
    let (kept, text) = bounded_graph(graph).unwrap();
    assert!(text.len() <= GRAPH_CONTEXT_BYTES);
    assert!(kept.nodes.len() > 1);
    assert_eq!(kept.nodes[0].id, "owner");
    assert!(kept
        .nodes
        .windows(2)
        .skip(1)
        .all(|pair| pair[0].degree <= pair[1].degree));
    assert!(kept
        .nodes
        .iter()
        .all(|n| n.traits.iter().all(|t| t.key != "wanda.tag")));
}
