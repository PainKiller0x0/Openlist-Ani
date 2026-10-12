use openlist_ani_rs::storage::{
    LegacyMementoRecord, LegacyResourceInput, LegacyResourceStore, LegacyTaskMementoStore,
};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use tempfile::tempdir;

fn create_legacy_data_db(path: &std::path::Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE resources (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL,
                title TEXT UNIQUE NOT NULL,
                anime_name TEXT,
                season INTEGER,
                episode INTEGER,
                fansub TEXT,
                quality TEXT,
                languages TEXT,
                version INTEGER,
                downloaded_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                episode_name TEXT
            );
            CREATE TABLE strict_rejections (
                url TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                rejected_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );",
        )
        .unwrap();
}

fn create_legacy_task_db(path: &std::path::Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE task_mementos (
                task_id TEXT PRIMARY KEY,
                state TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                payload TEXT NOT NULL
            );
            CREATE TABLE task_memento_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )
        .unwrap();
}

fn resource_input(
    title: &str,
    anime_name: &str,
    season: i64,
    episode: i64,
    fansub: &str,
) -> LegacyResourceInput {
    LegacyResourceInput {
        url: format!("https://example.test/{title}"),
        title: title.into(),
        anime_name: Some(anime_name.into()),
        season: Some(season),
        episode: Some(episode),
        fansub: Some(fansub.into()),
        quality: Some("1080p".into()),
        languages: Some("CHT".into()),
        version: Some(1),
        downloaded_at: None,
        episode_name: None,
    }
}

#[test]
fn resource_insert_is_idempotent_and_preserves_episode_name_column() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("data.db");
    create_legacy_data_db(&path);
    let store = LegacyResourceStore::new(&path);
    let first = LegacyResourceInput {
        url: "https://example.test/ep3".into(),
        title: "release title".into(),
        anime_name: Some("Example (2026)".into()),
        season: Some(1),
        episode: Some(3),
        fansub: Some("Group".into()),
        quality: Some("1080p".into()),
        languages: Some("CHT".into()),
        version: Some(1),
        downloaded_at: Some("2026-10-12 10:00:00".into()),
        episode_name: Some("Episode Three".into()),
    };

    assert!(store.insert_ignore_duplicate(&first).unwrap());
    let duplicate = LegacyResourceInput {
        url: "https://example.test/other".into(),
        episode_name: Some("must not replace".into()),
        ..first.clone()
    };
    assert!(!store.insert_ignore_duplicate(&duplicate).unwrap());

    let saved = store.find_by_title("release title").unwrap().unwrap();
    assert_eq!(saved.title, first.title);
    assert_eq!(saved.url, first.url);
    assert_eq!(saved.episode_name.as_deref(), Some("Episode Three"));
    assert_eq!(saved.episode, Some(3));
}

#[test]
fn memento_payload_unknown_fields_round_trip_and_terminal_save_deletes() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("task_mementos.db");
    create_legacy_task_db(&path);
    let store = LegacyTaskMementoStore::new(&path);
    let raw_payload = r#"{"schema_version":1,"state":"downloading","future_checkpoint":{"remote_nonce":"keep-me"}}"#;
    let row = LegacyMementoRecord {
        task_id: "task-1".into(),
        state: "downloading".into(),
        updated_at: "2026-10-12T10:00:00".into(),
        payload_json: raw_payload.into(),
    };

    store.save(&row).unwrap();
    assert_eq!(store.load_all().unwrap(), vec![row.clone()]);

    let failed = LegacyMementoRecord {
        task_id: "task-failed".into(),
        state: "failed".into(),
        updated_at: "2026-10-12T10:00:01".into(),
        payload_json: r#"{"schema_version":1,"state":"failed","future_field":true}"#.into(),
    };
    store.save(&failed).unwrap();
    let unknown_state = LegacyMementoRecord {
        task_id: "task-future".into(),
        state: "future_state".into(),
        updated_at: "2026-10-12T10:00:02".into(),
        payload_json: r#"{"schema_version":2,"state":"future_state","new_checkpoint":7}"#.into(),
    };
    store.save(&unknown_state).unwrap();
    assert_eq!(
        store.load_all().unwrap(),
        vec![row.clone(), failed.clone(), unknown_state.clone()]
    );

    let invalid = LegacyMementoRecord {
        task_id: "invalid".into(),
        state: "pending".into(),
        updated_at: "2026-10-12T10:00:03".into(),
        payload_json: "[]".into(),
    };
    assert!(store.save(&invalid).is_err());
    assert_eq!(store.load_all().unwrap().len(), 3);

    let terminal = LegacyMementoRecord {
        task_id: "task-1".into(),
        state: "completed".into(),
        updated_at: "2026-10-12T10:01:00".into(),
        payload_json: r#"{"schema_version":1,"state":"completed"}"#.into(),
    };
    store.save(&terminal).unwrap();
    assert_eq!(store.load_all().unwrap(), vec![failed, unknown_state]);
}

