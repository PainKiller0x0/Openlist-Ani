use axum::{Router, body::Body, http::Request, routing::get};
use openlist_ani_rs::{AppState, router};
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn parse_rss_http_preserves_total_order_and_limit() {
    const FEED: &str = r#"<?xml version="1.0"?><rss><channel>
        <item><title>第一集</title><enclosure url="magnet:?xt=urn:btih:one" type="application/x-bittorrent"/></item>
        <item><title>无下载项</title><link>https://example.test/page</link></item>
        <item><title>第二集</title><enclosure url="https://example.test/2.torrent" type="application/octet-stream"/></item>
    </channel></rss>"#;
    let source = Router::new().route("/feed", get(|| async { FEED }));
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
                .uri("/api/parse_rss")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"url": format!("http://{address}/feed"), "limit": 1}).to_string(),
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
    assert_eq!(body["total"], 2);
    assert_eq!(body["message"], "Parsed 1 of 2 entries");
    assert_eq!(body["entries"][0]["index"], 0);
    assert_eq!(body["entries"][0]["title"], "第一集");
    assert_eq!(
        body["entries"][0]["download_url"],
        "magnet:?xt=urn:btih:one"
    );
}

#[tokio::test]
async fn parse_rss_http_returns_failure_shape_without_leaking_feed_url() {
    let directory = tempdir().unwrap();
    let app = router(AppState::new(
        directory.path().join("unused-data.db"),
        directory.path().join("unused-tasks.db"),
    ));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/parse_rss")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"url": "http://127.0.0.1:1/feed?token=secret"}).to_string(),
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
    assert_eq!(body["total"], 0);
    assert_eq!(body["entries"], json!([]));
    assert!(!body.to_string().contains("token=secret"));
}

#[tokio::test]
async fn parse_rss_http_requires_url_field() {
    let directory = tempdir().unwrap();
    let app = router(AppState::new(
        directory.path().join("unused-data.db"),
        directory.path().join("unused-tasks.db"),
    ));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/parse_rss")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 422);
}
