use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, Path, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use fancy_regex::Regex;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::{
    auth::AuthSettings,
    config_store::{
        ConfigDocumentStore, ConfigStoreError, DownloadRuntimeSettings, UiRssSubscriptionInput,
        UiSettingsUpdate,
    },
    download_service::{
        DownloadServiceError, recover_submitted_tasks, run_download_worker, submit_download,
    },
    naming::{NamingInput, build_plan, validate_rename_format},
    openlist::OpenListClient,
    release_parser::parse_release_title,
    rss::{ParseRssResponse, RssError, parse_rss as fetch_parse_rss},
    rss_scan::{RssScanReport, scan_subscriptions, split_patterns as split_exclude_patterns},
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
    download_workers: Arc<Mutex<HashSet<String>>>,
    rss_scan_lock: Arc<tokio::sync::Mutex<()>>,
    rss_scan_running: Arc<AtomicBool>,
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
            download_workers: Arc::new(Mutex::new(HashSet::new())),
            rss_scan_lock: Arc::new(tokio::sync::Mutex::new(())),
            rss_scan_running: Arc::new(AtomicBool::new(false)),
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

    pub async fn recover_download_workers(&self) -> Result<(), String> {
        let Some(config) = &self.config else {
            return Ok(());
        };
        let config = Arc::clone(config);
        let settings = tokio::task::spawn_blocking(move || config.download_runtime_settings())
            .await
            .map_err(|_| "could not load download settings".to_string())?
            .map_err(|_| "could not load download settings".to_string())?;
        let task_ids = recover_submitted_tasks((*self.databases).clone())
            .await
            .map_err(|_| "could not recover submitted downloads".to_string())?;
        for task_id in task_ids {
            self.start_download_worker(task_id, settings.clone());
        }
        Ok(())
    }

    pub fn start_periodic_rss_scanner(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                run_rss_scan_logged(&state).await;
                let seconds = state.rss_poll_interval().await.unwrap_or(300).max(60) as u64;
                tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
            }
        });
    }

    fn schedule_rss_scan(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            run_rss_scan_logged(&state).await;
        });
    }

    async fn rss_poll_interval(&self) -> Option<i64> {
        let config = self.config.as_ref().cloned()?;
        tokio::task::spawn_blocking(move || {
            config
                .ui_settings()
                .map(|settings| settings.poll_interval_seconds)
        })
        .await
        .ok()?
        .ok()
    }

    fn start_download_worker(&self, task_id: String, settings: DownloadRuntimeSettings) {
        let Ok(mut active) = self.download_workers.lock() else {
            return;
        };
        if !active.insert(task_id.clone()) {
            return;
        }
        drop(active);

        let databases = (*self.databases).clone();
        let active = Arc::clone(&self.download_workers);
        tokio::spawn(async move {
            run_download_worker(databases, settings, task_id.clone()).await;
            if let Ok(mut active) = active.lock() {
                active.remove(&task_id);
            }
        });
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
        .route("/api/ui/rss/preview", post(ui_preview_rss))
        .route("/api/ui/rss", post(ui_add_rss).delete(ui_remove_rss))
        .route("/api/ui/rss/toggle", post(ui_toggle_rss))
        .route("/api/ui/rss/completed", post(ui_set_rss_completed))
        .route("/api/ui/rss/exclude", post(ui_update_rss_exclude))
        .route("/api/ui/rss/remove", post(ui_remove_rss))
        .route("/api/ui/rss/correct", post(ui_correct_rss))
        .route("/api/ui/rss/filter", post(ui_update_rss_filter))
        .route("/api/ui/scan", post(ui_scan_now))
        .route("/api/downloads", get(list_downloads).post(create_download))
        .route("/api/downloads/{task_id}", get(get_download))
        .route("/api/parse_rss", post(parse_rss_endpoint))
        .route("/api/resolve_torrent", post(resolve_torrent_endpoint))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, auth_gate))
}

#[derive(Deserialize)]
struct CreateDownloadRequest {
    download_url: String,
    title: String,
}

