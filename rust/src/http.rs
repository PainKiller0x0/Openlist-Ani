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
    config_store::{ConfigDocumentStore, ConfigStoreError, UiSettingsUpdate},
    naming::validate_rename_format,
    openlist::OpenListClient,
    rss::{ParseRssResponse, RssError, parse_rss as fetch_parse_rss},
    storage::{LegacyDbPaths, LegacyResourceStore, LegacyTaskMementoStore, check_legacy_databases},
    torrent::{ResolveTorrentResponse, TorrentError, resolve_torrent as fetch_resolve_torrent},
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
        .route(
            "/api/ui/settings",
            get(ui_settings).post(ui_update_settings),
        )
        .route("/api/ui/state", get(ui_state))
        .route("/api/downloads", get(list_downloads))
        .route("/api/downloads/{task_id}", get(get_download))
        .route("/api/parse_rss", post(parse_rss_endpoint))
        .route("/api/resolve_torrent", post(resolve_torrent_endpoint))
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

#[derive(Default, Deserialize)]
#[serde(default)]
struct UiSettingsUpdateRequest {
    global_exclude_patterns: String,
    openlist_url: Option<String>,
    llm_provider_type: Option<String>,
    llm_api_key: Option<String>,
    llm_base_url: Option<String>,
    llm_model: Option<String>,
    tmdb_language: Option<String>,
    metadata_parser_provider: Option<String>,
    download_path: Option<String>,
    rename_format: Option<String>,
    poll_interval_seconds: Option<i64>,
    max_download_retries: Option<i64>,
    mikan_base_url: Option<String>,
}

async fn ui_update_settings(
    State(state): State<AppState>,
    Json(request): Json<UiSettingsUpdateRequest>,
) -> Response {
    let Some(config) = state.config else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration is unavailable",
        );
    };
    if let Some(message) = validate_settings_request(&request) {
        return api_error(StatusCode::UNPROCESSABLE_ENTITY, &message);
    }
    let context = match config.openlist_validation_context() {
        Ok(context) => context,
        Err(_) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not read configuration",
            );
        }
    };
    let current = match config.ui_settings() {
        Ok(settings) => settings,
        Err(_) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not read configuration",
            );
        }
    };

    let candidate_url = match request.openlist_url.as_deref() {
        Some(value) => match OpenListClient::new(value.trim(), &context.token) {
            Ok(client) => client.base_url().to_string(),
            Err(error) => return api_error(StatusCode::BAD_REQUEST, &error.to_string()),
        },
        None => context.base_url.clone(),
    };
    let needs_openlist_validation =
        request.openlist_url.is_some() || request.download_path.is_some();
    let validated_path = if needs_openlist_validation {
        let client = match OpenListClient::new(&candidate_url, &context.token) {
            Ok(client) => client,
            Err(error) => return api_error(StatusCode::BAD_REQUEST, &error.to_string()),
        };
        if request.openlist_url.is_some()
            && !client
                .validate_endpoint(&context.offline_download_tool)
                .await
        {
            return api_error(
                StatusCode::BAD_REQUEST,
                &format!("无法连接或验证 OpenList：{candidate_url}"),
            );
        }
        let path = request
            .download_path
            .as_deref()
            .map(str::trim)
            .unwrap_or(&context.download_path);
        let (valid, result) = client.validate_path(path).await;
        if !valid {
            return api_error(StatusCode::BAD_REQUEST, &result);
        }
        Some(result)
    } else {
        None
    };
    if let Some(format) = request.rename_format.as_deref() {
        if validate_rename_format(format.trim()).is_err() {
            return api_error(StatusCode::BAD_REQUEST, "重命名规则无效");
        }
    }
    let mikan_base_url = match request.mikan_base_url.as_deref() {
        Some(value) => match normalize_ui_http_url(value, "Mikan 地址") {
            Ok(value) => Some(value),
            Err(message) => return api_error(StatusCode::BAD_REQUEST, &message),
        },
        None => None,
    };
    let global_exclude_patterns = split_exclude_patterns(&request.global_exclude_patterns);
    let changes = UiSettingsUpdate {
        global_exclude_patterns,
        openlist_url: request.openlist_url.map(|_| candidate_url),
        download_path: request
            .download_path
            .map(|_| validated_path.unwrap_or_default()),
        rename_format: request.rename_format.map(|value| value.trim().to_string()),
        poll_interval_seconds: request.poll_interval_seconds,
        max_download_retries: request.max_download_retries,
        mikan_base_url: mikan_base_url.clone(),
        llm_provider_type: non_empty(request.llm_provider_type),
        llm_api_key: non_empty(request.llm_api_key),
        llm_base_url: non_empty(request.llm_base_url)
            .map(|value| value.trim_end_matches('/').to_string()),
        llm_model: non_empty(request.llm_model),
        tmdb_language: non_empty(request.tmdb_language),
        metadata_parser_provider: non_empty(request.metadata_parser_provider),
    };
    match config.update_ui_settings(&context.revision, changes) {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": "设置已保存；LLM/解析器相关修改将在服务重启后生效。",
                "requires_restart": true,
                "mikan_base_url": mikan_base_url.unwrap_or(current.mikan_base_url),
            })),
        )
            .into_response(),
        Err(ConfigStoreError::RevisionConflict) => api_error(
            StatusCode::CONFLICT,
            "configuration changed; reload and retry",
        ),
        Err(ConfigStoreError::InvalidShape | ConfigStoreError::InvalidConfig(_)) => {
            api_error(StatusCode::BAD_REQUEST, "configuration update is invalid")
        }
        Err(_) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "could not save settings"),
    }
}

