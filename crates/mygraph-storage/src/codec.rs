//! Versioned binary encoding of MyGraph values and operations.
//!
//! All integers are little-endian. Strings and byte arrays are length-prefixed
//! with a u32. This encoding is independent of Rust's in-memory layout.

use mygraph_core::{Change, MyGraphError, Op, PropertyMap, Result, Value};

const MAX_DEPTH: usize = 64;

pub(crate) mod tag {
    pub const NULL: u8 = 0;
    pub const BOOL: u8 = 1;
    pub const INT: u8 = 2;
    pub const FLOAT: u8 = 3;
    pub const STRING: u8 = 4;
    pub const BYTES: u8 = 5;
    pub const LIST: u8 = 6;
    pub const MAP: u8 = 7;
}

#[derive(Default)]
pub(crate) struct Encoder {
    pub buf: Vec<u8>,
}

impl Encoder {
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn len(&mut self, n: usize) -> Result<()> {
        let n = u32::try_from(n).map_err(|_| MyGraphError::Storage("value too large to encode".into()))?;
        self.u32(n);
        Ok(())
    }

    pub fn bytes(&mut self, b: &[u8]) -> Result<()> {
        self.len(b.len())?;
        self.buf.extend_from_slice(b);
        Ok(())
    }

    pub fn str(&mut self, s: &str) -> Result<()> {
        self.bytes(s.as_bytes())
    }

    pub fn value(&mut self, v: &Value) -> Result<()> {
        match v {
            Value::Null => self.u8(tag::NULL),
            Value::Boolean(b) => {
                self.u8(tag::BOOL);
                self.u8(*b as u8);
            }
            Value::Integer(i) => {
                self.u8(tag::INT);
                self.u64(*i as u64);
            }
            Value::Float(f) => {
                self.u8(tag::FLOAT);
                self.u64(f.to_bits());
            }
            Value::String(s) => {
                self.u8(tag::STRING);
                self.str(s)?;
            }
            Value::Bytes(b) => {
                self.u8(tag::BYTES);
                self.bytes(b)?;
            }
            Value::List(items) => {
                self.u8(tag::LIST);
                self.len(items.len())?;
                for i in items {
                    self.value(i)?;
                }
            }
            Value::Map(m) => {
                self.u8(tag::MAP);
                self.properties(m)?;
            }
            Value::Node(_) | Value::Relationship(_) => {
                return Err(MyGraphError::Storage(format!("{} values cannot be persisted", v.type_name())))
            }
        }
        Ok(())
    }

    pub fn properties(&mut self, m: &PropertyMap) -> Result<()> {
        self.len(m.len())?;
        for (k, v) in m {
            self.str(k)?;
            self.value(v)?;
        }
        Ok(())
    }

    pub fn strings(&mut self, items: &[String]) -> Result<()> {
        self.len(items.len())?;
        for s in items {
            self.str(s)?;
        }
        Ok(())
    }

    fn opt_value(&mut self, v: &Option<Value>) -> Result<()> {
        match v {
            None => self.u8(0),
            Some(v) => {
                self.u8(1);
                self.value(v)?;
            }
        }
        Ok(())
    }

    pub fn change(&mut self, c: &Change) -> Result<()> {
        match c {
            Change::CreateNode { id, labels, properties, created_at, updated_at } => {
                self.u8(1);
                self.u64(*id);
                self.strings(labels)?;
                self.properties(properties)?;
                self.u64(*created_at);
                self.u64(*updated_at);
            }
            Change::DeleteNode { id } => {
                self.u8(2);
                self.u64(*id);
            }
            Change::CreateRelationship { id, start, end, rel_type, properties } => {
                self.u8(3);
                self.u64(*id);
                self.u64(*start);
                self.u64(*end);
                self.str(rel_type)?;
                self.properties(properties)?;
            }
            Change::DeleteRelationship { id } => {
                self.u8(4);
                self.u64(*id);
            }
            Change::SetNodeProperty { id, key, value, ts } => {
                self.u8(5);
                self.u64(*id);
                self.str(key)?;
                self.opt_value(value)?;
                self.u64(*ts);
            }
            Change::SetRelationshipProperty { id, key, value } => {
                self.u8(6);
                self.u64(*id);
                self.str(key)?;
                self.opt_value(value)?;
            }
            Change::AddLabel { id, label, ts } => {
                self.u8(7);
                self.u64(*id);
                self.str(label)?;
                self.u64(*ts);
            }
            Change::RemoveLabel { id, label, ts } => {
                self.u8(8);
                self.u64(*id);
                self.str(label)?;
                self.u64(*ts);
            }
        }
        Ok(())
    }

    pub fn op(&mut self, op: &Op) -> Result<()> {
        match op {
            Op::CreateDatabase { name } => {
                self.u8(1);
                self.str(name)?;
            }
            Op::DropDatabase { name } => {
                self.u8(2);
                self.str(name)?;
            }
            Op::CreateSchema { database, name } => {
                self.u8(3);
                self.str(database)?;
                self.str(name)?;
            }
            Op::DropSchema { database, name } => {
                self.u8(4);
                self.str(database)?;
                self.str(name)?;
            }
            Op::Graph { database, schema, change } => {
                self.u8(5);
                self.str(database)?;
                self.str(schema)?;
                self.change(change)?;
            }
        }
        Ok(())
    }
}

