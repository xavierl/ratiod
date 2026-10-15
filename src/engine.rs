//! The orchestrator. One task owns every piece of seeding state; the API and
//! the folder watcher talk to it through `EngineHandle`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::{Semaphore, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::bandwidth::{Bandwidth, uploaded_during};
use crate::config::{AppConfig, ConfigError};
use crate::emulation::{self, AnnounceParams, ClientProfile, Event};
use crate::library::Library;
use crate::torrent::{InfoHash, MockedTorrent};
use crate::tracker::{AnnounceError, AnnounceResponse, HttpClient, announce_url};

const MAX_CONCURRENT_ANNOUNCES: usize = 20;

#[derive(Debug, Clone)]
pub struct Timing {
    pub tick: Duration,
    pub global_refresh: Duration,
    pub rescan: Duration,
    /// Retry delay until a tracker gives us an interval (5 s).
    pub initial_interval: Duration,
    pub stop_timeout: Duration,
    pub max_consecutive_fails: u8,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(1),
            global_refresh: Duration::from_secs(20 * 60),
            rescan: Duration::from_secs(30),
            initial_interval: Duration::from_secs(5),
            stop_timeout: Duration::from_secs(5),
            max_consecutive_fails: 5,
        }
    }
}

pub struct EngineDeps {
    pub library: Library,
    pub clients_dir: PathBuf,
    pub config_path: PathBuf,
    pub config: AppConfig,
    pub peer_port: u16,
    pub http: HttpClient,
    pub timing: Timing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Starting,
    Seeding,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub running: bool,
    pub error: Option<String>,
    pub client: String,
    /// Sum of the speeds assigned to torrents, bytes/s.
    pub global_speed: u64,
    pub session_uploaded: u64,
    pub queued: usize,
    pub config: AppConfig,
    pub torrents: Vec<TorrentView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentView {
    pub info_hash: String,
    pub name: String,
    pub size: u64,
    pub status: Status,
    pub speed: u64,
    pub uploaded: u64,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    pub next_announce_in: Option<u64>,
    pub consecutive_fails: u8,
    pub last_error: Option<String>,
}

impl Snapshot {
    fn idle(config: &AppConfig) -> Self {
        Self {
            running: false,
            error: None,
            client: config.client.clone(),
            global_speed: 0,
            session_uploaded: 0,
            queued: 0,
            config: config.clone(),
            torrents: Vec::new(),
        }
    }
}

enum Command {
    Start,
    Stop(oneshot::Sender<()>),
    UpdateConfig(AppConfig, oneshot::Sender<Result<(), ConfigError>>),
    Rescan,
    Delete(InfoHash, oneshot::Sender<bool>),
    AnnounceDone { info_hash: InfoHash, event: Event, uploaded: u64, result: Result<AnnounceResponse, AnnounceError> },
    Shutdown(oneshot::Sender<()>),
}

#[derive(Clone)]
pub struct EngineHandle {
    tx: mpsc::UnboundedSender<Command>,
    snapshot: watch::Receiver<Snapshot>,
}

impl EngineHandle {
    pub fn start(&self) {
        let _ = self.tx.send(Command::Start);
    }

    pub async fn stop(&self) {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Command::Stop(tx)).is_ok() {
            let _ = rx.await;
        }
    }

    pub async fn update_config(&self, config: AppConfig) -> Result<(), ConfigError> {
        let gone = || ConfigError { field: "config", message: "le moteur est arrêté".into() };
        let (tx, rx) = oneshot::channel();
        self.tx.send(Command::UpdateConfig(config, tx)).map_err(|_| gone())?;
        rx.await.unwrap_or_else(|_| Err(gone()))
    }

    pub fn rescan(&self) {
        let _ = self.tx.send(Command::Rescan);
    }

    pub async fn delete(&self, info_hash: InfoHash) -> bool {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Command::Delete(info_hash, tx)).is_err() {
            return false;
        }
        rx.await.unwrap_or(false)
    }

    /// Sends `stopped` to every tracker, then ends the engine task.
    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Command::Shutdown(tx)).is_ok() {
            let _ = rx.await;
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Snapshot> {
        self.snapshot.clone()
    }
}

struct Active {
    path: PathBuf,
    torrent: MockedTorrent,
    status: Status,
    uploaded: u64,
    seeders: Option<u32>,
    leechers: Option<u32>,
    interval: Duration,
    /// `None` while an announce is in flight.
    next_announce: Option<Instant>,
    next_event: Event,
    consecutive_fails: u8,
    last_error: Option<String>,
    tracker_index: usize,
}

enum Retire {
    /// Removed from the folder or deleted: send `stopped`, keep the file where it is.
    Stop,
    ArchiveAndStop,
    /// Too many failures: the tracker is unreachable, do not bother announcing.
    ArchiveSilently,
}

