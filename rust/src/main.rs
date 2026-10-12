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
    let app = router(
        AppState::new(config.data_db, config.task_mementos_db)
            .with_auth(auth)
            .with_config_file(config_path),
    )
    .into_make_service_with_connect_info::<SocketAddr>();

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
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
