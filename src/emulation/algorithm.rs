//! Peer-id and key generation algorithms from Joal `.client` profiles.
//! Values are Latin-1 byte strings (Joal used Java chars ≤ 0xFF).

use rand::RngExt;
use serde::Deserialize;
use thiserror::Error;

pub const PEER_ID_LEN: usize = 20;
const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AlgorithmConfig {
    Regex {
        pattern: String,
    },
    #[serde(rename_all = "camelCase")]
    RandomPoolWithChecksum {
        prefix: String,
        characters_pool: String,
        base: usize,
    },
    Hash {
        length: usize,
    },
    HashNoLeadingZero {
        length: usize,
    },
    #[serde(rename = "DIGIT_RANGE_TRANSFORMED_TO_HEX_WITHOUT_LEADING_ZEROES", rename_all = "camelCase")]
    DigitRange {
        inclusive_lower_bound: u64,
        inclusive_upper_bound: u64,
    },
}

#[derive(Debug, Error)]
#[error("{0}")]
pub struct AlgorithmError(pub String);

pub enum Algorithm {
    Regex(rand_regex::Regex),
    Pool { prefix: Vec<u8>, pool: Vec<u8>, base: usize },
    Hex { length: usize, strip_leading_zeros: bool },
    DigitRange { lower: u64, upper: u64 },
}

impl Algorithm {
    pub fn compile(cfg: &AlgorithmConfig) -> Result<Self, AlgorithmError> {
        Ok(match cfg {
            AlgorithmConfig::Regex { pattern } => Algorithm::Regex(
                rand_regex::Regex::compile(pattern, 16)
                    .map_err(|e| AlgorithmError(format!("regex invalide `{pattern}` : {e}")))?,
            ),
            AlgorithmConfig::RandomPoolWithChecksum { prefix, characters_pool, base } => {
                let prefix = latin1(prefix)?;
                let pool = latin1(characters_pool)?;
                if prefix.is_empty() || prefix.len() >= PEER_ID_LEN {
                    return Err(AlgorithmError(format!("prefix doit faire 1 à {} caractères", PEER_ID_LEN - 1)));
                }
                if *base == 0 || *base > pool.len() {
                    return Err(AlgorithmError("base doit être entre 1 et la taille de charactersPool".into()));
                }
                Algorithm::Pool { prefix, pool, base: *base }
            }
            AlgorithmConfig::Hash { length } | AlgorithmConfig::HashNoLeadingZero { length } => {
                if *length == 0 {
                    return Err(AlgorithmError("length doit être supérieur à 0".into()));
                }
                Algorithm::Hex {
                    length: *length,
                    strip_leading_zeros: matches!(cfg, AlgorithmConfig::HashNoLeadingZero { .. }),
                }
            }
            AlgorithmConfig::DigitRange { inclusive_lower_bound: lower, inclusive_upper_bound: upper } => {
                if lower > upper {
                    return Err(AlgorithmError("inclusiveLowerBound doit être ≤ inclusiveUpperBound".into()));
                }
                Algorithm::DigitRange { lower: *lower, upper: *upper }
            }
        })
    }

    pub fn generate(&self) -> Vec<u8> {
        let mut rng = rand::rng();
        match self {
            Algorithm::Regex(re) => {
                let s: String = rng.sample(re);
                // Profiles only use chars ≤ U+00FF; anything else cannot be sent as one byte.
                s.chars().map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect()
            }
            Algorithm::Pool { prefix, pool, base } => {
                let suffix_len = PEER_ID_LEN - prefix.len();
                let mut out = prefix.clone();
                let mut total = 0usize;
                for _ in 0..suffix_len - 1 {
                    let index = usize::from(rng.random::<u8>()) % base;
                    total += index;
                    out.push(pool[index]);
                }
                let check = if total % base == 0 { 0 } else { base - total % base };
                out.push(pool[check]);
                out
            }
            Algorithm::Hex { length, strip_leading_zeros } => {
                let hex: Vec<u8> = (0..*length).map(|_| HEX_UPPER[rng.random_range(0..16)]).collect();
                if *strip_leading_zeros {
                    hex.into_iter().skip_while(|&c| c == b'0').collect()
                } else {
                    hex
                }
            }
            Algorithm::DigitRange { lower, upper } => format!("{:x}", rng.random_range(*lower..=*upper)).into_bytes(),
        }
    }
}

