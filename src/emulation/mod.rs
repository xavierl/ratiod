//! Emulation of real BitTorrent clients from Joal `.client` profiles.

pub mod algorithm;
pub mod generator;
pub mod urlencode;

pub use generator::Event;

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
        assert_eq!(profile().headers(), &[("User-Agent".to_owned(), "Test/1.0 (Linux; en-US; Java 21.0.4)".to_owned())]);
    }

    #[test]
    fn url_encodes_peer_id_when_asked() {
        let json = TEST_PROFILE
            .replace("-AB1234-[x]{12}", "-UT354S-(\u{d2})[x]{11}")
            .replace(r#""shouldUrlEncode": false"#, r#""shouldUrlEncode": true"#);
        let q = ClientProfile::from_json("t", json.as_bytes()).unwrap().build_query(&params(Event::None), Instant::now());
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
            ["deluge-2.1.1.client", "qbittorrent-5.10.0.client", "qbittorrent-5.2.4.client", "qbittorrent-4.6.7.client"]
        );
    }
}