#[test]
fn persistence_never_creates_missing_legacy_database_files() {
    let directory = tempdir().unwrap();
    let data_path = directory.path().join("missing-data.db");
    let task_path = directory.path().join("missing-tasks.db");

    let resource_result = LegacyResourceStore::new(&data_path).find_by_title("anything");
    let memento_result = LegacyTaskMementoStore::new(&task_path).load_all();

    assert!(resource_result.is_err());
    assert!(memento_result.is_err());
    assert!(!data_path.exists());
    assert!(!task_path.exists());
}

#[test]
fn title_lookup_deduplicates_and_chunks_large_candidate_lists() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("data.db");
    create_legacy_data_db(&path);
    let store = LegacyResourceStore::new(&path);
    for title in ["candidate-000", "candidate-949"] {
        store
            .insert_ignore_duplicate(&resource_input(title, "Example", 1, 1, "Group"))
            .unwrap();
    }
    let mut candidates = (0..950)
        .map(|index| format!("candidate-{index:03}"))
        .collect::<Vec<_>>();
    candidates.push("candidate-000".into());

    let existing = store.find_existing_titles(&candidates).unwrap();

    assert_eq!(
        existing,
        HashSet::from(["candidate-000".to_string(), "candidate-949".to_string()])
    );
}

#[test]
fn episode_lookup_is_season_specific_and_returns_all_release_versions() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("data.db");
    create_legacy_data_db(&path);
    let store = LegacyResourceStore::new(&path);
    store
        .insert_ignore_duplicate(&resource_input("episode-a", "Example", 1, 3, "Group A"))
        .unwrap();
    let mut second_version = resource_input("episode-b", "Example", 1, 3, "Group B");
    second_version.version = Some(2);
    store.insert_ignore_duplicate(&second_version).unwrap();
    store
        .insert_ignore_duplicate(&resource_input("episode-s2", "Example", 2, 3, "Group C"))
        .unwrap();
    let mut keys = vec![
        ("Example".into(), 1, 3),
        ("Example".into(), 9, 1),
        ("Example".into(), 1, 3),
    ];
    keys.extend((0..300).map(|episode| ("Other".into(), 1, episode)));

    let releases = store.find_releases_by_episodes(&keys).unwrap();

    assert_eq!(releases.len(), 302);
    assert_eq!(releases[&("Example".into(), 1, 3)].len(), 2);
    assert_eq!(releases[&("Example".into(), 9, 1)], Vec::new());
    let versions = releases[&("Example".into(), 1, 3)]
        .iter()
        .map(|release| release.version.unwrap())
        .collect::<HashSet<_>>();
    assert_eq!(versions, HashSet::from([1, 2]));
}

#[test]
fn latest_episode_lookup_prefers_exact_names_and_normalizes_metadata_suffixes() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("data.db");
    create_legacy_data_db(&path);
    let store = LegacyResourceStore::new(&path);
    for row in [
        resource_input("jojo-s7", "JOJO的奇妙冒险 (2012) Season 7", 7, 3, "Group"),
        resource_input("jojo-s6", "JOJO的奇妙冒险 (2012) Season 6", 6, 15, "Group"),
        resource_input("exact-s1", "Exact Name", 1, 6, "Group"),
        resource_input("exact-s2", "Exact Name", 2, 8, "Group"),
        resource_input("example-s2", "Example 第 2 季 (2025)", 2, 4, "Group"),
    ] {
        store.insert_ignore_duplicate(&row).unwrap();
    }
    let anime_names = vec![
        "JOJO的奇妙冒险".into(),
        "Exact Name".into(),
        "Example".into(),
    ];
    let requested_seasons = HashMap::from([
        ("JOJO的奇妙冒险".to_string(), 7),
        ("Exact Name".to_string(), 1),
        ("Example".to_string(), 2),
    ]);

    let latest = store
        .find_latest_episodes(&anime_names, &requested_seasons)
        .unwrap();

    assert_eq!(latest["JOJO的奇妙冒险"], 3);
    assert_eq!(latest["Exact Name"], 6);
    assert_eq!(latest["Example"], 4);

    let all_seasons = store
        .find_latest_episodes(&["Exact Name".into()], &HashMap::new())
        .unwrap();
    assert_eq!(all_seasons["Exact Name"], 8);
}
