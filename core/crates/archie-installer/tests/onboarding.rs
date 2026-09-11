use archie_installer::onboarding::Profile;
fn valid() -> serde_json::Value {
    serde_json::json!({
        "version":1,"hostname":"archie-node","username":"nelson",
        "owner":{"name":"Nelson","nickname":"Nel","address_as":"Nelson","ai_alias":"ARCHIE"},
        "location":null,"map":null,"perception":{}
    })
}
fn parse(v: serde_json::Value) -> Result<Profile, String> {
    Profile::parse(&serde_json::to_vec(&v).unwrap())
}
#[test]
fn roundtrip_preserves_unicode_preferences_without_credentials() {
    let mut v = valid();
    v["owner"]["address_as"] = serde_json::json!("Señor Nelson");
    let p = parse(v).unwrap();
    let saved = p.preferences_json().unwrap();
    assert_eq!(
        Profile::parse(saved.as_bytes()).unwrap().owner.address_as,
        "Señor Nelson"
    );
    assert!(!saved.contains("password"));
}
#[test]
fn unknown_fields_and_authentication_claims_are_refused() {
    for name in ["password", "fingerprint", "verified", "phone_approved"] {
        let mut v = valid();
        v[name] = serde_json::json!(true);
        assert!(parse(v).is_err());
    }
}
#[test]
fn consent_is_specific_and_does_not_claim_enrolment() {
    let mut v = valid();
    v["perception"] = serde_json::json!({"face_enrolment":true});
    assert!(parse(v).is_err());
    let mut v = valid();
    v["perception"] = serde_json::json!({"voice_enrolment":true,"camera":true});
    assert!(parse(v).is_err());
}
#[test]
fn location_requires_bounded_coordinates_and_provenance() {
    let mut v = valid();
    v["location"] = serde_json::json!({"latitude":-33.4,"longitude":-70.6,"source":"node","accuracy_m":null,"observed_at_ms":1});
    assert!(parse(v.clone()).is_ok());
    v["location"]["latitude"] = serde_json::json!(91);
    assert!(parse(v).is_err());
}
#[test]
fn invalid_names_versions_and_oversize_are_refused() {
    let mut v = valid();
    v["version"] = serde_json::json!(2);
    assert!(parse(v).is_err());
    let mut v = valid();
    v["owner"]["name"] = serde_json::json!("bad\nname");
    assert!(parse(v).is_err());
    assert!(Profile::parse(&vec![b' '; 65537]).is_err());
}
#[test]
fn map_paths_cannot_escape_or_inject_commands() {
    use archie_installer::mapdata::{safe_destination, safe_source};
    for path in [
        "/srv/../etc",
        "/srv/$(whoami)",
        "/srv/a;id",
        "/mnt/maps",
        "/var/tmp/maps",
        "/srv//maps",
    ] {
        assert!(!safe_destination(path), "{path}");
    }
    assert!(safe_destination("/srv/offline-maps"));
    assert!(safe_source(
        "https://download.geofabrik.de/south-america/chile-latest.osm.pbf"
    ));
    for url in [
        "https://evil.test/x.osm.pbf",
        "https://download.geofabrik.de/x;id.osm.pbf",
        "https://download.geofabrik.de/../x.osm.pbf",
    ] {
        assert!(!safe_source(url));
    }
}

#[test]
fn owner_preferences_reach_the_real_core_configuration_without_toml_injection() {
    let mut v = valid();
    v["owner"]["name"] = serde_json::json!("Nelson \"N\"");
    let p = parse(v).unwrap();
    let parsed = archie_core::config::ArchieConfig::from_toml(&p.owner_toml().unwrap()).unwrap();
    let owner = parsed.owner.unwrap();
    assert_eq!(owner.name, p.owner.name);
    assert_eq!(owner.ai_alias, "ARCHIE");
}
