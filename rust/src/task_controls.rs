//! Durable failed-task controls. Call from a blocking worker in HTTP handlers.

use std::{path::Path, time::Duration};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::storage::{LegacyMementoRecord, LegacyStorageError, LegacyTaskMementoStore};

#[derive(Debug, Error)]
pub enum TaskControlError {
    #[error("download task not found")]
    NotFound,
    #[error("{0}")]
    Conflict(&'static str),
    #[error(transparent)]
    Storage(#[from] LegacyStorageError),
}

impl TaskControlError {
    pub fn status_code(&self) -> u16 {
        match self {
            Self::NotFound => 404,
            Self::Conflict(_) => 409,
            Self::Storage(_) => 500,
        }
    }
}

impl From<rusqlite::Error> for TaskControlError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.into())
    }
}

impl From<serde_json::Error> for TaskControlError {
    fn from(error: serde_json::Error) -> Self {
        Self::Storage(error.into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryAction {
    /// Start the existing task's worker; never call submit_download.
    Resume,
    /// The HTTP layer may submit a new task using the returned release payload.
    Resubmit,
}

#[derive(Clone, Debug)]
pub struct RetryTaskResult {
    pub task: LegacyMementoRecord,
    pub action: RetryAction,
}

/// Resume a bound failed task, or release an explicitly rejected submission.
/// This function does not contact OpenList or start a worker.
pub fn retry_failed_task(
    path: impl AsRef<Path>,
    task_id: &str,
) -> Result<RetryTaskResult, TaskControlError> {
    let path = path.as_ref();
    let store = LegacyTaskMementoStore::new(path);
    // Use the legacy store's schema migration, then query under the write lock.
    store.load_submission_operations()?;
    let mut connection = open_database(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (mut task, mut payload) = load_failed_task(path, task_id)?;
    let url = payload["release"]["download_url"]
        .as_str()
        .filter(|url| !url.trim().is_empty())
        .ok_or(TaskControlError::Conflict("task has no download URL"))?;
    let identity = format!("{:x}", Sha256::digest(url.as_bytes()));
    let operation = transaction.query_row(
        "SELECT state, provider_task_id, source_identity, last_error FROM rust_download_operations WHERE task_id = ?1",
        params![task_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?)),
    ).optional()?;
    if operation
        .as_ref()
        .is_some_and(|operation| operation.2 != identity)
    {
        return Err(TaskControlError::Conflict(
            "reservation URL does not match task URL",
        ));
    }
    let duplicate: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM rust_download_operations
         WHERE source_identity = ?1 AND task_id != ?2
         AND state IN ('intent', 'sending', 'uncertain', 'unknown', 'submitted'))
         OR EXISTS(SELECT 1 FROM task_mementos WHERE task_id != ?2
         AND state NOT IN ('failed', 'completed', 'cancelled')
         AND json_valid(payload) AND json_extract(payload, '$.release.download_url') = ?3)",
        params![identity, task_id, url],
        |row| row.get(0),
    )?;
    if duplicate {
        return Err(TaskControlError::Conflict(
            "the same URL is already reserved by another task",
        ));
    }
    let checkpoint = &payload["downloader"]["payload"];
    if !payload["downloader"].is_null()
        && (payload["downloader"]["downloader_type"] != "openlist" || !checkpoint.is_object())
    {
        return Err(TaskControlError::Conflict(
            "task has no valid OpenList checkpoint",
        ));
    }
    for field in ["task_id", "workflow_state"] {
        if !checkpoint[field].is_null() && !checkpoint[field].is_string() {
            return Err(TaskControlError::Conflict(
                "task has a malformed remote binding or checkpoint",
            ));
        }
    }
    let payload_binding = remote_binding(checkpoint["task_id"].as_str())?;
    let operation_binding = remote_binding(
        operation
            .as_ref()
            .and_then(|operation| operation.1.as_deref()),
    )?;
    if payload_binding.is_some()
        && operation_binding.is_some()
        && payload_binding != operation_binding
    {
        return Err(TaskControlError::Conflict("remote bindings disagree"));
    }
    let Some(binding) = payload_binding.or(operation_binding) else {
        let rejected = match operation.as_ref() {
            Some(operation) => {
                operation.0 == "rejected"
                    || (operation.0 == "failed" && explicit_rejection(operation.3.as_deref()))
            }
            None => explicit_rejection(payload["retry"]["last_error"].as_str()),
        };
        if !rejected
            || checkpoint["workflow_state"]
                .as_str()
                .is_some_and(|state| state != "init")
        {
            return Err(TaskControlError::Conflict(
                "submission outcome is unknown; resubmission is forbidden",
            ));
        }
        // Keep the failed history/checkpoint until the caller's new submission
        // succeeds. Never clear other tasks' reservations for this URL.
        transaction.execute(
            "DELETE FROM rust_download_operations WHERE task_id = ?1 AND source_identity = ?2
             AND state IN ('failed', 'rejected')
             AND (provider_task_id IS NULL OR trim(provider_task_id) = '')",
            params![task_id, identity],
        )?;
        transaction.commit()?;
        return Ok(RetryTaskResult {
            task,
            action: RetryAction::Resubmit,
        });
    };
    if operation
        .as_ref()
        .is_some_and(|operation| operation.0 == "completed")
    {
        return Err(TaskControlError::Conflict("operation is already completed"));
    }
    let workflow_state = match checkpoint["workflow_state"].as_str() {
        Some("init") => "submitted",
        Some(
            state @ ("submitted" | "download_done" | "transfer_done" | "file_detected"
            | "materializing" | "moved"),
        ) => state,
        Some("file_resolved" | "done") => {
            return Err(TaskControlError::Conflict(
                "legacy file_resolved/done checkpoint cannot be resumed by the Rust worker; manual reconciliation is required",
            ));
        }
        None => {
            if checkpoint["materialized_filename"].as_str().is_some()
                || checkpoint["resolved_filename"].as_str().is_some()
            {
                return Err(TaskControlError::Conflict(
                    "legacy resolved/materialized file has no supported Rust checkpoint; manual reconciliation is required",
                ));
            } else if checkpoint["downloaded_filename"].as_str().is_some() {
                "file_detected"
            } else {
                "submitted"
            }
        }
        _ => {
            return Err(TaskControlError::Conflict(
                "unknown workflow checkpoint; cannot safely resume",
            ));
        }
    }
    .to_string();
    if payload["downloader"].is_null() {
        payload["downloader"] = json!({"downloader_type": "openlist", "payload": {}});
    }
    if payload["retry"].is_null() {
        payload["retry"] = json!({"retry_count": 0, "max_retries": 3, "last_error": null});
    }
    if !payload["retry"].is_object() {
        return Err(TaskControlError::Conflict(
            "task retry payload must be an object",
        ));
    }
    payload["downloader"]["payload"]["task_id"] = json!(binding);
    payload["downloader"]["payload"]["workflow_state"] = json!(workflow_state);
    payload["state"] = json!("downloading");
    payload["archived"] = json!(false);
    payload["retry"]["retry_count"] = json!(0);
    payload["retry"]["last_error"] = Value::Null;
    payload["completed_at"] = Value::Null;
    task.state = "downloading".into();
    persist_task(&transaction, &mut task, &mut payload)?;
    transaction.execute(
        "INSERT INTO rust_download_operations
         (task_id, source_identity, state, provider_task_id, updated_at, last_error)
         VALUES (?1, ?2, 'submitted', ?3, ?4, NULL)
         ON CONFLICT(task_id) DO UPDATE SET state = 'submitted',
         provider_task_id = excluded.provider_task_id, updated_at = excluded.updated_at, last_error = NULL",
        params![task_id, identity, binding, task.updated_at],
    )?;
    transaction.commit()?;
    Ok(RetryTaskResult {
        task,
        action: RetryAction::Resume,
    })
}

fn remote_binding(value: Option<&str>) -> Result<Option<String>, TaskControlError> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if matches!(value.to_ascii_lowercase().as_str(), "unknown" | "uncertain") => {
            Err(TaskControlError::Conflict("remote binding is unknown"))
        }
        value => Ok(value.map(str::to_string)),
    }
}

