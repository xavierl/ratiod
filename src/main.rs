use std::time::Duration;

use anyhow::Context;
use ratiod::api::{self, AppState};
use ratiod::config::{AppConfig, Settings};
use ratiod::emulation;
use ratiod::engine::{self, EngineDeps, EngineHandle, Timing};
use ratiod::library::Library;
use ratiod::tracker::HttpClient;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let settings = Settings::from_env()?;
    std::fs::create_dir_all(&settings.data_dir)
        .with_context(|| format!("cannot create {}", settings.data_dir.display()))?;
    let library = Library::open(&settings.data_dir).context("cannot open the torrents folder")?;
    let clients_dir = settings.clients_dir();
    let config = AppConfig::load_or_create(&settings.config_path(), &emulation::default_client(&clients_dir))?;

    spawn_peer_listener(settings.peer_port).await?;
    let engine = engine::spawn(EngineDeps {
        library: library.clone(),
        clients_dir: clients_dir.clone(),
        config_path: settings.config_path(),
        config,
        peer_port: settings.peer_port,
        http: HttpClient::new(),
        timing: Timing::default(),
    });
    if settings.autostart {
        engine.start();
    }
    let _watcher = watch_library(&library, engine.clone());

    let app = api::router(AppState { engine: engine.clone(), library, clients_dir });
    let listener =
        TcpListener::bind(settings.bind).await.with_context(|| format!("cannot listen on {}", settings.bind))?;
    tracing::info!("ratiod is ready on http://{}", settings.bind);

    // Not a graceful axum shutdown: SSE streams never end on their own.
    tokio::select! {
        result = axum::serve(listener, app) => result.context("HTTP server failed")?,
        () = shutdown_signal() => {}
    }
    engine.shutdown().await;
    tracing::info!("bye");
    Ok(())
}

/// Accepts and drops peer connections so the announced port looks open.
async fn spawn_peer_listener(port: u16) -> anyhow::Result<()> {
    let listener =
        TcpListener::bind(("0.0.0.0", port)).await.with_context(|| format!("cannot listen on peer port {port}"))?;
    tracing::info!("announcing peer port {port}");
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((socket, _)) => drop(socket),
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    });
    Ok(())
}

/// Debounced folder watching; the engine also rescans every 30 s as a fallback.
fn watch_library(library: &Library, engine: EngineHandle) -> Option<notify::RecommendedWatcher> {
    let (tx, mut rx) = mpsc::unbounded_channel::<()>();
    let watcher = match library.watch(move || {
        let _ = tx.send(());
    }) {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!("folder watching unavailable ({error}); relying on periodic rescans");
            return None;
        }
    };
    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            tokio::time::sleep(Duration::from_millis(500)).await;
            while rx.try_recv().is_ok() {}
            engine.rescan();
        }
    });
    Some(watcher)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("shutting down, sending stopped announces");
}