struct Engine {
    deps: EngineDeps,
    self_tx: mpsc::WeakUnboundedSender<Command>,
    snapshot_tx: watch::Sender<Snapshot>,
    running: bool,
    error: Option<String>,
    profile: Option<ClientProfile>,
    bandwidth: Bandwidth,
    files: BTreeMap<PathBuf, MockedTorrent>,
    warned_invalid: HashSet<PathBuf>,
    active: HashMap<InfoHash, Active>,
    session_uploaded: u64,
    last_tick: Instant,
    permits: Arc<Semaphore>,
}

pub fn spawn(deps: EngineDeps) -> EngineHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    let (snapshot_tx, snapshot_rx) = watch::channel(Snapshot::idle(&deps.config));
    let bandwidth = Bandwidth::new(deps.config.min_upload_rate, deps.config.max_upload_rate);
    let mut engine = Engine {
        deps,
        self_tx: tx.downgrade(),
        snapshot_tx,
        running: false,
        error: None,
        profile: None,
        bandwidth,
        files: BTreeMap::new(),
        warned_invalid: HashSet::new(),
        active: HashMap::new(),
        session_uploaded: 0,
        last_tick: Instant::now(),
        permits: Arc::new(Semaphore::new(MAX_CONCURRENT_ANNOUNCES)),
    };
    engine.rescan();
    engine.publish();
    tokio::spawn(engine.run(rx));
    EngineHandle { tx, snapshot: snapshot_rx }
}

fn current_tracker(t: &Active) -> String {
    t.torrent.trackers[t.tracker_index % t.torrent.trackers.len()].clone()
}

