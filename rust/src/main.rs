use std::{error::Error, net::SocketAddr, path::PathBuf};

use openlist_ani_rs::{AppConfig, AppState, AuthSettings, router};
use tokio::net::TcpListener;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn Error>> {
    let config_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config.toml"));
    let config = AppConfig::load(&config_path)?;
    let auth = AuthSettings::from_env()?;
    let listener = TcpListener::bind(config.bind).await?;
    let state = AppState::new(config.data_db, config.task_mementos_db)
        .with_auth(auth)
        .with_config_file(config_path);
    let background = std::env::var("OPENLIST_ANI_BACKGROUND_ENABLED").as_deref() != Ok("0");
    if background || std::env::var("OPENLIST_ANI_RECOVER_DOWNLOADS_ENABLED").as_deref() == Ok("1") {
        state
            .recover_download_workers()
            .await
            .map_err(std::io::Error::other)?;
    }
    if background {
        state.start_periodic_rss_scanner();
    }
    openlist_ani_rs::runtime::record("INFO", "OpenList-Ani Rust service started");
    let app = router(state).into_make_service_with_connect_info::<SocketAddr>();

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = terminate.recv() => {},
        _ = openlist_ani_rs::runtime::restart_requested() => {},
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = openlist_ani_rs::runtime::restart_requested() => {},
    }
}
