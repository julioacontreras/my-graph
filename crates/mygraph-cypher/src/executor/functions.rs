//! Scalar functions.

use mygraph_core::{now_millis, MyGraphError, Result};

use super::eval::{type_err, Env, Val};

fn arity(name: &str, args: &[Val], min: usize, max: usize) -> Result<()> {
    if args.len() < min || args.len() > max {
        let expected = if min == max { min.to_string() } else { format!("{min}..{max}") };
        return Err(MyGraphError::Semantic(format!(
            "function `{name}` expects {expected} argument(s), got {}",
            args.len()
        )));
    }
    Ok(())
}

fn expect_str<'a>(name: &str, v: &'a Val) -> Result<&'a str> {
    match v {
        Val::Str(s) => Ok(s),
        other => Err(type_err(format!("{name}() expects a STRING, got {}", other.type_name()))),
    }
}

fn expect_int(name: &str, v: &Val) -> Result<i64> {
    match v {
        Val::Int(i) => Ok(*i),
        other => Err(type_err(format!("{name}() expects an INTEGER, got {}", other.type_name()))),
    }
}

fn num(name: &str, v: &Val) -> Result<f64> {
    match v {
        Val::Int(i) => Ok(*i as f64),
        Val::Float(f) => Ok(*f),
        other => Err(type_err(format!("{name}() expects a number, got {}", other.type_name()))),
    }
}

