use std::{
    collections::HashMap,
    fs,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect},
    routing::{get, post},
};
use openlist_ani_rs::release_parser;
use serde_json::{Value, json};
use tempfile::TempDir;

// The integration owner registers the module in lib.rs; this test does not edit it.
#[path = "../src/metadata.rs"]
mod metadata;
use metadata::{MetadataError, resolve_rss_metadata};

#[derive(Clone)]
struct Stub {
    results: Value,
    details: Value,
    llm: Value,
    xml: String,
    html: String,
    detail_redirect: Option<String>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Default for Stub {
    fn default() -> Self {
        Self {
            results: json!([
                {"id": 90, "name": "Unrelated Popular Show", "genre_ids": [16]},
                {"id": 91, "name": "Exact Show", "genre_ids": [18]},
                {"id": 42, "name": "Exact Show", "original_name": "Original Show", "genre_ids": [16]}
            ]),
            details: json!({"id": 42, "name": "Localized Show", "first_air_date": "2024-01-02", "poster_path": "/show.jpg"}),
            llm: json!([{"status": "success", "anime_name": "Exact Show", "season": 2, "tmdb_id": 99999}]),
            xml: "<rss><channel><title>Feed</title><item><title>[Group] Exact Show S02E03 [1080p]</title><enclosure url=\"magnet:?xt=urn:btih:test\"/></item></channel></rss>".into(),
            html: "<p class=\"bangumi-title\"><a class=\"w-other-c\">Exact Show</a></p>".into(),
            detail_redirect: None,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

async fn search(
    State(stub): State<Stub>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Value> {
    assert_eq!(
        query.get("api_key").map(String::as_str),
        Some("local-tmdb-secret")
    );
    assert_eq!(query.get("language").map(String::as_str), Some("ja-JP"));
    stub.requests
        .lock()
        .unwrap()
        .push(format!("search:{}", query["query"]));
    Json(json!({"results": stub.results}))
}

async fn details(State(stub): State<Stub>, Path(id): Path<i64>) -> (StatusCode, Json<Value>) {
    stub.requests.lock().unwrap().push(format!("details:{id}"));
    (
        if stub.details.is_null() {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::OK
        },
        Json(stub.details),
    )
}

async fn anthropic(
    State(stub): State<Stub>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    assert_eq!(headers["x-api-key"], "local-llm-secret");
    assert_eq!(headers["anthropic-version"], "2023-06-01");
    assert_eq!(body["model"], "local-model");
    assert!(body["system"].is_string());
    stub.requests.lock().unwrap().push("anthropic".into());
    Json(
        json!({"content": [{"type": "thinking", "thinking": "not output"}, {"type": "text", "text": stub.llm.to_string()}]}),
    )
}

async fn llm(State(stub): State<Stub>, headers: HeaderMap, Json(body): Json<Value>) -> Json<Value> {
    assert_eq!(headers["authorization"], "Bearer local-llm-secret");
    assert_eq!(body["model"], "local-model");
    assert!(
        body["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("Exact Show")
    );
    stub.requests.lock().unwrap().push("llm".into());
    Json(json!({"choices": [{"message": {"content": format!("```json\n{}\n```", stub.llm)}}]}))
}

async fn harness(stub: Stub) -> (TempDir, String, Arc<Mutex<Vec<String>>>) {
    let requests = Arc::clone(&stub.requests);
    let external = Router::new()
        .route(
            "/feed",
            get(|State(stub): State<Stub>| async move { stub.xml }),
        )
        .route(
            "/RSS/Bangumi",
            get(|State(stub): State<Stub>| async move { stub.xml }),
        )
        .route(
            "/Home/Bangumi/7",
            get(|State(stub): State<Stub>| async move {
                if let Some(target) = stub.detail_redirect {
                    Redirect::temporary(&target).into_response()
                } else {
                    stub.html.into_response()
                }
            }),
        )
        .route("/3/search/tv", get(search))
        .route("/3/tv/{id}", get(details))
        .route("/v1/chat/completions", post(llm))
        .route("/v1/messages", post(anthropic))
        .with_state(stub);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, external).await.unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("config.toml"),
        format!(
            r#"
[llm]
tmdb_api_key = "local-tmdb-secret"
tmdb_base_url = "{base}/3"
tmdb_language = "ja-JP"
openai_api_key = "local-llm-secret"
openai_base_url = "{base}/v1"
openai_model = "local-model"
[metadata_parser]
provider = "regex"
[mikan]
base_url = "{base}/"
"#
        ),
    )
    .unwrap();
    (directory, base, requests)
}

#[tokio::test]
async fn selects_unique_animation_match_not_first_result_and_keeps_contract() {
    let (directory, base, requests) = harness(Stub::default()).await;
    let config = directory.path().join("config.toml");
    let before = fs::read(&config).unwrap();
    let result = resolve_rss_metadata(&config, &format!("{base}/feed"), "", None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        json!({
            "name": "Localized Show (2024)", "anime_name": "Localized Show (2024)",
            "season": 2, "year": 2024, "tmdb_id": 42,
            "poster_url": "https://image.tmdb.org/t/p/w500/show.jpg"
        })
    );
    assert_eq!(fs::read(&config).unwrap(), before);
    assert_eq!(
        *requests.lock().unwrap(),
        ["search:Exact Show", "details:42"]
    );
}

#[tokio::test]
async fn rejects_unrelated_results_and_same_name_remake_ambiguity() {
    for (results, expected) in [
        (
            json!([{"id": 8, "name": "Exact Show Extra", "genre_ids": [16]}]),
            MetadataError::Unresolved,
        ),
        (
            json!([{"id": 8, "name": "Exact Show", "genre_ids": [18]}]),
            MetadataError::Unresolved,
        ),
        (
            json!([
                {"id": 8, "name": "Exact Show", "genre_ids": [16]},
                {"id": 9, "original_name": "Exact Show", "genre_ids": [16]}
            ]),
            MetadataError::Ambiguous,
        ),
    ] {
        let (directory, base, _) = harness(Stub {
            results,
            ..Stub::default()
        })
        .await;
        assert_eq!(
            resolve_rss_metadata(
                directory.path().join("config.toml"),
                &format!("{base}/feed"),
                "",
                None
            )
            .await
            .unwrap_err(),
            expected
        );
    }
}

#[tokio::test]
async fn explicit_id_bypasses_feed_and_search_but_preserves_subscription_overrides() {
    let (directory, base, requests) = harness(Stub::default()).await;
    let config = directory.path().join("config.toml");
    let url = format!("{base}/unavailable-feed");
    let mut content = fs::read_to_string(&config).unwrap();
    content.push_str(&format!(
        r#"
[[rss.subscriptions]]
url = "{url}"
name = "Saved Display"
anime_name = "Custom Filename Name"
season = 8
filename_season_override = 1
directory_season_override = 6
tmdb_id = 9
enabled = false
completed = true
"#
    ));
    fs::write(&config, content).unwrap();
    let result = resolve_rss_metadata(&config, &url, "New Display", Some(42))
        .await
        .unwrap();
    assert_eq!(result.name, "New Display");
    assert_eq!(result.anime_name, "Custom Filename Name");
    assert_eq!(result.season, Some(8));
    assert_eq!(result.tmdb_id, Some(42));
    assert_eq!(*requests.lock().unwrap(), ["details:42"]);
}

#[tokio::test]
async fn saved_id_and_special_season_are_not_replaced_by_inference() {
    let (directory, base, requests) = harness(Stub::default()).await;
    let config = directory.path().join("config.toml");
    let url = format!("{base}/missing");
    let mut content = fs::read_to_string(&config).unwrap();
    content.push_str(&format!("\n[[rss.subscriptions]]\nurl = \"{url}\"\nname = \"Saved Display\"\nseason = 0\ntmdb_id = 42\n"));
    fs::write(&config, content).unwrap();
    let result = resolve_rss_metadata(&config, &url, "", None).await.unwrap();
    assert_eq!(result.name, "Saved Display");
    assert_eq!(result.season, Some(0));
    assert_eq!(*requests.lock().unwrap(), ["details:42"]);
}

#[tokio::test]
async fn llm_uses_configured_endpoint_but_cannot_supply_authoritative_id() {
    let (directory, base, requests) = harness(Stub::default()).await;
    let config = directory.path().join("config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("provider = \"regex\"", "provider = \"llm\""),
    )
    .unwrap();
    let result = resolve_rss_metadata(&config, &format!("{base}/feed"), "", None)
        .await
        .unwrap();
    assert_eq!(result.tmdb_id, Some(42));
    assert!(requests.lock().unwrap().contains(&"llm".into()));
    assert!(!requests.lock().unwrap().contains(&"details:99999".into()));
}

#[tokio::test]
async fn configured_mikan_detail_provides_trusted_alias_not_unrelated_html() {
    let stub = Stub {
        xml: "<rss><channel><item><title>[Group] Unmatched Show - 01</title></item></channel></rss>".into(),
        html: "<a>Unrelated Popular Show</a><p class='bangumi-title'><a class='w-other-c'>Exact Show</a></p>".into(),
        ..Stub::default()
    };
    let (directory, base, requests) = harness(stub).await;
    let result = resolve_rss_metadata(
        directory.path().join("config.toml"),
        &format!("{base}/RSS/Bangumi?bangumiId=7"),
        "",
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.tmdb_id, Some(42));
    assert!(
        requests
            .lock()
            .unwrap()
            .contains(&"search:Exact Show".into())
    );
    assert!(
        !requests
            .lock()
            .unwrap()
            .contains(&"search:Unrelated Popular Show".into())
    );
}

#[tokio::test]
async fn malformed_config_network_failures_and_id_errors_do_not_expose_keys() {
    let (directory, base, _) = harness(Stub::default()).await;
    let config = directory.path().join("config.toml");
    let error = resolve_rss_metadata(&config, &format!("{base}/missing"), "", None)
        .await
        .unwrap_err();
    assert_eq!(error, MetadataError::Request);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("local-tmdb-secret"));
    assert!(!rendered.contains(&base));
    assert_eq!(
        resolve_rss_metadata(&config, &format!("{base}/feed"), "", Some(0))
            .await
            .unwrap_err(),
        MetadataError::InvalidId
    );
    fs::write(&config, "[llm]\nopenai_api_key = \"private-unclosed").unwrap();
    let error = resolve_rss_metadata(&config, &format!("{base}/feed"), "", None)
        .await
        .unwrap_err();
    assert_eq!(error, MetadataError::Config);
    assert!(!format!("{error:?} {error}").contains("private-unclosed"));
}

#[tokio::test]
async fn details_must_confirm_id_and_empty_poster_or_date_stays_empty() {
    let (directory, base, _) = harness(Stub {
        details: json!({"id": 42, "name": "Exact Show", "first_air_date": "", "poster_path": null}),
        ..Stub::default()
    })
    .await;
    let result = resolve_rss_metadata(
        directory.path().join("config.toml"),
        &format!("{base}/feed"),
        "",
        Some(42),
    )
    .await
    .unwrap();
    assert_eq!(result.year, None);
    assert!(result.poster_url.is_empty());
    assert_eq!(result.name, "Exact Show");
    let (directory, base, _) = harness(Stub {
        details: json!({"id": 7, "name": "Wrong ID"}),
        ..Stub::default()
    })
    .await;
    assert_eq!(
        resolve_rss_metadata(
            directory.path().join("config.toml"),
            &format!("{base}/feed"),
            "",
            Some(42)
        )
        .await
        .unwrap_err(),
        MetadataError::InvalidResponse
    );
}

#[tokio::test]
async fn atom_namespaces_and_mikan_chinese_season_are_recognized() {
    let atom = Stub {
        xml: "<feed xmlns='http://www.w3.org/2005/Atom'><title>Not The Series</title><entry><title>[Group] Exact Show S03E02</title></entry></feed>".into(),
        ..Stub::default()
    };
    let (directory, base, _) = harness(atom).await;
    let result = resolve_rss_metadata(
        directory.path().join("config.toml"),
        &format!("{base}/feed"),
        "",
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.season, Some(3));
    let mikan = Stub {
        xml: "<rss><channel><item><title>Unparseable release</title></item></channel></rss>".into(),
        html: "<h3 class='extra bangumi-title'><a class='w-other-c'>Exact Show 第二季</a></h3>"
            .into(),
        ..Stub::default()
    };
    let (directory, base, _) = harness(mikan).await;
    let result = resolve_rss_metadata(
        directory.path().join("config.toml"),
        &format!("{base}/RSS/Bangumi?bangumiId=7"),
        "",
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.season, Some(2));
}

#[tokio::test]
async fn llm_hallucinated_title_is_not_a_trusted_match() {
    let stub = Stub {
        llm: json!([{"status": "success", "anime_name": "Unrelated Popular Show", "season": 8}]),
        results: json!([{"id": 90, "name": "Unrelated Popular Show", "genre_ids": [16]}]),
        ..Stub::default()
    };
    let (directory, base, _) = harness(stub).await;
    let config = directory.path().join("config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("provider = \"regex\"", "provider = \"llm\""),
    )
    .unwrap();
    assert_eq!(
        resolve_rss_metadata(&config, &format!("{base}/feed"), "", None)
            .await
            .unwrap_err(),
        MetadataError::Unresolved
    );
}

#[tokio::test]
async fn franchise_slash_is_not_split_into_an_unrelated_series() {
    let stub = Stub {
        xml: "<rss><channel><item><title>[Group] Fate/strange Fake - 01</title></item></channel></rss>".into(),
        results: json!([{"id": 90, "name": "Fate", "genre_ids": [16]}]),
        ..Stub::default()
    };
    let (directory, base, _) = harness(stub).await;
    assert_eq!(
        resolve_rss_metadata(
            directory.path().join("config.toml"),
            &format!("{base}/feed"),
            "",
            None
        )
        .await
        .unwrap_err(),
        MetadataError::Unresolved
    );
}

#[tokio::test]
async fn anthropic_extracts_title_when_the_regex_cannot_parse_episode() {
    let stub = Stub {
        xml: "<rss><channel><item><title>[Group] Exact Show [1080p]</title></item></channel></rss>"
            .into(),
        ..Stub::default()
    };
    let (directory, base, requests) = harness(stub).await;
    let config = directory.path().join("config.toml");
    let content = fs::read_to_string(&config)
        .unwrap()
        .replace("provider = \"regex\"", "provider = \"llm\"")
        .replace("[llm]", "[llm]\nprovider_type = \"anthropic\"");
    fs::write(&config, content).unwrap();
    let result = resolve_rss_metadata(&config, &format!("{base}/feed"), "", None)
        .await
        .unwrap();
    assert_eq!(result.tmdb_id, Some(42));
    assert_eq!(result.season, Some(2));
    assert!(requests.lock().unwrap().contains(&"anthropic".into()));
}

#[tokio::test]
async fn upstream_tmdb_failure_and_missing_key_are_safe_errors() {
    let (directory, base, _) = harness(Stub {
        details: Value::Null,
        ..Stub::default()
    })
    .await;
    let config = directory.path().join("config.toml");
    let error = resolve_rss_metadata(&config, &format!("{base}/feed"), "", Some(42))
        .await
        .unwrap_err();
    assert_eq!(error, MetadataError::Request);
    let output = format!("{error:?} {error}");
    assert!(!output.contains("local-tmdb-secret"));
    assert!(!output.contains("api_key"));
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("local-tmdb-secret", ""),
    )
    .unwrap();
    assert_eq!(
        resolve_rss_metadata(&config, &format!("{base}/feed"), "", Some(42))
            .await
            .unwrap_err(),
        MetadataError::MissingTmdbKey
    );
}

