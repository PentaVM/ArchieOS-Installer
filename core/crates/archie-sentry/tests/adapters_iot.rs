//! Integration tests for the IoT protocol adapters: MQTT 3.1.1 over TCP,
//! observation-only ONVIF over SOAP, and the stated Chromecast non-support.
use archie_sentry::{
    adapters::{
        chromecast::{CastDevice, Chromecast},
        mqtt::{Mqtt, MqttBinding},
        onvif::{base64_encode, password_digest, sha1, wsse_header, OnvifProbe},
    },
    contracts::*,
    engine::*,
    registry::ResourceDescriptor,
};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

// ---------- shared fixtures ----------

fn resource(adapter: &str, capabilities: Vec<CommandKind>) -> ResourceDescriptor {
    ResourceDescriptor {
        resource_id: "lamp".into(),
        edge_id: "edge".into(),
        adapter_id: adapter.into(),
        paired: true,
        capabilities,
    }
}

async fn http_server(
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

// ---------- MQTT fixtures ----------

async fn mqtt_read_packet(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut first = [0u8; 1];
    stream.read_exact(&mut first).await.unwrap();
    let (mut remaining, mut multiplier) = (0usize, 1usize);
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).await.unwrap();
        remaining += usize::from(byte[0] & 0x7f) * multiplier;
        if byte[0] & 0x80 == 0 {
            break;
        }
        multiplier *= 128;
    }
    let mut body = vec![0u8; remaining];
    stream.read_exact(&mut body).await.unwrap();
    (first[0], body)
}

/// One-byte remaining length suffices for every fixture below.
fn mqtt_publish_frame(topic: &str, payload: &[u8]) -> Vec<u8> {
    let mut body = (topic.len() as u16).to_be_bytes().to_vec();
    body.extend_from_slice(topic.as_bytes());
    body.extend_from_slice(payload);
    assert!(body.len() < 128);
    let mut frame = vec![0x30, body.len() as u8];
    frame.extend_from_slice(&body);
    frame
}

fn mqtt_adapter(address: &str, binding: MqttBinding) -> Mqtt {
    Mqtt::new(
        "edge",
        "mqtt",
        address,
        None,
        BTreeMap::from([("lamp".into(), binding)]),
    )
    .unwrap()
}

// ---------- MQTT tests ----------

#[tokio::test]
async fn mqtt_switch_publish_matches_expected_topic_and_payload_then_disconnects() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let (header, _) = mqtt_read_packet(&mut stream).await;
        assert_eq!(header >> 4, 1, "CONNECT");
        stream.write_all(&[0x20, 0x02, 0x00, 0x00]).await.unwrap(); // CONNACK accept
        let (header, body) = mqtt_read_packet(&mut stream).await;
        assert_eq!(header, 0x30, "PUBLISH qos 0");
        let (header, disconnect) = mqtt_read_packet(&mut stream).await;
        assert_eq!((header, disconnect.is_empty()), (0xE0, true), "DISCONNECT");
        body
    });
    let mut adapter = mqtt_adapter(
        &address.to_string(),
        MqttBinding::switch("home/lamp/set", "home/lamp/state", "ON", "OFF"),
    );
    let ack = adapter
        .apply(
            &resource("mqtt", vec![CommandKind::SwitchSet]),
            &Command::SwitchSet { on: true },
        )
        .await
        .unwrap();
    assert!(ack.accepted);
    let body = task.await.unwrap();
    let length = usize::from(u16::from_be_bytes([body[0], body[1]]));
    assert_eq!(&body[2..2 + length], b"home/lamp/set");
    assert_eq!(&body[2 + length..], b"ON");
}

