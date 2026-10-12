use reqwest::{Client, Url, header};
use serde_json::{Value, json};
use thiserror::Error;

const REQUEST_TIMEOUT_SECONDS: u64 = 30;

#[derive(Debug, Error)]
pub enum OpenListError {
    #[error("OpenList 地址必须是带 http:// 或 https:// 的完整地址")]
    InvalidBaseUrl,
    #[error("OpenList request failed")]
    Request(#[from] reqwest::Error),
}

pub struct OpenListClient {
    http: Client,
    base_url: String,
    token: String,
}

impl OpenListClient {
    pub fn new(base_url: &str, token: &str) -> Result<Self, OpenListError> {
        let base_url = normalize_base_url(base_url)?;
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .user_agent("OpenList-Ani/1.0")
            .build()?;
        Ok(Self {
            http,
            base_url,
            token: token.to_string(),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn validate_endpoint(&self, configured_tool: &str) -> bool {
        let healthy = self
            .get_json("/api/public/settings")
            .await
            .is_ok_and(|body| body.get("code").and_then(Value::as_i64) == Some(200));
        if !healthy {
            return false;
        }

        let tools = self
            .get_json("/api/public/offline_download_tools")
            .await
            .ok()
            .filter(|body| body.get("code").and_then(Value::as_i64) == Some(200))
            .and_then(|body| body.get("data").cloned());
        let Some(tools) = tools.and_then(|value| value.as_array().cloned()) else {
            return false;
        };
        tools.iter().any(|tool| {
            let name = tool
                .as_str()
                .or_else(|| tool.get("name").and_then(Value::as_str));
            name.is_some_and(|name| name.eq_ignore_ascii_case(configured_tool.trim()))
        })
    }

    pub async fn validate_path(&self, path: &str) -> (bool, String) {
        let mut normalized = path.trim().to_string();
        if !normalized.starts_with('/') {
            return (
                false,
                "下载根目录必须是 OpenList 的绝对路径，以 / 开头".to_string(),
            );
        }
        normalized = format!("/{}", normalized.trim_start_matches('/'));
        if normalized.len() > 1 {
            normalized = normalized.trim_end_matches('/').to_string();
        }
        if normalized.contains('\0') {
            return (false, "下载根目录包含非法字符".to_string());
        }

        let result = self
            .http
            .post(format!("{}/api/fs/list", self.base_url))
            .headers(self.headers())
            .json(&json!({
                "path": normalized,
                "password": "",
                "page": 1,
                "per_page": 0,
                "refresh": true
            }))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status);
        let valid = match result {
            Ok(response) => response
                .json::<Value>()
                .await
                .is_ok_and(|body| body.get("code").and_then(Value::as_i64) == Some(200)),
            Err(_) => false,
        };
        if valid {
            (true, normalized)
        } else {
            (false, format!("OpenList 无法访问目录：{normalized}"))
        }
    }

    async fn get_json(&self, path: &str) -> Result<Value, OpenListError> {
        Ok(self
            .http
            .get(format!("{}{path}", self.base_url))
            .headers(self.headers())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    fn headers(&self) -> header::HeaderMap {
        let mut headers = header::HeaderMap::new();
        if !self.token.is_empty() {
            if let Ok(value) = header::HeaderValue::from_str(&self.token) {
                headers.insert(header::AUTHORIZATION, value);
            }
        }
        headers
    }
}

pub fn normalize_base_url(base_url: &str) -> Result<String, OpenListError> {
    let normalized = base_url.trim().trim_end_matches('/');
    let parsed = Url::parse(normalized).map_err(|_| OpenListError::InvalidBaseUrl)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(OpenListError::InvalidBaseUrl);
    }
    Ok(normalized.to_string())
}

#[cfg(test)]
mod tests {
    use axum::{
        Json, Router,
        extract::State,
        http::HeaderMap,
        routing::{get, post},
    };
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    use super::{OpenListClient, OpenListError, normalize_base_url};

    #[test]
    fn base_url_validation_matches_openlist_endpoint_requirements() {
        assert_eq!(
            normalize_base_url(" https://example.test/proxy/// ").unwrap(),
            "https://example.test/proxy"
        );
        for invalid in [
            "",
            "example.test",
            "ftp://example.test",
            "https://example.test/path?secret=value",
            "https://user:pass@example.test",
        ] {
            assert!(matches!(
                normalize_base_url(invalid),
                Err(OpenListError::InvalidBaseUrl)
            ));
        }
    }

    #[tokio::test]
    async fn endpoint_and_path_checks_use_the_real_openlist_contract() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new()
            .route(
                "/api/public/settings",
                get(|| async { Json(json!({"code": 200})) }),
            )
            .route(
                "/api/public/offline_download_tools",
                get(|| async { Json(json!({"code": 200, "data": [{"name": "Thunder"}]})) }),
            )
            .route(
                "/api/fs/list",
                post(
                    |State(expected): State<String>,
                     headers: HeaderMap,
                     Json(body): Json<Value>| async move {
                        assert_eq!(
                            headers.get("authorization").unwrap(),
                            "synthetic-openlist-token"
                        );
                        assert_eq!(body["path"], "/media/anime");
                        assert_eq!(body["page"], 1);
                        assert_eq!(body["per_page"], 0);
                        assert_eq!(expected, "checked");
                        Json(json!({"code": 200, "data": {"content": []}}))
                    },
                ),
            )
            .with_state("checked".to_string());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client =
            OpenListClient::new(&format!("http://{address}"), "synthetic-openlist-token").unwrap();
        assert!(client.validate_endpoint("thunder").await);
        assert!(!client.validate_endpoint("qBittorrent").await);
        assert_eq!(
            client.validate_path(" /media/anime/ ").await,
            (true, "/media/anime".to_string())
        );
        assert_eq!(
            client.validate_path("relative/path").await,
            (
                false,
                "下载根目录必须是 OpenList 的绝对路径，以 / 开头".to_string()
            )
        );
    }
}
