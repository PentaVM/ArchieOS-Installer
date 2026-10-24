//! A local model marked ready whose server is down: routing treats it as offline once its
//! `/health` says so, and the question goes to the next admissible candidate, or nowhere.
//!
//! The "cloud" model here is HTTPS on a closed loopback port. Asking it fails without
//! anything leaving the machine, and its name in that failure proves the question went
//! veiled to it instead of to the dead local server.

use archie_model_runtime::liveness::Liveness;
use archie_model_runtime::routed::{Disclosure, Question, Routed, RoutedError};
use archie_registry::ModelRegistry;
use archie_types::{
    Availability, DataRetention, Locality, ModelDescriptor, ModelId, ModelRole, PrivacyGrade,
    ProviderKind, TokenCost,
};
use archie_vault::Veil;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BONSAI: &str = "local.bonsai-2-9b";
const CLOUD: &str = "cloud.fixture-reasoner";

fn model(id: &str, provider: ProviderKind, locality: Locality, base_url: &str) -> ModelDescriptor {
    let mut labels = BTreeMap::from([
        ("base-url".to_string(), base_url.to_string()),
        ("provider-model".to_string(), "fixture".to_string()),
    ]);
    if locality == Locality::Cloud {
        labels.insert(
            "credential-env".to_string(),
            "ARCHIE_LIVENESS_TEST_KEY".to_string(),
        );
    }
    ModelDescriptor {
        id: ModelId::parse(id).unwrap(),
        display_name: id.into(),
        provider,
        locality,
        node: None,
        context_window: 16384,
        max_output_tokens: 4096,
        capabilities: vec![ModelRole::ReasoningWorker.capability()],
        max_privacy_grade: if locality == Locality::Cloud {
            PrivacyGrade::Public
        } else {
            PrivacyGrade::Restricted
        },
        retention: DataRetention::Unknown,
        availability: Availability::Ready,
        cost: TokenCost::FREE,
        latency_ema_ms: Some(2500),
        quality_tier: 55,
        required_accelerator_bytes: None,
        labels,
    }
}

async fn closed_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

/// Bonsai at `bonsai_url`, a veiled-only cloud model that cannot be reached, and a
/// private record of local servers so no other test's answers count here.
async fn routed(bonsai_url: &str) -> Routed {
    std::env::set_var(
        "ARCHIE_LIVENESS_TEST_KEY",
        "a-key-for-a-port-nobody-listens-on",
    );
    let cloud = format!("https://127.0.0.1:{}/v1/", closed_port().await);
    let mut registry = ModelRegistry::new();
    registry
        .register(model(
            BONSAI,
            ProviderKind::LlamaCpp,
            Locality::Local,
            bonsai_url,
        ))
        .unwrap();
    registry
        .register(model(
            CLOUD,
            ProviderKind::OpenAiCompatible,
            Locality::Cloud,
            &cloud,
        ))
        .unwrap();
    Routed::new(registry).with_liveness(Some(Arc::new(Liveness::new(
        Duration::from_secs(60),
        Duration::from_millis(300),
    ))))
}

fn question(privacy: PrivacyGrade) -> Question<'static> {
    Question {
        role: ModelRole::ReasoningWorker,
        privacy,
        system: "You plan the owner's week.",
        user: "Which errand comes first?",
        image: None,
        max_tokens: 64,
        timeout: Duration::from_secs(5),
    }
}

fn kept(disclosure: Disclosure) -> Vec<String> {
    match disclosure {
        Disclosure::Kept { models } => models.iter().map(ToString::to_string).collect(),
        other => panic!("expected kept, got {other:?}"),
    }
}

#[tokio::test]
async fn a_ready_bonsai_whose_server_is_down_goes_veiled_or_is_refused() {
    let dead = format!("http://127.0.0.1:{}/v1/", closed_port().await);
    let routed = routed(&dead).await;
    // The manifest alone still says ready: without asking, private text would go to it.
    assert_eq!(
        kept(
            routed
                .disclosure(&question(PrivacyGrade::Sensitive), &Veil::new())
                .unwrap()
        ),
        [BONSAI]
    );
    routed.refresh_liveness().await;
    match routed
        .disclosure(&question(PrivacyGrade::Sensitive), &Veil::new())
        .unwrap()
    {
        Disclosure::Veiled { models, .. } => {
            assert_eq!(models, [ModelId::parse(CLOUD).unwrap()])
        }
        other => panic!("sensitive text should go veiled, got {other:?}"),
    }
    assert!(matches!(
        routed.disclosure(&question(PrivacyGrade::Restricted), &Veil::new()),
        Err(RoutedError::Withheld(_))
    ));
    // Asked end to end: restricted text is refused without a call, sensitive text is
    // asked of the cloud model and never of the dead server.
    assert!(matches!(
        routed
            .ask_private(&question(PrivacyGrade::Restricted), &Veil::new())
            .await,
        Err(RoutedError::Withheld(_))
    ));
    match routed
        .ask_private(&question(PrivacyGrade::Sensitive), &Veil::new())
        .await
    {
        Err(RoutedError::AllFailed(tried)) => {
            let tried: Vec<_> = tried.iter().map(|(model, _)| model.to_string()).collect();
            assert_eq!(tried, [CLOUD]);
        }
        other => panic!("expected the unreachable cloud model to be tried, got {other:?}"),
    }
}

#[tokio::test]
async fn a_live_bonsai_keeps_private_text_on_the_machine() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let live = format!("http://{}/v1/", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let read = stream.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..read]).starts_with("GET /health "));
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await;
        }
    });
    let routed = routed(&live).await;
    routed.refresh_liveness().await;
    for grade in [PrivacyGrade::Sensitive, PrivacyGrade::Restricted] {
        assert_eq!(
            kept(routed.disclosure(&question(grade), &Veil::new()).unwrap()),
            [BONSAI],
            "{grade}"
        );
    }
}

/// The server answered its health check and stopped before the question reached it: the
/// question is not lost with it, it is decided again and goes veiled.
#[tokio::test]
async fn a_server_that_stops_after_its_health_answer_is_asked_again_and_passed_over() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 2048];
        let read = stream.read(&mut request).await.unwrap();
        assert!(String::from_utf8_lossy(&request[..read]).starts_with("GET /health "));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
        // Dropping the listener here stops the server.
    });
    let routed = routed(&url).await;
    routed.refresh_liveness().await;
    server.await.unwrap();
    match routed
        .ask_private(&question(PrivacyGrade::Sensitive), &Veil::new())
        .await
    {
        Err(RoutedError::AllFailed(tried)) => {
            let tried: Vec<_> = tried.iter().map(|(model, _)| model.to_string()).collect();
            assert_eq!(tried, [CLOUD], "the veiled path is the one tried last");
        }
        other => panic!("expected a fallback to the veiled path, got {other:?}"),
    }
}
