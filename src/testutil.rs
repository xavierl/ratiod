//! Fixture builders shared by unit and integration tests.

/// Encodes a bencode byte string.
pub fn bstr(s: &[u8]) -> Vec<u8> {
    let mut out = format!("{}:", s.len()).into_bytes();
    out.extend_from_slice(s);
    out
}

/// A single-file `info` dictionary.
pub fn info_dict(name: &str, size: u64) -> Vec<u8> {
    let mut out = b"d".to_vec();
    out.extend(bstr(b"length"));
    out.extend(format!("i{size}e").into_bytes());
    out.extend(bstr(b"name"));
    out.extend(bstr(name.as_bytes()));
    out.extend(bstr(b"piece length"));
    out.extend(b"i16384e");
    out.extend(bstr(b"pieces"));
    out.extend(bstr(&[0u8; 20]));
    out.push(b'e');
    out
}

/// A single-file torrent announcing to one tracker.
pub fn torrent_file(name: &str, size: u64, announce: &str) -> Vec<u8> {
    torrent_with_trackers(name, size, &[announce])
}

/// A single-file torrent with one tracker per `announce-list` tier.
pub fn torrent_with_trackers(name: &str, size: u64, trackers: &[&str]) -> Vec<u8> {
    let mut out = b"d".to_vec();
    out.extend(bstr(b"announce"));
    out.extend(bstr(trackers.first().copied().unwrap_or("").as_bytes()));
    out.extend(bstr(b"announce-list"));
    out.push(b'l');
    for tracker in trackers {
        out.push(b'l');
        out.extend(bstr(tracker.as_bytes()));
        out.push(b'e');
    }
    out.push(b'e');
    out.extend(bstr(b"info"));
    out.extend(info_dict(name, size));
    out.push(b'e');
    out
}

/// A successful tracker announce response.
pub fn tracker_response(interval: u32, complete: u32, incomplete: u32) -> Vec<u8> {
    format!("d8:completei{complete}e10:incompletei{incomplete}e8:intervali{interval}e5:peers0:e").into_bytes()
}
