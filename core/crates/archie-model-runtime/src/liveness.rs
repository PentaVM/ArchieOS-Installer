//! Whether a local model's server answers now.
//!
//! A manifest's `ready` is what the node wrote when it installed its copy. A llama-server
//! that has since stopped, or is still loading its weights, reads `ready` all the same,
//! and every private and voice question would go to it and fail there. Before routing,
//! each such model's server is asked for `/health`, briefly, and the answer is kept for a
//! few seconds. A server that refuses the connection, does not answer in time, or answers
//! with a server error counts as offline until it answers again, so the router chooses
//! the next admissible candidate instead: a cloud model for veiled text when the grade
//! allows it, none otherwise.
//!
//! Only llama.cpp models are probed: their server documents `/health`, and it is where
//! Archie's own Bonsai runs. Nothing about the question is sent, and a cloud model is
//! never probed: a probe would tell a third party when the operator is working.

use crate::ProviderEndpoint;
use archie_types::{Locality, ModelDescriptor, ProviderKind};
use reqwest::Url;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// What the servers of the node's local models said, and when.
pub struct Liveness {
    ttl: Duration,
    timeout: Duration,
    client: Option<reqwest::Client>,
    seen: Mutex<HashMap<Url, Seen>>,
}

#[derive(Debug, Clone, Copy)]
struct Seen {
    at: Instant,
    live: bool,
}

impl Liveness {
    /// How long one answer stands.
    pub const TTL: Duration = Duration::from_secs(5);
    /// How long a loopback server has to answer `/health`.
    pub const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

    pub fn new(ttl: Duration, timeout: Duration) -> Self {
        // The probe goes straight to the configured server: an environment proxy would
        // answer for it, and a redirect could point it anywhere.
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .ok();
        Self {
            ttl,
            timeout,
            client,
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// The process's own record, shared by every router it builds, so a daemon that
    /// builds one per request still asks each server at most once per [`Self::TTL`].
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<Liveness>> = OnceLock::new();
        SHARED
            .get_or_init(|| Arc::new(Self::new(Self::TTL, Self::PROBE_TIMEOUT)))
            .clone()
    }

    /// Where `model`'s server is asked, if it is one whose server is asked: a usable,
    /// non-cloud llama.cpp model.
    pub fn health_url(model: &ModelDescriptor) -> Option<Url> {
        if model.provider != ProviderKind::LlamaCpp
            || model.locality == Locality::Cloud
            || !model.availability.usable()
        {
            return None;
        }
        ProviderEndpoint::for_model(model)
            .ok()?
            .base_url()
            .join("/health")
            .ok()
    }

    /// Whether `model`'s server was found down within the last [`Self::TTL`]. A model
    /// never asked about, or asked about too long ago, is taken at its manifest's word.
    pub fn is_down(&self, model: &ModelDescriptor) -> bool {
        Self::health_url(model)
            .and_then(|url| self.fresh(&url))
            .is_some_and(|live| !live)
    }

    /// Ask the server of each of `models` whose last answer is stale, all at once.
    pub async fn refresh<'a>(&self, models: impl IntoIterator<Item = &'a ModelDescriptor>) {
        let mut stale: Vec<Url> = models
            .into_iter()
            .filter_map(Self::health_url)
            .filter(|url| self.fresh(url).is_none())
            .collect();
        stale.sort();
        stale.dedup();
        if stale.is_empty() {
            return;
        }
        let answers = futures_util::future::join_all(
            stale
                .into_iter()
                .map(|url| async move { (self.probe(&url).await, url) }),
        )
        .await;
        let mut seen = self.seen.lock().unwrap_or_else(|p| p.into_inner());
        for (live, url) in answers {
            if let Some(live) = live {
                seen.insert(
                    url,
                    Seen {
                        at: Instant::now(),
                        live,
                    },
                );
            }
        }
    }

