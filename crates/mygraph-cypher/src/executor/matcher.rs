//! MATCH execution: a small rule-based planner plus a backtracking matcher.
//!
//! Planning (once per MATCH clause):
//! - `WHERE` is split into its `AND`-ed conjuncts. Each conjunct runs as soon
//!   as every variable it uses is bound, so failing branches are pruned early
//!   instead of after the whole pattern (and any cross product) is enumerated.
//! - Each path is anchored on its most selective node: a bound variable, then
//!   an `id(n) = …` lookup, then a property index, then the smallest label.
//!   A path anchored on its last node is reversed; one anchored in the middle
//!   is split into two paths that share the anchor. Named paths are only
//!   reversed, never split, and `shortestPath` patterns run last.
//!
//! Variable-length relationships (`*min..max`) are expanded depth-first with
//! relationship uniqueness; shortest paths use a breadth-first search.
//!
//! Matching uses the same rules as before (relationship uniqueness across the
//! whole MATCH, node and relationship checks on every step), so the plan only
//! changes how fast results are found, not which results are found.

use std::collections::{BTreeMap, HashMap, HashSet};

use mygraph_core::{Graph, LabelId, NodeId, PropertyMap, RelId, Result, Value};

use crate::ast::*;
use crate::executor::eval::{equals, type_err, Env, Row, Val};

/// Prefix of the variables the planner introduces. The leading space keeps
/// them out of `RETURN *` and makes collisions with user variables impossible.
const HIDDEN_PREFIX: &str = " ~";

/// One path of the plan: a written path, possibly reversed or split.
struct Planned {
    /// Steps in traversal order. `name` and `shortest` are taken from the
    /// written path; pieces of a split path have neither.
    pat: PathPattern,
    /// The steps run against the written direction, so variable-length
    /// relationship lists are reversed before they are bound.
    reversed: bool,
    /// For a named path: the written pattern, used to assemble the path once
    /// all of its elements are bound.
    written: Option<PathPattern>,
}

/// Gives every unnamed node and relationship of `p` a hidden variable, so a
/// named or shortest path can be assembled from the row.
fn name_elements(p: &mut PathPattern, counter: &mut usize) {
    let mut name = |v: &mut Option<String>| {
        if v.is_none() {
            *v = Some(format!("{HIDDEN_PREFIX}{counter}"));
            *counter += 1;
        }
    };
    name(&mut p.start.variable);
    for (r, n) in &mut p.steps {
        name(&mut r.variable);
        name(&mut n.variable);
    }
}

struct Conjunct {
    expr: Expr,
    vars: Vec<String>,
}

/// How a conjunct can narrow the start candidates of a variable.
enum Lookup {
    Id(Expr),
    IdIn(Expr),
    Prop(String, Expr),
    PropIn(String, Expr),
}

struct Hint {
    var: String,
    lookup: Lookup,
    /// Variables the lookup expression needs.
    deps: Vec<String>,
}

pub(crate) struct Matcher<'e, 'g> {
    env: &'e Env<'g>,
    pats: Vec<Planned>,
    /// Whether rows carry hidden planner variables that must be removed.
    hidden: bool,
    /// Conjuncts needing no pattern variable: checked once per input row.
    pre: Vec<Conjunct>,
    /// Conjuncts checked when their last variable becomes bound.
    conj: Vec<Conjunct>,
    hints: Vec<Hint>,
}

fn expr_vars(e: &Expr) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    e.walk(&mut |x| {
        if let Expr::Variable(v) = x {
            if !out.contains(v) {
                out.push(v.clone());
            }
        }
    });
    out
}

fn split_and(e: &Expr, out: &mut Vec<Expr>) {
    match e {
        Expr::Binary(BinaryOp::And, a, b) => {
            split_and(a, out);
            split_and(b, out);
        }
        other => out.push(other.clone()),
    }
}

/// `n.key` → (n, key)
fn as_property(e: &Expr) -> Option<(&str, &str)> {
    match e {
        Expr::Property(b, k) => match b.as_ref() {
            Expr::Variable(v) => Some((v, k)),
            _ => None,
        },
        _ => None,
    }
}

/// `id(n)` → n
fn as_id_call(e: &Expr) -> Option<&str> {
    match e {
        Expr::FunctionCall { name, args, distinct: false } if name.eq_ignore_ascii_case("id") && args.len() == 1 => {
            match &args[0] {
                Expr::Variable(v) => Some(v),
                _ => None,
            }
        }
        _ => None,
    }
}

