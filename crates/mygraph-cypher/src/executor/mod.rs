//! Clause-by-clause query executor.
//!
//! Clauses run in order over a materialized row set. MATCH is planned by
//! [`matcher::Matcher`]: it anchors each path on its most selective node
//! (bound variable, `id()`, property index, smallest label) and applies
//! `WHERE` conjuncts as soon as their variables are bound.

mod eval;
mod functions;
mod matcher;
mod paths;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::AddAssign;

use mygraph_core::{Graph, MyGraphError, NodeId, PropertyMap, Result, Value, WriteContext};

use crate::ast::*;
use eval::{arithmetic, group_key, order_cmp, type_err, Env, Row, Val};

/// Counters describing what a query changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryStats {
    pub nodes_created: u64,
    pub nodes_deleted: u64,
    pub relationships_created: u64,
    pub relationships_deleted: u64,
    pub properties_set: u64,
    pub labels_added: u64,
    pub labels_removed: u64,
}

impl QueryStats {
    pub fn contains_updates(&self) -> bool {
        *self != QueryStats::default()
    }
}

impl AddAssign<&QueryStats> for QueryStats {
    fn add_assign(&mut self, o: &QueryStats) {
        self.nodes_created += o.nodes_created;
        self.nodes_deleted += o.nodes_deleted;
        self.relationships_created += o.relationships_created;
        self.relationships_deleted += o.relationships_deleted;
        self.properties_set += o.properties_set;
        self.labels_added += o.labels_added;
        self.labels_removed += o.labels_removed;
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExecResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub stats: QueryStats,
}

/// Runs a query against one schema graph. All changes go through `ctx`, so the
/// caller can commit or roll them back.
pub fn execute(query: &Query, ctx: &mut WriteContext<'_>, params: &BTreeMap<String, Value>) -> Result<ExecResult> {
    run(query, &mut Target::Write(ctx), params)
}

/// Runs a read-only query (see [`Query::is_read_only`]) against a shared
/// graph, so several can run at once. A writing query is an error.
pub fn execute_read(query: &Query, graph: &Graph, params: &BTreeMap<String, Value>) -> Result<ExecResult> {
    run(query, &mut Target::Read(graph), params)
}

fn run(query: &Query, target: &mut Target<'_, '_>, params: &BTreeMap<String, Value>) -> Result<ExecResult> {
    crate::semantic::check_query(query)?;
    let params: BTreeMap<String, Val> = params.iter().map(|(k, v)| (k.clone(), Val::from_value(v))).collect();
    let mut exec = Executor { params: &params, stats: QueryStats::default() };
    exec.run(query, target)
}

/// What a query runs against: a shared graph it may only read, or a write
/// context that journals its changes.
enum Target<'a, 'w> {
    Read(&'a Graph),
    Write(&'a mut WriteContext<'w>),
}

impl<'w> Target<'_, 'w> {
    fn graph(&self) -> &Graph {
        match self {
            Target::Read(g) => g,
            Target::Write(ctx) => ctx.graph(),
        }
    }

    fn writer(&mut self) -> Result<&mut WriteContext<'w>> {
        match self {
            Target::Read(_) => Err(MyGraphError::Execution("this query writes, but runs in read-only mode".into())),
            Target::Write(ctx) => Ok(ctx),
        }
    }
}

struct Executor<'p> {
    params: &'p BTreeMap<String, Val>,
    stats: QueryStats,
}

fn bind(row: &mut Row, var: &Option<String>, val: Val) {
    if let Some(v) = var {
        row.insert(v.clone(), val);
    }
}

fn filter_rows(env: &Env<'_>, rows: Vec<Row>, filter: &Expr) -> Result<Vec<Row>> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        match env.eval(filter, &row)? {
            Val::Bool(true) => out.push(row),
            Val::Bool(false) | Val::Null => {}
            other => return Err(type_err(format!("WHERE expects a BOOLEAN, got {}", other.type_name()))),
        }
    }
    Ok(out)
}

