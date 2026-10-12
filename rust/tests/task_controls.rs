#[allow(dead_code)]
#[path = "../src/storage.rs"]
pub mod storage;

// Keep this independent until the HTTP integration owner registers the module.
#[path = "../src/task_controls.rs"]
mod task_controls;

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use storage::{LegacyMementoRecord, LegacyTaskMementoStore, SubmissionReservation};
use tempfile::{TempDir, tempdir};

struct Fixture {
    directory: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            directory: tempdir().unwrap(),
        };
        fixture
            .connection()
            .execute_batch(
                "CREATE TABLE task_mementos (
                task_id TEXT PRIMARY KEY, state TEXT NOT NULL,
                updated_at TEXT NOT NULL, payload TEXT NOT NULL
            );
            CREATE TABLE task_memento_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
            )
            .unwrap();
        fixture
    }

    fn path(&self) -> std::path::PathBuf {
        self.directory.path().join("task_mementos.db")
    }

    fn connection(&self) -> Connection {
        Connection::open(self.path()).unwrap()
    }

    fn store(&self) -> LegacyTaskMementoStore {
        LegacyTaskMementoStore::new(self.path())
    }

    fn save(&self, id: &str, payload: Value) {
        self.store()
            .save(&LegacyMementoRecord {
                task_id: id.into(),
                state: payload["state"].as_str().unwrap().into(),
                updated_at: "2026-10-12T00:00:00Z".into(),
                payload_json: payload.to_string(),
            })
            .unwrap();
    }

    fn task(&self, id: &str) -> LegacyMementoRecord {
        self.store()
            .load_all()
            .unwrap()
            .into_iter()
            .find(|task| task.task_id == id)
            .unwrap()
    }

    fn operation(
        &self,
        id: &str,
        url: &str,
        state: &str,
        binding: Option<&str>,
        error: Option<&str>,
    ) {
        self.store().load_submission_operations().unwrap();
        self.connection()
            .execute(
                "INSERT INTO rust_download_operations
             (task_id, source_identity, state, provider_task_id, updated_at, last_error)
             VALUES (?1, ?2, ?3, ?4, 'old', ?5)",
                params![
                    id,
                    format!("{:x}", Sha256::digest(url.as_bytes())),
                    state,
                    binding,
                    error
                ],
            )
            .unwrap();
    }
}

fn failed_payload() -> Value {
    let fixtures: Value =
        serde_json::from_str(include_str!("../fixtures/legacy-task-fixtures.json")).unwrap();
    let mut payload = fixtures["cases"][2]["task"].clone();
    payload["state"] = json!("failed");
    payload["retry"]["last_error"] = json!("transfer failed");
    payload
}

#[test]
fn archive_preserves_failed_checkpoint_and_operation() {
    let fixture = Fixture::new();
    let payload = failed_payload();
    fixture.save("failed", payload.clone());
    fixture.store().load_submission_operations().unwrap();
    fixture
        .connection()
        .execute(
            "INSERT INTO rust_download_operations (task_id, source_identity, state, updated_at)
         VALUES ('failed', 'reserved-url', 'uncertain', 'old')",
            [],
        )
        .unwrap();

    let archived = task_controls::archive_failed_task(fixture.path(), "failed").unwrap();
    let actual: Value = serde_json::from_str(&archived.payload_json).unwrap();
    assert_eq!(archived, fixture.task("failed"));
    assert_eq!(archived.state, "failed");
    assert_eq!(actual["archived"], true);
    assert_eq!(actual["downloader"], payload["downloader"]);
    assert_eq!(actual["retry"], payload["retry"]);
    assert_eq!(
        fixture.store().load_submission_operations().unwrap()[0].state,
        "uncertain"
    );
}