fn hint_from(e: &Expr) -> Option<Hint> {
    let Expr::Binary(op, l, r) = e else { return None };
    let make = |var: &str, lookup: Lookup, other: &Expr| {
        let deps = expr_vars(other);
        (!deps.iter().any(|d| d == var)).then(|| Hint { var: var.to_string(), lookup, deps })
    };
    match op {
        BinaryOp::Eq => {
            for (a, b) in [(l, r), (r, l)] {
                if let Some(v) = as_id_call(a) {
                    return make(v, Lookup::Id((**b).clone()), b);
                }
                if let Some((v, k)) = as_property(a) {
                    return make(v, Lookup::Prop(k.to_string(), (**b).clone()), b);
                }
            }
            None
        }
        BinaryOp::In => {
            if let Some(v) = as_id_call(l) {
                return make(v, Lookup::IdIn((**r).clone()), r);
            }
            if let Some((v, k)) = as_property(l) {
                return make(v, Lookup::PropIn(k.to_string(), (**r).clone()), r);
            }
            None
        }
        _ => None,
    }
}

fn flip(d: Direction) -> Direction {
    match d {
        Direction::Outgoing => Direction::Incoming,
        Direction::Incoming => Direction::Outgoing,
        Direction::Both => Direction::Both,
    }
}

/// Node `i` of a path (0 = start).
fn node_at(p: &PathPattern, i: usize) -> &NodePattern {
    if i == 0 {
        &p.start
    } else {
        &p.steps[i - 1].1
    }
}

/// The path read backwards from node `upto` to the start.
fn reversed_prefix(p: &PathPattern, upto: usize) -> PathPattern {
    let mut steps = Vec::with_capacity(upto);
    for i in (1..=upto).rev() {
        let (rel, _) = &p.steps[i - 1];
        let mut rel = rel.clone();
        rel.direction = flip(rel.direction);
        steps.push((rel, node_at(p, i - 1).clone()));
    }
    PathPattern { name: None, shortest: None, start: node_at(p, upto).clone(), steps }
}

fn suffix(p: &PathPattern, from: usize) -> PathPattern {
    PathPattern { name: None, shortest: None, start: node_at(p, from).clone(), steps: p.steps[from..].to_vec() }
}

/// Whether any inline property map uses a variable bound by this MATCH. Such
/// patterns keep their written order, so the variable is bound before use.
fn inline_props_depend_on(patterns: &[PathPattern], local: &HashSet<String>) -> bool {
    let uses_local = |e: &Option<Expr>| e.as_ref().is_some_and(|e| expr_vars(e).iter().any(|v| local.contains(v)));
    patterns.iter().any(|p| {
        uses_local(&p.start.properties)
            || p.steps.iter().any(|(r, n)| uses_local(&r.properties) || uses_local(&n.properties))
    })
}

impl<'e, 'g> Matcher<'e, 'g> {
    /// Plans a MATCH. `sample` is a representative input row: it tells which
    /// variables are already bound and gives values to estimate selectivity.
    pub fn new(env: &'e Env<'g>, patterns: &[PathPattern], filter: Option<&Expr>, sample: &Row) -> Self {
        let local: HashSet<String> = patterns
            .iter()
            .flat_map(|p| p.variables())
            .filter(|v| !sample.contains_key(*v))
            .map(str::to_string)
            .collect();

        let mut conjuncts = Vec::new();
        if let Some(f) = filter {
            split_and(f, &mut conjuncts);
        }
        let mut pre = Vec::new();
        let mut conj = Vec::new();
        let mut hints = Vec::new();
        for expr in conjuncts {
            if let Some(h) = hint_from(&expr) {
                if local.contains(&h.var) {
                    hints.push(h);
                }
            }
            let vars = expr_vars(&expr);
            if vars.iter().any(|v| local.contains(v)) {
                conj.push(Conjunct { expr, vars });
            } else {
                pre.push(Conjunct { expr, vars });
            }
        }

        let mut counter = 0;
        let patterns: Vec<PathPattern> = patterns
            .iter()
            .cloned()
            .map(|mut p| {
                if p.name.is_some() || p.shortest.is_some() {
                    name_elements(&mut p, &mut counter);
                }
                p
            })
            .collect();
        let mut m = Matcher { env, pats: Vec::new(), hidden: counter > 0, pre, conj, hints };
        m.pats = if inline_props_depend_on(&patterns, &local) {
            patterns.into_iter().map(|p| m.written_order(p)).collect()
        } else {
            m.plan(patterns, sample)
        };
        m
    }