impl Executor<'_> {
    fn env<'g>(&self, graph: &'g Graph) -> Env<'g>
    where
        Self: 'g,
    {
        Env { graph, params: self.params }
    }

    fn run(&mut self, q: &Query, target: &mut Target<'_, '_>) -> Result<ExecResult> {
        let mut rows: Vec<Row> = vec![Row::new()];
        let mut out = ExecResult::default();
        for clause in &q.clauses {
            rows = match clause {
                Clause::Match { optional, patterns, filter } => {
                    self.do_match(target.graph(), rows, *optional, patterns, filter.as_ref())?
                }
                Clause::Create { patterns } => {
                    let ctx = target.writer()?;
                    for row in rows.iter_mut() {
                        for p in patterns {
                            self.create_path(ctx, row, p)?;
                        }
                    }
                    rows
                }
                Clause::Set { items } => {
                    let ctx = target.writer()?;
                    for row in &rows {
                        self.do_set(ctx, row, items)?;
                    }
                    rows
                }
                Clause::Remove { items } => {
                    let ctx = target.writer()?;
                    for row in &rows {
                        self.do_remove(ctx, row, items)?;
                    }
                    rows
                }
                Clause::Delete { detach, exprs } => {
                    self.do_delete(target.writer()?, &rows, *detach, exprs)?;
                    rows
                }
                Clause::Unwind { expr, variable } => {
                    let env = self.env(target.graph());
                    let mut out = Vec::new();
                    for row in rows {
                        match env.eval(expr, &row)? {
                            Val::Null => {}
                            Val::List(items) => {
                                for item in items {
                                    let mut r = row.clone();
                                    r.insert(variable.clone(), item);
                                    out.push(r);
                                }
                            }
                            other => {
                                let mut r = row;
                                r.insert(variable.clone(), other);
                                out.push(r);
                            }
                        }
                    }
                    out
                }
                Clause::With { projection, filter } => {
                    let (_, projected) = self.project(target.graph(), rows, projection)?;
                    match filter {
                        Some(f) => filter_rows(&self.env(target.graph()), projected, f)?,
                        None => projected,
                    }
                }
                Clause::Return(projection) => {
                    let (columns, projected) = self.project(target.graph(), rows, projection)?;
                    let g = target.graph();
                    out.rows = projected
                        .iter()
                        .map(|row| columns.iter().map(|c| row[c].materialize(g)).collect::<Result<Vec<_>>>())
                        .collect::<Result<_>>()?;
                    out.columns = columns;
                    Vec::new()
                }
            };
        }
        out.stats = self.stats.clone();
        Ok(out)
    }

    // ---- MATCH ----

    fn do_match(
        &self,
        graph: &Graph,
        rows: Vec<Row>,
        optional: bool,
        patterns: &[PathPattern],
        filter: Option<&Expr>,
    ) -> Result<Vec<Row>> {
        let Some(sample) = rows.first() else { return Ok(rows) };
        let env = self.env(graph);
        let matcher = matcher::Matcher::new(&env, patterns, filter, sample);
        let mut out = Vec::new();
        for row in rows {
            let matches = matcher.run(row.clone())?;
            if matches.is_empty() && optional {
                let mut r = row;
                for p in patterns {
                    for v in p.variables() {
                        r.entry(v.to_string()).or_insert(Val::Null);
                    }
                }
                out.push(r);
            } else {
                out.extend(matches);
            }
        }
        Ok(out)
    }

    // ---- CREATE ----

    fn eval_properties(&self, graph: &Graph, expr: Option<&Expr>, row: &Row) -> Result<PropertyMap> {
        let Some(e) = expr else { return Ok(PropertyMap::new()) };
        match self.env(graph).eval(e, row)? {
            Val::Map(m) => {
                m.into_iter().filter(|(_, v)| *v != Val::Null).map(|(k, v)| Ok((k, v.into_property()?))).collect()
            }
            other => Err(type_err(format!("properties must be a MAP, got {}", other.type_name()))),
        }
    }

    fn create_node(&mut self, ctx: &mut WriteContext<'_>, row: &mut Row, np: &NodePattern) -> Result<NodeId> {
        if let Some(v) = &np.variable {
            if let Some(bound) = row.get(v) {
                if !np.labels.is_empty() || np.properties.is_some() {
                    return Err(MyGraphError::Semantic(format!(
                        "variable `{v}` is already declared; it cannot be created again with labels or properties"
                    )));
                }
                return match bound {
                    Val::Node(id) => Ok(*id),
                    other => Err(type_err(format!("variable `{v}` is a {}, expected a NODE", other.type_name()))),
                };
            }
        }
        let props = self.eval_properties(ctx.graph(), np.properties.as_ref(), row)?;
        let mut labels = np.labels.clone();
        labels.dedup();
        self.stats.labels_added += labels.iter().collect::<HashSet<_>>().len() as u64;
        self.stats.properties_set += props.len() as u64;
        let id = ctx.create_node(labels, props)?;
        self.stats.nodes_created += 1;
        bind(row, &np.variable, Val::Node(id));
        Ok(id)
    }

    fn create_path(&mut self, ctx: &mut WriteContext<'_>, row: &mut Row, p: &PathPattern) -> Result<()> {
        let mut prev = self.create_node(ctx, row, &p.start)?;
        for (rp, np) in &p.steps {
            let next = self.create_node(ctx, row, np)?;
            let props = self.eval_properties(ctx.graph(), rp.properties.as_ref(), row)?;
            let (start, end) = match rp.direction {
                Direction::Outgoing => (prev, next),
                Direction::Incoming => (next, prev),
                Direction::Both => {
                    return Err(MyGraphError::Semantic("only directed relationships can be created".into()))
                }
            };
            let rel_type = rp.types.first().ok_or_else(|| {
                MyGraphError::Semantic("exactly one relationship type must be given in CREATE".into())
            })?;
            self.stats.properties_set += props.len() as u64;
            let id = ctx.create_relationship(start, end, rel_type, props)?;
            self.stats.relationships_created += 1;
            bind(row, &rp.variable, Val::Rel(id));
            prev = next;
        }
        Ok(())
    }

    // ---- SET / REMOVE ----

    fn set_property(&mut self, ctx: &mut WriteContext<'_>, target: &Val, key: &str, value: Value) -> Result<()> {
        match target {
            Val::Null => return Ok(()),
            Val::Node(id) => ctx.set_node_property(*id, key, value)?,
            Val::Rel(id) => ctx.set_relationship_property(*id, key, value)?,
            other => return Err(type_err(format!("cannot set property `{key}` on a {}", other.type_name()))),
        }
        self.stats.properties_set += 1;
        Ok(())
    }

    fn lookup(row: &Row, var: &str) -> Result<Val> {
        row.get(var).cloned().ok_or_else(|| MyGraphError::Semantic(format!("variable `{var}` is not defined")))
    }

    fn do_set(&mut self, ctx: &mut WriteContext<'_>, row: &Row, items: &[SetItem]) -> Result<()> {
        for item in items {
            match item {
                SetItem::Property { variable, key, value } => {
                    let target = Self::lookup(row, variable)?;
                    let v = self.env(ctx.graph()).eval(value, row)?.into_property()?;
                    self.set_property(ctx, &target, key, v)?;
                }
                SetItem::Replace { variable, value } | SetItem::Merge { variable, value } => {
                    let target = Self::lookup(row, variable)?;
                    if target == Val::Null {
                        continue;
                    }
                    let env = self.env(ctx.graph());
                    let new_props: BTreeMap<String, Val> = match env.eval(value, row)? {
                        Val::Map(m) => m,
                        v @ (Val::Node(_) | Val::Rel(_)) => match functions::call(&env, "properties", vec![v])? {
                            Val::Map(m) => m,
                            _ => unreachable!("properties() returns a map"),
                        },
                        other => {
                            return Err(type_err(format!(
                                "SET {variable} = ... expects a MAP, got {}",
                                other.type_name()
                            )))
                        }
                    };
                    if matches!(item, SetItem::Replace { .. }) {
                        let existing: Vec<String> = match target {
                            Val::Node(id) => env.graph.node(id).map(|n| n.properties.keys().cloned().collect()),
                            Val::Rel(id) => env.graph.relationship(id).map(|r| r.properties.keys().cloned().collect()),
                            _ => None,
                        }
                        .unwrap_or_default();
                        for k in existing.into_iter().filter(|k| !new_props.contains_key(k)) {
                            self.set_property(ctx, &target, &k, Value::Null)?;
                        }
                    }
                    for (k, v) in new_props {
                        self.set_property(ctx, &target, &k, v.into_property()?)?;
                    }
                }
                SetItem::Labels { variable, labels } => match Self::lookup(row, variable)? {
                    Val::Null => {}
                    Val::Node(id) => {
                        for l in labels {
                            if ctx.add_label(id, l)? {
                                self.stats.labels_added += 1;
                            }
                        }
                    }
                    other => return Err(type_err(format!("cannot set labels on a {}", other.type_name()))),
                },
            }
        }
        Ok(())
    }

    fn do_remove(&mut self, ctx: &mut WriteContext<'_>, row: &Row, items: &[RemoveItem]) -> Result<()> {
        for item in items {
            match item {
                RemoveItem::Property { variable, key } => {
                    let target = Self::lookup(row, variable)?;
                    let exists = match target {
                        Val::Node(id) => ctx.graph().node(id).is_some_and(|n| n.properties.contains_key(key)),
                        Val::Rel(id) => ctx.graph().relationship(id).is_some_and(|r| r.properties.contains_key(key)),
                        _ => false,
                    };
                    if exists {
                        self.set_property(ctx, &target, key, Value::Null)?;
                    } else if !matches!(target, Val::Null | Val::Node(_) | Val::Rel(_)) {
                        return Err(type_err(format!("cannot remove property `{key}` from a {}", target.type_name())));
                    }
                }
                RemoveItem::Labels { variable, labels } => match Self::lookup(row, variable)? {
                    Val::Null => {}
                    Val::Node(id) => {
                        for l in labels {
                            if ctx.remove_label(id, l)? {
                                self.stats.labels_removed += 1;
                            }
                        }
                    }
                    other => return Err(type_err(format!("cannot remove labels from a {}", other.type_name()))),
                },
            }
        }
        Ok(())
    }

    // ---- DELETE ----

    fn do_delete(&mut self, ctx: &mut WriteContext<'_>, rows: &[Row], detach: bool, exprs: &[Expr]) -> Result<()> {
        let mut nodes = BTreeSet::new();
        let mut rels = BTreeSet::new();
        {
            let env = self.env(ctx.graph());
            for row in rows {
                for e in exprs {
                    match env.eval(e, row)? {
                        Val::Null => {}
                        Val::Node(id) => {
                            nodes.insert(id);
                        }
                        Val::Rel(id) => {
                            rels.insert(id);
                        }
                        other => {
                            return Err(type_err(format!(
                                "DELETE expects a NODE or RELATIONSHIP, got {}",
                                other.type_name()
                            )))
                        }
                    }
                }
            }
            if detach {
                for n in &nodes {
                    if let Some(node) = env.graph.node(*n) {
                        rels.extend(node.outgoing().iter().map(|a| a.rel));
                        rels.extend(node.incoming().iter().map(|a| a.rel));
                    }
                }
            }
        }
        for r in rels {
            if ctx.graph().relationship(r).is_some() {
                ctx.delete_relationship(r)?;
                self.stats.relationships_deleted += 1;
            }
        }
        for n in nodes {
            if ctx.graph().node(n).is_some() {
                ctx.delete_node(n)?;
                self.stats.nodes_deleted += 1;
            }
        }
        Ok(())
    }

    // ---- RETURN / WITH ----

    fn project(&self, graph: &Graph, rows: Vec<Row>, p: &Projection) -> Result<(Vec<String>, Vec<Row>)> {
        let env = self.env(graph);
        let mut items: Vec<(String, Expr)> = Vec::new();
        if p.star {
            let mut names: Vec<String> =
                rows.first().map(|r| r.keys().filter(|k| !k.starts_with(' ')).cloned().collect()).unwrap_or_default();
            names.sort();
            items.extend(names.into_iter().map(|n| (n.clone(), Expr::Variable(n))));
        }
        items.extend(p.items.iter().map(|i| (i.name.clone(), i.expr.clone())));
        let columns: Vec<String> = items.iter().map(|(n, _)| n.clone()).collect();

        let mut aggs: Vec<AggSpec> = Vec::new();
        let rewritten: Vec<(String, Expr, bool)> = items
            .iter()
            .map(|(n, e)| {
                let before = aggs.len();
                let e2 = extract_aggregates(e, &mut aggs)?;
                Ok((n.clone(), e2, aggs.len() > before))
            })
            .collect::<Result<_>>()?;

        // (projected row, scope row for ORDER BY)
        let mut produced: Vec<(Row, Row)> = Vec::new();
        if aggs.is_empty() {
            for row in rows {
                let mut pr = Row::new();
                for (n, e, _) in &rewritten {
                    pr.insert(n.clone(), env.eval(e, &row)?);
                }
                produced.push((pr, row));
            }
        } else {
            let mut groups: Vec<(Vec<Val>, Vec<Accumulator>, Row)> = Vec::new();
            let mut index: HashMap<String, usize> = HashMap::new();
            for row in rows {
                let key: Vec<Val> =
                    rewritten.iter().filter(|x| !x.2).map(|(_, e, _)| env.eval(e, &row)).collect::<Result<_>>()?;
                let k = group_key(&key.iter().collect::<Vec<_>>());
                let gi = match index.get(&k) {
                    Some(&gi) => gi,
                    None => {
                        groups.push((key, aggs.iter().map(Accumulator::new).collect(), row.clone()));
                        index.insert(k, groups.len() - 1);
                        groups.len() - 1
                    }
                };
                for (acc, spec) in groups[gi].1.iter_mut().zip(&aggs) {
                    let v = spec.arg.as_ref().map(|a| env.eval(a, &row)).transpose()?;
                    acc.update(v)?;
                }
            }
            if groups.is_empty() && rewritten.iter().all(|x| x.2) {
                groups.push((Vec::new(), aggs.iter().map(Accumulator::new).collect(), Row::new()));
            }
            for (key, accs, mut scope) in groups {
                for (i, acc) in accs.into_iter().enumerate() {
                    scope.insert(agg_var(i), acc.finish());
                }
                let mut pr = Row::new();
                let mut keys = key.into_iter();
                for (n, e, has_agg) in &rewritten {
                    let v = if *has_agg { env.eval(e, &scope)? } else { keys.next().expect("one key per item") };
                    pr.insert(n.clone(), v);
                }
                produced.push((pr, scope));
            }
        }

        if p.distinct {
            let mut seen = HashSet::new();
            produced.retain(|(pr, _)| seen.insert(group_key(&columns.iter().map(|c| &pr[c]).collect::<Vec<_>>())));
        }

        if !p.order_by.is_empty() {
            let mut keyed: Vec<(Vec<Val>, (Row, Row))> = Vec::with_capacity(produced.len());
            for (pr, scope) in produced {
                let mut s = scope.clone();
                s.extend(pr.iter().map(|(k, v)| (k.clone(), v.clone())));
                let mut keys = Vec::with_capacity(p.order_by.len());
                for o in &p.order_by {
                    // ORDER BY an expression that is also a projected item (e.g. an aggregate)
                    let projected = items.iter().position(|(_, e)| *e == o.expr).map(|i| pr[&columns[i]].clone());
                    keys.push(match projected {
                        Some(v) => v,
                        None => env.eval(&o.expr, &s)?,
                    });
                }
                keyed.push((keys, (pr, scope)));
            }
            keyed.sort_by(|a, b| {
                for (i, o) in p.order_by.iter().enumerate() {
                    let c = order_cmp(&a.0[i], &b.0[i]);
                    let c = if o.descending { c.reverse() } else { c };
                    if c.is_ne() {
                        return c;
                    }
                }
                std::cmp::Ordering::Equal
            });
            produced = keyed.into_iter().map(|(_, x)| x).collect();
        }

        let count = |e: &Option<Expr>, what: &str| -> Result<Option<usize>> {
            match e {
                None => Ok(None),
                Some(e) => match env.eval(e, &Row::new())? {
                    Val::Int(i) if i >= 0 => Ok(Some(i as usize)),
                    other => Err(type_err(format!("{what} expects a non-negative INTEGER, got {other:?}"))),
                },
            }
        };
        let skip = count(&p.skip, "SKIP")?.unwrap_or(0);
        let limit = count(&p.limit, "LIMIT")?.unwrap_or(usize::MAX);
        Ok((columns, produced.into_iter().skip(skip).take(limit).map(|(pr, _)| pr).collect()))
    }
}

