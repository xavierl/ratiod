//! User configuration (`config.json`, backward-compatible) and process settings (env).

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
    /// kB/s (1 kB = 1000 bytes).
    pub min_upload_rate: u64,
    pub max_upload_rate: u64,
    pub simultaneous_seed: u32,
    pub client: String,
    pub keep_torrent_with_zero_leechers: bool,
    /// `-1` disables the ratio limit.
    pub upload_ratio_target: f32,
}

#[derive(Debug, Clone, PartialEq, Error, Serialize)]
#[error("{field}: {message}")]
pub struct ConfigError {
    pub field: &'static str,
    /// Stable code the UI translates.
    pub code: &'static str,
    pub message: String,
}

fn invalid(field: &'static str, code: &'static str, message: impl Into<String>) -> ConfigError {
    ConfigError { field, code, message: message.into() }
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
            return Err(invalid("maxUploadRate", "max_below_min", "must be greater than or equal to the minimum rate"));
        }
        if self.simultaneous_seed < 1 {
            return Err(invalid("simultaneousSeed", "seed_at_least_one", "must be at least 1"));
        }
        if !(self.upload_ratio_target == -1.0 || self.upload_ratio_target > 0.0) {
            return Err(invalid("uploadRatioTarget", "invalid_ratio", "must be -1 (disabled) or greater than 0"));
        }
        if !known_clients.contains(&self.client) {
            return Err(invalid("client", "unknown_client", format!("unknown client profile \"{}\"", self.client)));
        }
        Ok(())
    }

    /// Reads `path`, or writes the defaults there if it does not exist yet.
    /// An unreadable file is an error: the user's file is never overwritten.
    pub fn load_or_create(path: &Path, default_client: &str) -> anyhow::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("{} is not a valid ratiod config file", path.display())),
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
    /// Opens the web UI in the default browser once the server is listening.
    pub open_browser: bool,
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
        let data_dir = get("RATIOD_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| default_data_dir(&get));
        let peer_port = match get("RATIOD_PEER_PORT") {
            Some(v) => v.parse().context("RATIOD_PEER_PORT must be a port number (1-65535)")?,
            None => rand::rng().random_range(49152..=65534),
        };
        let autostart = get("RATIOD_AUTOSTART").is_none_or(|v| is_true(&v));
        // Windows users start the .exe by double-clicking it: show them the UI.
        let open_browser = get("RATIOD_OPEN_BROWSER").map_or(cfg!(windows), |v| is_true(&v));
        Ok(Self { bind, data_dir, peer_port, autostart, open_browser })
    }

    pub fn config_path(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    pub fn clients_dir(&self) -> PathBuf {
        self.data_dir.join("clients")
    }
}

fn is_true(value: &str) -> bool {
    !matches!(value.trim().to_ascii_lowercase().as_str(), "false" | "0" | "no" | "off")
}

/// Default data folder: on Windows the portable rule of [`choose_windows_data_dir`]; elsewhere
/// `$XDG_DATA_HOME/ratiod` or `~/.local/share/ratiod`; `./data` if none of these is known.
fn default_data_dir(get: &impl Fn(&str) -> Option<String>) -> PathBuf {
    let non_empty = |key: &str| get(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
        let appdata = non_empty("APPDATA").map(|base| base.join("ratiod"));
        return choose_windows_data_dir(exe_dir.as_deref(), appdata.as_deref(), |p| p.exists(), dir_is_writable);
    }
    let base = non_empty("XDG_DATA_HOME").or_else(|| non_empty("HOME").map(|home| home.join(".local").join("share")));
    base.map_or_else(|| PathBuf::from("data"), |base| base.join("ratiod"))
}

/// Portable data folder on Windows. In order: the "data" folder next to the exe if it exists; the per-user
/// folder (`appdata`, i.e. "%APPDATA%/ratiod", v0.1.0 installs) if it exists; "data" next to the exe if the
/// exe folder is writable; the per-user folder otherwise. `data` when neither location is known.
pub fn choose_windows_data_dir(
    exe_dir: Option<&Path>,
    appdata: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
    writable: impl Fn(&Path) -> bool,
) -> PathBuf {
    let portable = exe_dir.map(|dir| dir.join("data"));
    if let Some(dir) = portable.as_ref().filter(|dir| exists(dir)) {
        return dir.clone();
    }
    if let Some(dir) = appdata.filter(|dir| exists(dir)) {
        return dir.to_path_buf();
    }
    if let (Some(exe_dir), Some(dir)) = (exe_dir, portable.as_ref())
        && writable(exe_dir)
    {
        return dir.clone();
    }
    appdata.map(Path::to_path_buf).or(portable).unwrap_or_else(|| PathBuf::from("data"))
}