async fn create_download(
    State(state): State<AppState>,
    Json(request): Json<CreateDownloadRequest>,
) -> Response {
    let Some(config) = state.config.as_ref().cloned() else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration is unavailable",
        );
    };
    let settings =
        match tokio::task::spawn_blocking(move || config.download_runtime_settings()).await {
            Ok(Ok(settings)) => settings,
            _ => {
                return api_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "download configuration is unavailable",
                );
            }
        };
    match submit_download(
        (*state.databases).clone(),
        settings.clone(),
        &request.title,
        &request.download_url,
    )
    .await
    {
        Ok(submitted) => {
            state.start_download_worker(submitted.task_id.clone(), settings);
            match task_response(
                &submitted.task_id,
                &submitted.state,
                &submitted.payload_json,
            ) {
                Ok(Some(task)) => Json(json!({
                    "success": true,
                    "message": format!("Download started: {}", request.title),
                    "task": task
                }))
                .into_response(),
                _ => api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "stored download task is invalid",
                ),
            }
        }
        Err(error) => download_service_error(error),
    }
}

fn download_service_error(error: DownloadServiceError) -> Response {
    match error {
        DownloadServiceError::InvalidUrl => api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "download URL must be magnet or HTTP(S)",
        ),
        DownloadServiceError::InvalidTitle => api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "title must include a recognizable season and episode",
        ),
        DownloadServiceError::Duplicate(task_id) => api_error(
            StatusCode::CONFLICT,
            &format!("Already downloading or previously submitted as task {task_id}"),
        ),
        DownloadServiceError::Rejected => {
            api_error(StatusCode::BAD_GATEWAY, "OpenList rejected the download")
        }
        DownloadServiceError::Uncertain => api_error(
            StatusCode::BAD_GATEWAY,
            "OpenList submission result is uncertain; task will not be resent",
        ),
        DownloadServiceError::InvalidConfiguration => api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "download configuration is invalid",
        ),
        _ => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not create download task",
        ),
    }
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
    let Some(config) = state.config.as_ref().cloned() else {
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
    let Some(config) = state.config.as_ref().cloned() else {
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

async fn run_rss_scan(state: &AppState) -> Result<RssScanReport, String> {
    let _scan = state.rss_scan_lock.lock().await;
    state.rss_scan_running.store(true, Ordering::Relaxed);
    let _running = RssScanRunning(Arc::clone(&state.rss_scan_running));
    let config = state
        .config
        .as_ref()
        .cloned()
        .ok_or_else(|| "configuration is unavailable".to_string())?;
    let (settings, runtime, subscriptions) = tokio::task::spawn_blocking(move || {
        Ok::<_, ConfigStoreError>((
            config.ui_settings()?,
            config.download_runtime_settings()?,
            config.ui_subscriptions()?,
        ))
    })
    .await
    .map_err(|_| "configuration worker failed".to_string())?
    .map_err(|_| "configuration is unavailable".to_string())?;
    let report = scan_subscriptions(
        (*state.databases).clone(),
        runtime.clone(),
        &settings.global_exclude_patterns,
        &subscriptions,
    )
    .await?;
    for task in &report.submitted {
        state.start_download_worker(task.task_id.clone(), runtime.clone());
    }
    Ok(report)
}

async fn run_rss_scan_logged(state: &AppState) {
    match run_rss_scan(state).await {
        Ok(report) if !report.errors.is_empty() => {
            eprintln!("RSS scan partially failed: {}", report.errors.join("; "));
        }
        Ok(report) if !report.submitted.is_empty() => {
            eprintln!(
                "RSS scan submitted {} download task(s); skipped {} duplicate/invalid item(s)",
                report.submitted.len(),
                report.skipped
            );
        }
        Ok(_) => {}
        Err(error) => eprintln!("RSS scan failed: {error}"),
    }
}

struct RssScanRunning(Arc<AtomicBool>);

impl Drop for RssScanRunning {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

async fn ui_scan_now(State(state): State<AppState>) -> Response {
    match run_rss_scan(&state).await {
        Ok(report) => rss_scan_response(report),
        Err(_) => api_error(StatusCode::SERVICE_UNAVAILABLE, "RSS 扫描未能完成"),
    }
}

fn rss_scan_response(report: RssScanReport) -> Response {
    Json(json!({
        "success": report.errors.is_empty(),
        "feeds_scanned": report.feeds_scanned,
        "entries_seen": report.entries_seen,
        "submitted": report.submitted.len(),
        "task_ids": report.submitted.iter().map(|task| task.task_id.as_str()).collect::<Vec<_>>(),
        "skipped": report.skipped,
        "truncated_feeds": report.truncated_feeds,
        "errors": report.errors
    }))
    .into_response()
}

async fn ui_state(State(state): State<AppState>) -> Response {
    let Some(config) = state.config else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration is unavailable",
        );
    };
    let databases = Arc::clone(&state.databases);
    let rss_scan_running = state.rss_scan_running.load(Ordering::Relaxed);
    match tokio::task::spawn_blocking(move || load_ui_state(&config, &databases, rss_scan_running))
        .await
    {
        Ok(Ok(state)) => Json(state).into_response(),
        _ => api_error(StatusCode::INTERNAL_SERVER_ERROR, "could not read UI state"),
    }
}

fn load_ui_state(
    config: &ConfigDocumentStore,
    databases: &LegacyDbPaths,
    rss_scan_running: bool,
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
            "enabled": true,
            "running": rss_scan_running,
            "status": if rss_scan_running { "scanning" } else { "idle" },
            "message": if rss_scan_running { "正在扫描 RSS" } else { "Rust RSS 扫描已启用" }
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

#[derive(Default, Deserialize)]
#[serde(default)]
struct UiRssRequest {
    url: String,
    name: String,
    anime_name: String,
    download_directory_name: String,
    episode_offset: i64,
    filename_season_override: i64,
    directory_season_override: i64,
    tmdb_id: Option<i64>,
    poster_url: String,
    season: Option<i64>,
    exclude_patterns: String,
    confirmed: bool,
}

async fn ui_preview_rss(
    State(state): State<AppState>,
    Json(request): Json<UiRssRequest>,
) -> Response {
    if let Some(message) = validate_ui_rss_request(&request) {
        return api_error(StatusCode::UNPROCESSABLE_ENTITY, &message);
    }
    let Some(config) = state.config.as_ref().cloned() else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration is unavailable",
        );
    };
    let settings = match config.ui_settings() {
        Ok(settings) => settings,
        Err(_) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not read configuration",
            );
        }
    };
    let parsed = match fetch_parse_rss(&request.url, Some(80)).await {
        Ok(parsed) if parsed.success => parsed,
        Ok(parsed) => return api_error(StatusCode::BAD_REQUEST, &parsed.message),
        Err(error) => return api_error(StatusCode::BAD_REQUEST, &rss_error_message(error)),
    };
    let global_patterns = settings.global_exclude_patterns;
    let mut patterns = global_patterns.clone();
    patterns.extend(split_exclude_patterns(&request.exclude_patterns));
    let patterns = match patterns
        .iter()
        .map(|pattern| Regex::new(pattern).map(|regex| (pattern, regex)))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(patterns) => patterns,
        Err(_) => {
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "排除规则不是有效的正则表达式",
            );
        }
    };

    let mut entries = Vec::with_capacity(parsed.entries.len());
    let mut included = 0;
    let mut excluded = 0;
    let mut inferred_name = None;
    let mut inferred_fansub = None;
    let mut first_directory = None;
    for entry in parsed.entries {
        let matched_pattern =
            match patterns
                .iter()
                .find_map(|(pattern, regex)| match regex.is_match(&entry.title) {
                    Ok(true) => Some(Ok((*pattern).clone())),
                    Ok(false) => None,
                    Err(_) => Some(Err(())),
                }) {
                Some(Ok(pattern)) => Some(pattern),
                Some(Err(())) => {
                    return api_error(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "排除规则执行失败，请检查正则表达式",
                    );
                }
                None => None,
            };
        let parsed_release = parse_release_title(&entry.title);
        let mut rename_preview = None;
        let mut download_directory = None;
        let mut season = None;
        let mut episode = None;
        let mut anime_name = None;
        let mut llm_parsed = false;
        let mut reason = matched_pattern;
        if let Some(release) = parsed_release {
            let adjusted_episode = i64::from(release.episode) - request.episode_offset;
            anime_name = Some(request.anime_name.trim())
                .filter(|name| !name.is_empty())
                .or(Some(release.anime_name.as_str()))
                .map(str::to_string);
            season = Some(request.season.unwrap_or(i64::from(release.season)));
            episode = i32::try_from(adjusted_episode)
                .ok()
                .filter(|value| *value > 0);
            if inferred_name.is_none() {
                inferred_name = Some(release.anime_name.clone());
            }
            if inferred_fansub.is_none() {
                inferred_fansub = release.fansub.clone();
            }
            llm_parsed = true;
            if reason.is_none() && episode.is_none() {
                reason = Some("集数扣除后无效".to_string());
            }
            if reason.is_none() {
                let plan = build_plan(
                    &settings.download_path,
                    NamingInput {
                        anime_name: release.anime_name,
                        anime_name_override: non_empty(Some(request.anime_name.clone())),
                        download_directory_name_override: non_empty(Some(
                            request.download_directory_name.clone(),
                        )),
                        season: season.and_then(|value| i32::try_from(value).ok()),
                        episode,
                        filename_season_override: positive_override(
                            request.filename_season_override,
                        ),
                        directory_season_override: positive_override(
                            request.directory_season_override,
                        ),
                        extension: Some(".mp4".to_string()),
                        version: Some(1),
                        rename_format: Some(settings.rename_format.clone()),
                        fansub: release.fansub,
                        quality: release.quality,
                        languages: release.languages,
                    },
                );
                match plan {
                    Ok(plan) => {
                        rename_preview = plan.filename.strip_suffix(".mp4").map(str::to_string);
                        download_directory = Some(plan.directory.clone());
                        first_directory.get_or_insert(plan.directory);
                    }
                    Err(_) => reason = Some("无法生成安全的文件名或目录".to_string()),
                }
            }
        } else if reason.is_none() {
            reason = Some("未能识别季集编号".to_string());
        }
        if reason.is_some() {
            excluded += 1;
        } else {
            included += 1;
        }
        entries.push(json!({
            "title": entry.title,
            "download_url": entry.download_url,
            "anime_name": anime_name,
            "season": season,
            "episode": episode,
            "fansub": entry.fansub,
            "quality": entry.quality,
            "languages": entry.languages,
            "excluded": reason.is_some(),
            "matched_pattern": reason,
            "llm_parsed": llm_parsed,
            "rename_preview": rename_preview,
            "download_directory": download_directory
        }));
    }
    let name = [request.name.trim(), inferred_name.as_deref().unwrap_or("")]
        .into_iter()
        .find(|name| !name.is_empty())
        .unwrap_or("未命名订阅");
    Json(json!({
        "success": true,
        "message": "RSS 预览已生成；未写入订阅配置",
        "url": request.url,
        "name": name,
        "anime_name": request.anime_name.trim(),
        "download_directory_name": request.download_directory_name.trim(),
        "episode_offset": request.episode_offset,
        "filename_season_override": request.filename_season_override,
        "directory_season_override": request.directory_season_override,
        "tmdb_id": request.tmdb_id,
        "poster_url": request.poster_url,
        "fansub": inferred_fansub,
        "season": request.season,
        "download_path": settings.download_path,
        "rename_format": settings.rename_format,
        "global_exclude_patterns": global_patterns,
        "exclude_patterns": split_exclude_patterns(&request.exclude_patterns),
        "download_directory": first_directory,
        "total": parsed.total,
        "included": included,
        "excluded": excluded,
        "truncated": parsed.total > entries.len(),
        "entries": entries
    }))
    .into_response()
}

