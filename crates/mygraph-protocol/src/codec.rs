//! Value encoding used in payloads.
//!
//! The scalar, list and map tags match the storage codec
//! (`mygraph-storage/src/codec.rs`). NODE and RELATIONSHIP exist only on the
//! wire, because they only appear in query results. Integers are
//! little-endian; strings, byte arrays, lists and maps are prefixed with a u32
//! length or count.

use mygraph_core::{MyGraphError, NodeValue, PropertyMap, RelationshipValue, Result, Value};

/// Maximum nesting of lists and maps accepted by the decoder.
pub const MAX_DEPTH: usize = 64;

pub mod tag {
    pub const NULL: u8 = 0;
    pub const BOOL: u8 = 1;
    pub const INT: u8 = 2;
    pub const FLOAT: u8 = 3;
    pub const STRING: u8 = 4;
    pub const BYTES: u8 = 5;
    pub const LIST: u8 = 6;
    pub const MAP: u8 = 7;
    pub const NODE: u8 = 8;
    pub const RELATIONSHIP: u8 = 9;
}

fn protocol_error(msg: impl Into<String>) -> MyGraphError {
    MyGraphError::Execution(format!("protocol error: {}", msg.into()))
}

/// Appends the encoding of `v` to `buf`.
pub fn encode_value(buf: &mut Vec<u8>, v: &Value) -> Result<()> {
    match v {
        Value::Null => buf.push(tag::NULL),
        Value::Boolean(b) => {
            buf.push(tag::BOOL);
            buf.push(*b as u8);
        }
        Value::Integer(i) => {
            buf.push(tag::INT);
            buf.extend_from_slice(&i.to_le_bytes());
        }
        Value::Float(f) => {
            buf.push(tag::FLOAT);
            buf.extend_from_slice(&f.to_bits().to_le_bytes());
        }
        Value::String(s) => {
            buf.push(tag::STRING);
            encode_bytes(buf, s.as_bytes())?;
        }
        Value::Bytes(b) => {
            buf.push(tag::BYTES);
            encode_bytes(buf, b)?;
        }
        Value::List(items) => {
            buf.push(tag::LIST);
            encode_len(buf, items.len())?;
            for i in items {
                encode_value(buf, i)?;
            }
        }
        Value::Map(m) => {
            buf.push(tag::MAP);
            encode_map(buf, m)?;
        }
        Value::Node(n) => {
            buf.push(tag::NODE);
            buf.extend_from_slice(&n.id.to_le_bytes());
            encode_len(buf, n.labels.len())?;
            for l in &n.labels {
                encode_bytes(buf, l.as_bytes())?;
            }
            encode_map(buf, &n.properties)?;
        }
        Value::Relationship(r) => {
            buf.push(tag::RELATIONSHIP);
            buf.extend_from_slice(&r.id.to_le_bytes());
            buf.extend_from_slice(&r.start_node.to_le_bytes());
            buf.extend_from_slice(&r.end_node.to_le_bytes());
            encode_bytes(buf, r.rel_type.as_bytes())?;
            encode_map(buf, &r.properties)?;
        }
    }
    Ok(())
}

fn encode_len(buf: &mut Vec<u8>, n: usize) -> Result<()> {
    let n = u32::try_from(n).map_err(|_| protocol_error("value too large to encode"))?;
    buf.extend_from_slice(&n.to_le_bytes());
    Ok(())
}

fn encode_bytes(buf: &mut Vec<u8>, b: &[u8]) -> Result<()> {
    encode_len(buf, b.len())?;
    buf.extend_from_slice(b);
    Ok(())
}

/// Encodes a map without its tag (count, then key/value pairs).
pub fn encode_map(buf: &mut Vec<u8>, m: &PropertyMap) -> Result<()> {
    encode_len(buf, m.len())?;
    for (k, v) in m {
        encode_bytes(buf, k.as_bytes())?;
        encode_value(buf, v)?;
    }
    Ok(())
}