pub(crate) fn call(env: &Env<'_>, name: &str, args: Vec<Val>) -> Result<Val> {
    let lname = name.to_ascii_lowercase();
    let n = lname.as_str();
    // Functions with special null handling.
    match n {
        "coalesce" => return Ok(args.into_iter().find(|a| *a != Val::Null).unwrap_or(Val::Null)),
        "timestamp" => {
            arity(n, &args, 0, 0)?;
            return Ok(Val::Int(now_millis() as i64));
        }
        "range" => {
            arity(n, &args, 2, 3)?;
            let (start, end) = (expect_int(n, &args[0])?, expect_int(n, &args[1])?);
            let step = args.get(2).map(|s| expect_int(n, s)).transpose()?.unwrap_or(1);
            if step == 0 {
                return Err(MyGraphError::Execution("range() step cannot be zero".into()));
            }
            let mut out = Vec::new();
            let mut i = start;
            while (step > 0 && i <= end) || (step < 0 && i >= end) {
                out.push(Val::Int(i));
                if out.len() > 10_000_000 {
                    return Err(MyGraphError::Execution("range() is too large".into()));
                }
                i = match i.checked_add(step) {
                    Some(v) => v,
                    None => break,
                };
            }
            return Ok(Val::List(out));
        }
        _ => {}
    }
    // Remaining functions return null when their first argument is null.
    if args.first() == Some(&Val::Null) {
        return match n {
            "id" | "labels" | "type" | "keys" | "properties" | "size" | "length" | "tolower" | "toupper" | "trim"
            | "ltrim" | "rtrim" | "tostring" | "tointeger" | "tofloat" | "toboolean" | "abs" | "ceil" | "floor"
            | "round" | "sqrt" | "sign" | "head" | "last" | "tail" | "reverse" | "replace" | "substring" | "split"
            | "startnode" | "endnode" | "left" | "right" | "nodes" | "relationships" | "rels" => Ok(Val::Null),
            _ => Err(MyGraphError::Semantic(format!("unknown function `{name}`"))),
        };
    }
    let g = env.graph;
    let one = |args: &[Val]| arity(n, args, 1, 1);
    Ok(match n {
        "id" => {
            one(&args)?;
            match &args[0] {
                Val::Node(id) | Val::Rel(id) => Val::Int(*id as i64),
                other => {
                    return Err(type_err(format!("id() expects a NODE or RELATIONSHIP, got {}", other.type_name())))
                }
            }
        }
        "labels" => {
            one(&args)?;
            match &args[0] {
                Val::Node(id) => Val::List(
                    g.node(*id)
                        .map(|node| g.node_label_names(node).into_iter().map(Val::Str).collect())
                        .unwrap_or_default(),
                ),
                other => return Err(type_err(format!("labels() expects a NODE, got {}", other.type_name()))),
            }
        }
        "type" => {
            one(&args)?;
            match &args[0] {
                Val::Rel(id) => {
                    g.relationship(*id).map_or(Val::Null, |r| Val::Str(g.rel_type_name(r.rel_type).to_string()))
                }
                other => return Err(type_err(format!("type() expects a RELATIONSHIP, got {}", other.type_name()))),
            }
        }
        "startnode" | "endnode" => {
            one(&args)?;
            match &args[0] {
                Val::Rel(id) => g
                    .relationship(*id)
                    .map_or(Val::Null, |r| Val::Node(if n == "startnode" { r.start_node } else { r.end_node })),
                other => return Err(type_err(format!("{n}() expects a RELATIONSHIP, got {}", other.type_name()))),
            }
        }
        "keys" | "properties" => {
            one(&args)?;
            let props: Vec<(String, Val)> = match &args[0] {
                Val::Node(id) => g
                    .node(*id)
                    .map(|x| x.properties.iter().map(|(k, v)| (k.to_string(), Val::from_value(v))).collect())
                    .unwrap_or_default(),
                Val::Rel(id) => g
                    .relationship(*id)
                    .map(|x| x.properties.iter().map(|(k, v)| (k.to_string(), Val::from_value(v))).collect())
                    .unwrap_or_default(),
                Val::Map(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                other => {
                    return Err(type_err(format!(
                        "{n}() expects a NODE, RELATIONSHIP or MAP, got {}",
                        other.type_name()
                    )))
                }
            };
            if n == "keys" {
                Val::List(props.into_iter().map(|(k, _)| Val::Str(k)).collect())
            } else {
                Val::Map(props.into_iter().collect())
            }
        }
        "nodes" | "relationships" | "rels" => {
            one(&args)?;
            match &args[0] {
                Val::Path { nodes, .. } if n == "nodes" => Val::List(nodes.iter().map(|x| Val::Node(*x)).collect()),
                Val::Path { rels, .. } => Val::List(rels.iter().map(|x| Val::Rel(*x)).collect()),
                other => return Err(type_err(format!("{n}() expects a PATH, got {}", other.type_name()))),
            }
        }
        "size" | "length" => {
            one(&args)?;
            match &args[0] {
                Val::Path { rels, .. } if n == "length" => Val::Int(rels.len() as i64),
                Val::List(l) => Val::Int(l.len() as i64),
                Val::Str(s) => Val::Int(s.chars().count() as i64),
                other => {
                    let expected = if n == "length" { "a PATH, LIST or STRING" } else { "a LIST or STRING" };
                    return Err(type_err(format!("{n}() expects {expected}, got {}", other.type_name())));
                }
            }
        }
        "tolower" => {
            one(&args)?;
            Val::Str(expect_str(n, &args[0])?.to_lowercase())
        }
        "toupper" => {
            one(&args)?;
            Val::Str(expect_str(n, &args[0])?.to_uppercase())
        }
        "trim" => {
            one(&args)?;
            Val::Str(expect_str(n, &args[0])?.trim().to_string())
        }
        "ltrim" => {
            one(&args)?;
            Val::Str(expect_str(n, &args[0])?.trim_start().to_string())
        }
        "rtrim" => {
            one(&args)?;
            Val::Str(expect_str(n, &args[0])?.trim_end().to_string())
        }
        "replace" => {
            arity(n, &args, 3, 3)?;
            if args[1] == Val::Null || args[2] == Val::Null {
                return Ok(Val::Null);
            }
            Val::Str(expect_str(n, &args[0])?.replace(expect_str(n, &args[1])?, expect_str(n, &args[2])?))
        }
        "substring" => {
            arity(n, &args, 2, 3)?;
            let chars: Vec<char> = expect_str(n, &args[0])?.chars().collect();
            let start = expect_int(n, &args[1])?.max(0) as usize;
            let len = match args.get(2) {
                Some(l) => expect_int(n, l)?.max(0) as usize,
                None => usize::MAX,
            };
            Val::Str(chars.iter().skip(start).take(len).collect())
        }
        "left" | "right" => {
            arity(n, &args, 2, 2)?;
            let chars: Vec<char> = expect_str(n, &args[0])?.chars().collect();
            let k = (expect_int(n, &args[1])?.max(0) as usize).min(chars.len());
            Val::Str(if n == "left" { chars[..k].iter().collect() } else { chars[chars.len() - k..].iter().collect() })
        }
        "split" => {
            arity(n, &args, 2, 2)?;
            let (s, d) = (expect_str(n, &args[0])?, expect_str(n, &args[1])?);
            Val::List(s.split(d).map(|p| Val::Str(p.to_string())).collect())
        }
        "reverse" => {
            one(&args)?;
            match &args[0] {
                Val::Str(s) => Val::Str(s.chars().rev().collect()),
                Val::List(l) => Val::List(l.iter().rev().cloned().collect()),
                other => {
                    return Err(type_err(format!("reverse() expects a LIST or STRING, got {}", other.type_name())))
                }
            }
        }
        "tostring" => {
            one(&args)?;
            match &args[0] {
                v @ (Val::Str(_) | Val::Int(_) | Val::Float(_) | Val::Bool(_)) => Val::Str(v.to_display_string()),
                other => return Err(type_err(format!("toString() cannot convert {}", other.type_name()))),
            }
        }
        "tointeger" => {
            one(&args)?;
            match &args[0] {
                Val::Int(i) => Val::Int(*i),
                Val::Float(f) if f.is_finite() && f.abs() < 9.2e18 => Val::Int(f.trunc() as i64),
                Val::Float(_) => Val::Null,
                Val::Bool(b) => Val::Int(*b as i64),
                Val::Str(s) => s
                    .trim()
                    .parse::<i64>()
                    .ok()
                    .or_else(|| s.trim().parse::<f64>().ok().filter(|f| f.is_finite()).map(|f| f.trunc() as i64))
                    .map_or(Val::Null, Val::Int),
                other => return Err(type_err(format!("toInteger() cannot convert {}", other.type_name()))),
            }
        }
        "tofloat" => {
            one(&args)?;
            match &args[0] {
                Val::Int(i) => Val::Float(*i as f64),
                Val::Float(f) => Val::Float(*f),
                Val::Str(s) => s.trim().parse::<f64>().map_or(Val::Null, Val::Float),
                other => return Err(type_err(format!("toFloat() cannot convert {}", other.type_name()))),
            }
        }
        "toboolean" => {
            one(&args)?;
            match &args[0] {
                Val::Bool(b) => Val::Bool(*b),
                Val::Str(s) if s.trim().eq_ignore_ascii_case("true") => Val::Bool(true),
                Val::Str(s) if s.trim().eq_ignore_ascii_case("false") => Val::Bool(false),
                Val::Str(_) => Val::Null,
                Val::Int(i) => Val::Bool(*i != 0),
                other => return Err(type_err(format!("toBoolean() cannot convert {}", other.type_name()))),
            }
        }
        "abs" => {
            one(&args)?;
            match &args[0] {
                Val::Int(i) => {
                    Val::Int(i.checked_abs().ok_or_else(|| MyGraphError::Execution("integer overflow".into()))?)
                }
                v => Val::Float(num(n, v)?.abs()),
            }
        }
        "ceil" => {
            one(&args)?;
            Val::Float(num(n, &args[0])?.ceil())
        }
        "floor" => {
            one(&args)?;
            Val::Float(num(n, &args[0])?.floor())
        }
        "round" => {
            one(&args)?;
            Val::Float(num(n, &args[0])?.round())
        }
        "sqrt" => {
            one(&args)?;
            Val::Float(num(n, &args[0])?.sqrt())
        }
        "sign" => {
            one(&args)?;
            let x = num(n, &args[0])?;
            Val::Int(if x > 0.0 {
                1
            } else if x < 0.0 {
                -1
            } else {
                0
            })
        }
        "head" | "last" | "tail" => {
            one(&args)?;
            let Val::List(l) = &args[0] else {
                return Err(type_err(format!("{n}() expects a LIST, got {}", args[0].type_name())));
            };
            match n {
                "head" => l.first().cloned().unwrap_or(Val::Null),
                "last" => l.last().cloned().unwrap_or(Val::Null),
                _ => Val::List(l.iter().skip(1).cloned().collect()),
            }
        }
        _ => return Err(MyGraphError::Semantic(format!("unknown function `{name}`"))),
    })
}
