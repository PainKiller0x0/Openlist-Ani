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