#[tokio::test]
async fn mqtt_climate_temperature_is_rendered_as_celsius_with_one_decimal() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        mqtt_read_packet(&mut stream).await;
        stream.write_all(&[0x20, 0x02, 0x00, 0x00]).await.unwrap();
        let (_, body) = mqtt_read_packet(&mut stream).await;
        mqtt_read_packet(&mut stream).await; // DISCONNECT
        body
    });
    let mut adapter = mqtt_adapter(
        &address.to_string(),
        MqttBinding::climate("home/thermostat/set", "home/thermostat/state", "set {celsius}"),
    );
    adapter
        .apply(
            &resource("mqtt", vec![CommandKind::ClimateSetTemperature]),
            &Command::ClimateSetTemperature {
                milli_celsius: 22500,
            },
        )
        .await
        .unwrap();
    let body = task.await.unwrap();
    let length = usize::from(u16::from_be_bytes([body[0], body[1]]));
    assert_eq!(&body[2..2 + length], b"home/thermostat/set");
    assert_eq!(&body[2 + length..], b"set 22.5");
}

#[tokio::test]
async fn mqtt_observe_subscribes_and_parses_matching_publish_into_switch_state() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        mqtt_read_packet(&mut stream).await; // CONNECT
        stream.write_all(&[0x20, 0x02, 0x00, 0x00]).await.unwrap();
        let (header, subscribe) = mqtt_read_packet(&mut stream).await;
        assert_eq!(header, 0x82, "SUBSCRIBE");
        let packet_id = [subscribe[0], subscribe[1]];
        let topic_length = usize::from(u16::from_be_bytes([subscribe[2], subscribe[3]]));
        assert_eq!(&subscribe[4..4 + topic_length], b"home/lamp/state");
        assert_eq!(subscribe[4 + topic_length], 0, "requested qos 0");
        let mut suback = vec![0x90, 0x03];
        suback.extend_from_slice(&packet_id);
        suback.push(0x00);
        stream.write_all(&suback).await.unwrap();
        stream
            .write_all(&mqtt_publish_frame("home/lamp/state", b"OFF"))
            .await
            .unwrap();
        let (header, _) = mqtt_read_packet(&mut stream).await;
        assert_eq!(header, 0xE0, "DISCONNECT");
    });
    let mut adapter = mqtt_adapter(
        &address.to_string(),
        MqttBinding::switch("home/lamp/set", "home/lamp/state", "ON", "OFF"),
    );
    let observation = adapter
        .observe(&resource("mqtt", vec![CommandKind::SwitchSet]))
        .await
        .unwrap();
    task.await.unwrap();
    assert_eq!(observation.state, ResourceState::Switch { on: false });
    assert_eq!(observation.origin, ObservationOrigin::Measurement);
    assert_eq!(observation.resource_id, "lamp");
    assert_eq!(observation.adapter_id, "mqtt");
    assert!(observation.evidence_id.starts_with("mqtt-"));
}

#[tokio::test]
async fn mqtt_truncated_packet_is_a_protocol_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        mqtt_read_packet(&mut stream).await;
        stream.write_all(&[0x20, 0x02, 0x00, 0x00]).await.unwrap();
        mqtt_read_packet(&mut stream).await; // SUBSCRIBE
        stream.write_all(&[0x90, 0x03, 0x00, 0x01, 0x00]).await.unwrap();
        // Declares a 10-byte body, sends 3 bytes, then closes.
        stream.write_all(&[0x30, 0x0A, 0x00, 0x01, b'a']).await.unwrap();
    });
    let mut adapter = mqtt_adapter(
        &address.to_string(),
        MqttBinding::switch("home/lamp/set", "home/lamp/state", "ON", "OFF"),
    );
    let result = adapter
        .observe(&resource("mqtt", vec![CommandKind::SwitchSet]))
        .await;
    task.await.unwrap();
    assert_eq!(result, Err(AdapterError::Protocol));
}

#[tokio::test]
async fn mqtt_oversized_remaining_length_is_rejected_without_reading_the_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        mqtt_read_packet(&mut stream).await;
        stream.write_all(&[0x20, 0x02, 0x00, 0x00]).await.unwrap();
        mqtt_read_packet(&mut stream).await; // SUBSCRIBE
        stream.write_all(&[0x90, 0x03, 0x00, 0x01, 0x00]).await.unwrap();
        // Remaining-length varint for 300_000 > 256 KiB cap, no body follows.
        stream.write_all(&[0x30, 0xE0, 0xA7, 0x12]).await.unwrap();
    });
    let mut adapter = mqtt_adapter(
        &address.to_string(),
        MqttBinding::switch("home/lamp/set", "home/lamp/state", "ON", "OFF"),
    );
    let result = adapter
        .observe(&resource("mqtt", vec![CommandKind::SwitchSet]))
        .await;
    task.await.unwrap();
    assert_eq!(result, Err(AdapterError::Protocol));
}