    /// A path planned in written order (anchored on its first node).
    fn written_order(&self, p: PathPattern) -> Planned {
        let written = p.name.is_some().then(|| p.clone());
        Planned { pat: p, reversed: false, written }
    }

    /// Estimated number of start candidates for node pattern `np`, given the
    /// variables in `bound`. Lower is better.
    fn cost(&self, np: &NodePattern, bound: &HashSet<String>, sample: &Row) -> usize {
        let g = self.env.graph;
        if np.variable.as_ref().is_some_and(|v| bound.contains(v)) {
            return 1;
        }
        let ready = |deps: &[String]| deps.iter().all(|d| bound.contains(d));
        let mut best = np.labels.iter().map(|l| g.label_count(l)).min().unwrap_or(g.node_count()) + 2;
        if let Some(v) = &np.variable {
            for h in self.hints.iter().filter(|h| &h.var == v && ready(&h.deps)) {
                let c = match &h.lookup {
                    Lookup::Id(_) => 1,
                    Lookup::IdIn(_) => 2,
                    Lookup::Prop(k, e) => match np.labels.iter().find(|l| g.has_index(l, k)) {
                        Some(l) => self.estimate_lookup(l, k, e, sample),
                        None => continue,
                    },
                    Lookup::PropIn(k, _) => match np.labels.iter().any(|l| g.has_index(l, k)) {
                        true => 10,
                        false => continue,
                    },
                };
                best = best.min(c);
            }
        }
        if let Some(Expr::Map(entries)) = &np.properties {
            for (k, e) in entries {
                if let Some(l) = np.labels.iter().find(|l| g.has_index(l, k)) {
                    best = best.min(self.estimate_lookup(l, k, e, sample));
                }
            }
        }
        best
    }

    /// Exact index hit count when the value can be computed from `sample`,
    /// otherwise a small constant (an index lookup is assumed selective).
    fn estimate_lookup(&self, label: &str, key: &str, e: &Expr, sample: &Row) -> usize {
        let exact = self
            .env
            .eval(e, sample)
            .ok()
            .and_then(|v| v.into_property().ok())
            .and_then(|v| self.env.graph.nodes_by_property(label, key, &v));
        exact.map_or(3, |ids| ids.len() + 1)
    }

    fn plan(&mut self, patterns: Vec<PathPattern>, sample: &Row) -> Vec<Planned> {
        let mut bound: HashSet<String> = sample.keys().cloned().collect();
        // shortest paths need their end nodes bound: they run after everything else
        let (shortest, mut remaining): (Vec<PathPattern>, Vec<PathPattern>) =
            patterns.into_iter().partition(|p| p.shortest.is_some());
        let mut out = Vec::new();
        while !remaining.is_empty() {
            // (cost, pattern index, node position); ties keep the written order
            let mut best = (usize::MAX, 0, 0);
            for (pi, p) in remaining.iter().enumerate() {
                let len = p.steps.len();
                for i in 0..=len {
                    if p.name.is_some() && i != 0 && i != len {
                        continue; // named paths are never split
                    }
                    let c = self.cost(node_at(p, i), &bound, sample);
                    if c < best.0 {
                        best = (c, pi, i);
                    }
                }
            }
            let (_, pi, i) = best;
            let mut p = remaining.remove(pi);
            for v in p.variables() {
                bound.insert(v.to_string());
            }
            let len = p.steps.len();
            if i == 0 {
                out.push(self.written_order(p));
            } else if i == len {
                let mut rev = reversed_prefix(&p, i);
                rev.name = p.name.clone();
                let written = p.name.is_some().then_some(p);
                out.push(Planned { pat: rev, reversed: true, written });
            } else {
                let anchor = &mut p.steps[i - 1].1;
                if anchor.variable.is_none() {
                    anchor.variable = Some(format!("{HIDDEN_PREFIX}a{}", out.len()));
                    self.hidden = true;
                }
                out.push(Planned { pat: reversed_prefix(&p, i), reversed: true, written: None });
                out.push(Planned { pat: suffix(&p, i), reversed: false, written: None });
            }
        }
        out.extend(shortest.into_iter().map(|p| self.written_order(p)));
        out
    }

