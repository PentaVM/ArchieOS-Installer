//! What the gate refuses, and what it lets through.
//!
//! The claim under test is the one the crate exists for: there is no route from a
//! model's proposed tool call to a transport that does not pass through here.

use archie_mcp::{
    Admission, CallerAuthority, McpGate, McpRegistry, McpServerManifest, McpTransport, Refusal,
    ToolAllowlist, ToolProposal, MAX_ARGUMENT_BYTES,
};
use archie_types::{Capability, CapabilitySet, PrivacyGrade};
use archie_vault::VaultRef;
use serde_json::json;

fn server(id: &str) -> McpServerManifest {
    McpServerManifest {
        id: id.into(),
        display_name: id.into(),
        transport: McpTransport::StreamableHttp {
            url: "https://api.example.com/mcp".into(),
        },
        secret_handle: Some(VaultRef::parse("vault:github/main").unwrap()),
        capabilities: CapabilitySet::new(),
        tools: ToolAllowlist::named(["create_issue", "list_issues"]),
        max_privacy_grade: PrivacyGrade::Internal,
        timeout_ms: 30_000,
        max_response_bytes: 1_048_576,
        enabled: true,
    }
}

fn registry_with(manifest: McpServerManifest) -> McpRegistry {
    let mut registry = McpRegistry::new();
    registry.register(manifest).unwrap();
    registry
}

fn proposal(server_id: &str, tool: &str) -> ToolProposal {
    ToolProposal::new(
        server_id,
        tool,
        json!({"title": "a bug"}),
        PrivacyGrade::Internal,
        "task-1",
        "trace-1",
    )
}

fn authority() -> CallerAuthority {
    CallerAuthority::new(CapabilitySet::new())
}

#[test]
fn an_allowlisted_tool_on_an_enabled_server_is_admitted() {
    let registry = registry_with(server("github"));
    let admission =
        McpGate::authorize(&registry, &authority(), &proposal("github", "create_issue"));

    let call = admission.allowed().expect("should be admitted");
    assert_eq!(call.tool(), "create_issue");
    assert_eq!(call.task_id(), "task-1");
    // The transport is told the credential's name and nothing more.
    assert_eq!(
        call.credential_ref().unwrap().to_string(),
        "vault:github/main"
    );
    assert_eq!(call.timeout_ms(), 30_000);
}

#[test]
fn a_tool_the_operator_did_not_list_is_refused_however_real_it_is() {
    // The central case: the server genuinely offers `delete_repository`, and the model
    // genuinely asked for it. Neither fact grants it.
    let registry = registry_with(server("github"));
    let admission = McpGate::authorize(
        &registry,
        &authority(),
        &proposal("github", "delete_repository"),
    );
    assert_eq!(
        admission.refusal().unwrap(),
        &Refusal::ToolNotAllowed {
            server_id: "github".into(),
            tool: "delete_repository".into()
        }
    );
}

#[test]
fn a_server_advertising_more_than_it_was_granted_gains_nothing() {
    // `tools/list` is the server describing itself. The intersection is what runs.
    let manifest = server("github");
    let advertised = ["create_issue", "delete_repository", "transfer_ownership"];
    assert_eq!(
        McpGate::permitted_tools(&manifest, advertised),
        vec!["create_issue".to_string()]
    );
}

#[test]
fn a_disabled_server_is_refused_distinctly_from_an_absent_one() {
    let mut manifest = server("github");
    manifest.enabled = false;
    let registry = registry_with(manifest);

    assert_eq!(
        McpGate::authorize(&registry, &authority(), &proposal("github", "create_issue"))
            .refusal()
            .unwrap(),
        &Refusal::ServerDisabled("github".into())
    );
    assert_eq!(
        McpGate::authorize(&registry, &authority(), &proposal("gitlab", "create_issue"))
            .refusal()
            .unwrap(),
        &Refusal::UnknownServer("gitlab".into())
    );
}

