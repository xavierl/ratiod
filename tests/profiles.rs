use std::time::Instant;

use ratiod::emulation::{self, AnnounceParams, Event};

#[test]
fn all_95_profiles_load_and_build_complete_queries() {
    let none = tempfile::tempdir().unwrap();
    let names = emulation::list_profiles(none.path());
    assert_eq!(names.len(), 95);
    for name in &names {
        let mut profile = emulation::load_profile(name, none.path()).unwrap_or_else(|e| panic!("{name}: {e}"));
        for event in [Event::Started, Event::None, Event::Stopped] {
            let p = AnnounceParams { info_hash: [7; 20], uploaded: 1, downloaded: 0, left: 0, port: 49200, event };
            let q = profile.build_query(&p, Instant::now());
            assert!(!q.contains('{') && !q.contains('}'), "{name}: placeholder left in {q}");
            assert!(q.starts_with("info_hash=") || q.contains("&info_hash="), "{name}: {q}");
            assert!(!q.starts_with('&') && !q.ends_with('&'), "{name}: {q}");
        }
        assert!(!profile.headers().is_empty(), "{name} has no headers");
    }
}

#[test]
fn qbittorrent_query_matches_expected_shape() {
    let none = tempfile::tempdir().unwrap();
    let mut profile = emulation::load_profile("qbittorrent-5.2.4.client", none.path()).unwrap();
    let p = AnnounceParams {
        info_hash: [0x12; 20],
        uploaded: 42,
        downloaded: 0,
        left: 0,
        port: 49200,
        event: Event::Started,
    };
    let q = profile.build_query(&p, Instant::now());
    let shape = regex::Regex::new(
        r"^info_hash=(%12){20}&peer_id=-qB5240-[A-Za-z0-9_~\(\)\!\.\*-]{12}&port=49200&uploaded=42&downloaded=0&left=0&corrupt=0&key=[0-9A-F]{1,8}&event=started&numwant=200&compact=1&no_peer_id=1&supportcrypto=1&redundant=0$",
    )
    .unwrap();
    assert!(shape.is_match(&q), "{q}");
    let headers: Vec<&str> = profile.headers().iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(headers, ["User-Agent", "Accept-Encoding", "Connection"]);
}

#[test]
fn default_client_is_latest_qbittorrent() {
    let none = tempfile::tempdir().unwrap();
    assert_eq!(emulation::default_client(none.path()), "qbittorrent-5.2.4.client");
}

#[test]
fn user_dir_adds_and_overrides_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let mut custom = std::fs::read_to_string("clients/qbittorrent-5.2.4.client").unwrap();
    custom = custom.replace("qBittorrent/5.2.4", "qBittorrent/9.9.9");
    std::fs::write(dir.path().join("qbittorrent-5.2.4.client"), &custom).unwrap();
    std::fs::write(dir.path().join("qbittorrent-9.9.9.client"), &custom).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();

    let names = emulation::list_profiles(dir.path());
    assert_eq!(names.len(), 96);
    assert_eq!(emulation::default_client(dir.path()), "qbittorrent-9.9.9.client");
    let profile = emulation::load_profile("qbittorrent-5.2.4.client", dir.path()).unwrap();
    assert_eq!(profile.headers()[0].1, "qBittorrent/9.9.9");
}

#[test]
fn unknown_or_path_like_names_are_not_found() {
    let none = tempfile::tempdir().unwrap();
    for name in ["nope.client", "../Cargo.toml", "a/b.client"] {
        assert!(matches!(emulation::load_profile(name, none.path()), Err(emulation::ProfileError::NotFound(_))));
    }
}
