use openlist_ani_rs::storage::{
    LegacyMementoRecord, LegacyResourceInput, LegacyResourceStore, LegacyTaskMementoStore,
};
use rusqlite::Connection;
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
    assert_eq!(store.load_all().unwrap(), vec![row]);

    let failed = LegacyMementoRecord {
        task_id: "task-failed".into(),
        state: "failed".into(),
        updated_at: "2026-10-12T10:00:01".into(),
        payload_json: r#"{"schema_version":1,"state":"failed","future_field":true}"#.into(),
    };
    store.save(&failed).unwrap();
    assert_eq!(store.load_all().unwrap().len(), 2);

    let terminal = LegacyMementoRecord {
        task_id: "task-1".into(),
        state: "completed".into(),
        updated_at: "2026-10-12T10:01:00".into(),
        payload_json: r#"{"schema_version":1,"state":"completed"}"#.into(),
    };
    store.save(&terminal).unwrap();
    assert_eq!(store.load_all().unwrap(), vec![failed]);
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