fn explicit_rejection(message: Option<&str>) -> bool {
    matches!(
        message.map(str::trim),
        Some(
            "OpenList rejected the download request" | "OpenList rejected the operation"
            // Exact legacy provider copyright refusal; generic Python failures
            // also cover timeouts and are deliberately not accepted here.
            | "\u{5e94}\u{7248}\u{6743}\u{65b9}\u{8981}\u{6c42}\u{ff0c}\u{65e0}\u{6cd5}\u{6dfb}\u{52a0}"
        )
    )
}

/// Hide a failed task without deleting any remote binding or reservation.
pub fn archive_failed_task(
    path: impl AsRef<Path>,
    task_id: &str,
) -> Result<LegacyMementoRecord, TaskControlError> {
    let path = path.as_ref();
    let mut connection = open_database(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (mut task, mut payload) = load_failed_task(path, task_id)?;
    payload["archived"] = json!(true);
    persist_task(&transaction, &mut task, &mut payload)?;
    transaction.commit()?;
    Ok(task)
}

fn open_database(path: &Path) -> Result<Connection, TaskControlError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_secs(30))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    Ok(connection)
}

fn load_failed_task(
    path: &Path,
    task_id: &str,
) -> Result<(LegacyMementoRecord, Value), TaskControlError> {
    // The caller holds an IMMEDIATE transaction: another writer cannot change
    // the task while the legacy store opens its read-only connection.
    let task = LegacyTaskMementoStore::new(path)
        .load_all()?
        .into_iter()
        .find(|task| task.task_id == task_id)
        .ok_or(TaskControlError::NotFound)?;
    let payload: Value = serde_json::from_str(&task.payload_json)?;
    if !payload.is_object() {
        return Err(TaskControlError::Conflict("task payload must be an object"));
    }
    if task.state != "failed" || payload["state"] != "failed" {
        return Err(TaskControlError::Conflict(
            "only failed tasks can be controlled",
        ));
    }
    Ok((task, payload))
}

fn persist_task(
    connection: &Connection,
    task: &mut LegacyMementoRecord,
    payload: &mut Value,
) -> Result<(), TaskControlError> {
    task.updated_at =
        connection.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })?;
    payload["updated_at"] = json!(task.updated_at);
    task.payload_json = payload.to_string();
    connection.execute(
        "UPDATE task_mementos SET state = ?2, updated_at = ?3, payload = ?4 WHERE task_id = ?1",
        params![task.task_id, task.state, task.updated_at, task.payload_json],
    )?;
    Ok(())
}
