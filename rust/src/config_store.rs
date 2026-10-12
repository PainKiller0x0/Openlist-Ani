use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use fs2::FileExt;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use toml_edit::{DocumentMut, Table};
use uuid::Uuid;

use crate::config::{AppConfig, ConfigError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigSnapshot {
    pub app: AppConfig,
    pub revision: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UiSettings {
    pub global_exclude_patterns: Vec<String>,
    pub llm: UiLlmSettings,
    pub download_path: String,
    pub openlist_url: String,
    pub rename_format: String,
    pub poll_interval_seconds: i64,
    pub max_download_retries: i64,
    pub mikan_base_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UiLlmSettings {
    pub provider_type: String,
    pub base_url: String,
    pub model: String,
    pub api_key_configured: bool,
    pub tmdb_language: String,
    pub metadata_parser_provider: String,
}

pub struct OpenListValidationContext {
    pub revision: String,
    pub base_url: String,
    pub download_path: String,
    pub token: String,
    pub offline_download_tool: String,
}

#[derive(Clone, Debug)]
pub struct DownloadRuntimeSettings {
    pub openlist_url: String,
    pub download_path: String,
    pub token: String,
    pub offline_download_tool: String,
    pub rename_format: String,
}

#[derive(Default)]
pub struct UiSettingsUpdate {
    pub global_exclude_patterns: Vec<String>,
    pub openlist_url: Option<String>,
    pub download_path: Option<String>,
    pub rename_format: Option<String>,
    pub poll_interval_seconds: Option<i64>,
    pub max_download_retries: Option<i64>,
    pub mikan_base_url: Option<String>,
    pub llm_provider_type: Option<String>,
    pub llm_api_key: Option<String>,
    pub llm_base_url: Option<String>,
    pub llm_model: Option<String>,
    pub tmdb_language: Option<String>,
    pub metadata_parser_provider: Option<String>,
}

#[derive(Debug, Error)]
pub enum ConfigStoreError {
    #[error("configuration file operation failed")]
    Io(#[from] std::io::Error),
    #[error("invalid configuration")]
    InvalidConfig(#[from] ConfigError),
    #[error("configuration changed since it was read")]
    RevisionConflict,
    #[error("configuration was replaced, but directory durability is uncertain")]
    DurabilityUncertain(#[source] std::io::Error),
    #[error("configuration section has an unexpected shape")]
    InvalidShape,
}

pub struct ConfigDocumentStore {
    path: PathBuf,
}

impl ConfigDocumentStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> Result<ConfigSnapshot, ConfigStoreError> {
        let source = fs::read_to_string(&self.path)?;
        let document = source.parse::<DocumentMut>().map_err(ConfigError::from)?;
        let app = AppConfig::from_document(&self.path, &document)?;
        Ok(ConfigSnapshot {
            app,
            revision: revision(source.as_bytes()),
        })
    }

    pub fn ui_settings(&self) -> Result<UiSettings, ConfigStoreError> {
        let source = fs::read_to_string(&self.path)?;
        let document = source.parse::<DocumentMut>().map_err(ConfigError::from)?;
        let text = |section: &str, key: &str, default: &str| {
            document
                .get(section)
                .and_then(toml_edit::Item::as_table)
                .and_then(|table| table.get(key))
                .and_then(toml_edit::Item::as_str)
                .unwrap_or(default)
                .to_string()
        };
        let integer = |section: &str, key: &str, default: i64| {
            document
                .get(section)
                .and_then(toml_edit::Item::as_table)
                .and_then(|table| table.get(key))
                .and_then(toml_edit::Item::as_integer)
                .unwrap_or(default)
        };
        let global_exclude_patterns = document
            .get("rss")
            .and_then(toml_edit::Item::as_table)
            .and_then(|rss| rss.get("filter"))
            .and_then(toml_edit::Item::as_table)
            .and_then(|filter| filter.get("exclude_patterns"))
            .and_then(toml_edit::Item::as_array)
            .map(|patterns| {
                patterns
                    .iter()
                    .filter_map(toml_edit::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let api_key_configured = !text("llm", "openai_api_key", "").trim().is_empty();

        Ok(UiSettings {
            global_exclude_patterns,
            llm: UiLlmSettings {
                provider_type: text("llm", "provider_type", "openai"),
                base_url: text("llm", "openai_base_url", "https://api.openai.com/v1"),
                model: text("llm", "openai_model", "gpt-4o"),
                api_key_configured,
                tmdb_language: text("llm", "tmdb_language", "zh-CN"),
                metadata_parser_provider: text("metadata_parser", "provider", "regex"),
            },
            download_path: text("openlist", "download_path", "/"),
            openlist_url: text("openlist", "url", "http://localhost:5244"),
            rename_format: text(
                "openlist",
                "rename_format",
                "{anime_name} - S{season:02d}E{episode:02d}",
            ),
            poll_interval_seconds: integer("rss", "interval_time", 300),
            max_download_retries: integer("rss", "max_download_retries", 3),
            mikan_base_url: text("mikan", "base_url", "https://mikanani.kas.pub/"),
        })
    }

    pub fn ui_subscriptions(&self) -> Result<Vec<Value>, ConfigStoreError> {
        let source = fs::read_to_string(&self.path)?;
        let document = source.parse::<DocumentMut>().map_err(ConfigError::from)?;
        let root = document
            .get("openlist")
            .and_then(toml_edit::Item::as_table)
            .and_then(|table| table.get("download_path"))
            .and_then(toml_edit::Item::as_str)
            .unwrap_or("/")
            .trim_end_matches('/')
            .to_string();
        let subscriptions = document
            .get("rss")
            .and_then(toml_edit::Item::as_table)
            .and_then(|rss| rss.get("subscriptions"))
            .and_then(toml_edit::Item::as_array_of_tables);

        if let Some(subscriptions) = subscriptions {
            return Ok(subscriptions
                .iter()
                .map(|item| subscription_json(item, &root))
                .collect());
        }

        let legacy_urls = document
            .get("rss")
            .and_then(toml_edit::Item::as_table)
            .and_then(|rss| rss.get("urls"))
            .and_then(toml_edit::Item::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml_edit::Value::as_str)
            .map(|url| {
                json!({
                    "url": url,
                    "name": "",
                    "anime_name": "",
                    "download_directory_name": "",
                    "episode_offset": 0,
                    "filename_season_override": 0,
                    "directory_season_override": 0,
                    "enabled": true,
                    "completed": false,
                    "tmdb_id": null,
                    "poster_url": "",
                    "season": 1,
                    "exclude_patterns": [],
                    "latest_episode": null,
                    "download_directory": ""
                })
            })
            .collect();
        Ok(legacy_urls)
    }

    pub fn openlist_validation_context(
        &self,
    ) -> Result<OpenListValidationContext, ConfigStoreError> {
        let source = fs::read_to_string(&self.path)?;
        let document = source.parse::<DocumentMut>().map_err(ConfigError::from)?;
        let value = |key: &str, default: &str| {
            document
                .get("openlist")
                .and_then(toml_edit::Item::as_table)
                .and_then(|table| table.get(key))
                .and_then(toml_edit::Item::as_str)
                .unwrap_or(default)
                .to_string()
        };
        Ok(OpenListValidationContext {
            revision: revision(source.as_bytes()),
            base_url: value("url", "http://localhost:5244"),
            download_path: value("download_path", "/"),
            token: value("token", ""),
            offline_download_tool: value("offline_download_tool", "qBittorrent"),
        })
    }

    pub fn download_runtime_settings(&self) -> Result<DownloadRuntimeSettings, ConfigStoreError> {
        let openlist = self.openlist_validation_context()?;
        let ui = self.ui_settings()?;
        Ok(DownloadRuntimeSettings {
            openlist_url: openlist.base_url,
            download_path: openlist.download_path,
            token: openlist.token,
            offline_download_tool: openlist.offline_download_tool,
            rename_format: ui.rename_format,
        })
    }

    pub fn update_ui_settings(
        &self,
        expected_revision: &str,
        settings: UiSettingsUpdate,
    ) -> Result<ConfigSnapshot, ConfigStoreError> {
        self.update_checked(expected_revision, |document| {
            let root = document.as_table_mut();
            if let Some(value) = settings.openlist_url {
                get_or_insert_table(root, "openlist")?.insert("url", toml_edit::value(value));
            }
            if let Some(value) = settings.download_path {
                get_or_insert_table(root, "openlist")?
                    .insert("download_path", toml_edit::value(value));
            }
            if let Some(value) = settings.rename_format {
                get_or_insert_table(root, "openlist")?
                    .insert("rename_format", toml_edit::value(value));
            }
            if let Some(value) = settings.poll_interval_seconds {
                get_or_insert_table(root, "rss")?.insert("interval_time", toml_edit::value(value));
            }
            if let Some(value) = settings.max_download_retries {
                get_or_insert_table(root, "rss")?
                    .insert("max_download_retries", toml_edit::value(value));
            }
            if let Some(value) = settings.mikan_base_url {
                get_or_insert_table(root, "mikan")?.insert("base_url", toml_edit::value(value));
            }
            if let Some(value) = settings.llm_provider_type {
                get_or_insert_table(root, "llm")?.insert("provider_type", toml_edit::value(value));
            }
            if let Some(value) = settings.llm_api_key {
                get_or_insert_table(root, "llm")?.insert("openai_api_key", toml_edit::value(value));
            }
            if let Some(value) = settings.llm_base_url {
                get_or_insert_table(root, "llm")?
                    .insert("openai_base_url", toml_edit::value(value));
            }
            if let Some(value) = settings.llm_model {
                get_or_insert_table(root, "llm")?.insert("openai_model", toml_edit::value(value));
            }
            if let Some(value) = settings.tmdb_language {
                get_or_insert_table(root, "llm")?.insert("tmdb_language", toml_edit::value(value));
            }
            if let Some(value) = settings.metadata_parser_provider {
                get_or_insert_table(root, "metadata_parser")?
                    .insert("provider", toml_edit::value(value));
            }
            let rss = get_or_insert_table(root, "rss")?;
            let filter = get_or_insert_table(rss, "filter")?;
            let mut patterns = toml_edit::Array::new();
            for pattern in settings.global_exclude_patterns {
                patterns.push(pattern);
            }
            filter.insert("exclude_patterns", toml_edit::value(patterns));
            Ok(())
        })
    }

    pub fn update(
        &self,
        expected_revision: &str,
        edit: impl FnOnce(&mut DocumentMut),
    ) -> Result<ConfigSnapshot, ConfigStoreError> {
        self.update_checked(expected_revision, |document| {
            edit(document);
            Ok(())
        })
    }

    fn update_checked(
        &self,
        expected_revision: &str,
        edit: impl FnOnce(&mut DocumentMut) -> Result<(), ConfigStoreError>,
    ) -> Result<ConfigSnapshot, ConfigStoreError> {
        let _lock = self.lock()?;
        let source = fs::read_to_string(&self.path)?;
        if revision(source.as_bytes()) != expected_revision {
            return Err(ConfigStoreError::RevisionConflict);
        }

        let mut document = source.parse::<DocumentMut>().map_err(ConfigError::from)?;
        let original = document.clone();
        edit(&mut document)?;
        preserve_value_decor(&original, &mut document);
        let updated = document.to_string();
        let validated = updated.parse::<DocumentMut>().map_err(ConfigError::from)?;
        let app = AppConfig::from_document(&self.path, &validated)?;
        atomic_replace(&self.path, updated.as_bytes())?;

        Ok(ConfigSnapshot {
            app,
            revision: revision(updated.as_bytes()),
        })
    }

    fn lock(&self) -> Result<File, ConfigStoreError> {
        let mut lock_path = self.path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(PathBuf::from(lock_path))?;
        file.lock_exclusive()?;
        Ok(file)
    }
}

fn get_or_insert_table<'a>(
    parent: &'a mut Table,
    key: &str,
) -> Result<&'a mut Table, ConfigStoreError> {
    let entry = parent
        .entry(key)
        .or_insert(toml_edit::Item::Table(Table::new()));
    entry.as_table_mut().ok_or(ConfigStoreError::InvalidShape)
}

fn preserve_value_decor(original: &DocumentMut, updated: &mut DocumentMut) {
    for (key, item) in updated.iter_mut() {
        if let Some(previous) = original.get(key.get()) {
            preserve_item_decor(previous, item);
        }
    }
}

fn preserve_item_decor(original: &toml_edit::Item, updated: &mut toml_edit::Item) {
    if let (Some(previous), Some(current)) = (original.as_value(), updated.as_value_mut()) {
        let decor = current.decor_mut();
        if decor.prefix().is_none() && decor.suffix().is_none() {
            *decor = previous.decor().clone();
        }
    }

    if let (Some(previous), Some(current)) = (original.as_table(), updated.as_table_mut()) {
        preserve_table_decor(previous, current);
    }

    if let (Some(previous), Some(current)) = (
        original.as_array_of_tables(),
        updated.as_array_of_tables_mut(),
    ) {
        for index in 0..current.len() {
            if let (Some(old_table), Some(new_table)) =
                (previous.get(index), current.get_mut(index))
            {
                preserve_table_decor(old_table, new_table);
            }
        }
    }
}

fn preserve_table_decor(original: &Table, updated: &mut Table) {
    for (key, item) in updated.iter_mut() {
        if let Some(previous) = original.get(key.get()) {
            preserve_item_decor(previous, item);
        }
    }
}

fn subscription_json(table: &Table, root: &str) -> Value {
    let text = |key: &str| {
        table
            .get(key)
            .and_then(toml_edit::Item::as_str)
            .unwrap_or("")
    };
    let integer = |key: &str, default: i64| {
        table
            .get(key)
            .and_then(toml_edit::Item::as_integer)
            .unwrap_or(default)
    };
    let boolean = |key: &str, default: bool| {
        table
            .get(key)
            .and_then(toml_edit::Item::as_bool)
            .unwrap_or(default)
    };
    let name = text("name");
    let anime_name = text("anime_name");
    let directory_override = integer("directory_season_override", 0);
    let season = integer("season", 1);
    let directory_name = [text("download_directory_name"), anime_name, name]
        .into_iter()
        .find(|value| !value.trim().is_empty())
        .unwrap_or("未命名订阅")
        .chars()
        .map(|character| {
            if "<>:\"/\\|?*".contains(character) {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .trim()
        .to_string();
    let archive_season = if directory_override > 0 {
        directory_override
    } else {
        season.max(1)
    };
    let download_directory = format!("{root}/{directory_name}/Season {archive_season}");
    let exclude_patterns = table
        .get("exclude_patterns")
        .and_then(toml_edit::Item::as_array)
        .map(|patterns| {
            patterns
                .iter()
                .filter_map(toml_edit::Value::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "url": text("url"),
        "name": name,
        "anime_name": anime_name,
        "download_directory_name": text("download_directory_name"),
        "episode_offset": integer("episode_offset", 0),
        "filename_season_override": integer("filename_season_override", 0),
        "directory_season_override": directory_override,
        "enabled": boolean("enabled", true),
        "completed": boolean("completed", false),
        "tmdb_id": table.get("tmdb_id").and_then(toml_edit::Item::as_integer),
        "poster_url": text("poster_url"),
        "season": season,
        "exclude_patterns": exclude_patterns,
        "latest_episode": null,
        "download_directory": download_directory
    })
}

fn revision(source: &[u8]) -> String {
    format!("{:x}", Sha256::digest(source))
}

fn atomic_replace(path: &Path, content: &[u8]) -> Result<(), ConfigStoreError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".openlist-ani-config-{}.tmp", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(content)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        File::open(parent)?
            .sync_all()
            .map_err(ConfigStoreError::DurabilityUncertain)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
