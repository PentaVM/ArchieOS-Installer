//! Opt-in actual owner-managed account proof, never part of ordinary CI.
use archie_types::PrivacyGrade;
use archie_vault::ContextDomain;
use archie_workstation::codex::CodexClient;
use std::path::PathBuf;
use std::time::Duration;

#[tokio::test]
#[ignore = "requires installed official Codex with owner-managed authenticated account"]
async fn real_managed_subscription_completes_and_resumes_a_scoped_turn() {
    let executable = PathBuf::from(
        std::env::var_os("ARCHIE_CODEX_EXECUTABLE").expect("explicit official executable required"),
    );
    let parent = PathBuf::from(
        std::env::var_os("ARCHIE_CODEX_PROOF_DIR")
            .expect("explicit isolated working parent required"),
    );
    let dir = tempfile::Builder::new()
        .prefix("codex-rust-public-proof-")
        .tempdir_in(parent)
        .unwrap();
    let client =
        CodexClient::new(executable.as_os_str(), dir.path(), Duration::from_secs(60)).unwrap();
    let account = client.account().await.unwrap();
    assert!(account.account_id.is_some());
    let model =
        std::env::var("ARCHIE_CODEX_PROOF_MODEL").expect("explicit available model required");
    let domain = ContextDomain::personal("integration-proof").unwrap();
    let first = client
        .complete(
            &domain,
            PrivacyGrade::Public,
            &model,
            "Remember this public synthetic marker: ALPHA-41. Reply exactly ARCHIE_ROUTE_OK.",
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(first.text.trim(), "ARCHIE_ROUTE_OK");
    let second = client
        .complete(
            &domain,
            PrivacyGrade::Public,
            &model,
            "Reply only with the public synthetic marker from the previous turn.",
            Some(&first.session),
            None,
        )
        .await
        .unwrap();
    assert_eq!(second.text.trim(), "ALPHA-41");
    assert_eq!(first.session, second.session);
    let other = ContextDomain::work("other", "project").unwrap();
    assert!(client
        .complete(
            &other,
            PrivacyGrade::Public,
            &model,
            "Never sent",
            Some(&second.session),
            None
        )
        .await
        .is_err());
}
