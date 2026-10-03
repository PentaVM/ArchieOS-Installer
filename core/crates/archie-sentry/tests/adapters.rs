use archie_sentry::{
    adapters::{home_assistant::HomeAssistant, ipp::*},
    contracts::*,
    engine::*,
    registry::*,
};
use archie_types::Timestamp;
use std::collections::BTreeMap;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn server(
    responses: Vec<(&'static str, Vec<u8>)>,
) -> (String, tokio::task::JoinHandle<Vec<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut requests = vec![];
        for (headers, body) in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![];
            let mut chunk = [0; 1024];
            loop {
                let size = stream.read(&mut chunk).await.unwrap();
                if size == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..size]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let text = String::from_utf8_lossy(&request[..end]);
                    let length = text
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(request);
            let response = format!(
                "HTTP/1.1 {headers}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
        }
        requests
    });
    (format!("http://{address}"), task)
}
fn resource() -> ResourceDescriptor {
    ResourceDescriptor {
        resource_id: "lamp".into(),
        edge_id: "edge".into(),
        adapter_id: "ha".into(),
        paired: true,
        capabilities: vec![CommandKind::SwitchSet],
    }
}
fn ha(base: &str) -> Result<HomeAssistant, AdapterError> {
    HomeAssistant::new(
        "edge",
        "ha",
        base,
        "fixture-token",
        BTreeMap::from([("lamp".into(), "switch.office".into())]),
    )
}

#[tokio::test]
async fn identical_ha_report_keeps_evidence_identity_instead_of_inventing_a_measurement() {
    let stamp = Timestamp::now();
    let body = serde_json::to_vec(
        &serde_json::json!({"entity_id":"switch.office","state":"on",
        "attributes":{},"last_reported":stamp.to_rfc3339()}),
    )
    .unwrap();
    let (url, task) = server(vec![
        ("200 OK\r\nContent-Type: application/json", b"[]".to_vec()),
        ("200 OK\r\nContent-Type: application/json", body.clone()),
        ("200 OK\r\nContent-Type: application/json", b"[]".to_vec()),
        ("200 OK\r\nContent-Type: application/json", body),
    ])
    .await;
    let mut adapter = ha(&url).unwrap();
    let before = adapter.observe(&resource()).await.unwrap();
    let after = adapter.observe(&resource()).await.unwrap();
    assert_eq!(before.evidence_id, after.evidence_id);
    assert_eq!(before.observed_at, after.observed_at);
    task.await.unwrap();
}

#[tokio::test]
async fn climate_uses_ha_config_units_without_invented_entity_attribute() {
    let stamp = Timestamp::now();
    let body = serde_json::to_vec(
        &serde_json::json!({"entity_id":"climate.office","state":"heat",
        "attributes":{"temperature":21.5},"last_reported":stamp.to_rfc3339()}),
    )
    .unwrap();
    let (url, task) = server(vec![
        ("200 OK\r\nContent-Type: application/json", b"[]".to_vec()),
        ("200 OK\r\nContent-Type: application/json", body),
        (
            "200 OK\r\nContent-Type: application/json",
            serde_json::to_vec(&serde_json::json!({"unit_system":{"temperature":"°C"}})).unwrap(),
        ),
    ])
    .await;
    let mut adapter = HomeAssistant::new(
        "edge",
        "ha",
        &url,
        "fixture-token",
        BTreeMap::from([("lamp".into(), "climate.office".into())]),
    )
    .unwrap();
    let report = adapter.observe(&resource()).await.unwrap();
    assert_eq!(
        report.state,
        ResourceState::Climate {
            target_milli_celsius: 21500
        }
    );
    let requests = task.await.unwrap();
    assert!(String::from_utf8_lossy(&requests[2]).starts_with("GET /api/config "));
}

