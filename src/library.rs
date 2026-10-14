//! The torrent folder: `torrents/` holds what to seed, `torrents/archived/`
//! what is done. Uploads and manual drops both land in `torrents/`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use thiserror::Error;

use crate::torrent::{MockedTorrent, TorrentError};

pub const MAX_TORRENT_SIZE: usize = 10 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum AddError {
    #[error("{0}")]
    Invalid(#[from] TorrentError),
    #[error("fichier trop volumineux (10 Mo maximum)")]
    TooLarge,
    #[error("ce torrent est déjà présent")]
    Duplicate,
    #[error("écriture impossible : {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Default)]
pub struct Scan {
    pub torrents: Vec<(PathBuf, MockedTorrent)>,
    pub invalid: Vec<(PathBuf, String)>,
}

#[derive(Debug, Clone)]
pub struct Library {
    torrents_dir: PathBuf,
    archive_dir: PathBuf,
    /// Serializes uploads so two copies of one torrent cannot both pass the duplicate check.
    add_lock: Arc<Mutex<()>>,
}

impl Library {
    pub fn open(data_dir: &Path) -> io::Result<Self> {
        let torrents_dir = data_dir.join("torrents");
        let archive_dir = torrents_dir.join("archived");
        fs::create_dir_all(&archive_dir)?;
        Ok(Self { torrents_dir, archive_dir, add_lock: Arc::default() })
    }

    pub fn torrents_dir(&self) -> &Path {
        &self.torrents_dir
    }

    pub fn archive_dir(&self) -> &Path {
        &self.archive_dir
    }

    pub fn scan(&self) -> Scan {
        let mut scan = Scan::default();
        let entries = match fs::read_dir(&self.torrents_dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!("cannot read {}: {e}", self.torrents_dir.display());
                return scan;
            }
        };
        for path in entries.flatten().map(|e| e.path()) {
            if !path.is_file() || !is_torrent_file(&path) {
                continue;
            }
            let parsed = fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| MockedTorrent::parse(&bytes).map_err(|e| e.to_string()));
            match parsed {
                Ok(torrent) => scan.torrents.push((path, torrent)),
                Err(reason) => scan.invalid.push((path, reason)),
            }
        }
        scan.torrents.sort_by(|a, b| a.0.cmp(&b.0));
        scan
    }

    pub fn add(&self, filename: &str, bytes: &[u8]) -> Result<(PathBuf, MockedTorrent), AddError> {
        if bytes.len() > MAX_TORRENT_SIZE {
            return Err(AddError::TooLarge);
        }
        let torrent = MockedTorrent::parse(bytes)?;
        let _guard = self.add_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.scan().torrents.iter().any(|(_, t)| t.info_hash == torrent.info_hash) {
            return Err(AddError::Duplicate);
        }
        let path = unique_path(&self.torrents_dir, &sanitize_filename(filename));
        let tmp = self.torrents_dir.join(format!(".{}.part", torrent.info_hash_hex()));
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, &path)?;
        Ok((path, torrent))
    }

    pub fn archive(&self, path: &Path) -> io::Result<PathBuf> {
        let name =
            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "torrent.torrent".to_owned());
        let target = unique_path(&self.archive_dir, &name);
        fs::rename(path, &target)?;
        Ok(target)
    }

    pub fn delete(&self, path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    /// Calls `on_change` for any event in `torrents/`. Keep the watcher alive.
    pub fn watch(&self, on_change: impl Fn() + Send + 'static) -> notify::Result<RecommendedWatcher> {
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if event.is_ok() {
                on_change();
            }
        })?;
        watcher.watch(&self.torrents_dir, RecursiveMode::NonRecursive)?;
        Ok(watcher)
    }
}

fn is_torrent_file(path: &Path) -> bool {
    let hidden = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
    !hidden && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("torrent"))
}

/// Keeps only the base name, replaces risky characters and forces `.torrent`.
pub fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let stem =
        if base.to_ascii_lowercase().ends_with(".torrent") { &base[..base.len() - ".torrent".len()] } else { base };
    let clean: String =
        stem.chars().map(|c| if c.is_alphanumeric() || " ._-()[]".contains(c) { c } else { '_' }).take(150).collect();
    let clean = clean.trim_matches(|c: char| c == '.' || c == ' ');
    format!("{}.torrent", if clean.is_empty() { "torrent" } else { clean })
}

fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let stem = filename.strip_suffix(".torrent").unwrap_or(filename);
    (1u32..)
        .map(|n| dir.join(format!("{stem}-{n}.torrent")))
        .find(|p| !p.exists())
        .expect("an unbounded range always yields a free name")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::torrent_file;
    use std::time::Duration;

    fn library() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        (dir, lib)
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn open_creates_folders() {
        let (dir, lib) = library();
        assert!(dir.path().join("torrents").is_dir());
        assert!(dir.path().join("torrents").join("archived").is_dir());
        assert_eq!(lib.archive_dir(), dir.path().join("torrents").join("archived"));
    }

    #[test]
    fn add_writes_valid_torrent() {
        let (_dir, lib) = library();
        let bytes = torrent_file("a", 10, "http://t/announce");
        let (path, torrent) = lib.add("a.torrent", &bytes).unwrap();
        assert_eq!(path, lib.torrents_dir().join("a.torrent"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(torrent.name, "a");
        assert_eq!(files_in(lib.torrents_dir()), ["a.torrent"]);
    }

    #[test]
    fn add_rejects_duplicate() {
        let (_dir, lib) = library();
        let bytes = torrent_file("a", 10, "http://t/announce");
        lib.add("a.torrent", &bytes).unwrap();
        assert!(matches!(lib.add("copy.torrent", &bytes), Err(AddError::Duplicate)));
        assert_eq!(files_in(lib.torrents_dir()), ["a.torrent"]);
    }

    #[test]
    fn add_rejects_invalid_and_oversized_without_writing() {
        let (_dir, lib) = library();
        assert!(matches!(lib.add("bad.torrent", b"<html>"), Err(AddError::Invalid(_))));
        assert!(matches!(lib.add("big.torrent", &vec![b'x'; MAX_TORRENT_SIZE + 1]), Err(AddError::TooLarge)));
        assert!(files_in(lib.torrents_dir()).is_empty());
    }

    #[test]
    fn add_sanitizes_hostile_filename() {
        let (_dir, lib) = library();
        let (path, _) = lib.add("../../evil.torrent", &torrent_file("e1", 1, "http://t/a")).unwrap();
        assert_eq!(path, lib.torrents_dir().join("evil.torrent"));
        assert_eq!(sanitize_filename(r"..\..\win.torrent"), "win.torrent");
        assert_eq!(sanitize_filename(""), "torrent.torrent");
        assert_eq!(sanitize_filename("..."), "torrent.torrent");
        assert_eq!(sanitize_filename("a:b*?.TORRENT"), "a_b__.torrent");
        assert_eq!(sanitize_filename("Ubuntu 24.04 (x64) [é].torrent"), "Ubuntu 24.04 (x64) [é].torrent");
    }

    #[test]
    fn add_avoids_name_collisions() {
        let (_dir, lib) = library();
        lib.add("x.torrent", &torrent_file("one", 1, "http://t/a")).unwrap();
        let (second, _) = lib.add("x.torrent", &torrent_file("two", 1, "http://t/a")).unwrap();
        assert_eq!(second, lib.torrents_dir().join("x-1.torrent"));
    }

    #[test]
    fn scan_reports_valid_and_invalid_torrents_only() {
        let (_dir, lib) = library();
        let dir = lib.torrents_dir();
        fs::write(dir.join("b.torrent"), torrent_file("b", 1, "http://t/a")).unwrap();
        fs::write(dir.join("a.TORRENT"), torrent_file("a", 1, "http://t/a")).unwrap();
        fs::write(dir.join("junk.torrent"), b"nope").unwrap();
        fs::write(dir.join("notes.txt"), b"hello").unwrap();
        fs::write(dir.join(".partial.torrent"), torrent_file("p", 1, "http://t/a")).unwrap();
        fs::write(lib.archive_dir().join("old.torrent"), torrent_file("o", 1, "http://t/a")).unwrap();
        let scan = lib.scan();
        let names: Vec<&str> = scan.torrents.iter().map(|(_, t)| t.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(scan.invalid.len(), 1);
        assert!(scan.invalid[0].0.ends_with("junk.torrent"));
    }

    #[test]
    fn archive_moves_file_and_avoids_collisions() {
        let (_dir, lib) = library();
        let (first, _) = lib.add("x.torrent", &torrent_file("one", 1, "http://t/a")).unwrap();
        assert_eq!(lib.archive(&first).unwrap(), lib.archive_dir().join("x.torrent"));
        let (again, _) = lib.add("x.torrent", &torrent_file("two", 1, "http://t/a")).unwrap();
        assert_eq!(lib.archive(&again).unwrap(), lib.archive_dir().join("x-1.torrent"));
        assert!(files_in(lib.torrents_dir()).is_empty());
    }

    #[test]
    fn delete_is_idempotent() {
        let (_dir, lib) = library();
        let (path, _) = lib.add("x.torrent", &torrent_file("x", 1, "http://t/a")).unwrap();
        lib.delete(&path).unwrap();
        lib.delete(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn watch_reports_changes() {
        let (_dir, lib) = library();
        let (tx, rx) = std::sync::mpsc::channel();
        let _watcher = lib
            .watch(move || {
                let _ = tx.send(());
            })
            .unwrap();
        fs::write(lib.torrents_dir().join("new.torrent"), torrent_file("n", 1, "http://t/a")).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).expect("a change notification");
    }
}