async fn ui_add_rss(State(state): State<AppState>, Json(request): Json<UiRssRequest>) -> Response {
    if let Some(message) = validate_ui_rss_request(&request) {
        return api_error(StatusCode::UNPROCESSABLE_ENTITY, &message);
    }
    if !request.confirmed {
        return api_error(
            StatusCode::CONFLICT,
            "请先完成 RSS 识别预览，再点击确认保存",
        );
    }
    let parsed = match fetch_parse_rss(&request.url, Some(1)).await {
        Ok(parsed) if parsed.success && !parsed.entries.is_empty() => parsed,
        Ok(parsed) => return api_error(StatusCode::BAD_REQUEST, &parsed.message),
        Err(error) => return api_error(StatusCode::BAD_REQUEST, &rss_error_message(error)),
    };
    let Some(config) = state.config.as_ref().cloned() else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration is unavailable",
        );
    };
    let config_for_write = Arc::clone(&config);
    let input = rss_subscription_input(&request);
    let result = tokio::task::spawn_blocking(move || {
        config_for_write.add_ui_subscription(input)?;
        let urls = config_for_write
            .ui_subscriptions()?
            .into_iter()
            .filter(|item| {
                item.get("enabled").and_then(serde_json::Value::as_bool) != Some(false)
                    && item.get("completed").and_then(serde_json::Value::as_bool) != Some(true)
            })
            .filter_map(|item| {
                item.get("url")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        Ok::<_, ConfigStoreError>(urls)
    })
    .await;
    match result {
        Ok(Ok(urls)) => {
            state.schedule_rss_scan();
            Json(json!({
            "success": true,
            "message": "RSS 已保存，并已触发后台扫描；新条目也会继续按全局周期检查。",
            "urls": urls,
            "preview": parsed.entries.first().map(|entry| entry.title.as_str()).unwrap_or_default(),
            "name": request.name.trim(),
            "anime_name": if request.anime_name.trim().is_empty() { request.name.trim() } else { request.anime_name.trim() },
            "download_directory_name": request.download_directory_name.trim(),
            "episode_offset": request.episode_offset,
            "filename_season_override": request.filename_season_override,
            "directory_season_override": request.directory_season_override,
            "tmdb_id": request.tmdb_id,
            "poster_url": request.poster_url.trim(),
            "exclude_patterns": split_exclude_patterns(&request.exclude_patterns),
            "scan_scheduled": true
        }))
            .into_response()
        }
        Ok(Err(ConfigStoreError::DuplicateSubscription)) => {
            api_error(StatusCode::CONFLICT, "该 RSS 订阅已存在")
        }
        Ok(Err(ConfigStoreError::RevisionConflict)) => {
            api_error(StatusCode::CONFLICT, "配置刚被其他操作修改，请重试")
        }
        _ => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not save RSS subscription",
        ),
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct UiSubscriptionToggleRequest {
    url: String,
    enabled: bool,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct UiSubscriptionCompletedRequest {
    url: String,
    completed: bool,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct UiSubscriptionRemoveRequest {
    url: String,
}

#[derive(Deserialize)]
struct UiSubscriptionCorrectRequest {
    original_url: String,
    #[serde(flatten)]
    request: UiRssRequest,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct UiRssFilterRequest {
    exclude_patterns: String,
}

async fn ui_toggle_rss(
    State(state): State<AppState>,
    Json(request): Json<UiSubscriptionToggleRequest>,
) -> Response {
    let Some(config) = state.config.as_ref().cloned() else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration is unavailable",
        );
    };
    if request.url.trim().is_empty() {
        return api_error(StatusCode::UNPROCESSABLE_ENTITY, "RSS 地址必填");
    }
    let should_scan = request.enabled;
    match rss_config_urls(config, move |config| {
        config.set_ui_subscription_enabled(request.url.trim(), request.enabled)
    })
    .await
    {
        Ok(urls) => {
            if should_scan {
                state.schedule_rss_scan();
            }
            Json(json!({"success": true, "message": "RSS 追踪状态已更新", "urls": urls}))
                .into_response()
        }
        Err(error) => rss_mutation_error(error),
    }
}

async fn ui_set_rss_completed(
    State(state): State<AppState>,
    Json(request): Json<UiSubscriptionCompletedRequest>,
) -> Response {
    let Some(config) = state.config.as_ref().cloned() else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration is unavailable",
        );
    };
    if request.url.trim().is_empty() {
        return api_error(StatusCode::UNPROCESSABLE_ENTITY, "RSS 地址必填");
    }
    let should_scan = !request.completed;
    match rss_config_urls(config, move |config| {
        config.set_ui_subscription_completed(request.url.trim(), request.completed)
    })
    .await
    {
        Ok(urls) => {
            if should_scan {
                state.schedule_rss_scan();
            }
            Json(json!({"success": true, "message": "RSS 完结状态已更新", "urls": urls}))
                .into_response()
        }
        Err(error) => rss_mutation_error(error),
    }
}

async fn ui_remove_rss(
    State(state): State<AppState>,
    Json(request): Json<UiSubscriptionRemoveRequest>,
) -> Response {
    let Some(config) = state.config.as_ref().cloned() else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration is unavailable",
        );
    };
    if request.url.trim().is_empty() {
        return api_error(StatusCode::UNPROCESSABLE_ENTITY, "RSS 地址必填");
    }
    match rss_config_urls(config, move |config| {
        config.remove_ui_subscription(request.url.trim())
    })
    .await
    {
        Ok(urls) => Json(json!({"success": true, "message": "RSS 订阅已删除", "urls": urls}))
            .into_response(),
        Err(error) => rss_mutation_error(error),
    }
}

