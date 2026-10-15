//! Server start-up shared by the headless mode and the Windows desktop mode.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing_subscriber::EnvFilter;

use crate::api::{self, AppState};
use crate::config::{AppConfig, Settings};
use crate::emulation;
use crate::engine::{self, EngineDeps, EngineHandle, Timing};
use crate::library::Library;
use crate::tracker::HttpClient;

#[derive(Debug, Error)]
pub enum StartError {
    /// The UI address is already taken (typically by another ratiod).
    #[error("cannot listen on {0}: the address is already in use")]
    AddrInUse(SocketAddr),
    #[error("{0:#}")]
    Other(#[from] anyhow::Error),
}

/// A started server: the engine, the HTTP task and what must stay alive with them.
pub struct Running {
    pub engine: EngineHandle,
    pub url: String,
    /// Not a graceful axum shutdown: SSE streams never end on their own.
    pub server: JoinHandle<std::io::Result<()>>,
    _watcher: Option<notify::RecommendedWatcher>,
}

/// Logs to stderr (console modes).
pub fn init_stderr_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
}

/// Logs to `<dir>/ratiod.log`, recreated at each launch (Windows has no console). If the file cannot be
/// created, logs are dropped rather than blocking the start.
pub fn init_file_logging(dir: &Path) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    match std::fs::File::create(dir.join("ratiod.log")) {
        Ok(file) => {
            tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(Mutex::new(file)).init()
        }
        Err(_) => tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::sink).init(),
    }
}

/// Opens the library and the engine, then serves the API and the UI on `settings.bind`.
/// The UI address is bound first so that a second instance fails before it starts any announce.
pub async fn start(settings: &Settings, on_show: Option<Arc<dyn Fn() + Send + Sync>>) -> Result<Running, StartError> {
    let listener = TcpListener::bind(settings.bind).await.map_err(|error| match error.kind() {
        std::io::ErrorKind::AddrInUse => StartError::AddrInUse(settings.bind),
        _ => StartError::Other(anyhow::Error::new(error).context(format!("cannot listen on {}", settings.bind))),
    })?;
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
    let watcher = watch_library(&library, engine.clone());

    let app = api::router(AppState { engine: engine.clone(), library, clients_dir, on_show });
    let url = ui_url(settings.bind);
    tracing::info!("ratiod is ready on {url}");
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(Running { engine, url, server, _watcher: watcher })
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

/// Browsers cannot open `0.0.0.0`: point them at the loopback address instead.
pub fn ui_url(bind: SocketAddr) -> String {
    format!("http://{}", loopback_addr(bind))
}

/// The address to contact the UI server on from this machine.
pub fn loopback_addr(bind: SocketAddr) -> SocketAddr {
    let host = match bind.ip() {
        ip if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(host, bind.port())
}
