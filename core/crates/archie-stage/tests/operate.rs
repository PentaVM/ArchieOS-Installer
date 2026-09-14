//! One action the whole way through, against a test compositor over an in-memory pipe.

use archie_policy::{PolicyEngine, Principal};
use archie_stage::{
    learn, open, ActOutcome, ActRequest, CommandKey, Key, KeyChord, NamedKey, Operator,
    PointerButton, StageAction, StageClient, StagePolicy, StageWorld, WireAction,
};
use archie_types::{AgentId, PrivacyGrade, Timestamp};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

const KEY: [u8; 32] = [9; 32];
const GREETING: &[u8] = b"{\"event\":\"surface\",\"surface\":4,\"app_id\":\"org.freecad.FreeCAD\",\"x\":10,\"y\":20,\"width\":800,\"height\":600,\"geometry_revision\":77}\n\
{\"event\":\"surface\",\"surface\":5,\"app_id\":\"foot\",\"x\":900,\"y\":20,\"width\":600,\"height\":400,\"geometry_revision\":3}\n\
{\"event\":\"authority\",\"mode\":\"autonomous\",\"reason\":\"start\"}\n";

type Client = StageClient<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

fn policy() -> StagePolicy {
    StagePolicy::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("policy/apps.toml")).unwrap()
}

fn request(app: &str, action: StageAction) -> ActRequest {
    ActRequest {
        app_id: app.into(),
        surface: None,
        action,
        text_grade: PrivacyGrade::Public,
        principal: Principal::Agent {
            agent: AgentId::new(),
        },
        task_id: None,
        user_present: false,
        approved: false,
        justification: "test".into(),
    }
}

/// A compositor that greets, then answers every verified line with `outcome`, and every
/// unverifiable one with sequence 0 — and reports what it received.
async fn start(outcome: &'static str) -> (Client, tokio::task::JoinHandle<Vec<WireAction>>) {
    let (client_side, compositor_side) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = tokio::io::split(client_side);
    let (compositor_read, mut compositor_write) = tokio::io::split(compositor_side);
    compositor_write.write_all(GREETING).await.unwrap();
    let compositor = tokio::spawn(async move {
        let key = CommandKey::from_bytes(&KEY);
        let mut lines = BufReader::new(compositor_read).lines();
        let mut received = Vec::new();
        let mut last = 0;
        while let Ok(Some(line)) = lines.next_line().await {
            let reply = match open(&key, format!("{line}\n").as_bytes(), last, Timestamp::now()) {
                Ok(command) => {
                    last = command.seq;
                    received.push(command.action.clone());
                    format!(
                        "{{\"event\":\"receipt\",\"seq\":{},\"outcome\":\"{outcome}\",\"observed_revision\":1}}\n",
                        command.seq
                    )
                }
                Err(error) => format!(
                    "{{\"event\":\"receipt\",\"seq\":0,\"outcome\":\"{}\",\"observed_revision\":1}}\n",
                    error.code()
                ),
            };
            if compositor_write.write_all(reply.as_bytes()).await.is_err() {
                break;
            }
        }
        received
    });
    (StageClient::new(client_read, client_write), compositor)
}

async fn learned(client: &mut Client) -> StageWorld {
    let mut world = StageWorld::default();
    learn(client, &mut world, Duration::from_millis(50)).await.unwrap();
    world
}

#[tokio::test]
async fn an_allowed_click_is_sealed_sent_and_confirmed() {
    let (mut client, compositor) = start("executed").await;
    let mut world = learned(&mut client).await;
    let mut operator = Operator::new(CommandKey::from_bytes(&KEY), policy(), PolicyEngine::default());
    let click = StageAction::Click {
        x: 40,
        y: 50,
        button: PointerButton::Left,
    };
    let outcome = operator
        .act(&mut client, &mut world, &request("org.freecad.FreeCAD", click), Duration::from_secs(5))
        .await
        .unwrap();
    assert!(outcome.executed(), "{outcome:?}");

    // A second action from the same operator seals a higher sequence and is accepted too.
    let focus = operator
        .act(&mut client, &mut world, &request("org.freecad.FreeCAD", StageAction::Focus), Duration::from_secs(5))
        .await
        .unwrap();
    assert!(focus.executed(), "{focus:?}");

    drop(client);
    let received = compositor.await.unwrap();
    assert_eq!(received.len(), 2);
}

#[tokio::test]
async fn nothing_is_sent_for_an_application_or_action_policy_does_not_allow() {
    let (mut client, compositor) = start("executed").await;
    let mut world = learned(&mut client).await;
    let mut operator = Operator::new(CommandKey::from_bytes(&KEY), policy(), PolicyEngine::default());

    // foot is in the policy, but dragging in it is not.
    let drag = StageAction::Drag {
        x: 1,
        y: 1,
        to_x: 50,
        to_y: 50,
        button: PointerButton::Left,
    };
    let refused = operator
        .act(&mut client, &mut world, &request("foot", drag), Duration::from_secs(1))
        .await
        .unwrap();
    assert!(matches!(refused, ActOutcome::Refused { .. }), "{refused:?}");

    // Enter in a terminal runs a command: the policy holds it for a person.
    let enter = StageAction::Key {
        chord: KeyChord::key(Key::Named(NamedKey::Enter)),
    };
    let held = operator
        .act(&mut client, &mut world, &request("foot", enter), Duration::from_secs(1))
        .await
        .unwrap();
    assert!(matches!(held, ActOutcome::NeedsApproval { .. }), "{held:?}");

    // An application with no surface on the stage.
    let absent = operator
        .act(&mut client, &mut world, &request("org.gnome.Nautilus", StageAction::Focus), Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(absent, ActOutcome::NoSurface);

    // A surface id that belongs to another application is not borrowed.
    let mut wrong = request("foot", StageAction::Focus);
    wrong.surface = Some(4);
    let borrowed = operator
        .act(&mut client, &mut world, &wrong, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(borrowed, ActOutcome::NoSurface);

    drop(client);
    assert!(compositor.await.unwrap().is_empty(), "none of those may reach the compositor");
}

#[tokio::test]
async fn an_approved_action_goes_through_and_a_wrong_key_is_answered_with_sequence_zero() {
    let (mut client, compositor) = start("executed").await;
    let mut world = learned(&mut client).await;

    let mut approved = request(
        "foot",
        StageAction::Key {
            chord: KeyChord::key(Key::Named(NamedKey::Enter)),
        },
    );
    approved.approved = true;
    approved.user_present = true;
    let mut operator = Operator::new(CommandKey::from_bytes(&KEY), policy(), PolicyEngine::default());
    let outcome = operator
        .act(&mut client, &mut world, &approved, Duration::from_secs(5))
        .await
        .unwrap();
    assert!(outcome.executed(), "{outcome:?}");

    let mut impostor = Operator::new(CommandKey::from_bytes(&[1; 32]), policy(), PolicyEngine::default());
    let rejected = impostor
        .act(&mut client, &mut world, &request("foot", StageAction::Focus), Duration::from_secs(5))
        .await
        .unwrap();
    match rejected {
        ActOutcome::Receipt { sequence, outcome } => {
            assert_eq!(sequence, 0);
            assert_ne!(outcome, "executed");
        }
        other => panic!("expected a sequence-0 receipt, got {other:?}"),
    }

    drop(client);
    assert_eq!(compositor.await.unwrap().len(), 1);
}
