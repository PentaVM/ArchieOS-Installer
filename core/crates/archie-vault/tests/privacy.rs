//! The privacy guarantees, asserted rather than asserted *about*.
//!
//! Each test here corresponds to a claim the system makes to its operator. They are
//! written against the crate's public surface only, because a guarantee that holds
//! only through private helpers is not a guarantee a caller can rely on.

use archie_types::{Locality, PrivacyGrade};
use archie_vault::{
    max_grade_for, ContextGateway, ContextRequest, Disclosure, Entity, EntityKind, EntryClass,
    GatewayError, PseudonymPolicy, Vault, VaultError, VaultKey, VaultRef,
};

const T0: i64 = 1_760_000_000_000;

fn vault() -> Vault {
    Vault::open(&VaultKey::from_bytes([7u8; 32])).unwrap()
}

fn r(path: &str) -> VaultRef {
    VaultRef::parse(path).unwrap()
}

fn store_secret(v: &mut Vault, path: &str, secret: &str) -> VaultRef {
    let reference = r(path);
    v.store(
        "test",
        reference.clone(),
        EntryClass::Credential,
        PrivacyGrade::Secret,
        secret,
        T0,
        None,
    )
    .unwrap();
    reference
}

fn store_note(v: &mut Vault, path: &str, grade: PrivacyGrade, text: &str) -> VaultRef {
    let reference = r(path);
    v.store(
        "test",
        reference.clone(),
        EntryClass::PrivateFact,
        grade,
        text,
        T0,
        None,
    )
    .unwrap();
    reference
}

#[test]
fn a_secret_never_comes_back_as_content() {
    let mut v = vault();
    let key = store_secret(&mut v, "vault:github/main", "ghp_realtokenvalue");

    // Asked for at every grade a caller could claim, including secret itself.
    for permitted in [
        PrivacyGrade::Public,
        PrivacyGrade::Internal,
        PrivacyGrade::Sensitive,
        PrivacyGrade::Restricted,
        PrivacyGrade::Secret,
    ] {
        let disclosure = v.disclose("test", &key, permitted, T0).unwrap();
        assert!(
            matches!(disclosure, Disclosure::CredentialReference { .. }),
            "{permitted} obtained content for a secret"
        );
        assert_eq!(disclosure.text(), None);
    }
}

#[test]
fn a_secret_never_reaches_a_capsule_even_a_local_one() {
    let mut v = vault();
    let key = store_secret(&mut v, "vault:github/main", "ghp_realtokenvalue");
    let gateway = ContextGateway::new();

    for destination in [Locality::Local, Locality::Fabric, Locality::Cloud] {
        let request =
            ContextRequest::new("task-1", "trace-1", "push a branch", destination, 60_000)
                .including(key.clone());
        let capsule = gateway.assemble(&mut v, &request, T0).unwrap();
        let rendered = capsule.render("task-1", T0).unwrap();

        assert!(
            !rendered.contains("ghp_realtokenvalue"),
            "secret leaked into a {destination:?} capsule"
        );
        // What the model is allowed to know: that the credential exists, by name.
        assert!(rendered.contains("credential_ref: vault:github/main"));
        assert_eq!(capsule.credential_references(), std::slice::from_ref(&key));
        assert!(capsule.entries().is_empty());
    }
}

#[test]
fn only_the_broker_yields_a_credential_and_only_for_a_credential() {
    let mut v = vault();
    let key = store_secret(&mut v, "vault:github/main", "ghp_realtokenvalue");
    let note = store_note(&mut v, "vault:notes/one", PrivacyGrade::Restricted, "hello");

    let brokered = v.broker_credential("executor", &key, T0).unwrap();
    assert_eq!(brokered.reference(), &key);
    // Debug is the accident that actually happens; it must not print the secret.
    assert!(!format!("{brokered:?}").contains("ghp_realtokenvalue"));
    assert_eq!(brokered.expose().as_str(), "ghp_realtokenvalue");

    // A note is not a credential, however much a caller would like it to be.
    assert_eq!(
        v.broker_credential("executor", &note, T0).unwrap_err(),
        VaultError::NotACredential
    );
}

#[test]
fn a_credential_cannot_be_stored_at_a_grade_that_would_let_it_travel() {
    let mut v = vault();
    for grade in [
        PrivacyGrade::Public,
        PrivacyGrade::Internal,
        PrivacyGrade::Sensitive,
        PrivacyGrade::Restricted,
    ] {
        assert_eq!(
            v.store(
                "test",
                r("vault:aws/key"),
                EntryClass::Credential,
                grade,
                "AKIAEXAMPLE",
                T0,
                None,
            )
            .unwrap_err(),
            VaultError::CredentialMustBeSecret,
            "a credential was accepted as {grade}"
        );
    }
    assert!(!v.contains(&r("vault:aws/key")));
}