pub(crate) struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}

fn corrupt(msg: impl Into<String>) -> MyGraphError {
    MyGraphError::Corruption(msg.into())
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
            return Err(corrupt("unexpected end of data"));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    pub fn bytes(&mut self) -> Result<Vec<u8>> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }

    pub fn str(&mut self) -> Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| corrupt("invalid UTF-8 string"))
    }

    pub fn strings(&mut self) -> Result<Vec<String>> {
        let n = self.u32()?;
        (0..n).map(|_| self.str()).collect()
    }

    pub fn value(&mut self) -> Result<Value> {
        self.value_at(0)
    }

    fn value_at(&mut self, depth: usize) -> Result<Value> {
        if depth > MAX_DEPTH {
            return Err(corrupt("value nesting too deep"));
        }
        Ok(match self.u8()? {
            tag::NULL => Value::Null,
            tag::BOOL => Value::Boolean(self.u8()? != 0),
            tag::INT => Value::Integer(self.u64()? as i64),
            tag::FLOAT => Value::Float(f64::from_bits(self.u64()?)),
            tag::STRING => Value::String(self.str()?),
            tag::BYTES => Value::Bytes(self.bytes()?),
            tag::LIST => {
                let n = self.u32()?;
                Value::List((0..n).map(|_| self.value_at(depth + 1)).collect::<Result<_>>()?)
            }
            tag::MAP => Value::Map(self.properties_at(depth + 1)?),
            t => return Err(corrupt(format!("unknown value tag {t}"))),
        })
    }

    pub fn properties(&mut self) -> Result<PropertyMap> {
        self.properties_at(0)
    }

    fn properties_at(&mut self, depth: usize) -> Result<PropertyMap> {
        let n = self.u32()?;
        let mut m = PropertyMap::new();
        for _ in 0..n {
            let k = self.str()?;
            m.insert(k, self.value_at(depth)?);
        }
        Ok(m)
    }

    fn opt_value(&mut self) -> Result<Option<Value>> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.value()?)),
            t => Err(corrupt(format!("invalid option tag {t}"))),
        }
    }

    pub fn change(&mut self) -> Result<Change> {
        Ok(match self.u8()? {
            1 => Change::CreateNode {
                id: self.u64()?,
                labels: self.strings()?,
                properties: self.properties()?,
                created_at: self.u64()?,
                updated_at: self.u64()?,
            },
            2 => Change::DeleteNode { id: self.u64()? },
            3 => Change::CreateRelationship {
                id: self.u64()?,
                start: self.u64()?,
                end: self.u64()?,
                rel_type: self.str()?,
                properties: self.properties()?,
            },
            4 => Change::DeleteRelationship { id: self.u64()? },
            5 => {
                Change::SetNodeProperty { id: self.u64()?, key: self.str()?, value: self.opt_value()?, ts: self.u64()? }
            }
            6 => Change::SetRelationshipProperty { id: self.u64()?, key: self.str()?, value: self.opt_value()? },
            7 => Change::AddLabel { id: self.u64()?, label: self.str()?, ts: self.u64()? },
            8 => Change::RemoveLabel { id: self.u64()?, label: self.str()?, ts: self.u64()? },
            t => return Err(corrupt(format!("unknown change tag {t}"))),
        })
    }

    pub fn op(&mut self) -> Result<Op> {
        Ok(match self.u8()? {
            1 => Op::CreateDatabase { name: self.str()? },
            2 => Op::DropDatabase { name: self.str()? },
            3 => Op::CreateSchema { database: self.str()?, name: self.str()? },
            4 => Op::DropSchema { database: self.str()?, name: self.str()? },
            5 => Op::Graph { database: self.str()?, schema: self.str()?, change: self.change()? },
            t => return Err(corrupt(format!("unknown operation tag {t}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_roundtrip() {
        let v = Value::from(vec![
            Value::Null,
            Value::from(true),
            Value::from(-42),
            Value::from(1.5),
            Value::from("héllo"),
            Value::Bytes(vec![1, 2, 3]),
            Value::Map(PropertyMap::from([("k".into(), Value::from(vec![1, 2]))])),
        ]);
        let mut e = Encoder::default();
        e.value(&v).unwrap();
        let mut d = Decoder::new(&e.buf);
        assert_eq!(d.value().unwrap(), v);
        assert!(d.is_empty());
    }

    #[test]
    fn op_roundtrip_and_truncation() {
        let op = Op::Graph {
            database: "db".into(),
            schema: "s".into(),
            change: Change::SetNodeProperty { id: 3, key: "name".into(), value: Some("x".into()), ts: 9 },
        };
        let mut e = Encoder::default();
        e.op(&op).unwrap();
        assert_eq!(Decoder::new(&e.buf).op().unwrap(), op);
        assert!(Decoder::new(&e.buf[..e.buf.len() - 1]).op().is_err());
    }
}