async fn ui_correct_rss(
    State(state): State<AppState>,
    Json(request): Json<UiSubscriptionCorrectRequest>,
) -> Response {
    if request.original_url.trim().is_empty() {
        return api_error(StatusCode::UNPROCESSABLE_ENTITY, "原 RSS 地址必填");
    }
    if let Some(message) = validate_ui_rss_request(&request.request) {
        return api_error(StatusCode::UNPROCESSABLE_ENTITY, &message);
    }
    match fetch_parse_rss(&request.request.url, Some(1)).await {
        Ok(parsed) if parsed.success && !parsed.entries.is_empty() => {}
        Ok(parsed) => return api_error(StatusCode::BAD_REQUEST, &parsed.message),
        Err(error) => return api_error(StatusCode::BAD_REQUEST, &rss_error_message(error)),
    }
    let Some(config) = state.config.as_ref().cloned() else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration is unavailable",
        );
    };
    let input = rss_subscription_input(&request.request);
    match rss_config_urls(config, move |config| {
        config.correct_ui_subscription(&request.original_url, input)
    })
    .await
    {
        Ok(urls) => {
            state.schedule_rss_scan();
            Json(json!({"success": true, "message": "RSS 订阅信息已修正", "urls": urls}))
                .into_response()
        }
        Err(error) => rss_mutation_error(error),
    }
}