#[tokio::test]
async fn visible_multilingual_aliases_match_original_name_and_ignore_nearby_class_names() {
    let stub = Stub {
        xml: "<rss><channel><item><title>Unparseable release</title></item></channel></rss>".into(),
        html: "<p class='not-bangumi-title'>Unrelated Popular Show</p><p class='bangumi-title'><a class='w-other-c'>Other Title / Original Show 第二季</a></p>".into(),
        ..Stub::default()
    };
    let (directory, base, requests) = harness(stub).await;
    let result = resolve_rss_metadata(
        directory.path().join("config.toml"),
        &format!("{base}/RSS/Bangumi?bangumiId=7"),
        "",
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.tmdb_id, Some(42));
    assert_eq!(result.season, Some(2));
    assert!(
        !requests
            .lock()
            .unwrap()
            .contains(&"search:Unrelated Popular Show".into())
    );
}

#[tokio::test]
async fn mikan_redirect_to_another_origin_is_not_a_trusted_alias_source() {
    let foreign = Router::new().route(
        "/page",
        get(|| async { "<p class='bangumi-title'>Exact Show</p>" }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = format!("http://{}/page", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, foreign).await.unwrap();
    });
    let stub = Stub {
        xml: "<rss><channel><item><title>Unparseable release</title></item></channel></rss>".into(),
        detail_redirect: Some(target),
        ..Stub::default()
    };
    let (directory, base, _) = harness(stub).await;
    assert_eq!(
        resolve_rss_metadata(
            directory.path().join("config.toml"),
            &format!("{base}/RSS/Bangumi?bangumiId=7"),
            "",
            None
        )
        .await
        .unwrap_err(),
        MetadataError::Unresolved
    );
}
