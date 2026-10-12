use axum::{body::Body, http::Request};
use openlist_ani_rs::{AppState, router};
use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

fn create_task_database(path: &std::path::Path) {
    let connection = Connection::open(path).unwrap();
    connection
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
    let payload = json!({
        "schema_version": 1,
        "state": "downloading",
        "release": {
            "title": "Example episode",
            "download_url": "https://example.test/download",
            "anime_name": "Example show",
            "season": 2,
            "episode": 7,
            "fansub": "Group",
            "quality": "1080p"
        },
        "base_path": "/media/anime",
        "downloader": {"payload": {"progress": 42.5}},
        "retry": {"retry_count": 2, "last_error": "temporary failure"},
        "created_at": "2026-10-12T10:00:00",
        "updated_at": "2026-10-12T10:01:00",
        "started_at": "2026-10-12T10:00:10",
        "completed_at": null,
        "output_path": "/media/anime/Example - S02E07.mkv",
        "archived": false
    })
    .to_string();
    connection
        .execute(
            "INSERT INTO task_mementos (task_id, state, updated_at, payload) VALUES (?1, ?2, ?3, ?4)",
            ("task-7", "downloading", "2026-10-12T10:01:00", payload),
        )
        .unwrap();
    let archived = json!({"archived": true}).to_string();
    connection
        .execute(
            "INSERT INTO task_mementos (task_id, state, updated_at, payload) VALUES (?1, ?2, ?3, ?4)",
            ("task-archived", "failed", "2026-10-12T10:02:00", archived),
        )
        .unwrap();
}

#[tokio::test]
async fn downloads_api_reads_legacy_mementos_and_hides_archived_tasks() {
    let directory = tempdir().unwrap();
    let tasks_path = directory.path().join("task_mementos.db");
    create_task_database(&tasks_path);
    let app = router(AppState::new(
        directory.path().join("unused-data.db"),
        &tasks_path,
    ));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/downloads")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .unwrap();
    let list: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(list["total"], 1);
    assert_eq!(
        list["tasks"][0],
        json!({
            "id": "task-7",
            "title": "Example episode",
            "download_url": "https://example.test/download",
            "state": "downloading",
            "anime_name": "Example show",
            "season": 2,
            "episode": 7,
            "fansub": "Group",
            "quality": "1080p",
            "progress": 42.5,
            "error_message": "temporary failure",
            "retry_count": 2,
            "created_at": "2026-10-12T10:00:00",
            "updated_at": "2026-10-12T10:01:00",
            "started_at": "2026-10-12T10:00:10",
            "completed_at": null,
            "save_path": "/media/anime",
            "final_path": "/media/anime/Example - S02E07.mkv"
        })
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/downloads/task-7")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let missing = app
        .oneshot(
            Request::builder()
                .uri("/api/downloads/not-found")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}
