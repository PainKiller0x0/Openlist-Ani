use openlist_ani_rs::config_store::{ConfigDocumentStore, ConfigStoreError};
use std::{
    fs,
    sync::{Arc, Barrier},
};
use tempfile::tempdir;
use toml_edit::value;

const CONFIG: &str = "# keep this comment\n[backend]\nhost = \"127.0.0.1\"\nport = 19004 # keep this too\n\n[[rss.subscriptions]]\nurl = \"https://example.invalid/rss\"\nseason = 1 # keep subscription comment\n\n[unknown_legacy_section]\nopaque_setting = \"preserve-me\"\n";

fn write_config(path: &std::path::Path) {
    fs::write(path, CONFIG).unwrap();
}

#[test]
fn update_preserves_comments_and_unknown_sections_and_changes_revision() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("config.toml");
    write_config(&path);
    let store = ConfigDocumentStore::new(&path);
    let before = store.load().unwrap();

    let after = store
        .update(&before.revision, |document| {
            document["backend"]["port"] = value(19005);
            document["rss"]["subscriptions"][0]["season"] = value(2);
        })
        .unwrap();
    let saved = fs::read_to_string(path).unwrap();

    assert_eq!(after.app.bind.port(), 19005);
    assert_ne!(after.revision, before.revision);
    assert!(saved.contains("# keep this comment"));
    assert!(saved.contains("# keep this too"));
    assert!(saved.contains("# keep subscription comment"));
    assert!(saved.contains("opaque_setting = \"preserve-me\""));
}

#[test]
fn stale_revision_and_invalid_update_leave_config_unchanged() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("config.toml");
    write_config(&path);
    let store = ConfigDocumentStore::new(&path);
    let original = store.load().unwrap();
    let saved = fs::read(&path).unwrap();
    let stale = store.update("stale-revision", |_| {}).unwrap_err();
    assert!(matches!(stale, ConfigStoreError::RevisionConflict));
    assert_eq!(fs::read(&path).unwrap(), saved);

    let invalid = store
        .update(&original.revision, |document| {
            document["backend"]["host"] = value("0.0.0.0");
        })
        .unwrap_err();
    assert!(matches!(invalid, ConfigStoreError::InvalidConfig(_)));
    assert_eq!(fs::read(&path).unwrap(), saved);
}

#[test]
fn concurrent_updates_from_one_revision_allow_only_one_writer() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("config.toml");
    write_config(&path);
    let store = Arc::new(ConfigDocumentStore::new(&path));
    let revision = store.load().unwrap().revision;
    let barrier = Arc::new(Barrier::new(2));

    let writers = (19005..=19006)
        .map(|port| {
            let store = Arc::clone(&store);
            let revision = revision.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store.update(&revision, |document| {
                    document["backend"]["port"] = value(port);
                })
            })
        })
        .collect::<Vec<_>>();
    let results = writers
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(ConfigStoreError::RevisionConflict)))
            .count(),
        1
    );
    let final_config = store.load().unwrap();
    assert!([19005, 19006].contains(&final_config.app.bind.port()));
}

#[cfg(unix)]
#[test]
fn config_and_lock_files_are_private_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().unwrap();
    let path = directory.path().join("config.toml");
    write_config(&path);
    let store = ConfigDocumentStore::new(&path);
    let revision = store.load().unwrap().revision;
    store.update(&revision, |_| {}).unwrap();

    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let lock_path = path.with_file_name("config.toml.lock");
    assert_eq!(
        fs::metadata(lock_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
