//! Refresh policies deciding when a peer-id or key is regenerated.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::algorithm::{Algorithm, AlgorithmConfig, AlgorithmError};
use crate::torrent::InfoHash;

const PERSISTENT_EVICTION: Duration = Duration::from_secs(120 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    None,
    Started,
    Stopped,
}

impl Event {
    pub fn as_str(self) -> &'static str {
        match self {
            Event::None => "",
            Event::Started => "started",
            Event::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RefreshOn {
    Never,
    Always,
    Timed,
    TimedOrAfterStartedAnnounce,
    TorrentPersistent,
    TorrentVolatile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Casing {
    Upper,
    Lower,
    #[default]
    None,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratorConfig {
    pub algorithm: AlgorithmConfig,
    pub refresh_on: RefreshOn,
    #[serde(default)]
    pub refresh_every: Option<u64>,
    #[serde(default)]
    pub key_case: Casing,
    #[serde(default)]
    pub should_url_encode: bool,
}

pub struct Generator {
    algorithm: Algorithm,
    refresh_on: RefreshOn,
    refresh_every: Duration,
    casing: Casing,
    current: Option<(Vec<u8>, Instant)>,
    per_torrent: HashMap<InfoHash, (Vec<u8>, Instant)>,
}

impl Generator {
    pub fn new(cfg: &GeneratorConfig) -> Result<Self, AlgorithmError> {
        let refresh_every = cfg.refresh_every.unwrap_or(0);
        let timed = matches!(cfg.refresh_on, RefreshOn::Timed | RefreshOn::TimedOrAfterStartedAnnounce);
        if timed && refresh_every < 1 {
            return Err(AlgorithmError("refreshEvery doit être supérieur à 0".into()));
        }
        Ok(Self {
            algorithm: Algorithm::compile(&cfg.algorithm)?,
            refresh_on: cfg.refresh_on,
            refresh_every: Duration::from_secs(refresh_every),
            casing: cfg.key_case,
            current: None,
            per_torrent: HashMap::new(),
        })
    }

    /// A fresh value that does not affect the generator state (used for validation).
    pub fn sample(&self) -> Vec<u8> {
        generate(&self.algorithm, self.casing)
    }

    pub fn get(&mut self, info_hash: &InfoHash, event: Event, now: Instant) -> Vec<u8> {
        match self.refresh_on {
            RefreshOn::Never => self.global(now, false),
            RefreshOn::Always => generate(&self.algorithm, self.casing),
            RefreshOn::Timed => self.global(now, true),
            RefreshOn::TimedOrAfterStartedAnnounce => {
                let value = self.global(now, true);
                if event == Event::Started {
                    let fresh = generate(&self.algorithm, self.casing);
                    if let Some((current, _)) = self.current.as_mut() {
                        *current = fresh;
                    }
                }
                value
            }
            RefreshOn::TorrentPersistent => {
                self.per_torrent.retain(|_, (_, last_access)| now.duration_since(*last_access) < PERSISTENT_EVICTION);
                let (algorithm, casing) = (&self.algorithm, self.casing);
                let entry = self.per_torrent.entry(*info_hash).or_insert_with(|| (generate(algorithm, casing), now));
                entry.1 = now;
                entry.0.clone()
            }
            RefreshOn::TorrentVolatile => {
                let (algorithm, casing) = (&self.algorithm, self.casing);
                let value =
                    self.per_torrent.entry(*info_hash).or_insert_with(|| (generate(algorithm, casing), now)).0.clone();
                if event == Event::Stopped {
                    self.per_torrent.remove(info_hash);
                }
                value
            }
        }
    }

    fn global(&mut self, now: Instant, timed: bool) -> Vec<u8> {
        if let Some((value, generated_at)) = &self.current {
            if !timed || now.duration_since(*generated_at) < self.refresh_every {
                return value.clone();
            }
        }
        let value = generate(&self.algorithm, self.casing);
        self.current = Some((value.clone(), now));
        value
    }
}

fn generate(algorithm: &Algorithm, casing: Casing) -> Vec<u8> {
    let mut value = algorithm.generate();
    match casing {
        Casing::Upper => value.make_ascii_uppercase(),
        Casing::Lower => value.make_ascii_lowercase(),
        Casing::None => {}
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const A: InfoHash = [1; 20];
    const B: InfoHash = [2; 20];

    fn generator(refresh_on: &str, extra: &str) -> Generator {
        let json = format!(
            r#"{{"algorithm":{{"type":"HASH","length":16}},"refreshOn":"{refresh_on}"{extra}}}"#
        );
        Generator::new(&serde_json::from_str(&json).unwrap()).unwrap()
    }

    #[test]
    fn never_keeps_one_value() {
        let mut g = generator("NEVER", "");
        let t0 = Instant::now();
        let v = g.get(&A, Event::Started, t0);
        assert_eq!(g.get(&B, Event::None, t0 + Duration::from_secs(99_999)), v);
    }

    #[test]
    fn always_changes_every_call() {
        let mut g = generator("ALWAYS", "");
        let t0 = Instant::now();
        assert_ne!(g.get(&A, Event::None, t0), g.get(&A, Event::None, t0));
    }

    #[test]
    fn timed_refreshes_after_period() {
        let mut g = generator("TIMED", r#","refreshEvery":10"#);
        let t0 = Instant::now();
        let v = g.get(&A, Event::None, t0);
        assert_eq!(g.get(&B, Event::None, t0 + Duration::from_secs(9)), v);
        assert_ne!(g.get(&A, Event::None, t0 + Duration::from_secs(10)), v);
    }

    #[test]
    fn timed_or_after_started_rotates_after_started() {
        let mut g = generator("TIMED_OR_AFTER_STARTED_ANNOUNCE", r#","refreshEvery":3600"#);
        let t0 = Instant::now();
        let first = g.get(&A, Event::Started, t0);
        let second = g.get(&A, Event::None, t0);
        assert_ne!(first, second, "value must rotate after a started announce");
        assert_eq!(g.get(&B, Event::None, t0), second);
    }

    #[test]
    fn torrent_persistent_is_per_torrent_and_evicted_when_idle() {
        let mut g = generator("TORRENT_PERSISTENT", "");
        let t0 = Instant::now();
        let a = g.get(&A, Event::Started, t0);
        assert_ne!(g.get(&B, Event::Started, t0), a);
        assert_eq!(g.get(&A, Event::Stopped, t0 + Duration::from_secs(60)), a);
        assert_ne!(g.get(&A, Event::None, t0 + Duration::from_secs(60 + 121 * 60)), a);
    }

    #[test]
    fn torrent_volatile_forgets_after_stopped() {
        let mut g = generator("TORRENT_VOLATILE", "");
        let t0 = Instant::now();
        let a = g.get(&A, Event::Started, t0);
        assert_eq!(g.get(&A, Event::None, t0), a);
        assert_eq!(g.get(&A, Event::Stopped, t0), a);
        assert_ne!(g.get(&A, Event::Started, t0), a);
    }

    #[test]
    fn applies_key_case() {
        let mut g = generator("ALWAYS", r#","keyCase":"lower""#);
        let v = g.get(&A, Event::None, Instant::now());
        assert!(v.iter().all(|c| !c.is_ascii_uppercase()));
    }

    #[test]
    fn timed_requires_refresh_every() {
        let json = r#"{"algorithm":{"type":"HASH","length":8},"refreshOn":"TIMED"}"#;
        assert!(Generator::new(&serde_json::from_str(json).unwrap()).is_err());
    }

    #[test]
    fn event_names() {
        assert_eq!(Event::Started.as_str(), "started");
        assert_eq!(Event::Stopped.as_str(), "stopped");
        assert_eq!(Event::None.as_str(), "");
    }
}