#[test]
fn restricted_material_refuses_to_leave_the_machine() {
    let mut v = vault();
    let private = store_note(
        &mut v,
        "vault:notes/diagnosis",
        PrivacyGrade::Restricted,
        "the operator's medical history",
    );
    let gateway = ContextGateway::new();

    let cloud = ContextRequest::new("task-1", "trace-1", "summarise", Locality::Cloud, 60_000)
        .including(private.clone());
    match gateway.assemble(&mut v, &cloud, T0) {
        Err(GatewayError::Undisclosable { grade, .. }) => {
            assert_eq!(grade, PrivacyGrade::Restricted)
        }
        other => panic!("restricted material was cleared for the cloud: {other:?}"),
    }

    // The same material locally is fine — the rule is about destination, not secrecy
    // theatre.
    let local = ContextRequest::new("task-1", "trace-1", "summarise", Locality::Local, 60_000)
        .including(private);
    let capsule = gateway.assemble(&mut v, &local, T0).unwrap();
    assert!(capsule
        .render("task-1", T0)
        .unwrap()
        .contains("medical history"));
}

#[test]
fn one_undisclosable_reference_refuses_the_whole_capsule() {
    // A context with a silent hole in it is worse than no context: the model reports
    // confidence earned on material it never saw.
    let mut v = vault();
    let ok = store_note(
        &mut v,
        "vault:notes/ok",
        PrivacyGrade::Internal,
        "shareable",
    );
    let no = store_note(
        &mut v,
        "vault:notes/no",
        PrivacyGrade::Restricted,
        "private",
    );

    let request = ContextRequest::new("t", "tr", "summarise", Locality::Cloud, 60_000)
        .including(ok)
        .including(no);
    assert!(matches!(
        ContextGateway::new().assemble(&mut v, &request, T0),
        Err(GatewayError::Undisclosable { .. })
    ));
}

#[test]
fn entities_are_pseudonymised_on_the_way_out_and_restored_on_the_way_back() {
    let mut v = vault();
    let note = store_note(
        &mut v,
        "vault:notes/deal",
        PrivacyGrade::Internal,
        "John Smith, CEO of SecretCompany, signed contract ABC-2731.",
    );
    let gateway = ContextGateway::new()
        .knowing(Entity::new(EntityKind::Person, "John Smith"))
        .knowing(Entity::new(EntityKind::Company, "SecretCompany"))
        .knowing(Entity::new(EntityKind::Contract, "ABC-2731"));

    let request =
        ContextRequest::new("t", "tr", "summarise", Locality::Cloud, 60_000).including(note);
    let capsule = gateway.assemble(&mut v, &request, T0).unwrap();
    let rendered = capsule.render("t", T0).unwrap();

    for real in ["John Smith", "SecretCompany", "ABC-2731"] {
        assert!(
            !rendered.contains(real),
            "{real} reached the cloud verbatim"
        );
    }
    assert!(rendered.contains("PERSON_1"));
    assert!(rendered.contains("COMPANY_1"));
    assert!(rendered.contains("CONTRACT_1"));

    // The model answers in aliases; resolution happens locally.
    let restored = capsule
        .resolve("PERSON_1 should countersign CONTRACT_1.", T0)
        .unwrap();
    assert_eq!(restored, "John Smith should countersign ABC-2731.");
}

#[test]
fn a_longer_name_is_substituted_before_the_shorter_one_it_contains() {
    let mut v = vault();
    let note = store_note(
        &mut v,
        "vault:notes/acme",
        PrivacyGrade::Internal,
        "Acme Holdings is the parent of Acme.",
    );
    let gateway = ContextGateway::new()
        .knowing(Entity::new(EntityKind::Company, "Acme"))
        .knowing(Entity::new(EntityKind::Company, "Acme Holdings"));

    let request =
        ContextRequest::new("t", "tr", "summarise", Locality::Cloud, 60_000).including(note);
    let rendered = gateway
        .assemble(&mut v, &request, T0)
        .unwrap()
        .render("t", T0)
        .unwrap();

    assert!(
        !rendered.contains("Acme"),
        "shortest-first substitution left a real name behind: {rendered}"
    );
}

