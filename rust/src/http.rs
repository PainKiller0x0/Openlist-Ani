use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, Path, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::{
    auth::AuthSettings,
    config_store::ConfigDocumentStore,
    storage::{LegacyDbPaths, LegacyResourceStore, LegacyTaskMementoStore, check_legacy_databases},
};

const SESSION_COOKIE: &str = "op_ani_session";
const SESSION_TTL_SECONDS: usize = 30 * 24 * 60 * 60;
const MAX_LOGIN_BODY_BYTES: usize = 8 * 1024;

#[derive(Clone)]
pub struct AppState {
    databases: Arc<LegacyDbPaths>,
    auth: Option<Arc<AuthSettings>>,
    config: Option<Arc<ConfigDocumentStore>>,
}

impl AppState {
    pub fn new(data_db: impl Into<PathBuf>, task_mementos_db: impl Into<PathBuf>) -> Self {
        Self {
            databases: Arc::new(LegacyDbPaths {
                data_db: data_db.into(),
                task_mementos_db: task_mementos_db.into(),
            }),
            auth: None,
            config: None,
        }
    }

    pub fn with_auth(mut self, auth: AuthSettings) -> Self {
        self.auth = Some(Arc::new(auth));
        self
    }

    pub fn with_config_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.config = Some(Arc::new(ConfigDocumentStore::new(path)));
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
        .route("/api/ui/settings", get(ui_settings))
        .route("/api/ui/state", get(ui_state))
        .route("/api/downloads", get(list_downloads))
        .route("/api/downloads/{task_id}", get(get_download))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, auth_gate))
}

async fn auth_gate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let public = matches!(
        path,
        "/healthz" | "/readyz" | "/api/auth/login" | "/api/auth/logout" | "/api/auth/session"
    );
    if public || state.auth.is_none() {
        return next.run(request).await;
    }

    let authenticated = state.auth.as_ref().is_some_and(|auth| {
        cookie_value(request.headers(), SESSION_COOKIE)
            .is_some_and(|token| auth.session_username(&token).is_some())
    });
    if authenticated {
        return next.run(request).await;
    }
    if path.starts_with("/api/") {
        api_error(StatusCode::UNAUTHORIZED, "请先登录 op-ani")
    } else {
        (StatusCode::SEE_OTHER, [(header::LOCATION, "/login")]).into_response()
    }
}

async fn ui_settings(State(state): State<AppState>) -> Response {
    let Some(config) = state.config else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration is unavailable",
        );
    };
    match tokio::task::spawn_blocking(move || config.ui_settings()).await {
        Ok(Ok(settings)) => Json(settings).into_response(),
        _ => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not read UI settings",
        ),
    }
}

async fn ui_state(State(state): State<AppState>) -> Response {
    let Some(config) = state.config else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration is unavailable",
        );
    };
    let databases = Arc::clone(&state.databases);
    match tokio::task::spawn_blocking(move || load_ui_state(&config, &databases)).await {
        Ok(Ok(state)) => Json(state).into_response(),
        _ => api_error(StatusCode::INTERNAL_SERVER_ERROR, "could not read UI state"),
    }
}

fn load_ui_state(
    config: &ConfigDocumentStore,
    databases: &LegacyDbPaths,
) -> Result<serde_json::Value, ()> {
    let settings = config.ui_settings().map_err(|_| ())?;
    let mut subscriptions = config.ui_subscriptions().map_err(|_| ())?;
    let anime_names = subscriptions
        .iter()
        .filter_map(subscription_identity)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let seasons = subscriptions
        .iter()
        .filter_map(|item| {
            let name = subscription_identity(item)?;
            let season = item.get("season").and_then(serde_json::Value::as_i64)?;
            Some((name.to_string(), season))
        })
        .collect::<HashMap<_, _>>();
    let latest = LegacyResourceStore::new(&databases.data_db)
        .find_latest_episodes(&anime_names, &seasons)
        .map_err(|_| ())?;
    for item in &mut subscriptions {
        if let Some(episode) = subscription_identity(item).and_then(|name| latest.get(name)) {
            item["latest_episode"] = json!(episode);
        }
    }
    let rss_urls = subscriptions
        .iter()
        .filter(|item| {
            item.get("enabled").and_then(serde_json::Value::as_bool) != Some(false)
                && item.get("completed").and_then(serde_json::Value::as_bool) != Some(true)
        })
        .filter_map(|item| item.get("url").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    let records = LegacyTaskMementoStore::new(&databases.task_mementos_db)
        .load_all()
        .map_err(|_| ())?;
    let tasks = records
        .iter()
        .map(|record| task_response(&record.task_id, &record.state, &record.payload_json))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    Ok(json!({
        "rss_urls": rss_urls,
        "rss_subscriptions": subscriptions,
        "global_exclude_patterns": settings.global_exclude_patterns,
        "openlist_url": settings.openlist_url,
        "download_path": settings.download_path,
        "poll_interval_seconds": settings.poll_interval_seconds,
        "max_download_retries": settings.max_download_retries,
        "mikan_base_url": settings.mikan_base_url,
        "rss_status": {
            "enabled": false,
            "running": false,
            "status": "disabled",
            "message": "Rust RSS scanner is not running"
        },
        "tasks": tasks
    }))
}

fn subscription_identity(item: &serde_json::Value) -> Option<&str> {
    item.get("anime_name")
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.is_empty())
        .or_else(|| {
            item.get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.is_empty())
        })
}