#[tokio::test]
async fn climate_rechecks_units_after_observation_and_refuses_changed_config() {
    let stamp = Timestamp::now();
    let state = serde_json::to_vec(
        &serde_json::json!({"entity_id":"climate.office","state":"heat",
        "attributes":{"temperature":21.5},"last_reported":stamp.to_rfc3339()}),
    )
    .unwrap();
    let (url, task) = server(vec![
        ("200 OK\r\nContent-Type: application/json", b"[]".to_vec()),
        ("200 OK\r\nContent-Type: application/json", state),
        (
            "200 OK\r\nContent-Type: application/json",
            serde_json::to_vec(&serde_json::json!({"unit_system":{"temperature":"°C"}})).unwrap(),
        ),
        (
            "200 OK\r\nContent-Type: application/json",
            serde_json::to_vec(&serde_json::json!({"unit_system":{"temperature":"°F"}})).unwrap(),
        ),
    ])
    .await;
    let mut adapter = HomeAssistant::new(
        "edge",
        "ha",
        &url,
        "fixture-token",
        BTreeMap::from([("lamp".into(), "climate.office".into())]),
    )
    .unwrap();
    let mut resource = resource();
    resource.capabilities = vec![CommandKind::ClimateSetTemperature];
    adapter.observe(&resource).await.unwrap();
    assert_eq!(
        adapter
            .apply(
                &resource,
                &Command::ClimateSetTemperature {
                    milli_celsius: 22500
                }
            )
            .await,
        Err(AdapterError::Unsupported)
    );
    let requests = task.await.unwrap();
    assert!(String::from_utf8_lossy(&requests[3]).starts_with("GET /api/config "));
    assert!(!requests
        .iter()
        .any(|r| String::from_utf8_lossy(r).starts_with("POST /api/services/climate/")));
}