fn validate_settings_request(request: &UiSettingsUpdateRequest) -> Option<String> {
    let max_lengths = [
        (request.openlist_url.as_deref(), 1000),
        (request.llm_provider_type.as_deref(), 50),
        (request.llm_api_key.as_deref(), 500),
        (request.llm_base_url.as_deref(), 1000),
        (request.llm_model.as_deref(), 200),
        (request.tmdb_language.as_deref(), 30),
        (request.metadata_parser_provider.as_deref(), 50),
        (request.download_path.as_deref(), 1000),
        (request.rename_format.as_deref(), 1000),
        (request.mikan_base_url.as_deref(), 1000),
    ];
    if request.global_exclude_patterns.chars().count() > 4000
        || max_lengths
            .iter()
            .any(|(value, limit)| value.is_some_and(|value| value.chars().count() > *limit))
    {
        return Some("setting value exceeds its maximum length".to_string());
    }
    if request
        .poll_interval_seconds
        .is_some_and(|value| !(60..=86_400).contains(&value))
    {
        return Some("RSS 轮询间隔必须在 60–86400 秒之间".to_string());
    }
    if request
        .max_download_retries
        .is_some_and(|value| !(0..=100).contains(&value))
    {
        return Some("最大下载重试次数必须在 0–100 之间".to_string());
    }
    None
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_ui_http_url(value: &str, label: &str) -> Result<String, String> {
    let normalized = value.trim();
    let parsed =
        reqwest::Url::parse(normalized).map_err(|_| format!("{label}必须是完整的 http(s) 地址"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(format!("{label}必须是完整的 http(s) 地址"));
    }
    Ok(format!("{}/", normalized.trim_end_matches('/')))
}

fn split_exclude_patterns(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let (mut parentheses, mut brackets, mut braces) = (0usize, 0usize, 0usize);
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        match character {
            '(' => parentheses += 1,
            ')' => parentheses = parentheses.saturating_sub(1),
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            '{' => braces += 1,
            '}' => braces = braces.saturating_sub(1),
            '|' if parentheses == 0 && brackets == 0 && braces == 0 => {
                parts.push(value[start..index].trim().to_string());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(value[start..].trim().to_string());
    let mut seen = std::collections::HashSet::new();
    parts.retain(|part| !part.is_empty() && seen.insert(part.clone()));
    parts
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

#[derive(Deserialize)]
struct ParseRssRequest {
    url: String,
    limit: Option<i64>,
}

async fn parse_rss_endpoint(Json(request): Json<ParseRssRequest>) -> Json<ParseRssResponse> {
    if request.url.trim().is_empty() {
        return Json(ParseRssResponse::failure("'url' is required."));
    }
    match fetch_parse_rss(&request.url, request.limit).await {
        Ok(response) => Json(response),
        Err(error) => Json(ParseRssResponse::failure(rss_error_message(error))),
    }
}

fn rss_error_message(error: RssError) -> String {
    match error {
        RssError::InvalidUrl => {
            "Cannot pick feed source: RSS URL must be a complete HTTP(S) URL".to_string()
        }
        RssError::Request(_) => "Failed to fetch RSS: upstream request failed".to_string(),
        RssError::TooLarge => "Failed to fetch RSS: response exceeds the 10 MiB limit".to_string(),
        RssError::InvalidEncoding | RssError::InvalidXml => {
            "Failed to fetch RSS: response is not valid RSS/Atom XML".to_string()
        }
    }
}

#[derive(Deserialize)]
struct ResolveTorrentRequest {
    url: String,
}

async fn resolve_torrent_endpoint(
    Json(request): Json<ResolveTorrentRequest>,
) -> Json<ResolveTorrentResponse> {
    match fetch_resolve_torrent(&request.url).await {
        Ok(response) => Json(response),
        Err(error) => Json(ResolveTorrentResponse::failure(torrent_error_message(
            error,
        ))),
    }
}

fn torrent_error_message(error: TorrentError) -> String {
    match error {
        TorrentError::InvalidUrl => {
            "Invalid torrent URL (expected 'http(s)://.../*.torrent').".to_string()
        }
        TorrentError::Request(Some(status)) => {
            format!("Failed to download .torrent: HTTP {status}.")
        }
        TorrentError::Request(None) => {
            "Failed to download .torrent: HTTP client request failed.".to_string()
        }
        TorrentError::TooLarge => {
            "Failed to download .torrent: file exceeds 10485760 bytes; refusing to read.".to_string()
        }
        TorrentError::InvalidMetadata => "Downloaded the .torrent file but could not parse a name from it. Ask the user for the release title; do NOT fabricate one.".to_string(),
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
