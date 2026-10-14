//! `.torrent` metadata needed to announce: info-hash, name, size, HTTP trackers.

use std::collections::HashSet;

use sha1::{Digest, Sha1};
use thiserror::Error;

use crate::bencode::{self, BencodeError, Value};

pub type InfoHash = [u8; 20];

#[derive(Debug, Clone, PartialEq)]
pub struct MockedTorrent {
    pub info_hash: InfoHash,
    pub name: String,
    pub size: u64,
    /// HTTP(S) announce URLs, in announce-list order.
    pub trackers: Vec<String>,
}

#[derive(Debug, Error)]
pub enum TorrentError {
    #[error("fichier .torrent illisible : {0}")]
    Bencode(#[from] BencodeError),
    #[error("champ `{0}` absent ou invalide")]
    Field(&'static str),
    #[error("aucun tracker HTTP(S) dans ce torrent")]
    NoHttpTracker,
}

impl MockedTorrent {
    pub fn parse(data: &[u8]) -> Result<Self, TorrentError> {
        let root = bencode::decode(data)?;
        let info = root.get("info").ok_or(TorrentError::Field("info"))?;
        let raw_info = info.raw().ok_or(TorrentError::Field("info"))?;
        let mut info_hash = [0u8; 20];
        info_hash.copy_from_slice(&Sha1::digest(raw_info));
        let name = info
            .get("name")
            .and_then(Value::as_bytes)
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .ok_or(TorrentError::Field("name"))?;
        let size = torrent_size(info)?;
        let trackers = http_trackers(&root);
        if trackers.is_empty() {
            return Err(TorrentError::NoHttpTracker);
        }
        Ok(Self { info_hash, name, size, trackers })
    }

    pub fn info_hash_hex(&self) -> String {
        hex::encode(self.info_hash)
    }
}

fn torrent_size(info: &Value<'_>) -> Result<u64, TorrentError> {
    if let Some(length) = info.get("length").and_then(Value::as_int) {
        return u64::try_from(length).map_err(|_| TorrentError::Field("length"));
    }
    let files = info.get("files").and_then(Value::as_list).ok_or(TorrentError::Field("length"))?;
    files.iter().try_fold(0u64, |total, file| {
        let length = file
            .get("length")
            .and_then(Value::as_int)
            .and_then(|l| u64::try_from(l).ok())
            .ok_or(TorrentError::Field("files.length"))?;
        total.checked_add(length).ok_or(TorrentError::Field("files.length"))
    })
}

fn http_trackers(root: &Value<'_>) -> Vec<String> {
    let mut urls = Vec::new();
    if let Some(tiers) = root.get("announce-list").and_then(Value::as_list) {
        for tier in tiers {
            for url in tier.as_list().unwrap_or(&[]) {
                if let Some(url) = url.as_str() {
                    urls.push(url.trim().to_owned());
                }
            }
        }
    }
    if urls.is_empty() {
        if let Some(url) = root.get("announce").and_then(Value::as_str) {
            urls.push(url.trim().to_owned());
        }
    }
    let mut seen = HashSet::new();
    urls.into_iter()
        .filter(|url| {
            let lower = url.to_ascii_lowercase();
            lower.starts_with("http://") || lower.starts_with("https://")
        })
        .filter(|url| seen.insert(url.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{bstr, info_dict, torrent_file, torrent_with_trackers};
    use sha1::{Digest, Sha1};

    #[test]
    fn parses_real_ubuntu_torrent() {
        let data = std::fs::read("tests/fixtures/ubuntu-17.04-desktop-amd64.iso.torrent").unwrap();
        let t = MockedTorrent::parse(&data).unwrap();
        // Reference value computed independently (Node.js sha1 of the raw info dict).
        assert_eq!(t.info_hash_hex(), "59066769b9ad42da2e508611c33d7c4480b3857b");
        assert_eq!(t.name, "ubuntu-17.04-desktop-amd64.iso");
        assert_eq!(t.size, 1_609_039_872);
        assert_eq!(
            t.trackers,
            vec!["http://torrent.ubuntu.com:6969/announce", "http://ipv6.torrent.ubuntu.com:6969/announce"]
        );
    }

    #[test]
    fn info_hash_is_sha1_of_raw_info_dict() {
        let t = MockedTorrent::parse(&torrent_file("a", 5, "http://t/announce")).unwrap();
        let expected: [u8; 20] = Sha1::digest(info_dict("a", 5)).into();
        assert_eq!(t.info_hash, expected);
    }

    #[test]
    fn non_canonical_info_dict_is_hashed_as_is() {
        // Keys out of order: re-encoding would change the hash.
        let info = b"d4:name1:x6:lengthi5ee";
        let mut data = b"d8:announce".to_vec();
        data.extend(bstr(b"http://t/announce"));
        data.extend(b"4:info");
        data.extend(info);
        data.push(b'e');
        let t = MockedTorrent::parse(&data).unwrap();
        let expected: [u8; 20] = Sha1::digest(info).into();
        assert_eq!(t.info_hash, expected);
        assert_eq!(t.size, 5);
    }

    #[test]
    fn sums_multi_file_sizes() {
        let mut data = b"d8:announce".to_vec();
        data.extend(bstr(b"http://t/announce"));
        data.extend(b"4:infod5:filesld6:lengthi3e4:pathl1:aeed6:lengthi4e4:pathl1:beee4:name3:dire");
        data.push(b'e');
        assert_eq!(MockedTorrent::parse(&data).unwrap().size, 7);
    }

    #[test]
    fn keeps_only_http_trackers_in_order_without_duplicates() {
        let data = torrent_with_trackers(
            "a",
            1,
            &["udp://u/announce", "https://s/announce", "http://h/announce", "https://s/announce"],
        );
        let t = MockedTorrent::parse(&data).unwrap();
        assert_eq!(t.trackers, vec!["https://s/announce", "http://h/announce"]);
    }

    #[test]
    fn rejects_udp_only_torrent() {
        let data = torrent_file("a", 1, "udp://tracker/announce");
        assert!(matches!(MockedTorrent::parse(&data), Err(TorrentError::NoHttpTracker)));
    }

    #[test]
    fn rejects_garbage_and_negative_sizes() {
        assert!(matches!(MockedTorrent::parse(b"not a torrent"), Err(TorrentError::Bencode(_))));
        let mut data = b"d8:announce".to_vec();
        data.extend(bstr(b"http://t/announce"));
        data.extend(b"4:infod6:lengthi-1e4:name1:xee");
        assert!(matches!(MockedTorrent::parse(&data), Err(TorrentError::Field("length"))));
    }
}