    /// All matches for one input row (before the final projection).
    pub fn run(&self, row: Row) -> Result<Vec<Row>> {
        for c in &self.pre {
            if !self.holds(&c.expr, &row)? {
                return Ok(Vec::new());
            }
        }
        let mut out = Vec::new();
        let mut used = Vec::new();
        self.match_patterns(0, row, &mut used, &mut out)?;
        if self.hidden {
            for r in &mut out {
                r.retain(|k, _| !k.starts_with(HIDDEN_PREFIX));
            }
        }
        Ok(out)
    }

    fn holds(&self, e: &Expr, row: &Row) -> Result<bool> {
        match self.env.eval(e, row)? {
            Val::Bool(b) => Ok(b),
            Val::Null => Ok(false),
            other => Err(type_err(format!("WHERE expects a BOOLEAN, got {}", other.type_name()))),
        }
    }

    /// Binds `var` and runs the conjuncts it completes. `false` prunes the branch.
    fn bind(&self, row: &mut Row, var: &Option<String>, val: Val) -> Result<bool> {
        let Some(v) = var else { return Ok(true) };
        if row.insert(v.clone(), val).is_some() {
            return Ok(true); // already bound: its conjuncts already ran
        }
        for c in &self.conj {
            if c.vars.iter().any(|x| x == v)
                && c.vars.iter().all(|x| row.contains_key(x))
                && !self.holds(&c.expr, row)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn match_patterns(&self, pi: usize, row: Row, used: &mut Vec<RelId>, out: &mut Vec<Row>) -> Result<()> {
        if pi == self.pats.len() {
            out.push(row);
            return Ok(());
        }
        let p = &self.pats[pi].pat;
        if let Some(kind) = p.shortest {
            return self.match_shortest(pi, kind, row, used, out);
        }
        let check = NodeCheck::new(self.env, &p.start, &row)?;
        if check.never() {
            return Ok(());
        }
        for nid in self.start_candidates(&p.start, &row)? {
            if !check.matches(self.env.graph, nid) {
                continue;
            }
            let mut r = row.clone();
            if !self.bind(&mut r, &p.start.variable, Val::Node(nid))? {
                continue;
            }
            self.expand(pi, 0, nid, r, used, out)?;
        }
        Ok(())
    }

    fn start_candidates(&self, np: &NodePattern, row: &Row) -> Result<Vec<NodeId>> {
        let g = self.env.graph;
        if let Some(v) = &np.variable {
            if let Some(bound) = row.get(v) {
                return match bound {
                    Val::Node(id) => Ok(vec![*id]),
                    Val::Null => Ok(vec![]),
                    other => Err(type_err(format!("variable `{v}` is a {}, expected a NODE", other.type_name()))),
                };
            }
            let ready = |h: &&Hint| &h.var == v && h.deps.iter().all(|d| row.contains_key(d));
            for h in self.hints.iter().filter(ready) {
                if let Some(ids) = self.lookup(np, &h.lookup, row)? {
                    return Ok(ids);
                }
            }
        }
        if let Some(Expr::Map(entries)) = &np.properties {
            for (k, e) in entries {
                if let Some(ids) = self.lookup(np, &Lookup::Prop(k.clone(), e.clone()), row)? {
                    return Ok(ids);
                }
            }
        }
        let smallest = np.labels.iter().min_by_key(|l| g.label_count(l));
        Ok(match smallest {
            Some(label) => g.node_ids_with_label(label),
            None => g.nodes().map(|n| n.id).collect(),
        })
    }

    /// Candidates from one lookup, or `None` when it cannot be used (no index,
    /// or a value of a type the index does not cover).
    fn lookup(&self, np: &NodePattern, lookup: &Lookup, row: &Row) -> Result<Option<Vec<NodeId>>> {
        let g = self.env.graph;
        let id_of = |v: &Val| -> Option<Option<NodeId>> {
            match v {
                Val::Int(i) => Some((*i >= 0).then_some(*i as NodeId)),
                Val::Float(f) if f.fract() == 0.0 && *f >= 0.0 && *f < 9.0e15 => Some(Some(*f as NodeId)),
                Val::Null => Some(None),
                _ => None,
            }
        };
        let by_prop = |key: &str, v: Val| -> Option<Vec<NodeId>> {
            let v = v.into_property().ok()?;
            np.labels.iter().find_map(|l| g.nodes_by_property(l, key, &v))
        };
        Ok(match lookup {
            Lookup::Id(e) => {
                id_of(&self.env.eval(e, row)?).map(|id| id.filter(|id| g.node(*id).is_some()).into_iter().collect())
            }
            Lookup::IdIn(e) => match self.env.eval(e, row)? {
                Val::Null => Some(Vec::new()),
                Val::List(items) => {
                    let mut ids = Vec::with_capacity(items.len());
                    for item in &items {
                        match id_of(item) {
                            Some(id) => ids.extend(id.filter(|id| g.node(*id).is_some())),
                            None => return Ok(None),
                        }
                    }
                    ids.sort_unstable();
                    ids.dedup();
                    Some(ids)
                }
                _ => None,
            },
            Lookup::Prop(k, e) => {
                if !np.labels.iter().any(|l| g.has_index(l, k)) {
                    return Ok(None);
                }
                by_prop(k, self.env.eval(e, row)?)
            }
            Lookup::PropIn(k, e) => {
                if !np.labels.iter().any(|l| g.has_index(l, k)) {
                    return Ok(None);
                }
                match self.env.eval(e, row)? {
                    Val::Null => Some(Vec::new()),
                    Val::List(items) => {
                        let mut ids = Vec::new();
                        for item in items {
                            match by_prop(k, item) {
                                Some(found) => ids.extend(found),
                                None => return Ok(None),
                            }
                        }
                        ids.sort_unstable();
                        ids.dedup();
                        Some(ids)
                    }
                    _ => None,
                }
            }
        })
    }

    fn expand(
        &self,
        pi: usize,
        step: usize,
        cur: NodeId,
        row: Row,
        used: &mut Vec<RelId>,
        out: &mut Vec<Row>,
    ) -> Result<()> {
        let planned = &self.pats[pi];
        let p = &planned.pat;
        if step == p.steps.len() {
            let mut row = row;
            if let Some(written) = &planned.written {
                let path = self.assemble(written, &row)?;
                if !self.bind(&mut row, &written.name, path)? {
                    return Ok(());
                }
            }
            return self.match_patterns(pi + 1, row, used, out);
        }
        let (rp, np) = &p.steps[step];
        let Some(st) = StepCtx::new(self.env, rp, np, &row)? else { return Ok(()) };
        if let Some(len) = rp.length {
            let mut trail = Vec::new();
            return self.expand_var(pi, step, &st, len, cur, &row, used, &mut trail, out);
        }
        for (rid, other) in st.edges(self.env.graph, cur, used) {
            match st.bound_rel {
                None => {}
                Some(Val::Rel(b)) if *b == rid => {}
                Some(Val::Rel(_) | Val::Null) => continue,
                Some(v) => return Err(type_err(format!("expected a RELATIONSHIP, got {}", v.type_name()))),
            }
            if !st.node_ok(self.env.graph, other)? {
                continue;
            }
            let mut r = row.clone();
            if !self.bind(&mut r, &rp.variable, Val::Rel(rid))? || !self.bind(&mut r, &np.variable, Val::Node(other))? {
                continue;
            }
            used.push(rid);
            self.expand(pi, step + 1, other, r, used, out)?;
            used.pop();
        }
        Ok(())
    }

    /// Depth-first expansion of a variable-length step. `trail` holds the
    /// relationships taken so far, in traversal order.
    #[allow(clippy::too_many_arguments)]
    fn expand_var(
        &self,
        pi: usize,
        step: usize,
        st: &StepCtx<'_>,
        len: VarLength,
        cur: NodeId,
        row: &Row,
        used: &mut Vec<RelId>,
        trail: &mut Vec<RelId>,
        out: &mut Vec<Row>,
    ) -> Result<()> {
        let g = self.env.graph;
        let depth = trail.len() as u32;
        if depth >= len.min && st.node_ok(g, cur)? {
            let mut rels: Vec<Val> = trail.iter().map(|r| Val::Rel(*r)).collect();
            if self.pats[pi].reversed {
                rels.reverse();
            }
            let list = Val::List(rels);
            let same_as_bound = st.bound_rel.is_none_or(|b| *b == list);
            let mut r = row.clone();
            if same_as_bound
                && self.bind(&mut r, &st.rel.variable, list)?
                && self.bind(&mut r, &st.node.variable, Val::Node(cur))?
            {
                self.expand(pi, step + 1, cur, r, used, out)?;
            }
        }
        if len.max.is_none_or(|m| depth < m) {
            for (rid, other) in st.edges(g, cur, used) {
                used.push(rid);
                trail.push(rid);
                self.expand_var(pi, step, st, len, other, row, used, trail, out)?;
                trail.pop();
                used.pop();
            }
        }
        Ok(())
    }

    /// Builds the value of a named path from the elements bound in `row`.
    fn assemble(&self, written: &PathPattern, row: &Row) -> Result<Val> {
        let g = self.env.graph;
        let get = |v: &Option<String>| v.as_ref().and_then(|v| row.get(v)).cloned().unwrap_or(Val::Null);
        let Val::Node(first) = get(&written.start.variable) else {
            return Err(type_err("a path must start with a node"));
        };
        let (mut nodes, mut rels) = (vec![first], Vec::new());
        for (rp, np) in &written.steps {
            match get(&rp.variable) {
                Val::Rel(r) => {
                    rels.push(r);
                    let Val::Node(n) = get(&np.variable) else { return Err(type_err("a path node is not bound")) };
                    nodes.push(n);
                }
                Val::List(items) => {
                    for item in items {
                        let Val::Rel(r) = item else { return Err(type_err("a path relationship is not bound")) };
                        let rel = g.relationship(r).ok_or_else(|| type_err("a path relationship has been deleted"))?;
                        let cur = *nodes.last().expect("starts with a node");
                        nodes.push(if rel.start_node == cur { rel.end_node } else { rel.start_node });
                        rels.push(r);
                    }
                }
                other => {
                    return Err(type_err(format!("expected a RELATIONSHIP in the path, got {}", other.type_name())))
                }
            }
        }
        Ok(Val::Path { nodes, rels })
    }

    /// `shortestPath((a)-[*..n]->(b))` / `allShortestPaths(...)`.
    fn match_shortest(
        &self,
        pi: usize,
        kind: Shortest,
        row: Row,
        used: &mut Vec<RelId>,
        out: &mut Vec<Row>,
    ) -> Result<()> {
        let g = self.env.graph;
        let p = &self.pats[pi].pat;
        let (rp, end) = &p.steps[0];
        let len = rp.length.unwrap_or(VarLength { min: 1, max: Some(1) });
        let start_check = NodeCheck::new(self.env, &p.start, &row)?;
        if start_check.never() {
            return Ok(());
        }
        for s in self.start_candidates(&p.start, &row)? {
            if !start_check.matches(g, s) {
                continue;
            }
            let mut r1 = row.clone();
            if !self.bind(&mut r1, &p.start.variable, Val::Node(s))? {
                continue;
            }
            let Some(st) = StepCtx::new(self.env, rp, end, &r1)? else { return Ok(()) };
            for e in self.start_candidates(end, &r1)? {
                if !st.node_ok(g, e)? {
                    continue;
                }
                let mut r2 = r1.clone();
                if !self.bind(&mut r2, &end.variable, Val::Node(e))? {
                    continue;
                }
                for (nodes, rels) in st.shortest(g, s, e, len, kind == Shortest::All, used) {
                    let list = Val::List(rels.iter().map(|r| Val::Rel(*r)).collect());
                    if st.bound_rel.is_some_and(|b| *b != list) {
                        continue;
                    }
                    let mut r3 = r2.clone();
                    if !self.bind(&mut r3, &rp.variable, list)?
                        || !self.bind(&mut r3, &p.name, Val::Path { nodes, rels: rels.clone() })?
                    {
                        continue;
                    }
                    used.extend(&rels);
                    self.match_patterns(pi + 1, r3, used, out)?;
                    used.truncate(used.len() - rels.len());
                }
            }
        }
        Ok(())
    }
}

/// What one relationship step needs, resolved once per row.
struct StepCtx<'a> {
    rel: &'a RelPattern,
    node: &'a NodePattern,
    /// Allowed relationship types; empty means any.
    types: Vec<u32>,
    rel_props: Option<BTreeMap<String, Val>>,
    node_check: NodeCheck,
    bound_rel: Option<&'a Val>,
    bound_node: Option<&'a Val>,
}

impl<'a> StepCtx<'a> {
    /// `None` when nothing can match (an unknown type or label).
    fn new(env: &Env<'_>, rel: &'a RelPattern, node: &'a NodePattern, row: &'a Row) -> Result<Option<Self>> {
        let types: Vec<u32> = rel.types.iter().filter_map(|t| env.graph.rel_type_id(t)).collect();
        if !rel.types.is_empty() && types.is_empty() {
            return Ok(None);
        }
        let node_check = NodeCheck::new(env, node, row)?;
        if node_check.never() {
            return Ok(None);
        }
        Ok(Some(StepCtx {
            rel,
            node,
            types,
            rel_props: eval_props(env, &rel.properties, row)?,
            node_check,
            bound_rel: rel.variable.as_ref().and_then(|v| row.get(v)),
            bound_node: node.variable.as_ref().and_then(|v| row.get(v)),
        }))
    }

