#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use ratiod::config::AppConfig;
use ratiod::engine::{self, EngineDeps, EngineHandle, Timing};
use ratiod::library::Library;
use ratiod::testutil;
use ratiod::tracker::HttpClient;

#[derive(Clone, Copy, Debug)]
pub enum Behavior {
    Ok { interval: u32, complete: u32, incomplete: u32 },
    Fail,
    Hang,
}

pub fn ok(interval: u32, complete: u32, incomplete: u32) -> Behavior {
    Behavior::Ok { interval, complete, incomplete }
}

type Requests = Arc<Mutex<Vec<HashMap<String, String>>>>;

#[derive(Clone)]
pub struct FakeTracker {
    pub url: String,
    requests: Requests,
    behavior: Arc<Mutex<Behavior>>,
}

impl FakeTracker {
    pub async fn start(behavior: Behavior) -> Self {
        let requests: Requests = Arc::default();
        let behavior = Arc::new(Mutex::new(behavior));
        let app = Router::new().route("/announce", get(announce)).with_state((requests.clone(), behavior.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/announce", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { url, requests, behavior }
    }

    pub fn set(&self, behavior: Behavior) {
        *self.behavior.lock().unwrap() = behavior;
    }

    /// Query parameters of every announce received, raw (not percent-decoded).
    pub fn requests(&self) -> Vec<HashMap<String, String>> {
        self.requests.lock().unwrap().clone()
    }

    /// `event` of every announce, `""` for regular announces.
    pub fn events(&self) -> Vec<String> {
        self.requests().iter().map(|r| r.get("event").cloned().unwrap_or_default()).collect()
    }
}

async fn announce(
    State((requests, behavior)): State<(Requests, Arc<Mutex<Behavior>>)>,
    RawQuery(query): RawQuery,
) -> Response {
    let params = query
        .unwrap_or_default()
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    requests.lock().unwrap().push(params);
    let current = *behavior.lock().unwrap();
    match current {
        Behavior::Ok { interval, complete, incomplete } => {
            testutil::tracker_response(interval, complete, incomplete).into_response()
        }
        Behavior::Fail => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        Behavior::Hang => {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            StatusCode::OK.into_response()
        }
    }
}

pub fn test_timing() -> Timing {
    Timing {
        tick: Duration::from_millis(50),
        global_refresh: Duration::from_secs(3600),
        rescan: Duration::from_secs(3600),
        initial_interval: Duration::from_millis(100),
        stop_timeout: Duration::from_millis(500),
        max_consecutive_fails: 5,
    }
}

/// Default config with a fixed 1 MB/s global speed.
pub fn test_config() -> AppConfig {
    let mut config = AppConfig::with_client("qbittorrent-5.2.4.client".into());
    config.min_upload_rate = 1000;
    config.max_upload_rate = 1000;
    config
}

pub fn torrent(name: &str, size: u64, tracker: &FakeTracker) -> Vec<u8> {
    testutil::torrent_file(name, size, &tracker.url)
}

pub struct Harness {
    pub dir: tempfile::TempDir,
    pub library: Library,
    pub engine: EngineHandle,
}

/// Writes `torrents` (file name, bytes) into a fresh data dir, then spawns the engine.
pub async fn harness(config: AppConfig, torrents: &[(&str, Vec<u8>)]) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(dir.path()).unwrap();
    for (name, bytes) in torrents {
        std::fs::write(library.torrents_dir().join(name), bytes).unwrap();
    }
    let engine = engine::spawn(EngineDeps {
        library: library.clone(),
        clients_dir: dir.path().join("clients"),
        config_path: dir.path().join("config.json"),
        config,
        peer_port: 49200,
        http: HttpClient::new(),
        timing: test_timing(),
    });
    Harness { dir, library, engine }
}

/// Polls `check` every 20 ms; panics after 5 s.
pub async fn wait_for(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !check() {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

pub fn hash_from_hex(hex_hash: &str) -> [u8; 20] {
    let mut hash = [0u8; 20];
    hex::decode_to_slice(hex_hash, &mut hash).unwrap();
    hash
}
