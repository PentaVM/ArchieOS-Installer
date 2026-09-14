//! Look, ground, activate, verify: against an in-memory compositor and a loopback grounding
//! server, so every step is the real code and nothing leaves the machine.

use archie_iris::{LocateAnythingClient, LocateAnythingError, MotorError, NodeId, NodeKind};
use archie_model_runtime::{ProviderEndpoint, RuntimeError};
use archie_policy::{PolicyEngine, Principal};
use archie_sight::{Sight, SightError, Target};
use archie_stage::{
    learn, open, CommandKey, Operator, StageClient, StagePolicy, StageWorld, WireAction,
};
use archie_types::{
    AgentId, Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId,
    PrivacyGrade, ProviderKind, Timestamp, TokenCost,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

const KEY: [u8; 32] = [5; 32];
const APP: &str = "org.example.Form";

fn policy() -> StagePolicy {
    StagePolicy::parse(
        r#"
[[app]]
app_id = "org.example.Form"
display_name = "A form"
allow = ["click", "snapshot"]
"#,
    )
    .unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("archie-sight-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("snapshots")).unwrap();
    dir
}

#[derive(Default)]
struct Seen {
    clicks: Vec<(i32, i32)>,
    captures: usize,
}

/// A compositor with one 640x480 window. `changes` decides whether a capture after a click
/// differs from the one before it.
async fn compositor(
    dir: PathBuf,
    changes: bool,
) -> (
    StageClient<
        tokio::io::ReadHalf<tokio::io::DuplexStream>,
        tokio::io::WriteHalf<tokio::io::DuplexStream>,
    >,
    Arc<Mutex<Seen>>,
) {
    let (client_side, compositor_side) = tokio::io::duplex(256 * 1024);
    let (client_read, client_write) = tokio::io::split(client_side);
    let (compositor_read, mut compositor_write) = tokio::io::split(compositor_side);
    compositor_write
        .write_all(
            b"{\"event\":\"surface\",\"surface\":4,\"app_id\":\"org.example.Form\",\"x\":40,\"y\":60,\"width\":640,\"height\":480,\"geometry_revision\":9}\n\
              {\"event\":\"authority\",\"mode\":\"autonomous\",\"reason\":\"start\"}\n",
        )
        .await
        .unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let record = seen.clone();
    tokio::spawn(async move {
        let key = CommandKey::from_bytes(&KEY);
        let mut lines = BufReader::new(compositor_read).lines();
        let mut last = 0;
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(command) = open(&key, format!("{line}\n").as_bytes(), last, Timestamp::now())
            else {
                continue;
            };
            last = command.seq;
            let mut reply = String::new();
            match command.action {
                WireAction::Snapshot => {
                    let index = {
                        let mut seen = record.lock().unwrap();
                        seen.captures += 1;
                        if changes {
                            seen.captures
                        } else {
                            1
                        }
                    };
                    let png: Vec<u8> = [
                        b"\x89PNG\r\n\x1a\n".as_slice(),
                        format!("window as drawn, version {index}").as_bytes(),
                    ]
                    .concat();
                    let path = dir.join("snapshots").join(format!("{}.png", command.seq));
                    std::fs::write(&path, &png).unwrap();
                    let digest =
                        hex::encode(ring::digest::digest(&ring::digest::SHA256, &png).as_ref());
                    reply.push_str(&format!(
                        "{{\"event\":\"snapshot\",\"seq\":{},\"surface\":4,\"width\":640,\"height\":480,\"sha256\":\"{digest}\",\"path\":{}}}\n",
                        command.seq,
                        serde_json::to_string(&path.to_string_lossy()).unwrap()
                    ));
                }
                WireAction::Click { x, y, .. } => record.lock().unwrap().clicks.push((x, y)),
                _ => {}
            }
            reply.push_str(&format!(
                "{{\"event\":\"receipt\",\"seq\":{},\"outcome\":\"executed\",\"observed_revision\":1}}\n",
                command.seq
            ));
            if compositor_write.write_all(reply.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    (StageClient::new(client_read, client_write), seen)
}

/// A grounding server that answers every request with one box and records what it was sent.
async fn grounding_server(
    provider_model: &'static str,
    reply_box: &'static str,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let record = requests.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut bytes = Vec::new();
            let mut buffer = vec![0; 64 * 1024];
            // Read headers, then as much body as Content-Length says.
            loop {
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                if read == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&bytes);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            record
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&bytes).into_owned());
            let response = format!(
                r#"{{"model":"{provider_model}","choices":[{{"message":{{"role":"assistant","content":"{reply_box}"}},"finish_reason":"stop"}}]}}"#
            );
            let _ = socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                        response.len()
                    )
                    .as_bytes(),
                )
                .await;
        }
    });
    (format!("http://{address}/v1"), requests)
}

