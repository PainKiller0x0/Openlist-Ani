use axum::{Router, body::Body, http::Request, routing::get};
use openlist_ani_rs::{AppState, router};
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn resolve_torrent_http_parses_fixture_without_fetching_media() {
    let source = Router::new().route(
        "/fixture.torrent",
        get(|| async {
            Body::from(include_bytes!("../fixtures/torrent-metadata-seed-v1.torrent").to_vec())
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, source).await.unwrap();
    });

    let directory = tempdir().unwrap();
    let app = router(AppState::new(
        directory.path().join("unused-data.db"),
        directory.path().join("unused-tasks.db"),
    ));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/resolve_torrent")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"url": format!("http://{address}/fixture.torrent")}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["source"], "torrent_file");
    assert_eq!(body["title"], "synthetic-nonzero.mp4");
    assert_eq!(body["file_count"], 1);
    assert_eq!(body["files"][0]["name"], "synthetic-nonzero.mp4");
    assert_eq!(body["files"][0]["size"], 131_072);
}

#[tokio::test]
async fn resolve_torrent_http_does_not_invent_a_title_for_invalid_metadata() {
    let source = Router::new().route("/bad.torrent", get(|| async { "not a torrent" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, source).await.unwrap();
    });

    let directory = tempdir().unwrap();
    let app = router(AppState::new(
        directory.path().join("unused-data.db"),
        directory.path().join("unused-tasks.db"),
    ));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/resolve_torrent")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"url": format!("http://{address}/bad.torrent")}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["success"], false);
    assert_eq!(body["title"], Value::Null);
    assert_eq!(body["files"], json!([]));
}
