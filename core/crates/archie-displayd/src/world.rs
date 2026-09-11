//! Read-only loopback client for the separately sandboxed world-data adapter.
//!
//! This module has no provider URL and no Internet client. `archie-displayd` can only
//! request the normalized contract from `archie-worldd` on a fixed loopback address.

use archie_types::Timestamp;
pub use archie_worldd::{
    EarthquakeSeverity, GeoPosition, ObservationDetails, ObservationProvenance, ProviderCondition,
    ProviderRetryState, WorldDataSource, WorldObservation, WorldProviderId, WorldProviderStatus,
    WorldSnapshot,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::RwLock;

const WORLD_ENDPOINT: &str = "127.0.0.1:7667";
const MAX_NORMALIZED_RESPONSE_BYTES: usize = 512 * 1024;
const LOCAL_REQUEST_TIMEOUT: Duration = Duration::from_millis(1_500);

#[derive(Clone)]
pub struct WorldService {
    endpoint: Option<SocketAddr>,
    state: Arc<RwLock<WorldSnapshot>>,
}

impl WorldService {
    /// Connect to the fixed local adapter. There is deliberately no provider URL or
    /// routable-address option in this process.
    pub fn live() -> Self {
        Self::at_loopback(
            WORLD_ENDPOINT
                .parse()
                .expect("static world endpoint is valid"),
        )
    }

    pub fn disabled(source: WorldDataSource) -> Self {
        debug_assert!(source != WorldDataSource::LiveProvider);
        Self {
            endpoint: None,
            state: Arc::new(RwLock::new(WorldSnapshot::service_unavailable(
                source,
                Timestamp::now(),
            ))),
        }
    }

    fn at_loopback(endpoint: SocketAddr) -> Self {
        assert!(
            endpoint.ip().is_loopback(),
            "world adapter must be loopback-only"
        );
        Self {
            endpoint: Some(endpoint),
            state: Arc::new(RwLock::new(WorldSnapshot::service_unavailable(
                WorldDataSource::LiveProvider,
                Timestamp::now(),
            ))),
        }
    }

    pub async fn refresh(&self) {
        let Some(endpoint) = self.endpoint else {
            return;
        };
        let result = tokio::time::timeout(LOCAL_REQUEST_TIMEOUT, fetch_normalized(endpoint)).await;
        let mut state = self.state.write().await;
        match result {
            Ok(Ok(snapshot)) => *state = snapshot,
            Ok(Err(_)) | Err(_) => {
                *state = state.clone().with_transport_failure(Timestamp::now());
            }
        }
    }

    pub async fn current(&self) -> WorldSnapshot {
        self.state.read().await.clone()
    }
}

async fn fetch_normalized(endpoint: SocketAddr) -> Result<WorldSnapshot, &'static str> {
    if !endpoint.ip().is_loopback() {
        return Err("world endpoint is not loopback");
    }
    let mut stream = TcpStream::connect(endpoint)
        .await
        .map_err(|_| "world service unavailable")?;
    stream
        .write_all(b"GET /v1/world HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .map_err(|_| "world service unavailable")?;

    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8 * 1024];
    loop {
        let count = stream
            .read(&mut chunk)
            .await
            .map_err(|_| "world service unavailable")?;
        if count == 0 {
            break;
        }
        if bytes.len().saturating_add(count) > MAX_NORMALIZED_RESPONSE_BYTES {
            return Err("world response exceeded limit");
        }
        bytes.extend_from_slice(&chunk[..count]);
    }

    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("invalid world response")?;
    let head = std::str::from_utf8(&bytes[..split]).map_err(|_| "invalid world response")?;
    if !head
        .lines()
        .next()
        .is_some_and(|line| line.contains(" 200 "))
    {
        return Err("world service returned an error");
    }
    let snapshot: WorldSnapshot =
        serde_json::from_slice(&bytes[split + 4..]).map_err(|_| "invalid world contract")?;
    validate_contract(&snapshot)?;
    Ok(snapshot)
}

fn validate_contract(snapshot: &WorldSnapshot) -> Result<(), &'static str> {
    if snapshot.schema_version != 1
        || snapshot.source != WorldDataSource::LiveProvider
        || snapshot.providers.len() > 8
        || snapshot.observations.len() > 200
    {
        return Err("unsupported world contract");
    }
    for observation in &snapshot.observations {
        let p = observation.position;
        if observation.id.len() > 160
            || observation.summary.chars().count() > 160
            || !p.latitude.is_finite()
            || !(-90.0..=90.0).contains(&p.latitude)
            || !p.longitude.is_finite()
            || !(-180.0..=180.0).contains(&p.longitude)
            || !p.depth_km.is_finite()
        {
            return Err("invalid normalized observation");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn serve_once(body: Vec<u8>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 512];
            let _ = stream.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
        });
        address
    }

    #[tokio::test]
    async fn only_accepts_typed_normalized_loopback_data() {
        let snapshot = WorldSnapshot::service_unavailable(
            WorldDataSource::LiveProvider,
            Timestamp::from_millis(1),
        );
        let address = serve_once(serde_json::to_vec(&snapshot).unwrap()).await;
        let service = WorldService::at_loopback(address);
        service.refresh().await;
        assert_eq!(service.current().await.schema_version, 1);
    }

    #[tokio::test]
    async fn disconnect_keeps_cache_but_marks_it_unavailable() {
        let snapshot = WorldSnapshot::service_unavailable(
            WorldDataSource::LiveProvider,
            Timestamp::from_millis(1),
        );
        let address = serve_once(serde_json::to_vec(&snapshot).unwrap()).await;
        let service = WorldService::at_loopback(address);
        service.refresh().await;
        service.refresh().await;
        let current = service.current().await;
        assert_eq!(current.providers[0].condition, ProviderCondition::Degraded);
        assert_eq!(
            current.providers[0].error.as_deref(),
            Some("world service unavailable")
        );
    }

    #[test]
    #[should_panic(expected = "loopback-only")]
    fn refuses_a_routable_adapter() {
        let _ = WorldService::at_loopback("192.0.2.1:7667".parse().unwrap());
    }
}