fn local_model() -> ModelDescriptor {
    let mut labels = BTreeMap::new();
    labels.insert("provider-model".into(), "nvidia/LocateAnything-3B".into());
    ModelDescriptor {
        id: ModelId::parse("nvidia.locate-anything-3b").unwrap(),
        display_name: "LocateAnything on this machine".into(),
        provider: ProviderKind::NvidiaNim,
        locality: Locality::Local,
        node: None,
        context_window: 24_000,
        max_output_tokens: 128,
        capabilities: vec![Capability::known("compute.inference")],
        max_privacy_grade: PrivacyGrade::Restricted,
        retention: DataRetention::ZeroVerified,
        availability: Availability::Ready,
        cost: TokenCost::FREE,
        latency_ema_ms: None,
        quality_tier: 50,
        required_accelerator_bytes: None,
        labels,
    }
}

fn target(screen: PrivacyGrade) -> Target {
    Target {
        app_id: APP.into(),
        node: NodeId::new("button:save").unwrap(),
        kind: NodeKind::Button,
        description: "the save button".into(),
        description_grade: PrivacyGrade::Public,
        screen_grade: screen,
        task_id: "sight-test".into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_named_button_is_seen_grounded_clicked_and_seen_to_change() {
    let dir = scratch("changes");
    let (mut client, seen) = compositor(dir.clone(), true).await;
    let mut world = StageWorld::default();
    learn(&mut client, &mut world, Duration::from_millis(50))
        .await
        .unwrap();
    let (url, requests) = grounding_server(
        "nvidia/LocateAnything-3B",
        "<box> 500, 500, 600, 600 </box>",
    )
    .await;
    let grounding = LocateAnythingClient::new(
        ProviderEndpoint::nvidia_nim(&url, Locality::Local).unwrap(),
        local_model(),
    )
    .unwrap();

    let mut operator = Operator::new(
        CommandKey::from_bytes(&KEY),
        policy(),
        PolicyEngine::default(),
    );
    let mut sight = Sight {
        operator: &mut operator,
        client: &mut client,
        world: &mut world,
        snapshot_dir: dir.join("snapshots"),
        principal: Principal::Agent {
            agent: AgentId::new(),
        },
        engine: PolicyEngine::default(),
        timeout: Duration::from_secs(5),
    };
    let sighting = sight
        .find_and_activate(
            &target(PrivacyGrade::Internal),
            &grounding,
            Locality::Local,
            77,
        )
        .await
        .unwrap();

    // 500..600 permille of 640x480 is x 320..384, y 240..288; the centre is (352, 264).
    assert_eq!(
        (
            sighting.bbox.x,
            sighting.bbox.y,
            sighting.bbox.width,
            sighting.bbox.height
        ),
        (320, 240, 64, 48)
    );
    assert_eq!((sighting.point.x, sighting.point.y), (352, 264));
    assert_eq!(
        seen.lock().unwrap().clicks,
        vec![(352, 264)],
        "the click lands where grounding said, in surface coordinates"
    );
    assert_eq!(
        seen.lock().unwrap().captures,
        2,
        "one look before, one after"
    );
    assert_eq!(sighting.outcome.outcome_code, "clicked_and_changed");
    assert!(sighting.after.is_some());

    let wire = requests.lock().unwrap().join("\n");
    assert!(
        wire.contains("[vault:stage/target"),
        "the target travels as a capsule entry"
    );
    assert!(wire.contains("the save button"));
    assert!(
        wire.contains("data:image/png;base64,"),
        "and the capture goes with it"
    );
    assert_eq!(
        std::fs::read_dir(dir.join("snapshots")).unwrap().count(),
        0,
        "no capture is left on disk"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_window_that_did_not_change_is_not_verification() {
    let dir = scratch("still");
    let (mut client, seen) = compositor(dir.clone(), false).await;
    let mut world = StageWorld::default();
    learn(&mut client, &mut world, Duration::from_millis(50))
        .await
        .unwrap();
    let (url, _) = grounding_server(
        "nvidia/LocateAnything-3B",
        "<box> 100, 100, 200, 200 </box>",
    )
    .await;
    let grounding = LocateAnythingClient::new(
        ProviderEndpoint::nvidia_nim(&url, Locality::Local).unwrap(),
        local_model(),
    )
    .unwrap();
    let mut operator = Operator::new(
        CommandKey::from_bytes(&KEY),
        policy(),
        PolicyEngine::default(),
    );
    let mut sight = Sight {
        operator: &mut operator,
        client: &mut client,
        world: &mut world,
        snapshot_dir: dir.join("snapshots"),
        principal: Principal::Agent {
            agent: AgentId::new(),
        },
        engine: PolicyEngine::default(),
        timeout: Duration::from_secs(5),
    };
    let result = sight
        .find_and_activate(
            &target(PrivacyGrade::Public),
            &grounding,
            Locality::Local,
            78,
        )
        .await;
    assert!(
        matches!(
            result,
            Err(SightError::Motor(MotorError::VerificationFailed))
        ),
        "a click the window does not show is reported as unverified: {result:?}"
    );
    assert_eq!(seen.lock().unwrap().clicks.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cloud_model_of_unknown_retention_never_sees_an_internal_window() {
    let dir = scratch("cloud");
    let (mut client, seen) = compositor(dir.clone(), true).await;
    let mut world = StageWorld::default();
    learn(&mut client, &mut world, Duration::from_millis(50))
        .await
        .unwrap();
    let mut cloud = local_model();
    cloud.provider = ProviderKind::NebiusTokenFactory;
    cloud.locality = Locality::Cloud;
    cloud.retention = DataRetention::Unknown;
    cloud.max_privacy_grade = PrivacyGrade::Public;
    let grounding = LocateAnythingClient::new(
        ProviderEndpoint::nebius("https://api.tokenfactory.nebius.com/v1").unwrap(),
        cloud,
    )
    .unwrap();
    let mut operator = Operator::new(
        CommandKey::from_bytes(&KEY),
        policy(),
        PolicyEngine::default(),
    );
    let mut sight = Sight {
        operator: &mut operator,
        client: &mut client,
        world: &mut world,
        snapshot_dir: dir.join("snapshots"),
        principal: Principal::Agent {
            agent: AgentId::new(),
        },
        engine: PolicyEngine::default(),
        timeout: Duration::from_secs(5),
    };
    let result = sight
        .find_and_activate(
            &target(PrivacyGrade::Internal),
            &grounding,
            Locality::Cloud,
            79,
        )
        .await;
    assert!(
        // By locality alone an internal window may go to the cloud, so the grounding client
        // passes it on; the runtime refuses it immediately before egress, because this model
        // is cleared for public data and nobody verified what it retains.
        matches!(
            result,
            Err(SightError::Grounding(LocateAnythingError::Transport(
                RuntimeError::PrivacyDenied { .. }
            )))
        ),
        "{result:?}"
    );
    assert!(
        seen.lock().unwrap().clicks.is_empty(),
        "nothing is clicked on a grounding that never happened"
    );
}
