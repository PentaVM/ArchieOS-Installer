//! The client against a test compositor over an in-memory pipe.

use archie_policy::{Action, CapabilityRequest, Constraints, PolicyEngine, Principal, Scope};
use archie_stage::{
    open, stage_resource, AuthorityMode, ClientError, CommandKey, DamageRect, PointerButton,
    Repainted, Sealer, StageAction, StageClient, StageEvent, StageGate, StagePolicy, StageProposal,
    StageWorld, MAX_EVENT_BYTES,
};
use archie_types::{AgentId, PrivacyGrade, Timestamp, TraceId};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const KEY: [u8; 32] = [3; 32];

#[tokio::test]
async fn a_command_reaches_the_compositor_and_its_receipt_comes_back() {
    let (client_side, compositor_side) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = tokio::io::split(client_side);
    let (compositor_read, mut compositor_write) = tokio::io::split(compositor_side);

    // The compositor announces a surface and hands the stage to the agent.
    compositor_write
        .write_all(
            b"{\"event\":\"surface\",\"surface\":4,\"app_id\":\"org.freecad.FreeCAD\",\"x\":10,\"y\":20,\"width\":800,\"height\":600,\"geometry_revision\":77}\n\
              {\"event\":\"authority\",\"mode\":\"autonomous\",\"reason\":\"resumed\"}\n",
        )
        .await
        .unwrap();

    let mut client = StageClient::new(client_read, client_write);
    let mut world = StageWorld::default();
    assert!(
        !world.agent_may_act(),
        "nobody has handed the agent anything yet"
    );
    for _ in 0..2 {
        let event = client.next_event().await.unwrap().unwrap();
        world.apply(&event);
    }
    assert!(world.agent_may_act());
    let surface = world.surface(4).unwrap().clone();

    // Prepare against the mirror, gate, seal, send.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("policy/apps.toml");
    let policy = StagePolicy::load(path).unwrap();
    let principal = Principal::Agent {
        agent: AgentId::new(),
    };
    let lease = PolicyEngine::default()
        .decide(&CapabilityRequest {
            trace_id: TraceId::new(),
            task_id: None,
            principal: principal.clone(),
            resource: stage_resource(&surface.app_id),
            scope: Scope::OneOperation { operation: 1 },
            constraints: Constraints::with_actions([Action::Execute]),
            justification: "click".into(),
        })
        .lease
        .unwrap();
    let proposal = StageProposal {
        app_id: surface.app_id.clone(),
        surface: surface.surface,
        geometry_revision: surface.geometry_revision,
        action: StageAction::Click {
            x: 50,
            y: 60,
            button: PointerButton::Left,
        },
        text_grade: PrivacyGrade::Public,
        operation_id: 1,
        task_id: None,
        trace_id: "trace-client".into(),
    };
    let now = Timestamp::now();
    let command = StageGate::authorize(&policy, &lease, &principal, false, &proposal, now)
        .allowed()
        .unwrap()
        .clone();
    let mut sealer = Sealer::new(CommandKey::from_bytes(&KEY), 1);
    let sealed = sealer.seal(&command, now, 2_000).unwrap();
    client.send(&sealed).await.unwrap();

    // The test compositor verifies exactly as the real one must, then reports.
    let mut lines = BufReader::new(compositor_read);
    let mut line = String::new();
    lines.read_line(&mut line).await.unwrap();
    let wire = open(&CommandKey::from_bytes(&KEY), line.as_bytes(), 0, now).unwrap();
    assert_eq!(wire.geometry_revision, 77);
    compositor_write
        .write_all(format!("{{\"event\":\"receipt\",\"seq\":{},\"outcome\":\"executed\",\"observed_revision\":78}}\n", wire.seq).as_bytes())
        .await
        .unwrap();

    let receipt = client.next_event().await.unwrap().unwrap();
    world.apply(&receipt);
    assert_eq!(world.last_receipt(), Some((1, "executed")));
}

#[tokio::test]
async fn the_mirror_follows_what_the_compositor_reports() {
    let mut world = StageWorld::default();
    world.apply(&StageEvent::Surface {
        surface: 1,
        app_id: "foot".into(),
        x: 0,
        y: 0,
        width: 100,
        height: 100,
        geometry_revision: 5,
        pid: 4321,
    });
    world.apply(&StageEvent::Focus { surface: Some(1) });
    world.apply(&StageEvent::Authority {
        mode: AuthorityMode::Autonomous,
        reason: "start".into(),
    });
    assert_eq!(world.surfaces_of("foot").count(), 1);
    world.apply(&StageEvent::Authority {
        mode: AuthorityMode::Operator,
        reason: "operator_button".into(),
    });
    assert!(!world.agent_may_act());
    world.apply(&StageEvent::Unmapped { surface: 1 });
    assert!(world.surface(1).is_none());
    assert_eq!(world.focus(), None, "focus does not outlive its surface");
}

