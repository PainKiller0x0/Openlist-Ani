use std::fs;

use axum::{body::Body, http::Request, response::Response};
use openlist_ani_rs::{AppState, auth::AuthSettings, router};
use serde_json::Value;
use tempfile::tempdir;
use tower::ServiceExt;

async fn get(app: axum::Router, path: &str, cookie: Option<&str>) -> Response {
    let mut request = Request::builder().uri(path);
    if let Some(cookie) = cookie {
        request = request.header("cookie", format!("op_ani_session={cookie}"));
    }
    app.oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

fn web_root(directory: &std::path::Path) -> std::path::PathBuf {
    let web = directory.join("web");
    fs::create_dir_all(web.join("assets/css")).unwrap();
    fs::write(web.join("index.html"), "<main>index fixture</main>").unwrap();
    fs::write(web.join("login.html"), "<main>login fixture</main>").unwrap();
    fs::write(web.join("assets/css/app.css"), "body { color: red; }").unwrap();
    web
}

fn fixture_auth() -> AuthSettings {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/auth.synthetic.json")).unwrap();
    AuthSettings::new(
        fixture["username"].as_str().unwrap(),
        fixture["passwordHash"].as_str().unwrap(),
        Some(fixture["secret"].as_str().unwrap().to_string()),
    )
    .unwrap()
}

#[tokio::test]
async fn web_pages_are_uncached_and_static_assets_are_served_without_auth() {
    let directory = tempdir().unwrap();
    let app = router(
        AppState::new(
            directory.path().join("data.db"),
            directory.path().join("tasks.db"),
        )
        .with_web_root(web_root(directory.path())),
    );

    let index = get(app.clone(), "/", None).await;
    assert_eq!(index.status(), 200);
    assert_eq!(
        index.headers()["cache-control"],
        "no-store, no-cache, must-revalidate, max-age=0"
    );
    assert_eq!(index.headers()["pragma"], "no-cache");
    assert_eq!(index.headers()["expires"], "0");
    assert_eq!(
        axum::body::to_bytes(index.into_body(), 4096).await.unwrap(),
        "<main>index fixture</main>"
    );

    let login = get(app.clone(), "/login", None).await;
    assert_eq!(login.status(), 200);
    assert!(
        login.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("no-store")
    );

    let asset = get(app, "/assets/css/app.css", None).await;
    assert_eq!(asset.status(), 200);
    assert_eq!(asset.headers()["content-type"], "text/css");
}

#[tokio::test]
async fn auth_redirects_private_pages_but_keeps_login_and_assets_public() {
    let directory = tempdir().unwrap();
    let auth = fixture_auth();
    let session = auth.issue_session();
    let app = router(
        AppState::new(
            directory.path().join("data.db"),
            directory.path().join("tasks.db"),
        )
        .with_auth(auth)
        .with_web_root(web_root(directory.path())),
    );

    let index = get(app.clone(), "/", None).await;
    assert_eq!(index.status(), 303);
    assert_eq!(index.headers()["location"], "/login");

    let login = get(app.clone(), "/login", None).await;
    assert_eq!(login.status(), 200);

    let asset = get(app.clone(), "/assets/css/app.css", None).await;
    assert_eq!(asset.status(), 200);

    let api = get(app.clone(), "/api/ui/state", None).await;
    assert_eq!(api.status(), 401);

    let authenticated = get(app, "/", Some(&session)).await;
    assert_eq!(authenticated.status(), 200);
}
