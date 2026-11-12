//! Shortest-path searches between two nodes.
//!
//! The searches know nothing about patterns or rows: the caller passes a
//! function that appends the relationships a node may follow (already
//! filtered by direction, type, properties and relationship uniqueness) to a
//! buffer the search reuses. `backward` asks for the relationships followed
//! against the pattern's direction, used by the half of a bidirectional
//! search that starts at the end node.
//!
//! - [`breadth_first`]: fewest relationships, one path or all of them.
//! - [`bidirectional_bfs`]: fewest relationships, one path, from both ends.
//! - [`bidirectional_dijkstra`]: smallest total weight (non-negative), one
//!   path, from both ends.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use mygraph_core::{IdMap, IdSet, NodeId, RelId, Result};

/// A path as its nodes and the relationships between them
/// (`nodes.len() == rels.len() + 1`).
pub(crate) type Path = (Vec<NodeId>, Vec<RelId>);

/// A relationship a weighted search may follow: (relationship, neighbour, weight).
pub(crate) type WeightedEdge = (RelId, NodeId, f64);

/// Breadth-first search from `s` to `e`, at most `max` relationships long.
/// Returns one shortest path, or all of them, in adjacency order.
pub(crate) fn breadth_first(
    s: NodeId,
    e: NodeId,
    max: u32,
    all: bool,
    mut edges: impl FnMut(NodeId, &mut Vec<(RelId, NodeId)>),
) -> Vec<Path> {
    // node -> (depth, [(relationship, parent)])
    let mut seen: IdMap<NodeId, (u32, Vec<(RelId, NodeId)>)> = IdMap::default();
    seen.insert(s, (0, Vec::new()));
    let mut buf = Vec::new();
    let mut frontier = vec![s];
    let mut depth = 0;
    while !frontier.is_empty() && depth < max && !seen.contains_key(&e) {
        depth += 1;
        let mut next = Vec::new();
        for u in frontier {
            buf.clear();
            edges(u, &mut buf);
            for &(rid, v) in &buf {
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

/// One shortest path (fewest relationships, at most `max`), searching from
/// both ends and always growing the smaller frontier by a full level. Visits
/// far fewer nodes than a one-sided search on large graphs.
pub(crate) fn bidirectional_bfs(
    s: NodeId,
    e: NodeId,
    max: u32,
    mut edges: impl FnMut(NodeId, bool, &mut Vec<(RelId, NodeId)>),
) -> Option<Path> {
    // node -> (relationship, neighbour one step closer to that side's root)
    let mut seen: [IdMap<NodeId, Option<(RelId, NodeId)>>; 2] = [IdMap::default(), IdMap::default()];
    seen[0].insert(s, None);
    seen[1].insert(e, None);
    let mut buf = Vec::new();
    let mut frontier = [vec![s], vec![e]];
    let mut depth = 0u32;
    while depth < max && !frontier[0].is_empty() && !frontier[1].is_empty() {
        let side = usize::from(frontier[1].len() < frontier[0].len());
        let mut next = Vec::new();
        let mut meet: Option<NodeId> = None;
        for u in std::mem::take(&mut frontier[side]) {
            buf.clear();
            edges(u, side == 1, &mut buf);
            for &(rid, v) in &buf {
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
            return Some(join(m, |side, n| seen[side].get(&n).copied().flatten()));
        }
        frontier[side] = next;
    }
    None
}

/// Per side of a weighted search: node -> (distance from that side's root,
/// (relationship, neighbour one step closer to the root)).
type Distances = IdMap<NodeId, (f64, Option<(RelId, NodeId)>)>;

/// A heap entry ordered so that `BinaryHeap` (a max-heap) pops the smallest
/// distance first. Ties pop the smaller node id first, for stable results.
#[derive(Clone, Copy, PartialEq)]
struct Entry(f64, NodeId);

impl Eq for Entry {}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0).then_with(|| other.1.cmp(&self.1))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One path of smallest total weight from `s` to `e`, with that weight.
/// `edges` appends `(relationship, neighbour, weight)`; weights must be
/// non-negative and finite (the caller validates them).
///
/// Two Dijkstra searches, one from each end, each step settling the side
/// whose closest unsettled node is nearer. Every relaxed relationship that
/// reaches a node the other side has seen proposes a path; the search stops
/// once the two closest unsettled distances add up to at least the best path
/// found, which no path through an unsettled node can beat.
pub(crate) fn bidirectional_dijkstra(
    s: NodeId,
    e: NodeId,
    mut edges: impl FnMut(NodeId, bool, &mut Vec<WeightedEdge>) -> Result<()>,
) -> Result<Option<(Path, f64)>> {
    if s == e {
        return Ok(Some(((vec![s], Vec::new()), 0.0)));
    }
    let mut dist: [Distances; 2] = [Distances::default(), Distances::default()];
    dist[0].insert(s, (0.0, None));
    dist[1].insert(e, (0.0, None));
    let mut settled: [IdSet<NodeId>; 2] = [IdSet::default(), IdSet::default()];
    let mut buf = Vec::new();
    let mut heap = [BinaryHeap::from([Entry(0.0, s)]), BinaryHeap::from([Entry(0.0, e)])];
    let mut best = f64::INFINITY;
    let mut meet: Option<NodeId> = None;
    loop {
        let top = |h: &BinaryHeap<Entry>| h.peek().map_or(f64::INFINITY, |x| x.0);
        let (forward, backward) = (top(&heap[0]), top(&heap[1]));
        // also stops when a side is exhausted: it has then seen every node it
        // can reach, so any path would already have been proposed
        if forward + backward >= best {
            break;
        }
        let side = usize::from(backward < forward);
        let Entry(d, u) = heap[side].pop().expect("its top is finite");
        if !settled[side].insert(u) {
            continue; // an outdated entry: `u` was reached more cheaply since
        }
        buf.clear();
        edges(u, side == 1, &mut buf)?;
        for &(rid, v, w) in &buf {
            let nd = d + w;
            let known = dist[side].get(&v).map_or(f64::INFINITY, |x| x.0);
            if nd < known {
                dist[side].insert(v, (nd, Some((rid, u))));
                heap[side].push(Entry(nd, v));
            }
            if let Some((other, _)) = dist[1 - side].get(&v) {
                let total = dist[side][&v].0 + other;
                if total < best {
                    best = total;
                    meet = Some(v);
                }
            }
        }
    }
    Ok(meet.map(|m| {
        let path = join(m, |side, n| dist[side].get(&n).and_then(|x| x.1));
        (without_cycles(path), best)
    }))
}

/// Joins the two halves of a bidirectional search at `m`. `parent(side, n)`
/// gives the relationship and node one step closer to that side's root.
fn join(m: NodeId, parent: impl Fn(usize, NodeId) -> Option<(RelId, NodeId)>) -> Path {
    let walk = |side: usize| {
        let (mut nodes, mut rels, mut cur) = (Vec::new(), Vec::new(), m);
        while let Some((rid, prev)) = parent(side, cur) {
            rels.push(rid);
            nodes.push(prev);
            cur = prev;
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
    (nodes, rels)
}

/// Cuts out any part of `path` that returns to a node it already visited.
/// The two halves of a weighted search can overlap through relationships of
/// weight 0; the cut-out cycle then weighs 0, so the path stays shortest, and
/// no relationship is used twice.
fn without_cycles((nodes, rels): Path) -> Path {
    let mut at: IdMap<NodeId, usize> = IdMap::default();
    let (mut out_nodes, mut out_rels) = (Vec::with_capacity(nodes.len()), Vec::with_capacity(rels.len()));
    for (i, n) in nodes.into_iter().enumerate() {
        if let Some(&j) = at.get(&n) {
            for dropped in out_nodes.drain(j + 1..) {
                at.remove(&dropped);
            }
            out_rels.truncate(j);
            continue;
        }
        if i > 0 {
            out_rels.push(rels[i - 1]);
        }
        at.insert(n, out_nodes.len());
        out_nodes.push(n);
    }
    (out_nodes, out_rels)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directed multigraph: (relationship id = index, from, to, weight).
    struct G(Vec<(NodeId, NodeId, f64)>);

    impl G {
        fn edges(&self, u: NodeId, backward: bool) -> Vec<WeightedEdge> {
            self.0
                .iter()
                .enumerate()
                .filter_map(|(i, &(a, b, w))| match backward {
                    false if a == u => Some((i as RelId, b, w)),
                    true if b == u => Some((i as RelId, a, w)),
                    _ => None,
                })
                .collect()
        }

        fn weighted(&self) -> impl FnMut(NodeId, bool, &mut Vec<WeightedEdge>) -> Result<()> + '_ {
            |u, backward, out| {
                out.extend(self.edges(u, backward));
                Ok(())
            }
        }

        /// Reference single-source Dijkstra over every node, O(V^2).
        fn distance(&self, s: NodeId, e: NodeId, n: usize) -> Option<f64> {
            let mut dist = vec![f64::INFINITY; n];
            let mut done = vec![false; n];
            dist[s as usize] = 0.0;
            for _ in 0..n {
                let u = (0..n).filter(|&i| !done[i]).min_by(|&a, &b| dist[a].total_cmp(&dist[b]))?;
                if dist[u].is_infinite() {
                    break;
                }
                done[u] = true;
                for &(a, b, w) in &self.0 {
                    if a as usize == u && dist[u] + w < dist[b as usize] {
                        dist[b as usize] = dist[u] + w;
                    }
                }
            }
            dist[e as usize].is_finite().then_some(dist[e as usize])
        }

        /// Checks that `path` is a real s→e path of weight `cost`, with no
        /// relationship used twice.
        fn check(&self, s: NodeId, e: NodeId, (nodes, rels): &Path, cost: f64) {
            assert_eq!(nodes.first(), Some(&s));
            assert_eq!(nodes.last(), Some(&e));
            assert_eq!(nodes.len(), rels.len() + 1);
            let mut sum = 0.0;
            for (i, r) in rels.iter().enumerate() {
                let (a, b, w) = self.0[*r as usize];
                assert_eq!((a, b), (nodes[i], nodes[i + 1]), "relationship {r} does not join the path");
                sum += w;
            }
            assert!((sum - cost).abs() < 1e-9, "path weighs {sum}, reported {cost}");
            let unique: std::collections::HashSet<_> = rels.iter().collect();
            assert_eq!(unique.len(), rels.len(), "a relationship is used twice");
        }
    }

    fn rng(seed: u64) -> impl FnMut(u64) -> u64 {
        let mut x = seed;
        move |n| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % n
        }
    }

    #[test]
    fn dijkstra_matches_a_reference_on_random_graphs() {
        let mut next = rng(0x9E37_79B9_7F4A_7C15);
        for round in 0..200 {
            let n = 2 + next(40) as usize;
            let m = next(4 * n as u64) as usize;
            // weights include 0 and repeated values, to exercise ties
            let g = G((0..m).map(|_| (next(n as u64), next(n as u64), next(6) as f64)).collect());
            for _ in 0..10 {
                let (s, e) = (next(n as u64), next(n as u64));
                let found = bidirectional_dijkstra(s, e, g.weighted()).unwrap();
                match (found, g.distance(s, e, n)) {
                    (Some((path, cost)), Some(expected)) => {
                        assert_eq!(cost, expected, "round {round}: {s} -> {e}");
                        g.check(s, e, &path, cost);
                    }
                    (None, None) => {}
                    (found, expected) => panic!("round {round}: {s} -> {e}: found {found:?}, expected {expected:?}"),
                }
            }
        }
    }

    #[test]
    fn dijkstra_prefers_lighter_longer_paths() {
        // 0 -> 3 directly weighs 10; 0 -> 1 -> 2 -> 3 weighs 3
        let g = G(vec![(0, 3, 10.0), (0, 1, 1.0), (1, 2, 1.0), (2, 3, 1.0)]);
        let (path, cost) = bidirectional_dijkstra(0, 3, g.weighted()).unwrap().unwrap();
        assert_eq!(cost, 3.0);
        assert_eq!(path, (vec![0, 1, 2, 3], vec![1, 2, 3]));
        assert!(bidirectional_dijkstra(3, 0, g.weighted()).unwrap().is_none());
        assert_eq!(bidirectional_dijkstra(2, 2, g.weighted()).unwrap(), Some(((vec![2], vec![]), 0.0)));
    }

    #[test]
    fn dijkstra_reports_edge_errors() {
        let err = bidirectional_dijkstra(0, 1, |_, _, _| Err(mygraph_core::MyGraphError::Type("bad weight".into())));
        assert!(err.is_err());
    }

    #[test]
    fn cycles_through_zero_weights_are_cut() {
        assert_eq!(without_cycles((vec![0, 1, 2, 1, 3], vec![10, 11, 12, 13])), (vec![0, 1, 3], vec![10, 13]));
        assert_eq!(without_cycles((vec![0, 1, 0, 2], vec![10, 11, 12])), (vec![0, 2], vec![12]));
        assert_eq!(without_cycles((vec![5], vec![])), (vec![5], vec![]));
    }

    #[test]
    fn bidirectional_bfs_agrees_with_breadth_first() {
        let mut next = rng(0x2545_F491_4F6C_DD1D);
        for _ in 0..200 {
            let n = 2 + next(30) as usize;
            let g = G((0..next(3 * n as u64)).map(|_| (next(n as u64), next(n as u64), 1.0)).collect());
            let plain = |u, out: &mut Vec<_>| out.extend(g.edges(u, false).into_iter().map(|(r, v, _)| (r, v)));
            let both = |u, b, out: &mut Vec<_>| out.extend(g.edges(u, b).into_iter().map(|(r, v, _)| (r, v)));
            let (s, e) = (next(n as u64), next(n as u64));
            if s == e {
                continue;
            }
            let one = breadth_first(s, e, u32::MAX, false, plain);
            let two = bidirectional_bfs(s, e, u32::MAX, both);
            assert_eq!(one.first().map(|p| p.1.len()), two.as_ref().map(|p| p.1.len()), "{s} -> {e}");
        }
    }
}
