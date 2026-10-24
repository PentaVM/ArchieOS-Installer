use crate::{
    store::WandaStore,
    types::{QueryResponse, WandaEvent, WandaView},
    WandaError, MAX_REQUEST_BYTES,
};
use archie_vault::ContextDomain;
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Query, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{RwLock, Semaphore};

/// The first retry of evidence the model could not turn into advice, and the longest wait.
const RETRY_BASE_MS: i64 = 60_000;
const RETRY_CAP_MS: i64 = 3_600_000;
/// How long a query or an edit waits for a busy service before it answers `wanda_busy`.
const QUEUE_WAIT: std::time::Duration = std::time::Duration::from_secs(25);
const REFRESH_QUESTION: &str = "Revisa las evidencias recientes. Propón detalles, respuestas y compromisos útiles; distingue lo observado de las hipótesis y ofrece alternativas. No inventes ofertas o intenciones.";

/// A generation the model failed on, and when it may be tried again.
struct Attempt {
    generation: String,
    failures: u32,
    retry_at_ms: i64,
}

pub struct Service {
    store: WandaStore,
    /// The last advice that was produced, per domain. A later failure never replaces it;
    /// the view drops each recommendation on its own once it or its evidence expires.
    last: RwLock<BTreeMap<ContextDomain, QueryResponse>>,
    processed: RwLock<BTreeMap<ContextDomain, String>>,
    failed: RwLock<BTreeMap<ContextDomain, Attempt>>,
    inference: Semaphore,
}

/// What a domain's evidence is, for deciding whether it was already reasoned over. Full
/// bodies never enter the ledger. Reposts do not trigger another inference; new event IDs
/// or authoritative graph writes do. Reads the filesystem: call from blocking work.
fn generation(
    store: &WandaStore,
    domain: &ContextDomain,
    events: &[crate::types::StoredEvent],
) -> String {
    let mut refs = events
        .iter()
        .map(|e| e.evidence_ref.to_string())
        .collect::<Vec<_>>();
    refs.sort();
    let modified = store
        .config()
        .domains
        .iter()
        .find(|b| &b.domain == domain)
        .and_then(|b| std::fs::metadata(&b.graph_path).ok())
        .and_then(|m| m.modified().ok());
    format!("{refs:?}:{modified:?}")
}

