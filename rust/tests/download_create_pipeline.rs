use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, Request},
    routing::{get, post},
};
use openlist_ani_rs::{AppState, router};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use tokio::time::{Duration, Instant, sleep};
use tower::ServiceExt;

#[derive(Clone, Default)]
struct OpenListStub(Arc<Mutex<OpenListState>>);

#[derive(Default)]
struct OpenListState {
    temporary_path: Option<String>,
    source_filename: Option<String>,
    target_directory: Option<String>,
    target_filename: Option<String>,
    download_requests: usize,
    renamed: bool,
    moved: bool,
    cleaned: bool,
    notified: bool,
    stale_destination: bool,
}

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
            );",
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_download_runs_openlist_materialization_and_resource_commit() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stub = OpenListStub::default();
    let app =
        Router::new()
            .route(
                "/trigger",
                post(
                    |State(stub): State<OpenListStub>, Json(body): Json<Value>| async move {
                        assert_eq!(body["event"], "download.completed");
                        assert_eq!(
                            body["path"],
                            "/library/Example Show/Season 1/Example Show - S01E02.mkv"
                        );
                        stub.0.lock().unwrap().notified = true;
                        Json(json!({"ok": true}))
                    },
                ),
            )
            .route(
                "/api/fs/add_offline_download",
                post(
                    |State(stub): State<OpenListStub>,
                     headers: HeaderMap,
                     Json(body): Json<Value>| async move {
                        assert_eq!(headers.get("authorization").unwrap(), "test-token");
                        assert_eq!(body["urls"], json!(["magnet:?xt=urn:btih:abc"]));
                        assert!(
                            body["path"]
                                .as_str()
                                .unwrap()
                                .starts_with("/library/.oani-download-tmp/")
                        );
                        assert_eq!(body["tool"], "qBittorrent");
                        assert_eq!(body["name"], "Example Show S01E02");
                        let mut state = stub.0.lock().unwrap();
                        state.temporary_path = body["path"].as_str().map(str::to_string);
                        state.source_filename = Some("payload.mkv".to_string());
                        state.download_requests += 1;
                        Json(json!({
                            "code": 200,
                            "data": {"tasks": [{
                                "id": "remote-1",
                                "name": "Example Show S01E02",
                                "state": 1
                            }]}
                        }))
                    },
                ),
            )
            .route(
                "/api/task/offline_download/undone",
                get(|| async { Json(json!({"code": 200, "data": []})) }),
            )
            .route(
                "/api/task/offline_download/done",
                get(|| async {
                    Json(json!({"code": 200, "data": [{
                        "id": "remote-1",
                        "name": "Example Show S01E02",
                        "state": 2
                    }]}))
                }),
            )
            .route(
                "/api/task/offline_download_transfer/undone",
                get(|| async { Json(json!({"code": 200, "data": []})) }),
            )
            .route(
                "/api/task/offline_download_transfer/done",
                get(|| async {
                    Json(json!({"code": 200, "data": [{
                        "id": "transfer-1",
                        "name": "remote-1",
                        "state": 2
                    }]}))
                }),
            )
            .route(
                "/api/fs/mkdir",
                post(|Json(_body): Json<Value>| async { Json(json!({"code": 200})) }),
            )
            .route(
                "/api/fs/list",
                post(
                    |State(stub): State<OpenListStub>, Json(body): Json<Value>| async move {
                        let path = body["path"].as_str().unwrap();
                        let mut state = stub.0.lock().unwrap();
                        let content = if state.target_directory.as_deref() == Some(path) {
                            if state.stale_destination {
                                state.stale_destination = false;
                                json!(null)
                            } else {
                                state
                                .target_filename
                                .as_ref()
                                .map(|name| json!([{"name": name, "is_dir": false, "size": 4096}]))
                                .unwrap_or_else(|| json!([]))
                            }
                        } else if state.temporary_path.as_deref() == Some(path) {
                            state
                                .source_filename
                                .as_ref()
                                .map(|name| json!([{"name": name, "is_dir": false, "size": 4096}]))
                                .unwrap_or_else(|| json!([]))
                        } else {
                            json!([])
                        };
                        Json(json!({"code": 200, "data": {"content": content}}))
                    },
                ),
            )
            .route(
                "/api/fs/rename",
                post(
                    |State(stub): State<OpenListStub>, Json(body): Json<Value>| async move {
                        let mut state = stub.0.lock().unwrap();
                        assert_eq!(
                            body["path"],
                            format!("{}/payload.mkv", state.temporary_path.as_deref().unwrap())
                        );
                        state.source_filename = body["name"].as_str().map(str::to_string);
                        state.renamed = true;
                        Json(json!({"code": 200}))
                    },
                ),
            )
            .route(
                "/api/fs/move",
                post(
                    |State(stub): State<OpenListStub>, Json(body): Json<Value>| async move {
                        let mut state = stub.0.lock().unwrap();
                        assert_eq!(body["src_dir"], state.temporary_path.as_deref().unwrap());
                        state.target_directory = body["dst_dir"].as_str().map(str::to_string);
                        state.target_filename = body["names"][0].as_str().map(str::to_string);
                        state.source_filename = None;
                        state.moved = true;
                        state.stale_destination = true;
                        Json(json!({"code": 200}))
                    },
                ),
            )
            .route(
                "/api/fs/remove",
                post(
                    |State(stub): State<OpenListStub>, Json(body): Json<Value>| async move {
                        assert_eq!(body["dir"], "/library/.oani-download-tmp");
                        assert_eq!(body["names"][0].as_str().unwrap().len(), 36);
                        stub.0.lock().unwrap().cleaned = true;
                        Json(json!({"code": 200}))
                    },
                ),
            )
            .with_state(stub.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let directory = tempdir().unwrap();
    std::fs::create_dir(directory.path().join("data")).unwrap();
    let data_path = directory.path().join("data/data.db");
    let tasks_path = directory.path().join("data/task_mementos.db");
    let config_path = directory.path().join("config.toml");
    create_legacy_databases(&data_path, &tasks_path);
    std::fs::write(
        &config_path,
        format!(
            "[openlist]\nurl = \"http://{address}\"\ndownload_path = \"/library\"\ntoken = \"test-token\"\noffline_download_tool = \"qBittorrent\"\nrename_format = \"{{anime_name}} - S{{season:02d}}E{{episode:02d}}\"\n[smartstrm]\nenabled = true\ntrigger_url = \"http://{address}/trigger\"\n"
        ),
    )
    .unwrap();

    let app = router(AppState::new(&data_path, &tasks_path).with_config_file(&config_path));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/downloads")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "title": "[Group] Example Show S01E02 [1080p][CHT]",
                        "download_url": "magnet:?xt=urn:btih:abc"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 16 * 1024).await.unwrap()).unwrap();
    assert_eq!(response["success"], true);
    assert_eq!(response["task"]["state"], "downloading");
    let task_id = response["task"]["id"].as_str().unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut operations = Vec::new();
    while Instant::now() < deadline {
        operations = openlist_ani_rs::storage::LegacyTaskMementoStore::new(&tasks_path)
            .load_submission_operations()
            .unwrap();
        if operations
            .iter()
            .any(|operation| operation.state == "completed")
            && stub.0.lock().unwrap().notified
        {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].task_id, task_id);
    assert_eq!(operations[0].state, "completed");

    let stub = stub.0.lock().unwrap();
    assert_eq!(stub.download_requests, 1);
    assert!(stub.renamed && stub.moved && stub.cleaned && stub.notified);
    assert_eq!(
        stub.target_directory.as_deref(),
        Some("/library/Example Show/Season 1")
    );
    assert_eq!(
        stub.target_filename.as_deref(),
        Some("Example Show - S01E02.mkv")
    );
    drop(stub);

    let resource = openlist_ani_rs::storage::LegacyResourceStore::new(&data_path)
        .find_by_title("[Group] Example Show S01E02 [1080p][CHT]")
        .unwrap()
        .unwrap();
    assert_eq!(resource.url, "magnet:?xt=urn:btih:abc");
    assert_eq!(resource.anime_name.as_deref(), Some("Example Show"));
    assert_eq!((resource.season, resource.episode), (Some(1), Some(2)));
    assert_eq!(resource.fansub.as_deref(), Some("Group"));
    assert_eq!(resource.quality.as_deref(), Some("1080p"));
    assert_eq!(resource.languages.as_deref(), Some("繁"));
    let task_store = openlist_ani_rs::storage::LegacyTaskMementoStore::new(&tasks_path);
    assert!(
        task_store
            .mark_submission_completed(task_id, "2026-10-12T00:00:00Z")
            .unwrap()
    );
    assert!(task_store.load_all().unwrap().is_empty());
}
