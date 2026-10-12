use std::fs;

use axum::{
    Router,
    body::Body,
    extract::Query,
    http::{Request, StatusCode},
    routing::get,
};
use openlist_ani_rs::{AppState, router};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

const GROUPS_HTML: &str = r##"
    <a class="subgroup-name" data-anchor="#583">字幕组&amp;伙伴</a>
    <div id="583"></div>
    <div class="episode-table"><table><tbody>
        <tr><td>Episode 1</td></tr>
        <tr><td>Episode 2</td></tr>
    </tbody></table></div>
"##;

#[derive(Deserialize)]
struct SearchQuery {
    searchstr: String,
}

async fn post(app: Router, path: &str, body: Value) -> (StatusCode, Value) {
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
    let bytes = axum::body::to_bytes(response.into_body(), 32 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn mikan_search_and_groups_use_custom_base_url_and_keep_response_contract() {
    let external = Router::new()
        .route(
            "/Home/Search",
            get(|Query(query): Query<SearchQuery>| async move {
                assert_eq!(query.searchstr, "葬送の芙莉蓮");
                "<a href=\"/Home/Bangumi/4064\">葬送の芙莉蓮</a>"
            }),
        )
        .route("/Home/Bangumi/4064", get(|| async { GROUPS_HTML }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, external).await.unwrap();
    });

    let directory = tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    fs::write(
        &config_path,
        include_str!("../fixtures/config.synthetic.toml"),
    )
    .unwrap();
    let app = router(
        AppState::new(
            directory.path().join("data.db"),
            directory.path().join("tasks.db"),
        )
        .with_config_file(config_path),
    );
    let base_url = format!("http://{address}/");

    let (status, body) = post(
        app.clone(),
        "/api/ui/mikan/search",
        json!({"keyword": "葬送の芙莉蓮", "base_url": base_url}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["success"], true);
    assert_eq!(body["results"][0]["bangumi_id"], 4064);
    assert_eq!(body["results"][0]["name"], "葬送の芙莉蓮");

    let (status, body) = post(
        app.clone(),
        "/api/ui/mikan/groups",
        json!({"bangumi_id": 4064, "base_url": format!("http://{address}")}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["bangumi_id"], 4064);
    assert_eq!(
        body["all_rss_url"],
        format!("http://{address}/RSS/Bangumi?bangumiId=4064")
    );
    assert_eq!(body["groups"][0]["id"], 583);
    assert_eq!(body["groups"][0]["name"], "字幕组&伙伴");
    assert_eq!(body["groups"][0]["release_count"], 2);
    assert_eq!(
        body["groups"][0]["rss_url"],
        format!("http://{address}/RSS/Bangumi?bangumiId=4064&subgroupid=583")
    );
}

#[tokio::test]
async fn mikan_rss_uses_configured_site_and_rejects_invalid_ids_or_schemes() {
    let directory = tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    fs::write(
        &config_path,
        include_str!("../fixtures/config.synthetic.toml"),
    )
    .unwrap();
    let app = router(
        AppState::new(
            directory.path().join("data.db"),
            directory.path().join("tasks.db"),
        )
        .with_config_file(config_path),
    );

    let (status, body) = post(
        app.clone(),
        "/api/ui/mikan/rss",
        json!({"bangumi_id": 4064, "subgroup_id": 583}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["rss_url"],
        "http://127.0.0.1:19002/RSS/Bangumi?bangumiId=4064&subgroupid=583"
    );

    let (status, _) = post(app.clone(), "/api/ui/mikan/rss", json!({"bangumi_id": 0})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, _) = post(
        app,
        "/api/ui/mikan/rss",
        json!({"bangumi_id": 4064, "base_url": "file:///etc"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
