use archie_lan::spotify::parse;

#[test]
fn spotify_device_type_is_a_device_claim_and_never_an_owner() {
    let snapshot = parse(
        br#"{"status":101,"spotifyError":0,"deviceType":"Smartphone",
        "remoteName":"Phone","brandDisplayName":"Vendor","modelDisplayName":"Model",
        "activeUser":"private-person","deviceID":"private-id","publicKey":"secretish"}"#,
    )
    .unwrap();
    assert_eq!(snapshot.class.as_deref(), Some("mobile"));
    assert_eq!(snapshot.state["spotify.device_type"], "Smartphone");
    assert_eq!(snapshot.state.len(), 4);
    assert!(!format!("{:?}", snapshot).contains("private-person"));
    let computer = parse(br#"{"status":101,"spotifyError":0,"deviceType":"Computer"}"#).unwrap();
    assert_eq!(computer.class.as_deref(), Some("host"));
    let speaker = parse(br#"{"status":101,"spotifyError":0,"deviceType":"Speaker"}"#).unwrap();
    assert_eq!(speaker.class, None);
}

#[test]
fn failed_or_oversized_spotify_reports_are_not_identification() {
    assert!(parse(br#"{"status":102,"deviceType":"Smartphone"}"#).is_none());
    assert!(parse(br#"{"status":101,"spotifyError":1,"deviceType":"Smartphone"}"#).is_none());
    assert!(parse(b"not JSON").is_none());
    assert!(parse(&vec![b' '; 262_145]).is_none());
}
