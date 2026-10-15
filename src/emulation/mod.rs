//! Emulation of real BitTorrent clients from `.client` profiles.

pub mod algorithm;
pub mod generator;
pub mod urlencode;

pub use generator::Event;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Instant;

use regex::Regex;
use rust_embed::RustEmbed;
use serde::Deserialize;
use thiserror::Error;

use crate::torrent::InfoHash;
use algorithm::PEER_ID_LEN;
use generator::{Casing, Generator, GeneratorConfig};
use urlencode::UrlEncoder;

#[derive(RustEmbed)]
#[folder = "clients/"]
struct EmbeddedClients;

/// Values substituted into `{java}`, `{os}` and `{locale}` header placeholders.
const HEADER_JAVA: &str = "21.0.4";
const HEADER_OS: &str = "Linux";
const HEADER_LOCALE: &str = "en-US";

const KNOWN_PLACEHOLDERS: [&str; 8] =
    ["{infohash}", "{uploaded}", "{downloaded}", "{left}", "{port}", "{numwant}", "{peerid}", "{key}"];

static AMPERSAND_DUPES: LazyLock<Regex> = LazyLock::new(|| Regex::new("&{2,}").expect("static regex"));
static IP_PARAM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&*\w+=\{ip(?:v6)?\}").expect("static regex"));
static EVENT_PARAM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&*\w+=\{event\}").expect("static regex"));
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{.*?\}").expect("static regex"));

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("profil client « {0} » introuvable")]
    NotFound(String),
    #[error("profil client « {name} » invalide : {reason}")]
    Invalid { name: String, reason: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileFile {
    key_generator: Option<GeneratorConfig>,
    peer_id_generator: GeneratorConfig,
    url_encoder: UrlEncoderFile,
    query: String,
    numwant: u32,
    numwant_on_stop: u32,
    #[serde(default)]
    request_headers: Vec<HeaderFile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UrlEncoderFile {
    encoding_exclusion_pattern: String,
    encoded_hex_case: Casing,
}

#[derive(Deserialize)]
struct HeaderFile {
    name: String,
    value: String,
}

#[derive(Debug, Clone, Copy)]
pub struct AnnounceParams {
    pub info_hash: InfoHash,
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
    pub port: u16,
    pub event: Event,
}

pub struct ClientProfile {
    name: String,
    key: Option<Generator>,
    peer_id: Generator,
    url_encode_peer_id: bool,
    encoder: UrlEncoder,
    query: String,
    numwant: u32,
    numwant_on_stop: u32,
    headers: Vec<(String, String)>,
}

impl ClientProfile {
    pub fn from_json(name: &str, json: &[u8]) -> Result<Self, ProfileError> {
        let invalid = |reason: String| ProfileError::Invalid { name: name.to_owned(), reason };
        let file: ProfileFile = serde_json::from_slice(json).map_err(|e| invalid(e.to_string()))?;
        let peer_id = Generator::new(&file.peer_id_generator).map_err(|e| invalid(e.to_string()))?;
        let key = file.key_generator.as_ref().map(Generator::new).transpose().map_err(|e| invalid(e.to_string()))?;
        let encoder = UrlEncoder::new(
            &file.url_encoder.encoding_exclusion_pattern,
            file.url_encoder.encoded_hex_case == Casing::Upper,
        )
        .map_err(|e| invalid(e.to_string()))?;
        let headers = file
            .request_headers
            .into_iter()
            .map(|h| Ok((h.name, render_header(&h.value)?)))
            .collect::<Result<Vec<_>, String>>()
            .map_err(invalid)?;
        let sample = peer_id.sample();
        if sample.len() != PEER_ID_LEN {
            return Err(invalid(format!("le peer_id doit faire {PEER_ID_LEN} octets, pas {}", sample.len())));
        }
        let profile = Self {
            name: name.to_owned(),
            key,
            peer_id,
            url_encode_peer_id: file.peer_id_generator.should_url_encode,
            encoder,
            query: AMPERSAND_DUPES.replace_all(&file.query, "&").into_owned(),
            numwant: file.numwant,
            numwant_on_stop: file.numwant_on_stop,
            headers,
        };
        profile.check_query().map_err(invalid)?;
        Ok(profile)
    }

    fn check_query(&self) -> Result<(), String> {
        if self.query.contains("{key}") && self.key.is_none() {
            return Err("la query utilise {key} mais aucun keyGenerator n'est défini".into());
        }
        let mut q = IP_PARAM.replace_all(&self.query, "").into_owned();
        q = EVENT_PARAM.replace_all(&q, "").into_owned();
        for placeholder in KNOWN_PLACEHOLDERS {
            q = q.replace(placeholder, "");
        }
        match PLACEHOLDER.find(&q) {
            Some(m) => Err(format!("placeholder inconnu {} dans la query", m.as_str())),
            None => Ok(()),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// Fills the query template: placeholders, ip/event parameter removal, `&` cleanup.
    pub fn build_query(&mut self, p: &AnnounceParams, now: Instant) -> String {
        let numwant = if p.event == Event::Stopped { self.numwant_on_stop } else { self.numwant };
        let mut q = self
            .query
            .replace("{infohash}", &self.encoder.encode(&p.info_hash))
            .replace("{uploaded}", &p.uploaded.to_string())
            .replace("{downloaded}", &p.downloaded.to_string())
            .replace("{left}", &p.left.to_string())
            .replace("{port}", &p.port.to_string())
            .replace("{numwant}", &numwant.to_string());
        let peer_id = self.peer_id.get(&p.info_hash, p.event, now);
        let peer_id = if self.url_encode_peer_id {
            self.encoder.encode(&peer_id)
        } else {
            peer_id.iter().map(|&b| char::from(b)).collect()
        };
        q = q.replace("{peerid}", &peer_id);
        q = IP_PARAM.replace_all(&q, "").into_owned();
        q = if p.event == Event::None {
            EVENT_PARAM.replace_all(&q, "").into_owned()
        } else {
            q.replace("{event}", p.event.as_str())
        };
        if q.contains("{key}")
            && let Some(key) = self.key.as_mut()
        {
            let key = key.get(&p.info_hash, p.event, now);
            q = q.replace("{key}", &self.encoder.encode(&key));
        }
        let q = q.strip_prefix('&').unwrap_or(&q);
        q.strip_suffix('&').unwrap_or(q).to_owned()
    }
}

fn render_header(value: &str) -> Result<String, String> {
    let value = value.replace("{java}", HEADER_JAVA).replace("{os}", HEADER_OS).replace("{locale}", HEADER_LOCALE);
    match PLACEHOLDER.find(&value) {
        Some(m) => Err(format!("placeholder inconnu {} dans un header", m.as_str())),
        None => Ok(value),
    }
}

/// Loads a profile, preferring `user_dir/<name>` over the embedded copy.
pub fn load_profile(name: &str, user_dir: &Path) -> Result<ClientProfile, ProfileError> {
    if name.contains(['/', '\\']) || name.contains("..") {
        return Err(ProfileError::NotFound(name.to_owned()));
    }
    if let Ok(bytes) = std::fs::read(user_dir.join(name)) {
        return ClientProfile::from_json(name, &bytes);
    }
    let file = EmbeddedClients::get(name).ok_or_else(|| ProfileError::NotFound(name.to_owned()))?;
    ClientProfile::from_json(name, &file.data)
}

/// Embedded and user profile names: client name A→Z, then newest version first.
pub fn list_profiles(user_dir: &Path) -> Vec<String> {
    let mut names: BTreeSet<String> = EmbeddedClients::iter().map(|n| n.into_owned()).collect();
    if let Ok(entries) = std::fs::read_dir(user_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".client") {
                names.insert(name);
            }
        }
    }
    let mut names: Vec<String> = names.into_iter().collect();
    sort_profiles(&mut names);
    names
}

/// The newest qBittorrent profile, or the first profile if there is none.
pub fn default_client(user_dir: &Path) -> String {
    let names = list_profiles(user_dir);
    names.iter().find(|n| n.starts_with("qbittorrent-")).or(names.first()).cloned().unwrap_or_default()
}

fn sort_profiles(names: &mut [String]) {
    names.sort_by(|a, b| {
        let (client_a, version_a) = sort_key(a);
        let (client_b, version_b) = sort_key(b);
        client_a.cmp(&client_b).then_with(|| version_b.cmp(&version_a)).then_with(|| a.cmp(b))
    });
}

fn sort_key(name: &str) -> (String, Vec<u64>) {
    let stem = name.strip_suffix(".client").unwrap_or(name);
    let (client, version) = stem.split_once('-').unwrap_or((stem, ""));
    let numbers = version.split(|c: char| !c.is_ascii_digit()).filter_map(|s| s.parse().ok()).collect();
    (client.to_ascii_lowercase(), numbers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Deterministic profile: fixed peer-id and key, every placeholder exercised.
    const TEST_PROFILE: &str = r#"{
        "keyGenerator": { "algorithm": { "type": "REGEX", "pattern": "KEY[a]{5}" }, "refreshOn": "NEVER", "keyCase": "none" },
        "peerIdGenerator": { "algorithm": { "type": "REGEX", "pattern": "-AB1234-[x]{12}" }, "refreshOn": "NEVER", "shouldUrlEncode": false },
        "urlEncoder": { "encodingExclusionPattern": "[A-Za-z0-9-]", "encodedHexCase": "lower" },
        "query": "info_hash={infohash}&&peer_id={peerid}&port={port}&uploaded={uploaded}&downloaded={downloaded}&left={left}&ipv6={ipv6}&key={key}&event={event}&numwant={numwant}&compact=1",
        "numwant": 200,
        "numwantOnStop": 0,
        "requestHeaders": [ { "name": "User-Agent", "value": "Test/1.0 ({os}; {locale}; Java {java})" } ]
    }"#;

    fn params(event: Event) -> AnnounceParams {
        AnnounceParams { info_hash: [0xab; 20], uploaded: 1234, downloaded: 0, left: 0, port: 49200, event }
    }

    fn profile() -> ClientProfile {
        ClientProfile::from_json("test.client", TEST_PROFILE.as_bytes()).unwrap()
    }

    #[test]
    fn builds_started_query_exactly() {
        let q = profile().build_query(&params(Event::Started), Instant::now());
        let hash = "%ab".repeat(20);
        assert_eq!(
            q,
            format!(
                "info_hash={hash}&peer_id=-AB1234-xxxxxxxxxxxx&port=49200&uploaded=1234&downloaded=0&left=0&key=KEYaaaaa&event=started&numwant=200&compact=1"
            )
        );
    }

    #[test]
    fn regular_announce_drops_event_param() {
        let q = profile().build_query(&params(Event::None), Instant::now());
        assert!(!q.contains("event="));
        assert!(q.contains("&key=KEYaaaaa&numwant=200&"));
    }

    #[test]
    fn stopped_uses_numwant_on_stop() {
        let q = profile().build_query(&params(Event::Stopped), Instant::now());
        assert!(q.contains("event=stopped&numwant=0&"));
    }

    #[test]
    fn renders_header_placeholders() {
        assert_eq!(
            profile().headers(),
            &[("User-Agent".to_owned(), "Test/1.0 (Linux; en-US; Java 21.0.4)".to_owned())]
        );
    }

    #[test]
    fn url_encodes_peer_id_when_asked() {
        let json = TEST_PROFILE
            .replace("-AB1234-[x]{12}", "-UT354S-(\u{d2})[x]{11}")
            .replace(r#""shouldUrlEncode": false"#, r#""shouldUrlEncode": true"#);
        let q =
            ClientProfile::from_json("t", json.as_bytes()).unwrap().build_query(&params(Event::None), Instant::now());
        assert!(q.contains("peer_id=-UT354S-%d2xxxxxxxxxxx&"));
    }

    #[test]
    fn rejects_unknown_query_placeholder() {
        let json = TEST_PROFILE.replace("&compact=1", "&compact={foo}");
        assert!(matches!(ClientProfile::from_json("t", json.as_bytes()), Err(ProfileError::Invalid { .. })));
    }

    #[test]
    fn rejects_key_placeholder_without_key_generator() {
        let start = TEST_PROFILE.find(r#""keyGenerator""#).unwrap();
        let end = TEST_PROFILE.find(r#""peerIdGenerator""#).unwrap();
        let json = format!("{}{}", &TEST_PROFILE[..start], &TEST_PROFILE[end..]);
        assert!(matches!(ClientProfile::from_json("t", json.as_bytes()), Err(ProfileError::Invalid { .. })));
    }

    #[test]
    fn rejects_unknown_header_placeholder_and_bad_peer_id_length() {
        let header = TEST_PROFILE.replace("{os}", "{arch}");
        assert!(ClientProfile::from_json("t", header.as_bytes()).is_err());
        let short = TEST_PROFILE.replace("[x]{12}", "[x]{5}");
        assert!(ClientProfile::from_json("t", short.as_bytes()).is_err());
    }

    #[test]
    fn sort_key_orders_versions_numerically() {
        let mut names = vec![
            "qbittorrent-4.6.7.client".to_owned(),
            "qbittorrent-5.2.4.client".to_owned(),
            "deluge-2.1.1.client".to_owned(),
            "qbittorrent-5.10.0.client".to_owned(),
        ];
        sort_profiles(&mut names);
        assert_eq!(
            names,
            [
                "deluge-2.1.1.client",
                "qbittorrent-5.10.0.client",
                "qbittorrent-5.2.4.client",
                "qbittorrent-4.6.7.client"
            ]
        );
    }
}