/// True when a file can be created in `dir` (read-only media, Program Files without admin rights…).
fn dir_is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".ratiod-write-test-{}", std::process::id()));
    let ok = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
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
    fn defaults_are_the_documented_values() {
        let c = valid();
        assert_eq!((c.min_upload_rate, c.max_upload_rate, c.simultaneous_seed), (30, 170, 200));
        assert!(c.keep_torrent_with_zero_leechers);
        assert_eq!(c.upload_ratio_target, -1.0);
        assert_eq!(c.validate(&clients()), Ok(()));
    }

    #[test]
    fn validation_names_the_bad_field() {
        type Case = (fn(&mut AppConfig), &'static str, &'static str);
        let cases: [Case; 5] = [
            (|c| c.max_upload_rate = 10, "maxUploadRate", "max_below_min"),
            (|c| c.simultaneous_seed = 0, "simultaneousSeed", "seed_at_least_one"),
            (|c| c.upload_ratio_target = 0.0, "uploadRatioTarget", "invalid_ratio"),
            (|c| c.upload_ratio_target = f32::NAN, "uploadRatioTarget", "invalid_ratio"),
            (|c| c.client = "nope.client".into(), "client", "unknown_client"),
        ];
        for (mutate, field, code) in cases {
            let mut c = valid();
            mutate(&mut c);
            let error = c.validate(&clients()).unwrap_err();
            assert_eq!((error.field, error.code), (field, code));
            assert!(error.message.is_ascii(), "messages are English: {}", error.message);
        }
        let mut ratio = valid();
        ratio.upload_ratio_target = 2.5;
        assert_eq!(ratio.validate(&clients()), Ok(()));
    }

    #[test]
    fn reads_existing_config() {
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
        // Windows: the portable folder next to the exe (this test binary) since no APPDATA is known.
        let expected_data = if cfg!(windows) {
            std::env::current_exe().unwrap().parent().unwrap().join("data")
        } else {
            PathBuf::from("data")
        };
        assert_eq!(empty.data_dir, expected_data);
        assert!((49152..=65534).contains(&empty.peer_port));
        assert!(empty.autostart);
        assert_eq!(empty.open_browser, cfg!(windows));
        assert_eq!(empty.config_path(), expected_data.join("config.json"));

        let env: HashMap<&str, &str> = HashMap::from([
            ("RATIOD_BIND", "0.0.0.0:8080"),
            ("RATIOD_DATA_DIR", "/data"),
            ("RATIOD_PEER_PORT", "49200"),
            ("RATIOD_AUTOSTART", "false"),
            ("RATIOD_OPEN_BROWSER", "true"),
            ("HOME", "/home/me"),
        ]);
        let s = Settings::from_lookup(|k| env.get(k).map(|v| v.to_string())).unwrap();
        assert_eq!(s.bind, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(s.data_dir, PathBuf::from("/data"));
        assert_eq!(s.peer_port, 49200);
        assert!(!s.autostart);
        assert!(s.open_browser);
    }

    #[test]
    fn data_dir_defaults_to_the_user_data_folder() {
        let env: HashMap<&str, &str> =
            HashMap::from([("HOME", "/home/me"), ("APPDATA", r"C:\Users\me\AppData\Roaming")]);
        let s = Settings::from_lookup(|k| env.get(k).map(|v| v.to_string())).unwrap();
        if cfg!(windows) {
            // Portable rule: next to the exe, or the per-user folder (see the choose_windows_data_dir tests).
            let appdata = PathBuf::from(r"C:\Users\me\AppData\Roaming").join("ratiod");
            let exe_data = std::env::current_exe().unwrap().parent().unwrap().join("data");
            assert!(s.data_dir == exe_data || s.data_dir == appdata, "{:?}", s.data_dir);
        } else {
            assert_eq!(s.data_dir, PathBuf::from("/home/me/.local/share/ratiod"));
            let xdg = Settings::from_lookup(|k| match k {
                "XDG_DATA_HOME" => Some("/xdg".into()),
                "HOME" => Some("/home/me".into()),
                _ => None,
            })
            .unwrap();
            assert_eq!(xdg.data_dir, PathBuf::from("/xdg/ratiod"));
        }
    }

    fn dirs(existing: &[&str]) -> impl Fn(&Path) -> bool {
        let existing: Vec<PathBuf> = existing.iter().map(PathBuf::from).collect();
        move |p| existing.iter().any(|e| e == p)
    }

    const EXE: &str = "C:/Apps/ratiod";
    const APPDATA: &str = "C:/Users/me/AppData/Roaming/ratiod";

    fn choose(existing: &[&str], writable: bool) -> PathBuf {
        choose_windows_data_dir(Some(Path::new(EXE)), Some(Path::new(APPDATA)), dirs(existing), |_| writable)
    }

    #[test]
    fn windows_data_dir_prefers_an_existing_portable_folder() {
        let portable = Path::new(EXE).join("data");
        assert_eq!(choose(&[portable.to_str().unwrap(), APPDATA], false), portable);
    }

    #[test]
    fn windows_data_dir_keeps_an_existing_v0_1_0_install() {
        assert_eq!(choose(&[APPDATA], true), PathBuf::from(APPDATA));
    }

    #[test]
    fn windows_data_dir_creates_next_to_a_writable_exe() {
        assert_eq!(choose(&[], true), Path::new(EXE).join("data"));
    }

    #[test]
    fn windows_data_dir_falls_back_to_appdata_when_the_exe_folder_is_read_only() {
        assert_eq!(choose(&[], false), PathBuf::from(APPDATA));
    }

    #[test]
    fn windows_data_dir_without_any_known_location() {
        assert_eq!(choose_windows_data_dir(None, None, |_| false, |_| true), PathBuf::from("data"));
    }

    #[test]
    fn settings_reject_bad_values() {
        assert!(Settings::from_lookup(|k| (k == "RATIOD_BIND").then(|| "nope".into())).is_err());
        assert!(Settings::from_lookup(|k| (k == "RATIOD_PEER_PORT").then(|| "70000".into())).is_err());
    }
}