#[tokio::test]
async fn climate_sends_celsius_only_after_checking_system_units() {
    let (url, task) = server(vec![
        (
            "200 OK\r\nContent-Type: application/json",
            serde_json::to_vec(&serde_json::json!({"unit_system":{"temperature":"°C"}})).unwrap(),
        ),
        ("200 OK\r\nContent-Type: application/json", b"[]".to_vec()),
    ])
    .await;
    let mut adapter = HomeAssistant::new(
        "edge",
        "ha",
        &url,
        "fixture-token",
        BTreeMap::from([("lamp".into(), "climate.office".into())]),
    )
    .unwrap();
    let mut resource = resource();
    resource.capabilities = vec![CommandKind::ClimateSetTemperature];
    adapter
        .apply(
            &resource,
            &Command::ClimateSetTemperature {
                milli_celsius: 22500,
            },
        )
        .await
        .unwrap();
    let requests = task.await.unwrap();
    let text = String::from_utf8_lossy(&requests[1]);
    assert!(text.starts_with("POST /api/services/climate/set_temperature "));
    assert!(text.contains("\"temperature\":22.5"));
}
#[tokio::test]
async fn home_assistant_refreshes_entity_and_uses_report_timestamp() {
    let stamp = Timestamp::now();
    let body = serde_json::to_vec(
        &serde_json::json!({"entity_id":"switch.office","state":"off",
        "attributes":{},"last_reported":stamp.to_rfc3339()}),
    )
    .unwrap();
    let (url, task) = server(vec![
        ("200 OK\r\nContent-Type: application/json", b"[]".to_vec()),
        ("200 OK\r\nContent-Type: application/json", body),
    ])
    .await;
    let mut adapter = ha(&url).unwrap();
    let observation = adapter.observe(&resource()).await.unwrap();
    assert_eq!(observation.state, ResourceState::Switch { on: false });
    assert_eq!(observation.observed_at, stamp);
    let requests = task.await.unwrap();
    assert!(String::from_utf8_lossy(&requests[0])
        .starts_with("POST /api/services/homeassistant/update_entity "));
    assert!(String::from_utf8_lossy(&requests[1]).starts_with("GET /api/states/switch.office "));
}
#[tokio::test]
async fn home_assistant_mutation_calls_service_and_never_writes_synthetic_state() {
    let (url, task) = server(vec![(
        "200 OK\r\nContent-Type: application/json",
        b"[]".to_vec(),
    )])
    .await;
    let mut adapter = ha(&url).unwrap();
    assert!(
        adapter
            .apply(&resource(), &Command::SwitchSet { on: true })
            .await
            .unwrap()
            .accepted
    );
    let requests = task.await.unwrap();
    let text = String::from_utf8_lossy(&requests[0]);
    assert!(text.starts_with("POST /api/services/switch/turn_on "));
    assert!(text.contains("{\"entity_id\":\"switch.office\"}"));
}
#[test]
fn credentialed_adapter_rejects_public_endpoints_cleartext_lan_and_url_injection() {
    for url in [
        "https://example.com",
        "http://192.168.1.10",
        "http://user:pass@127.0.0.1",
        "http://127.0.0.1/?token=a",
        "http://127.0.0.1/api/other",
    ] {
        assert!(ha(url).is_err(), "{url}");
    }
    let bindings = BTreeMap::from([("lamp".into(), "switch.office/../../api/states".into())]);
    assert!(
        HomeAssistant::new("edge", "ha", "http://127.0.0.1", "fixture-token", bindings).is_err()
    );
}
#[tokio::test]
async fn redirect_and_oversized_response_are_rejected() {
    for headers in [
        "302 Found\r\nLocation: http://127.0.0.1:9/leak",
        "200 OK\r\nContent-Type: application/json",
    ] {
        let oversized = headers.starts_with("200");
        let body = if oversized {
            vec![b'x'; 1048577]
        } else {
            vec![]
        };
        let (url, task) = server(vec![(headers, body)]).await;
        let mut adapter = ha(&url).unwrap();
        assert!(adapter.observe(&resource()).await.is_err());
        task.await.unwrap();
    }
}
fn ipp_response(request_id: u32) -> Vec<u8> {
    let mut body = vec![1, 1, 0, 0];
    body.extend_from_slice(&request_id.to_be_bytes());
    body.push(4);
    fn attribute(body: &mut Vec<u8>, tag: u8, name: &str, value: &[u8]) {
        body.push(tag);
        body.extend_from_slice(&(name.len() as u16).to_be_bytes());
        body.extend_from_slice(name.as_bytes());
        body.extend_from_slice(&(value.len() as u16).to_be_bytes());
        body.extend_from_slice(value);
    }
    attribute(&mut body, 0x42, "printer-name", b"Brother DCP-T720DW");
    attribute(&mut body, 0x23, "printer-state", &3i32.to_be_bytes());
    attribute(&mut body, 0x22, "printer-is-accepting-jobs", &[1]);
    attribute(
        &mut body,
        0x45,
        "printer-uuid",
        b"urn:uuid:e3248000-80ce-11db-8000-202b20a4fe3b",
    );
    body.push(3);
    body
}
#[test]
fn ipp_codec_preserves_request_identity_and_rejects_truncation() {
    let request = encode_get_printer_attributes("ipp://192.168.1.232:631/ipp/print", 42).unwrap();
    assert_eq!(&request[..8], &[1, 1, 0, 11, 0, 0, 0, 42]);
    assert_eq!(request.last(), Some(&3));
    let bytes = ipp_response(42);
    let snapshot = parse_printer_attributes(&bytes, 42).unwrap();
    assert_eq!(snapshot.name.as_deref(), Some("Brother DCP-T720DW"));
    assert_eq!(snapshot.state, PrinterState::Idle);
    assert_eq!(snapshot.accepting_jobs, Some(true));
    assert!(parse_printer_attributes(&bytes, 43).is_err());
    for length in 0..bytes.len() {
        assert!(parse_printer_attributes(&bytes[..length], 42).is_err());
    }
}
#[tokio::test]
async fn ipp_client_sends_only_get_printer_attributes() {
    let (url, task) = server(vec![(
        "200 OK\r\nContent-Type: application/ipp",
        ipp_response(42),
    )])
    .await;
    let client = IppClient::new(&format!("{url}/ipp/print")).unwrap();
    let snapshot = client.get_printer_attributes(42).await.unwrap();
    assert_eq!(snapshot.state, PrinterState::Idle);
    let requests = task.await.unwrap();
    let end = requests[0]
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap()
        + 4;
    assert_eq!(&requests[0][end..end + 8], &[1, 1, 0, 11, 0, 0, 0, 42]);
}
