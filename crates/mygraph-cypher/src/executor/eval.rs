//! Runtime values and expression evaluation.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use mygraph_core::{Graph, MyGraphError, NodeId, RelId, Result, Value};

use crate::ast::{is_aggregate_function, BinaryOp, Expr, UnaryOp};
use crate::executor::functions;

/// Executor-internal value. Unlike [`Value`], nodes and relationships are
/// references by id, so bindings stay cheap and always see current data.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<Val>),
    Map(BTreeMap<String, Val>),
    Node(NodeId),
    Rel(RelId),
    /// `nodes.len() == rels.len() + 1`; rel `i` joins nodes `i` and `i + 1`.
    Path {
        nodes: Vec<NodeId>,
        rels: Vec<RelId>,
    },
}

pub(crate) type Row = HashMap<String, Val>;

pub(crate) fn type_err(msg: impl Into<String>) -> MyGraphError {
    MyGraphError::Type(msg.into())
}

impl Val {
    pub fn from_value(v: &Value) -> Val {
        match v {
            Value::Null => Val::Null,
            Value::Boolean(b) => Val::Bool(*b),
            Value::Integer(i) => Val::Int(*i),
            Value::Float(f) => Val::Float(*f),
            Value::String(s) => Val::Str(s.clone()),
            Value::Bytes(b) => Val::Bytes(b.clone()),
            Value::List(l) => Val::List(l.iter().map(Val::from_value).collect()),
            Value::Map(m) => Val::Map(m.iter().map(|(k, v)| (k.clone(), Val::from_value(v))).collect()),
            Value::Node(n) => Val::Node(n.id),
            Value::Relationship(r) => Val::Rel(r.id),
            Value::Path(p) => Val::Path {
                nodes: p.nodes.iter().map(|n| n.id).collect(),
                rels: p.relationships.iter().map(|r| r.id).collect(),
            },
        }
    }

    /// Converts to a storable property value.
    pub fn into_property(self) -> Result<Value> {
        Ok(match self {
            Val::Null => Value::Null,
            Val::Bool(b) => Value::Boolean(b),
            Val::Int(i) => Value::Integer(i),
            Val::Float(f) => Value::Float(f),
            Val::Str(s) => Value::String(s),
            Val::Bytes(b) => Value::Bytes(b),
            Val::List(l) => Value::List(l.into_iter().map(Val::into_property).collect::<Result<_>>()?),
            Val::Map(m) => Value::Map(m.into_iter().map(|(k, v)| Ok((k, v.into_property()?))).collect::<Result<_>>()?),
            Val::Node(_) | Val::Rel(_) | Val::Path { .. } => {
                return Err(type_err("nodes, relationships and paths cannot be stored as property values"))
            }
        })
    }