// ---- aggregation ----

#[derive(Debug, Clone, Copy, PartialEq)]
enum AggFn {
    Count,
    Sum,
    Avg,
    Min,
    Max,
    Collect,
}

struct AggSpec {
    func: AggFn,
    arg: Option<Expr>,
    distinct: bool,
}

/// Synthetic row variable holding the result of aggregate `i`. The leading
/// space makes it impossible to collide with user variables.
fn agg_var(i: usize) -> String {
    format!(" agg{i}")
}

/// Replaces aggregate calls with references to synthetic variables.
fn extract_aggregates(e: &Expr, aggs: &mut Vec<AggSpec>) -> Result<Expr> {
    let rec = |x: &Expr, aggs: &mut Vec<AggSpec>| extract_aggregates(x, aggs).map(Box::new);
    Ok(match e {
        Expr::CountStar => {
            aggs.push(AggSpec { func: AggFn::Count, arg: None, distinct: false });
            Expr::Variable(agg_var(aggs.len() - 1))
        }
        Expr::FunctionCall { name, args, distinct } if is_aggregate_function(name) => {
            let func = match name.to_ascii_lowercase().as_str() {
                "count" => AggFn::Count,
                "sum" => AggFn::Sum,
                "avg" => AggFn::Avg,
                "min" => AggFn::Min,
                "max" => AggFn::Max,
                _ => AggFn::Collect,
            };
            if args.len() != 1 {
                return Err(MyGraphError::Semantic(format!("`{name}` expects exactly one argument")));
            }
            if args[0].contains_aggregate() {
                return Err(MyGraphError::Semantic("aggregate functions cannot be nested".into()));
            }
            aggs.push(AggSpec { func, arg: Some(args[0].clone()), distinct: *distinct });
            Expr::Variable(agg_var(aggs.len() - 1))
        }
        Expr::Literal(_) | Expr::Parameter(_) | Expr::Variable(_) => e.clone(),
        Expr::Property(b, k) => Expr::Property(rec(b, aggs)?, k.clone()),
        Expr::Index(a, b) => Expr::Index(rec(a, aggs)?, rec(b, aggs)?),
        Expr::List(items) => Expr::List(items.iter().map(|i| extract_aggregates(i, aggs)).collect::<Result<_>>()?),
        Expr::Map(entries) => Expr::Map(
            entries.iter().map(|(k, v)| Ok((k.clone(), extract_aggregates(v, aggs)?))).collect::<Result<_>>()?,
        ),
        Expr::Unary(op, x) => Expr::Unary(*op, rec(x, aggs)?),
        Expr::Binary(op, a, b) => Expr::Binary(*op, rec(a, aggs)?, rec(b, aggs)?),
        Expr::IsNull { expr, negated } => Expr::IsNull { expr: rec(expr, aggs)?, negated: *negated },
        Expr::HasLabels(x, l) => Expr::HasLabels(rec(x, aggs)?, l.clone()),
        Expr::FunctionCall { name, args, distinct } => Expr::FunctionCall {
            name: name.clone(),
            args: args.iter().map(|a| extract_aggregates(a, aggs)).collect::<Result<_>>()?,
            distinct: *distinct,
        },
        Expr::Case { operand, branches, default } => Expr::Case {
            operand: operand.as_ref().map(|o| rec(o, aggs)).transpose()?,
            branches: branches
                .iter()
                .map(|(w, t)| Ok((extract_aggregates(w, aggs)?, extract_aggregates(t, aggs)?)))
                .collect::<Result<_>>()?,
            default: default.as_ref().map(|d| rec(d, aggs)).transpose()?,
        },
    })
}

