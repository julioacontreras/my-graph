//! Static checks run before execution: variable scoping and clause rules.

use std::collections::HashSet;

use mygraph_core::{MyGraphError, Result};

use crate::ast::*;

pub fn check_query(q: &Query) -> Result<()> {
    let mut first_columns: Option<Vec<String>> = None;
    for (i, part) in q.parts.iter().enumerate() {
        let columns = check_single(part)?;
        if q.parts.len() > 1 {
            if let Some(cols) = columns {
                match &first_columns {
                    Some(first) if *first != cols => return Err(union_columns_error(first, &cols)),
                    None if i == 0 => first_columns = Some(cols),
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn union_columns_error(first: &[String], other: &[String]) -> MyGraphError {
    MyGraphError::Semantic(format!(
        "all parts of a UNION must return the same columns in the same order: [{}] vs [{}]",
        first.join(", "),
        other.join(", ")
    ))
}

/// Checks one part of a query. Returns its column names when they are known
/// statically (a final RETURN without `*`).
fn check_single(q: &SingleQuery) -> Result<Option<Vec<String>>> {
    let mut scope: HashSet<String> = HashSet::new();
    let mut columns = None;
    for clause in &q.clauses {
        match clause {
            Clause::Match { patterns, filter, .. } => {
                for p in patterns {
                    for v in p.variables() {
                        scope.insert(v.to_string());
                    }
                }
                check_patterns(patterns, &scope)?;
                if let Some(f) = filter {
                    check_expr(f, &scope, false)?;
                }
            }
            Clause::Create { patterns } => {
                for p in patterns {
                    if p.name.is_some() || p.shortest.is_some() {
                        return Err(MyGraphError::Semantic(
                            "named paths and shortestPath are not supported in CREATE".into(),
                        ));
                    }
                    for (r, _) in &p.steps {
                        if r.length.is_some() {
                            return Err(MyGraphError::Semantic(
                                "variable-length relationships cannot be created".into(),
                            ));
                        }
                        if let Some(v) = &r.variable {
                            if scope.contains(v) {
                                return Err(MyGraphError::Semantic(format!(
                                    "variable `{v}` is already declared; CREATE needs a new relationship variable"
                                )));
                            }
                        }
                        if r.types.len() != 1 {
                            return Err(MyGraphError::Semantic(
                                "exactly one relationship type must be given in CREATE".into(),
                            ));
                        }
                        if r.direction == Direction::Both {
                            return Err(MyGraphError::Semantic("only directed relationships can be created".into()));
                        }
                    }
                    for v in p.variables() {
                        scope.insert(v.to_string());
                    }
                }
                check_patterns(patterns, &scope)?;
            }
            Clause::Merge { pattern, on_create, on_match } => {
                check_merge_pattern(pattern, &scope)?;
                check_patterns(std::slice::from_ref(pattern), &scope)?;
                for v in pattern.variables() {
                    scope.insert(v.to_string());
                }
                check_set_items(on_create, &scope)?;
                check_set_items(on_match, &scope)?;
            }
            Clause::Set { items } => check_set_items(items, &scope)?,
            Clause::Call { procedure, args, yields, filter } => {
                for a in args {
                    check_expr(a, &scope, false)?;
                }
                for v in call_columns(procedure, args.len(), yields.as_deref())? {
                    if !scope.insert(v.clone()) {
                        return Err(MyGraphError::Semantic(format!(
                            "variable `{v}` is already declared; rename it with `YIELD … AS`"
                        )));
                    }
                }
                if let Some(f) = filter {
                    check_expr(f, &scope, false)?;
                }
            }
            Clause::Remove { items } => {
                for item in items {
                    match item {
                        RemoveItem::Property { variable, .. } | RemoveItem::Labels { variable, .. } => {
                            check_var(variable, &scope)?
                        }
                    }
                }
            }
            Clause::Delete { exprs, .. } => {
                for e in exprs {
                    check_expr(e, &scope, false)?;
                }
            }
            Clause::Unwind { expr, variable } => {
                check_expr(expr, &scope, false)?;
                scope.insert(variable.clone());
            }
            Clause::With { projection, filter } => {
                scope = check_projection(projection, &scope)?;
                if let Some(f) = filter {
                    check_expr(f, &scope, false)?;
                }
            }
            Clause::Return(projection) => {
                check_projection(projection, &scope)?;
                if !projection.star {
                    columns = Some(projection.items.iter().map(|i| i.name.clone()).collect());
                }
            }
        }
    }
    Ok(columns)
}

/// The variables a `CALL` binds (also its columns when it runs on its own).
/// Checks that the procedure exists, its arity and the `YIELD` names.
pub(crate) fn call_columns(
    procedure: &str,
    nargs: usize,
    yields: Option<&[(String, Option<String>)]>,
) -> Result<Vec<String>> {
    let p = crate::executor::procedure(procedure)
        .ok_or_else(|| MyGraphError::Semantic(format!("there is no procedure `{procedure}`")))?;
    if nargs != p.arity {
        return Err(MyGraphError::Semantic(format!(
            "procedure `{}` expects {} argument(s), got {nargs}",
            p.name, p.arity
        )));
    }
    let Some(items) = yields else { return Ok(p.outputs.iter().map(|o| o.to_string()).collect()) };
    let mut out = Vec::new();
    for (name, alias) in items {
        if !p.outputs.contains(&name.as_str()) {
            return Err(MyGraphError::Semantic(format!(
                "procedure `{}` has no output `{name}`; it yields {}",
                p.name,
                p.outputs.join(", ")
            )));
        }
        let v = alias.clone().unwrap_or_else(|| name.clone());
        if out.contains(&v) {
            return Err(MyGraphError::Semantic(format!("`{v}` is yielded twice")));
        }
        out.push(v);
    }
    Ok(out)
}

fn check_set_items(items: &[SetItem], scope: &HashSet<String>) -> Result<()> {
    for item in items {
        match item {
            SetItem::Property { variable, value, .. }
            | SetItem::Replace { variable, value }
            | SetItem::Merge { variable, value } => {
                check_var(variable, scope)?;
                check_expr(value, scope, false)?;
            }
            SetItem::Labels { variable, .. } => check_var(variable, scope)?,
        }
    }
    Ok(())
}

fn check_merge_pattern(p: &PathPattern, scope: &HashSet<String>) -> Result<()> {
    let err = |m: String| Err(MyGraphError::Semantic(m));
    if p.name.is_some() || p.shortest.is_some() {
        return err("named paths and shortestPath are not supported in MERGE".into());
    }
    let nodes = std::iter::once(&p.start).chain(p.steps.iter().map(|(_, n)| n));
    for n in nodes {
        if let Some(v) = n.variable.as_ref().filter(|v| scope.contains(*v)) {
            if !n.labels.is_empty() || n.properties.is_some() {
                return err(format!(
                    "variable `{v}` is already declared; it cannot be merged again with labels or properties"
                ));
            }
        }
    }
    if p.steps.is_empty() && p.start.variable.as_ref().is_some_and(|v| scope.contains(v)) {
        return err("MERGE needs a new node or relationship; every variable in the pattern is already bound".into());
    }
    for (r, _) in &p.steps {
        if r.length.is_some() {
            return err("variable-length relationships cannot be merged".into());
        }
        if let Some(v) = r.variable.as_ref().filter(|v| scope.contains(*v)) {
            return err(format!("variable `{v}` is already declared; MERGE needs a new relationship variable"));
        }
        if r.types.len() != 1 {
            return err("exactly one relationship type must be given in MERGE".into());
        }
    }
    Ok(())
}

fn check_var(v: &str, scope: &HashSet<String>) -> Result<()> {
    if scope.contains(v) {
        Ok(())
    } else {
        Err(MyGraphError::Semantic(format!("variable `{v}` is not defined")))
    }
}

fn check_patterns(patterns: &[PathPattern], scope: &HashSet<String>) -> Result<()> {
    for p in patterns {
        let props = std::iter::once(&p.start.properties)
            .chain(p.steps.iter().flat_map(|(r, n)| [&r.properties, &n.properties]));
        for e in props.flatten() {
            check_expr(e, scope, false)?;
        }
    }
    Ok(())
}

/// Returns the scope visible after the projection.
fn check_projection(p: &Projection, scope: &HashSet<String>) -> Result<HashSet<String>> {
    let mut out: HashSet<String> = if p.star { scope.clone() } else { HashSet::new() };
    let mut seen = HashSet::new();
    for item in &p.items {
        check_expr(&item.expr, scope, true)?;
        if !seen.insert(item.name.clone()) {
            return Err(MyGraphError::Semantic(format!("duplicate column name `{}`", item.name)));
        }
        out.insert(item.name.clone());
    }
    if p.star && scope.is_empty() {
        return Err(MyGraphError::Semantic("`*` is not allowed when there are no variables in scope".into()));
    }
    let sort_scope: HashSet<String> = scope.union(&out).cloned().collect();
    for s in &p.order_by {
        check_expr(&s.expr, &sort_scope, true)?;
    }
    for e in p.skip.iter().chain(p.limit.iter()) {
        check_expr(e, &HashSet::new(), false)?;
    }
    Ok(out)
}

fn check_expr(e: &Expr, scope: &HashSet<String>, aggregates_allowed: bool) -> Result<()> {
    let mut err = None;
    e.walk(&mut |sub| {
        if err.is_some() {
            return;
        }
        match sub {
            Expr::Variable(v) if !scope.contains(v) => {
                err = Some(MyGraphError::Semantic(format!("variable `{v}` is not defined")));
            }
            Expr::CountStar if !aggregates_allowed => {
                err = Some(MyGraphError::Semantic("aggregate functions are only allowed in RETURN and WITH".into()));
            }
            Expr::FunctionCall { name, .. } if !aggregates_allowed && is_aggregate_function(name) => {
                err = Some(MyGraphError::Semantic("aggregate functions are only allowed in RETURN and WITH".into()));
            }
            _ => {}
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