    /// Relationships of `cur` this step may follow (direction, type,
    /// properties, uniqueness), with the node at their other end.
    fn edges(&self, g: &Graph, cur: NodeId, used: &[RelId]) -> Vec<(RelId, NodeId)> {
        self.edges_towards(g, cur, used, self.rel.direction)
    }

    /// Like [`StepCtx::edges`], following `dir` instead of the pattern's direction.
    fn edges_towards(&self, g: &Graph, cur: NodeId, used: &[RelId], dir: Direction) -> Vec<(RelId, NodeId)> {
        let Some(node) = g.node(cur) else { return Vec::new() };
        let mut out = Vec::new();
        let mut consider = |rid: RelId, outgoing: bool| {
            if used.contains(&rid) {
                return;
            }
            let Some(r) = g.relationship(rid) else { return };
            // a self-loop is already listed among the outgoing relationships
            if !outgoing && dir == Direction::Both && r.start_node == r.end_node {
                return;
            }
            if !self.types.is_empty() && !self.types.contains(&r.rel_type) {
                return;
            }
            if self.rel_props.as_ref().is_some_and(|m| !props_equal(m, &r.properties)) {
                return;
            }
            out.push((rid, if outgoing { r.end_node } else { r.start_node }));
        };
        if matches!(dir, Direction::Outgoing | Direction::Both) {
            node.outgoing().iter().for_each(|r| consider(*r, true));
        }
        if matches!(dir, Direction::Incoming | Direction::Both) {
            node.incoming().iter().for_each(|r| consider(*r, false));
        }
        out
    }