async fn ui_update_rss_filter(
    State(state): State<AppState>,
    Json(request): Json<UiRssFilterRequest>,
) -> Response {
    if request.exclude_patterns.chars().count() > 4_000 {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "全局排除规则不能超过 4000 个字符",
        );
    }
    let patterns = split_exclude_patterns(&request.exclude_patterns);
    if patterns.iter().any(|pattern| Regex::new(pattern).is_err()) {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "排除规则不是有效的正则表达式",
        );
    }
    let Some(config) = state.config.as_ref().cloned() else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration is unavailable",
        );
    };
    match rss_config_urls(config, move |config| {
        config.update_ui_global_exclusions(patterns)
    })
    .await
    {
        Ok(urls) => {
            Json(json!({"success": true, "message": "全局 RSS 排除规则已保存", "urls": urls}))
                .into_response()
        }
        Err(error) => rss_mutation_error(error),
    }
}

async fn ui_update_rss_exclude(
    State(state): State<AppState>,
    Json(request): Json<UiRssRequest>,
) -> Response {
    if let Some(message) = validate_ui_rss_request(&request) {
        return api_error(StatusCode::UNPROCESSABLE_ENTITY, &message);
    }
    let patterns = split_exclude_patterns(&request.exclude_patterns);
    if patterns.iter().any(|pattern| Regex::new(pattern).is_err()) {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "排除规则不是有效的正则表达式",
        );
    }
    let Some(config) = state.config.as_ref().cloned() else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "configuration is unavailable",
        );
    };
    let response_patterns = patterns.clone();
    match rss_config_urls(config, move |config| {
        config.set_ui_subscription_exclusions(request.url.trim(), patterns)
    })
    .await
    {
        Ok(urls) => Json(json!({
            "success": true,
            "message": "单个 RSS 排除规则已保存，下一次扫描立即生效",
            "urls": urls,
            "exclude_patterns": response_patterns
        }))
        .into_response(),
        Err(error) => rss_mutation_error(error),
    }
}

