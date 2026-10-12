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

pub struct Service {
    store: WandaStore,
    last: RwLock<BTreeMap<ContextDomain, QueryResponse>>,
    processed: RwLock<BTreeMap<ContextDomain, String>>,
    inference: Semaphore,
}
impl Service {
    pub fn new(store: WandaStore) -> Arc<Self> {
        Arc::new(Self {
            store,
            last: RwLock::new(BTreeMap::new()),
            processed: RwLock::new(BTreeMap::new()),
            inference: Semaphore::new(1),
        })
    }
    fn generation(&self, domain: &ContextDomain, events: &[crate::types::StoredEvent]) -> String {
        // Full bodies never enter the generation ledger. Reposts do not trigger
        // another inference; new event IDs or authoritative graph writes do.
        let mut refs = events
            .iter()
            .map(|e| e.evidence_ref.to_string())
            .collect::<Vec<_>>();
        refs.sort();
        let modified = self
            .store
            .config()
            .domains
            .iter()
            .find(|b| &b.domain == domain)
            .and_then(|b| std::fs::metadata(&b.graph_path).ok())
            .and_then(|m| m.modified().ok());
        format!("{refs:?}:{modified:?}")
    }
    /// Coalesced caller-owned timer, not a separate cognitive engine. An unavailable
    /// model keeps evidence pending. One inference may run; there is no request queue.
    pub async fn refresh(&self) {
        for domain in self.store.domains() {
            let Ok(_permit) = self.inference.try_acquire() else {
                return;
            };
            let now = now_ms();
            let Ok(events) = self.store.events(&domain, 64, now) else {
                continue;
            };
            if events.is_empty() {
                continue;
            }
            let generation = self.generation(&domain, &events);
            if self.processed.read().await.get(&domain) == Some(&generation) {
                continue;
            }
            if let Ok(result)=crate::runtime::query(&self.store,&domain,"Revisa las evidencias recientes. Propón detalles, respuestas y compromisos útiles; distingue lo observado de las hipótesis y ofrece alternativas. No inventes ofertas o intenciones.",false,now).await {
                if result.state=="ready" {self.processed.write().await.insert(domain.clone(),generation);}
                self.last.write().await.insert(domain,result);
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
    let _permit = service
        .inference
        .try_acquire()
        .map_err(|_| WandaError::Busy)?;
    let events = service.store.events(&request.domain, 64, now_ms())?;
    let generation = service.generation(&request.domain, &events);
    let result = crate::runtime::query(
        &service.store,
        &request.domain,
        &request.question,
        request.deep,
        now_ms(),
    )
    .await?;
    if result.state == "ready" {
        service
            .processed
            .write()
            .await
            .insert(request.domain.clone(), generation);
    }
    service
        .last
        .write()
        .await
        .insert(request.domain, result.clone());
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
    let sources = service.store.sources(&request.domain, now)?;
    let events = service.store.events(&request.domain, 64, now)?;
    let generation = service.generation(&request.domain, &events);
    let pending_events = if service.processed.read().await.get(&request.domain) == Some(&generation)
    {
        0
    } else {
        events.len()
    };
    let mut recommendations = service
        .last
        .read()
        .await
        .get(&request.domain)
        .map(|r| r.recommendations.clone())
        .unwrap_or_default();
    // A cited source must still exist in this scope and remain within retention.
    let dir = service.store.vault_path();
    let vault = archie_cerebra::vault_io::open_vault(&dir).map_err(|_| WandaError::Storage)?;
    let entries = vault.sealed_entries();
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
