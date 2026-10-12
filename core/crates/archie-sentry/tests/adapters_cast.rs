//! Chromecast over CAST v2 against an in-process TLS receiver. Protocol tests
//! on loopback: no physical Chromecast has been driven by this adapter.
mod servers;

use archie_sentry::{
    adapters::chromecast::{probe_certificate, CastDevice, Chromecast},
    contracts::*,
    engine::*,
    registry::ResourceDescriptor,
};
use servers::cast::{CastReceiver, CERT, OTHER_CERT, OTHER_KEY};
use sha2::{Digest, Sha256};
use std::net::{IpAddr, Ipv4Addr};

const YOUTUBE: &str = "233637DE";
const MEDIA: &str = "CC1AD845";

fn pin(der: &[u8]) -> String {
    format!("{:x}", Sha256::digest(der))
}
fn resource() -> ResourceDescriptor {
    ResourceDescriptor {
        resource_id: "living-cast".into(),
        edge_id: "edge".into(),
        adapter_id: "cast".into(),
        paired: true,
        capabilities: Chromecast::CAPABILITIES.to_vec(),
    }
}
fn adapter(receiver: &CastReceiver, certificate: &[u8]) -> Chromecast {
    let device = CastDevice::new(
        "living-cast",
        "Living Room",
        "Chromecast",
        receiver.address.ip(),
        receiver.address.port(),
        &pin(certificate),
    )
    .unwrap();
    Chromecast::new("edge", "cast", vec![device]).unwrap()
}

#[test]
fn device_descriptor_is_bounded_and_needs_a_pin() {
    let address = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 30));
    let public = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));
    let good = pin(CERT);
    assert!(CastDevice::new("tv", "Living Room", "Chromecast", address, 8009, &good).is_ok());
    for (name, model, addr, port, fingerprint) in [
        ("x".repeat(65), "Chromecast", address, 8009, good.clone()),
        (
            "Living\nRoom".into(),
            "Chromecast",
            address,
            8009,
            good.clone(),
        ),
        ("Living Room".into(), "", address, 8009, good.clone()),
        (
            "Living Room".into(),
            "Chromecast",
            public,
            8009,
            good.clone(),
        ),
        ("Living Room".into(), "Chromecast", address, 0, good.clone()),
        (
            "Living Room".into(),
            "Chromecast",
            address,
            8009,
            String::new(),
        ),
        (
            "Living Room".into(),
            "Chromecast",
            address,
            8009,
            good.to_uppercase(),
        ),
    ] {
        assert!(CastDevice::new("tv", &name, model, addr, port, &fingerprint).is_err());
    }
    assert!(CastDevice::new("bad id!", "Living Room", "Chromecast", address, 8009, &good).is_err());
}

#[tokio::test]
async fn probe_reports_the_presented_certificate_and_sends_no_cast_message() {
    let receiver = CastReceiver::start(&[MEDIA]).await;
    let fingerprint = probe_certificate(receiver.address.ip(), receiver.address.port())
        .await
        .unwrap();
    assert_eq!(fingerprint, pin(CERT));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(receiver.log().is_empty(), "{:?}", receiver.log());
}

#[tokio::test]
async fn a_certificate_other_than_the_pinned_one_is_refused_before_any_cast_byte() {
    let receiver = CastReceiver::start_with(OTHER_CERT, OTHER_KEY, &[MEDIA]).await;
    let mut cast = adapter(&receiver, CERT);
    assert_eq!(
        cast.observe(&resource()).await,
        Err(AdapterError::Authentication)
    );
    let result = servers::execute(
        &resource(),
        &mut cast,
        Command::AppLaunch {
            app_id: MEDIA.into(),
        },
    )
    .await;
    assert_eq!(result.status, ExecutionStatus::Unavailable);
    assert!(result.dispatched_at.is_none());
    assert!(receiver.state.lock().unwrap().running.is_none());
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(receiver
        .log()
        .iter()
        .all(|entry| !entry.starts_with("receiver:") && !entry.starts_with("connection:")));
}

#[tokio::test]
async fn observe_reads_receiver_status_and_answers_the_heartbeat() {
    let receiver = CastReceiver::start(&[MEDIA]).await;
    let mut cast = adapter(&receiver, CERT);
    let observation = cast.observe(&resource()).await.unwrap();
    assert_eq!(observation.origin, ObservationOrigin::Measurement);
    assert_eq!(
        observation.state,
        ResourceState::Cast {
            active_app: None,
            volume_percent: Some(50),
            muted: Some(false),
            standby: Some(false),
        }
    );
    assert!(observation.evidence_id.starts_with("cast-"));
    // The CLOSE frame reaches the receiver task after observe returns.
    for _ in 0..100 {
        if receiver.log().len() >= 4 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        receiver.log(),
        [
            "connection:CONNECT",
            "receiver:GET_STATUS",
            "heartbeat:PONG",
            "connection:CLOSE"
        ]
    );
}

