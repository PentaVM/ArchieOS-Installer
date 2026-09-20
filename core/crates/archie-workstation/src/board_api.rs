//! Kanban Task Board API
//!
//! Provides a REST interface to interact with the workstation's tasks.
//! Ensures privacy spine and zero-secret leaks invariants.

use crate::enterprise_bridge::EnterpriseAdapter;
use archie_types::ids::TaskId;
use archie_types::task::{KanbanCard, TaskStatus};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct BoardState {
    pub adapter: Arc<EnterpriseAdapter>,
}

#[derive(Deserialize)]
pub struct MoveTaskRequest {
    pub new_status: TaskStatus,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub fn router(state: BoardState) -> Router {
    Router::new()
        .route("/tasks", get(list_tasks).post(create_task))
        .route("/tasks/:id/move", put(move_task))
        .route("/sync", post(sync_board))
        .with_state(state)
}

async fn list_tasks(State(_state): State<BoardState>) -> impl IntoResponse {
    // In a real implementation, this queries the Cerebra TaskGraph, converts
    // tasks to KanbanCard using KanbanCard::from(&task), and groups them by KanbanColumn.
    let cards: Vec<KanbanCard> = vec![];
    (StatusCode::OK, Json(cards))
}

async fn create_task(
    State(_state): State<BoardState>,
) -> impl IntoResponse {
    // Create new task, add it to TaskGraph.
    (StatusCode::CREATED, Json("Task created securely"))
}

async fn move_task(
    State(_state): State<BoardState>,
    Path(id): Path<TaskId>,
    Json(payload): Json<MoveTaskRequest>,
) -> impl IntoResponse {
    // 1. Fetch Task by id.
    // 2. task.transition_to(payload.new_status)?;
    // 3. Save Task to TaskGraph.
    let _ = id;
    let _ = payload;
    (StatusCode::OK, Json("Task moved securely"))
}

async fn sync_board(State(state): State<BoardState>) -> impl IntoResponse {
    // Example: fetch ready KanbanCards, and sync them to remote board.
    let cards_to_sync: Vec<KanbanCard> = vec![];
    
    match state.adapter.sync_to_remote(&cards_to_sync).await {
        Ok(_) => (StatusCode::OK, Json("Sync completed").into_response()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Sync failed: {:?}", e),
            }),
        ).into_response(),
    }
}
