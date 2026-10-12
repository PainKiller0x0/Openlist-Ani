use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::{
    auth::AuthSettings,
    storage::{LegacyDbPaths, check_legacy_databases},
};

const SESSION_COOKIE: &str = "op_ani_session";
const SESSION_TTL_SECONDS: usize = 30 * 24 * 60 * 60;
const MAX_LOGIN_BODY_BYTES: usize = 8 * 1024;

#[derive(Clone)]
pub struct AppState {
    databases: Arc<LegacyDbPaths>,
    auth: Option<Arc<AuthSettings>>,
}

impl AppState {
    pub fn new(data_db: impl Into<PathBuf>, task_mementos_db: impl Into<PathBuf>) -> Self {
        Self {
            databases: Arc::new(LegacyDbPaths {
                data_db: data_db.into(),
                task_mementos_db: task_mementos_db.into(),
            }),
            auth: None,
        }
    }

    pub fn with_auth(mut self, auth: AuthSettings) -> Self {
        self.auth = Some(Arc::new(auth));
        self
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
        .route("/api/auth/session", get(auth_session))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
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

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct SessionResponse {
    authenticated: bool,
    username: Option<String>,
}

async fn auth_session(State(state): State<AppState>, headers: HeaderMap) -> Json<SessionResponse> {
    let username = state.auth.as_ref().and_then(|auth| {
        cookie_value(&headers, SESSION_COOKIE)
            .and_then(|token| auth.session_username(&token).map(str::to_string))
    });
    Json(SessionResponse {
        authenticated: username.is_some(),
        username,
    })
}

async fn auth_login(State(state): State<AppState>, request: Request<Body>) -> Response {
    let Some(auth) = state.auth else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "authentication is not configured",
        );
    };

    let secure_cookie = request.uri().scheme_str() == Some("https")
        || (request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .is_some_and(|peer| peer.0.ip().is_loopback())
            && request
                .headers()
                .get("x-forwarded-proto")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| {
                    value
                        .split(',')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .eq_ignore_ascii_case("https")
                }));

    let body = match to_bytes(request.into_body(), MAX_LOGIN_BODY_BYTES).await {
        Ok(body) => body,
        Err(_) => return api_error(StatusCode::PAYLOAD_TOO_LARGE, "request body is too large"),
    };
    let payload = match serde_json::from_slice::<LoginRequest>(&body) {
        Ok(payload) => payload,
        Err(_) => return api_error(StatusCode::UNPROCESSABLE_ENTITY, "invalid login request"),
    };
    let username = payload.username.trim();
    if username.is_empty()
        || username.chars().count() > 100
        || payload.password.is_empty()
        || payload.password.chars().count() > 500
    {
        return api_error(StatusCode::UNPROCESSABLE_ENTITY, "invalid login request");
    }

    let password = payload.password;
    let verifier = Arc::clone(&auth);
    let password_valid = tokio::task::spawn_blocking(move || verifier.verify_password(&password))
        .await
        .unwrap_or(false);
    if username != auth.username() || !password_valid {
        return api_error(StatusCode::UNAUTHORIZED, "用户名或密码错误");
    }

    let mut response = (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "authenticated": true,
            "username": auth.username(),
        })),
    )
        .into_response();
    let mut cookie = format!(
        "{SESSION_COOKIE}={}; Max-Age={SESSION_TTL_SECONDS}; Path=/; HttpOnly; SameSite=Lax",
        auth.issue_session(),
    );
    if secure_cookie {
        cookie.push_str("; Secure");
    }
    match HeaderValue::from_str(&cookie) {
        Ok(cookie) => {
            response.headers_mut().insert(header::SET_COOKIE, cookie);
            response
        }
        Err(_) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not issue session cookie",
        ),
    }
}

async fn auth_logout() -> Response {
    let mut response = (StatusCode::OK, Json(json!({"success": true}))).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("op_ani_session=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax"),
    );
    response
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|item| item.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then(|| value.to_string()))
}

fn api_error(status: StatusCode, detail: &str) -> Response {
    (status, Json(json!({"detail": detail}))).into_response()
}
