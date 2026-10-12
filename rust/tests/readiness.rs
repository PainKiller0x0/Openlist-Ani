use std::path::Path;

use axum::{body::Body, http::Request};
use openlist_ani_rs::{AppConfig, AppState, router};
use rusqlite::Connection;
use tempfile::tempdir;
use tower::ServiceExt;

fn create_legacy_databases(data_db: &Path, task_db: &Path) {
    let db = Connection::open(data_db).unwrap();
    db.execute_batch(
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
        CREATE INDEX idx_title ON resources(title);
        CREATE INDEX idx_anime_episode ON resources(anime_name, season, episode);
        CREATE TABLE strict_rejections (
            url TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            rejected_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        INSERT INTO resources (url, title, anime_name, season, episode, episode_name)
        VALUES ('https://example.invalid/ep3', 'release-3', 'show', 1, 3, 'episode 3');",
    )
    .unwrap();

    let db = Connection::open(task_db).unwrap();
    db.execute_batch(
        "CREATE TABLE task_mementos (
            task_id TEXT PRIMARY KEY,
            state TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            payload TEXT NOT NULL
        );
        CREATE INDEX idx_task_mementos_updated_at ON task_mementos(updated_at);
        CREATE TABLE task_memento_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        INSERT INTO task_memento_metadata (key, value) VALUES ('schema_version', '1');",
    )
    .unwrap();
}

#[tokio::test]
async fn readyz_accepts_legacy_schema_and_does_not_change_existing_data() {
    let dir = tempdir().unwrap();
    let data_db = dir.path().join("data.db");
    let task_db = dir.path().join("task_mementos.db");
    create_legacy_databases(&data_db, &task_db);
    let db = Connection::open(&data_db).unwrap();
    let table_count_before: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let response = router(AppState::new(&data_db, &task_db))
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let db = Connection::open(&data_db).unwrap();
    let row: (i64, String) = db
        .query_row(
            "SELECT COUNT(*), MAX(episode_name) FROM resources",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, (1, "episode 3".to_string()));
    let table_count_after: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(table_count_after, table_count_before);
}

#[tokio::test]
async fn readyz_reports_missing_databases_without_creating_them() {
    let dir = tempdir().unwrap();
    let data_db = dir.path().join("missing-data.db");
    let task_db = dir.path().join("missing-tasks.db");

    let response = router(AppState::new(&data_db, &task_db))
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 503);
    assert!(!data_db.exists());
    assert!(!task_db.exists());
}

#[test]
fn config_uses_legacy_backend_settings_and_rejects_public_bind_addresses() {
    let dir = tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(&config, "[backend]\nhost = \"127.0.0.1\"\nport = 26666\n").unwrap();

    let loaded = AppConfig::load(&config).unwrap();
    assert_eq!(loaded.bind.to_string(), "127.0.0.1:26666");
    assert_eq!(loaded.data_db, dir.path().join("data/data.db"));
    assert_eq!(
        loaded.task_mementos_db,
        dir.path().join("data/task_mementos.db")
    );

    std::fs::write(&config, "[backend]\nhost = \"0.0.0.0\"\nport = 26666\n").unwrap();
    assert!(AppConfig::load(&config).is_err());
}

#[tokio::test]
async fn healthz_reports_process_liveness_without_database_access() {
    let dir = tempdir().unwrap();
    let response = router(AppState::new(
        dir.path().join("missing-data.db"),
        dir.path().join("missing-tasks.db"),
    ))
    .oneshot(
        Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(response.status(), 200);
}