#[tokio::test]
async fn mqtt_connack_refusal_maps_to_authentication() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let (_, connect) = mqtt_read_packet(&mut stream).await;
        // Username flag set and username present; the password never appears
        // anywhere else (and is never logged by the adapter).
        assert!(connect.windows(8).any(|w| w == b"\x00\x06bridge"));
        stream.write_all(&[0x20, 0x02, 0x00, 0x05]).await.unwrap(); // not authorised
    });
    let mut adapter = Mqtt::new(
        "edge",
        "mqtt",
        &address.to_string(),
        Some(("bridge".into(), "mqtt-password".into())),
        BTreeMap::from([(
            "lamp".into(),
            MqttBinding::switch("home/lamp/set", "home/lamp/state", "ON", "OFF"),
        )]),
    )
    .unwrap();
    let result = adapter
        .apply(
            &resource("mqtt", vec![CommandKind::SwitchSet]),
            &Command::SwitchSet { on: true },
        )
        .await;
    task.await.unwrap();
    assert_eq!(result, Err(AdapterError::Authentication));
    let debug = format!("{adapter:?}");
    assert!(!debug.contains("mqtt-password"));
}

#[test]
fn mqtt_constructor_rejects_public_brokers_wildcards_and_bad_templates() {
    let binding = || MqttBinding::switch("home/lamp/set", "home/lamp/state", "ON", "OFF");
    let bindings = || BTreeMap::from([("lamp".into(), binding())]);
    assert!(Mqtt::new("edge", "mqtt", "8.8.8.8:1883", None, bindings()).is_err());
    assert!(Mqtt::new("edge", "mqtt", "127.0.0.1:0", None, bindings()).is_err());
    let mut wildcard = binding();
    wildcard.command_topic = "home/+/set".into();
    assert!(
        Mqtt::new(
            "edge",
            "mqtt",
            "127.0.0.1:1883",
            None,
            BTreeMap::from([("lamp".into(), wildcard)])
        )
        .is_err()
    );
    let mut bad_template = binding();
    bad_template.temperature_template = "set point".into();
    assert!(
        Mqtt::new(
            "edge",
            "mqtt",
            "127.0.0.1:1883",
            None,
            BTreeMap::from([("lamp".into(), bad_template)])
        )
        .is_err()
    );
}

#[tokio::test]
async fn mqtt_unsupported_command_and_unconfigured_resource_are_unsupported() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut adapter = mqtt_adapter(
        &address.to_string(),
        MqttBinding::switch("home/lamp/set", "home/lamp/state", "ON", "OFF"),
    );
    // MediaPause is not an MQTT-renderable command here.
    let mut media = resource("mqtt", vec![CommandKind::MediaPause]);
    assert_eq!(
        adapter.apply(&media, &Command::MediaPause).await,
        Err(AdapterError::Unsupported)
    );
    // A SwitchSet on a switch resource without the declared capability is refused.
    media.capabilities = vec![CommandKind::LightSetBrightness];
    assert_eq!(
        adapter
            .apply(&media, &Command::SwitchSet { on: true })
            .await,
        Err(AdapterError::Unsupported)
    );
    // Unpaired resource is refused before any network traffic.
    media.paired = false;
    media.capabilities = vec![CommandKind::SwitchSet];
    assert_eq!(
        adapter
            .observe(&media)
            .await,
        Err(AdapterError::Unsupported)
    );
}

// ---------- ONVIF tests ----------

fn soap_ok(inner: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <s:Envelope xmlns:s=\"http://www.w3.org/2003/05/soap-envelope\">\
         <s:Body>{inner}</s:Body></s:Envelope>"
    )
    .into_bytes()
}

