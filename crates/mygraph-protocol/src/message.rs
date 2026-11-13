//! Message types and payload helpers.

use mygraph_core::{MyGraphError, PropertyMap, Result, Value};

use crate::codec::{encode_map, Decoder};

/// Message type byte of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    Hello = 0x01,
    Auth = 0x02,
    Query = 0x10,
    QueryResult = 0x11,
    Begin = 0x20,
    Commit = 0x21,
    Rollback = 0x22,
    Ok = 0x23,
    Ping = 0x30,
    Pong = 0x31,
    Goodbye = 0x40,
    Error = 0x7F,
}

impl MessageType {
    pub fn from_u8(b: u8) -> Option<Self> {
        use MessageType::*;
        Some(match b {
            0x01 => Hello,
            0x02 => Auth,
            0x10 => Query,
            0x11 => QueryResult,
            0x20 => Begin,
            0x21 => Commit,
            0x22 => Rollback,
            0x23 => Ok,
            0x30 => Ping,
            0x31 => Pong,
            0x40 => Goodbye,
            0x7F => Error,
            _ => return None,
        })
    }
}

/// A message payload: always a map of named values.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Payload(pub PropertyMap);

impl Payload {
    pub fn new() -> Self {
        Payload::default()
    }

    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.0.insert(key.into(), value.into());
        self
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        encode_map(&mut buf, &self.0)?;
        Ok(buf)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut d = Decoder::new(bytes);
        let m = d.map()?;
        if !d.is_empty() {
            return Err(MyGraphError::Execution("protocol error: trailing bytes after payload".into()));
        }
        Ok(Payload(m))
    }
}

/// Stable error code for an engine error, sent in ERROR messages.
pub fn error_code(e: &MyGraphError) -> &'static str {
    match e {
        MyGraphError::Syntax { .. } => "Syntax",
        MyGraphError::Semantic(_) => "Semantic",
        MyGraphError::Type(_) => "Type",
        MyGraphError::Execution(_) => "Execution",
        MyGraphError::NotFound(_) => "NotFound",
        MyGraphError::AlreadyExists(_) => "AlreadyExists",
        MyGraphError::InvalidName(_) => "InvalidName",
        MyGraphError::Unsupported(_) => "Unsupported",
        MyGraphError::ConstraintViolation(_) => "ConstraintViolation",
        MyGraphError::Transaction(_) => "Transaction",
        MyGraphError::Storage(_) => "Storage",
        MyGraphError::Corruption(_) => "Corruption",
        MyGraphError::Io(_) => "Io",
    }
}

/// ERROR payload for an engine error: `{code, message, line?, column?}`.
pub fn error_payload(e: &MyGraphError) -> Payload {
    let mut p = Payload::new().with("code", error_code(e)).with("message", e.to_string());
    if let MyGraphError::Syntax { line, column, .. } = e {
        p = p.with("line", *line as i64).with("column", *column as i64);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_roundtrip() {
        let p = Payload::new().with("query", "RETURN 1").with("params", Value::Map(PropertyMap::new()));
        assert_eq!(Payload::decode(&p.encode().unwrap()).unwrap(), p);
        let mut bytes = p.encode().unwrap();
        bytes.push(0);
        assert!(Payload::decode(&bytes).is_err());
    }

    #[test]
    fn message_types() {
        for b in 0..=255u8 {
            if let Some(t) = MessageType::from_u8(b) {
                assert_eq!(t as u8, b);
            }
        }
    }

    #[test]
    fn syntax_error_has_position() {
        let e = MyGraphError::Syntax { message: "x".into(), line: 2, column: 5 };
        let p = error_payload(&e);
        assert_eq!(p.get("code"), Some(&Value::from("Syntax")));
        assert_eq!(p.get("line"), Some(&Value::Integer(2)));
    }
}