struct Accumulator {
    func: AggFn,
    distinct: bool,
    seen: HashSet<String>,
    count: i64,
    sum: Val,
    float_sum: f64,
    best: Option<Val>,
    items: Vec<Val>,
}

impl Accumulator {
    fn new(spec: &AggSpec) -> Self {
        Accumulator {
            func: spec.func,
            distinct: spec.distinct,
            seen: HashSet::new(),
            count: 0,
            sum: Val::Int(0),
            float_sum: 0.0,
            best: None,
            items: Vec::new(),
        }
    }

    /// `None` is a `count(*)` row.
    fn update(&mut self, v: Option<Val>) -> Result<()> {
        let Some(v) = v else {
            self.count += 1;
            return Ok(());
        };
        if v == Val::Null {
            return Ok(());
        }
        if self.distinct && !self.seen.insert(group_key(&[&v])) {
            return Ok(());
        }
        match self.func {
            AggFn::Count => self.count += 1,
            AggFn::Sum => {
                if !matches!(v, Val::Int(_) | Val::Float(_)) {
                    return Err(type_err(format!("sum() expects numbers, got {}", v.type_name())));
                }
                self.sum = arithmetic(BinaryOp::Add, std::mem::replace(&mut self.sum, Val::Null), v)?;
            }
            AggFn::Avg => match v {
                Val::Int(i) => {
                    self.float_sum += i as f64;
                    self.count += 1;
                }
                Val::Float(f) => {
                    self.float_sum += f;
                    self.count += 1;
                }
                other => return Err(type_err(format!("avg() expects numbers, got {}", other.type_name()))),
            },
            AggFn::Min | AggFn::Max => {
                let replace = match &self.best {
                    None => true,
                    Some(b) => {
                        let c = order_cmp(&v, b);
                        if self.func == AggFn::Min {
                            c.is_lt()
                        } else {
                            c.is_gt()
                        }
                    }
                };
                if replace {
                    self.best = Some(v);
                }
            }
            AggFn::Collect => self.items.push(v),
        }
        Ok(())
    }

    fn finish(self) -> Val {
        match self.func {
            AggFn::Count => Val::Int(self.count),
            AggFn::Sum => self.sum,
            AggFn::Avg => {
                if self.count == 0 {
                    Val::Null
                } else {
                    Val::Float(self.float_sum / self.count as f64)
                }
            }
            AggFn::Min | AggFn::Max => self.best.unwrap_or(Val::Null),
            AggFn::Collect => Val::List(self.items),
        }
    }
}