#[test]
fn bound_failed_retry_restores_operation_without_resetting_checkpoint() {
    let fixture = Fixture::new();
    let mut payload = failed_payload();
    payload["archived"] = json!(true);
    payload["retry"]["retry_count"] = json!(3);
    fixture.save("failed", payload.clone());
    fixture.operation(
        "failed",
        payload["release"]["download_url"].as_str().unwrap(),
        "failed",
        payload["downloader"]["payload"]["task_id"].as_str(),
        Some("transfer failed"),
    );

    let result = task_controls::retry_failed_task(fixture.path(), "failed").unwrap();
    assert_eq!(result.action, task_controls::RetryAction::Resume);
    assert_eq!(result.task, fixture.task("failed"));
    assert_eq!(result.task.state, "downloading");
    let actual: Value = serde_json::from_str(&result.task.payload_json).unwrap();
    assert_eq!(actual["state"], "downloading");
    assert_eq!(actual["downloader"], payload["downloader"]);
    assert_eq!(actual["pipeline"], payload["pipeline"]);
    assert_eq!(actual["output_path"], payload["output_path"]);
    assert_eq!(actual["archived"], false);
    assert_eq!(actual["retry"]["retry_count"], 0);
    assert!(actual["retry"]["last_error"].is_null());
    let operations = fixture.store().load_submission_operations().unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].state, "submitted");
    assert_eq!(
        operations[0].provider_task_id.as_deref(),
        payload["downloader"]["payload"]["task_id"].as_str()
    );
    assert!(operations[0].last_error.is_none());
}

#[test]
fn explicit_rejection_releases_only_its_reservation_and_allows_new_submission() {
    let fixture = Fixture::new();
    let mut payload = failed_payload();
    payload["downloader"]["payload"]["task_id"] = Value::Null;
    payload["downloader"]["payload"]["workflow_state"] = json!("init");
    payload["retry"]["last_error"] = json!("OpenList rejected the download request");
    let url = payload["release"]["download_url"].as_str().unwrap();
    fixture.save("rejected", payload.clone());
    fixture.operation(
        "rejected",
        url,
        "failed",
        None,
        Some("OpenList rejected the download request"),
    );
    fixture.operation(
        "unrelated",
        "https://example.test/other",
        "uncertain",
        None,
        None,
    );
    let before = fixture.task("rejected");

    let result = task_controls::retry_failed_task(fixture.path(), "rejected").unwrap();
    assert_eq!(result.action, task_controls::RetryAction::Resubmit);
    assert_eq!(result.task, before);
    assert_eq!(fixture.task("rejected"), before);
    let operations = fixture.store().load_submission_operations().unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].task_id, "unrelated");
    assert_eq!(operations[0].state, "uncertain");
    let new_task = LegacyMementoRecord {
        task_id: "new-task".into(),
        state: "pending".into(),
        ..before
    };
    assert_eq!(
        fixture.store().reserve_submission(&new_task, url).unwrap(),
        SubmissionReservation::Created("new-task".into())
    );
}

#[test]
fn unknown_outcomes_return_409_without_releasing_or_modifying_tasks() {
    for (state, error) in [
        ("unknown", None),
        ("uncertain", Some("response lost")),
        ("sending", None),
        ("intent", None),
        ("submitted", None),
        ("failed", Some("Failed to create offline download task")),
    ] {
        let fixture = Fixture::new();
        let mut payload = failed_payload();
        payload["downloader"]["payload"]["task_id"] = Value::Null;
        payload["downloader"]["payload"]["workflow_state"] = json!("init");
        // Payload alone must not override an uncertain operation journal.
        payload["retry"]["last_error"] = json!("OpenList rejected the download request");
        fixture.save("failed", payload.clone());
        fixture.operation(
            "failed",
            payload["release"]["download_url"].as_str().unwrap(),
            state,
            None,
            error,
        );
        let tasks = fixture.store().load_all().unwrap();
        let operations = fixture.store().load_submission_operations().unwrap();

        let error = task_controls::retry_failed_task(fixture.path(), "failed").unwrap_err();
        assert_eq!(error.status_code(), 409, "{state}");
        assert_eq!(fixture.store().load_all().unwrap(), tasks, "{state}");
        assert_eq!(
            fixture.store().load_submission_operations().unwrap(),
            operations,
            "{state}"
        );
    }
}