    /// Whether `n` can be the step's end node: the node pattern matches and,
    /// if its variable is bound, it is that node.
    fn node_ok(&self, g: &Graph, n: NodeId) -> Result<bool> {
        match self.bound_node {
            None => {}
            Some(Val::Node(b)) if *b == n => {}
            Some(Val::Node(_) | Val::Null) => return Ok(false),
            Some(v) => return Err(type_err(format!("expected a NODE, got {}", v.type_name()))),
        }
        Ok(self.node_check.matches(g, n))
    }

    /// Breadth-first search from `s` to `e`. Returns one shortest path, or all
    /// of them, as (nodes, relationships).
    fn shortest(
        &self,
        g: &Graph,
        s: NodeId,
        e: NodeId,
        len: VarLength,
        all: bool,
        used: &[RelId],
    ) -> Vec<(Vec<NodeId>, Vec<RelId>)> {
        if s == e {
            return if len.min == 0 { vec![(vec![s], Vec::new())] } else { Vec::new() };
        }
        let max = len.max.unwrap_or(u32::MAX);
        if !all {
            return self.bidirectional(g, s, e, max, used).into_iter().collect();
        }
        // node -> (depth, [(relationship, parent)])
        let mut seen: HashMap<NodeId, (u32, Vec<(RelId, NodeId)>)> = HashMap::from([(s, (0, Vec::new()))]);
        let mut frontier = vec![s];
        let mut depth = 0;
        while !frontier.is_empty() && depth < max && !seen.contains_key(&e) {
            depth += 1;
            let mut next = Vec::new();
            for u in frontier {
                for (rid, v) in self.edges(g, u, used) {
                    match seen.get_mut(&v) {
                        None => {
                            seen.insert(v, (depth, vec![(rid, u)]));
                            next.push(v);
                        }
                        Some((d, parents)) if *d == depth && all => parents.push((rid, u)),
                        Some(_) => {}
                    }
                }
            }
            frontier = next;
        }
        if !seen.contains_key(&e) {
            return Vec::new();
        }
        // walk the parent links back from `e`
        let mut paths = Vec::new();
        let mut stack: Vec<(NodeId, Vec<NodeId>, Vec<RelId>)> = vec![(e, vec![e], Vec::new())];
        while let Some((n, nodes, rels)) = stack.pop() {
            if n == s {
                let (mut nodes, mut rels) = (nodes, rels);
                nodes.reverse();
                rels.reverse();
                paths.push((nodes, rels));
                if !all {
                    break;
                }
                continue;
            }
            let parents = &seen[&n].1;
            let take = if all { parents.len() } else { 1 };
            // push in reverse so paths come out in adjacency order
            for (rid, parent) in parents[..take].iter().rev() {
                let (mut ns, mut rs) = (nodes.clone(), rels.clone());
                ns.push(*parent);
                rs.push(*rid);
                stack.push((*parent, ns, rs));
            }
        }
        paths
    }

