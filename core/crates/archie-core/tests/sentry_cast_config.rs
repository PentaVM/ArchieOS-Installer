//! `[[sentry.chromecasts]]`: a Chromecast becomes an actuable route only with a
//! well-formed certificate pin. Validation-only; no device is contacted.
use archie_core::config::ArchieConfig;
use archie_sentry::contracts::CommandKind;

fn config(pin: &str, address: &str) -> String {
    format!(
        "[sentry]\n\
         [[sentry.chromecasts]]\n\
         resource_id='living-cast'\n\
         name='Living Room'\n\
         model='Chromecast'\n\
         address='{address}'\n\
         port=8009\n\
         certificate_sha256='{pin}'\n"
    )
}

#[test]
fn a_pinned_chromecast_is_an_actuable_cast_route() {
    let config = ArchieConfig::from_toml(&config(&"ab".repeat(32), "192.168.1.30")).unwrap();
    assert_eq!(
        config.sentry.actuable_route("living-cast"),
        Some((
            "cast",
            vec![
                CommandKind::AppLaunch,
                CommandKind::AppStop,
                CommandKind::VolumeSet,
                CommandKind::MuteSet
            ]
        ))
    );
    assert!(config
        .sentry
        .cast_adapter()
        .unwrap()
        .descriptor("living-cast")
        .is_some());
}

#[test]
fn a_missing_or_malformed_pin_or_a_public_address_is_refused() {
    for (pin, address) in [
        ("", "192.168.1.30"),
        (&*"AB".repeat(32), "192.168.1.30"),
        (&*"ab".repeat(31), "192.168.1.30"),
        (&*"ab".repeat(32), "8.8.8.8"),
    ] {
        assert!(
            ArchieConfig::from_toml(&config(pin, address)).is_err(),
            "{pin} {address}"
        );
    }
    let without_pin = "[sentry]\n[[sentry.chromecasts]]\nresource_id='c'\nname='n'\nmodel='m'\naddress='192.168.1.30'\nport=8009\n";
    assert!(ArchieConfig::from_toml(without_pin).is_err());
}