#[test]
fn legacy_python_generic_submission_failure_is_not_proof_of_rejection() {
    let fixture = Fixture::new();
    let mut payload = failed_payload();
    payload["downloader"]["payload"]["task_id"] = Value::Null;
    payload["downloader"]["payload"]["workflow_state"] = json!("init");
    payload["retry"]["last_error"] = json!("Failed to create offline download task");
    fixture.save("legacy", payload);
    let before = fixture.task("legacy");
    let error = task_controls::retry_failed_task(fixture.path(), "legacy").unwrap_err();
    assert_eq!(error.status_code(), 409);
    assert_eq!(fixture.task("legacy"), before);
    assert!(
        fixture
            .store()
            .load_submission_operations()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn legacy_python_supported_checkpoints_resume_and_unsupported_ones_conflict() {
    let cases: Value =
        serde_json::from_str(include_str!("../fixtures/legacy-task-fixtures.json")).unwrap();
    for case in cases["cases"].as_array().unwrap() {
        let fixture = Fixture::new();
        let mut payload = case["task"].clone();
        payload["state"] = json!("failed");
        payload["downloader"]["payload"]["task_id"] = json!("legacy-remote");
        fixture.save("legacy", payload.clone());
        if matches!(case["phase"].as_str(), Some("file_resolved" | "done")) {
            let before = fixture.task("legacy");
            let error = task_controls::retry_failed_task(fixture.path(), "legacy").unwrap_err();
            assert_eq!(error.status_code(), 409);
            assert!(error.to_string().contains("manual reconciliation"));
            assert_eq!(fixture.task("legacy"), before);
            assert!(
                fixture
                    .store()
                    .load_submission_operations()
                    .unwrap()
                    .is_empty()
            );
            continue;
        }
        let result = task_controls::retry_failed_task(fixture.path(), "legacy").unwrap();
        assert_eq!(result.action, task_controls::RetryAction::Resume);
        let actual: Value = serde_json::from_str(&result.task.payload_json).unwrap();
        let mut expected = payload["downloader"].clone();
        if case["phase"] == "init" {
            expected["payload"]["workflow_state"] = json!("submitted");
        }
        assert_eq!(actual["downloader"], expected, "{}", case["phase"]);
        assert_eq!(actual["release"], payload["release"]);
        assert_eq!(actual["pipeline"], payload["pipeline"]);
        assert_eq!(
            fixture.store().load_submission_operations().unwrap()[0].state,
            "submitted"
        );
    }
}

#[test]
fn operation_only_binding_repairs_missing_payload_binding_without_submitting() {
    let fixture = Fixture::new();
    let mut payload = failed_payload();
    payload["downloader"]["payload"]
        .as_object_mut()
        .unwrap()
        .remove("task_id");
    payload["downloader"]["payload"]["workflow_state"] = json!("init");
    fixture.save("failed", payload.clone());
    fixture.operation(
        "failed",
        payload["release"]["download_url"].as_str().unwrap(),
        "uncertain",
        Some("known-remote"),
        Some("response lost"),
    );
    let result = task_controls::retry_failed_task(fixture.path(), "failed").unwrap();
    assert_eq!(result.action, task_controls::RetryAction::Resume);
    let actual: Value = serde_json::from_str(&result.task.payload_json).unwrap();
    assert_eq!(actual["downloader"]["payload"]["task_id"], "known-remote");
    assert_eq!(
        actual["downloader"]["payload"]["workflow_state"],
        "submitted"
    );
    assert_eq!(
        actual["downloader"]["payload"]["temp_path"],
        payload["downloader"]["payload"]["temp_path"]
    );
}

#[test]
fn missing_legacy_checkpoint_is_inferred_only_when_supported_by_rust_worker() {
    for (field, phase) in [
        ("none", "submitted"),
        ("downloaded_filename", "file_detected"),
        ("resolved_filename", "file_resolved"),
        ("materialized_filename", "done"),
    ] {
        let fixture = Fixture::new();
        let mut payload = failed_payload();
        payload["downloader"]["payload"]
            .as_object_mut()
            .unwrap()
            .remove("workflow_state");
        if field != "none" {
            payload["downloader"]["payload"][field] = json!("video.mkv");
        }
        fixture.save("legacy", payload);
        if matches!(phase, "file_resolved" | "done") {
            let before = fixture.task("legacy");
            let error = task_controls::retry_failed_task(fixture.path(), "legacy").unwrap_err();
            assert_eq!(error.status_code(), 409);
            assert!(error.to_string().contains("manual reconciliation"));
            assert_eq!(fixture.task("legacy"), before);
            assert!(
                fixture
                    .store()
                    .load_submission_operations()
                    .unwrap()
                    .is_empty()
            );
            continue;
        }
        let result = task_controls::retry_failed_task(fixture.path(), "legacy").unwrap();
        let actual: Value = serde_json::from_str(&result.task.payload_json).unwrap();
        assert_eq!(actual["downloader"]["payload"]["workflow_state"], phase);
    }
}

#[test]
fn missing_and_nonfailed_tasks_return_404_and_409() {
    let fixture = Fixture::new();
    assert_eq!(
        task_controls::archive_failed_task(fixture.path(), "absent")
            .unwrap_err()
            .status_code(),
        404
    );
    assert_eq!(
        task_controls::retry_failed_task(fixture.path(), "absent")
            .unwrap_err()
            .status_code(),
        404
    );
    let mut payload = failed_payload();
    payload["state"] = json!("downloading");
    fixture.save("active", payload);
    let before = fixture.task("active");
    assert_eq!(
        task_controls::archive_failed_task(fixture.path(), "active")
            .unwrap_err()
            .status_code(),
        409
    );
    assert_eq!(
        task_controls::retry_failed_task(fixture.path(), "active")
            .unwrap_err()
            .status_code(),
        409
    );
    assert_eq!(fixture.task("active"), before);
}

#[test]
fn archive_is_idempotent_and_does_not_create_operations_for_old_python_tasks() {
    let fixture = Fixture::new();
    fixture.save("legacy", failed_payload());
    task_controls::archive_failed_task(fixture.path(), "legacy").unwrap();
    let result = task_controls::archive_failed_task(fixture.path(), "legacy").unwrap();
    let actual: Value = serde_json::from_str(&result.payload_json).unwrap();
    assert_eq!(actual["archived"], true);
    let exists: bool = fixture
        .connection()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'rust_download_operations')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!exists);
}