async fn list_downloads(State(state): State<AppState>) -> Response {
    let database = Arc::clone(&state.databases);
    match tokio::task::spawn_blocking(move || {
        LegacyTaskMementoStore::new(&database.task_mementos_db).load_all()
    })
    .await
    {
        Ok(Ok(records)) => {
            let tasks = match records
                .iter()
                .map(|record| task_response(&record.task_id, &record.state, &record.payload_json))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(tasks) => tasks.into_iter().flatten().collect::<Vec<_>>(),
                Err(()) => {
                    return api_error(StatusCode::INTERNAL_SERVER_ERROR, "stored task is invalid");
                }
            };
            Json(json!({"tasks": tasks, "total": tasks.len()})).into_response()
        }
        _ => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not read download tasks",
        ),
    }
}

async fn get_download(State(state): State<AppState>, Path(task_id): Path<String>) -> Response {
    let database = Arc::clone(&state.databases);
    match tokio::task::spawn_blocking(move || {
        LegacyTaskMementoStore::new(&database.task_mementos_db).load_all()
    })
    .await
    {
        Ok(Ok(records)) => {
            let Some(record) = records.iter().find(|record| record.task_id == task_id) else {
                return api_error(StatusCode::NOT_FOUND, &format!("Task not found: {task_id}"));
            };
            match task_response(&record.task_id, &record.state, &record.payload_json) {
                Ok(Some(task)) => Json(task).into_response(),
                Ok(None) => api_error(StatusCode::NOT_FOUND, &format!("Task not found: {task_id}")),
                Err(()) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "stored task is invalid"),
            }
        }
        _ => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not read download tasks",
        ),
    }
}

fn task_response(
    task_id: &str,
    state: &str,
    payload_json: &str,
) -> Result<Option<serde_json::Value>, ()> {
    let payload: serde_json::Value = serde_json::from_str(payload_json).map_err(|_| ())?;
    if payload.get("archived").and_then(serde_json::Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let release = payload
        .get("release")
        .and_then(serde_json::Value::as_object)
        .ok_or(())?;
    let required_text = |key: &str| {
        release
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or(())
    };
    let optional = |value: Option<&serde_json::Value>| {
        value
            .filter(|item| !item.is_null())
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    };
    let retry = payload.get("retry").and_then(serde_json::Value::as_object);
    let downloader_payload = payload
        .get("downloader")
        .and_then(|downloader| downloader.get("payload"));
    let created_at = payload
        .get("created_at")
        .and_then(serde_json::Value::as_str)
        .ok_or(())?;
    let updated_at = payload
        .get("updated_at")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(created_at);

    Ok(Some(json!({
        "id": task_id,
        "title": required_text("title")?,
        "download_url": required_text("download_url")?,
        "state": state,
        "anime_name": optional(release.get("anime_name")),
        "season": optional(release.get("season")),
        "episode": optional(release.get("episode")),
        "fansub": optional(release.get("fansub")),
        "quality": optional(release.get("quality")),
        "progress": optional(downloader_payload.and_then(|value| value.get("progress"))),
        "error_message": optional(retry.and_then(|value| value.get("last_error"))),
        "retry_count": retry.and_then(|value| value.get("retry_count")).and_then(serde_json::Value::as_i64).unwrap_or(0),
        "created_at": created_at,
        "updated_at": updated_at,
        "started_at": optional(payload.get("started_at")),
        "completed_at": optional(payload.get("completed_at")),
        "save_path": payload.get("base_path").and_then(serde_json::Value::as_str).unwrap_or(""),
        "final_path": optional(payload.get("output_path")),
    })))
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
