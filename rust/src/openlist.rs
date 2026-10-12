use reqwest::{Client, Url, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

const REQUEST_TIMEOUT_SECONDS: u64 = 30;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OfflineTask {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub state: Value,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub progress: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OpenListFile {
    pub name: String,
    #[serde(default)]
    pub is_dir: bool,
    pub size: Option<u64>,
}

#[derive(Debug, Error)]
pub enum OpenListError {
    #[error("OpenList 地址必须是带 http:// 或 https:// 的完整地址")]
    InvalidBaseUrl,
    #[error("OpenList rejected the operation")]
    Rejected,
    #[error("OpenList returned an invalid response")]
    InvalidResponse,
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

    pub async fn add_offline_download(
        &self,
        urls: &[String],
        path: &str,
        tool: &str,
        name: &str,
    ) -> Result<Vec<OfflineTask>, OpenListError> {
        let mut payload = json!({
            "urls": urls,
            "path": path,
            "tool": tool,
            "delete_policy": "delete_always"
        });
        if !name.is_empty() {
            payload["name"] = json!(name);
        }
        let response = self
            .post_json("/api/fs/add_offline_download", &payload)
            .await?;
        if response.get("code").and_then(Value::as_i64) != Some(200) {
            return Err(OpenListError::Rejected);
        }
        let tasks = response
            .pointer("/data/tasks")
            .and_then(Value::as_array)
            .ok_or(OpenListError::InvalidResponse)?;
        tasks
            .iter()
            .cloned()
            .map(serde_json::from_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| OpenListError::InvalidResponse)
    }

    pub async fn offline_download_tasks(
        &self,
        done: bool,
    ) -> Result<Vec<OfflineTask>, OpenListError> {
        let path = if done {
            "/api/task/offline_download/done"
        } else {
            "/api/task/offline_download/undone"
        };
        self.get_task_list(path).await
    }

    pub async fn offline_transfer_tasks(
        &self,
        done: bool,
    ) -> Result<Vec<OfflineTask>, OpenListError> {
        let path = if done {
            "/api/task/offline_download_transfer/done"
        } else {
            "/api/task/offline_download_transfer/undone"
        };
        self.get_task_list(path).await
    }

    pub async fn mkdir(&self, path: &str) -> Result<bool, OpenListError> {
        let response = self
            .post_json("/api/fs/mkdir", &json!({"path": path}))
            .await?;
        Ok(response.get("code").and_then(Value::as_i64) == Some(200))
    }

    pub async fn move_files(
        &self,
        source_directory: &str,
        destination_directory: &str,
        names: &[String],
    ) -> Result<bool, OpenListError> {
        let response = self
            .post_json(
                "/api/fs/move",
                &json!({
                    "src_dir": source_directory,
                    "dst_dir": destination_directory,
                    "names": names
                }),
            )
            .await?;
        Ok(response.get("code").and_then(Value::as_i64) == Some(200))
    }

    pub async fn remove_path(
        &self,
        directory: &str,
        names: &[String],
    ) -> Result<bool, OpenListError> {
        let response = self
            .post_json("/api/fs/remove", &json!({"dir": directory, "names": names}))
            .await?;
        Ok(response.get("code").and_then(Value::as_i64) == Some(200))
    }

    pub async fn rename_file(&self, path: &str, name: &str) -> Result<bool, OpenListError> {
        let response = self
            .post_json("/api/fs/rename", &json!({"path": path, "name": name}))
            .await?;
        Ok(response.get("code").and_then(Value::as_i64) == Some(200))
    }

    pub async fn list_files(&self, path: &str) -> Result<Vec<OpenListFile>, OpenListError> {
        let response = self
            .post_json(
                "/api/fs/list",
                &json!({
                    "path": path,
                    "password": "",
                    "page": 1,
                    "per_page": 0,
                    "refresh": true
                }),
            )
            .await?;
        if response.get("code").and_then(Value::as_i64) != Some(200) {
            return Err(OpenListError::Rejected);
        }
        let files = response
            .pointer("/data/content")
            .and_then(Value::as_array)
            .ok_or(OpenListError::InvalidResponse)?;
        files
            .iter()
            .cloned()
            .map(serde_json::from_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| OpenListError::InvalidResponse)
    }

    async fn get_task_list(&self, path: &str) -> Result<Vec<OfflineTask>, OpenListError> {
        let response = self.get_json(path).await?;
        if response.get("code").and_then(Value::as_i64) != Some(200) {
            return Err(OpenListError::Rejected);
        }
        let tasks = response
            .get("data")
            .and_then(Value::as_array)
            .ok_or(OpenListError::InvalidResponse)?;
        tasks
            .iter()
            .cloned()
            .map(serde_json::from_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| OpenListError::InvalidResponse)
    }

    async fn post_json(&self, path: &str, payload: &Value) -> Result<Value, OpenListError> {
        Ok(self
            .http
            .post(format!("{}{path}", self.base_url))
            .headers(self.headers())
            .json(payload)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
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

    use super::{OfflineTask, OpenListClient, OpenListError, normalize_base_url};

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

    #[tokio::test]
    async fn download_and_file_operations_match_openlist_http_contract() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new()
            .route(
                "/api/fs/add_offline_download",
                post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                    assert_eq!(headers.get("authorization").unwrap(), "test-token");
                    assert_eq!(body["urls"], json!(["magnet:?xt=urn:btih:abc"]));
                    assert_eq!(body["path"], "/.oani-download-tmp/task-1");
                    assert_eq!(body["tool"], "qBittorrent");
                    assert_eq!(body["delete_policy"], "delete_always");
                    assert_eq!(body["name"], "Series S01E03");
                    Json(json!({"code": 200, "data": {"tasks": [{
                        "id": "remote-1", "name": "Series S01E03", "state": 1,
                        "status": "downloading", "progress": 37
                    }]}}))
                }),
            )
            .route(
                "/api/task/offline_download/undone",
                get(|| async {
                    Json(json!({"code": 200, "data": [{
                        "id": "remote-1", "name": "Series S01E03", "state": 1,
                        "status": "downloading", "progress": 37
                    }]}))
                }),
            )
            .route(
                "/api/task/offline_download_transfer/done",
                get(|| async { Json(json!({"code": 200, "data": []})) }),
            )
            .route(
                "/api/fs/mkdir",
                post(|Json(body): Json<Value>| async move {
                    assert_eq!(body, json!({"path": "/media/Series/Season 1"}));
                    Json(json!({"code": 200}))
                }),
            )
            .route(
                "/api/fs/move",
                post(|Json(body): Json<Value>| async move {
                    assert_eq!(body["src_dir"], "/temp/task-1");
                    assert_eq!(body["dst_dir"], "/media/Series/Season 1");
                    assert_eq!(body["names"], json!(["episode.mkv"]));
                    Json(json!({"code": 200}))
                }),
            )
            .route(
                "/api/fs/remove",
                post(|Json(body): Json<Value>| async move {
                    assert_eq!(body, json!({"dir": "/temp", "names": ["task-1"]}));
                    Json(json!({"code": 200}))
                }),
            );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = OpenListClient::new(&format!("http://{address}"), "test-token").unwrap();
        let urls = vec!["magnet:?xt=urn:btih:abc".to_string()];
        let tasks = client
            .add_offline_download(
                &urls,
                "/.oani-download-tmp/task-1",
                "qBittorrent",
                "Series S01E03",
            )
            .await
            .unwrap();
        assert_eq!(tasks[0].id, "remote-1");
        assert_eq!(tasks[0].state, json!(1));
        assert_eq!(tasks[0].progress, Some(json!(37)));
        assert_eq!(client.offline_download_tasks(false).await.unwrap(), tasks);
        assert!(
            client
                .offline_transfer_tasks(true)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(client.mkdir("/media/Series/Season 1").await.unwrap());
        assert!(
            client
                .move_files(
                    "/temp/task-1",
                    "/media/Series/Season 1",
                    &["episode.mkv".to_string()]
                )
                .await
                .unwrap()
        );
        assert!(
            client
                .remove_path("/temp", &["task-1".to_string()])
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn task_list_api_errors_are_not_treated_as_empty_lists() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/api/task/offline_download/undone",
            get(|| async { Json(json!({"code": 500, "message": "unavailable"})) }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = OpenListClient::new(&format!("http://{address}"), "test-token").unwrap();
        assert!(matches!(
            client.offline_download_tasks(false).await,
            Err(OpenListError::Rejected)
        ));
        let malformed = serde_json::from_value::<OfflineTask>(json!({"id": "t", "state": 1}));
        assert!(malformed.is_ok());
    }
}