fn latin1(s: &str) -> Result<Vec<u8>, AlgorithmError> {
    s.chars()
        .map(|c| u8::try_from(u32::from(c)).map_err(|_| AlgorithmError(format!("caractère non Latin-1 : {c}"))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_json(json: &str) -> Algorithm {
        let cfg: AlgorithmConfig = serde_json::from_str(json).unwrap();
        Algorithm::compile(&cfg).unwrap()
    }

    #[test]
    fn regex_generates_matching_ascii() {
        let a = compile_json(r#"{"type":"REGEX","pattern":"-qB5240-[A-Za-z0-9_~\\(\\)\\!\\.\\*-]{12}"}"#);
        let check = regex::Regex::new(r"^-qB5240-[A-Za-z0-9_~\(\)\!\.\*-]{12}$").unwrap();
        for _ in 0..50 {
            let id = a.generate();
            assert_eq!(id.len(), PEER_ID_LEN);
            assert!(check.is_match(std::str::from_utf8(&id).unwrap()));
        }
    }

    #[test]
    fn regex_maps_latin1_chars_to_single_bytes() {
        // Same shape as utorrent-3.5.4_44498.client after JSON unescaping.
        let a = Algorithm::compile(&AlgorithmConfig::Regex {
            pattern: "-UT354S-(\u{d2})(\u{ad})[\u{1}-\u{ff}]{10}".into(),
        })
        .unwrap();
        let id = a.generate();
        assert_eq!(id.len(), 20);
        assert_eq!(&id[..8], b"-UT354S-");
        assert_eq!((id[8], id[9]), (0xd2, 0xad));
    }

    #[test]
    fn random_pool_has_prefix_and_valid_checksum() {
        let a = compile_json(
            r#"{"type":"RANDOM_POOL_WITH_CHECKSUM","prefix":"-TR2820-","charactersPool":"0123456789abcdefghijklmnopqrstuvwxyz","base":36}"#,
        );
        let pool = b"0123456789abcdefghijklmnopqrstuvwxyz";
        for _ in 0..50 {
            let id = a.generate();
            assert_eq!(id.len(), 20);
            assert_eq!(&id[..8], b"-TR2820-");
            let sum: usize = id[8..].iter().map(|c| pool.iter().position(|p| p == c).unwrap()).sum();
            assert_eq!(sum % 36, 0);
        }
    }

    #[test]
    fn hash_is_uppercase_hex_of_given_length() {
        let a = compile_json(r#"{"type":"HASH","length":8}"#);
        let key = a.generate();
        assert_eq!(key.len(), 8);
        assert!(key.iter().all(|c| b"0123456789ABCDEF".contains(c)));
    }

    #[test]
    fn hash_no_leading_zero_strips_zeros() {
        let a = compile_json(r#"{"type":"HASH_NO_LEADING_ZERO","length":8}"#);
        for _ in 0..200 {
            let key = a.generate();
            assert!(key.first() != Some(&b'0'));
            assert!(key.len() <= 8);
        }
    }

    #[test]
    fn digit_range_is_lowercase_hex() {
        let a = compile_json(
            r#"{"type":"DIGIT_RANGE_TRANSFORMED_TO_HEX_WITHOUT_LEADING_ZEROES","inclusiveLowerBound":255,"inclusiveUpperBound":255}"#,
        );
        assert_eq!(a.generate(), b"ff");
    }

    #[test]
    fn rejects_invalid_configs() {
        let bad = [
            AlgorithmConfig::Regex { pattern: "(".into() },
            AlgorithmConfig::RandomPoolWithChecksum { prefix: "-X-".into(), characters_pool: "ab".into(), base: 3 },
            AlgorithmConfig::RandomPoolWithChecksum { prefix: "".into(), characters_pool: "ab".into(), base: 2 },
            AlgorithmConfig::Hash { length: 0 },
            AlgorithmConfig::DigitRange { inclusive_lower_bound: 2, inclusive_upper_bound: 1 },
        ];
        for cfg in bad {
            assert!(Algorithm::compile(&cfg).is_err(), "{cfg:?} should be rejected");
        }
    }
}