impl Engine {
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Command>) {
        let timing = self.deps.timing.clone();
        let mut tick = tokio::time::interval(timing.tick);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let start = tokio::time::Instant::now();
        let mut refresh = tokio::time::interval_at(start + timing.global_refresh, timing.global_refresh);
        let mut rescan = tokio::time::interval_at(start + timing.rescan, timing.rescan);
        loop {
            tokio::select! {
                command = rx.recv() => match command {
                    Some(command) => {
                        if !self.handle(command).await {
                            break;
                        }
                    }
                    None => break,
                },
                _ = tick.tick() => self.on_tick(Instant::now()),
                _ = refresh.tick() => self.bandwidth.refresh_global(),
                _ = rescan.tick() => self.rescan(),
            }
            self.publish();
        }
    }

    /// Returns `false` when the engine must exit.
    async fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::Start => self.start(),
            Command::Stop(done) => {
                self.stop_all().await;
                self.publish();
                let _ = done.send(());
            }
            Command::UpdateConfig(config, reply) => {
                let result = self.update_config(config).await;
                self.publish();
                let _ = reply.send(result);
            }
            Command::Rescan => self.rescan(),
            Command::Delete(info_hash, reply) => {
                let found = self.delete(&info_hash);
                self.publish();
                let _ = reply.send(found);
            }
            Command::AnnounceDone { info_hash, event, uploaded, result } => {
                self.on_announce_done(info_hash, event, uploaded, result);
            }
            Command::Shutdown(done) => {
                self.stop_all().await;
                self.publish();
                let _ = done.send(());
                return false;
            }
        }
        true
    }

    fn start(&mut self) {
        if self.running {
            return;
        }
        match emulation::load_profile(&self.deps.config.client, &self.deps.clients_dir) {
            Ok(profile) => {
                tracing::info!(client = profile.name(), "seeding started");
                self.profile = Some(profile);
                self.error = None;
                self.running = true;
                self.last_tick = Instant::now();
                self.bandwidth.set_rates(self.deps.config.min_upload_rate, self.deps.config.max_upload_rate);
                self.fill_slots();
            }
            Err(error) => {
                tracing::error!("cannot start seeding: {error}");
                self.error = Some(error.to_string());
            }
        }
    }

    async fn stop_all(&mut self) {
        let was_running = self.running;
        self.running = false;
        let stopping: Vec<Active> = self.active.drain().map(|(_, t)| t).collect();
        let mut pending = Vec::new();
        for t in &stopping {
            if t.next_event != Event::Started {
                let tracker = current_tracker(t);
                if let Some(handle) =
                    self.send_announce(t.torrent.info_hash, tracker, Event::Stopped, t.uploaded, false)
                {
                    pending.push(handle);
                }
            }
        }
        self.bandwidth.clear();
        let all = futures_util::future::join_all(pending);
        if tokio::time::timeout(self.deps.timing.stop_timeout, all).await.is_err() {
            tracing::warn!("some trackers did not answer the stopped announce in time");
        }
        if was_running {
            tracing::info!("seeding stopped");
        }
    }

    async fn update_config(&mut self, config: AppConfig) -> Result<(), ConfigError> {
        config.validate(&emulation::list_profiles(&self.deps.clients_dir))?;
        config.save(&self.deps.config_path).map_err(|e| ConfigError {
            field: "config",
            message: format!("impossible d'enregistrer la configuration : {e}"),
        })?;
        let was_running = self.running;
        if was_running {
            self.stop_all().await;
        }
        self.bandwidth.set_rates(config.min_upload_rate, config.max_upload_rate);
        self.deps.config = config;
        if was_running {
            self.start();
        }
        Ok(())
    }

    fn rescan(&mut self) {
        let scan = self.deps.library.scan();
        for (path, reason) in &scan.invalid {
            if self.warned_invalid.insert(path.clone()) {
                tracing::warn!(path = %path.display(), "ignoring invalid torrent: {reason}");
            }
        }
        self.files = scan.torrents.into_iter().collect();
        let gone: Vec<InfoHash> =
            self.active.iter().filter(|(_, t)| !self.files.contains_key(&t.path)).map(|(h, _)| *h).collect();
        for info_hash in gone {
            self.retire(&info_hash, Retire::Stop);
        }
        self.fill_slots();
    }

    fn delete(&mut self, info_hash: &InfoHash) -> bool {
        let paths: Vec<PathBuf> =
            self.files.iter().filter(|(_, t)| &t.info_hash == info_hash).map(|(p, _)| p.clone()).collect();
        if paths.is_empty() && !self.active.contains_key(info_hash) {
            return false;
        }
        for path in &paths {
            self.files.remove(path);
            if let Err(e) = self.deps.library.delete(path) {
                tracing::warn!(path = %path.display(), "cannot delete torrent file: {e}");
            }
        }
        self.retire(info_hash, Retire::Stop);
        self.fill_slots();
        true
    }

    /// Files in the folder that are not active, one per info-hash, in path order.
    fn queued(&self) -> impl Iterator<Item = (&PathBuf, &MockedTorrent)> {
        let mut seen: HashSet<InfoHash> = self.active.keys().copied().collect();
        self.files.iter().filter(move |(_, t)| seen.insert(t.info_hash))
    }

    fn fill_slots(&mut self) {
        if !self.running {
            return;
        }
        let free = (self.deps.config.simultaneous_seed as usize).saturating_sub(self.active.len());
        let picked: Vec<(PathBuf, MockedTorrent)> =
            self.queued().take(free).map(|(p, t)| (p.clone(), t.clone())).collect();
        let now = Instant::now();
        for (path, torrent) in picked {
            tracing::info!(torrent = %torrent.name, "seeding torrent");
            self.bandwidth.register(torrent.info_hash);
            self.active.insert(
                torrent.info_hash,
                Active {
                    path,
                    status: Status::Starting,
                    uploaded: 0,
                    seeders: None,
                    leechers: None,
                    interval: self.deps.timing.initial_interval,
                    next_announce: Some(now),
                    next_event: Event::Started,
                    consecutive_fails: 0,
                    last_error: None,
                    tracker_index: 0,
                    torrent,
                },
            );
        }
        self.dispatch_due(now);
    }

    fn retire(&mut self, info_hash: &InfoHash, how: Retire) {
        let Some(t) = self.active.remove(info_hash) else { return };
        self.bandwidth.unregister(info_hash);
        if matches!(how, Retire::Stop | Retire::ArchiveAndStop) && t.next_event != Event::Started {
            self.send_announce(*info_hash, current_tracker(&t), Event::Stopped, t.uploaded, false);
        }
        if matches!(how, Retire::ArchiveAndStop | Retire::ArchiveSilently) {
            self.files.remove(&t.path);
            match self.deps.library.archive(&t.path) {
                Ok(target) => tracing::info!(torrent = %t.torrent.name, to = %target.display(), "archived"),
                Err(e) => tracing::warn!(torrent = %t.torrent.name, "cannot archive: {e}"),
            }
        }
        self.fill_slots();
    }

    fn on_tick(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.last_tick);
        self.last_tick = now;
        for (info_hash, t) in self.active.iter_mut() {
            let delta = uploaded_during(self.bandwidth.speed_of(info_hash), elapsed);
            t.uploaded = t.uploaded.saturating_add(delta);
            self.session_uploaded = self.session_uploaded.saturating_add(delta);
        }
        self.dispatch_due(now);
    }

    fn dispatch_due(&mut self, now: Instant) {
        let due: Vec<InfoHash> =
            self.active.iter().filter(|(_, t)| t.next_announce.is_some_and(|at| at <= now)).map(|(h, _)| *h).collect();
        for info_hash in due {
            let Some(t) = self.active.get_mut(&info_hash) else { continue };
            t.next_announce = None;
            let (tracker, event, uploaded) = (current_tracker(t), t.next_event, t.uploaded);
            self.send_announce(info_hash, tracker, event, uploaded, true);
        }
    }

    /// Spawns one announce. With `report`, the result comes back as `AnnounceDone`.
    fn send_announce(
        &mut self,
        info_hash: InfoHash,
        tracker: String,
        event: Event,
        uploaded: u64,
        report: bool,
    ) -> Option<JoinHandle<()>> {
        let profile = self.profile.as_mut()?;
        let params = AnnounceParams { info_hash, uploaded, downloaded: 0, left: 0, port: self.deps.peer_port, event };
        let url = announce_url(&tracker, &profile.build_query(&params, Instant::now()));
        let headers = profile.headers().to_vec();
        let http = self.deps.http.clone();
        let permits = Arc::clone(&self.permits);
        let reply_to = self.self_tx.clone();
        tracing::debug!(%url, event = event.as_str(), "announce");
        Some(tokio::spawn(async move {
            let _permit = permits.acquire_owned().await;
            let result = http.announce(&url, &headers).await;
            if report && let Some(tx) = reply_to.upgrade() {
                let _ = tx.send(Command::AnnounceDone { info_hash, event, uploaded, result });
            }
        }))
    }

    fn on_announce_done(
        &mut self,
        info_hash: InfoHash,
        event: Event,
        uploaded: u64,
        result: Result<AnnounceResponse, AnnounceError>,
    ) {
        let now = Instant::now();
        let keep_zero = self.deps.config.keep_torrent_with_zero_leechers;
        let ratio_target = self.deps.config.upload_ratio_target;
        let max_fails = self.deps.timing.max_consecutive_fails;
        // Unknown hash: the torrent was retired while this announce was in flight.
        let Some(t) = self.active.get_mut(&info_hash) else { return };
        match result {
            Ok(response) => {
                t.consecutive_fails = 0;
                t.last_error = None;
                t.seeders = Some(response.seeders);
                t.leechers = Some(response.leechers);
                t.interval = Duration::from_secs(u64::from(response.interval));
                t.next_event = Event::None;
                t.status = Status::Seeding;
                t.next_announce = Some(now + t.interval);
                let no_peers = response.seeders < 1 || response.leechers < 1;
                let ratio_reached = event == Event::None
                    && ratio_target > 0.0
                    && uploaded as f64 >= f64::from(ratio_target) * t.torrent.size as f64;
                let name = t.torrent.name.clone();
                self.bandwidth.update_peers(&info_hash, response.seeders, response.leechers);
                if no_peers && !keep_zero {
                    tracing::info!(torrent = %name, "no peers left");
                    self.retire(&info_hash, Retire::ArchiveAndStop);
                } else if ratio_reached {
                    tracing::info!(torrent = %name, "ratio target reached");
                    self.retire(&info_hash, Retire::ArchiveAndStop);
                }
            }
            Err(error) => {
                t.consecutive_fails = t.consecutive_fails.saturating_add(1);
                t.last_error = Some(error.to_string());
                t.tracker_index = (t.tracker_index + 1) % t.torrent.trackers.len();
                tracing::warn!(torrent = %t.torrent.name, fails = t.consecutive_fails, "announce failed: {error}");
                if t.consecutive_fails >= max_fails {
                    self.retire(&info_hash, Retire::ArchiveSilently);
                } else {
                    t.next_announce = Some(now + t.interval);
                }
            }
        }
    }

    fn publish(&self) {
        let now = Instant::now();
        let mut torrents: Vec<TorrentView> = self
            .active
            .iter()
            .map(|(info_hash, t)| TorrentView {
                info_hash: hex::encode(info_hash),
                name: t.torrent.name.clone(),
                size: t.torrent.size,
                status: t.status,
                speed: self.bandwidth.speed_of(info_hash),
                uploaded: t.uploaded,
                seeders: t.seeders,
                leechers: t.leechers,
                next_announce_in: t.next_announce.map(|at| at.saturating_duration_since(now).as_secs()),
                consecutive_fails: t.consecutive_fails,
                last_error: t.last_error.clone(),
            })
            .collect();
        torrents.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.info_hash.cmp(&b.info_hash)));
        self.snapshot_tx.send_replace(Snapshot {
            running: self.running,
            error: self.error.clone(),
            client: self.deps.config.client.clone(),
            global_speed: self.bandwidth.total_speed(),
            session_uploaded: self.session_uploaded,
            queued: self.queued().count(),
            config: self.deps.config.clone(),
            torrents,
        });
    }
}