#[test]
fn conflicting_bindings_url_or_checkpoint_return_409_without_writes() {
    for problem in ["binding", "url", "checkpoint", "unknown-id", "completed"] {
        let fixture = Fixture::new();
        let mut payload = failed_payload();
        if problem == "checkpoint" {
            payload["downloader"]["payload"]["workflow_state"] = json!("unknown");
        }
        if problem == "unknown-id" {
            payload["downloader"]["payload"]["task_id"] = json!("unknown");
        }
        fixture.save("failed", payload.clone());
        fixture.operation(
            "failed",
            if problem == "url" {
                "https://example.test/wrong"
            } else {
                payload["release"]["download_url"].as_str().unwrap()
            },
            if problem == "completed" {
                "completed"
            } else {
                "failed"
            },
            if problem == "binding" {
                Some("different-remote")
            } else {
                payload["downloader"]["payload"]["task_id"].as_str()
            },
            None,
        );
        let before = fixture.task("failed");
        let operations = fixture.store().load_submission_operations().unwrap();
        assert_eq!(
            task_controls::retry_failed_task(fixture.path(), "failed")
                .unwrap_err()
                .status_code(),
            409,
            "{problem}"
        );
        assert_eq!(fixture.task("failed"), before);
        assert_eq!(
            fixture.store().load_submission_operations().unwrap(),
            operations
        );
    }
}

#[test]
fn competing_url_reservation_blocks_both_resume_and_resubmit() {
    for bound in [false, true] {
        let fixture = Fixture::new();
        let mut payload = failed_payload();
        if !bound {
            payload["downloader"]["payload"]["task_id"] = Value::Null;
            payload["downloader"]["payload"]["workflow_state"] = json!("init");
        }
        let url = payload["release"]["download_url"].as_str().unwrap();
        fixture.save("failed", payload.clone());
        fixture.operation(
            "failed",
            url,
            "failed",
            payload["downloader"]["payload"]["task_id"].as_str(),
            Some("OpenList rejected the download request"),
        );
        fixture.operation("other", url, "uncertain", None, Some("unknown"));
        let before = fixture.task("failed");
        let operations = fixture.store().load_submission_operations().unwrap();
        assert_eq!(
            task_controls::retry_failed_task(fixture.path(), "failed")
                .unwrap_err()
                .status_code(),
            409
        );
        assert_eq!(fixture.task("failed"), before);
        assert_eq!(
            fixture.store().load_submission_operations().unwrap(),
            operations
        );
    }
}

