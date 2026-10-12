use std::fs;

use axum::{body::Body, http::Request};
use openlist_ani_rs::{AppState, router};
use rusqlite::Connection;
use serde_json::Value;
use tempfile::tempdir;
use tower::ServiceExt;

fn create_legacy_databases(data_path: &std::path::Path, tasks_path: &std::path::Path) {
    let data = Connection::open(data_path).unwrap();
    data.execute_batch(
        "CREATE TABLE resources (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            url TEXT NOT NULL,
            title TEXT UNIQUE NOT NULL,
            anime_name TEXT,
            season INTEGER,
            episode INTEGER,
            fansub TEXT,
            quality TEXT,
            languages TEXT,
            version INTEGER,
            downloaded_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            episode_name TEXT
        );
        CREATE TABLE strict_rejections (
            url TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            rejected_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .unwrap();
    data.execute(
        "INSERT INTO resources (url, title, anime_name, season, episode) VALUES (?1, ?2, ?3, ?4, ?5)",
        (
            "https://example.test/jojo-3",
            "jojo episode 3",
            "套马的汉子",
            8,
            3,
        ),
    )
    .unwrap();

    let tasks = Connection::open(tasks_path).unwrap();
    tasks
        .execute_batch(
            "CREATE TABLE task_mementos (
                task_id TEXT PRIMARY KEY,
                state TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                payload TEXT NOT NULL
            );
            CREATE TABLE task_memento_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )
        .unwrap();
}

#[tokio::test]
async fn ui_state_reads_subscription_progress_and_reports_scanner_as_active() {
    let directory = tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    let data_path = directory.path().join("data.db");
    let tasks_path = directory.path().join("task_mementos.db");
    fs::write(
        &config_path,
        include_str!("../fixtures/config.synthetic.toml"),
    )
    .unwrap();
    create_legacy_databases(&data_path, &tasks_path);
    let app = router(AppState::new(&data_path, &tasks_path).with_config_file(&config_path));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/ui/state")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = axum::body::to_bytes(response.into_body(), 32 * 1024)
        .await
        .unwrap();
    let state: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(state["rss_urls"].as_array().unwrap().len(), 2);
    assert_eq!(state["rss_subscriptions"][0]["latest_episode"], 3);
    assert_eq!(
        state["rss_subscriptions"][0]["download_directory"],
        "/迅雷/videos/番/JOJO的奇妙冒险 (2012)/Season 6"
    );
    assert_eq!(state["rss_status"]["enabled"], true);
    assert_eq!(state["rss_status"]["running"], false);
    assert_eq!(state["rss_status"]["status"], "idle");
}