    /// Converts to a result value, reading node/relationship data from the graph.
    pub fn materialize(&self, g: &Graph) -> Result<Value> {
        Ok(match self {
            Val::Node(id) => Value::Node(Box::new(
                g.node_value(*id).ok_or_else(|| MyGraphError::Execution(format!("node {id} has been deleted")))?,
            )),
            Val::Rel(id) => Value::Relationship(Box::new(
                g.relationship_value(*id)
                    .ok_or_else(|| MyGraphError::Execution(format!("relationship {id} has been deleted")))?,
            )),
            Val::Path { nodes, rels } => {
                let gone = || MyGraphError::Execution("a path element has been deleted".into());
                Value::Path(Box::new(mygraph_core::PathValue {
                    nodes: nodes.iter().map(|n| g.node_value(*n).ok_or_else(gone)).collect::<Result<_>>()?,
                    relationships: rels
                        .iter()
                        .map(|r| g.relationship_value(*r).ok_or_else(gone))
                        .collect::<Result<_>>()?,
                }))
            }
            Val::List(l) => Value::List(l.iter().map(|v| v.materialize(g)).collect::<Result<_>>()?),
            Val::Map(m) => {
                Value::Map(m.iter().map(|(k, v)| Ok((k.clone(), v.materialize(g)?))).collect::<Result<_>>()?)
            }
            other => other.clone().into_property()?,
        })
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Val::Null => "NULL",
            Val::Bool(_) => "BOOLEAN",
            Val::Int(_) => "INTEGER",
            Val::Float(_) => "FLOAT",
            Val::Str(_) => "STRING",
            Val::Bytes(_) => "BYTES",
            Val::List(_) => "LIST",
            Val::Map(_) => "MAP",
            Val::Node(_) => "NODE",
            Val::Rel(_) => "RELATIONSHIP",
            Val::Path { .. } => "PATH",
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            Val::Int(i) => Some(*i as f64),
            Val::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Text used when concatenating with a string.
    pub fn to_display_string(&self) -> String {
        match self {
            Val::Str(s) => s.clone(),
            Val::Int(i) => i.to_string(),
            Val::Float(f) => Value::Float(*f).to_string(),
            Val::Bool(b) => b.to_string(),
            other => format!("{other:?}"),
        }
    }
}

/// Key used for grouping and DISTINCT. Values that are equal under Cypher
/// equivalence map to the same key (integers and equal floats included).
pub(crate) fn group_key(vals: &[&Val]) -> String {
    fn push(v: &Val, out: &mut String) {
        match v {
            Val::Float(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => out.push_str(&format!("N{}", *f as i64)),
            Val::Int(i) => out.push_str(&format!("N{i}")),
            Val::List(items) => {
                out.push('[');
                for i in items {
                    push(i, out);
                    out.push(',');
                }
                out.push(']');
            }
            Val::Map(m) => {
                out.push('{');
                for (k, v) in m {
                    out.push_str(&format!("{k:?}:"));
                    push(v, out);
                    out.push(',');
                }
                out.push('}');
            }
            other => out.push_str(&format!("{other:?}")),
        }
    }
    let mut s = String::new();
    for v in vals {
        push(v, &mut s);
        s.push('|');
    }
    s
}

/// Cypher equality: `None` means `null` (unknown).
pub(crate) fn equals(a: &Val, b: &Val) -> Option<bool> {
    match (a, b) {
        (Val::Null, _) | (_, Val::Null) => None,
        (Val::Int(x), Val::Int(y)) => Some(x == y),
        (Val::Int(_) | Val::Float(_), Val::Int(_) | Val::Float(_)) => Some(a.as_f64() == b.as_f64()),
        (Val::List(x), Val::List(y)) => {
            if x.len() != y.len() {
                return Some(false);
            }
            let mut result = Some(true);
            for (p, q) in x.iter().zip(y) {
                match equals(p, q) {
                    Some(false) => return Some(false),
                    None => result = None,
                    Some(true) => {}
                }
            }
            result
        }
        (Val::Map(x), Val::Map(y)) => {
            if x.len() != y.len() || x.keys().ne(y.keys()) {
                return Some(false);
            }
            let mut result = Some(true);
            for (p, q) in x.values().zip(y.values()) {
                match equals(p, q) {
                    Some(false) => return Some(false),
                    None => result = None,
                    Some(true) => {}
                }
            }
            result
        }
        _ => Some(a == b),
    }
}

/// Comparison for `<`, `<=`, `>`, `>=`. `None` means the values are not comparable (null).
pub(crate) fn compare(a: &Val, b: &Val) -> Option<Ordering> {
    match (a, b) {
        (Val::Int(x), Val::Int(y)) => Some(x.cmp(y)),
        (Val::Int(_) | Val::Float(_), Val::Int(_) | Val::Float(_)) => a.as_f64()?.partial_cmp(&b.as_f64()?),
        (Val::Str(x), Val::Str(y)) => Some(x.cmp(y)),
        (Val::Bool(x), Val::Bool(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

fn order_rank(v: &Val) -> u8 {
    match v {
        Val::Map(_) => 0,
        Val::Node(_) => 1,
        Val::Rel(_) => 2,
        Val::Path { .. } => 8,
        Val::List(_) => 3,
        Val::Str(_) => 4,
        Val::Bool(_) => 5,
        Val::Int(_) | Val::Float(_) => 6,
        Val::Bytes(_) => 7,
        Val::Null => 9,
    }
}

/// Total order used by ORDER BY (nulls sort last in ascending order).
pub(crate) fn order_cmp(a: &Val, b: &Val) -> Ordering {
    let (ra, rb) = (order_rank(a), order_rank(b));
    if ra != rb {
        return ra.cmp(&rb);
    }
    match (a, b) {
        (Val::Int(x), Val::Int(y)) => x.cmp(y),
        (Val::Int(_) | Val::Float(_), _) => a.as_f64().unwrap().total_cmp(&b.as_f64().unwrap()),
        (Val::Str(x), Val::Str(y)) => x.cmp(y),
        (Val::Bool(x), Val::Bool(y)) => x.cmp(y),
        (Val::Bytes(x), Val::Bytes(y)) => x.cmp(y),
        (Val::Node(x), Val::Node(y)) | (Val::Rel(x), Val::Rel(y)) => x.cmp(y),
        (Val::Path { nodes: a, rels: x }, Val::Path { nodes: b, rels: y }) => a.cmp(b).then_with(|| x.cmp(y)),
        (Val::List(x), Val::List(y)) => {
            for (p, q) in x.iter().zip(y) {
                let c = order_cmp(p, q);
                if c != Ordering::Equal {
                    return c;
                }
            }
            x.len().cmp(&y.len())
        }
        (Val::Map(x), Val::Map(y)) => {
            for ((ka, va), (kb, vb)) in x.iter().zip(y) {
                let c = ka.cmp(kb).then_with(|| order_cmp(va, vb));
                if c != Ordering::Equal {
                    return c;
                }
            }
            x.len().cmp(&y.len())
        }
        _ => Ordering::Equal,
    }
}

fn truth(v: &Val, op: &str) -> Result<Option<bool>> {
    match v {
        Val::Null => Ok(None),
        Val::Bool(b) => Ok(Some(*b)),
        other => Err(type_err(format!("{op} expects BOOLEAN operands, got {}", other.type_name()))),
    }
}

fn from_truth(t: Option<bool>) -> Val {
    t.map_or(Val::Null, Val::Bool)
}

pub(crate) fn arithmetic(op: BinaryOp, l: Val, r: Val) -> Result<Val> {
    use Val::*;
    let overflow = || MyGraphError::Execution("integer overflow".into());
    if let (List(mut a), b) = (l.clone(), r.clone()) {
        if op == BinaryOp::Add {
            match b {
                List(b) => a.extend(b),
                other => a.push(other),
            }
            return Ok(List(a));
        }
    }
    if let (a, List(b)) = (&l, &r) {
        if op == BinaryOp::Add {
            let mut out = vec![a.clone()];
            out.extend(b.iter().cloned());
            return Ok(List(out));
        }
    }
    if matches!(l, Null) || matches!(r, Null) {
        return Ok(Null);
    }
    Ok(match (op, &l, &r) {
        (BinaryOp::Add, Str(a), b @ (Str(_) | Int(_) | Float(_))) => Str(format!("{a}{}", b.to_display_string())),
        (BinaryOp::Add, a @ (Int(_) | Float(_)), Str(b)) => Str(format!("{}{b}", a.to_display_string())),
        (BinaryOp::Add, Int(a), Int(b)) => Int(a.checked_add(*b).ok_or_else(overflow)?),
        (BinaryOp::Sub, Int(a), Int(b)) => Int(a.checked_sub(*b).ok_or_else(overflow)?),
        (BinaryOp::Mul, Int(a), Int(b)) => Int(a.checked_mul(*b).ok_or_else(overflow)?),
        (BinaryOp::Div, Int(a), Int(b)) => {
            if *b == 0 {
                return Err(MyGraphError::Execution("division by zero".into()));
            }
            Int(a.checked_div(*b).ok_or_else(overflow)?)
        }
        (BinaryOp::Mod, Int(a), Int(b)) => {
            if *b == 0 {
                return Err(MyGraphError::Execution("division by zero".into()));
            }
            Int(a.checked_rem(*b).ok_or_else(overflow)?)
        }
        (_, Int(_) | Float(_), Int(_) | Float(_)) => {
            let (a, b) = (l.as_f64().unwrap(), r.as_f64().unwrap());
            Float(match op {
                BinaryOp::Add => a + b,
                BinaryOp::Sub => a - b,
                BinaryOp::Mul => a * b,
                BinaryOp::Div => a / b,
                BinaryOp::Mod => a % b,
                BinaryOp::Pow => a.powf(b),
                _ => unreachable!("not arithmetic"),
            })
        }
        _ => return Err(type_err(format!("cannot apply {op:?} to {} and {}", l.type_name(), r.type_name()))),
    })
}

/// Evaluation environment: the graph being queried and the query parameters.
pub(crate) struct Env<'a> {
    pub graph: &'a Graph,
    pub params: &'a BTreeMap<String, Val>,
}

impl Env<'_> {
    pub fn eval(&self, e: &Expr, row: &Row) -> Result<Val> {
        match e {
            Expr::Literal(v) => Ok(Val::from_value(v)),
            Expr::Parameter(name) => self
                .params
                .get(name)
                .cloned()
                .ok_or_else(|| MyGraphError::Semantic(format!("missing parameter `${name}`"))),
            Expr::Variable(name) => row
                .get(name)
                .cloned()
                .ok_or_else(|| MyGraphError::Semantic(format!("variable `{name}` is not defined"))),
            Expr::Property(base, key) => {
                let b = self.eval(base, row)?;
                self.property(&b, key)
            }
            Expr::Index(base, idx) => {
                let (b, i) = (self.eval(base, row)?, self.eval(idx, row)?);
                match (&b, &i) {
                    (Val::Null, _) | (_, Val::Null) => Ok(Val::Null),
                    (Val::List(items), Val::Int(i)) => {
                        let len = items.len() as i64;
                        let idx = if *i < 0 { len + i } else { *i };
                        Ok(if (0..len).contains(&idx) { items[idx as usize].clone() } else { Val::Null })
                    }
                    (Val::Map(_) | Val::Node(_) | Val::Rel(_), Val::Str(k)) => self.property(&b, k),
                    _ => Err(type_err(format!("cannot index {} with {}", b.type_name(), i.type_name()))),
                }
            }
            Expr::List(items) => Ok(Val::List(items.iter().map(|i| self.eval(i, row)).collect::<Result<_>>()?)),
            Expr::Map(entries) => {
                Ok(Val::Map(entries.iter().map(|(k, v)| Ok((k.clone(), self.eval(v, row)?))).collect::<Result<_>>()?))
            }
            Expr::Unary(op, inner) => {
                let v = self.eval(inner, row)?;
                match (op, v) {
                    (UnaryOp::Not, v) => Ok(from_truth(truth(&v, "NOT")?.map(|b| !b))),
                    (_, Val::Null) => Ok(Val::Null),
                    (UnaryOp::Neg, Val::Int(i)) => {
                        Ok(Val::Int(i.checked_neg().ok_or_else(|| MyGraphError::Execution("integer overflow".into()))?))
                    }
                    (UnaryOp::Neg, Val::Float(f)) => Ok(Val::Float(-f)),
                    (UnaryOp::Pos, v @ (Val::Int(_) | Val::Float(_))) => Ok(v),
                    (_, v) => Err(type_err(format!("unary {op:?} expects a number, got {}", v.type_name()))),
                }
            }
            Expr::Binary(op, l, r) => self.binary(*op, l, r, row),
            Expr::IsNull { expr, negated } => {
                let is_null = matches!(self.eval(expr, row)?, Val::Null);
                Ok(Val::Bool(is_null != *negated))
            }
            Expr::HasLabels(expr, labels) => match self.eval(expr, row)? {
                Val::Null => Ok(Val::Null),
                Val::Node(id) => Ok(Val::Bool(
                    self.graph.node(id).is_some_and(|n| labels.iter().all(|l| self.graph.node_has_label(n, l))),
                )),
                other => Err(type_err(format!("label check expects a NODE, got {}", other.type_name()))),
            },
            Expr::FunctionCall { name, args, .. } => {
                if is_aggregate_function(name) {
                    return Err(MyGraphError::Semantic(format!(
                        "aggregate function `{name}` is only allowed in RETURN and WITH"
                    )));
                }
                let args = args.iter().map(|a| self.eval(a, row)).collect::<Result<Vec<_>>>()?;
                functions::call(self, name, args)
            }
            Expr::CountStar => Err(MyGraphError::Semantic("count(*) is only allowed in RETURN and WITH".into())),
            Expr::Case { operand, branches, default } => {
                let operand = operand.as_ref().map(|o| self.eval(o, row)).transpose()?;
                for (when, then) in branches {
                    let w = self.eval(when, row)?;
                    let hit = match &operand {
                        Some(o) => equals(o, &w) == Some(true),
                        None => w == Val::Bool(true),
                    };
                    if hit {
                        return self.eval(then, row);
                    }
                }
                default.as_ref().map_or(Ok(Val::Null), |d| self.eval(d, row))
            }
        }
    }

    pub fn property(&self, base: &Val, key: &str) -> Result<Val> {
        Ok(match base {
            Val::Null => Val::Null,
            Val::Node(id) => {
                self.graph.node(*id).and_then(|n| n.properties.get(key)).map_or(Val::Null, Val::from_value)
            }
            Val::Rel(id) => {
                self.graph.relationship(*id).and_then(|r| r.properties.get(key)).map_or(Val::Null, Val::from_value)
            }
            Val::Map(m) => m.get(key).cloned().unwrap_or(Val::Null),
            other => return Err(type_err(format!("cannot read property `{key}` of a {}", other.type_name()))),
        })
    }

    fn binary(&self, op: BinaryOp, l: &Expr, r: &Expr, row: &Row) -> Result<Val> {
        match op {
            BinaryOp::And => {
                let a = truth(&self.eval(l, row)?, "AND")?;
                if a == Some(false) {
                    return Ok(Val::Bool(false));
                }
                let b = truth(&self.eval(r, row)?, "AND")?;
                return Ok(match (a, b) {
                    (_, Some(false)) => Val::Bool(false),
                    (Some(true), Some(true)) => Val::Bool(true),
                    _ => Val::Null,
                });
            }
            BinaryOp::Or => {
                let a = truth(&self.eval(l, row)?, "OR")?;
                if a == Some(true) {
                    return Ok(Val::Bool(true));
                }
                let b = truth(&self.eval(r, row)?, "OR")?;
                return Ok(match (a, b) {
                    (_, Some(true)) => Val::Bool(true),
                    (Some(false), Some(false)) => Val::Bool(false),
                    _ => Val::Null,
                });
            }
            BinaryOp::Xor => {
                let a = truth(&self.eval(l, row)?, "XOR")?;
                let b = truth(&self.eval(r, row)?, "XOR")?;
                return Ok(match (a, b) {
                    (Some(a), Some(b)) => Val::Bool(a != b),
                    _ => Val::Null,
                });
            }
            _ => {}
        }
        let (a, b) = (self.eval(l, row)?, self.eval(r, row)?);
        Ok(match op {
            BinaryOp::Eq => from_truth(equals(&a, &b)),
            BinaryOp::Neq => from_truth(equals(&a, &b).map(|x| !x)),
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => match compare(&a, &b) {
                None => Val::Null,
                Some(o) => Val::Bool(match op {
                    BinaryOp::Lt => o == Ordering::Less,
                    BinaryOp::Le => o != Ordering::Greater,
                    BinaryOp::Gt => o == Ordering::Greater,
                    _ => o != Ordering::Less,
                }),
            },
            BinaryOp::In => match b {
                Val::Null => Val::Null,
                Val::List(items) => {
                    let mut result = Some(false);
                    for item in &items {
                        match equals(&a, item) {
                            Some(true) => return Ok(Val::Bool(true)),
                            None => result = None,
                            Some(false) => {}
                        }
                    }
                    from_truth(result)
                }
                other => return Err(type_err(format!("IN expects a LIST, got {}", other.type_name()))),
            },
            BinaryOp::StartsWith | BinaryOp::EndsWith | BinaryOp::Contains => match (&a, &b) {
                (Val::Str(s), Val::Str(t)) => Val::Bool(match op {
                    BinaryOp::StartsWith => s.starts_with(t.as_str()),
                    BinaryOp::EndsWith => s.ends_with(t.as_str()),
                    _ => s.contains(t.as_str()),
                }),
                _ => Val::Null,
            },
            _ => arithmetic(op, a, b)?,
        })
    }
}