const DEVICE_INFO: &str = "<tds:GetDeviceInformationResponse \
    xmlns:tds=\"http://www.onvif.org/ver10/device/wsdl\">\
    <tds:Manufacturer>ArchieCam</tds:Manufacturer><tds:Model>AC-1</tds:Model>\
    <tds:FirmwareVersion>1.2.3</tds:FirmwareVersion>\
    <tds:SerialNumber>SN-9</tds:SerialNumber><tds:HardwareId>HW-1</tds:HardwareId>\
    </tds:GetDeviceInformationResponse>";
const PROFILES: &str = "<trt:GetProfilesResponse \
    xmlns:trt=\"http://www.onvif.org/ver10/media/wsdl\">\
    <trt:Profiles token=\"profile_1\"><tt:Name \
    xmlns:tt=\"http://www.onvif.org/ver10/schema\">Main</tt:Name></trt:Profiles>\
    </trt:GetProfilesResponse>";
const STREAM: &str = "<trt:GetStreamUriResponse \
    xmlns:trt=\"http://www.onvif.org/ver10/media/wsdl\"><trt:MediaUri><tt:Uri \
    xmlns:tt=\"http://www.onvif.org/ver10/schema\">rtsp://192.168.1.20:554/stream1</tt:Uri>\
    </trt:MediaUri></trt:GetStreamUriResponse>";

#[tokio::test]
async fn onvif_probe_parses_device_information_and_stream_uri() {
    let (url, task) = http_server(vec![
        ("200 OK\r\nContent-Type: application/soap+xml", soap_ok(DEVICE_INFO)),
        ("200 OK\r\nContent-Type: application/soap+xml", soap_ok(PROFILES)),
        ("200 OK\r\nContent-Type: application/soap+xml", soap_ok(STREAM)),
    ])
    .await;
    let probe = OnvifProbe::new(&url, Some(("operator".into(), "cam-secret".into()))).unwrap();
    let info = probe.probe().await.unwrap();
    assert_eq!(info.manufacturer, "ArchieCam");
    assert_eq!(info.model, "AC-1");
    assert_eq!(info.firmware, "1.2.3");
    assert_eq!(info.stream_uris, vec!["rtsp://192.168.1.20:554/stream1"]);
    let requests = task.await.unwrap();
    assert_eq!(requests.len(), 3);
    for request in &requests {
        let text = String::from_utf8_lossy(request);
        assert!(text.starts_with("POST / "));
        assert!(text.contains("<wsse:Username>operator</wsse:Username>"));
        assert!(text.contains("#PasswordDigest"));
        // Digest authentication: the cleartext password never crosses the wire.
        assert!(!text.contains("cam-secret"));
    }
    let stream_request = String::from_utf8_lossy(&requests[2]);
    assert!(stream_request.contains("<trt:ProfileToken>profile_1</trt:ProfileToken>"));
}

#[tokio::test]
async fn onvif_http_401_maps_to_authentication() {
    let (url, task) = http_server(vec![("401 Unauthorized", vec![])]).await;
    let probe = OnvifProbe::new(&url, Some(("operator".into(), "cam-secret".into()))).unwrap();
    assert_eq!(probe.probe().await, Err(AdapterError::Authentication));
    task.await.unwrap();
}

#[tokio::test]
async fn onvif_malformed_xml_is_a_protocol_error() {
    let (url, task) = http_server(vec![(
        "200 OK\r\nContent-Type: application/soap+xml",
        b"this is not xml at all".to_vec(),
    )])
    .await;
    let probe = OnvifProbe::new(&url, None).unwrap();
    assert_eq!(probe.probe().await, Err(AdapterError::Protocol));
    task.await.unwrap();
}

#[tokio::test]
async fn onvif_soap_fault_is_a_protocol_error() {
    let fault = soap_ok(
        "<s:Fault><s:Code><s:Value>s:Sender</s:Value></s:Code>\
         <s:Reason><s:Text>ter:NotAuthorized</s:Text></s:Reason></s:Fault>",
    );
    let (url, task) = http_server(vec![("200 OK\r\nContent-Type: application/soap+xml", fault)]).await;
    let probe = OnvifProbe::new(&url, None).unwrap();
    assert_eq!(probe.probe().await, Err(AdapterError::Protocol));
    task.await.unwrap();
}

