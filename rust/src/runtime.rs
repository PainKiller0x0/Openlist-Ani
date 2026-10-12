use std::{
    collections::VecDeque,
    io::Write,
    sync::{Mutex, OnceLock},
};

const MAX_LINES: usize = 1_000;
static LOG: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
static RESTART: OnceLock<tokio::sync::Notify> = OnceLock::new();

pub fn request_restart() {
    RESTART.get_or_init(tokio::sync::Notify::new).notify_one();
}

pub async fn restart_requested() {
    RESTART
        .get_or_init(tokio::sync::Notify::new)
        .notified()
        .await;
}

pub fn record(level: &str, message: &str) {
    let line = format!(
        "{} | {level:<8} | {}",
        crate::download_service::now_rfc3339(),
        message.replace(['\r', '\n'], " ")
    );
    eprintln!("{line}");
    let Ok(mut lines) = LOG.get_or_init(|| Mutex::new(VecDeque::new())).lock() else {
        return;
    };
    if lines.len() == MAX_LINES {
        lines.pop_front();
    }
    lines.push_back(line.clone());
    if let Some(path) = std::env::var_os("OPENLIST_ANI_LOG_FILE") {
        let path = std::path::PathBuf::from(path);
        if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 1024 * 1024) {
            let _ = std::fs::rename(&path, path.with_extension("log.1"));
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

pub fn lines(limit: usize) -> Vec<String> {
    LOG.get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .map(|lines| {
            lines
                .iter()
                .skip(lines.len().saturating_sub(limit))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}