#[test]
fn operation_update_failure_rolls_back_payload_resume() {
    let fixture = Fixture::new();
    let payload = failed_payload();
    fixture.save("failed", payload.clone());
    fixture.operation(
        "failed",
        payload["release"]["download_url"].as_str().unwrap(),
        "failed",
        payload["downloader"]["payload"]["task_id"].as_str(),
        None,
    );
    let before = fixture.task("failed");
    let operations = fixture.store().load_submission_operations().unwrap();
    fixture
        .connection()
        .execute_batch(
            "CREATE TRIGGER refuse_resume BEFORE UPDATE ON rust_download_operations
         BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;",
        )
        .unwrap();
    assert_eq!(
        task_controls::retry_failed_task(fixture.path(), "failed")
            .unwrap_err()
            .status_code(),
        500
    );
    assert_eq!(fixture.task("failed"), before);
    assert_eq!(
        fixture.store().load_submission_operations().unwrap(),
        operations
    );
}

#[test]
fn legacy_active_task_with_same_url_blocks_resubmission() {
    let fixture = Fixture::new();
    let mut payload = failed_payload();
    payload["downloader"] = Value::Null;
    payload["retry"]["last_error"] = json!("OpenList rejected the download request");
    fixture.save("failed", payload.clone());
    let mut active = payload;
    active["state"] = json!("pending");
    fixture.save("legacy-active", active);
    let before = fixture.store().load_all().unwrap();
    assert_eq!(
        task_controls::retry_failed_task(fixture.path(), "failed")
            .unwrap_err()
            .status_code(),
        409
    );
    assert_eq!(fixture.store().load_all().unwrap(), before);
}

#[test]
fn malformed_checkpoint_or_binding_cannot_be_treated_as_absent() {
    for field in ["workflow_state", "task_id"] {
        let fixture = Fixture::new();
        let mut payload = failed_payload();
        payload["downloader"]["payload"][field] = json!(123);
        if field == "task_id" {
            payload["downloader"]["payload"]["workflow_state"] = json!("init");
            payload["retry"]["last_error"] = json!("OpenList rejected the download request");
        }
        fixture.save("failed", payload);
        let before = fixture.task("failed");
        assert_eq!(
            task_controls::retry_failed_task(fixture.path(), "failed")
                .unwrap_err()
                .status_code(),
            409
        );
        assert_eq!(fixture.task("failed"), before);
    }
}

#[test]
fn legacy_explicit_refusal_and_rejected_journal_allow_resubmit() {
    for legacy in [false, true] {
        let fixture = Fixture::new();
        let mut payload = failed_payload();
        payload["downloader"] = Value::Null;
        payload["retry"]["last_error"] = json!(
            "\u{5e94}\u{7248}\u{6743}\u{65b9}\u{8981}\u{6c42}\u{ff0c}\u{65e0}\u{6cd5}\u{6dfb}\u{52a0}"
        );
        fixture.save("failed", payload.clone());
        if !legacy {
            fixture.operation(
                "failed",
                payload["release"]["download_url"].as_str().unwrap(),
                "rejected",
                None,
                None,
            );
        }
        let before = fixture.task("failed");
        let result = task_controls::retry_failed_task(fixture.path(), "failed").unwrap();
        assert_eq!(result.action, task_controls::RetryAction::Resubmit);
        assert_eq!(result.task, before);
        assert!(
            fixture
                .store()
                .load_submission_operations()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn concurrent_bound_retries_allow_exactly_one_resume() {
    for wal in [false, true] {
        let fixture = Fixture::new();
        if wal {
            fixture
                .connection()
                .pragma_update(None, "journal_mode", "WAL")
                .unwrap();
        }
        fixture.save("failed", failed_payload());
        fixture.store().load_submission_operations().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let path = fixture.path();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    task_controls::retry_failed_task(path, "failed")
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        for result in results {
            match result {
                Ok(result) => assert_eq!(result.action, task_controls::RetryAction::Resume),
                Err(error) => assert_eq!(error.status_code(), 409),
            }
        }
        assert_eq!(fixture.task("failed").state, "downloading");
        assert_eq!(
            fixture.store().load_submission_operations().unwrap().len(),
            1
        );
    }
}
