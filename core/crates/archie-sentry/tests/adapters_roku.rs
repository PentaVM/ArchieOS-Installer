use archie_sentry::{
    adapters::roku::Roku,
    contracts::*,
    engine::*,
    registry::*,
};
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

const INFO_ON: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\r\n<device-info>\
    <udn>uuid:roku:ecp:000000000000</udn>\
    <serial-number>X0000TESTSERIAL</serial-number>\
    <device-id>000000000000</device-id>\
    <vendor-name>Roku</vendor-name>\
    <model-name>Roku Ultra</model-name>\
    <user-device-name>Living room</user-device-name>\
    <software-version>12.5.0</software-version>\
    <is-tv>true</is-tv>\
    <power-mode>PowerOn</power-mode></device-info>";
const INFO_OFF: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\r\n<device-info>\
    <serial-number>X0000TESTSERIAL</serial-number>\
    <is-tv>true</is-tv><power-mode>PowerOff</power-mode></device-info>";
const APP_NETFLIX: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\r\n<active-app>\
    <app id=\"12\" type=\"appl\" subtype=\"rsga\" version=\"5.1\">Netflix</app>\
    <screensaver id=\"55545\" type=\"ssvr\" version=\"1.0\">Default screensaver</screensaver>\
    </active-app>";
const APP_HOME: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\r\n<active-app></active-app>";
const PLAYER_PLAY: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\r\n\
    <player error=\"false\" state=\"play\" plugin=\"12.5.0\" format=\"hls\" live=\"false\"/>";
const PLAYER_PAUSE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\r\n\
    <player error=\"false\" state=\"pause\" plugin=\"12.5.0\" format=\"hls\" live=\"false\"/>";
const XML: &str = "200 OK\r\nContent-Type: text/xml";

fn resource() -> ResourceDescriptor {
    ResourceDescriptor {
        resource_id: "tv".into(),
        edge_id: "edge".into(),
        adapter_id: "roku".into(),
        paired: true,
        capabilities: vec![CommandKind::RemoteKey, CommandKind::AppLaunch],
    }
}
fn adapter(base: &str) -> Result<Roku, AdapterError> {
    Roku::new("edge", "roku", BTreeMap::from([("tv".into(), base.into())]))
}

#[tokio::test]
async fn observe_maps_device_info_active_app_and_media_player_to_tv_state() {
    let (url, task) = server(vec![
        (XML, INFO_ON.into()),
        (XML, APP_NETFLIX.into()),
        (XML, PLAYER_PLAY.into()),
        (XML, INFO_ON.into()),
        (XML, APP_NETFLIX.into()),
        (XML, PLAYER_PAUSE.into()),
    ])
    .await;
    let mut adapter = adapter(&url).unwrap();
    let playing = adapter.observe(&resource()).await.unwrap();
    assert_eq!(
        playing.state,
        ResourceState::Tv {
            power_on: true,
            showing_home: false,
            active_app: Some("12".into()),
            playing: Some(true),
        }
    );
    assert_eq!(playing.origin, ObservationOrigin::Measurement);
    assert_eq!(playing.resource_id, "tv");
    assert_eq!(playing.adapter_id, "roku");
    // The serial number in device-info is never retained in any form.
    let serialized = serde_json::to_string(&playing).unwrap();
    assert!(!serialized.contains("X0000TESTSERIAL"));
    assert!(!serialized.contains("Living room"));
    let paused = adapter.observe(&resource()).await.unwrap();
    assert_eq!(
        paused.state,
        ResourceState::Tv {
            power_on: true,
            showing_home: false,
            active_app: Some("12".into()),
            playing: Some(false),
        }
    );
    // Identical measured state keeps evidence identity; a changed state changes it.
    assert_ne!(playing.evidence_id, paused.evidence_id);
    let requests = task.await.unwrap();
    assert!(String::from_utf8_lossy(&requests[0]).starts_with("GET /query/device-info HTTP/1.1"));
    assert!(String::from_utf8_lossy(&requests[1]).starts_with("GET /query/active-app HTTP/1.1"));
    assert!(String::from_utf8_lossy(&requests[2]).starts_with("GET /query/media-player HTTP/1.1"));
}

#[tokio::test]
async fn missing_media_player_leaves_playing_unknown_and_home_screen_is_detected() {
    let (url, task) = server(vec![
        (XML, INFO_OFF.into()),
        (XML, APP_HOME.into()),
        ("404 Not Found", vec![]),
    ])
    .await;
    let mut adapter = adapter(&url).unwrap();
    let observation = adapter.observe(&resource()).await.unwrap();
    assert_eq!(
        observation.state,
        ResourceState::Tv {
            power_on: false,
            showing_home: true,
            active_app: None,
            playing: None,
        }
    );
    task.await.unwrap();
}

#[tokio::test]
async fn keypress_posts_named_key_with_empty_body() {
    let (url, task) = server(vec![("200 OK", vec![])]).await;
    let mut adapter = adapter(&url).unwrap();
    let ack = adapter
        .apply(&resource(), &Command::RemoteKey { key: RemoteKeyName::Home })
        .await
        .unwrap();
    assert!(ack.accepted);
    let requests = task.await.unwrap();
    let request = &requests[0];
    assert!(request.starts_with(b"POST /keypress/Home HTTP/1.1\r\n"));
    let end = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    assert_eq!(request.len(), end, "keypress carries no body");
}

