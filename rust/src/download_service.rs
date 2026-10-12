use std::time::Duration;

use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    config_store::DownloadRuntimeSettings,
    download_workflow::{
        DownloadWorkflowError, MaterializationPlan, detect_video_file, execute_materialization,
        prepare_materialization,
    },
    naming::{DownloadPlan, NamingInput, build_plan},
    openlist::{OfflineTask, OpenListClient, OpenListError},
    release_parser::parse_release_title,
    storage::{
        LegacyDbPaths, LegacyMementoRecord, LegacyResourceInput, LegacyResourceStore,
        LegacyStorageError, LegacyTaskMementoStore, SubmissionReservation,
    },
};

const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(5);
const MAX_MISSING_REMOTE_CHECKS: usize = 6;
const FILE_DETECTION_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSFER_MISSING_CHECKS: usize = 3;

#[derive(Debug, Error)]
pub enum DownloadServiceError {
    #[error("download URL must be a magnet or HTTP(S) URL")]
    InvalidUrl,
    #[error("release title does not contain a recognizable season and episode")]
    InvalidTitle,
    #[error("download already exists as task {0}")]
    Duplicate(String),
    #[error("OpenList rejected the download request")]
    Rejected,
    #[error(
        "OpenList accepted the request but returned no task ID; it will not be retried automatically"
    )]
    Uncertain,
    #[error("download configuration is invalid")]
    InvalidConfiguration,
    #[error(transparent)]
    OpenList(#[from] OpenListError),
    #[error(transparent)]
    Workflow(#[from] DownloadWorkflowError),
    #[error(transparent)]
    Storage(#[from] LegacyStorageError),
    #[error("blocking storage operation failed")]
    StorageWorker,
    #[error("download plan is invalid")]
    InvalidPlan,
    #[error("download task no longer exists")]
    TaskNotFound,
}

#[derive(Clone, Debug)]
pub struct SubmittedDownload {
    pub task_id: String,
    pub state: String,
    pub payload_json: String,
}

#[derive(Clone, Debug, Default)]
pub struct DownloadOverrides {
    pub source_url: Option<String>,
    pub anime_name: Option<String>,
    pub directory_name: Option<String>,
    pub episode_offset: i64,
    pub season: Option<i64>,
    pub filename_season: Option<i64>,
    pub directory_season: Option<i64>,
}

pub async fn submit_download(
    databases: LegacyDbPaths,
    settings: DownloadRuntimeSettings,
    title: &str,
    download_url: &str,
) -> Result<SubmittedDownload, DownloadServiceError> {
    submit_download_with_overrides(
        databases,
        settings,
        title,
        download_url,
        DownloadOverrides::default(),
    )
    .await
}

pub async fn submit_download_with_overrides(
    databases: LegacyDbPaths,
    settings: DownloadRuntimeSettings,
    title: &str,
    download_url: &str,
    overrides: DownloadOverrides,
) -> Result<SubmittedDownload, DownloadServiceError> {
    let title = title.trim();
    let download_url = download_url.trim();
    if !valid_download_url(download_url) {
        return Err(DownloadServiceError::InvalidUrl);
    }
    let parsed = apply_release_overrides(
        parse_release_title(title).ok_or(DownloadServiceError::InvalidTitle)?,
        &overrides,
    )?;
    let plan = build_download_plan(&settings, &parsed, ".mp4", &overrides)?;
    let client = OpenListClient::new(&settings.openlist_url, &settings.token)
        .map_err(|_| DownloadServiceError::InvalidConfiguration)?;
    let task_id = Uuid::new_v4().to_string();
    let now = now_rfc3339();
    let temporary_root = join_path(
        &join_path(&settings.download_path, ".oani-download-tmp"),
        &task_id,
    );
    let payload = json!({
        "schema_version": 1,
        "task_id": task_id,
        "state": "pending",
        "release": {
            "title": title,
            "download_url": download_url,
            "source_url": overrides.source_url,
            "anime_name_override": overrides.anime_name,
            "download_directory_name_override": overrides.directory_name,
            "episode_offset": overrides.episode_offset,
            "anime_name": parsed.anime_name,
            "season": parsed.season,
            "season_explicit": parsed.explicit_season,
            "filename_season_override": overrides.filename_season,
            "directory_season_override": overrides.directory_season,
            "episode": parsed.episode,
            "fansub": parsed.fansub,
            "quality": parsed.quality,
            "languages": parsed.languages,
            "version": 1
        },
        "base_path": settings.download_path,
        "target_directory": plan.directory,
        "rename_format": settings.rename_format,
        "downloader": {
            "downloader_type": "openlist",
            "payload": {
                "temp_path": temporary_root,
                "workflow_state": "init"
            }
        },
        "pipeline": {
            "next_buffer": "download",
            "downloaded_directory_path": null,
            "downloaded_filename": null,
            "renamed_path": null
        },
        "retry": {"retry_count": 0, "max_retries": 3, "last_error": null},
        "output_path": null,
        "created_at": now,
        "updated_at": now,
        "started_at": null,
        "completed_at": null,
        "archived": false
    });
    let record = LegacyMementoRecord {
        task_id: task_id.clone(),
        state: "pending".to_string(),
        updated_at: now.clone(),
        payload_json: payload.to_string(),
    };
    let reservation = db_call({
        let path = databases.task_mementos_db.clone();
        let record = record.clone();
        let download_url = download_url.to_string();
        move || LegacyTaskMementoStore::new(path).reserve_submission(&record, &download_url)
    })
    .await?;
    if let SubmissionReservation::Existing(existing_id) = reservation {
        return Err(DownloadServiceError::Duplicate(existing_id));
    }

    let began = db_call({
        let path = databases.task_mementos_db.clone();
        let task_id = task_id.clone();
        move || LegacyTaskMementoStore::new(path).begin_submission(&task_id)
    })
    .await?;
    if !began {
        return Err(DownloadServiceError::Uncertain);
    }

    let download_name = format!(
        "{} S{:02}E{:02}",
        plan.display_name, parsed.season, parsed.episode
    );
    let remote_tasks = client
        .add_offline_download(
            &[download_url.to_string()],
            &temporary_root,
            &settings.offline_download_tool,
            &download_name,
        )
        .await;
    let remote_task = match remote_tasks {
        Ok(tasks) => tasks.into_iter().next().filter(|task| !task.id.is_empty()),
        Err(OpenListError::Rejected) => {
            fail_submission(&databases, record, "OpenList rejected the download request").await?;
            return Err(DownloadServiceError::Rejected);
        }
        Err(_) => {
            mark_submission_uncertain(&databases, record, "OpenList submission outcome is unknown")
                .await?;
            return Err(DownloadServiceError::Uncertain);
        }
    };
    let Some(remote_task) = remote_task else {
        mark_submission_uncertain(
            &databases,
            record,
            "OpenList response did not contain a task ID",
        )
        .await?;
        return Err(DownloadServiceError::Uncertain);
    };

    let now = now_rfc3339();
    let accepted = db_call({
        let path = databases.task_mementos_db.clone();
        let task_id = task_id.clone();
        let provider_id = remote_task.id.clone();
        let now = now.clone();
        move || {
            LegacyTaskMementoStore::new(path).mark_submission_accepted(&task_id, &provider_id, &now)
        }
    })
    .await?;
    if !accepted {
        return Err(DownloadServiceError::Uncertain);
    }
    let mut payload = payload;
    payload["state"] = json!("downloading");
    payload["started_at"] = json!(now);
    payload["updated_at"] = json!(now);
    payload["downloader"]["payload"]["task_id"] = json!(remote_task.id);
    payload["downloader"]["payload"]["workflow_state"] = json!("submitted");
    let accepted_record = LegacyMementoRecord {
        task_id: task_id.clone(),
        state: "downloading".to_string(),
        updated_at: now,
        payload_json: payload.to_string(),
    };
    save_record(&databases, accepted_record.clone()).await?;

    Ok(SubmittedDownload {
        task_id,
        state: accepted_record.state,
        payload_json: accepted_record.payload_json,
    })
}

pub async fn recover_submitted_tasks(
    databases: LegacyDbPaths,
) -> Result<Vec<String>, DownloadServiceError> {
    db_call({
        let path = databases.task_mementos_db.clone();
        move || LegacyTaskMementoStore::new(path).recover_interrupted_submissions()
    })
    .await?;
    let operations = db_call({
        let path = databases.task_mementos_db.clone();
        move || LegacyTaskMementoStore::new(path).load_submission_operations()
    })
    .await?;
    let records = db_call({
        let path = databases.task_mementos_db.clone();
        move || LegacyTaskMementoStore::new(path).load_all()
    })
    .await?;
    let mut resumable = Vec::new();
    for mut record in records {
        let Some(operation) = operations
            .iter()
            .find(|item| item.task_id == record.task_id)
        else {
            continue;
        };
        if operation.state != "submitted" {
            continue;
        }
        let Ok(mut payload) = serde_json::from_str::<Value>(&record.payload_json) else {
            continue;
        };
        if payload["downloader"]["payload"]["task_id"]
            .as_str()
            .is_none()
        {
            let Some(provider_task_id) = operation.provider_task_id.as_deref() else {
                continue;
            };
            payload["downloader"]["payload"]["task_id"] = json!(provider_task_id);
            if payload["downloader"]["payload"]["workflow_state"]
                .as_str()
                .is_none_or(|state| state == "init")
            {
                payload["downloader"]["payload"]["workflow_state"] = json!("submitted");
            }
        }
        payload["state"] = json!("downloading");
        let now = now_rfc3339();
        payload["updated_at"] = json!(now);
        record.state = "downloading".to_string();
        record.updated_at = now;
        record.payload_json = payload.to_string();
        save_record(&databases, record.clone()).await?;
        resumable.push(record.task_id);
    }
    Ok(resumable)
}

pub async fn run_download_worker(
    databases: LegacyDbPaths,
    settings: DownloadRuntimeSettings,
    task_id: String,
) {
    loop {
        match run_download_worker_inner(&databases, &settings, &task_id).await {
            Ok(()) => return,
            Err(error) if retryable(&error) => {
                let _ = mark_task_waiting(&databases, &task_id, &error.to_string()).await;
                tokio::time::sleep(WORKER_POLL_INTERVAL).await;
            }
            Err(error) => {
                let _ = fail_task(&databases, &task_id, &error.to_string()).await;
                return;
            }
        }
    }
}

fn retryable(error: &DownloadServiceError) -> bool {
    matches!(
        error,
        DownloadServiceError::Uncertain
            | DownloadServiceError::InvalidConfiguration
            | DownloadServiceError::OpenList(_)
            | DownloadServiceError::Storage(_)
            | DownloadServiceError::StorageWorker
            | DownloadServiceError::Workflow(DownloadWorkflowError::OpenList(_))
    )
}

async fn run_download_worker_inner(
    databases: &LegacyDbPaths,
    settings: &DownloadRuntimeSettings,
    task_id: &str,
) -> Result<(), DownloadServiceError> {
    let mut record = load_task(databases, task_id).await?;
    let mut payload: Value = serde_json::from_str(&record.payload_json)
        .map_err(|_| DownloadServiceError::InvalidConfiguration)?;
    let client = OpenListClient::new(&settings.openlist_url, &settings.token)
        .map_err(|_| DownloadServiceError::InvalidConfiguration)?;
    let remote_id = payload["downloader"]["payload"]["task_id"]
        .as_str()
        .ok_or(DownloadServiceError::Uncertain)?
        .to_string();
    let temporary_root = payload["downloader"]["payload"]["temp_path"]
        .as_str()
        .ok_or(DownloadServiceError::Uncertain)?
        .to_string();

    let workflow_state = payload["downloader"]["payload"]["workflow_state"]
        .as_str()
        .unwrap_or("submitted")
        .to_string();
    if workflow_state == "submitted" || workflow_state == "init" {
        wait_download_complete(databases, &mut record, &mut payload, &client, &remote_id).await?;
        payload["downloader"]["payload"]["workflow_state"] = json!("download_done");
        checkpoint(databases, &mut record, &mut payload).await?;
    }

    let workflow_state = payload["downloader"]["payload"]["workflow_state"]
        .as_str()
        .unwrap_or("download_done")
        .to_string();
    if workflow_state == "download_done" {
        wait_transfer_complete(&client, &remote_id).await?;
        payload["downloader"]["payload"]["workflow_state"] = json!("transfer_done");
        checkpoint(databases, &mut record, &mut payload).await?;
    }

    let workflow_state = payload["downloader"]["payload"]["workflow_state"]
        .as_str()
        .unwrap_or("transfer_done")
        .to_string();
    if workflow_state == "transfer_done" {
        let detected = detect_file_until_found(&client, &temporary_root).await?;
        payload["downloader"]["payload"]["downloaded_filename"] = json!(detected);
        payload["downloader"]["payload"]["workflow_state"] = json!("file_detected");
        checkpoint(databases, &mut record, &mut payload).await?;
    }

    let workflow_state = payload["downloader"]["payload"]["workflow_state"]
        .as_str()
        .unwrap_or("file_detected")
        .to_string();
    if workflow_state == "file_detected" {
        let detected = payload["downloader"]["payload"]["downloaded_filename"]
            .as_str()
            .ok_or(DownloadServiceError::InvalidConfiguration)?;
        let extension = detected
            .rsplit('/')
            .next()
            .and_then(|name| name.rfind('.').map(|index| &name[index..]))
            .ok_or(DownloadServiceError::InvalidPlan)?;
        let parsed = release_from_payload(&payload)?;
        let frozen_settings = DownloadRuntimeSettings {
            download_path: required_string(&payload, "base_path")?,
            rename_format: required_string(&payload, "rename_format")?,
            ..settings.clone()
        };
        let overrides = download_overrides_from_payload(&payload["release"]);
        let plan = build_download_plan(&frozen_settings, &parsed, extension, &overrides)?;
        let materialization = prepare_materialization(
            &client,
            &frozen_settings.download_path,
            &temporary_root,
            payload["target_directory"]
                .as_str()
                .ok_or(DownloadServiceError::InvalidPlan)?,
            detected,
            &plan.filename,
        )
        .await?;
        payload["downloader"]["payload"]["materialization_plan"] =
            serde_json::to_value(materialization).map_err(|_| DownloadServiceError::InvalidPlan)?;
        payload["downloader"]["payload"]["workflow_state"] = json!("materializing");
        payload["final_filename"] = json!(plan.filename);
        checkpoint(databases, &mut record, &mut payload).await?;
    }

    let workflow_state = payload["downloader"]["payload"]["workflow_state"]
        .as_str()
        .unwrap_or("materializing")
        .to_string();
    if workflow_state == "materializing" {
        let materialization: MaterializationPlan = serde_json::from_value(
            payload["downloader"]["payload"]["materialization_plan"].clone(),
        )
        .map_err(|_| DownloadServiceError::InvalidPlan)?;
        let materialized =
            execute_materialization(&client, &temporary_root, &materialization).await?;
        payload["output_path"] = json!(materialized.path);
        payload["downloader"]["payload"]["materialized_filename"] =
            json!(materialization.final_filename);
        payload["downloader"]["payload"]["materialized_directory_path"] =
            json!(materialization.target_directory);
        payload["downloader"]["payload"]["workflow_state"] = json!("moved");
        checkpoint(databases, &mut record, &mut payload).await?;
    }

    if payload["downloader"]["payload"]["workflow_state"] == "moved" {
        commit_resource(databases, &payload).await?;
        let now = now_rfc3339();
        let completed = db_call({
            let path = databases.task_mementos_db.clone();
            let task_id = task_id.to_string();
            let now = now.clone();
            move || LegacyTaskMementoStore::new(path).mark_submission_completed(&task_id, &now)
        })
        .await?;
        if !completed {
            return Err(DownloadServiceError::Uncertain);
        }
    }
    Ok(())
}

async fn wait_download_complete(
    databases: &LegacyDbPaths,
    record: &mut LegacyMementoRecord,
    payload: &mut Value,
    client: &OpenListClient,
    remote_id: &str,
) -> Result<(), DownloadServiceError> {
    let mut missing = 0;
    loop {
        let undone = match client.offline_download_tasks(false).await {
            Ok(tasks) => tasks,
            Err(_) => {
                tokio::time::sleep(WORKER_POLL_INTERVAL).await;
                continue;
            }
        };
        if let Some(task) = undone.iter().find(|task| task.id == remote_id) {
            missing = 0;
            if remote_failed(task) {
                return Err(DownloadServiceError::Rejected);
            }
            if task.progress.is_some() {
                payload["downloader"]["payload"]["progress"] = task.progress.clone().unwrap();
                checkpoint(databases, record, payload).await?;
            }
            if task_succeeded(task) {
                return Ok(());
            }
            tokio::time::sleep(WORKER_POLL_INTERVAL).await;
            continue;
        }

        if let Ok(done) = client.offline_download_tasks(true).await {
            if let Some(task) = done.iter().find(|task| task.id == remote_id) {
                if task_succeeded(task) {
                    return Ok(());
                }
                if remote_failed(task) {
                    return Err(DownloadServiceError::Rejected);
                }
            } else {
                missing += 1;
            }
        }
        if missing >= MAX_MISSING_REMOTE_CHECKS {
            return Err(DownloadServiceError::Uncertain);
        }
        tokio::time::sleep(WORKER_POLL_INTERVAL).await;
    }
}

async fn wait_transfer_complete(
    client: &OpenListClient,
    remote_id: &str,
) -> Result<(), DownloadServiceError> {
    let mut missing = 0;
    loop {
        let undone = client.offline_transfer_tasks(false).await?;
        let matching = undone.iter().find(|task| task.name.contains(remote_id));
        if let Some(task) = matching {
            missing = 0;
            if remote_failed(task) {
                return Err(DownloadServiceError::Rejected);
            }
            if task_succeeded(task) {
                return Ok(());
            }
            tokio::time::sleep(WORKER_POLL_INTERVAL).await;
            continue;
        }

        let done = client.offline_transfer_tasks(true).await?;
        if let Some(task) = done.iter().find(|task| task.name.contains(remote_id)) {
            return if task_succeeded(task) {
                Ok(())
            } else {
                Err(DownloadServiceError::Rejected)
            };
        }
        missing += 1;
        if missing >= TRANSFER_MISSING_CHECKS {
            return Ok(());
        }
        tokio::time::sleep(WORKER_POLL_INTERVAL).await;
    }
}

async fn detect_file_until_found(
    client: &OpenListClient,
    temporary_root: &str,
) -> Result<String, DownloadServiceError> {
    let deadline = tokio::time::Instant::now() + FILE_DETECTION_TIMEOUT;
    loop {
        if let Some(path) = detect_video_file(client, temporary_root).await? {
            return Ok(path);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(DownloadServiceError::InvalidPlan);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn task_succeeded(task: &OfflineTask) -> bool {
    task.state.as_i64() == Some(2)
        || task
            .state
            .as_str()
            .is_some_and(|state| state.eq_ignore_ascii_case("succeeded"))
}

fn remote_failed(task: &OfflineTask) -> bool {
    matches!(task.state.as_i64(), Some(3..=7))
        || task.state.as_str().is_some_and(|state| {
            matches!(
                state.to_ascii_lowercase().as_str(),
                "canceled" | "cancelled" | "errored" | "failing" | "failed"
            )
        })
}

async fn checkpoint(
    databases: &LegacyDbPaths,
    record: &mut LegacyMementoRecord,
    payload: &mut Value,
) -> Result<(), DownloadServiceError> {
    let now = now_rfc3339();
    payload["updated_at"] = json!(now);
    payload["retry"]["last_error"] = Value::Null;
    record.updated_at = now;
    record.payload_json = payload.to_string();
    save_record(databases, record.clone()).await
}

async fn mark_task_waiting(
    databases: &LegacyDbPaths,
    task_id: &str,
    message: &str,
) -> Result<(), DownloadServiceError> {
    let mut record = load_task(databases, task_id).await?;
    let mut payload: Value = serde_json::from_str(&record.payload_json)
        .map_err(|_| DownloadServiceError::InvalidConfiguration)?;
    let now = now_rfc3339();
    payload["retry"]["last_error"] = json!(message);
    payload["updated_at"] = json!(now);
    record.updated_at = now;
    record.payload_json = payload.to_string();
    save_record(databases, record).await
}

async fn commit_resource(
    databases: &LegacyDbPaths,
    payload: &Value,
) -> Result<(), DownloadServiceError> {
    let release = &payload["release"];
    let input = LegacyResourceInput {
        url: required_string(release, "download_url")?,
        title: required_string(release, "title")?,
        anime_name: release["anime_name"].as_str().map(str::to_string),
        season: release["season"].as_i64(),
        episode: release["episode"].as_i64(),
        fansub: release["fansub"].as_str().map(str::to_string),
        quality: release["quality"].as_str().map(str::to_string),
        languages: release["languages"]
            .as_array()
            .map(|languages| {
                languages
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("")
            })
            .filter(|value| !value.is_empty()),
        version: release["version"].as_i64(),
        downloaded_at: None,
        episode_name: None,
    };
    db_call({
        let path = databases.data_db.clone();
        move || LegacyResourceStore::new(path).insert_ignore_duplicate(&input)
    })
    .await?;
    Ok(())
}

async fn fail_submission(
    databases: &LegacyDbPaths,
    mut record: LegacyMementoRecord,
    message: &str,
) -> Result<(), DownloadServiceError> {
    set_task_error(&mut record, message);
    save_record(databases, record.clone()).await?;
    db_call({
        let path = databases.task_mementos_db.clone();
        let task_id = record.task_id;
        let message = message.to_string();
        let now = record.updated_at;
        move || LegacyTaskMementoStore::new(path).mark_submission_failed(&task_id, &message, &now)
    })
    .await?;
    Ok(())
}

async fn mark_submission_uncertain(
    databases: &LegacyDbPaths,
    mut record: LegacyMementoRecord,
    message: &str,
) -> Result<(), DownloadServiceError> {
    let now = now_rfc3339();
    let _ = db_call({
        let path = databases.task_mementos_db.clone();
        let task_id = record.task_id.clone();
        let message = message.to_string();
        let now = now.clone();
        move || {
            LegacyTaskMementoStore::new(path).mark_submission_uncertain(&task_id, &message, &now)
        }
    })
    .await?;
    set_task_error(&mut record, message);
    save_record(databases, record).await
}

async fn fail_task(
    databases: &LegacyDbPaths,
    task_id: &str,
    message: &str,
) -> Result<(), DownloadServiceError> {
    let Ok(mut record) = load_task(databases, task_id).await else {
        return Ok(());
    };
    set_task_error(&mut record, message);
    let state = record.state.clone();
    let updated = record.updated_at.clone();
    save_record(databases, record).await?;
    if state == "downloading" {
        let _ = db_call({
            let path = databases.task_mementos_db.clone();
            let task_id = task_id.to_string();
            let message = message.to_string();
            move || {
                LegacyTaskMementoStore::new(path)
                    .mark_submission_failed(&task_id, &message, &updated)
            }
        })
        .await;
    }
    Ok(())
}

fn set_task_error(record: &mut LegacyMementoRecord, message: &str) {
    if let Ok(mut payload) = serde_json::from_str::<Value>(&record.payload_json) {
        payload["state"] = json!("failed");
        payload["retry"]["last_error"] = json!(message);
        payload["updated_at"] = json!(now_rfc3339());
        record.state = "failed".to_string();
        record.updated_at = payload["updated_at"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        record.payload_json = payload.to_string();
    }
}

async fn load_task(
    databases: &LegacyDbPaths,
    task_id: &str,
) -> Result<LegacyMementoRecord, DownloadServiceError> {
    db_call({
        let path = databases.task_mementos_db.clone();
        let task_id = task_id.to_string();
        move || {
            Ok(LegacyTaskMementoStore::new(path)
                .load_all()?
                .into_iter()
                .find(|record| record.task_id == task_id))
        }
    })
    .await?
    .ok_or(DownloadServiceError::TaskNotFound)
}

async fn save_record(
    databases: &LegacyDbPaths,
    record: LegacyMementoRecord,
) -> Result<(), DownloadServiceError> {
    db_call({
        let path = databases.task_mementos_db.clone();
        move || LegacyTaskMementoStore::new(path).save(&record)
    })
    .await
}

async fn db_call<T, F>(operation: F) -> Result<T, DownloadServiceError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, LegacyStorageError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| DownloadServiceError::StorageWorker)?
        .map_err(Into::into)
}

fn release_from_payload(
    payload: &Value,
) -> Result<crate::release_parser::ParsedReleaseTitle, DownloadServiceError> {
    let release = &payload["release"];
    Ok(crate::release_parser::ParsedReleaseTitle {
        anime_name: required_string(release, "anime_name")?,
        season: release["season"]
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or(DownloadServiceError::InvalidPlan)?,
        explicit_season: release["season_explicit"].as_bool().unwrap_or(false),
        episode: release["episode"]
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or(DownloadServiceError::InvalidPlan)?,
        fansub: release["fansub"].as_str().map(str::to_string),
        quality: release["quality"].as_str().map(str::to_string),
        languages: release["languages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
    })
}

fn build_download_plan(
    settings: &DownloadRuntimeSettings,
    release: &crate::release_parser::ParsedReleaseTitle,
    extension: &str,
    overrides: &DownloadOverrides,
) -> Result<DownloadPlan, DownloadServiceError> {
    build_plan(
        &settings.download_path,
        NamingInput {
            anime_name: release.anime_name.clone(),
            anime_name_override: overrides.anime_name.clone(),
            download_directory_name_override: overrides.directory_name.clone(),
            season: Some(release.season),
            episode: Some(release.episode),
            filename_season_override: overrides
                .filename_season
                .and_then(|value| i32::try_from(value).ok()),
            directory_season_override: overrides
                .directory_season
                .and_then(|value| i32::try_from(value).ok()),
            extension: Some(extension.to_string()),
            version: Some(1),
            rename_format: Some(settings.rename_format.clone()),
            fansub: release.fansub.clone(),
            quality: release.quality.clone(),
            languages: release.languages.clone(),
        },
    )
    .map_err(|_| DownloadServiceError::InvalidPlan)
}

fn download_overrides_from_payload(release: &Value) -> DownloadOverrides {
    DownloadOverrides {
        source_url: release["source_url"].as_str().map(str::to_string),
        anime_name: release["anime_name_override"].as_str().map(str::to_string),
        directory_name: release["download_directory_name_override"]
            .as_str()
            .map(str::to_string),
        episode_offset: release["episode_offset"].as_i64().unwrap_or(0),
        season: None,
        filename_season: release["filename_season_override"].as_i64(),
        directory_season: release["directory_season_override"].as_i64(),
    }
}

fn apply_release_overrides(
    mut release: crate::release_parser::ParsedReleaseTitle,
    overrides: &DownloadOverrides,
) -> Result<crate::release_parser::ParsedReleaseTitle, DownloadServiceError> {
    release.episode = i32::try_from(i64::from(release.episode) - overrides.episode_offset)
        .ok()
        .filter(|episode| *episode > 0)
        .ok_or(DownloadServiceError::InvalidTitle)?;
    if !release.explicit_season {
        if let Some(season) = overrides.season {
            release.season =
                i32::try_from(season).map_err(|_| DownloadServiceError::InvalidPlan)?;
        }
    }
    if let Some(name) = overrides
        .anime_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
    {
        release.anime_name = name.trim().to_string();
    }
    Ok(release)
}

fn valid_download_url(value: &str) -> bool {
    if value.starts_with("magnet:?") {
        return value.contains("xt=urn:btih:");
    }
    reqwest::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

fn required_string(object: &Value, key: &str) -> Result<String, DownloadServiceError> {
    object[key]
        .as_str()
        .map(str::to_string)
        .ok_or(DownloadServiceError::InvalidConfiguration)
}

fn join_path(root: &str, child: &str) -> String {
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        format!("/{child}")
    } else {
        format!("{root}/{child}")
    }
}

fn now_rfc3339() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{:02}-{day:02}T{:02}:{:02}:{:02}Z",
        month,
        day_seconds / 3_600,
        day_seconds % 3_600 / 60,
        day_seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::{
        DownloadOverrides, apply_release_overrides, build_download_plan,
        download_overrides_from_payload,
    };
    use crate::release_parser::parse_release_title;

    #[test]
    fn rss_overrides_apply_episode_offset_and_keep_explicit_season() {
        let overrides = DownloadOverrides {
            anime_name: Some("套马的汉子".to_string()),
            directory_name: Some("JOJO的奇妙冒险 (2012)".to_string()),
            episode_offset: 12,
            season: Some(2),
            filename_season: Some(1),
            directory_season: Some(6),
            ..DownloadOverrides::default()
        };
        let trailing = apply_release_overrides(
            parse_release_title("[Group] Original Show - 15 [1080P]").unwrap(),
            &overrides,
        )
        .unwrap();
        let explicit = apply_release_overrides(
            parse_release_title("[Group] Clevatess S02E13 [1080P]").unwrap(),
            &overrides,
        )
        .unwrap();
        assert_eq!((trailing.season, trailing.episode), (2, 3));
        assert_eq!(trailing.anime_name, "套马的汉子");
        assert_eq!((explicit.season, explicit.episode), (2, 1));

        let plan = build_download_plan(
            &crate::config_store::DownloadRuntimeSettings {
                openlist_url: "http://127.0.0.1:5244".to_string(),
                download_path: "/迅雷/videos/番".to_string(),
                token: String::new(),
                offline_download_tool: "qBittorrent".to_string(),
                rename_format: "{anime_name} - S{season:02d}E{episode:02d}".to_string(),
            },
            &trailing,
            ".mkv",
            &overrides,
        )
        .unwrap();
        assert_eq!(plan.filename, "套马的汉子 - S01E03.mkv");
        assert_eq!(
            plan.directory,
            "/迅雷/videos/番/JOJO的奇妙冒险 (2012)/Season 6"
        );
    }

    #[test]
    fn download_naming_overrides_survive_worker_recovery_payload() {
        let overrides = download_overrides_from_payload(&serde_json::json!({
            "source_url": "https://example.test/rss",
            "anime_name_override": "套马的汉子",
            "download_directory_name_override": "JOJO",
            "episode_offset": 12,
            "filename_season_override": 1,
            "directory_season_override": 6
        }));
        assert_eq!(
            overrides.source_url.as_deref(),
            Some("https://example.test/rss")
        );
        assert_eq!(overrides.anime_name.as_deref(), Some("套马的汉子"));
        assert_eq!(overrides.episode_offset, 12);
        assert_eq!(overrides.filename_season, Some(1));
        assert_eq!(overrides.directory_season, Some(6));
    }
}