    /// One shortest path, searching from both ends and always growing the
    /// smaller frontier by a full level. Visits far fewer nodes than a
    /// one-sided search on large graphs.
    fn bidirectional(
        &self,
        g: &Graph,
        s: NodeId,
        e: NodeId,
        max: u32,
        used: &[RelId],
    ) -> Option<(Vec<NodeId>, Vec<RelId>)> {
        // node -> (relationship, neighbour one step closer to that side's root)
        let mut seen: [HashMap<NodeId, Option<(RelId, NodeId)>>; 2] =
            [HashMap::from([(s, None)]), HashMap::from([(e, None)])];
        let mut frontier = [vec![s], vec![e]];
        let dirs = [self.rel.direction, flip(self.rel.direction)];
        let mut depth = 0u32;
        while depth < max && !frontier[0].is_empty() && !frontier[1].is_empty() {
            let side = usize::from(frontier[1].len() < frontier[0].len());
            let mut next = Vec::new();
            let mut meet: Option<NodeId> = None;
            for u in std::mem::take(&mut frontier[side]) {
                for (rid, v) in self.edges_towards(g, u, used, dirs[side]) {
                    if seen[side].contains_key(&v) {
                        continue;
                    }
                    seen[side].insert(v, Some((rid, u)));
                    if meet.is_none() && seen[1 - side].contains_key(&v) {
                        // the sides were disjoint before this level, so any
                        // meeting node on it gives the same total length
                        meet = Some(v);
                    }
                    next.push(v);
                }
                if meet.is_some() {
                    break;
                }
            }
            depth += 1;
            if let Some(m) = meet {
                let walk = |side: usize| {
                    let (mut nodes, mut rels, mut cur) = (Vec::new(), Vec::new(), m);
                    while let Some(Some((rid, prev))) = seen[side].get(&cur) {
                        rels.push(*rid);
                        nodes.push(*prev);
                        cur = *prev;
                    }
                    (nodes, rels)
                };
                let (mut nodes, mut rels) = walk(0);
                nodes.reverse();
                rels.reverse();
                nodes.push(m);
                let (tail_nodes, tail_rels) = walk(1);
                nodes.extend(tail_nodes);
                rels.extend(tail_rels);
                return Some((nodes, rels));
            }
            frontier[side] = next;
        }
        None
    }
}

fn eval_props(env: &Env<'_>, expr: &Option<Expr>, row: &Row) -> Result<Option<BTreeMap<String, Val>>> {
    match expr {
        None => Ok(None),
        Some(e) => match env.eval(e, row)? {
            Val::Map(m) => Ok(Some(m)),
            _ => Err(type_err("pattern properties must be a MAP")),
        },
    }
}

/// Whether every expected entry equals the stored property (Cypher equality).
fn props_equal(expected: &BTreeMap<String, Val>, props: &PropertyMap) -> bool {
    expected.iter().all(|(k, v)| {
        props.get(k).is_some_and(|actual| match (actual, v) {
            // fast paths that avoid converting the stored value
            (Value::String(a), Val::Str(b)) => a == b,
            (Value::Integer(a), Val::Int(b)) => a == b,
            (Value::Boolean(a), Val::Bool(b)) => a == b,
            _ => equals(&Val::from_value(actual), v) == Some(true),
        })
    })
}

/// A node pattern's labels and inline properties, resolved once per row
/// instead of once per candidate node.
struct NodeCheck {
    /// `None` when a required label does not exist: nothing can match.
    labels: Option<Vec<LabelId>>,
    props: Option<BTreeMap<String, Val>>,
}

impl NodeCheck {
    fn new(env: &Env<'_>, np: &NodePattern, row: &Row) -> Result<Self> {
        let labels = np.labels.iter().map(|l| env.graph.label_id(l)).collect::<Option<Vec<_>>>();
        Ok(NodeCheck { labels, props: eval_props(env, &np.properties, row)? })
    }

    fn never(&self) -> bool {
        self.labels.is_none()
    }

    fn matches(&self, g: &Graph, nid: NodeId) -> bool {
        let Some(node) = g.node(nid) else { return false };
        let Some(labels) = &self.labels else { return false };
        labels.iter().all(|l| node.labels.contains(l))
            && self.props.as_ref().is_none_or(|m| props_equal(m, &node.properties))
    }
}
