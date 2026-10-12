use std::collections::HashSet;

use fancy_regex::Regex;
use serde_json::Value;

use crate::{
    config_store::DownloadRuntimeSettings,
    download_service::{
        DownloadOverrides, DownloadServiceError, SubmittedDownload, submit_download_with_overrides,
    },
    release_parser::parse_release_title,
    rss::parse_rss,
    storage::{LegacyDbPaths, LegacyEpisodeKey, LegacyResourceStore, LegacyTaskMementoStore},
};

const RSS_SCAN_LIMIT: i64 = 200;

#[derive(Default)]
pub struct RssScanReport {
    pub feeds_scanned: usize,
    pub entries_seen: usize,
    pub submitted: Vec<SubmittedDownload>,
    pub skipped: usize,
    pub errors: Vec<String>,
    pub truncated_feeds: usize,
}

struct Candidate {
    title: String,
    download_url: String,
    key: LegacyEpisodeKey,
    overrides: DownloadOverrides,
}

pub async fn scan_subscriptions(
    databases: LegacyDbPaths,
    settings: DownloadRuntimeSettings,
    global_patterns: &[String],
    subscriptions: &[Value],
) -> Result<RssScanReport, String> {
    let mut report = RssScanReport::default();
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    for subscription in subscriptions.iter().filter(|item| is_active(item)) {
        let url = subscription
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if url.is_empty() {
            continue;
        }
        let name = subscription
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .or_else(|| {
                subscription
                    .get("anime_name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.trim().is_empty())
            })
            .unwrap_or("未命名订阅")
            .to_string();
        let patterns = match compile_patterns(global_patterns, subscription) {
            Ok(patterns) => patterns,
            Err(()) => {
                report.errors.push(format!("{name}：排除规则无效"));
                continue;
            }
        };
        let feed = match parse_rss(url, Some(RSS_SCAN_LIMIT)).await {
            Ok(feed) if feed.success => feed,
            _ => {
                report.errors.push(format!("{name}：RSS 读取失败"));
                continue;
            }
        };
        report.feeds_scanned += 1;
        report.entries_seen += feed.total;
        report.truncated_feeds += usize::from(feed.total > feed.entries.len());
        for entry in feed.entries {
            if matches_exclusion(&patterns, &entry.title).unwrap_or(true) {
                report.skipped += 1;
                continue;
            }
            let Some(release) = parse_release_title(&entry.title) else {
                report.skipped += 1;
                continue;
            };
            let offset = subscription
                .get("episode_offset")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let Some(episode) = i64::from(release.episode)
                .checked_sub(offset)
                .filter(|value| *value > 0)
            else {
                report.skipped += 1;
                continue;
            };
            let season = if release.explicit_season {
                i64::from(release.season)
            } else {
                subscription
                    .get("season")
                    .and_then(Value::as_i64)
                    .unwrap_or(1)
            };
            let anime_name = subscription
                .get("anime_name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(&release.anime_name)
                .trim()
                .to_string();
            let key = (anime_name.clone(), season, episode);
            if !seen.insert(key.clone()) {
                report.skipped += 1;
                continue;
            }
            candidates.push(Candidate {
                title: entry.title,
                download_url: entry.download_url,
                key,
                overrides: DownloadOverrides {
                    source_url: Some(url.to_string()),
                    anime_name: Some(anime_name),
                    directory_name: non_empty(subscription, "download_directory_name"),
                    episode_offset: offset,
                    season: Some(season),
                    filename_season: positive(subscription, "filename_season_override"),
                    directory_season: positive(subscription, "directory_season_override"),
                },
            });
        }
    }

    let keys = candidates
        .iter()
        .map(|candidate| candidate.key.clone())
        .collect::<Vec<_>>();
    let data_path = databases.data_db.clone();
    let titles = candidates
        .iter()
        .map(|candidate| candidate.title.clone())
        .collect::<Vec<_>>();
    let existing = tokio::task::spawn_blocking(move || {
        let store = LegacyResourceStore::new(data_path);
        Ok::<_, crate::storage::LegacyStorageError>((
            store.find_releases_by_episodes(&keys)?,
            store.find_existing_titles(&titles)?,
        ))
    })
    .await
    .map_err(|_| "resource lookup worker failed".to_string())?
    .map_err(|_| "resource database is unavailable".to_string())?;

    let task_path = databases.task_mementos_db.clone();
    let records =
        tokio::task::spawn_blocking(move || LegacyTaskMementoStore::new(task_path).load_all())
            .await
            .map_err(|_| "task lookup worker failed".to_string())?
            .map_err(|_| "task database is unavailable".to_string())?;
    // Refused or uncertain tasks require an explicit retry, not a job each scan.
    let failed_sources = records
        .iter()
        .filter(|record| record.state == "failed")
        .filter_map(|record| serde_json::from_str::<Value>(&record.payload_json).ok())
        .filter_map(|payload| {
            payload["release"]["download_url"]
                .as_str()
                .map(str::to_string)
        })
        .collect::<HashSet<_>>();
    let active_tasks = records
        .into_iter()
        .filter(|record| !matches!(record.state.as_str(), "completed" | "failed" | "cancelled"))
        .filter_map(|record| serde_json::from_str::<Value>(&record.payload_json).ok())
        .filter_map(|payload| {
            let release = &payload["release"];
            Some((
                release["anime_name"].as_str()?.to_string(),
                release["season"].as_i64()?,
                release["episode"].as_i64()?,
            ))
        })
        .collect::<HashSet<_>>();

    for candidate in candidates {
        if failed_sources.contains(&candidate.download_url)
            || existing.1.contains(&candidate.title)
            || existing
                .0
                .get(&candidate.key)
                .is_some_and(|releases| !releases.is_empty())
            || active_tasks.contains(&candidate.key)
        {
            report.skipped += 1;
            continue;
        }
        match submit_download_with_overrides(
            databases.clone(),
            settings.clone(),
            &candidate.title,
            &candidate.download_url,
            candidate.overrides,
        )
        .await
        {
            Ok(submitted) => report.submitted.push(submitted),
            Err(DownloadServiceError::Duplicate(_)) => report.skipped += 1,
            Err(_) => report.errors.push("有条目未能提交到 OpenList".to_string()),
        }
    }
    Ok(report)
}

pub fn split_patterns(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let (mut parentheses, mut brackets, mut braces) = (0usize, 0usize, 0usize);
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        match character {
            '(' => parentheses += 1,
            ')' => parentheses = parentheses.saturating_sub(1),
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            '{' => braces += 1,
            '}' => braces = braces.saturating_sub(1),
            '|' if parentheses == 0 && brackets == 0 && braces == 0 => {
                parts.push(value[start..index].trim().to_string());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(value[start..].trim().to_string());
    let mut seen = HashSet::new();
    parts.retain(|part| !part.is_empty() && seen.insert(part.clone()));
    parts
}

fn compile_patterns(global: &[String], subscription: &Value) -> Result<Vec<(String, Regex)>, ()> {
    global
        .iter()
        .cloned()
        .chain(
            subscription
                .get("exclude_patterns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string),
        )
        .map(|pattern| {
            Regex::new(&pattern)
                .map(|regex| (pattern, regex))
                .map_err(|_| ())
        })
        .collect()
}

fn matches_exclusion(patterns: &[(String, Regex)], title: &str) -> Result<bool, ()> {
    for (_, pattern) in patterns {
        if pattern.is_match(title).map_err(|_| ())? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_active(subscription: &Value) -> bool {
    subscription.get("enabled").and_then(Value::as_bool) != Some(false)
        && subscription.get("completed").and_then(Value::as_bool) != Some(true)
}

fn non_empty(subscription: &Value, key: &str) -> Option<String> {
    subscription
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn positive(subscription: &Value, key: &str) -> Option<i64> {
    subscription
        .get(key)
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
}

#[cfg(test)]
mod tests {
    use super::split_patterns;

    #[test]
    fn pipe_splitting_preserves_alternations_inside_regex_groups() {
        assert_eq!(
            split_patterns(r"1080p|(?:1080|720)p|\|literal"),
            ["1080p", "(?:1080|720)p", r"\|literal"]
        );
    }
}
