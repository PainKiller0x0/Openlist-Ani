use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::params_from_iter;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

const SQLITE_PARAMETER_CHUNK_SIZE: usize = 900;
const SQLITE_EPISODE_KEY_CHUNK_SIZE: usize = SQLITE_PARAMETER_CHUNK_SIZE / 3;

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
pub struct LegacyEpisodeRelease {
    pub fansub: Option<String>,
    pub quality: Option<String>,
    pub languages: Option<String>,
    pub version: Option<i64>,
}

pub type LegacyEpisodeKey = (String, i64, i64);
pub type LegacyEpisodeLookup = HashMap<LegacyEpisodeKey, Vec<LegacyEpisodeRelease>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyMementoRecord {
    pub task_id: String,
    pub state: String,
    pub updated_at: String,
    pub payload_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmissionReservation {
    Created(String),
    Existing(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmissionOperation {
    pub task_id: String,
    pub state: String,
    pub provider_task_id: Option<String>,
    pub updated_at: String,
    pub last_error: Option<String>,
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

    pub fn find_existing_titles(
        &self,
        candidate_titles: &[String],
    ) -> Result<HashSet<String>, LegacyStorageError> {
        let mut seen = HashSet::new();
        let unique_titles = candidate_titles
            .iter()
            .filter(|title| seen.insert(title.as_str()))
            .collect::<Vec<_>>();
        if unique_titles.is_empty() {
            return Ok(HashSet::new());
        }

        let connection = open_readonly_database(&self.path, DATA_SCHEMA)?;
        let mut found = HashSet::new();
        for chunk in unique_titles.chunks(SQLITE_PARAMETER_CHUNK_SIZE) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let query = format!("SELECT title FROM resources WHERE title IN ({placeholders})");
            let mut statement = connection.prepare(&query)?;
            let titles = statement.query_map(params_from_iter(chunk.iter()), |row| {
                row.get::<_, String>(0)
            })?;
            found.extend(titles.collect::<Result<Vec<_>, _>>()?);
        }
        Ok(found)
    }

    pub fn find_releases_by_episodes(
        &self,
        keys: &[LegacyEpisodeKey],
    ) -> Result<LegacyEpisodeLookup, LegacyStorageError> {
        let mut records = keys
            .iter()
            .cloned()
            .map(|key| (key, Vec::new()))
            .collect::<HashMap<_, _>>();
        let mut seen = HashSet::new();
        let unique_keys = keys
            .iter()
            .filter(|key| seen.insert((key.0.as_str(), key.1, key.2)))
            .collect::<Vec<_>>();
        if unique_keys.is_empty() {
            return Ok(records);
        }

        let connection = open_readonly_database(&self.path, DATA_SCHEMA)?;
        for chunk in unique_keys.chunks(SQLITE_EPISODE_KEY_CHUNK_SIZE) {
            let clauses = ["(anime_name = ? AND season = ? AND episode = ?)"]
                .repeat(chunk.len())
                .join(" OR ");
            let query = format!(
                "SELECT anime_name, season, episode, fansub, quality, languages, version \
                 FROM resources WHERE {clauses}"
            );
            let values = chunk
                .iter()
                .flat_map(|(name, season, episode)| {
                    [
                        rusqlite::types::Value::Text(name.clone()),
                        rusqlite::types::Value::Integer(*season),
                        rusqlite::types::Value::Integer(*episode),
                    ]
                })
                .collect::<Vec<_>>();
            let mut statement = connection.prepare(&query)?;
            let rows = statement.query_map(params_from_iter(values), |row| {
                let key = (row.get(0)?, row.get(1)?, row.get(2)?);
                let release = LegacyEpisodeRelease {
                    fansub: row.get(3)?,
                    quality: row.get(4)?,
                    languages: row.get(5)?,
                    version: row.get(6)?,
                };
                Ok((key, release))
            })?;
            for row in rows {
                let (key, release) = row?;
                records.entry(key).or_default().push(release);
            }
        }
        Ok(records)
    }

    pub fn find_latest_episodes(
        &self,
        anime_names: &[String],
        seasons_by_name: &HashMap<String, i64>,
    ) -> Result<HashMap<String, i64>, LegacyStorageError> {
        let mut seen = HashSet::new();
        let names = anime_names
            .iter()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty() && seen.insert(*name))
            .collect::<Vec<_>>();
        if names.is_empty() {
            return Ok(HashMap::new());
        }

        let connection = open_readonly_database(&self.path, DATA_SCHEMA)?;
        let mut latest = HashMap::new();
        for chunk in names.chunks(SQLITE_PARAMETER_CHUNK_SIZE) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let query = format!(
                "SELECT anime_name, season, MAX(episode) FROM resources \
                 WHERE anime_name IN ({placeholders}) AND episode IS NOT NULL \
                 GROUP BY anime_name, season"
            );
            let mut statement = connection.prepare(&query)?;
            let rows = statement.query_map(params_from_iter(chunk.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;
            let mut grouped: HashMap<String, Vec<(Option<i64>, i64)>> = HashMap::new();
            for row in rows {
                let (name, season, episode) = row?;
                grouped.entry(name).or_default().push((season, episode));
            }

            for name in chunk {
                let requested_season = seasons_by_name.get(*name);
                let episode = grouped.get(*name).and_then(|candidates| {
                    candidates
                        .iter()
                        .filter(|(season, _)| {
                            requested_season.is_none_or(|requested| *season == Some(*requested))
                        })
                        .map(|(_, episode)| *episode)
                        .max()
                });
                if let Some(episode) = episode {
                    latest.insert((*name).to_string(), episode);
                }
            }
        }

        let missing_names = names
            .iter()
            .filter(|name| !latest.contains_key(**name))
            .copied()
            .collect::<Vec<_>>();
        if missing_names.is_empty() {
            return Ok(latest);
        }

        let mut statement = connection.prepare(
            "SELECT anime_name, season, episode FROM resources \
             WHERE anime_name IS NOT NULL AND episode IS NOT NULL",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut normalized: HashMap<String, Vec<(Option<i64>, i64)>> = HashMap::new();
        for row in rows {
            let (name, season, episode) = row?;
            normalized
                .entry(normalize_series_name(&name))
                .or_default()
                .push((season, episode));
        }

        for name in missing_names {
            let requested_season = seasons_by_name.get(name);
            let episode = normalized
                .get(&normalize_series_name(name))
                .and_then(|candidates| {
                    candidates
                        .iter()
                        .filter(|(season, _)| {
                            requested_season.is_none_or(|requested| *season == Some(*requested))
                        })
                        .map(|(_, episode)| *episode)
                        .max()
                });
            if let Some(episode) = episode {
                latest.insert(name.to_string(), episode);
            }
        }
        Ok(latest)
    }
}

fn normalize_series_name(name: &str) -> String {
    let chars = name.chars().collect::<Vec<_>>();
    let mut without_years = Vec::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        if let Some(end) = bracketed_year_end(&chars, index) {
            index = end;
        } else {
            without_years.push(chars[index]);
            index += 1;
        }
    }

    let mut normalized = String::new();
    index = 0;
    while index < without_years.len() {
        if let Some(end) = season_token_end(&without_years, index) {
            index = end;
        } else {
            for lowercase in without_years[index].to_lowercase() {
                if lowercase.is_alphanumeric() || lowercase == '_' {
                    normalized.push(lowercase);
                }
            }
            index += 1;
        }
    }
    normalized
}

fn bracketed_year_end(chars: &[char], start: usize) -> Option<usize> {
    if !matches!(chars.get(start)?, '（' | '(' | '[' | '【') {
        return None;
    }
    let mut index = start + 1;
    while chars
        .get(index)
        .is_some_and(|character| character.is_whitespace())
    {
        index += 1;
    }
    for _ in 0..4 {
        if !chars.get(index).is_some_and(char::is_ascii_digit) {
            return None;
        }
        index += 1;
    }
    while chars
        .get(index)
        .is_some_and(|character| character.is_whitespace())
    {
        index += 1;
    }
    if chars
        .get(index)
        .is_some_and(|character| matches!(character, '）' | ')' | ']' | '】'))
    {
        Some(index + 1)
    } else {
        None
    }
}

fn season_token_end(chars: &[char], start: usize) -> Option<usize> {
    if chars.get(start) == Some(&'第') {
        let mut index = start + 1;
        while chars
            .get(index)
            .is_some_and(|character| character.is_whitespace())
        {
            index += 1;
        }
        let number_start = index;
        while chars.get(index).is_some_and(|character| {
            character.is_ascii_digit()
                || matches!(
                    character,
                    '一' | '二'
                        | '三'
                        | '四'
                        | '五'
                        | '六'
                        | '七'
                        | '八'
                        | '九'
                        | '十'
                        | '百'
                        | '千'
                        | '万'
                )
        }) {
            index += 1;
        }
        if index == number_start {
            return None;
        }
        while chars
            .get(index)
            .is_some_and(|character| character.is_whitespace())
        {
            index += 1;
        }
        return (chars.get(index) == Some(&'季')).then_some(index + 1);
    }

    if ascii_word_at(chars, start, "season") {
        let mut index = start + "season".len();
        while chars
            .get(index)
            .is_some_and(|character| character.is_whitespace())
        {
            index += 1;
        }
        let digits_start = index;
        while chars.get(index).is_some_and(char::is_ascii_digit) {
            index += 1;
        }
        if index > digits_start && is_season_boundary(chars.get(index)) {
            return Some(index);
        }
    }

    if chars
        .get(start)
        .is_some_and(|character| character.eq_ignore_ascii_case(&'s'))
    {
        let mut index = start + 1;
        while index - start <= 2 && chars.get(index).is_some_and(char::is_ascii_digit) {
            index += 1;
        }
        let digit_count = index - start - 1;
        if (1..=2).contains(&digit_count) && is_season_boundary(chars.get(index)) {
            return Some(index);
        }
    }
    None
}

fn ascii_word_at(chars: &[char], start: usize, word: &str) -> bool {
    chars.get(start..start + word.len()).is_some_and(|slice| {
        slice
            .iter()
            .zip(word.chars())
            .all(|(actual, expected)| actual.eq_ignore_ascii_case(&expected))
    })
}

fn is_season_boundary(character: Option<&char>) -> bool {
    character.is_none_or(|character| {
        character.is_whitespace()
            || matches!(
                character,
                '.' | '_' | '(' | ')' | '（' | '）' | '[' | ']' | '【' | '】' | '-'
            )
    })
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
        let _: Map<String, Value> = serde_json::from_str(&record.payload_json)?;
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

    pub fn reserve_submission(
        &self,
        record: &LegacyMementoRecord,
        download_url: &str,
    ) -> Result<SubmissionReservation, LegacyStorageError> {
        let _: Map<String, Value> = serde_json::from_str(&record.payload_json)?;
        let source_identity = format!("{:x}", Sha256::digest(download_url.as_bytes()));
        let mut connection = open_writable_database(&self.path, TASK_SCHEMA)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_submission_schema(&transaction)?;

        let existing = transaction
            .query_row(
                "SELECT task_id FROM rust_download_operations \
                 WHERE source_identity = ?1 AND state IN \
                 ('intent', 'sending', 'uncertain', 'submitted')",
                params![source_identity],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(task_id) = existing {
            transaction.commit()?;
            return Ok(SubmissionReservation::Existing(task_id));
        }

        transaction.execute(
            "INSERT INTO task_mementos (task_id, state, updated_at, payload) \
             VALUES (?1, ?2, ?3, ?4)",
            params![
                record.task_id,
                record.state,
                record.updated_at,
                record.payload_json,
            ],
        )?;
        transaction.execute(
            "INSERT INTO rust_download_operations \
             (task_id, source_identity, state, updated_at) VALUES (?1, ?2, 'intent', ?3)",
            params![record.task_id, source_identity, record.updated_at],
        )?;
        transaction.commit()?;
        Ok(SubmissionReservation::Created(record.task_id.clone()))
    }

    pub fn begin_submission(&self, task_id: &str) -> Result<bool, LegacyStorageError> {
        let mut connection = open_writable_database(&self.path, TASK_SCHEMA)?;
        let transaction = connection.transaction()?;
        ensure_submission_schema(&transaction)?;
        let changed = transaction.execute(
            "UPDATE rust_download_operations SET state = 'sending' \
             WHERE task_id = ?1 AND state = 'intent'",
            params![task_id],
        )?;
        transaction.commit()?;
        Ok(changed == 1)
    }

    pub fn mark_submission_accepted(
        &self,
        task_id: &str,
        provider_task_id: &str,
        updated_at: &str,
    ) -> Result<bool, LegacyStorageError> {
        let mut connection = open_writable_database(&self.path, TASK_SCHEMA)?;
        let transaction = connection.transaction()?;
        ensure_submission_schema(&transaction)?;
        let changed = transaction.execute(
            "UPDATE rust_download_operations SET state = 'submitted', \
             provider_task_id = ?2, updated_at = ?3 \
             WHERE task_id = ?1 AND state = 'sending'",
            params![task_id, provider_task_id, updated_at],
        )?;
        transaction.commit()?;
        Ok(changed == 1)
    }

    pub fn mark_submission_uncertain(
        &self,
        task_id: &str,
        error: &str,
        updated_at: &str,
    ) -> Result<bool, LegacyStorageError> {
        let mut connection = open_writable_database(&self.path, TASK_SCHEMA)?;
        let transaction = connection.transaction()?;
        ensure_submission_schema(&transaction)?;
        let changed = transaction.execute(
            "UPDATE rust_download_operations SET state = 'uncertain', \
             last_error = ?2, updated_at = ?3 \
             WHERE task_id = ?1 AND state = 'sending'",
            params![task_id, error, updated_at],
        )?;
        transaction.commit()?;
        Ok(changed == 1)
    }

    pub fn mark_submission_completed(
        &self,
        task_id: &str,
        updated_at: &str,
    ) -> Result<bool, LegacyStorageError> {
        let mut connection = open_writable_database(&self.path, TASK_SCHEMA)?;
        let transaction = connection.transaction()?;
        ensure_submission_schema(&transaction)?;
        let changed = transaction.execute(
            "UPDATE rust_download_operations SET state = 'completed', last_error = NULL, updated_at = ?2 \
             WHERE task_id = ?1 AND state = 'submitted'",
            params![task_id, updated_at],
        )?;
        let completed = changed == 1
            || transaction
                .query_row(
                    "SELECT state FROM rust_download_operations WHERE task_id = ?1",
                    params![task_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .as_deref()
                == Some("completed");
        if completed {
            transaction.execute(
                "DELETE FROM task_mementos WHERE task_id = ?1",
                params![task_id],
            )?;
        }
        transaction.commit()?;
        Ok(completed)
    }

    pub fn mark_submission_failed(
        &self,
        task_id: &str,
        error: &str,
        updated_at: &str,
    ) -> Result<bool, LegacyStorageError> {
        let mut connection = open_writable_database(&self.path, TASK_SCHEMA)?;
        let transaction = connection.transaction()?;
        ensure_submission_schema(&transaction)?;
        let changed = transaction.execute(
            "UPDATE rust_download_operations SET state = 'failed', last_error = ?2, updated_at = ?3 \
             WHERE task_id = ?1 AND state IN ('sending', 'submitted')",
            params![task_id, error, updated_at],
        )?;
        transaction.commit()?;
        Ok(changed == 1)
    }

    pub fn recover_interrupted_submissions(&self) -> Result<usize, LegacyStorageError> {
        let mut connection = open_writable_database(&self.path, TASK_SCHEMA)?;
        let transaction = connection.transaction()?;
        ensure_submission_schema(&transaction)?;
        let changed = transaction.execute(
            "UPDATE rust_download_operations SET state = 'uncertain', \
             last_error = COALESCE(last_error, 'process stopped during provider submission') \
             WHERE state = 'sending'",
            [],
        )?;
        transaction.commit()?;
        Ok(changed)
    }

    pub fn load_submission_operations(
        &self,
    ) -> Result<Vec<SubmissionOperation>, LegacyStorageError> {
        let connection = open_writable_database(&self.path, TASK_SCHEMA)?;
        ensure_submission_schema(&connection)?;
        let mut statement = connection.prepare(
            "SELECT task_id, state, provider_task_id, updated_at, last_error \
             FROM rust_download_operations ORDER BY updated_at, task_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(SubmissionOperation {
                task_id: row.get(0)?,
                state: row.get(1)?,
                provider_task_id: row.get(2)?,
                updated_at: row.get(3)?,
                last_error: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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

fn ensure_submission_schema(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS rust_download_operations (
            task_id TEXT PRIMARY KEY,
            source_identity TEXT NOT NULL,
            state TEXT NOT NULL,
            provider_task_id TEXT,
            updated_at TEXT NOT NULL,
            last_error TEXT
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_rust_download_active_source \
        ON rust_download_operations(source_identity) \
        WHERE state IN ('intent', 'sending', 'uncertain', 'submitted');",
    )
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