#[test]
fn local_context_is_not_pseudonymised_by_default() {
    // The machine holding the alias table is the machine running the model; the
    // substitution would cost accuracy and buy nothing.
    let mut v = vault();
    let note = store_note(
        &mut v,
        "vault:notes/deal",
        PrivacyGrade::Restricted,
        "John Smith signed.",
    );
    let gateway = ContextGateway::new().knowing(Entity::new(EntityKind::Person, "John Smith"));

    let request =
        ContextRequest::new("t", "tr", "summarise", Locality::Local, 60_000).including(note);
    let rendered = gateway
        .assemble(&mut v, &request, T0)
        .unwrap()
        .render("t", T0)
        .unwrap();
    assert!(rendered.contains("John Smith"));
}

#[test]
fn pseudonymisation_can_be_demanded_even_locally() {
    let mut v = vault();
    let note = store_note(
        &mut v,
        "vault:notes/deal",
        PrivacyGrade::Restricted,
        "John Smith signed.",
    );
    let gateway = ContextGateway::new().knowing(Entity::new(EntityKind::Person, "John Smith"));

    let mut request =
        ContextRequest::new("t", "tr", "summarise", Locality::Local, 60_000).including(note);
    request.pseudonymize = PseudonymPolicy::Always;
    let rendered = gateway
        .assemble(&mut v, &request, T0)
        .unwrap()
        .render("t", T0)
        .unwrap();
    assert!(!rendered.contains("John Smith"));
    assert!(rendered.contains("PERSON_1"));
}

#[test]
fn a_capsule_stops_existing_when_its_ttl_runs_out() {
    let mut v = vault();
    let note = store_note(&mut v, "vault:notes/one", PrivacyGrade::Internal, "content");
    let request =
        ContextRequest::new("t", "tr", "summarise", Locality::Cloud, 30_000).including(note);
    let capsule = ContextGateway::new()
        .assemble(&mut v, &request, T0)
        .unwrap();

    assert!(capsule.render("t", T0 + 29_999).is_ok());
    assert!(matches!(
        capsule.render("t", T0 + 30_000),
        Err(GatewayError::Expired { .. })
    ));
    assert!(matches!(
        capsule.resolve("PERSON_1", T0 + 30_000),
        Err(GatewayError::Expired { .. })
    ));
}

#[test]
fn a_capsule_is_context_for_the_task_it_was_built_for() {
    let mut v = vault();
    let note = store_note(&mut v, "vault:notes/one", PrivacyGrade::Internal, "content");
    let request =
        ContextRequest::new("task-a", "tr", "summarise", Locality::Cloud, 60_000).including(note);
    let capsule = ContextGateway::new()
        .assemble(&mut v, &request, T0)
        .unwrap();

    assert!(capsule.render("task-a", T0).is_ok());
    assert!(matches!(
        capsule.render("task-b", T0),
        Err(GatewayError::WrongTask { .. })
    ));
}

#[test]
fn disposal_empties_the_capsule_and_cannot_be_undone() {
    let mut v = vault();
    let note = store_note(&mut v, "vault:notes/one", PrivacyGrade::Internal, "content");
    let request =
        ContextRequest::new("t", "tr", "summarise", Locality::Cloud, 60_000).including(note);
    let mut capsule = ContextGateway::new()
        .assemble(&mut v, &request, T0)
        .unwrap();

    capsule.dispose();
    capsule.dispose(); // idempotent
    assert!(capsule.is_disposed());
    assert!(matches!(
        capsule.render("t", T0),
        Err(GatewayError::Disposed)
    ));
}

#[test]
fn the_receipt_carries_shape_and_never_content() {
    let mut v = vault();
    let note = store_note(
        &mut v,
        "vault:notes/deal",
        PrivacyGrade::Internal,
        "John Smith signed contract ABC-2731.",
    );
    let key = store_secret(&mut v, "vault:github/main", "ghp_realtokenvalue");
    let gateway = ContextGateway::new()
        .knowing(Entity::new(EntityKind::Person, "John Smith"))
        .knowing(Entity::new(EntityKind::Contract, "ABC-2731"));

    let request = ContextRequest::new("t", "tr", "summarise", Locality::Cloud, 60_000)
        .including(note)
        .including(key);
    let capsule = gateway.assemble(&mut v, &request, T0).unwrap();

    let receipt = serde_json::to_string(&capsule.receipt()).unwrap();
    for forbidden in [
        "John Smith",
        "ABC-2731",
        "ghp_realtokenvalue",
        "signed contract",
    ] {
        assert!(
            !receipt.contains(forbidden),
            "receipt disclosed {forbidden}: {receipt}"
        );
    }
    // It still says enough to audit the disclosure.
    assert!(receipt.contains("vault:notes/deal"));
    assert!(receipt.contains("vault:github/main"));
    assert!(receipt.contains("\"aliases_issued\":2"));

    // And `{:?}` on the capsule itself is equally quiet.
    let debug = format!("{capsule:?}");
    assert!(!debug.contains("John Smith") && !debug.contains("ghp_realtokenvalue"));
}