#[test]
fn onvif_wsse_digest_is_sha1_base64_and_deterministic() {
    // SHA-1 and base64 ground truths (FIPS 180-1 / RFC 4648 examples).
    let digest_hex: String = sha1(b"abc").iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(digest_hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    // Fixed (nonce, created, password) → fixed digest, verified independently.
    let digest = password_digest(b"0123456789abcdef", "2024-01-01T00:00:00Z", "cam-secret");
    assert_eq!(digest, "ic0sEg9LTjlTPKiNBt8E6a/OTxc=");
    let header = wsse_header("operator", "cam-secret", b"0123456789abcdef", "2024-01-01T00:00:00Z");
    assert!(header.contains(
        "#PasswordDigest\">ic0sEg9LTjlTPKiNBt8E6a/OTxc=</wsse:Password>"
    ));
    assert!(header.contains("#Base64Binary\">MDEyMzQ1Njc4OWFiY2RlZg==</wsse:Nonce>"));
    assert!(header.contains("<wsu:Created>2024-01-01T00:00:00Z</wsu:Created>"));
    assert!(!header.contains("cam-secret"));
}

#[test]
fn onvif_probe_rejects_credentialed_cleartext_off_loopback() {
    // Repo endpoint policy: http + credentials is loopback-only.
    assert!(OnvifProbe::new(
        "http://192.168.1.20/onvif/device_service",
        Some(("operator".into(), "cam-secret".into()))
    )
    .is_err());
    // Anonymous probes may reach private LAN addresses over http.
    assert!(OnvifProbe::new("http://192.168.1.20/onvif/device_service", None).is_ok());
    assert!(OnvifProbe::new("https://example.com/onvif/device_service", None).is_err());
}

// ---------- Chromecast tests ----------

fn cast_device() -> CastDevice {
    CastDevice::new(
        "tv",
        "Living Room",
        "Chromecast",
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 30)),
        8009,
    )
    .unwrap()
}

#[test]
fn cast_device_descriptor_stores_bounded_fields() {
    let device = cast_device();
    assert_eq!(device.name, "Living Room");
    assert_eq!(device.model, "Chromecast");
    assert_eq!(device.port, 8009);
    let address = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 30));
    let public = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));
    let cases: [(String, String, IpAddr, u16); 5] = [
        ("x".repeat(65), "Chromecast".to_owned(), address, 8009),
        ("Living\nRoom".to_owned(), "Chromecast".to_owned(), address, 8009),
        ("Living Room".to_owned(), String::new(), address, 8009),
        ("Living Room".to_owned(), "Chromecast".to_owned(), public, 8009),
        ("Living Room".to_owned(), "Chromecast".to_owned(), address, 0),
    ];
    for (name, model, addr, port) in cases {
        assert!(CastDevice::new("tv", &name, &model, addr, port).is_err());
    }
    assert!(CastDevice::new("bad id!", "Living Room", "Chromecast", address, 8009).is_err());
}

#[tokio::test]
async fn chromecast_apply_is_explicitly_unsupported_with_a_documented_reason() {
    let mut adapter = Chromecast::new("edge", "cast", vec![cast_device()]).unwrap();
    let descriptor = adapter.descriptor("tv").unwrap();
    assert_eq!(descriptor.model, "Chromecast");
    assert!(adapter.descriptor("unknown").is_none());
    let resource = ResourceDescriptor {
        resource_id: "tv".into(),
        edge_id: "edge".into(),
        adapter_id: "cast".into(),
        paired: true,
        capabilities: vec![CommandKind::MediaPause],
    };
    assert_eq!(
        adapter.apply(&resource, &Command::MediaPause).await,
        Err(AdapterError::Unsupported)
    );
    assert_eq!(
        adapter.observe(&resource).await,
        Err(AdapterError::Unsupported)
    );
    // The absence is a stated decision, not a silence.
    assert!(Chromecast::UNSUPPORTED_REASON.contains("TLS"));
    assert!(Chromecast::UNSUPPORTED_REASON.contains("protobuf"));
    assert!(Chromecast::UNSUPPORTED_REASON.contains("out of scope"));
}
