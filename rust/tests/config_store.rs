use openlist_ani_rs::config_store::{
    ConfigDocumentStore, ConfigStoreError, UiRssSubscriptionInput,
};
use serde_json::json;
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

#[test]
fn add_subscription_preserves_legacy_urls_and_rejects_duplicates() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("config.toml");
    fs::write(
        &path,
        "# keep me\n[backend]\nhost = \"127.0.0.1\"\nport = 19004\n\n[rss]\nurls = [\"https://legacy.example/rss\"]\n",
    )
    .unwrap();
    let store = ConfigDocumentStore::new(&path);
    let before = store.ui_subscriptions().unwrap();
    assert_eq!(before.len(), 1);
    assert!(
        before
            .iter()
            .any(|item| item["url"] == "https://legacy.example/rss")
    );

    let added = UiRssSubscriptionInput {
        url: "https://new.example/rss".to_string(),
        name: "New Show".to_string(),
        anime_name: "New Show filename".to_string(),
        download_directory_name: "New Show folder".to_string(),
        episode_offset: 12,
        filename_season_override: 1,
        directory_season_override: 6,
        tmdb_id: Some(283938),
        poster_url: "https://images.example/poster.jpg".to_string(),
        season: 8,
        exclude_patterns: vec!["(?i)1080p".to_string()],
    };
    store.add_ui_subscription(added.clone()).unwrap();
    let subscriptions = store.ui_subscriptions().unwrap();
    assert_eq!(subscriptions.len(), 2);
    let new = subscriptions
        .iter()
        .find(|item| item["url"] == "https://new.example/rss")
        .unwrap();
    assert_eq!(new["anime_name"], "New Show filename");
    assert_eq!(new["episode_offset"], 12);
    assert_eq!(new["season"], 8);
    assert_eq!(new["enabled"], true);
    assert_eq!(new["completed"], false);
    assert_eq!(new["exclude_patterns"], json!(["(?i)1080p"]));
    assert!(fs::read_to_string(&path).unwrap().contains("# keep me"));

    let original = fs::read(&path).unwrap();
    let duplicate = UiRssSubscriptionInput {
        url: "https://legacy.example/rss".to_string(),
        ..added
    };
    assert!(matches!(
        store.add_ui_subscription(duplicate),
        Err(ConfigStoreError::DuplicateSubscription)
    ));
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn subscription_controls_and_correction_preserve_state_rules() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("config.toml");
    write_config(&path);
    let store = ConfigDocumentStore::new(&path);
    let url = "https://example.invalid/rss";

    store.set_ui_subscription_completed(url, true).unwrap();
    let item = store.ui_subscriptions().unwrap().remove(0);
    assert_eq!(item["completed"], true);
    assert_eq!(item["enabled"], true);

    store.set_ui_subscription_enabled(url, false).unwrap();
    assert_eq!(store.ui_subscriptions().unwrap()[0]["enabled"], false);
    store.set_ui_subscription_enabled(url, true).unwrap();
    let item = store.ui_subscriptions().unwrap().remove(0);
    assert_eq!(item["enabled"], true);
    assert_eq!(item["completed"], false);

    let corrected_url = "https://example.invalid/corrected";
    store
        .correct_ui_subscription(
            url,
            UiRssSubscriptionInput {
                url: corrected_url.to_string(),
                name: "Corrected Show".to_string(),
                anime_name: "Corrected filename".to_string(),
                download_directory_name: "Corrected folder".to_string(),
                episode_offset: 12,
                filename_season_override: 1,
                directory_season_override: 6,
                tmdb_id: None,
                poster_url: String::new(),
                season: 8,
                exclude_patterns: vec![],
            },
        )
        .unwrap();
    let item = store.ui_subscriptions().unwrap().remove(0);
    assert_eq!(item["url"], corrected_url);
    assert_eq!(item["anime_name"], "Corrected filename");
    assert_eq!(item["completed"], false);

    store.remove_ui_subscription(corrected_url).unwrap();
    assert!(store.ui_subscriptions().unwrap().is_empty());
    assert!(matches!(
        store.remove_ui_subscription(corrected_url),
        Err(ConfigStoreError::SubscriptionNotFound)
    ));
}