/// Reads values from a byte slice.
pub struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Decoder { data, pos: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.data.len() - self.pos < n {
            return Err(protocol_error("unexpected end of payload"));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn bytes(&mut self) -> Result<Vec<u8>> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }

    fn str(&mut self) -> Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| protocol_error("invalid UTF-8 string"))
    }

    /// Guards against counts that cannot fit in the remaining bytes (each
    /// element takes at least one byte), so a hostile count cannot make the
    /// decoder allocate or loop for long.
    fn count(&mut self) -> Result<u32> {
        let n = self.u32()?;
        if n as usize > self.data.len() - self.pos {
            return Err(protocol_error("element count exceeds payload size"));
        }
        Ok(n)
    }

    pub fn value(&mut self) -> Result<Value> {
        self.value_at(0)
    }

    fn value_at(&mut self, depth: usize) -> Result<Value> {
        if depth > MAX_DEPTH {
            return Err(protocol_error("value nesting too deep"));
        }
        Ok(match self.u8()? {
            tag::NULL => Value::Null,
            tag::BOOL => Value::Boolean(self.u8()? != 0),
            tag::INT => Value::Integer(self.u64()? as i64),
            tag::FLOAT => Value::Float(f64::from_bits(self.u64()?)),
            tag::STRING => Value::String(self.str()?),
            tag::BYTES => Value::Bytes(self.bytes()?),
            tag::LIST => {
                let n = self.count()?;
                Value::List((0..n).map(|_| self.value_at(depth + 1)).collect::<Result<_>>()?)
            }
            tag::MAP => Value::Map(self.map_at(depth + 1)?),
            tag::NODE => {
                let id = self.u64()?;
                let n = self.count()?;
                let labels = (0..n).map(|_| self.str()).collect::<Result<_>>()?;
                let properties = self.map_at(depth + 1)?;
                Value::Node(Box::new(NodeValue { id, labels, properties }))
            }
            tag::RELATIONSHIP => Value::Relationship(Box::new(RelationshipValue {
                id: self.u64()?,
                start_node: self.u64()?,
                end_node: self.u64()?,
                rel_type: self.str()?,
                properties: self.map_at(depth + 1)?,
            })),
            t => return Err(protocol_error(format!("unknown value tag {t}"))),
        })
    }

    /// Decodes a map written by [`encode_map`] (no tag).
    pub fn map(&mut self) -> Result<PropertyMap> {
        self.map_at(0)
    }

    fn map_at(&mut self, depth: usize) -> Result<PropertyMap> {
        let n = self.count()?;
        let mut m = PropertyMap::new();
        for _ in 0..n {
            let k = self.str()?;
            m.insert(k, self.value_at(depth)?);
        }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(v: &Value) -> Value {
        let mut buf = Vec::new();
        encode_value(&mut buf, v).unwrap();
        let mut d = Decoder::new(&buf);
        let out = d.value().unwrap();
        assert!(d.is_empty());
        out
    }

    #[test]
    fn scalars_lists_maps_roundtrip() {
        let v = Value::from(vec![
            Value::Null,
            Value::from(true),
            Value::from(-42),
            Value::from(1.5),
            Value::from("héllo"),
            Value::Bytes(vec![0, 255]),
            Value::Map(PropertyMap::from([("k".into(), Value::from(vec![1, 2]))])),
        ]);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn nodes_and_relationships_roundtrip() {
        let props = PropertyMap::from([("name".into(), Value::from("Alice"))]);
        let n = Value::Node(Box::new(NodeValue {
            id: 7,
            labels: vec!["User".into(), "Admin".into()],
            properties: props.clone(),
        }));
        let r = Value::Relationship(Box::new(RelationshipValue {
            id: 3,
            start_node: 7,
            end_node: 8,
            rel_type: "KNOWS".into(),
            properties: props,
        }));
        assert_eq!(roundtrip(&n), n);
        assert_eq!(roundtrip(&r), r);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(Decoder::new(&[42]).value().is_err());
        assert!(Decoder::new(&[tag::INT, 1, 2]).value().is_err());
        // A list claiming u32::MAX elements in a 5-byte payload.
        assert!(Decoder::new(&[tag::LIST, 255, 255, 255, 255]).value().is_err());
        let mut deep = [tag::LIST, 1, 0, 0, 0].repeat(MAX_DEPTH + 2);
        deep.push(tag::NULL);
        assert!(Decoder::new(&deep).value().is_err());
    }
}
