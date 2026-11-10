//! Static checks run before execution: variable scoping and clause rules.

use std::collections::HashSet;

use mygraph_core::{MyGraphError, Result};

use crate::ast::*;

pub fn check_query(q: &Query) -> Result<()> {
    let mut scope: HashSet<String> = HashSet::new();
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
                    for (r, _) in &p.steps {
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
            Clause::Set { items } => {
                for item in items {
                    match item {
                        SetItem::Property { variable, value, .. }
                        | SetItem::Replace { variable, value }
                        | SetItem::Merge { variable, value } => {
                            check_var(variable, &scope)?;
                            check_expr(value, &scope, false)?;
                        }
                        SetItem::Labels { variable, .. } => check_var(variable, &scope)?,
                    }
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
            }
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
