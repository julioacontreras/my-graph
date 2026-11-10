use std::collections::{BTreeMap, HashMap};
use std::fmt;

/// Properties of a node or relationship. Ordered so iteration, display and the
/// persisted encoding are deterministic.
pub type PropertyMap = BTreeMap<String, Value>;

/// A MyGraph value.
///
/// Every variant except `Node` and `Relationship` can be stored as a property.
/// `Node` and `Relationship` only appear in query results.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
    Node(Box<NodeValue>),
    Relationship(Box<RelationshipValue>),
}

/// A node as returned in a query result.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeValue {
    pub id: u64,
    pub labels: Vec<String>,
    pub properties: PropertyMap,
}

/// A relationship as returned in a query result.
#[derive(Debug, Clone, PartialEq)]
pub struct RelationshipValue {
    pub id: u64,
    pub start_node: u64,
    pub end_node: u64,
    pub rel_type: String,
    pub properties: PropertyMap,
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Whether the value may be stored as a property.
    pub fn is_storable(&self) -> bool {
        match self {
            Value::Node(_) | Value::Relationship(_) => false,
            Value::List(items) => items.iter().all(Value::is_storable),
            Value::Map(map) => map.values().all(Value::is_storable),
            _ => true,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "NULL",
            Value::Boolean(_) => "BOOLEAN",
            Value::Integer(_) => "INTEGER",
            Value::Float(_) => "FLOAT",
            Value::String(_) => "STRING",
            Value::Bytes(_) => "BYTES",
            Value::List(_) => "LIST",
            Value::Map(_) => "MAP",
            Value::Node(_) => "NODE",
            Value::Relationship(_) => "RELATIONSHIP",
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_node(&self) -> Option<&NodeValue> {
        match self {
            Value::Node(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_relationship(&self) -> Option<&RelationshipValue> {
        match self {
            Value::Relationship(r) => Some(r),
            _ => None,
        }
    }
}

macro_rules! impl_from {
    ($($t:ty => $variant:ident as $conv:ty),* $(,)?) => {
        $(impl From<$t> for Value {
            fn from(v: $t) -> Self {
                Value::$variant(<$conv>::from(v))
            }
        })*
    };
}

impl_from! {
    bool => Boolean as bool,
    i8 => Integer as i64,
    i16 => Integer as i64,
    i32 => Integer as i64,
    i64 => Integer as i64,
    u8 => Integer as i64,
    u16 => Integer as i64,
    u32 => Integer as i64,
    f32 => Float as f64,
    f64 => Float as f64,
    String => String as String,
    &str => String as String,
    &String => String as String,
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::List(v.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}

impl<T: Into<Value>> From<BTreeMap<String, T>> for Value {
    fn from(m: BTreeMap<String, T>) -> Self {
        Value::Map(m.into_iter().map(|(k, v)| (k, v.into())).collect())
    }
}

impl<T: Into<Value>> From<HashMap<String, T>> for Value {
    fn from(m: HashMap<String, T>) -> Self {
        Value::Map(m.into_iter().map(|(k, v)| (k, v.into())).collect())
    }
}

/// Formats an identifier, quoting it with backticks when needed.
pub fn format_identifier(name: &str) -> String {
    let simple = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if simple {
        name.to_string()
    } else {
        format!("`{}`", name.replace('`', "``"))
    }
}

pub(crate) fn format_float(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        }
    } else if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

fn write_string_literal(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in s.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c => write!(f, "{c}")?,
        }
    }
    f.write_str("\"")
}

fn write_map(f: &mut fmt::Formatter<'_>, map: &BTreeMap<String, Value>) -> fmt::Result {
    f.write_str("{")?;
    for (i, (k, v)) in map.iter().enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{}: {v}", format_identifier(k))?;
    }
    f.write_str("}")
}

/// Displays values using Cypher literal syntax.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => f.write_str("null"),
            Value::Boolean(b) => write!(f, "{b}"),
            Value::Integer(i) => write!(f, "{i}"),
            Value::Float(x) => f.write_str(&format_float(*x)),
            Value::String(s) => write_string_literal(f, s),
            Value::Bytes(b) => {
                f.write_str("bytes(0x")?;
                for byte in b {
                    write!(f, "{byte:02x}")?;
                }
                f.write_str(")")
            }
            Value::List(items) => {
                f.write_str("[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_str("]")
            }
            Value::Map(map) => write_map(f, map),
            Value::Node(n) => {
                f.write_str("(")?;
                for l in &n.labels {
                    write!(f, ":{}", format_identifier(l))?;
                }
                if !n.properties.is_empty() {
                    if !n.labels.is_empty() {
                        f.write_str(" ")?;
                    }
                    write_map(f, &n.properties)?;
                }
                f.write_str(")")
            }
            Value::Relationship(r) => {
                write!(f, "[:{}", format_identifier(&r.rel_type))?;
                if !r.properties.is_empty() {
                    f.write_str(" ")?;
                    write_map(f, &r.properties)?;
                }
                f.write_str("]")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_uses_cypher_literals() {
        let v = Value::from(vec![Value::from(1), Value::from(2.0), Value::from("a\"b")]);
        assert_eq!(v.to_string(), r#"[1, 2.0, "a\"b"]"#);
        assert_eq!(format_identifier("my label"), "`my label`");
    }

    #[test]
    fn graph_values_are_not_storable() {
        let n = Value::Node(Box::new(NodeValue { id: 1, labels: vec![], properties: PropertyMap::new() }));
        assert!(!Value::List(vec![n]).is_storable());
        assert!(Value::from(vec![1, 2]).is_storable());
    }
}
