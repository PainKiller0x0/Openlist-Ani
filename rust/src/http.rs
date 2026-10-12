use std::{path::PathBuf, sync::Arc};

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use serde::Serialize;

use crate::storage::{LegacyDbPaths, check_legacy_databases};

#[derive(Clone)]
pub struct AppState {
    databases: Arc<LegacyDbPaths>,
}

impl AppState {
    pub fn new(data_db: impl Into<PathBuf>, task_mementos_db: impl Into<PathBuf>) -> Self {
        Self {
            databases: Arc::new(LegacyDbPaths {
                data_db: data_db.into(),
                task_mementos_db: task_mementos_db.into(),
            }),
        }
    }
}

#[derive(Serialize)]
struct StatusResponse {
    status: &'static str,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state)
}

async fn healthz() -> Json<StatusResponse> {
    Json(StatusResponse { status: "ok" })
}

async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    let databases = Arc::clone(&state.databases);
    let ready = tokio::task::spawn_blocking(move || check_legacy_databases(&databases))
        .await
        .is_ok_and(|result| result.is_ok());

    if ready {
        (StatusCode::OK, Json(StatusResponse { status: "ready" }))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(StatusResponse {
                status: "not_ready",
            }),
        )
    }
}