    /// Forget what `model`'s server said, so the next refresh asks it again.
    pub fn forget(&self, model: &ModelDescriptor) {
        if let Some(url) = Self::health_url(model) {
            self.seen
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&url);
        }
    }

    fn fresh(&self, url: &Url) -> Option<bool> {
        self.seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(url)
            .filter(|seen| seen.at.elapsed() < self.ttl)
            .map(|seen| seen.live)
    }

    /// `None` when this process cannot ask at all; then the manifest stands.
    async fn probe(&self, url: &Url) -> Option<bool> {
        let client = self.client.as_ref()?;
        let answered = tokio::time::timeout(self.timeout, client.get(url.clone()).send()).await;
        // Any answer but a server error is a server that is up: one without a `/health`
        // route (404) still serves, and llama-server says 503 while it loads.
        Some(matches!(answered, Ok(Ok(response)) if !response.status().is_server_error()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_types::{Availability, DataRetention, ModelId, ModelRole, PrivacyGrade, TokenCost};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn llama(id: &str, base_url: &str) -> ModelDescriptor {
        ModelDescriptor {
            id: ModelId::parse(id).unwrap(),
            display_name: id.into(),
            provider: ProviderKind::LlamaCpp,
            locality: Locality::Local,
            node: None,
            context_window: 16384,
            max_output_tokens: 4096,
            capabilities: vec![ModelRole::ReasoningWorker.capability()],
            max_privacy_grade: PrivacyGrade::Restricted,
            retention: DataRetention::Unknown,
            availability: Availability::Ready,
            cost: TokenCost::FREE,
            latency_ema_ms: Some(2500),
            quality_tier: 55,
            required_accelerator_bytes: None,
            labels: BTreeMap::from([
                ("base-url".to_string(), base_url.to_string()),
                ("provider-model".to_string(), "bonsai".to_string()),
            ]),
        }
    }

    /// A port nothing listens on: bound, then released.
    async fn closed_port() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/", listener.local_addr().unwrap());
        drop(listener);
        url
    }

    /// A server that answers every request with `status`, counting them, and keeping
    /// the first request line it saw.
    async fn server(status: &'static str) -> (String, Arc<AtomicUsize>, Arc<Mutex<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let first = Arc::new(Mutex::new(String::new()));
        let (counted, line) = (count.clone(), first.clone());
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                counted.fetch_add(1, Ordering::SeqCst);
                let mut buffer = [0; 2048];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                {
                    let mut first = line.lock().unwrap();
                    if first.is_empty() {
                        *first = request.lines().next().unwrap_or_default().to_string();
                    }
                }
                let body = r#"{"status":"ok"}"#;
                let _ = stream
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await;
            }
        });
        (url, count, first)
    }

    #[tokio::test]
    async fn a_ready_manifest_whose_server_is_gone_is_down() {
        let liveness = Liveness::new(Duration::from_secs(60), Duration::from_millis(300));
        let bonsai = llama("local.bonsai-2-9b", &closed_port().await);
        assert!(
            !liveness.is_down(&bonsai),
            "never asked: the manifest stands"
        );
        liveness.refresh([&bonsai]).await;
        assert!(liveness.is_down(&bonsai));
    }

    #[tokio::test]
    async fn a_live_server_is_asked_its_health_once_per_ttl_and_never_the_question() {
        let liveness = Liveness::new(Duration::from_secs(60), Duration::from_millis(300));
        let (url, count, first) = server("200 OK").await;
        let bonsai = llama("local.bonsai-2-9b", &url);
        let fast = llama("local.bonsai-2-9b-fast", &url);
        liveness.refresh([&bonsai, &fast]).await;
        assert!(!liveness.is_down(&bonsai));
        assert!(!liveness.is_down(&fast));
        assert_eq!(first.lock().unwrap().as_str(), "GET /health HTTP/1.1");
        // Two manifests on one server are one question, and a fresh answer is kept.
        liveness.refresh([&bonsai, &fast]).await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        liveness.forget(&bonsai);
        liveness.refresh([&bonsai]).await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_loading_server_is_down_and_a_server_without_the_route_is_up() {
        let liveness = Liveness::new(Duration::from_secs(60), Duration::from_millis(300));
        let (loading, _, _) = server("503 Service Unavailable").await;
        let (no_route, _, _) = server("404 Not Found").await;
        let loading = llama("local.loading", &loading);
        let no_route = llama("local.no-route", &no_route);
        liveness.refresh([&loading, &no_route]).await;
        assert!(liveness.is_down(&loading));
        assert!(!liveness.is_down(&no_route));
    }

    #[tokio::test]
    async fn an_answer_older_than_its_ttl_no_longer_counts() {
        let liveness = Liveness::new(Duration::from_millis(50), Duration::from_millis(300));
        let bonsai = llama("local.bonsai-2-9b", &closed_port().await);
        liveness.refresh([&bonsai]).await;
        assert!(liveness.is_down(&bonsai));
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(!liveness.is_down(&bonsai));
    }

    #[test]
    fn only_usable_local_llama_cpp_models_are_asked() {
        let ready = llama("local.bonsai-2-9b", "http://127.0.0.1:8081/v1/");
        assert_eq!(
            Liveness::health_url(&ready).unwrap().as_str(),
            "http://127.0.0.1:8081/health"
        );
        let mut offline = ready.clone();
        offline.availability = Availability::Offline;
        assert!(Liveness::health_url(&offline).is_none());
        let mut other = ready.clone();
        other.provider = ProviderKind::OpenAiCompatible;
        assert!(Liveness::health_url(&other).is_none());
        let mut cloud = ready;
        cloud.locality = Locality::Cloud;
        assert!(Liveness::health_url(&cloud).is_none());
    }
}
