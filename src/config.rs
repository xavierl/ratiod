//! User configuration (`config.json`, Joal-compatible) and process settings (env).

use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    /// kB/s (1 kB = 1000 bytes), like Joal.
    pub min_upload_rate: u64,
    pub max_upload_rate: u64,
    pub simultaneous_seed: u32,
    pub client: String,
    pub keep_torrent_with_zero_leechers: bool,
    /// `-1` disables the ratio limit.
    pub upload_ratio_target: f32,
}

#[derive(Debug, Clone, PartialEq, Error, Serialize)]
#[error("{field} : {message}")]
pub struct ConfigError {
    pub field: &'static str,
    pub message: String,
}

fn invalid(field: &'static str, message: impl Into<String>) -> ConfigError {
    ConfigError { field, message: message.into() }
}

impl AppConfig {
    pub fn with_client(client: String) -> Self {
        Self {
            min_upload_rate: 30,
            max_upload_rate: 170,
            simultaneous_seed: 200,
            client,
            keep_torrent_with_zero_leechers: true,
            upload_ratio_target: -1.0,
        }
    }

    pub fn validate(&self, known_clients: &[String]) -> Result<(), ConfigError> {
        if self.max_upload_rate < self.min_upload_rate {
            return Err(invalid("maxUploadRate", "doit être supérieur ou égal au débit minimum"));
        }
        if self.simultaneous_seed < 1 {
            return Err(invalid("simultaneousSeed", "doit être au moins 1"));
        }
        if !(self.upload_ratio_target == -1.0 || self.upload_ratio_target > 0.0) {
            return Err(invalid("uploadRatioTarget", "doit être -1 (désactivé) ou supérieur à 0"));
        }
        if !known_clients.contains(&self.client) {
            return Err(invalid("client", format!("profil client inconnu « {} »", self.client)));
        }
        Ok(())
    }

    /// Reads `path`, or writes the defaults there if it does not exist yet.
    /// An unreadable file is an error: the user's file is never overwritten.
    pub fn load_or_create(path: &Path, default_client: &str) -> anyhow::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("{} is not a valid ratiod/Joal config file", path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let config = Self::with_client(default_client.to_owned());
                config.save(path).with_context(|| format!("cannot write {}", path.display()))?;
                Ok(config)
            }
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    /// Atomic write: temp file then rename.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub peer_port: u16,
    pub autostart: bool,
}

impl Settings {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let bind = get("RATIOD_BIND")
            .unwrap_or_else(|| "127.0.0.1:5081".into())
            .parse()
            .context("RATIOD_BIND must look like 0.0.0.0:5081")?;
        let data_dir = get("RATIOD_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("data"));
        let peer_port = match get("RATIOD_PEER_PORT") {
            Some(v) => v.parse().context("RATIOD_PEER_PORT must be a port number (1-65535)")?,
            None => rand::rng().random_range(49152..=65534),
        };
        let autostart = match get("RATIOD_AUTOSTART") {
            None => true,
            Some(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "false" | "0" | "no" | "off"),
        };
        Ok(Self { bind, data_dir, peer_port, autostart })
    }

    pub fn config_path(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    pub fn clients_dir(&self) -> PathBuf {
        self.data_dir.join("clients")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn clients() -> Vec<String> {
        vec!["qbittorrent-5.2.4.client".to_owned()]
    }

    fn valid() -> AppConfig {
        AppConfig::with_client("qbittorrent-5.2.4.client".into())
    }

    #[test]
    fn defaults_match_joal() {
        let c = valid();
        assert_eq!((c.min_upload_rate, c.max_upload_rate, c.simultaneous_seed), (30, 170, 200));
        assert!(c.keep_torrent_with_zero_leechers);
        assert_eq!(c.upload_ratio_target, -1.0);
        assert_eq!(c.validate(&clients()), Ok(()));
    }

    #[test]
    fn validation_names_the_bad_field() {
        type Case = (fn(&mut AppConfig), &'static str);
        let cases: [Case; 5] = [
            (|c| c.max_upload_rate = 10, "maxUploadRate"),
            (|c| c.simultaneous_seed = 0, "simultaneousSeed"),
            (|c| c.upload_ratio_target = 0.0, "uploadRatioTarget"),
            (|c| c.upload_ratio_target = f32::NAN, "uploadRatioTarget"),
            (|c| c.client = "nope.client".into(), "client"),
        ];
        for (mutate, field) in cases {
            let mut c = valid();
            mutate(&mut c);
            assert_eq!(c.validate(&clients()).unwrap_err().field, field);
        }
        let mut ratio = valid();
        ratio.upload_ratio_target = 2.5;
        assert_eq!(ratio.validate(&clients()), Ok(()));
    }

    #[test]
    fn reads_existing_joal_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"minUploadRate":10,"maxUploadRate":20,"simultaneousSeed":5,"client":"utorrent-3.5.0_43916.client","keepTorrentWithZeroLeechers":false,"uploadRatioTarget":-1.0,"someFutureField":1}"#,
        )
        .unwrap();
        let c = AppConfig::load_or_create(&path, "ignored").unwrap();
        assert_eq!(c.client, "utorrent-3.5.0_43916.client");
        assert_eq!(c.simultaneous_seed, 5);
        assert!(!c.keep_torrent_with_zero_leechers);
    }

    #[test]
    fn creates_missing_config_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let c = AppConfig::load_or_create(&path, "qbittorrent-5.2.4.client").unwrap();
        assert_eq!(c, valid());
        let on_disk: AppConfig = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(on_disk, valid());
    }

    #[test]
    fn invalid_config_file_is_an_error_and_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(AppConfig::load_or_create(&path, "x").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }

    #[test]
    fn settings_defaults_and_overrides() {
        let empty = Settings::from_lookup(|_| None).unwrap();
        assert_eq!(empty.bind, "127.0.0.1:5081".parse().unwrap());
        assert_eq!(empty.data_dir, PathBuf::from("data"));
        assert!((49152..=65534).contains(&empty.peer_port));
        assert!(empty.autostart);
        assert_eq!(empty.config_path(), PathBuf::from("data").join("config.json"));

        let env: HashMap<&str, &str> = HashMap::from([
            ("RATIOD_BIND", "0.0.0.0:8080"),
            ("RATIOD_DATA_DIR", "/data"),
            ("RATIOD_PEER_PORT", "49200"),
            ("RATIOD_AUTOSTART", "false"),
        ]);
        let s = Settings::from_lookup(|k| env.get(k).map(|v| v.to_string())).unwrap();
        assert_eq!(s.bind, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(s.data_dir, PathBuf::from("/data"));
        assert_eq!(s.peer_port, 49200);
        assert!(!s.autostart);
    }

    #[test]
    fn settings_reject_bad_values() {
        assert!(Settings::from_lookup(|k| (k == "RATIOD_BIND").then(|| "nope".into())).is_err());
        assert!(Settings::from_lookup(|k| (k == "RATIOD_PEER_PORT").then(|| "70000".into())).is_err());
    }
}