#[test]
fn a_caller_without_the_capability_cannot_reach_the_server() {
    let mut manifest = server("github");
    let mut needed = CapabilitySet::new();
    needed.insert(Capability::known("tool.vcs"));
    manifest.capabilities = needed;
    let registry = registry_with(manifest);

    // Ungranted.
    assert!(matches!(
        McpGate::authorize(&registry, &authority(), &proposal("github", "create_issue")).refusal(),
        Some(Refusal::CapabilityNotGranted(_))
    ));

    // Granted.
    let mut granted = CapabilitySet::new();
    granted.insert(Capability::known("tool.vcs"));
    let admission = McpGate::authorize(
        &registry,
        &CallerAuthority::new(granted),
        &proposal("github", "create_issue"),
    );
    assert!(admission.allowed().is_some());
}

#[test]
fn secret_material_is_never_admitted_as_an_argument() {
    let registry = registry_with(server("github"));
    let mut p = proposal("github", "create_issue");
    p.argument_grade = PrivacyGrade::Secret;

    assert_eq!(
        McpGate::authorize(&registry, &authority(), &p)
            .refusal()
            .unwrap(),
        &Refusal::SecretInArguments
    );
}

#[test]
fn arguments_above_the_servers_ceiling_are_refused() {
    let registry = registry_with(server("github"));
    let mut p = proposal("github", "create_issue");
    p.argument_grade = PrivacyGrade::Sensitive;

    assert!(matches!(
        McpGate::authorize(&registry, &authority(), &p).refusal(),
        Some(Refusal::PrivacyExceeded { .. })
    ));
}

#[test]
fn a_remote_server_cannot_receive_restricted_arguments_even_if_cleared() {
    // Two gates again: the operator's ceiling and the locality. A manifest cannot
    // declare its way past the second, and validation catches the attempt at
    // registration rather than at call time.
    let mut manifest = server("github");
    manifest.max_privacy_grade = PrivacyGrade::Restricted;
    let mut registry = McpRegistry::new();
    assert!(registry.register(manifest).is_err());
}

#[test]
fn an_oversized_argument_payload_is_refused_before_anything_is_sent() {
    let registry = registry_with(server("github"));
    let mut p = proposal("github", "create_issue");
    p.arguments = json!({"body": "x".repeat(MAX_ARGUMENT_BYTES)});

    assert!(matches!(
        McpGate::authorize(&registry, &authority(), &p).refusal(),
        Some(Refusal::ArgumentsTooLarge { .. })
    ));
}

#[test]
fn arguments_must_be_an_object() {
    let registry = registry_with(server("github"));
    let mut p = proposal("github", "create_issue");
    p.arguments = json!("just a string");

    assert_eq!(
        McpGate::authorize(&registry, &authority(), &p)
            .refusal()
            .unwrap(),
        &Refusal::ArgumentsNotAnObject
    );
}

#[test]
fn a_tool_marked_consequential_is_admitted_but_held_for_a_person() {
    let registry = registry_with(server("github"));
    let authority = authority().requiring_approval("github", "create_issue");

    match McpGate::authorize(&registry, &authority, &proposal("github", "create_issue")) {
        Admission::NeedsApproval { call, .. } => {
            // It is a real, fully-formed call — the gate is not refusing it, it is
            // declining to run it unasked.
            assert_eq!(call.tool(), "create_issue");
        }
        other => panic!("expected an approval hold, got {other:?}"),
    }

    // And a different tool on the same server still goes straight through.
    assert!(
        McpGate::authorize(&registry, &authority, &proposal("github", "list_issues"))
            .allowed()
            .is_some()
    );
}

#[test]
fn the_receipt_describes_the_call_without_reproducing_it() {
    let registry = registry_with(server("github"));
    let mut p = proposal("github", "create_issue");
    p.arguments = json!({"title": "the operator's private bug title"});

    let call = McpGate::authorize(&registry, &authority(), &p)
        .allowed()
        .unwrap()
        .clone();
    let receipt = serde_json::to_string(&call.receipt()).unwrap();

    assert!(!receipt.contains("private bug title"));
    assert!(receipt.contains("create_issue"));
    assert!(receipt.contains("trace-1"));
    assert!(receipt.contains("argument_bytes"));
}

#[test]
fn a_server_granted_nothing_can_be_registered_and_reached_and_still_do_nothing() {
    // The correct state for a server that has been added but not reviewed.
    let mut manifest = server("github");
    manifest.tools = ToolAllowlist::None;
    let registry = registry_with(manifest);

    assert!(
        McpGate::authorize(&registry, &authority(), &proposal("github", "create_issue"))
            .refusal()
            .is_some()
    );
}
