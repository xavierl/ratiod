//! Minimal bencode decoder. Dictionaries keep their raw byte span so the
//! torrent info-hash can be computed without re-encoding.

use thiserror::Error;

const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum Value<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    List(Vec<Value<'a>>),
    /// Entries in file order, then the raw bytes of the whole dictionary.
    Dict(Vec<(&'a [u8], Value<'a>)>, &'a [u8]),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BencodeError {
    #[error("unexpected end of data")]
    Eof,
    #[error("invalid bencode at byte {0}")]
    Invalid(usize),
    #[error("nesting too deep")]
    TooDeep,
}

/// Decodes the first bencoded value in `data`; trailing bytes are ignored.
pub fn decode(data: &[u8]) -> Result<Value<'_>, BencodeError> {
    Parser { data, pos: 0 }.value(0)
}

impl<'a> Value<'a> {
    pub fn get(&self, key: &str) -> Option<&Value<'a>> {
        match self {
            Value::Dict(entries, _) => entries.iter().find(|(k, _)| *k == key.as_bytes()).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&'a [u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&'a str> {
        std::str::from_utf8(self.as_bytes()?).ok()
    }

    pub fn as_list(&self) -> Option<&[Value<'a>]> {
        match self {
            Value::List(items) => Some(items),
            _ => None,
        }
    }

    /// Raw bytes of a dictionary, exactly as they appear in the input.
    pub fn raw(&self) -> Option<&'a [u8]> {
        match self {
            Value::Dict(_, raw) => Some(raw),
            _ => None,
        }
    }
}

struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Result<u8, BencodeError> {
        self.data.get(self.pos).copied().ok_or(BencodeError::Eof)
    }

    fn value(&mut self, depth: usize) -> Result<Value<'a>, BencodeError> {
        if depth > MAX_DEPTH {
            return Err(BencodeError::TooDeep);
        }
        match self.peek()? {
            b'i' => {
                self.pos += 1;
                Ok(Value::Int(self.number(b'e')?))
            }
            b'l' => {
                self.pos += 1;
                let mut items = Vec::new();
                while self.peek()? != b'e' {
                    items.push(self.value(depth + 1)?);
                }
                self.pos += 1;
                Ok(Value::List(items))
            }
            b'd' => {
                let start = self.pos;
                self.pos += 1;
                let mut entries = Vec::new();
                while self.peek()? != b'e' {
                    let key = self.bytes()?;
                    let value = self.value(depth + 1)?;
                    entries.push((key, value));
                }
                self.pos += 1;
                Ok(Value::Dict(entries, &self.data[start..self.pos]))
            }
            b'0'..=b'9' => Ok(Value::Bytes(self.bytes()?)),
            _ => Err(BencodeError::Invalid(self.pos)),
        }
    }

    fn number(&mut self, terminator: u8) -> Result<i64, BencodeError> {
        let start = self.pos;
        let len = self.data[start..].iter().position(|&b| b == terminator).ok_or(BencodeError::Eof)?;
        let text = std::str::from_utf8(&self.data[start..start + len]).map_err(|_| BencodeError::Invalid(start))?;
        let n = text.parse::<i64>().map_err(|_| BencodeError::Invalid(start))?;
        self.pos = start + len + 1;
        Ok(n)
    }

    fn bytes(&mut self) -> Result<&'a [u8], BencodeError> {
        let start = self.pos;
        if !self.peek()?.is_ascii_digit() {
            return Err(BencodeError::Invalid(start));
        }
        let len = usize::try_from(self.number(b':')?).map_err(|_| BencodeError::Invalid(start))?;
        let end = self.pos.checked_add(len).ok_or(BencodeError::Invalid(start))?;
        if end > self.data.len() {
            return Err(BencodeError::Eof);
        }
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_integers() {
        assert_eq!(decode(b"i42e").unwrap(), Value::Int(42));
        assert_eq!(decode(b"i-7e").unwrap(), Value::Int(-7));
    }

    #[test]
    fn decodes_byte_strings() {
        assert_eq!(decode(b"4:spam").unwrap(), Value::Bytes(b"spam"));
        assert_eq!(decode(b"0:").unwrap(), Value::Bytes(b""));
    }

    #[test]
    fn decodes_lists_and_dicts() {
        let v = decode(b"d4:listli1ei2ee3:str3:abce").unwrap();
        let list = v.get("list").and_then(Value::as_list).unwrap();
        assert_eq!(list, &[Value::Int(1), Value::Int(2)]);
        assert_eq!(v.get("str").and_then(Value::as_str), Some("abc"));
        assert_eq!(v.get("missing"), None);
    }

    #[test]
    fn dict_keeps_its_raw_bytes() {
        let data = b"d4:infod1:ai1eee";
        let v = decode(data).unwrap();
        assert_eq!(v.get("info").unwrap().raw(), Some(&b"d1:ai1ee"[..]));
        assert_eq!(v.raw(), Some(&data[..]));
    }

    #[test]
    fn ignores_trailing_data() {
        assert_eq!(decode(b"i1eXYZ").unwrap(), Value::Int(1));
    }

    #[test]
    fn rejects_truncated_input() {
        assert_eq!(decode(b"5:abc"), Err(BencodeError::Eof));
        assert_eq!(decode(b"li1e"), Err(BencodeError::Eof));
        assert_eq!(decode(b""), Err(BencodeError::Eof));
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(decode(b"<html>"), Err(BencodeError::Invalid(0))));
        assert!(matches!(decode(b"iabce"), Err(BencodeError::Invalid(_))));
        assert!(matches!(decode(b"d1:a"), Err(BencodeError::Eof)));
    }

    #[test]
    fn rejects_deep_nesting() {
        let data = "l".repeat(100) + &"e".repeat(100);
        assert_eq!(decode(data.as_bytes()), Err(BencodeError::TooDeep));
    }

    #[test]
    fn decodes_testutil_fixtures() {
        let t = crate::testutil::torrent_file("x", 5, "http://t/announce");
        let v = decode(&t).unwrap();
        assert_eq!(v.get("info").and_then(|i| i.get("length")).and_then(Value::as_int), Some(5));
    }
}
