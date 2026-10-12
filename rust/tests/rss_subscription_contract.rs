use axum::{Router, body::Body, http::Request, routing::get};
use openlist_ani_rs::{AppState, router};
use serde_json::{Value, json};
use std::fs;
use tempfile::tempdir;
use tower::ServiceExt;

const CONFIG: &str = r#"# preserve this comment
[backend]
host = "127.0.0.1"
port = 19004

[openlist]
download_path = "/迅雷/videos/番"
rename_format = "{anime_name} - S{season:02d}E{episode:02d}"

[rss.filter]
exclude_patterns = ["(?i)1080p"]
"#;

async fn rss_server() -> String {
    const FEED: &str = r#"<?xml version="1.0"?><rss><channel>
        <item><title>[Group] Original Show - 03 [1080P]</title><enclosure url="https://files.test/03.torrent" type="application/x-bittorrent"/></item>
        <item><title>[Group] Original Show - 04 [720P]</title><enclosure url="https://files.test/04.torrent" type="application/x-bittorrent"/></item>
        <item><title>[Group] Original Show - 05 [480P]</title><enclosure url="https://files.test/05.torrent" type="application/x-bittorrent"/></item>
    </channel></rss>"#;
    let source = Router::new().route("/feed", get(|| async { FEED }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, source).await.unwrap();
    });
    format!("http://{address}/feed")
}

async fn post(app: Router, path: &str, body: Value) -> (axum::http::StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

async fn delete(app: Router, path: &str, body: Value) -> axum::http::StatusCode {
    app.oneshot(
        Request::builder()
            .method("DELETE")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
    .status()
}

fn app(directory: &std::path::Path) -> Router {
    router(
        AppState::new(
            directory.join("data.db"),
            directory.join("task_mementos.db"),
        )
        .with_config_file(directory.join("config.toml")),
    )
}

#[tokio::test]
async fn preview_applies_regex_offset_and_naming_without_persisting() {
    let directory = tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    fs::write(&config_path, CONFIG).unwrap();
    let original = fs::read(&config_path).unwrap();
    let url = rss_server().await;

    let (status, body) = post(
        app(directory.path()),
        "/api/ui/rss/preview",
        json!({
            "url": url,
            "name": "Original Show",
            "anime_name": "套马的汉子",
            "download_directory_name": "JOJO的奇妙冒险 (2012)",
            "episode_offset": 2,
            "filename_season_override": 1,
            "directory_season_override": 6,
            "exclude_patterns": "(?=.*720P).+"
        }),
    )
    .await;

    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert_eq!(body["success"], true);
    assert_eq!(body["total"], 3);
    assert_eq!(body["included"], 1, "{body}");
    assert_eq!(body["excluded"], 2);
    assert_eq!(body["entries"][0]["excluded"], true);
    assert_eq!(body["entries"][2]["episode"], 3);
    assert_eq!(body["entries"][2]["rename_preview"], "套马的汉子 - S01E03");
    assert_eq!(
        body["entries"][2]["download_directory"],
        "/迅雷/videos/番/JOJO的奇妙冒险 (2012)/Season 6"
    );
    assert_eq!(fs::read(config_path).unwrap(), original);
}

#[tokio::test]
async fn unconfirmed_add_does_not_write_and_confirmed_add_persists_once() {
    let directory = tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    fs::write(&config_path, CONFIG).unwrap();
    let original = fs::read(&config_path).unwrap();
    let url = rss_server().await;
    let request = json!({
        "url": url,
        "name": "Original Show",
        "anime_name": "套马的汉子",
        "download_directory_name": "JOJO的奇妙冒险 (2012)",
        "episode_offset": 2,
        "filename_season_override": 1,
        "directory_season_override": 6,
        "tmdb_id": 283938,
        "season": 8,
        "exclude_patterns": "(?=.*720P).+"
    });

    let (status, _) = post(app(directory.path()), "/api/ui/rss", request.clone()).await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT);
    assert_eq!(fs::read(&config_path).unwrap(), original);

    let mut confirmed = request;
    confirmed["confirmed"] = json!(true);
    let (status, body) = post(app(directory.path()), "/api/ui/rss", confirmed.clone()).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert_eq!(body["success"], true);
    assert_eq!(body["scan_scheduled"], true);
    let saved = fs::read_to_string(&config_path).unwrap();
    assert!(saved.contains("# preserve this comment"));
    assert!(saved.contains("url = \"http://"));
    assert!(saved.contains("anime_name = \"套马的汉子\""));
    assert!(saved.contains("tmdb_id = 283938"));
    assert!(saved.contains("season = 8"));

    let (status, duplicate) = post(app(directory.path()), "/api/ui/rss", confirmed).await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "{duplicate}");
    assert_eq!(fs::read_to_string(config_path).unwrap(), saved);
}