#[test]
fn every_access_is_recorded_and_refusals_are_recorded_as_refusals() {
    let mut v = vault();
    let key = store_secret(&mut v, "vault:github/main", "ghp_realtokenvalue");
    let _ = v.disclose("assembler", &key, PrivacyGrade::Internal, T0);
    let _ = v.broker_credential("executor", &key, T0);
    let _ = v.disclose(
        "assembler",
        &r("vault:missing/one"),
        PrivacyGrade::Public,
        T0,
    );

    let log = v.audit_log();
    assert_eq!(log.len(), 4, "store + disclose + broker + miss");

    let attempted_disclosure = &log[1];
    assert!(!attempted_disclosure.granted);
    assert_eq!(
        attempted_disclosure.refusal,
        Some(archie_vault::refusals::SECRET_NOT_DISCLOSED_AS_CONTENT)
    );

    assert!(log[2].granted, "the broker is the legitimate route");
    assert_eq!(log[3].refusal, Some(archie_vault::refusals::NOT_FOUND));

    // Nothing in the log is the thing itself.
    let rendered = serde_json::to_string(log).unwrap();
    assert!(!rendered.contains("ghp_realtokenvalue"));
}

#[test]
fn an_expired_entry_is_gone_even_before_it_is_evicted() {
    let mut v = vault();
    let reference = r("vault:tokens/session");
    v.store(
        "test",
        reference.clone(),
        EntryClass::Credential,
        PrivacyGrade::Secret,
        "st_shortlived",
        T0,
        Some(T0 + 1_000),
    )
    .unwrap();

    assert!(v
        .broker_credential("executor", &reference, T0 + 999)
        .is_ok());
    assert_eq!(
        v.broker_credential("executor", &reference, T0 + 1_000)
            .unwrap_err(),
        VaultError::Expired
    );
    assert_eq!(v.evict_expired("janitor", T0 + 2_000), 1);
    assert!(!v.contains(&reference));
}

#[test]
fn sealed_entries_carry_no_readable_content_and_survive_a_reload() {
    let mut v = vault();
    let note = store_note(
        &mut v,
        "vault:notes/one",
        PrivacyGrade::Internal,
        "a private sentence",
    );
    let sealed = v.sealed_entries();

    let serialised = serde_json::to_string(&sealed).unwrap();
    assert!(!serialised.contains("a private sentence"));

    let mut reopened = Vault::load(&VaultKey::from_bytes([7u8; 32]), sealed).unwrap();
    let text = reopened
        .disclose("test", &note, PrivacyGrade::Internal, T0)
        .unwrap();
    assert_eq!(text.text(), Some("a private sentence"));
}

#[test]
fn a_rewritten_grade_does_not_open_the_entry() {
    // The grade is authenticated data, so downgrading the stored record to something
    // permissive destroys the record rather than liberating it.
    let mut v = vault();
    let reference = store_note(
        &mut v,
        "vault:notes/one",
        PrivacyGrade::Restricted,
        "a private sentence",
    );

    let mut sealed = v.sealed_entries();
    sealed[0].grade = PrivacyGrade::Public;

    let mut tampered = Vault::load(&VaultKey::from_bytes([7u8; 32]), sealed).unwrap();
    assert_eq!(
        tampered
            .disclose("attacker", &reference, PrivacyGrade::Public, T0)
            .unwrap_err(),
        VaultError::Crypto
    );
}

#[test]
fn another_key_opens_nothing() {
    let mut v = vault();
    let note = store_note(
        &mut v,
        "vault:notes/one",
        PrivacyGrade::Internal,
        "secret-ish",
    );
    let mut other = Vault::load(&VaultKey::from_bytes([9u8; 32]), v.sealed_entries()).unwrap();
    assert_eq!(
        other
            .disclose("attacker", &note, PrivacyGrade::Internal, T0)
            .unwrap_err(),
        VaultError::Crypto
    );
}

#[test]
fn the_destination_decides_the_ceiling() {
    assert_eq!(max_grade_for(Locality::Cloud), PrivacyGrade::Internal);
    assert_eq!(max_grade_for(Locality::Fabric), PrivacyGrade::Sensitive);
    assert_eq!(max_grade_for(Locality::Local), PrivacyGrade::Restricted);
    // And no destination reaches secret, which is the invariant the rest depends on.
    for locality in [Locality::Local, Locality::Fabric, Locality::Cloud] {
        assert!(max_grade_for(locality) < PrivacyGrade::Secret);
    }
}
