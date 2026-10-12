use std::fs;

use axum::{body::Body, http::Request};
use openlist_ani_rs::{AppState, router};
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn ui_settings_returns_legacy_safe_fields_without_secrets() {
    let directory = tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    fs::write(
        &config_path,
        include_str!("../fixtures/config.synthetic.toml"),
    )
    .unwrap();
    let app = router(
        AppState::new(
            directory.path().join("unused-data.db"),
            directory.path().join("unused-tasks.db"),
        )
        .with_config_file(&config_path),
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/ui/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let raw = String::from_utf8(body.to_vec()).unwrap();
    assert!(!raw.contains("synthetic-api-key-marker"));
    assert!(!raw.contains("synthetic-tmdb-marker"));
    let settings: Value = serde_json::from_str(&raw).unwrap();

    assert_eq!(
        settings,
        json!({
            "global_exclude_patterns": [],
            "llm": {
                "provider_type": "openai",
                "base_url": "http://127.0.0.1:19003/v1",
                "model": "synthetic-model",
                "api_key_configured": true,
                "tmdb_language": "zh-CN",
                "metadata_parser_provider": "llm"
            },
            "download_path": "/迅雷/videos/番",
            "openlist_url": "http://127.0.0.1:19001",
            "rename_format": "{anime_name} - S{season:02d}E{episode:02d}",
            "poll_interval_seconds": 300,
            "max_download_retries": 3,
            "mikan_base_url": "http://127.0.0.1:19002/"
        })
    );
}

#[tokio::test]
async fn ui_settings_reports_unavailable_without_a_config_path() {
    let directory = tempdir().unwrap();
    let app = router(AppState::new(
        directory.path().join("unused-data.db"),
        directory.path().join("unused-tasks.db"),
    ));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/ui/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
}

#[tokio::test]
async fn settings_update_validates_openlist_then_atomically_preserves_secrets() {
    use axum::{
        Json, Router,
        http::HeaderMap,
        routing::{get, post},
    };
    use serde_json::json;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let openlist = Router::new()
        .route(
            "/api/public/settings",
            get(|| async { Json(json!({"code": 200})) }),
        )
        .route(
            "/api/public/offline_download_tools",
            get(|| async { Json(json!({"code": 200, "data": [{"name": "Thunder"}]})) }),
        )
        .route(
            "/api/fs/list",
            post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(
                    headers.get("authorization").unwrap(),
                    "synthetic-secret-marker"
                );
                let valid = body["path"] != "/invalid";
                Json(json!({"code": if valid { 200 } else { 404 }}))
            }),
        );
    tokio::spawn(async move {
        axum::serve(listener, openlist).await.unwrap();
    });

    let directory = tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    let fixture = include_str!("../fixtures/config.synthetic.toml")
        .replace("http://127.0.0.1:19001", &format!("http://{address}"));
    fs::write(&config_path, fixture).unwrap();
    let app = router(
        AppState::new(
            directory.path().join("unused-data.db"),
            directory.path().join("unused-tasks.db"),
        )
        .with_config_file(&config_path),
    );
    let original = fs::read(&config_path).unwrap();
    let body = json!({
        "global_exclude_patterns": "promo|(ABEMA|CR)",
        "openlist_url": format!("http://{address}"),
        "download_path": "/media/anime",
        "poll_interval_seconds": 600,
        "max_download_retries": 4,
        "mikan_base_url": "https://mikanani.kas.pub",
        "rename_format": "{anime_name} - S{season:02d}E{episode:02d}",
        "llm_provider_type": "openai-compatible",
        "llm_base_url": "https://llm.example.test/v1",
        "llm_model": "test-model",
        "tmdb_language": "zh-CN",
        "metadata_parser_provider": "regex"
    });

    let failed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ui/settings")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "global_exclude_patterns": "should-not-save",
                        "openlist_url": format!("http://{address}"),
                        "download_path": "/invalid"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(failed.status(), 400);
    assert_eq!(fs::read(&config_path).unwrap(), original);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ui/settings")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response_body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let response_json: Value = serde_json::from_slice(&response_body).unwrap();
    assert_eq!(response_json["requires_restart"], true);
    assert_eq!(response_json["mikan_base_url"], "https://mikanani.kas.pub/");

    let saved = fs::read_to_string(&config_path).unwrap();
    assert!(saved.contains("synthetic-api-key-marker"));
    assert!(saved.contains("synthetic-tmdb-marker"));
    assert!(saved.contains("opaque_setting = \"do-not-delete\""));
    assert!(saved.contains("interval_time = 600"));
    assert!(saved.contains("max_download_retries = 4"));

    let settings = app
        .oneshot(
            Request::builder()
                .uri("/api/ui/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let settings_body = axum::body::to_bytes(settings.into_body(), 8192)
        .await
        .unwrap();
    let settings: Value = serde_json::from_slice(&settings_body).unwrap();
    assert_eq!(
        settings["global_exclude_patterns"],
        json!(["promo", "(ABEMA|CR)"])
    );
    assert_eq!(settings["download_path"], "/media/anime");
    assert_eq!(settings["llm"]["api_key_configured"], true);
    assert!(
        !settings_body
            .windows(b"synthetic-api-key-marker".len())
            .any(|window| window == b"synthetic-api-key-marker")
    );
}
