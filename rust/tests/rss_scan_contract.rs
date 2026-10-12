use axum::{
    Json, Router,
    routing::{get, post},
};
use openlist_ani_rs::{
    config_store::DownloadRuntimeSettings, rss_scan::scan_subscriptions, storage::LegacyDbPaths,
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tempfile::tempdir;

fn create_legacy_databases(data_path: &std::path::Path, tasks_path: &std::path::Path) {
    Connection::open(data_path)
        .unwrap()
        .execute_batch(
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
            );
            INSERT INTO resources(url, title, anime_name, season, episode)
            VALUES ('magnet:?xt=urn:btih:old', 'previous release', 'Archive Name', 2, 3);",
        )
        .unwrap();
    Connection::open(tasks_path)
        .unwrap()
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
async fn scan_applies_subscription_identity_offset_and_database_deduplication() {
    const FEED: &str = r#"<?xml version="1.0"?><rss><channel>
        <item><title>[Group] Original Show - 15 [1080P]</title><enclosure url="magnet:?xt=urn:btih:new" type="application/x-bittorrent"/></item>
    </channel></rss>"#;
    let source = Router::new().route("/feed", get(|| async { FEED }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, source).await.unwrap();
    });

    let directory = tempdir().unwrap();
    let data_path = directory.path().join("data.db");
    let task_path = directory.path().join("tasks.db");
    create_legacy_databases(&data_path, &task_path);
    let report = scan_subscriptions(
        LegacyDbPaths {
            data_db: data_path,
            task_mementos_db: task_path,
        },
        DownloadRuntimeSettings {
            openlist_url: "http://127.0.0.1:1".to_string(),
            download_path: "/library".to_string(),
            token: "unused".to_string(),
            offline_download_tool: "qBittorrent".to_string(),
            rename_format: "{anime_name} - S{season:02d}E{episode:02d}".to_string(),
        },
        &[],
        &[json!({
            "url": format!("http://{address}/feed"),
            "name": "Display Name",
            "anime_name": "Archive Name",
            "episode_offset": 12,
            "season": 2,
            "enabled": true,
            "completed": false
        })],
    )
    .await
    .unwrap();

    assert_eq!(report.feeds_scanned, 1);
    assert_eq!(report.entries_seen, 1);
    assert_eq!(report.skipped, 1);
    assert!(report.submitted.is_empty());
    assert!(report.errors.is_empty());
}

#[tokio::test]
async fn scan_submits_episode_with_persisted_subscription_naming_rules() {
    const FEED: &str = r#"<?xml version="1.0"?><rss><channel>
        <item><title>[Group] Original Show - 15 [1080P]</title><enclosure url="magnet:?xt=urn:btih:new" type="application/x-bittorrent"/></item>
    </channel></rss>"#;
    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = Arc::clone(&requests);
    let server = Router::new().route("/feed", get(|| async { FEED })).route(
        "/api/fs/add_offline_download",
        post(move |Json(body): Json<Value>| {
            let request_count = Arc::clone(&request_count);
            async move {
                assert_eq!(body["name"], "Archive Name S02E03");
                request_count.fetch_add(1, Ordering::SeqCst);
                Json(json!({"code": 200, "data": {"tasks": [{"id": "remote-1"}]}}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, server).await.unwrap();
    });

    let directory = tempdir().unwrap();
    let data_path = directory.path().join("data.db");
    let task_path = directory.path().join("tasks.db");
    create_legacy_databases(&data_path, &task_path);
    Connection::open(&data_path)
        .unwrap()
        .execute("DELETE FROM resources", [])
        .unwrap();
    let report = scan_subscriptions(
        LegacyDbPaths {
            data_db: data_path,
            task_mementos_db: task_path,
        },
        DownloadRuntimeSettings {
            openlist_url: format!("http://{address}"),
            download_path: "/迅雷/videos/番".to_string(),
            token: "test-token".to_string(),
            offline_download_tool: "qBittorrent".to_string(),
            rename_format: "{anime_name} - S{season:02d}E{episode:02d}".to_string(),
        },
        &[],
        &[json!({
            "url": format!("http://{address}/feed"),
            "name": "Display Name",
            "anime_name": "Archive Name",
            "download_directory_name": "Archive Folder",
            "episode_offset": 12,
            "season": 2,
            "filename_season_override": 1,
            "directory_season_override": 6,
            "enabled": true,
            "completed": false
        })],
    )
    .await
    .unwrap();

    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(report.submitted.len(), 1);
    let payload: Value = serde_json::from_str(&report.submitted[0].payload_json).unwrap();
    assert_eq!(payload["release"]["anime_name"], "Archive Name");
    assert_eq!(payload["release"]["season"], 2);
    assert_eq!(payload["release"]["episode"], 3);
    assert_eq!(payload["release"]["episode_offset"], 12);
    assert_eq!(
        payload["release"]["source_url"],
        format!("http://{address}/feed")
    );
    assert_eq!(
        payload["target_directory"],
        "/迅雷/videos/番/Archive Folder/Season 6"
    );
}
