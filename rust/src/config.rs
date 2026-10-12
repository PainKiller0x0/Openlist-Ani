use std::{
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
};

use thiserror::Error;
use toml_edit::{DocumentMut, Item};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppConfig {
    pub bind: SocketAddr,
    pub data_db: PathBuf,
    pub task_mementos_db: PathBuf,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read config file")]
    Read(#[from] std::io::Error),
    #[error("invalid TOML config")]
    Parse(#[from] toml_edit::TomlError),
    #[error("backend.host must be a loopback IP address")]
    NonLoopbackHost,
    #[error("backend.port must be an integer from 1 to 65535")]
    InvalidPort,
}

impl AppConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let source = std::fs::read_to_string(path)?;
        let document = source.parse::<DocumentMut>()?;
        let backend = document.get("backend").and_then(Item::as_table);

        let host = backend
            .and_then(|table| table.get("host"))
            .and_then(Item::as_str)
            .unwrap_or("127.0.0.1");
        let ip = host
            .parse::<IpAddr>()
            .ok()
            .filter(IpAddr::is_loopback)
            .ok_or(ConfigError::NonLoopbackHost)?;
        let port = backend
            .and_then(|table| table.get("port"))
            .and_then(Item::as_integer)
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port != 0)
            .ok_or(ConfigError::InvalidPort)?;

        let data_dir = path.parent().unwrap_or_else(|| Path::new(".")).join("data");
        Ok(Self {
            bind: SocketAddr::new(ip, port),
            data_db: data_dir.join("data.db"),
            task_mementos_db: data_dir.join("task_mementos.db"),
        })
    }
}
