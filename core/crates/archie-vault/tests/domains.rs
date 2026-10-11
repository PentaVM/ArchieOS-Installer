use archie_types::{Locality, PrivacyGrade};
use archie_vault::{
    ContextDomain, ContextGateway, ContextRequest, EntryClass, Vault, VaultKey, VaultRef,
};

#[test]
fn a_mixed_domain_request_opens_nothing() {
    let mut vault = Vault::open(&VaultKey::from_bytes([4; 32])).unwrap();
    let work = VaultRef::parse("vault:work/studio/house/brief").unwrap();
    let personal = VaultRef::parse("vault:personal/owner/diary").unwrap();
    for reference in [&work, &personal] {
        vault
            .store(
                "test",
                reference.clone(),
                EntryClass::PrivateFact,
                PrivacyGrade::Restricted,
                "private",
                100,
                None,
            )
            .unwrap();
    }
    let before = vault.audit_log().len();
    let request = ContextRequest::new("task", "trace", "cad", Locality::Local, 1000)
        .in_domain(ContextDomain::work("studio", "house").unwrap())
        .including(work)
        .including(personal);
    assert!(ContextGateway::new()
        .assemble(&mut vault, &request, 101)
        .is_err());
    assert_eq!(
        vault.audit_log().len(),
        before,
        "no entry should have been opened"
    );
}

#[test]
fn a_domain_is_exact_and_survives_a_capsule_receipt() {
    let domain = ContextDomain::work("studio", "house").unwrap();
    assert!(domain.contains(&VaultRef::parse("vault:work/studio/house/brief").unwrap()));
    for path in [
        "work/studio/house2/brief",
        "work/other/house/brief",
        "personal/owner/brief",
        "work/studio/house",
    ] {
        assert!(!domain.contains(&VaultRef::from_path(path).unwrap()));
    }
    let mut vault = Vault::open(&VaultKey::from_bytes([4; 32])).unwrap();
    let request = ContextRequest::new("task", "trace", "cad", Locality::Local, 1000)
        .in_domain(domain.clone());
    let capsule = ContextGateway::new()
        .assemble(&mut vault, &request, 101)
        .unwrap();
    assert_eq!(capsule.receipt().domain, Some(domain));
}

#[test]
fn domains_cannot_escape_or_be_forged_by_deserialization() {
    for id in ["", "..", "a/b", "a b", "owner\n", "."] {
        assert!(ContextDomain::personal(id).is_err());
        assert!(ContextDomain::work("studio", id).is_err());
    }
    for value in [
        "work/studio",
        "personal/owner/extra",
        "work/studio/../personal",
        "public/all",
    ] {
        assert!(serde_json::from_str::<ContextDomain>(&format!("\"{value}\"")).is_err());
    }
}

#[test]
fn scoped_entries_require_a_domain_even_for_legacy_callers() {
    let mut vault = Vault::open(&VaultKey::from_bytes([4; 32])).unwrap();
    let request = ContextRequest::new("task", "trace", "cad", Locality::Local, 1000)
        .including(VaultRef::parse("vault:work/studio/house/brief").unwrap());
    assert!(ContextGateway::new()
        .assemble(&mut vault, &request, 101)
        .is_err());
    assert!(vault.audit_log().is_empty());
}

#[test]
fn renaming_a_personal_ciphertext_does_not_make_it_work_context() {
    let key = VaultKey::from_bytes([4; 32]);
    let mut vault = Vault::open(&key).unwrap();
    vault
        .store(
            "test",
            VaultRef::parse("vault:personal/owner/diary").unwrap(),
            EntryClass::PrivateFact,
            PrivacyGrade::Restricted,
            "private diary",
            100,
            None,
        )
        .unwrap();
    let work = VaultRef::parse("vault:work/studio/house/brief").unwrap();
    let mut entries = vault.sealed_entries();
    entries[0].reference = work.clone();
    let mut renamed = Vault::load(&key, entries).unwrap();
    let request = ContextRequest::new("task", "trace", "cad", Locality::Local, 1000)
        .in_domain(ContextDomain::work("studio", "house").unwrap())
        .including(work);
    assert!(ContextGateway::new()
        .assemble(&mut renamed, &request, 101)
        .is_err());
}
