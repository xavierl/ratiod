//! HTTP API (no authentication: local use) and the embedded web UI.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures_util::{Stream, StreamExt};
use rust_embed::RustEmbed;
use serde::Serialize;
use tokio_stream::wrappers::WatchStream;

use crate::config::AppConfig;
use crate::emulation;
use crate::engine::EngineHandle;
use crate::library::{AddError, Library};

#[derive(RustEmbed)]
#[folder = "web/dist/"]
#[allow_missing = true]
struct Assets;

#[derive(Clone)]
pub struct AppState {
    pub engine: EngineHandle,
    pub library: Library,
    pub clients_dir: PathBuf,
    /// Brings the desktop window to the front; `None` outside the Windows desktop mode.
    pub on_show: Option<Arc<dyn Fn() + Send + Sync>>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/events", get(events))
        .route("/api/seed/start", post(start))
        .route("/api/seed/stop", post(stop))
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/clients", get(clients))
        .route("/api/torrents", post(upload))
        .route("/api/torrents/{info_hash}", delete(delete_torrent))
        .route("/api/desktop/show", post(desktop_show))
        .fallback(static_files)
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(state)
}

/// Used by a second launch of the desktop app to wake up the first one.
async fn desktop_show(State(state): State<AppState>) -> StatusCode {
    match &state.on_show {
        Some(show) => {
            show();
            StatusCode::NO_CONTENT
        }
        None => StatusCode::NOT_FOUND,
    }
}

async fn events(State(state): State<AppState>) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = WatchStream::new(state.engine.subscribe())
        .map(|snapshot| Ok(SseEvent::default().event("snapshot").json_data(&snapshot).unwrap_or_default()));
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn start(State(state): State<AppState>) -> StatusCode {
    state.engine.start();
    StatusCode::NO_CONTENT
}

async fn stop(State(state): State<AppState>) -> StatusCode {
    state.engine.stop().await;
    StatusCode::NO_CONTENT
}

async fn get_config(State(state): State<AppState>) -> Json<AppConfig> {
    Json(state.engine.snapshot().config)
}

async fn put_config(State(state): State<AppState>, Json(config): Json<AppConfig>) -> Response {
    match state.engine.update_config(config.clone()).await {
        Ok(()) => Json(config).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(error)).into_response(),
    }
}

async fn clients(State(state): State<AppState>) -> Json<Vec<String>> {
    Json(emulation::list_profiles(&state.clients_dir))
}

#[derive(Serialize)]
struct UploadResult {
    file: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

async fn upload(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    let mut results = Vec::new();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => return (StatusCode::BAD_REQUEST, error.body_text()).into_response(),
        };
        if field.name() != Some("files") {
            continue;
        }
        let file = field.file_name().unwrap_or("sans-nom.torrent").to_owned();
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => return (StatusCode::BAD_REQUEST, error.body_text()).into_response(),
        };
        let library = state.library.clone();
        let name = file.clone();
        let outcome = tokio::task::spawn_blocking(move || library.add(&name, &bytes)).await;
        results.push(match outcome {
            Ok(Ok(_)) => UploadResult { file, status: "added", message: None },
            Ok(Err(AddError::Duplicate)) => {
                UploadResult { file, status: "duplicate", message: Some(AddError::Duplicate.to_string()) }
            }
            Ok(Err(error)) => UploadResult { file, status: "invalid", message: Some(error.to_string()) },
            Err(error) => UploadResult { file, status: "invalid", message: Some(error.to_string()) },
        });
    }
    if results.iter().any(|r| r.status == "added") {
        state.engine.rescan();
    }
    Json(results).into_response()
}

async fn delete_torrent(State(state): State<AppState>, Path(info_hash): Path<String>) -> StatusCode {
    let mut hash = [0u8; 20];
    if hex::decode_to_slice(&info_hash, &mut hash).is_err() {
        return StatusCode::NOT_FOUND;
    }
    if state.engine.delete(hash).await { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }
}

async fn static_files(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if !path.is_empty()
        && let Some(file) = Assets::get(path)
    {
        return asset(path, file);
    }
    if path.starts_with("api/") || path.rsplit('/').next().is_some_and(|last| last.contains('.')) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match Assets::get("index.html") {
        Some(file) => asset("index.html", file),
        None => (StatusCode::SERVICE_UNAVAILABLE, "Interface non compilée : lancez `npm run build` dans web/.")
            .into_response(),
    }
}

fn asset(path: &str, file: rust_embed::EmbeddedFile) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    ([(header::CONTENT_TYPE, mime.essence_str().to_owned()), (header::CACHE_CONTROL, cache.to_owned())], file.data)
        .into_response()
}
