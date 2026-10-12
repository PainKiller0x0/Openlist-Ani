use std::net::SocketAddr;

use axum::{body::Body, extract::ConnectInfo, http::Request};
use openlist_ani_rs::{AppState, auth::AuthSettings, router};
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

fn fixture_auth() -> (AuthSettings, Value) {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/auth.synthetic.json")).unwrap();
    let auth = AuthSettings::new(
        fixture["username"].as_str().unwrap(),
        fixture["passwordHash"].as_str().unwrap(),
        Some(fixture["secret"].as_str().unwrap().to_string()),
    )
    .unwrap();
    (auth, fixture)
}

#[test]
fn legacy_password_hash_and_stateless_cookie_match_python_fixture() {
    let (auth, fixture) = fixture_auth();
    let now = fixture["frozenNowUnix"].as_u64().unwrap();
    let token = fixture["legacyToken"].as_str().unwrap();

    assert!(auth.verify_password(fixture["password"].as_str().unwrap()));
    assert!(!auth.verify_password("incorrect-password"));
    assert_eq!(auth.session_username_at(token, now), Some("synthetic-user"));
    assert!(
        auth.session_username_at(token, fixture["expiresAt"].as_u64().unwrap())
            .is_none()
    );
    assert!(
        auth.session_username_at(&format!("{token}x"), now)
            .is_none()
    );
}

#[tokio::test]
async fn auth_http_contract_issues_secure_cookie_and_accepts_it() {
    let (auth, fixture) = fixture_auth();
    let directory = tempdir().unwrap();
    let app = router(
        AppState::new(
            directory.path().join("unused-data.db"),
            directory.path().join("unused-tasks.db"),
        )
        .with_auth(auth),
    );

    let session = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/auth/session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(session.status(), 200);
    let session_body: Value = serde_json::from_slice(
        &axum::body::to_bytes(session.into_body(), 4096)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        session_body,
        json!({"authenticated": false, "username": null})
    );

    let bad_login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "username": fixture["username"],
                        "password": "wrong"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_login.status(), 401);

    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("https://127.0.0.1/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "username": fixture["username"],
                        "password": fixture["password"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), 200);
    let set_cookie = login.headers().get("set-cookie").unwrap().to_str().unwrap();
    assert!(set_cookie.contains("op_ani_session="));
    assert!(set_cookie.contains("Max-Age=2592000"));
    assert!(set_cookie.contains("Path=/"));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Lax"));
    assert!(set_cookie.contains("Secure"));
    let cookie = set_cookie.split(';').next().unwrap();

    let authenticated = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/auth/session")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let authenticated_body: Value = serde_json::from_slice(
        &axum::body::to_bytes(authenticated.into_body(), 4096)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        authenticated_body,
        json!({"authenticated": true, "username": fixture["username"]})
    );

    let logout = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout.status(), 200);
    assert!(
        logout
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
}

#[tokio::test]
async fn forwarded_https_is_trusted_only_from_loopback_proxy() {
    let (auth, fixture) = fixture_auth();
    let directory = tempdir().unwrap();
    let app = router(
        AppState::new(
            directory.path().join("unused-data.db"),
            directory.path().join("unused-tasks.db"),
        )
        .with_auth(auth),
    );

    for (loopback_peer, expected_secure) in [(false, false), (true, true)] {
        let body = json!({
            "username": fixture["username"],
            "password": fixture["password"]
        })
        .to_string();
        let mut request = Request::builder()
            .method("POST")
            .uri("http://127.0.0.1/api/auth/login")
            .header("content-type", "application/json")
            .header("x-forwarded-proto", "https")
            .body(Body::from(body))
            .unwrap();
        if loopback_peer {
            request
                .extensions_mut()
                .insert(ConnectInfo("127.0.0.1:3000".parse::<SocketAddr>().unwrap()));
        }
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), 200);
        let set_cookie = response
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(set_cookie.contains("; Secure"), expected_secure);
    }
}

#[tokio::test]
async fn private_api_requires_a_valid_session_when_auth_is_configured() {
    let (auth, _) = fixture_auth();
    let directory = tempdir().unwrap();
    let app = router(
        AppState::new(
            directory.path().join("unused-data.db"),
            directory.path().join("unused-tasks.db"),
        )
        .with_auth(auth),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/ui/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 401);
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        json!({"detail": "请先登录 op-ani"})
    );
}