fn rss_subscription_input(request: &UiRssRequest) -> UiRssSubscriptionInput {
    UiRssSubscriptionInput {
        url: request.url.trim().to_string(),
        name: request.name.trim().to_string(),
        anime_name: if request.anime_name.trim().is_empty() {
            request.name.trim().to_string()
        } else {
            request.anime_name.trim().to_string()
        },
        download_directory_name: request.download_directory_name.trim().to_string(),
        episode_offset: request.episode_offset,
        filename_season_override: request.filename_season_override,
        directory_season_override: request.directory_season_override,
        tmdb_id: request.tmdb_id,
        poster_url: request.poster_url.trim().to_string(),
        season: request.season.unwrap_or(1),
        exclude_patterns: split_exclude_patterns(&request.exclude_patterns),
    }
}

enum RssMutationError {
    Config(ConfigStoreError),
    Worker,
}

async fn rss_config_urls(
    config: Arc<ConfigDocumentStore>,
    update: impl FnOnce(
        &ConfigDocumentStore,
    ) -> Result<crate::config_store::ConfigSnapshot, ConfigStoreError>
    + Send
    + 'static,
) -> Result<Vec<String>, RssMutationError> {
    tokio::task::spawn_blocking(move || {
        update(&config).map_err(RssMutationError::Config)?;
        let subscriptions = config
            .ui_subscriptions()
            .map_err(RssMutationError::Config)?;
        Ok(subscriptions
            .into_iter()
            .filter(|item| {
                item.get("enabled").and_then(serde_json::Value::as_bool) != Some(false)
                    && item.get("completed").and_then(serde_json::Value::as_bool) != Some(true)
            })
            .filter_map(|item| {
                item.get("url")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect())
    })
    .await
    .map_err(|_| RssMutationError::Worker)?
}

fn rss_mutation_error(error: RssMutationError) -> Response {
    match error {
        RssMutationError::Config(ConfigStoreError::SubscriptionNotFound) => {
            api_error(StatusCode::NOT_FOUND, "RSS 订阅不存在")
        }
        RssMutationError::Config(ConfigStoreError::DuplicateSubscription) => {
            api_error(StatusCode::CONFLICT, "该 RSS 地址已被其他订阅使用")
        }
        RssMutationError::Config(ConfigStoreError::RevisionConflict) => {
            api_error(StatusCode::CONFLICT, "配置刚被其他操作修改，请重试")
        }
        _ => api_error(StatusCode::INTERNAL_SERVER_ERROR, "RSS 配置更新失败"),
    }
}

fn validate_ui_rss_request(request: &UiRssRequest) -> Option<String> {
    if request.url.trim().is_empty() || request.url.chars().count() > 2_000 {
        return Some("RSS 地址必填且不能超过 2000 个字符".to_string());
    }
    if [
        request.name.as_str(),
        request.anime_name.as_str(),
        request.download_directory_name.as_str(),
    ]
    .iter()
    .any(|value| value.chars().count() > 200)
    {
        return Some("名称字段不能超过 200 个字符".to_string());
    }
    if request.poster_url.chars().count() > 1_000
        || request.exclude_patterns.chars().count() > 2_000
    {
        return Some("海报地址或排除规则过长".to_string());
    }
    if !(0..=999).contains(&request.episode_offset)
        || !(0..=99).contains(&request.filename_season_override)
        || !(0..=99).contains(&request.directory_season_override)
        || request
            .season
            .is_some_and(|season| !(0..=99).contains(&season))
        || request.tmdb_id.is_some_and(|id| id <= 0)
    {
        return Some("集数扣除值、季号覆盖、季号或 TMDB ID 超出范围".to_string());
    }
    None
}

fn positive_override(value: i64) -> Option<i32> {
    (value > 0).then(|| i32::try_from(value).ok()).flatten()
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
