use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use fs2::FileExt;
use sha2::{Digest, Sha256};
use thiserror::Error;
use toml_edit::{DocumentMut, Table};
use uuid::Uuid;

use crate::config::{AppConfig, ConfigError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigSnapshot {
    pub app: AppConfig,
    pub revision: String,
}

#[derive(Debug, Error)]
pub enum ConfigStoreError {
    #[error("configuration file operation failed")]
    Io(#[from] std::io::Error),
    #[error("invalid configuration")]
    InvalidConfig(#[from] ConfigError),
    #[error("configuration changed since it was read")]
    RevisionConflict,
    #[error("configuration was replaced, but directory durability is uncertain")]
    DurabilityUncertain(#[source] std::io::Error),
}

pub struct ConfigDocumentStore {
    path: PathBuf,
}

impl ConfigDocumentStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> Result<ConfigSnapshot, ConfigStoreError> {
        let source = fs::read_to_string(&self.path)?;
        let document = source.parse::<DocumentMut>().map_err(ConfigError::from)?;
        let app = AppConfig::from_document(&self.path, &document)?;
        Ok(ConfigSnapshot {
            app,
            revision: revision(source.as_bytes()),
        })
    }

    pub fn update(
        &self,
        expected_revision: &str,
        edit: impl FnOnce(&mut DocumentMut),
    ) -> Result<ConfigSnapshot, ConfigStoreError> {
        let _lock = self.lock()?;
        let source = fs::read_to_string(&self.path)?;
        if revision(source.as_bytes()) != expected_revision {
            return Err(ConfigStoreError::RevisionConflict);
        }

        let mut document = source.parse::<DocumentMut>().map_err(ConfigError::from)?;
        let original = document.clone();
        edit(&mut document);
        preserve_value_decor(&original, &mut document);
        let updated = document.to_string();
        let validated = updated.parse::<DocumentMut>().map_err(ConfigError::from)?;
        let app = AppConfig::from_document(&self.path, &validated)?;
        atomic_replace(&self.path, updated.as_bytes())?;

        Ok(ConfigSnapshot {
            app,
            revision: revision(updated.as_bytes()),
        })
    }

    fn lock(&self) -> Result<File, ConfigStoreError> {
        let mut lock_path = self.path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(PathBuf::from(lock_path))?;
        file.lock_exclusive()?;
        Ok(file)
    }
}

fn preserve_value_decor(original: &DocumentMut, updated: &mut DocumentMut) {
    for (key, item) in updated.iter_mut() {
        if let Some(previous) = original.get(key.get()) {
            preserve_item_decor(previous, item);
        }
    }
}

fn preserve_item_decor(original: &toml_edit::Item, updated: &mut toml_edit::Item) {
    if let (Some(previous), Some(current)) = (original.as_value(), updated.as_value_mut()) {
        let decor = current.decor_mut();
        if decor.prefix().is_none() && decor.suffix().is_none() {
            *decor = previous.decor().clone();
        }
    }

    if let (Some(previous), Some(current)) = (original.as_table(), updated.as_table_mut()) {
        preserve_table_decor(previous, current);
    }

    if let (Some(previous), Some(current)) = (
        original.as_array_of_tables(),
        updated.as_array_of_tables_mut(),
    ) {
        for index in 0..current.len() {
            if let (Some(old_table), Some(new_table)) =
                (previous.get(index), current.get_mut(index))
            {
                preserve_table_decor(old_table, new_table);
            }
        }
    }
}

fn preserve_table_decor(original: &Table, updated: &mut Table) {
    for (key, item) in updated.iter_mut() {
        if let Some(previous) = original.get(key.get()) {
            preserve_item_decor(previous, item);
        }
    }
}

fn revision(source: &[u8]) -> String {
    format!("{:x}", Sha256::digest(source))
}

fn atomic_replace(path: &Path, content: &[u8]) -> Result<(), ConfigStoreError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".openlist-ani-config-{}.tmp", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(content)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        File::open(parent)?
            .sync_all()
            .map_err(ConfigStoreError::DurabilityUncertain)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
