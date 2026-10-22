use archie_speech::SpeechToolIntentCandidate;
use archie_voice::actions::{ActionDispatcher, AsyncActions};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

fn intent() -> SpeechToolIntentCandidate {
    SpeechToolIntentCandidate {
        correlation_id: "turn#0".into(),
        tool_name: "look".into(),
        raw_arguments: "{}".into(),
        source_adapter: "test".into(),
        confidence: None,
    }
}

#[test]
fn dispatch_is_nonblocking_and_drop_cancels_the_owned_future() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let ran = Arc::new(AtomicBool::new(false));
    let reached = ran.clone();
    let mut actions = AsyncActions::new(
        runtime.handle().clone(),
        Duration::from_secs(1),
        move |_| {
            let reached = reached.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(80)).await;
                reached.store(true, Ordering::SeqCst);
                "late".to_string()
            }
        },
    );
    let at = Instant::now();
    let mut pending = actions.start(intent());
    assert!(at.elapsed() < Duration::from_millis(50));
    assert!(pending.poll().is_none());
    drop(pending);
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        !ran.load(Ordering::SeqCst),
        "cancelled tool must not continue after its wait"
    );
}

#[test]
fn timeouts_return_an_explicit_failure_instead_of_waiting_forever() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut actions = AsyncActions::new(
        runtime.handle().clone(),
        Duration::from_millis(20),
        |_| async { std::future::pending::<String>().await },
    );
    let mut pending = actions.start(intent());
    let deadline = Instant::now() + Duration::from_secs(2);
    let result = loop {
        if let Some(result) = pending.poll() {
            break result;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(result.contains("tiempo"), "{result}");
}
