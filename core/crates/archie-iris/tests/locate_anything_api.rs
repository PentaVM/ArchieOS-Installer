use archie_iris::{
    CortexRequest, KeyframeReference, LocateAnythingClient, LocateAnythingError, NodeId, Rect,
    SourceId,
};
use archie_model_runtime::{ProviderEndpoint, VisionImage, VisionMime};
use archie_types::{
    Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId, PrivacyGrade,
    ProviderKind, TokenCost,
};
use archie_vault::{ContextGateway, ContextRequest, EntryClass, Vault, VaultKey, VaultRef};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const NOW: i64 = 1_760_000_000_000;

fn model() -> ModelDescriptor {
    let mut labels = BTreeMap::new();
    labels.insert("provider-model".into(), "nvidia/LocateAnything-3B".into());
    ModelDescriptor {
        id: ModelId::parse("nvidia.locate-anything-3b").unwrap(),
        display_name: "NVIDIA LocateAnything 3B".into(),
        provider: ProviderKind::NvidiaNim,
        locality: Locality::Local,
        node: None,
        context_window: 24_000,
        max_output_tokens: 512,
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

fn capsule(destination: Locality) -> archie_vault::ContextCapsule {
    let mut vault = Vault::open(&VaultKey::from_bytes([7; 32])).unwrap();
    let reference = VaultRef::parse("vault:iris/target").unwrap();
    vault
        .store(
            "test",
            reference.clone(),
            EntryClass::PrivateFact,
            PrivacyGrade::Restricted,
            "search button",
            NOW,
            None,
        )
        .unwrap();
    ContextGateway::new()
        .assemble(
            &mut vault,
            &ContextRequest::new(
                "iris-task",
                "iris-trace",
                "visual-grounding",
                destination,
                5_000,
            )
            .including(reference),
            NOW,
        )
        .unwrap()
}

fn request(image: &[u8]) -> CortexRequest {
    CortexRequest {
        task_class: "gui-grounding".into(),
        scene_revision: 7,
        nodes: vec![NodeId::new("button:search").unwrap()],
        keyframes: vec![KeyframeReference {
            source: SourceId::new("browser:demo").unwrap(),
            captured_at_ms: NOW,
            roi: Rect::new(0, 0, 32, 24).unwrap(),
            content_sha256: format!("{:x}", Sha256::digest(image)),
        }],
        privacy: PrivacyGrade::Restricted,
    }
}

#[tokio::test]
async fn capsule_to_locate_anything_api_to_archie_geometry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = vec![0; 32 * 1024];
        let read = socket.read(&mut bytes).await.unwrap();
        bytes.truncate(read);
        let response = r#"{"model":"nvidia/LocateAnything-3B","choices":[{"message":{"role":"assistant","content":"<box> 100, 200, 700, 800 </box>"},"finish_reason":"stop"}]}"#;
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        String::from_utf8(bytes).unwrap()
    });
    let endpoint =
        ProviderEndpoint::nvidia_nim(&format!("http://{address}/v1"), Locality::Local).unwrap();
    let client = LocateAnythingClient::new(endpoint, model()).unwrap();
    let png = b"\x89PNG\r\n\x1a\nsynthetic-public-fixture";
    let proposal = client
        .ground(
            "iris-task",
            &request(png),
            &capsule(Locality::Local),
            VisionImage {
                mime: VisionMime::Png,
                bytes: png,
                width: 32,
                height: 24,
            },
            NOW + 1,
        )
        .await
        .unwrap();
    assert_eq!(proposal.referenced_node.as_str(), "button:search");
    assert_eq!(proposal.bbox, Rect::new(3, 4, 19, 15).unwrap());
    let wire = server.await.unwrap();
    assert!(wire.contains("[vault:iris/target"));
    assert!(wire.contains("search button"));
    assert!(wire.contains("data:image/png;base64,"));
    assert!(wire.contains(r#""model":"nvidia/LocateAnything-3B""#));
}

#[tokio::test]
async fn digest_mismatch_fails_before_any_api_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let endpoint =
        ProviderEndpoint::nvidia_nim(&format!("http://{address}/v1"), Locality::Local).unwrap();
    let client = LocateAnythingClient::new(endpoint, model()).unwrap();
    let png = b"\x89PNG\r\n\x1a\nsynthetic-public-fixture";
    let mut mismatched = request(png);
    mismatched.keyframes[0].content_sha256 = "0".repeat(64);
    assert!(matches!(
        client
            .ground(
                "iris-task",
                &mismatched,
                &capsule(Locality::Local),
                VisionImage {
                    mime: VisionMime::Png,
                    bytes: png,
                    width: 32,
                    height: 24,
                },
                NOW + 1,
            )
            .await,
        Err(LocateAnythingError::InvalidRequest)
    ));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn capsule_from_another_task_fails_before_any_api_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let endpoint =
        ProviderEndpoint::nvidia_nim(&format!("http://{address}/v1"), Locality::Local).unwrap();
    let client = LocateAnythingClient::new(endpoint, model()).unwrap();
    let png = b"\x89PNG\r\n\x1a\nsynthetic-public-fixture";
    assert!(matches!(
        client
            .ground(
                "different-task",
                &request(png),
                &capsule(Locality::Local),
                VisionImage {
                    mime: VisionMime::Png,
                    bytes: png,
                    width: 32,
                    height: 24,
                },
                NOW + 1,
            )
            .await,
        Err(LocateAnythingError::InvalidRequest)
    ));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}