impl Service {
    pub fn new(store: WandaStore) -> Arc<Self> {
        let parallel = usize::from(store.config().parallel_queries.clamp(1, 8));
        Arc::new(Self {
            store,
            last: RwLock::new(BTreeMap::new()),
            processed: RwLock::new(BTreeMap::new()),
            failed: RwLock::new(BTreeMap::new()),
            inference: Semaphore::new(parallel),
        })
    }
    /// A permit for a request that a person is waiting on: it waits its turn behind the one
    /// running (a tick's read is bounded) instead of being refused at once, and gives up
    /// before the voice client's 30 s do.
    async fn wait_for_inference(&self) -> Result<tokio::sync::SemaphorePermit<'_>, WandaError> {
        tokio::time::timeout(QUEUE_WAIT, self.inference.acquire())
            .await
            .map_err(|_| WandaError::Busy)?
            .map_err(|_| WandaError::Busy)
    }
    /// The domain's newest events and their generation.
    async fn current(
        &self,
        domain: &ContextDomain,
        now: i64,
    ) -> Result<(usize, String), WandaError> {
        let domain = domain.clone();
        self.store
            .blocking(move |store| {
                let events = store.events(&domain, 64, now)?;
                Ok((events.len(), generation(store, &domain, &events)))
            })
            .await
    }
    /// Records how a query over `generation` went. Advice that came back replaces the
    /// last advice; anything else leaves it and backs the generation off, doubling from
    /// a minute to an hour, so a model that is down is not asked again every tick.
    async fn settle(
        &self,
        domain: &ContextDomain,
        generation: String,
        result: Option<&QueryResponse>,
        now: i64,
    ) {
        match result {
            Some(result) if result.state == "ready" => {
                self.failed.write().await.remove(domain);
                self.processed
                    .write()
                    .await
                    .insert(domain.clone(), generation);
                self.last
                    .write()
                    .await
                    .insert(domain.clone(), result.clone());
            }
            _ => {
                let mut failed = self.failed.write().await;
                let failures = failed
                    .get(domain)
                    .filter(|a| a.generation == generation)
                    .map_or(1, |a| a.failures.saturating_add(1));
                let wait = RETRY_BASE_MS
                    .saturating_mul(1i64 << failures.saturating_sub(1).min(10))
                    .min(RETRY_CAP_MS);
                failed.insert(
                    domain.clone(),
                    Attempt {
                        generation,
                        failures,
                        retry_at_ms: now.saturating_add(wait),
                    },
                );
            }
        }
    }
    /// Coalesced caller-owned timer, not a separate cognitive engine. An unavailable
    /// model keeps evidence pending and is retried with backoff. One inference may run;
    /// there is no request queue.
    pub async fn refresh(&self) {
        for domain in self.store.domains() {
            let Ok(_permit) = self.inference.try_acquire() else {
                return;
            };
            let now = now_ms();
            let Ok((count, generation)) = self.current(&domain, now).await else {
                continue;
            };
            if count == 0 || self.processed.read().await.get(&domain) == Some(&generation) {
                continue;
            }
            if self
                .failed
                .read()
                .await
                .get(&domain)
                .is_some_and(|a| a.generation == generation && now < a.retry_at_ms)
            {
                continue;
            }
            let result =
                crate::runtime::query(&self.store, &domain, REFRESH_QUESTION, false, now).await;
            self.settle(&domain, generation, result.as_ref().ok(), now)
                .await;
        }
        // The local expert reads what arrived since the last tick into each domain's state.
        if self.store.lncm().is_some() {
            for domain in self.store.domains() {
                let Ok(_permit) = self.inference.try_acquire() else {
                    return;
                };
                let now = now_ms();
                let _ = self
                    .store
                    .blocking(move |store| {
                        crate::social::read_new(store, &domain, now, crate::social::TICK_READ_TURNS)
                    })
                    .await;
            }
        }
    }
}
pub fn router(store: WandaStore) -> Router {
    router_with_service(Service::new(store))
}
pub fn router_with_service(service: Arc<Service>) -> Router {
    Router::new()
        .route("/v1/events", post(ingest))
        .route("/v1/query", post(query))
        .route("/v1/view", get(view))
        .route("/v1/domains", get(domains))
        .route("/v1/people", get(people))
        .route("/v1/people/edit", post(edit_person))
        .route("/v1/scene/text", post(scene_text))
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/styles.css", get(css))
        .route("/app.mjs", get(app))
        .route("/model.mjs", get(model))
        .route("/demo.mjs", get(demo))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn(boundary))
        .with_state(service)
}
async fn boundary(request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let valid = reqwest::Url::parse(&format!("http://{host}"))
        .ok()
        .is_some_and(|url| {
            url.username().is_empty()
                && url.password().is_none()
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none()
                && url
                    .host_str()
                    .is_some_and(|h| matches!(h, "127.0.0.1" | "localhost" | "::1" | "[::1]"))
        });
    if !valid {
        return StatusCode::FORBIDDEN.into_response();
    }
    if request.uri().path() != "/v1/events" {
        if let Some(origin) = request.headers().get(header::ORIGIN) {
            if !origin
                .to_str()
                .ok()
                .is_some_and(|o| o == format!("http://{host}") || o == format!("https://{host}"))
            {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("referrer-policy", "no-referrer".parse().unwrap());
    headers.insert("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'".parse().unwrap());
    response
}
fn bearer(headers: &HeaderMap) -> Result<&str, WandaError> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|s| !s.is_empty() && s.len() <= 4096)
        .ok_or(WandaError::Unauthorized)
}
fn owner(service: &Service, headers: &HeaderMap) -> Result<(), WandaError> {
    if service.store.owner_authenticated(bearer(headers)?) {
        Ok(())
    } else {
        Err(WandaError::Unauthorized)
    }
}
fn json<T: serde::de::DeserializeOwned>(
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<T, WandaError> {
    if !headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim() == "application/json")
        })
    {
        return Err(WandaError::InvalidEvent("json_required"));
    }
    serde_json::from_slice(body).map_err(|_| WandaError::InvalidEvent("invalid_json"))
}
async fn ingest(
    State(service): State<Arc<Service>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<crate::types::IngestReport>, WandaError> {
    let token = bearer(&headers)?.to_owned();
    let events: Vec<WandaEvent> = json(&headers, &body)?;
    let result =
        tokio::task::spawn_blocking(move || service.store.ingest(&token, &events, now_ms()))
            .await
            .map_err(|_| WandaError::Storage)??;
    Ok(Json(result))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryRequest {
    domain: ContextDomain,
    question: String,
    #[serde(default)]
    deep: bool,
}
async fn query(
    State(service): State<Arc<Service>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<QueryResponse>, WandaError> {
    owner(&service, &headers)?;
    let request: QueryRequest = json(&headers, &body)?;
    let _permit = service.wait_for_inference().await?;
    let now = now_ms();
    let (_, generation) = service.current(&request.domain, now).await?;
    let result = crate::runtime::query(
        &service.store,
        &request.domain,
        &request.question,
        request.deep,
        now,
    )
    .await;
    service
        .settle(&request.domain, generation, result.as_ref().ok(), now)
        .await;
    let result = result?;
    Ok(Json(result))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewRequest {
    domain: ContextDomain,
}
async fn view(
    State(service): State<Arc<Service>>,
    headers: HeaderMap,
    Query(request): Query<ViewRequest>,
) -> Result<Json<WandaView>, WandaError> {
    owner(&service, &headers)?;
    service.store.require_domain(&request.domain)?;
    let now = now_ms();
    let graph = crate::graph::project(service.store.config(), &request.domain)
        .await
        .ok();
    let domain = request.domain.clone();
    let (sources, count, generation, entries) = service
        .store
        .blocking(move |store| {
            let sources = store.sources(&domain, now)?;
            let events = store.events(&domain, 64, now)?;
            let generation = generation(store, &domain, &events);
            let vault = archie_cerebra::vault_io::open_vault(&store.vault_path())
                .map_err(|_| WandaError::Storage)?;
            Ok((sources, events.len(), generation, vault.sealed_entries()))
        })
        .await?;
    let pending_events = if service.processed.read().await.get(&request.domain) == Some(&generation)
    {
        0
    } else {
        count
    };
    let mut recommendations = service
        .last
        .read()
        .await
        .get(&request.domain)
        .map(|r| r.recommendations.clone())
        .unwrap_or_default();
    // A cited source must still exist in this scope and remain within retention.
    recommendations.retain(|r| {
        r.expires_at_ms > now
            && r.evidence_refs.iter().all(|s| {
                entries.iter().any(|e| {
                    e.reference.to_string() == *s
                        && request.domain.contains(&e.reference)
                        && e.expires_at_ms.is_none_or(|at| at > now)
                })
            })
    });
    Ok(Json(WandaView {
        schema_version: 1,
        domain: request.domain,
        generated_at_ms: now,
        availability: if graph.is_some() {
            "ready"
        } else {
            "graph_unavailable"
        }
        .into(),
        graph,
        recommendations,
        sources,
        pending_events,
    }))
}
async fn people(
    State(service): State<Arc<Service>>,
    headers: HeaderMap,
    Query(request): Query<ViewRequest>,
) -> Result<Json<serde_json::Value>, WandaError> {
    owner(&service, &headers)?;
    let domain = request.domain;
    let now = now_ms();
    let people = service
        .store
        .blocking(move |store| crate::social::people(store, &domain, now))
        .await?;
    Ok(Json(serde_json::json!({"people": people})))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EditRequest {
    domain: ContextDomain,
    person: String,
    #[serde(default)]
    key: Option<String>,
    action: crate::social::EditAction,
}
async fn edit_person(
    State(service): State<Arc<Service>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, WandaError> {
    owner(&service, &headers)?;
    let request: EditRequest = json(&headers, &body)?;
    // The reader holds this permit while it updates a state; an edit waits its turn.
    let _permit = service.wait_for_inference().await?;
    let now = now_ms();
    let changed = service
        .store
        .blocking(move |store| {
            crate::social::edit(
                store,
                &request.domain,
                &request.person,
                request.key.as_deref(),
                request.action,
                now,
            )
        })
        .await?;
    Ok(Json(serde_json::json!({"changed": changed})))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneRequest {
    domain: ContextDomain,
    delta: archie_slink::links::SceneTextDelta,
}
/// `scene.text` (lines IRIS read on a screen or a camera) into the domain's social state.
async fn scene_text(
    State(service): State<Arc<Service>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, WandaError> {
    use archie_slink::Payload;
    owner(&service, &headers)?;
    let request: SceneRequest = json(&headers, &body)?;
    request
        .delta
        .validate()
        .map_err(|_| WandaError::InvalidEvent("invalid_scene_text"))?;
    let _permit = service.wait_for_inference().await?;
    let now = now_ms();
    let report = service
        .store
        .blocking(move |store| {
            crate::social::read_scene(
                store,
                &request.domain,
                &request.delta.focus,
                &request.delta.lines,
                now,
            )
        })
        .await?;
    Ok(Json(
        serde_json::json!({"read": report.map(|r| serde_json::json!({
            "turns": r.turns, "applied": r.applied, "unchanged": r.unchanged, "rejected": r.rejected,
        }))}),
    ))
}
async fn domains(
    State(service): State<Arc<Service>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, WandaError> {
    owner(&service, &headers)?;
    Ok(Json(serde_json::json!({"domains":service.store.domains()})))
}
fn asset(mime: &'static str, text: &'static str) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, mime)], text)
}
async fn index() -> impl IntoResponse {
    asset(
        "text/html; charset=utf-8",
        include_str!("../../../../display/wanda/index.html"),
    )
}
async fn css() -> impl IntoResponse {
    asset(
        "text/css; charset=utf-8",
        include_str!("../../../../display/wanda/styles.css"),
    )
}
async fn app() -> impl IntoResponse {
    asset(
        "text/javascript; charset=utf-8",
        include_str!("../../../../display/wanda/app.mjs"),
    )
}
async fn model() -> impl IntoResponse {
    asset(
        "text/javascript; charset=utf-8",
        include_str!("../../../../display/wanda/model.mjs"),
    )
}
async fn demo() -> impl IntoResponse {
    asset(
        "text/javascript; charset=utf-8",
        include_str!("../../../../display/wanda/demo.mjs"),
    )
}
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
impl IntoResponse for WandaError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "authentication_required"),
            Self::UnknownDomain => (StatusCode::NOT_FOUND, "unknown_domain"),
            Self::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "input_budget"),
            Self::InvalidEvent(_) => (StatusCode::BAD_REQUEST, "invalid_input"),
            Self::Busy => (StatusCode::SERVICE_UNAVAILABLE, "wanda_busy"),
            Self::InvalidAdvice => (StatusCode::UNPROCESSABLE_ENTITY, "unverified_advice"),
            _ => (StatusCode::SERVICE_UNAVAILABLE, "wanda_unavailable"),
        };
        (status, Json(serde_json::json!({"error":code}))).into_response()
    }
}