#[tokio::test]
async fn launch_volume_mute_and_stop_are_verified_against_the_receiver_status() {
    let receiver = CastReceiver::start(&[MEDIA, YOUTUBE]).await;
    let mut cast = adapter(&receiver, CERT);
    let r = resource();

    let launched = servers::execute(
        &r,
        &mut cast,
        Command::AppLaunch {
            app_id: YOUTUBE.into(),
        },
    )
    .await;
    assert_eq!(launched.status, ExecutionStatus::Verified, "{launched:?}");
    assert_eq!(
        receiver
            .state
            .lock()
            .unwrap()
            .running
            .as_ref()
            .map(|r| r.0.as_str()),
        Some(YOUTUBE)
    );
    assert!(matches!(
        launched.after.unwrap().state,
        ResourceState::Cast { active_app: Some(ref a), .. } if a == YOUTUBE
    ));

    let volume = servers::execute(&r, &mut cast, Command::VolumeSet { percent: 35 }).await;
    assert_eq!(volume.status, ExecutionStatus::Verified, "{volume:?}");
    assert!((receiver.state.lock().unwrap().level - 0.35).abs() < 1e-9);

    let muted = servers::execute(&r, &mut cast, Command::MuteSet { muted: true }).await;
    assert_eq!(muted.status, ExecutionStatus::Verified, "{muted:?}");
    assert!(receiver.state.lock().unwrap().muted);

    let stopped = servers::execute(&r, &mut cast, Command::AppStop).await;
    assert_eq!(stopped.status, ExecutionStatus::Verified, "{stopped:?}");
    assert!(receiver.state.lock().unwrap().running.is_none());
    // STOP named the session the receiver itself reported.
    assert!(receiver.log().iter().any(|e| e == "receiver:STOP"));
}

#[tokio::test]
async fn a_launch_the_receiver_refuses_is_not_verified_and_changes_nothing() {
    let receiver = CastReceiver::start(&[MEDIA]).await;
    let mut cast = adapter(&receiver, CERT);
    let result = servers::execute(
        &resource(),
        &mut cast,
        Command::AppLaunch {
            app_id: "0000BEEF".into(),
        },
    )
    .await;
    assert_eq!(result.status, ExecutionStatus::Indeterminate);
    assert!(result.after.is_none());
    assert!(receiver.state.lock().unwrap().running.is_none());
}

#[tokio::test]
async fn a_receiver_that_quantizes_volume_diverges_instead_of_pretending() {
    let receiver = CastReceiver::start(&[MEDIA]).await;
    receiver.state.lock().unwrap().step = 0.05;
    let mut cast = adapter(&receiver, CERT);
    let result = servers::execute(&resource(), &mut cast, Command::VolumeSet { percent: 33 }).await;
    assert_eq!(result.status, ExecutionStatus::Diverged);
    assert!(matches!(
        result.after.unwrap().state,
        ResourceState::Cast {
            volume_percent: Some(35),
            ..
        }
    ));
}

#[tokio::test]
async fn stop_with_nothing_running_does_not_dispatch() {
    let receiver = CastReceiver::start(&[MEDIA]).await;
    let mut cast = adapter(&receiver, CERT);
    let result = servers::execute(&resource(), &mut cast, Command::AppStop).await;
    assert_ne!(result.status, ExecutionStatus::Verified);
    assert!(result.dispatched_at.is_none());
    assert!(!receiver.log().iter().any(|e| e == "receiver:STOP"));
}

#[tokio::test]
async fn media_play_pause_and_unknown_resources_are_unsupported() {
    let receiver = CastReceiver::start(&[MEDIA]).await;
    let mut cast = adapter(&receiver, CERT);
    let mut r = resource();
    r.capabilities.push(CommandKind::MediaPause);
    assert_eq!(
        cast.apply(&r, &Command::MediaPause).await,
        Err(AdapterError::Unsupported)
    );
    r.resource_id = "other".into();
    assert_eq!(cast.observe(&r).await, Err(AdapterError::Unsupported));
    let mut unpaired = resource();
    unpaired.paired = false;
    assert_eq!(
        cast.observe(&unpaired).await,
        Err(AdapterError::Unsupported)
    );
    assert!(receiver.log().is_empty());
}
