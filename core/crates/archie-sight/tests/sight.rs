//! Look, ground, act, verify: against an in-memory compositor and a loopback grounding
//! server, so every step is the real code and nothing leaves the machine.

use archie_iris::{LocateAnythingClient, LocateAnythingError, MotorError, NodeId, NodeKind};
use archie_model_runtime::{ProviderEndpoint, RuntimeError};
use archie_policy::{PolicyEngine, Principal};
use archie_sight::{Knowledge, Sight, SightError, Target};
use archie_stage::{
    learn, open, CommandKey, Operator, StageClient, StagePolicy, StageWorld, WireAction,
};
use archie_types::{
    AgentId, Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId,
    PrivacyGrade, ProviderKind, Timestamp, TokenCost,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

const KEY: [u8; 32] = [5; 32];
const APP: &str = "org.example.Form";
/// Where the window draws its save button, which is also what the grounding server points
/// at: 500..600 permille of 640x480.
const BUTTON: (u32, u32, u32, u32) = (320, 240, 64, 48);
/// And its note field: 100..300 by 100..150 permille.
const FIELD: (u32, u32, u32, u32) = (64, 48, 128, 24);

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

/// The window this compositor draws, and what it has been asked to do.
struct Stage {
    clicks: Vec<(i32, i32)>,
    captures: usize,
    /// How many marks the button is drawn with. Changing it redraws a different control.
    bars: usize,
    /// Whether a click makes the window redraw where it landed.
    reacts: bool,
}

impl Default for Stage {
    fn default() -> Self {
        Self {
            clicks: Vec::new(),
            captures: 0,
            bars: 3,
            reacts: true,
        }
    }
}

/// A 640x480 window with a field and a button in it, in grey.
fn window_png(bars: usize) -> Vec<u8> {
    let (width, height) = (640_u32, 480_u32);
    let mut pixels = vec![0xf2_u8; (width * height) as usize];
    let mut draw = |(bx, by, bw, bh): (u32, u32, u32, u32), marks: usize| {
        for y in by..by + bh {
            for x in bx..bx + bw {
                let edge = x == bx || x == bx + bw - 1 || y == by || y == by + bh - 1;
                pixels[(y * width + x) as usize] = if edge { 0x40 } else { 0xd8 };
            }
        }
        for mark in 0..marks {
            let left = bx + 6 + mark as u32 * 7;
            if left + 3 >= bx + bw {
                break;
            }
            for y in by + 6..by + bh - 6 {
                for x in left..left + 3 {
                    pixels[(y * width + x) as usize] = 0x10;
                }
            }
        }
    };
    draw(FIELD, 2);
    draw(BUTTON, bars);
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
    png
}

type Pipe = StageClient<
    tokio::io::ReadHalf<tokio::io::DuplexStream>,
    tokio::io::WriteHalf<tokio::io::DuplexStream>,
>;

/// A compositor with that one window. It captures what it draws, clicks where it is told,
/// and redraws a patch where a click landed, which is the only thing that verifies one.
/// The returned sender injects raw events, for a window that resizes or redraws itself.
async fn compositor(
    dir: PathBuf,
    stage: Arc<Mutex<Stage>>,
) -> (Pipe, tokio::sync::mpsc::UnboundedSender<String>) {
    let (client_side, compositor_side) = tokio::io::duplex(1024 * 1024);
    let (client_read, client_write) = tokio::io::split(client_side);
    let (compositor_read, mut compositor_write) = tokio::io::split(compositor_side);
    compositor_write
        .write_all(
            b"{\"event\":\"surface\",\"surface\":4,\"app_id\":\"org.example.Form\",\"x\":40,\"y\":60,\"width\":640,\"height\":480,\"geometry_revision\":9,\"pid\":4321}\n\
              {\"event\":\"authority\",\"mode\":\"autonomous\",\"reason\":\"start\"}\n",
        )
        .await
        .unwrap();
    let (inject, mut injected) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let key = CommandKey::from_bytes(&KEY);
        let mut lines = BufReader::new(compositor_read).lines();
        let mut last = 0;
        loop {
            let line = tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(line)) => line,
                    _ => break,
                },
                event = injected.recv() => match event {
                    Some(event) => {
                        if compositor_write.write_all(event.as_bytes()).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    None => break,
                },
            };
            let Ok(command) = open(&key, format!("{line}\n").as_bytes(), last, Timestamp::now())
            else {
                continue;
            };
            last = command.seq;
            let mut reply = String::new();
            match command.action {
                WireAction::Snapshot => {
                    let png = {
                        let mut stage = stage.lock().unwrap();
                        stage.captures += 1;
                        window_png(stage.bars)
                    };
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
                WireAction::Click { x, y, .. } => {
                    let reacts = {
                        let mut stage = stage.lock().unwrap();
                        stage.clicks.push((x, y));
                        stage.reacts
                    };
                    if reacts {
                        reply.push_str(&format!(
                            "{{\"event\":\"damage\",\"surface\":4,\"rects\":[{{\"x\":{},\"y\":{},\"width\":20,\"height\":20}}]}}\n",
                            x - 10,
                            y - 10
                        ));
                    }
                }
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
    (StageClient::new(client_read, client_write), inject)
}

/// A grounding server that answers by what the request asks for, and keeps every request.
async fn grounding_server(
    provider_model: &'static str,
    boxes: &'static [(&'static str, &'static str)],
    slowly: Duration,
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
            if !slowly.is_zero() {
                tokio::time::sleep(slowly).await;
            }
            let request = String::from_utf8_lossy(&bytes).into_owned();
            let answer = boxes
                .iter()
                .find(|(asked, _)| request.contains(asked))
                .map(|(_, reply)| *reply)
                .unwrap_or(boxes[0].1);
            record.lock().unwrap().push(request);
            let response = format!(
                r#"{{"model":"{provider_model}","choices":[{{"message":{{"role":"assistant","content":"{answer}"}},"finish_reason":"stop"}}]}}"#
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

const SAVE: &str = "the save button";
const NOTE: &str = "the note field";

fn target(node: &str, description: &str, screen: PrivacyGrade) -> Target {
    Target {
        app_id: APP.into(),
        node: NodeId::new(node.to_string()).unwrap(),
        kind: NodeKind::Button,
        description: description.into(),
        description_grade: PrivacyGrade::Public,
        screen_grade: screen,
        task_id: "sight-test".into(),
    }
}

/// A session against that compositor and that grounding server.
async fn session(
    dir: &Path,
    client: Pipe,
) -> Sight<
    tokio::io::ReadHalf<tokio::io::DuplexStream>,
    tokio::io::WriteHalf<tokio::io::DuplexStream>,
> {
    let mut client = client;
    let mut world = StageWorld::default();
    learn(&mut client, &mut world, Duration::from_millis(50))
        .await
        .unwrap();
    let operator = Operator::new(
        CommandKey::from_bytes(&KEY),
        policy(),
        PolicyEngine::default(),
    );
    Sight::new(
        operator,
        client,
        world,
        dir.join("snapshots"),
        Principal::Agent {
            agent: AgentId::new(),
        },
    )
    .with_timeouts(Duration::from_secs(5), Duration::from_millis(500))
}

fn client_for(url: &str) -> LocateAnythingClient {
    LocateAnythingClient::new(
        ProviderEndpoint::nvidia_nim(url, Locality::Local).unwrap(),
        local_model(),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_named_button_is_found_clicked_and_seen_to_redraw() {
    let dir = scratch("found");
    let stage = Arc::new(Mutex::new(Stage::default()));
    let (client, _inject) = compositor(dir.clone(), stage.clone()).await;
    let (url, requests) = grounding_server(
        "nvidia/LocateAnything-3B",
        &[(SAVE, "<box> 500, 500, 600, 600 </box>")],
        Duration::ZERO,
    )
    .await;
    let grounding = client_for(&url);
    let mut sight = session(&dir, client).await;

    let sighting = sight
        .find_and_activate(
            &target("button:save", SAVE, PrivacyGrade::Internal),
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
    assert_eq!(sighting.knowledge, Knowledge::Grounded);
    assert_eq!(sighting.outcome.outcome_code, "clicked_and_repainted");
    assert_eq!(
        stage.lock().unwrap().clicks,
        vec![(352, 264)],
        "the click lands where grounding said, in surface coordinates"
    );
    assert_eq!(
        stage.lock().unwrap().captures,
        1,
        "one look, and the window's own redraw is the verification"
    );

    let wire = requests.lock().unwrap().join("\n");
    assert!(
        wire.contains("[vault:stage/target"),
        "the target travels as a capsule entry"
    );
    assert!(wire.contains(SAVE));
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
async fn the_model_is_asked_once_and_the_window_confirms_the_rest() {
    let dir = scratch("confirm");
    let stage = Arc::new(Mutex::new(Stage::default()));
    let (client, _inject) = compositor(dir.clone(), stage.clone()).await;
    let (url, requests) = grounding_server(
        "nvidia/LocateAnything-3B",
        &[(SAVE, "<box> 500, 500, 600, 600 </box>")],
        Duration::ZERO,
    )
    .await;
    let grounding = client_for(&url);
    let mut sight = session(&dir, client).await;
    let save = target("button:save", SAVE, PrivacyGrade::Internal);

    for pass in 0..3 {
        let sighting = sight
            .find_and_activate(&save, &grounding, Locality::Local, 100 + pass)
            .await
            .unwrap();
        assert_eq!(sighting.outcome.outcome_code, "clicked_and_repainted");
        assert_eq!(
            sighting.knowledge,
            if pass == 0 {
                Knowledge::Grounded
            } else {
                // Its own click redrew it, so it is looked at again, and the box still
                // holds the same control.
                Knowledge::Confirmed
            }
        );
    }
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "the model is asked once, however many times the button is pressed"
    );
    assert_eq!(stage.lock().unwrap().captures, 3);
    assert_eq!(stage.lock().unwrap().clicks.len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_target_nothing_has_touched_is_used_without_looking() {
    let dir = scratch("trusted");
    let stage = Arc::new(Mutex::new(Stage::default()));
    let (client, _inject) = compositor(dir.clone(), stage.clone()).await;
    let (url, requests) = grounding_server(
        "nvidia/LocateAnything-3B",
        &[
            (SAVE, "<box> 500, 500, 600, 600 </box>"),
            (NOTE, "<box> 100, 100, 300, 150 </box>"),
        ],
        Duration::ZERO,
    )
    .await;
    let grounding = client_for(&url);
    let mut sight = session(&dir, client).await;
    let save = target("button:save", SAVE, PrivacyGrade::Internal);
    let note = target("input:note", NOTE, PrivacyGrade::Internal);

    let first = sight
        .find_and_activate(&save, &grounding, Locality::Local, 200)
        .await
        .unwrap();
    assert_eq!(first.knowledge, Knowledge::Grounded);
    // Looking for the field also confirms the button, which is still drawn as it was.
    let field = sight
        .find_and_activate(&note, &grounding, Locality::Local, 201)
        .await
        .unwrap();
    assert_eq!(field.knowledge, Knowledge::Grounded);
    assert_eq!((field.point.x, field.point.y), (128, 60));

    let again = sight
        .find_and_activate(&save, &grounding, Locality::Local, 202)
        .await
        .unwrap();
    assert_eq!(
        again.knowledge,
        Knowledge::Trusted,
        "nothing was drawn over the button since it was last seen there"
    );
    assert!(again.before.is_none(), "so nothing was captured for it");
    assert_eq!(again.timings.look_ms, 0);
    assert_eq!(again.timings.ground_ms, 0);
    assert_eq!(stage.lock().unwrap().captures, 2);
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(stage.lock().unwrap().clicks.len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_click_that_changes_nothing_is_not_verification() {
    let dir = scratch("inert");
    let stage = Arc::new(Mutex::new(Stage {
        reacts: false,
        ..Stage::default()
    }));
    let (client, _inject) = compositor(dir.clone(), stage.clone()).await;
    let (url, requests) = grounding_server(
        "nvidia/LocateAnything-3B",
        &[(SAVE, "<box> 500, 500, 600, 600 </box>")],
        Duration::ZERO,
    )
    .await;
    let grounding = client_for(&url);
    let mut sight = session(&dir, client).await;
    let save = target("button:save", SAVE, PrivacyGrade::Internal);

    let refused = sight
        .find_and_activate(&save, &grounding, Locality::Local, 300)
        .await;
    assert!(
        matches!(
            refused,
            Err(SightError::Motor(MotorError::VerificationFailed))
        ),
        "{refused:?}"
    );
    let _ = sight
        .find_and_activate(&save, &grounding, Locality::Local, 301)
        .await;
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "a position that did not work is not kept"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_control_that_was_replaced_is_found_again() {
    let dir = scratch("replaced");
    let stage = Arc::new(Mutex::new(Stage::default()));
    let (client, inject) = compositor(dir.clone(), stage.clone()).await;
    let (url, requests) = grounding_server(
        "nvidia/LocateAnything-3B",
        &[(SAVE, "<box> 500, 500, 600, 600 </box>")],
        Duration::ZERO,
    )
    .await;
    let grounding = client_for(&url);
    let mut sight = session(&dir, client).await;
    let save = target("button:save", SAVE, PrivacyGrade::Internal);

    sight
        .find_and_activate(&save, &grounding, Locality::Local, 400)
        .await
        .unwrap();
    // The window now draws something else in that box, and says so.
    stage.lock().unwrap().bars = 8;
    inject
        .send(
            "{\"event\":\"damage\",\"surface\":4,\"rects\":[{\"x\":320,\"y\":240,\"width\":64,\"height\":48}]}\n"
                .to_string(),
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;

    let after = sight
        .find_and_activate(&save, &grounding, Locality::Local, 401)
        .await
        .unwrap();
    assert_eq!(
        after.knowledge,
        Knowledge::Grounded,
        "a box holding something else is not the control it held"
    );
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_window_that_moved_is_looked_at_again() {
    let dir = scratch("moved");
    let stage = Arc::new(Mutex::new(Stage::default()));
    let (client, inject) = compositor(dir.clone(), stage.clone()).await;
    let (url, requests) = grounding_server(
        "nvidia/LocateAnything-3B",
        &[(SAVE, "<box> 500, 500, 600, 600 </box>")],
        Duration::ZERO,
    )
    .await;
    let grounding = client_for(&url);
    let mut sight = session(&dir, client).await;
    let save = target("button:save", SAVE, PrivacyGrade::Internal);

    sight
        .find_and_activate(&save, &grounding, Locality::Local, 500)
        .await
        .unwrap();
    inject
        .send("{\"event\":\"surface\",\"surface\":4,\"app_id\":\"org.example.Form\",\"x\":10,\"y\":10,\"width\":640,\"height\":480,\"geometry_revision\":10}\n".to_string())
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;

    let after = sight
        .find_and_activate(&save, &grounding, Locality::Local, 501)
        .await
        .unwrap();
    assert_eq!(
        after.knowledge,
        Knowledge::Grounded,
        "where a control was in the old geometry says nothing about the new one"
    );
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cloud_model_of_unknown_retention_never_sees_an_internal_window() {
    let dir = scratch("cloud");
    let stage = Arc::new(Mutex::new(Stage::default()));
    let (client, _inject) = compositor(dir.clone(), stage.clone()).await;
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
    let mut sight = session(&dir, client).await;

    let result = sight
        .find_and_activate(
            &target("button:save", SAVE, PrivacyGrade::Internal),
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
        stage.lock().unwrap().clicks.is_empty(),
        "nothing is clicked on a grounding that never happened"
    );
}

/// An application that declares its own controls, as a toolkit's accessibility bridge does.
struct Declaring(Vec<archie_sight::Control>);

impl archie_sight::Declares for Declaring {
    fn controls(
        &self,
        pid: u32,
        window: (i32, i32),
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<archie_sight::Control>, archie_sight::DeclareError>,
                > + Send
                + '_,
        >,
    > {
        let controls = self.0.clone();
        let matches_window = window == (640, 480);
        Box::pin(async move {
            if pid == 4321 && matches_window {
                Ok(controls)
            } else {
                Err(archie_sight::DeclareError::NoApplication(pid))
            }
        })
    }
}

fn declared_button() -> archie_sight::Control {
    archie_sight::Control {
        name: "Save".into(),
        kind: NodeKind::Button,
        bounds: archie_iris::Rect::new(320, 240, 64, 48).unwrap(),
        path: "/org/a11y/atspi/accessible/7".into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_application_that_says_where_its_button_is_needs_no_model() {
    let dir = scratch("declared");
    let stage = Arc::new(Mutex::new(Stage::default()));
    let (client, _inject) = compositor(dir.clone(), stage.clone()).await;
    let (url, requests) = grounding_server(
        "nvidia/LocateAnything-3B",
        &[(SAVE, "<box> 500, 500, 600, 600 </box>")],
        Duration::ZERO,
    )
    .await;
    let grounding = client_for(&url);
    let mut sight = session(&dir, client)
        .await
        .asking(Box::new(Declaring(vec![declared_button()])));

    let sighting = sight
        .find_and_activate(
            &target("button:save", SAVE, PrivacyGrade::Internal),
            &grounding,
            Locality::Local,
            600,
        )
        .await
        .unwrap();

    assert_eq!(sighting.knowledge, Knowledge::Declared);
    assert_eq!((sighting.point.x, sighting.point.y), (352, 264));
    assert_eq!(sighting.outcome.outcome_code, "clicked_and_repainted");
    assert!(sighting.before.is_none(), "nothing was captured");
    assert_eq!(stage.lock().unwrap().captures, 0);
    assert!(
        requests.lock().unwrap().is_empty(),
        "and no model was asked where a button the application named already is"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_control_the_application_does_not_name_still_goes_to_the_model() {
    let dir = scratch("undeclared");
    let stage = Arc::new(Mutex::new(Stage::default()));
    let (client, _inject) = compositor(dir.clone(), stage.clone()).await;
    let (url, requests) = grounding_server(
        "nvidia/LocateAnything-3B",
        &[(SAVE, "<box> 500, 500, 600, 600 </box>")],
        Duration::ZERO,
    )
    .await;
    let grounding = client_for(&url);
    // Two controls with the same name: the window cannot say which one was meant.
    let mut sight = session(&dir, client).await.asking(Box::new(Declaring(vec![
        declared_button(),
        archie_sight::Control {
            name: "Save".into(),
            ..declared_button()
        },
    ])));

    let sighting = sight
        .find_and_activate(
            &target("button:save", SAVE, PrivacyGrade::Internal),
            &grounding,
            Locality::Local,
            601,
        )
        .await
        .unwrap();
    assert_eq!(sighting.knowledge, Knowledge::Grounded);
    assert_eq!(stage.lock().unwrap().captures, 1);
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_model_does_not_stop_the_work() {
    let dir = scratch("asking");
    let stage = Arc::new(Mutex::new(Stage::default()));
    let (client, _inject) = compositor(dir.clone(), stage.clone()).await;
    let (url, requests) = grounding_server(
        "nvidia/LocateAnything-3B",
        &[(SAVE, "<box> 500, 500, 600, 600 </box>")],
        Duration::from_millis(900),
    )
    .await;
    let grounding = std::sync::Arc::new(client_for(&url));
    // The application names its field and says nothing about its save button.
    let mut sight =
        session(&dir, client)
            .await
            .asking(Box::new(Declaring(vec![archie_sight::Control {
                name: "Note".into(),
                kind: NodeKind::Input,
                bounds: archie_iris::Rect::new(64, 48, 128, 24).unwrap(),
                path: "/org/a11y/atspi/accessible/9".into(),
            }])));
    let save = target("button:save", SAVE, PrivacyGrade::Internal);
    let mut note = target("input:note", "the Note field", PrivacyGrade::Internal);
    note.kind = NodeKind::Input;

    let asked = sight
        .start_finding(std::slice::from_ref(&save), &grounding, Locality::Local)
        .await
        .unwrap();
    assert_eq!(asked, 1, "one target nothing else can answer for");

    // While that model thinks, the field the application named is clicked anyway.
    let started = std::time::Instant::now();
    let field = sight
        .find_and_activate(&note, &grounding, Locality::Local, 700)
        .await
        .unwrap();
    assert_eq!(field.knowledge, Knowledge::Declared);
    assert!(
        started.elapsed() < Duration::from_millis(800),
        "the work did not wait for the model: {:?}",
        started.elapsed()
    );

    let button = sight
        .find_and_activate(&save, &grounding, Locality::Local, 701)
        .await
        .unwrap();
    assert_eq!(button.knowledge, Knowledge::Grounded);
    assert_eq!((button.point.x, button.point.y), (352, 264));
    assert!(
        button.before.is_none(),
        "the capture it was found in was taken before, not now"
    );
    assert_eq!(
        stage.lock().unwrap().captures,
        1,
        "one capture for the asking, and none for the clicking"
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
}

/// A local model that sees two controls in every window, as `archie-reflex` does.
struct Seeing(Vec<archie_iris::Rect>);

impl archie_sight::Sees for Seeing {
    fn controls_in(&self, _png: &[u8]) -> Vec<archie_iris::Rect> {
        self.0.clone()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_model_that_points_near_a_control_is_put_on_it() {
    let dir = scratch("snapped");
    let stage = Arc::new(Mutex::new(Stage::default()));
    let (client, _inject) = compositor(dir.clone(), stage.clone()).await;
    // The grounding model points 12 px above and left of the button the window draws.
    let (url, _requests) = grounding_server(
        "nvidia/LocateAnything-3B",
        &[(SAVE, "<box> 481, 475, 581, 575 </box>")],
        Duration::ZERO,
    )
    .await;
    let grounding = client_for(&url);
    let button = archie_iris::Rect::new(320, 240, 64, 48).unwrap();
    let mut sight = session(&dir, client)
        .await
        .recognising(std::sync::Arc::new(Seeing(vec![
            button,
            archie_iris::Rect::new(64, 48, 128, 24).unwrap(),
        ])));

    let sighting = sight
        .find_and_activate(
            &target("button:save", SAVE, PrivacyGrade::Internal),
            &grounding,
            Locality::Local,
            800,
        )
        .await
        .unwrap();
    assert_eq!(sighting.knowledge, Knowledge::Grounded);
    assert_eq!(
        (
            sighting.bbox.x,
            sighting.bbox.y,
            sighting.bbox.width,
            sighting.bbox.height
        ),
        (320, 240, 64, 48),
        "the box became the control the local model sees, not the one the model drew"
    );
    assert_eq!((sighting.point.x, sighting.point.y), (352, 264));
    assert_eq!(stage.lock().unwrap().clicks, vec![(352, 264)]);
}
