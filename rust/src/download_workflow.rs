use crate::openlist::{OpenListClient, OpenListError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const VIDEO_EXTENSIONS: &[&str] = &[
    ".mp4", ".mkv", ".avi", ".mov", ".flv", ".wmv", ".webm", ".mpg", ".mpeg",
];
const MAX_DIRECTORY_DEPTH: usize = 32;
const MAX_CONFLICT_SUFFIX: usize = 99;

#[derive(Debug, Error)]
pub enum DownloadWorkflowError {
    #[error(transparent)]
    OpenList(#[from] OpenListError),
    #[error("OpenList returned an unsafe path component")]
    UnsafePath,
    #[error("target directory must be inside the configured download root")]
    TargetOutsideRoot,
    #[error("no video file was found in the download directory")]
    VideoNotFound,
    #[error("could not create the target directory")]
    CreateDirectory,
    #[error("could not rename a conflicting downloaded file")]
    ResolveConflict,
    #[error("could not move the downloaded file")]
    MoveFile,
    #[error("all conflict suffixes are already in use")]
    ConflictLimit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedFile {
    pub path: String,
    pub cleanup_succeeded: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct MaterializationPlan {
    pub source_directory: String,
    pub original_filename: String,
    pub final_filename: String,
    pub target_directory: String,
}

pub async fn detect_video_file(
    client: &OpenListClient,
    directory: &str,
) -> Result<Option<String>, DownloadWorkflowError> {
    let mut candidates = Vec::new();
    collect_videos(client, directory, "", 0, &mut candidates).await?;
    candidates.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    Ok(candidates.into_iter().next().map(|candidate| candidate.0))
}

pub async fn materialize_video_file(
    client: &OpenListClient,
    download_root: &str,
    temporary_root: &str,
    target_directory: &str,
    relative_filename: &str,
    desired_filename: &str,
) -> Result<MaterializedFile, DownloadWorkflowError> {
    let plan = prepare_materialization(
        client,
        download_root,
        temporary_root,
        target_directory,
        relative_filename,
        desired_filename,
    )
    .await?;
    execute_materialization(client, temporary_root, &plan).await
}

pub async fn prepare_materialization(
    client: &OpenListClient,
    download_root: &str,
    temporary_root: &str,
    target_directory: &str,
    relative_filename: &str,
    desired_filename: &str,
) -> Result<MaterializationPlan, DownloadWorkflowError> {
    let relative_parts = safe_relative_parts(relative_filename)?;
    let desired_parts = safe_relative_parts(desired_filename)?;
    if desired_parts.len() != 1 {
        return Err(DownloadWorkflowError::UnsafePath);
    }
    let root = normalize_absolute(download_root)?;
    let temporary_root = normalize_absolute(temporary_root)?;
    let target = normalize_absolute(target_directory)?;
    if temporary_root == "/" {
        return Err(DownloadWorkflowError::UnsafePath);
    }
    if target != root && !target.starts_with(&format!("{}/", root.trim_end_matches('/'))) {
        return Err(DownloadWorkflowError::TargetOutsideRoot);
    }

    for path in directory_paths(&root, &target)? {
        if !client.mkdir(&path).await? && client.list_files(&path).await.is_err() {
            return Err(DownloadWorkflowError::CreateDirectory);
        }
    }

    let filename = relative_parts.last().expect("validated non-empty path");
    let desired_filename = &desired_parts[0];
    let parent = relative_parts[..relative_parts.len() - 1]
        .iter()
        .fold(temporary_root.clone(), |path, part| join_path(&path, part));
    let source_files = client.list_files(&parent).await?;
    if !source_files.iter().any(|file| file.name == *filename) {
        return Err(DownloadWorkflowError::VideoNotFound);
    }

    let destination_files = client.list_files(&target).await?;
    let existing_names = destination_files
        .iter()
        .map(|file| file.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut final_name = desired_filename.clone();
    if existing_names.contains(final_name.as_str()) {
        final_name = next_available_name(desired_filename, &existing_names)?;
    }
    Ok(MaterializationPlan {
        source_directory: parent,
        original_filename: filename.clone(),
        final_filename: final_name,
        target_directory: target,
    })
}

pub async fn execute_materialization(
    client: &OpenListClient,
    temporary_root: &str,
    plan: &MaterializationPlan,
) -> Result<MaterializedFile, DownloadWorkflowError> {
    let source_files = client.list_files(&plan.source_directory).await?;
    let destination_files = client.list_files(&plan.target_directory).await?;
    let source_has_final = source_files
        .iter()
        .any(|file| !file.is_dir && file.name == plan.final_filename);
    let source_has_original = source_files
        .iter()
        .any(|file| !file.is_dir && file.name == plan.original_filename);
    let already_moved = destination_files
        .iter()
        .any(|file| !file.is_dir && file.name == plan.final_filename);

    if !source_has_final && !source_has_original && already_moved {
        return Ok(MaterializedFile {
            path: join_path(&plan.target_directory, &plan.final_filename),
            cleanup_succeeded: cleanup_task_directory(client, temporary_root).await,
        });
    }
    if !source_has_final && !source_has_original {
        return Err(DownloadWorkflowError::VideoNotFound);
    }

    if !source_has_final && plan.original_filename != plan.final_filename {
        let source_path = join_path(&plan.source_directory, &plan.original_filename);
        let renamed = client.rename_file(&source_path, &plan.final_filename).await;
        if !wait_for_file(client, &plan.source_directory, &plan.final_filename).await? {
            let _ = renamed;
            return Err(DownloadWorkflowError::ResolveConflict);
        }
    }

    let moved = client
        .move_files(
            &plan.source_directory,
            &plan.target_directory,
            std::slice::from_ref(&plan.final_filename),
        )
        .await;
    if !matches!(moved, Ok(true)) {
        let source_files = client.list_files(&plan.source_directory).await?;
        let destination_files = client.list_files(&plan.target_directory).await?;
        if source_files
            .iter()
            .any(|file| !file.is_dir && file.name == plan.final_filename)
            || !destination_files
                .iter()
                .any(|file| !file.is_dir && file.name == plan.final_filename)
        {
            return Err(DownloadWorkflowError::MoveFile);
        }
    }

    if !wait_for_file(client, &plan.target_directory, &plan.final_filename).await? {
        return Err(DownloadWorkflowError::MoveFile);
    }

    Ok(MaterializedFile {
        path: join_path(&plan.target_directory, &plan.final_filename),
        cleanup_succeeded: cleanup_task_directory(client, temporary_root).await,
    })
}

async fn wait_for_file(
    client: &OpenListClient,
    directory: &str,
    filename: &str,
) -> Result<bool, DownloadWorkflowError> {
    // Cloud rename/move may return HTTP 200 before refreshed listings catch up.
    for attempt in 0..30 {
        if client
            .list_files(directory)
            .await?
            .iter()
            .any(|file| !file.is_dir && file.name == filename)
        {
            return Ok(true);
        }
        if attempt < 29 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
    Ok(false)
}

async fn cleanup_task_directory(client: &OpenListClient, path: &str) -> bool {
    let Some((parent, name)) = path.trim_end_matches('/').rsplit_once('/') else {
        return false;
    };
    if name.is_empty() {
        return false;
    }
    let parent = if parent.is_empty() { "/" } else { parent };
    client
        .remove_path(parent, &[name.to_string()])
        .await
        .unwrap_or(false)
}

async fn collect_videos(
    client: &OpenListClient,
    directory: &str,
    relative_prefix: &str,
    depth: usize,
    output: &mut Vec<(String, u64)>,
) -> Result<(), DownloadWorkflowError> {
    let mut pending = vec![(directory.to_string(), relative_prefix.to_string(), depth)];
    while let Some((current_directory, prefix, current_depth)) = pending.pop() {
        if current_depth > MAX_DIRECTORY_DEPTH {
            continue;
        }
        for entry in client.list_files(&current_directory).await? {
            validate_component(&entry.name)?;
            let relative = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{prefix}/{}", entry.name)
            };
            if entry.is_dir {
                pending.push((
                    join_path(&current_directory, &entry.name),
                    relative,
                    current_depth + 1,
                ));
            } else if is_video(&entry.name) {
                output.push((relative, entry.size.unwrap_or_default()));
            }
        }
    }
    Ok(())
}

fn is_video(name: &str) -> bool {
    let lowercase = name.to_ascii_lowercase();
    VIDEO_EXTENSIONS
        .iter()
        .any(|extension| lowercase.ends_with(extension))
}

fn safe_relative_parts(value: &str) -> Result<Vec<String>, DownloadWorkflowError> {
    if value.starts_with('/') || value.contains('\\') || value.contains('\0') {
        return Err(DownloadWorkflowError::UnsafePath);
    }
    let parts = value.split('/').map(str::to_string).collect::<Vec<_>>();
    if parts.is_empty()
        || parts
            .iter()
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(DownloadWorkflowError::UnsafePath);
    }
    Ok(parts)
}

fn validate_component(value: &str) -> Result<(), DownloadWorkflowError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        Err(DownloadWorkflowError::UnsafePath)
    } else {
        Ok(())
    }
}

fn normalize_absolute(path: &str) -> Result<String, DownloadWorkflowError> {
    if !path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return Err(DownloadWorkflowError::UnsafePath);
    }
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.iter().any(|part| *part == "." || *part == "..") {
        return Err(DownloadWorkflowError::UnsafePath);
    }
    Ok(format!("/{}", parts.join("/")))
}

fn directory_paths(root: &str, target: &str) -> Result<Vec<String>, DownloadWorkflowError> {
    if target == root {
        return Ok(Vec::new());
    }
    let relative = if root == "/" {
        target.trim_start_matches('/')
    } else {
        target
            .strip_prefix(&format!("{}/", root.trim_end_matches('/')))
            .ok_or(DownloadWorkflowError::TargetOutsideRoot)?
    };
    let mut current = root.trim_end_matches('/').to_string();
    if current.is_empty() {
        current.push('/');
    }
    let mut paths = Vec::new();
    for part in relative.split('/') {
        validate_component(part)?;
        current = join_path(&current, part);
        paths.push(current.clone());
    }
    Ok(paths)
}

fn next_available_name(
    filename: &str,
    existing: &std::collections::HashSet<&str>,
) -> Result<String, DownloadWorkflowError> {
    let (stem, extension) = filename
        .rfind('.')
        .filter(|index| *index > 0)
        .map_or((filename, ""), |index| filename.split_at(index));
    for suffix in 1..=MAX_CONFLICT_SUFFIX {
        let candidate = format!("{stem} ({suffix}){extension}");
        if !existing.contains(candidate.as_str()) {
            return Ok(candidate);
        }
    }
    Err(DownloadWorkflowError::ConflictLimit)
}

fn join_path(directory: &str, name: &str) -> String {
    if directory == "/" {
        format!("/{name}")
    } else {
        format!("{}/{name}", directory.trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DownloadWorkflowError, detect_video_file, directory_paths, is_video,
        materialize_video_file, next_available_name, normalize_absolute, safe_relative_parts,
    };
    use crate::openlist::OpenListClient;
    use axum::{Json, Router, extract::State, routing::post};
    use serde_json::{Value, json};
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[test]
    fn validates_paths_and_builds_nested_directories_without_escape() {
        assert_eq!(normalize_absolute("/media/anime/").unwrap(), "/media/anime");
        assert_eq!(
            directory_paths("/media", "/media/Series/Season 1").unwrap(),
            ["/media/Series", "/media/Series/Season 1"]
        );
        assert!(matches!(
            normalize_absolute("/media/../other"),
            Err(DownloadWorkflowError::UnsafePath)
        ));
        assert!(matches!(
            safe_relative_parts("../episode.mkv"),
            Err(DownloadWorkflowError::UnsafePath)
        ));
        assert!(matches!(
            directory_paths("/media", "/other/Series"),
            Err(DownloadWorkflowError::TargetOutsideRoot)
        ));
    }

    #[test]
    fn video_detection_is_case_insensitive_and_conflicts_use_python_style_suffixes() {
        assert!(is_video("Episode.MKV"));
        assert!(!is_video("subtitles.ass"));
        let existing = HashSet::from(["Episode.mkv", "Episode (1).mkv"]);
        assert_eq!(
            next_available_name("Episode.mkv", &existing).unwrap(),
            "Episode (2).mkv"
        );
    }

    #[derive(Default)]
    struct MaterializeStub {
        renamed: bool,
        moved: bool,
        cleaned: bool,
        source_removed: bool,
    }

    #[tokio::test]
    async fn materializes_largest_video_and_resolves_existing_target_name() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(MaterializeStub::default()));
        let app =
            Router::new()
                .route(
                    "/api/fs/list",
                    post(
                        |State(state): State<Arc<Mutex<MaterializeStub>>>,
                         Json(body): Json<Value>| async move {
                            let path = body["path"].as_str().unwrap();
                            let state = state.lock().unwrap();
                            let entries = match path {
                                "/downloads/.oani-download-tmp/task-7" => json!([
                                    {"name":"nested", "is_dir":true},
                                    {"name":"small.mkv", "size":10}
                                ]),
                                "/downloads/.oani-download-tmp/task-7/nested" => {
                                    if state.source_removed {
                                        json!([])
                                    } else {
                                        let name = if state.renamed {
                                            "episode (1).mkv"
                                        } else {
                                            "episode.mkv"
                                        };
                                        json!([
                                            {"name":name, "is_dir":false, "size":100},
                                            {"name":"subtitle.ass", "is_dir":false, "size":5}
                                        ])
                                    }
                                }
                                "/media/anime/Series/Season 1" => {
                                    if state.moved {
                                        json!([
                                            {"name":"episode.mkv", "is_dir":false, "size":100},
                                            {"name":"episode (1).mkv", "is_dir":false, "size":100}
                                        ])
                                    } else {
                                        json!([{"name":"episode.mkv", "is_dir":false, "size":100}])
                                    }
                                }
                                "/downloads/.oani-download-tmp" => json!([]),
                                _ => panic!("unexpected list path: {path}"),
                            };
                            Json(json!({"code":200,"data":{"content":entries}}))
                        },
                    ),
                )
                .route(
                    "/api/fs/mkdir",
                    post(|Json(body): Json<Value>| async move {
                        assert!(body["path"].as_str().unwrap().starts_with("/media/anime/"));
                        Json(json!({"code":200}))
                    }),
                )
                .route(
                    "/api/fs/rename",
                    post(
                        |State(state): State<Arc<Mutex<MaterializeStub>>>,
                         Json(body): Json<Value>| async move {
                            assert_eq!(
                                body["path"],
                                "/downloads/.oani-download-tmp/task-7/nested/episode.mkv"
                            );
                            assert_eq!(body["name"], "episode (1).mkv");
                            state.lock().unwrap().renamed = true;
                            Json(json!({"code":200}))
                        },
                    ),
                )
                .route(
                    "/api/fs/move",
                    post(
                        |State(state): State<Arc<Mutex<MaterializeStub>>>,
                         Json(body): Json<Value>| async move {
                            assert_eq!(
                                body["src_dir"],
                                "/downloads/.oani-download-tmp/task-7/nested"
                            );
                            assert_eq!(body["dst_dir"], "/media/anime/Series/Season 1");
                            assert_eq!(body["names"], json!(["episode (1).mkv"]));
                            state.lock().unwrap().moved = true;
                            Json(json!({"code":200}))
                        },
                    ),
                )
                .route(
                    "/api/fs/remove",
                    post(
                        |State(state): State<Arc<Mutex<MaterializeStub>>>,
                         Json(body): Json<Value>| async move {
                            assert_eq!(
                                body,
                                json!({"dir":"/downloads/.oani-download-tmp","names":["task-7"]})
                            );
                            state.lock().unwrap().cleaned = true;
                            Json(json!({"code":200}))
                        },
                    ),
                )
                .with_state(Arc::clone(&state));
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = OpenListClient::new(&format!("http://{address}"), "test-token").unwrap();
        let detected = detect_video_file(&client, "/downloads/.oani-download-tmp/task-7")
            .await
            .unwrap();
        assert_eq!(detected.as_deref(), Some("nested/episode.mkv"));
        let materialized = materialize_video_file(
            &client,
            "/media/anime",
            "/downloads/.oani-download-tmp/task-7",
            "/media/anime/Series/Season 1",
            detected.as_deref().unwrap(),
            "episode.mkv",
        )
        .await
        .unwrap();
        assert_eq!(
            materialized.path,
            "/media/anime/Series/Season 1/episode (1).mkv"
        );
        assert!(materialized.cleanup_succeeded);
        {
            let mut state = state.lock().unwrap();
            assert!(state.renamed && state.moved && state.cleaned);
            state.source_removed = true;
        }
        assert!(matches!(
            materialize_video_file(
                &client,
                "/media/anime",
                "/downloads/.oani-download-tmp/task-7",
                "/media/anime/Series/Season 1",
                "episode.mkv",
                "episode.mkv"
            )
            .await,
            Err(DownloadWorkflowError::VideoNotFound)
        ));
    }
}
