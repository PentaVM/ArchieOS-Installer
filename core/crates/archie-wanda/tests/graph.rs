use archie_vault::ContextDomain;
use archie_wanda::{
    config::{DomainBinding, WandaConfig},
    graph::project,
    WandaError,
};
#[tokio::test]
async fn real_graph_authority_projects_tags_without_promoting_hypotheses() {
    let binary = std::env::var_os("WANDA_TEST_GRAPH_BINARY")
        .expect("run with the built archie-sentry binary");
    let dir = tempfile::tempdir().unwrap();
    let domain = ContextDomain::personal("owner").unwrap();
    let graph = dir.path().join("graph.tsv");
    std::fs::write(&graph,"v\t1\nowner\towner\nsubject\towner\towner\tOwner\nsubject\tana\tconsented_person\tAna\ntrait\tana\twanda.tag\tfamily\towner_declared\t-\tfixture\nhyp\towner\tana\tfriend\tmodel_hypothesis\t\tguess\n").unwrap();
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
    assert_eq!(projection.edges[0].standing, "hypothesis");
    assert!(matches!(
        project(&config, &ContextDomain::personal("other").unwrap()).await,
        Err(WandaError::UnknownDomain)
    ));
}