#[tokio::test]
async fn confirmed_add_rejects_unreadable_feed_without_persisting() {
    let directory = tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    fs::write(&config_path, CONFIG).unwrap();
    let original = fs::read(&config_path).unwrap();

    let (status, _) = post(
        app(directory.path()),
        "/api/ui/rss",
        json!({"url": "http://127.0.0.1:1/feed", "confirmed": true}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(fs::read(config_path).unwrap(), original);
}

#[tokio::test]
async fn subscription_management_routes_update_and_remove_the_persisted_rule() {
    let directory = tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    let original_url = "https://old.example.test/feed";
    fs::write(
        &config_path,
        format!(
            "{CONFIG}\n[[rss.subscriptions]]\nurl = \"{original_url}\"\nname = \"Original Show\"\nanime_name = \"Original Show\"\nseason = 1\nenabled = true\ncompleted = false\nexclude_patterns = []\n"
        ),
    )
    .unwrap();
    let app = app(directory.path());

    let (status, body) = post(
        app.clone(),
        "/api/ui/rss/toggle",
        json!({"url": original_url, "enabled": false}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert!(body["urls"].as_array().unwrap().is_empty());

    let (status, body) = post(
        app.clone(),
        "/api/ui/rss/completed",
        json!({"url": original_url, "completed": true}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");

    let (status, body) = post(
        app.clone(),
        "/api/ui/rss/completed",
        json!({"url": original_url, "completed": false}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert_eq!(body["urls"], json!([original_url]));

    let (status, body) = post(
        app.clone(),
        "/api/ui/rss/exclude",
        json!({"url": original_url, "exclude_patterns": "promo|(?=.*BD).+"}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert_eq!(body["exclude_patterns"], json!(["promo", "(?=.*BD).+"]));

    let (status, body) = post(
        app.clone(),
        "/api/ui/rss/filter",
        json!({"exclude_patterns": "(?i)1080p|广告"}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");

    let replacement_url = rss_server().await;
    let (status, body) = post(
        app.clone(),
        "/api/ui/rss/correct",
        json!({
            "original_url": original_url,
            "url": replacement_url,
            "name": "Corrected Show",
            "anime_name": "Corrected Show",
            "season": 2,
            "exclude_patterns": "promo"
        }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert_eq!(body["urls"], json!([replacement_url]));

    let saved = fs::read_to_string(&config_path).unwrap();
    assert!(saved.contains(&format!("url = \"{replacement_url}\"")));
    assert!(saved.contains("name = \"Corrected Show\""));
    assert!(saved.contains("season = 2"));
    assert!(saved.contains("exclude_patterns = [\"promo\"]"));
    assert!(saved.contains("exclude_patterns = [\"(?i)1080p\", \"广告\"]"));

    let (status, body) = post(
        app.clone(),
        "/api/ui/rss/remove",
        json!({"url": replacement_url}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert!(body["urls"].as_array().unwrap().is_empty());
    assert_eq!(
        delete(app, "/api/ui/rss", json!({"url": replacement_url})).await,
        axum::http::StatusCode::NOT_FOUND
    );
}
