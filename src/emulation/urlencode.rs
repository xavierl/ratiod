//! Configurable URL encoder: bytes matching the exclusion pattern are kept,
//! every other byte becomes `%xx` in the configured hex case.

use regex::Regex;

#[derive(Debug, Clone)]
pub struct UrlEncoder {
    keep: [bool; 256],
    upper_hex: bool,
}

impl UrlEncoder {
    /// `exclusion_pattern` is matched against each byte seen as a Latin-1 char.
    pub fn new(exclusion_pattern: &str, upper_hex: bool) -> Result<Self, regex::Error> {
        let re = Regex::new(&format!("^(?:{exclusion_pattern})$"))?;
        let mut keep = [false; 256];
        let mut buf = [0u8; 4];
        for byte in 0..=255u8 {
            keep[usize::from(byte)] = re.is_match(char::from(byte).encode_utf8(&mut buf));
        }
        Ok(Self { keep, upper_hex })
    }

    pub fn encode(&self, bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 3);
        for &byte in bytes {
            if self.keep[usize::from(byte)] {
                out.push(char::from(byte));
            } else if self.upper_hex {
                out.push_str(&format!("%{byte:02X}"));
            } else {
                out.push_str(&format!("%{byte:02x}"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_non_excluded_bytes() {
        let e = UrlEncoder::new("[a-zA-Z0-9]", false).unwrap();
        assert_eq!(e.encode(&[0x00, 0x01, 0x10, 0x1e, 0xff]), "%00%01%10%1e%ff");
        assert_eq!(e.encode(b"2nz"), "2nz");
    }

    #[test]
    fn hex_case_applies_only_to_encoded_bytes() {
        let e = UrlEncoder::new("[a-zA-Z0-9]", true).unwrap();
        assert_eq!(e.encode(b"yY"), "yY");
        assert_eq!(e.encode(&[0xae]), "%AE");
    }

    #[test]
    fn encodes_mixed_input() {
        let lower = UrlEncoder::new("[a-zA-Z0-9]", false).unwrap();
        assert_eq!(lower.encode(&[b'a', 0x11, b'q', 0xf3]), "a%11q%f3");
        let upper_nothing_kept = UrlEncoder::new("", true).unwrap();
        assert_eq!(upper_nothing_kept.encode(&[0xa2, 0x11, 0xf3]), "%A2%11%F3");
    }

    #[test]
    fn accepts_escaped_punctuation() {
        let e = UrlEncoder::new(r"[A-Za-z0-9_~\(\)\!\.\*-]", false).unwrap();
        assert_eq!(e.encode(b"-qB(x)!*~._"), "-qB(x)!*~._");
        assert_eq!(e.encode(b" /"), "%20%2f");
    }

    #[test]
    fn latin1_bytes_are_never_kept_by_ascii_classes() {
        let e = UrlEncoder::new("[A-Za-z]", false).unwrap();
        assert_eq!(e.encode(&[0xd2]), "%d2");
    }
}
