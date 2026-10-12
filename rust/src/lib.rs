//! Rust replacement backend for OpenList-Ani.

pub mod auth;
pub mod config;
pub mod config_store;
pub mod download_service;
pub mod download_workflow;
pub mod http;
pub mod naming;
pub mod openlist;
pub mod release_parser;
pub mod rss;
pub mod rss_scan;
pub mod storage;
pub mod torrent;

pub use auth::AuthSettings;
pub use config::AppConfig;
pub use http::{AppState, router};
