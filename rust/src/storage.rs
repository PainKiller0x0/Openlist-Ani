use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, params};
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
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_secs(1))?;
    connection.pragma_update(None, "query_only", true)?;

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
