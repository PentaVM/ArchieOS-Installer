//! Kanban Task Board API
//!
//! Provides a REST interface to interact with the workstation's tasks.
//! Ensures privacy spine and zero-secret leaks invariants.

use crate::enterprise_bridge::EnterpriseAdapter;
use archie_types::ids::{TaskId, TraceId};
use archie_types::task::{KanbanCard, QueueClass, Task, TaskGraph, TaskStatus};
use archie_vault::store::Vault;
use axum::{
    extract::{Path, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, post, put},
    Json, Router,
};
use ring::constant_time::verify_slices;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

#[derive(Clone)]
pub struct BoardState {
    pub adapter: Arc<EnterpriseAdapter>,
    pub task_graph: Arc<RwLock<TaskGraph>>,
    pub vault: Arc<Mutex<Vault>>,
    pub auth_token_hash: Vec<u8>,
}

#[derive(Deserialize)]
pub struct MoveTaskRequest {
    pub new_status: TaskStatus,
}

#[derive(Deserialize)]
pub struct CreateTaskRequest {
    pub title: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Centralized Error type to prevent leaking internal traces.
pub enum AppError {
    Internal(String),
    BadRequest(String),
    Unauthorized,
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match self {
            AppError::Internal(_err) => {
                // In a real app: tracing::error!("Internal Board Error: {}", _err);
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
            }
            AppError::BadRequest(err) => (StatusCode::BAD_REQUEST, err.as_str()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized"),
        };
        (
            status,
            Json(ErrorResponse {
                error: msg.into(),
            }),
        )
            .into_response()
    }
}

pub fn router(state: BoardState) -> Router {
    Router::new()
        .route("/tasks", get(list_tasks).post(create_task))
        .route("/tasks/:id/move", put(move_task))
        .route("/sync", post(sync_board))
        .layer(middleware::from_fn_with_state(state.clone(), auth_guard))
        .with_state(state)
}

/// Strict Boundary Auth: verifies the Bearer token in constant time.
async fn auth_guard(
    State(state): State<BoardState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<axum::response::Response, AppError> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    let token = auth_header.ok_or(AppError::Unauthorized)?;

    // Compute SHA-256 of the incoming token to compare with the stored hash
    let hash = ring::digest::digest(&ring::digest::SHA256, token.as_bytes());

    // Constant-time verification to prevent timing side-channels
    if verify_slices(hash.as_ref(), &state.auth_token_hash).is_err() {
        return Err(AppError::Unauthorized);
    }

    Ok(next.run(req).await)
}

async fn list_tasks(State(state): State<BoardState>) -> Result<impl IntoResponse, AppError> {
    let graph = state.task_graph.read().await;
    let cards: Vec<KanbanCard> = graph.iter().map(KanbanCard::from).collect();
    Ok((StatusCode::OK, Json(cards)))
}

async fn create_task(
    State(state): State<BoardState>,
    Json(payload): Json<CreateTaskRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut graph = state.task_graph.write().await;

    let mut task = Task::new(payload.title, TraceId::new(), QueueClass::Background);

    // Privacy Spine Ingress: Force all tasks created via the bridge to be explicitly restricted.
    task.labels
        .insert("privacy_grade".into(), "restricted".into());

    graph
        .insert(task)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok((StatusCode::CREATED, Json("Task created securely")))
}

async fn move_task(
    State(state): State<BoardState>,
    Path(id): Path<TaskId>,
    Json(payload): Json<MoveTaskRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut graph = state.task_graph.write().await;

    let task = graph
        .get_mut(&id)
        .ok_or_else(|| AppError::BadRequest("Task not found".into()))?;

    // DAG Integrity: Respect invariant state machine
    task.transition_to(payload.new_status)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    Ok((StatusCode::OK, Json("Task moved securely")))
}

async fn sync_board(State(state): State<BoardState>) -> Result<impl IntoResponse, AppError> {
    let graph = state.task_graph.read().await;
    let cards_to_sync: Vec<KanbanCard> = graph.iter().map(KanbanCard::from).collect();

    let mut vault = state.vault.lock().await;

    match state
        .adapter
        .sync_to_remote(&cards_to_sync, &mut vault)
        .await
    {
        Ok(_) => Ok((StatusCode::OK, Json("Sync completed"))),
        Err(e) => Err(AppError::Internal(e.to_string())),
    }
}
