use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;
use thiserror::Error;

const DATA_SCHEMA: &[(&str, &[&str])] = &[
    (
        "resources",
        &[
            "id",
            "url",
            "title",
            "anime_name",
            "season",
            "episode",
            "fansub",
            "quality",
            "languages",
            "version",
            "downloaded_at",
            "episode_name",
        ],
    ),
    ("strict_rejections", &["url", "title", "rejected_at"]),
];

const TASK_SCHEMA: &[(&str, &[&str])] = &[
    (
        "task_mementos",
        &["task_id", "state", "updated_at", "payload"],
    ),
    ("task_memento_metadata", &["key", "value"]),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyDbPaths {
    pub data_db: PathBuf,
    pub task_mementos_db: PathBuf,
}

#[derive(Debug, Error)]
pub enum LegacyStorageError {
    #[error("legacy database is unavailable or unreadable")]
    Database(#[from] rusqlite::Error),
    #[error("legacy database schema is incomplete")]
    Schema,
    #[error("task memento payload is not valid JSON")]
    InvalidPayload(#[from] serde_json::Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyResourceInput {
    pub url: String,
    pub title: String,
    pub anime_name: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub fansub: Option<String>,
    pub quality: Option<String>,
    pub languages: Option<String>,
    pub version: Option<i64>,
    pub downloaded_at: Option<String>,
    pub episode_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyResourceRecord {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub anime_name: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub fansub: Option<String>,
    pub quality: Option<String>,
    pub languages: Option<String>,
    pub version: Option<i64>,
    pub downloaded_at: Option<String>,
    pub episode_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyMementoRecord {
    pub task_id: String,
    pub state: String,
    pub updated_at: String,
    pub payload_json: String,
}

pub struct LegacyResourceStore {
    path: PathBuf,
}

impl LegacyResourceStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn insert_ignore_duplicate(
        &self,
        resource: &LegacyResourceInput,
    ) -> Result<bool, LegacyStorageError> {
        let connection = open_writable_database(&self.path, DATA_SCHEMA)?;
        let changed = connection.execute(
            "INSERT OR IGNORE INTO resources
                (url, title, anime_name, season, episode, fansub, quality, languages,
                 version, downloaded_at, episode_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                     COALESCE(?10, CURRENT_TIMESTAMP), ?11)",
            params![
                resource.url,
                resource.title,
                resource.anime_name,
                resource.season,
                resource.episode,
                resource.fansub,
                resource.quality,
                resource.languages,
                resource.version,
                resource.downloaded_at,
                resource.episode_name,
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn find_by_title(
        &self,
        title: &str,
    ) -> Result<Option<LegacyResourceRecord>, LegacyStorageError> {
        let connection = open_readonly_database(&self.path, DATA_SCHEMA)?;
        let result = connection.query_row(
            "SELECT id, url, title, anime_name, season, episode, fansub, quality,
                    languages, version, downloaded_at, episode_name
             FROM resources WHERE title = ?1",
            params![title],
            |row| {
                Ok(LegacyResourceRecord {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    title: row.get(2)?,
                    anime_name: row.get(3)?,
                    season: row.get(4)?,
                    episode: row.get(5)?,
                    fansub: row.get(6)?,
                    quality: row.get(7)?,
                    languages: row.get(8)?,
                    version: row.get(9)?,
                    downloaded_at: row.get(10)?,
                    episode_name: row.get(11)?,
                })
            },
        );

        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

pub struct LegacyTaskMementoStore {
    path: PathBuf,
}

impl LegacyTaskMementoStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load_all(&self) -> Result<Vec<LegacyMementoRecord>, LegacyStorageError> {
        let connection = open_readonly_database(&self.path, TASK_SCHEMA)?;
        let mut statement = connection.prepare(
            "SELECT task_id, state, updated_at, payload
             FROM task_mementos ORDER BY updated_at, task_id",
        )?;
        let records = statement
            .query_map([], |row| {
                Ok(LegacyMementoRecord {
                    task_id: row.get(0)?,
                    state: row.get(1)?,
                    updated_at: row.get(2)?,
                    payload_json: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    pub fn save(&self, record: &LegacyMementoRecord) -> Result<(), LegacyStorageError> {
        if matches!(record.state.as_str(), "completed" | "cancelled") {
            return self.delete(&record.task_id);
        }
        let _: Value = serde_json::from_str(&record.payload_json)?;
        let connection = open_writable_database(&self.path, TASK_SCHEMA)?;
        connection.execute(
            "INSERT INTO task_mementos (task_id, state, updated_at, payload)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(task_id) DO UPDATE SET
                state = excluded.state,
                updated_at = excluded.updated_at,
                payload = excluded.payload",
            params![
                record.task_id,
                record.state,
                record.updated_at,
                record.payload_json,
            ],
        )?;
        Ok(())
    }

    pub fn delete(&self, task_id: &str) -> Result<(), LegacyStorageError> {
        let connection = open_writable_database(&self.path, TASK_SCHEMA)?;
        connection.execute(
            "DELETE FROM task_mementos WHERE task_id = ?1",
            params![task_id],
        )?;
        Ok(())
    }
}

pub fn check_legacy_databases(paths: &LegacyDbPaths) -> Result<(), LegacyStorageError> {
    check_database(&paths.data_db, DATA_SCHEMA)?;
    check_database(&paths.task_mementos_db, TASK_SCHEMA)?;
    Ok(())
}

fn check_database(
    path: &Path,
    expected_schema: &[(&str, &[&str])],
) -> Result<(), LegacyStorageError> {
    let _connection = open_readonly_database(path, expected_schema)?;
    Ok(())
}

fn open_readonly_database(
    path: &Path,
    expected_schema: &[(&str, &[&str])],
) -> Result<Connection, LegacyStorageError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_secs(1))?;
    connection.pragma_update(None, "query_only", true)?;
    check_connection_schema(&connection, expected_schema)?;
    Ok(connection)
}

fn open_writable_database(
    path: &Path,
    expected_schema: &[(&str, &[&str])],
) -> Result<Connection, LegacyStorageError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_secs(30))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    check_connection_schema(&connection, expected_schema)?;
    Ok(connection)
}

fn check_connection_schema(
    connection: &Connection,
    expected_schema: &[(&str, &[&str])],
) -> Result<(), LegacyStorageError> {
    for (table, columns) in expected_schema {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
            params![table],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(LegacyStorageError::Schema);
        }

        let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
        let found = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        if columns
            .iter()
            .any(|column| !found.iter().any(|name| name == column))
        {
            return Err(LegacyStorageError::Schema);
        }
    }

    connection.query_row("SELECT 1", [], |_| Ok(()))?;
    Ok(())
}