#[tokio::test]
async fn launch_posts_app_id_as_path_segment() {
    let (url, task) = server(vec![("200 OK", vec![])]).await;
    let mut adapter = adapter(&url).unwrap();
    assert!(
        adapter
            .apply(&resource(), &Command::AppLaunch { app_id: "12".into() })
            .await
            .unwrap()
            .accepted
    );
    let requests = task.await.unwrap();
    assert!(requests[0].starts_with(b"POST /launch/12 HTTP/1.1\r\n"));
}

#[tokio::test]
async fn forbidden_and_other_failures_map_to_authentication_and_unavailable() {
    let (url, task) = server(vec![
        ("403 Forbidden", vec![]),
        ("500 Internal Server Error", vec![]),
    ])
    .await;
    let mut adapter = adapter(&url).unwrap();
    let home = Command::RemoteKey { key: RemoteKeyName::Home };
    assert_eq!(
        adapter.apply(&resource(), &home).await,
        Err(AdapterError::Authentication)
    );
    assert_eq!(
        adapter.apply(&resource(), &home).await,
        Err(AdapterError::Unavailable)
    );
    task.await.unwrap();
}

#[tokio::test]
async fn connection_refused_is_transport() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut adapter = adapter(&format!("http://{address}")).unwrap();
    let home = Command::RemoteKey { key: RemoteKeyName::Home };
    assert_eq!(
        adapter.apply(&resource(), &home).await,
        Err(AdapterError::Transport)
    );
    assert_eq!(
        adapter.observe(&resource()).await,
        Err(AdapterError::Transport)
    );
}

#[tokio::test]
async fn unsupported_commands_and_resources_never_send_a_request() {
    let (url, task) = server(vec![]).await;
    let mut adapter = adapter(&url).unwrap();
    assert_eq!(
        adapter
            .apply(&resource(), &Command::SwitchSet { on: true })
            .await,
        Err(AdapterError::Unsupported)
    );
    assert_eq!(
        adapter.apply(&resource(), &Command::MediaPause).await,
        Err(AdapterError::Unsupported)
    );
    // A contract-invalid app id is a protocol error, not a dispatch.
    assert_eq!(
        adapter
            .apply(&resource(), &Command::AppLaunch { app_id: "bad id!".into() })
            .await,
        Err(AdapterError::Protocol)
    );
    // Unpaired or unconfigured resources are unsupported.
    let mut other = resource();
    other.paired = false;
    let home = Command::RemoteKey { key: RemoteKeyName::Home };
    assert_eq!(
        adapter.apply(&other, &home).await,
        Err(AdapterError::Unsupported)
    );
    let mut unknown = resource();
    unknown.resource_id = "bedroom".into();
    assert_eq!(
        adapter.observe(&unknown).await,
        Err(AdapterError::Unsupported)
    );
    assert!(task.await.unwrap().is_empty());
}

#[tokio::test]
async fn oversized_and_malformed_xml_are_bounded_and_rejected() {
    let oversized = vec![b'x'; 1_048_577];
    let (url, task) = server(vec![(XML, oversized)]).await;
    let mut roku = adapter(&url).unwrap();
    assert_eq!(
        roku.observe(&resource()).await,
        Err(AdapterError::Protocol)
    );
    task.await.unwrap();

    let malformed: &[u8] = b"<device-info><model-name>Roku</model-name></device-info>";
    let (url, task) = server(vec![(XML, malformed.into())]).await;
    let mut roku = adapter(&url).unwrap();
    assert_eq!(
        roku.observe(&resource()).await,
        Err(AdapterError::Protocol)
    );
    task.await.unwrap();

    let garbage_active_app: &[u8] = b"this is not xml at all";
    let (url, task) = server(vec![
        (XML, INFO_ON.into()),
        (XML, garbage_active_app.into()),
    ])
    .await;
    let mut roku = adapter(&url).unwrap();
    assert_eq!(
        roku.observe(&resource()).await,
        Err(AdapterError::Protocol)
    );
    task.await.unwrap();
}

#[test]
fn constructor_rejects_public_endpoints_paths_and_injection() {
    for url in [
        "https://example.com",
        "http://8.8.8.8:8060",
        "http://127.0.0.1:8060/query/device-info",
        "http://127.0.0.1/?token=a",
        "http://user:pass@127.0.0.1",
    ] {
        assert!(
            Roku::new("edge", "roku", BTreeMap::from([("tv".into(), url.into())])).is_err(),
            "{url}"
        );
    }
    // A bare private IP is accepted (ECP's 8060 is the default port).
    assert!(
        Roku::new("edge", "roku", BTreeMap::from([("tv".into(), "http://127.0.0.1".into())]))
            .is_ok()
    );
    assert!(Roku::new("edge", "roku", BTreeMap::new()).is_err());
}