#[tokio::test]
async fn an_unbounded_or_invalid_event_is_not_believed() {
    let (client_side, compositor_side) = tokio::io::duplex(256 * 1024);
    let (client_read, client_write) = tokio::io::split(client_side);
    let (_compositor_read, mut compositor_write) = tokio::io::split(compositor_side);
    let mut client = StageClient::new(client_read, client_write);

    compositor_write
        .write_all(b"{\"event\":\"surface\",\"surface\":1,\"app_id\":\"a\",\"x\":0,\"y\":0,\"width\":0,\"height\":10,\"geometry_revision\":1}\n")
        .await
        .unwrap();
    assert!(matches!(
        client.next_event().await,
        Err(ClientError::Malformed)
    ));

    compositor_write
        .write_all(b"{\"event\":\"receipt\",\"seq\":1,\"outcome\":\"ran it, all good\",\"observed_revision\":2}\n")
        .await
        .unwrap();
    assert!(matches!(
        client.next_event().await,
        Err(ClientError::Malformed)
    ));

    compositor_write
        .write_all(b"{\"event\":\"focus\",\"surface\":null,\"extra\":1}\n")
        .await
        .unwrap();
    assert!(matches!(
        client.next_event().await,
        Err(ClientError::Malformed)
    ));

    let mut flood = vec![b'x'; MAX_EVENT_BYTES + 10];
    flood.push(b'\n');
    compositor_write.write_all(&flood).await.unwrap();
    compositor_write
        .write_all(b"{\"event\":\"focus\",\"surface\":null}\n")
        .await
        .unwrap();
    assert!(matches!(
        client.next_event().await,
        Err(ClientError::LineTooLong)
    ));
    assert!(
        matches!(client.next_event().await, Err(ClientError::LineTooLong)),
        "after an oversized line the stream's framing cannot be trusted again"
    );

    drop(compositor_write);
}

#[tokio::test]
async fn a_wait_that_gives_up_mid_line_keeps_its_place() {
    let (client_side, compositor_side) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = tokio::io::split(client_side);
    let (_compositor_read, mut compositor_write) = tokio::io::split(compositor_side);
    let mut client = StageClient::new(client_read, client_write);

    // Half an event, and a caller that stops waiting for the rest of it.
    compositor_write
        .write_all(b"{\"event\":\"damage\",\"surface\":4,\"rects\":[{\"x\":10,\"y\":")
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), client.next_event())
            .await
            .is_err(),
        "nothing to report yet"
    );
    compositor_write
        .write_all(b"20,\"width\":30,\"height\":40}]}\n")
        .await
        .unwrap();
    assert_eq!(
        client.next_event().await.unwrap().unwrap(),
        StageEvent::Damage {
            surface: 4,
            rects: vec![DamageRect {
                x: 10,
                y: 20,
                width: 30,
                height: 40
            }]
        },
        "the line resumes where the abandoned read left it, not in the middle of it"
    );
}

#[test]
fn the_mirror_remembers_what_repainted_where() {
    let mut world = StageWorld::default();
    let button = DamageRect {
        x: 100,
        y: 100,
        width: 40,
        height: 20,
    };
    let ring = DamageRect {
        x: 96,
        y: 96,
        width: 2,
        height: 2,
    };
    assert!(!button.intersects(&ring));
    assert!(
        button.grown(8).intersects(&ring),
        "a focus ring is drawn just outside the control it belongs to"
    );

    let before = world.damage_mark();
    assert_eq!(world.repainted_since(before, 4, &button), Repainted::No);
    world.apply(&StageEvent::Damage {
        surface: 4,
        rects: vec![DamageRect {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        }],
    });
    assert_eq!(
        world.repainted_since(before, 4, &button),
        Repainted::No,
        "a corner of the window is not that button"
    );
    world.apply(&StageEvent::Damage {
        surface: 4,
        rects: vec![DamageRect {
            x: 130,
            y: 110,
            width: 20,
            height: 20,
        }],
    });
    assert_eq!(world.repainted_since(before, 4, &button), Repainted::Yes);

    let after = world.damage_mark();
    assert_eq!(
        world.repainted_since(after, 4, &button),
        Repainted::No,
        "asked again, it is a new question"
    );
    world.apply(&StageEvent::Damage {
        surface: 5,
        rects: vec![DamageRect {
            x: 130,
            y: 110,
            width: 20,
            height: 20,
        }],
    });
    assert_eq!(
        world.repainted_since(after, 4, &button),
        Repainted::No,
        "another window redrawing itself says nothing about this one"
    );

    for _ in 0..600 {
        world.apply(&StageEvent::Damage {
            surface: 4,
            rects: vec![DamageRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }],
        });
    }
    assert_eq!(
        world.repainted_since(after, 4, &button),
        Repainted::Unknown,
        "past what it keeps, the mirror says it cannot tell instead of guessing"
    );
}
