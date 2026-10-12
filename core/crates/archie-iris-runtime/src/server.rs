//! The loopback HTTP API of `iris_serve`, served from Rust.
//!
//! `POST /detect?mode=all|phrases&phrases=a,b&threshold=0.35&embed=1` with a JPEG or PNG body,
//! `POST /act?session=&instruction=&skill=`, `POST /act/reset?session=`, `POST
//! /act/recorded`, `GET /health`. Frames are decoded in memory and dropped; nothing is
//! written and frames stay out of the logs. It binds to 127.0.0.1 only.

use crate::act::Actor;
use crate::detect::Detector;
use crate::preprocess::decode;
use crate::Error;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

pub struct Service {
    pub detector: Detector,
    pub actor: Option<Actor>,
    pub info: Value,
    latencies: Vec<f64>,
    started: Instant,
}

impl Service {
    pub fn new(detector: Detector, actor: Option<Actor>, info: Value) -> Self {
        Self {
            detector,
            actor,
            info,
            latencies: Vec::new(),
            started: Instant::now(),
        }
    }

    fn record(&mut self, latency: f64) {
        self.latencies.push(latency);
        if self.latencies.len() > 200 {
            self.latencies.remove(0);
        }
    }

    pub fn health(&self) -> Value {
        let mut ordered = self.latencies.clone();
        ordered.sort_by(f64::total_cmp);
        let mut info = self.info.clone();
        info["requests"] = json!(ordered.len());
        info["p50_ms"] = ordered
            .get(ordered.len() / 2)
            .map_or(Value::Null, |v| json!((v * 10.0).round() / 10.0));
        info["text"] = self.detector.phrases.describe();
        info["runtime"] = json!("archie-iris-runtime");
        info
    }

    pub fn detect(&mut self, body: &[u8], query: &HashMap<String, String>) -> Result<Value, Error> {
        let image = decode(body)?;
        let phrases: Vec<String> = query
            .get("phrases")
            .map(|p| {
                p.split(',')
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let threshold = query
            .get("threshold")
            .map_or(Ok(0.35), |t| t.parse::<f32>())
            .map_err(|_| Error::Refused("threshold is not a number".into()))?
            .clamp(0.01, 0.99);
        let everything = query.get("mode").is_some_and(|m| m == "all");
        let embed = query.get("embed").is_some_and(|e| e == "1");
        let started = Instant::now();
        let hits = if everything || phrases.is_empty() {
            self.detector
                .detect_everything(&image, threshold, &phrases, 60, 0.5, embed)?
        } else {
            self.detector
                .detect(&image, &phrases, threshold, 20, 0.5, embed)?
        };
        let latency = started.elapsed().as_secs_f64() * 1000.0;
        self.record(latency);
        Ok(
            json!({"detections": hits, "latency_ms": (latency * 10.0).round() / 10.0,
                  "image_size": [image.width, image.height], "step": self.info["step"]}),
        )
    }

    pub fn act(&mut self, body: &[u8], query: &HashMap<String, String>) -> Result<Value, Error> {
        let field = |name: &str, limit: usize| {
            query
                .get(name)
                .map_or(String::new(), |v| v.chars().take(limit).collect())
        };
        if !field("adapter", 64).is_empty() {
            return Err(Error::Refused(
                "skill adapters are not loaded by the Rust runtime yet".into(),
            ));
        }
        if !field("record", 128).is_empty() {
            return Err(Error::Refused(
                "recording for REM is not available in the Rust runtime yet".into(),
            ));
        }
        let image = decode(body)?;
        let session = query
            .get("session")
            .map_or("default".to_owned(), |v| v.chars().take(64).collect());
        let now = self.started.elapsed().as_secs_f64();
        let actor = self
            .actor
            .as_mut()
            .ok_or_else(|| Error::Refused("these weights have no action head".into()))?;
        let started = Instant::now();
        let mut control = actor.act(
            &self.detector.model,
            self.detector.phrases.as_mut(),
            &session,
            &image,
            &field("instruction", 300),
            &field("skill", 300),
            now,
        )?;
        let latency = started.elapsed().as_secs_f64() * 1000.0;
        self.record(latency);
        control["latency_ms"] = json!((latency * 10.0).round() / 10.0);
        control["step"] = self.info["step"].clone();
        Ok(control)
    }
}

type Shared = Arc<Mutex<Service>>;

fn reply(result: Result<Value, Error>) -> Response {
    match result {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": error.to_string()})),
        )
            .into_response(),
    }
}

async fn blocking(
    state: Shared,
    work: impl FnOnce(&mut Service) -> Result<Value, Error> + Send + 'static,
) -> Response {
    let joined = tokio::task::spawn_blocking(move || {
        let mut service = state
            .lock()
            .map_err(|_| Error::Refused("the model is unavailable".into()))?;
        work(&mut service)
    })
    .await;
    match joined {
        Ok(result) => reply(result),
        Err(_) => reply(Err(Error::Refused("inference stopped".into()))),
    }
}

fn frame_size_ok(body: &Bytes) -> Option<Response> {
    if body.is_empty() || body.len() > MAX_FRAME_BYTES {
        return Some(
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({"error": "frame_size_invalid"})),
            )
                .into_response(),
        );
    }
    None
}

pub fn router(service: Service) -> Router {
    let state: Shared = Arc::new(Mutex::new(service));
    Router::new()
        .route("/", get(|| async { "IRIS (archie-iris-runtime). POST frames to /detect or /act.\n" }))
        .route(
            "/health",
            get(|State(state): State<Shared>| async move {
                let health = state.lock().map(|s| s.health()).unwrap_or_else(|_| json!({"error": "unavailable"}));
                Json(health)
            }),
        )
        .route(
            "/detect",
            post(|State(state): State<Shared>, Query(query): Query<HashMap<String, String>>, body: Bytes| async move {
                if let Some(refusal) = frame_size_ok(&body) {
                    return refusal;
                }
                blocking(state, move |s| s.detect(&body, &query)).await
            }),
        )
        .route(
            "/act",
            post(|State(state): State<Shared>, Query(query): Query<HashMap<String, String>>, body: Bytes| async move {
                if let Some(refusal) = frame_size_ok(&body) {
                    return refusal;
                }
                blocking(state, move |s| s.act(&body, &query)).await
            }),
        )
        .route(
            "/act/reset",
            post(|State(state): State<Shared>, Query(query): Query<HashMap<String, String>>| async move {
                let session = query.get("session").map_or("default".to_owned(), Clone::clone);
                if let Ok(mut service) = state.lock() {
                    if let Some(actor) = service.actor.as_mut() {
                        actor.reset(&session);
                    }
                }
                Json(json!({"reset": true}))
            }),
        )
        // Nothing is recorded by this runtime, so a run has no samples, as with an
        // iris_serve started without --rem-spool.
        .route("/act/recorded", post(|| async { Json(json!({"samples": [], "errors": 0})) }))
        .with_state(state)
}
