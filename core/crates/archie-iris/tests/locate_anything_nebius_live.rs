use archie_iris::{CortexRequest, KeyframeReference, LocateAnythingClient, NodeId, Rect, SourceId};
use archie_model_runtime::{ProviderEndpoint, VisionImage, VisionMime};
use archie_types::{
    Availability, Capability, DataRetention, Locality, ModelDescriptor, ModelId, PrivacyGrade,
    ProviderKind, TokenCost,
};
use archie_vault::{ContextGateway, ContextRequest, EntryClass, Vault, VaultKey, VaultRef};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const NOW: i64 = 1_760_000_000_000;
const TASK_ID: &str = "iris-live-nebius";

fn model() -> ModelDescriptor {
    let mut labels = BTreeMap::new();
    labels.insert("provider-model".into(), "google/gemma-3-27b-it".into());
    ModelDescriptor {
        id: ModelId::parse("google.gemma-3-27b-it").unwrap(),
        display_name: "Gemma 3 27B IT on Nebius".into(),
        provider: ProviderKind::NebiusTokenFactory,
        locality: Locality::Cloud,
        node: None,
        context_window: 128_000,
        max_output_tokens: 128,
        capabilities: vec![Capability::known("compute.inference")],
        max_privacy_grade: PrivacyGrade::Public,
        retention: DataRetention::Unknown,
        availability: Availability::Ready,
        cost: TokenCost::FREE,
        latency_ema_ms: None,
        quality_tier: 50,
        required_accelerator_bytes: None,
        labels,
    }
}

fn capsule() -> archie_vault::ContextCapsule {
    let mut vault = Vault::open(&VaultKey::from_bytes([9; 32])).unwrap();
    let reference = VaultRef::parse("vault:synthetic/red-rectangle").unwrap();
    vault
        .store(
            "live-test",
            reference.clone(),
            EntryClass::Document,
            PrivacyGrade::Public,
            "the solid red rectangle on the white background",
            NOW,
            None,
        )
        .unwrap();
    ContextGateway::new()
        .assemble(
            &mut vault,
            &ContextRequest::new(
                TASK_ID,
                "iris-live-trace",
                "synthetic-visual-grounding",
                Locality::Cloud,
                120_000,
            )
            .including(reference),
            NOW,
        )
        .unwrap()
}

fn synthetic_png() -> Vec<u8> {
    let mut pixels = vec![255_u8; 256 * 256 * 3];
    for y in 80..176 {
        for x in 64..192 {
            let offset = (y * 256 + x) * 3;
            pixels[offset] = 220;
            pixels[offset + 1] = 20;
            pixels[offset + 2] = 20;
        }
    }
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, 256, 256);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
    }
    encoded
}

#[tokio::test]
#[ignore = "requires an explicitly supplied Nebius Token Factory credential"]
async fn public_capsule_maps_a_synthetic_object_through_nebius() {
    let image = synthetic_png();
    let request = CortexRequest {
        task_class: "synthetic-grounding-smoke".into(),
        scene_revision: 1,
        nodes: vec![NodeId::new("synthetic:red-rectangle").unwrap()],
        keyframes: vec![KeyframeReference {
            source: SourceId::new("synthetic:memory").unwrap(),
            captured_at_ms: NOW,
            roi: Rect::new(0, 0, 256, 256).unwrap(),
            content_sha256: format!("{:x}", Sha256::digest(&image)),
        }],
        privacy: PrivacyGrade::Public,
    };
    let endpoint = ProviderEndpoint::nebius("https://api.tokenfactory.nebius.com/v1").unwrap();
    let client = LocateAnythingClient::new(endpoint, model()).unwrap();
    let proposal = client
        .ground(
            TASK_ID,
            &request,
            &capsule(),
            VisionImage {
                mime: VisionMime::Png,
                bytes: &image,
                width: 256,
                height: 256,
            },
            NOW + 1,
        )
        .await
        .unwrap();
    assert_eq!(proposal.referenced_node.as_str(), "synthetic:red-rectangle");
    let center_x = proposal.bbox.x + (proposal.bbox.width / 2) as i32;
    let center_y = proposal.bbox.y + (proposal.bbox.height / 2) as i32;
    assert!((64..192).contains(&center_x));
    assert!((80..176).contains(&center_y));
}
