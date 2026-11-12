//! Cache of shortest-path search results, one per graph.
//!
//! An entry maps a search (end nodes, direction, relationship types, length
//! bounds, weight property) to the paths it found, possibly none. Entries are
//! only dropped when a change could alter their answer:
//!
//! - creating a relationship drops the entries whose types admit it: it may
//!   open a shorter path, or a path where there was none;
//! - deleting a relationship drops the entries whose paths use it (no other
//!   answer can improve, and "no path" stays true);
//! - raising a relationship's weight drops the weighted entries whose paths
//!   use it, for that weight property;
//! - lowering a weight, or making it missing or invalid, drops every
//!   weighted entry for that property whose types admit the relationship.
//!
//! Node, label and index changes never matter: a shortest-path search only
//! constrains its two end nodes, which are part of the key. Rollbacks apply
//! inverse changes, so they invalidate the same way.
//!
//! Creations and weight decreases are tracked with counters (so they cost
//! O(1), not a scan of the cache); deletions and weight increases drop the
//! affected entries through an index from relationship to entries.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::graph::{IdMap, NodeId, RelId, RelTypeId};
use crate::value::Value;

/// What a cached search was asked, and so what its result depends on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PathKey {
    pub start: NodeId,
    pub end: NodeId,
    /// Follows relationships leaving / entering the current node.
    pub outgoing: bool,
    pub incoming: bool,
    /// Allowed relationship types, sorted; empty means any.
    pub types: Vec<RelTypeId>,
    pub min: u32,
    pub max: Option<u32>,
    /// Every shortest path, rather than one.
    pub all: bool,
    /// Property minimised by a weighted search; `None` counts relationships.
    pub weight: Option<String>,
}

/// A search result: paths as (nodes, relationships).
pub type CachedPaths = Vec<(Vec<NodeId>, Vec<RelId>)>;

#[derive(Debug)]
struct Entry {
    key: PathKey,
    paths: CachedPaths,
    /// Change counter when the search ran.
    computed_at: u64,
    /// For least-recently-used eviction.
    last_used: u64,
}

/// See the module documentation.
#[derive(Debug)]
pub struct PathCache {
    capacity: usize,
    slots: IdMap<u64, Entry>,
    by_key: HashMap<PathKey, u64>,
    /// Relationship -> slots whose paths use it.
    by_rel: IdMap<RelId, Vec<u64>>,
    next_slot: u64,
    /// Counts changes that matter to some entry; entries remember its value.
    clock: u64,
    /// When a relationship of each type (or of any type) was last created.
    created: IdMap<RelTypeId, u64>,
    created_any: u64,
    /// When a weight property last decreased, per type and for any type.
    lighter: HashMap<String, (IdMap<RelTypeId, u64>, u64)>,
    tick: u64,
    hits: u64,
    misses: u64,
}

/// Default number of cached searches per graph.
pub const DEFAULT_PATH_CACHE_ENTRIES: usize = 10_000;

impl Default for PathCache {
    fn default() -> Self {
        PathCache::new(DEFAULT_PATH_CACHE_ENTRIES)
    }
}

impl Clone for PathCache {
    /// A clone starts empty: cached answers belong to the original graph's history.
    fn clone(&self) -> Self {
        PathCache::new(self.capacity)
    }
}

impl PathCache {
    /// A cache holding at most `capacity` searches; 0 disables it.
    pub fn new(capacity: usize) -> Self {
        PathCache {
            capacity,
            slots: IdMap::default(),
            by_key: HashMap::new(),
            by_rel: IdMap::default(),
            next_slot: 0,
            clock: 0,
            created: IdMap::default(),
            created_any: 0,
            lighter: HashMap::new(),
            tick: 0,
            hits: 0,
            misses: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Changes the capacity, evicting entries if needed.
    pub fn set_capacity(&mut self, capacity: usize) {
        if capacity == self.capacity {
            return;
        }
        self.capacity = capacity;
        while self.slots.len() > capacity {
            self.evict();
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// (hits, misses) since the cache was created.
    pub fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }

    /// The cached answer for `key`, if it is still valid.
    pub fn get(&mut self, key: &PathKey) -> Option<CachedPaths> {
        let Some(&slot) = self.by_key.get(key) else {
            self.misses += 1;
            return None;
        };
        let computed_at = self.slots[&slot].computed_at;
        if self.changed_since(key, computed_at) {
            self.remove(slot);
            self.misses += 1;
            return None;
        }
        self.tick += 1;
        let entry = self.slots.get_mut(&slot).expect("indexed");
        entry.last_used = self.tick;
        self.hits += 1;
        Some(entry.paths.clone())
    }

    /// Stores the answer to `key`, computed against the current graph.
    pub fn insert(&mut self, key: PathKey, paths: CachedPaths) {
        if self.capacity == 0 {
            return;
        }
        if let Some(old) = self.by_key.get(&key).copied() {
            self.remove(old);
        }
        while self.slots.len() >= self.capacity {
            self.evict();
        }
        let slot = self.next_slot;
        self.next_slot += 1;
        let mut rels: Vec<RelId> = paths.iter().flat_map(|(_, r)| r.iter().copied()).collect();
        rels.sort_unstable();
        rels.dedup();
        for r in rels {
            self.by_rel.entry(r).or_default().push(slot);
        }
        self.tick += 1;
        self.by_key.insert(key.clone(), slot);
        self.slots.insert(slot, Entry { key, paths, computed_at: self.clock, last_used: self.tick });
    }

    /// Whether a creation or a weight decrease since `at` may change `key`'s answer.
    fn changed_since(&self, key: &PathKey, at: u64) -> bool {
        let created = if key.types.is_empty() {
            self.created_any
        } else {
            key.types.iter().map(|t| self.created.get(t).copied().unwrap_or(0)).max().unwrap_or(0)
        };
        if created > at {
            return true;
        }
        let Some(w) = &key.weight else { return false };
        let Some((by_type, any)) = self.lighter.get(w) else { return false };
        let lighter = if key.types.is_empty() {
            *any
        } else {
            key.types.iter().map(|t| by_type.get(t).copied().unwrap_or(0)).max().unwrap_or(0)
        };
        lighter > at
    }

    fn remove(&mut self, slot: u64) {
        let Some(entry) = self.slots.remove(&slot) else { return };
        self.by_key.remove(&entry.key);
        for r in entry.paths.iter().flat_map(|(_, rels)| rels) {
            if let Some(list) = self.by_rel.get_mut(r) {
                list.retain(|s| *s != slot);
                if list.is_empty() {
                    self.by_rel.remove(r);
                }
            }
        }
    }

    /// Drops the least recently used tenth of the entries (at least one), so
    /// a full cache does not pay for a scan on every insert.
    fn evict(&mut self) {
        let mut ages: Vec<(u64, u64)> = self.slots.iter().map(|(s, e)| (e.last_used, *s)).collect();
        let n = (ages.len() / 10).max(1);
        ages.select_nth_unstable(n - 1);
        for (_, slot) in ages.into_iter().take(n) {
            self.remove(slot);
        }
    }

    fn drop_using(&mut self, rel: RelId, weight: Option<&str>) {
        let Some(slots) = self.by_rel.get(&rel).cloned() else { return };
        for slot in slots {
            let affected = match weight {
                None => true,
                Some(w) => self.slots.get(&slot).is_some_and(|e| e.key.weight.as_deref() == Some(w)),
            };
            if affected {
                self.remove(slot);
            }
        }
    }

    // ---- notifications from Graph::apply ----

    pub(crate) fn relationship_created(&mut self, rel_type: RelTypeId) {
        if self.slots.is_empty() {
            return;
        }
        self.clock += 1;
        self.created.insert(rel_type, self.clock);
        self.created_any = self.clock;
    }

    pub(crate) fn relationship_deleted(&mut self, rel: RelId) {
        self.drop_using(rel, None);
    }

    /// Property `key` of relationship `rel` (of type `rel_type`) went from
    /// `old` to `new` (`None`: absent).
    pub(crate) fn relationship_property_set(
        &mut self,
        rel: RelId,
        rel_type: RelTypeId,
        key: &str,
        old: Option<&Value>,
        new: Option<&Value>,
    ) {
        if self.slots.is_empty() {
            return;
        }
        let weight = |v: Option<&Value>| match v {
            Some(Value::Integer(i)) if *i >= 0 => Some(*i as f64),
            Some(Value::Float(f)) if f.is_finite() && *f >= 0.0 => Some(*f),
            _ => None,
        };
        match (weight(old), weight(new)) {
            (Some(o), Some(n)) if n == o => {}
            // only paths through `rel` can get worse
            (Some(o), Some(n)) if n > o => self.drop_using(rel, Some(key)),
            // lighter, negative, missing or not a number: any weighted answer may change
            _ => {
                self.clock += 1;
                let (by_type, any) = self.lighter.entry(key.to_string()).or_default();
                by_type.insert(rel_type, self.clock);
                *any = self.clock;
            }
        }
    }
}

/// A [`PathCache`] a graph can update while queries only hold `&Graph`.
#[derive(Debug)]
pub struct SharedPathCache {
    cache: Mutex<PathCache>,
    /// Copy of the capacity, so applying the configured size on every
    /// statement does not lock the cache.
    capacity: AtomicUsize,
}

impl Default for SharedPathCache {
    fn default() -> Self {
        SharedPathCache {
            cache: Mutex::new(PathCache::default()),
            capacity: AtomicUsize::new(DEFAULT_PATH_CACHE_ENTRIES),
        }
    }
}

impl Clone for SharedPathCache {
    fn clone(&self) -> Self {
        let cache = self.lock().clone();
        SharedPathCache { capacity: AtomicUsize::new(cache.capacity()), cache: Mutex::new(cache) }
    }
}

impl SharedPathCache {
    /// Applies a capacity; only locks when it changes.
    pub fn set_capacity(&self, entries: usize) {
        if self.capacity.load(Ordering::Relaxed) != entries {
            let mut cache = self.lock();
            cache.set_capacity(entries);
            self.capacity.store(entries, Ordering::Relaxed);
        }
    }

    /// The cache stays usable after a panic elsewhere: every operation leaves
    /// it consistent, and a wrong answer is impossible because entries are
    /// only ever dropped.
    pub fn lock(&self) -> MutexGuard<'_, PathCache> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Direct access when the graph is borrowed mutably (no locking).
    pub fn get_mut(&mut self) -> &mut PathCache {
        self.cache.get_mut().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(start: NodeId, types: Vec<RelTypeId>, weight: Option<&str>) -> PathKey {
        PathKey {
            start,
            end: 99,
            outgoing: true,
            incoming: false,
            types,
            min: 1,
            max: None,
            all: false,
            weight: weight.map(str::to_string),
        }
    }

    fn path(rels: &[RelId]) -> CachedPaths {
        vec![((0..=rels.len() as u64).collect(), rels.to_vec())]
    }

    #[test]
    fn creations_invalidate_matching_types_only() {
        let mut c = PathCache::new(10);
        c.insert(key(1, vec![], None), path(&[10]));
        c.insert(key(2, vec![7], None), path(&[11]));
        c.insert(key(3, vec![8], None), Vec::new()); // no path
        c.relationship_created(8);
        assert!(c.get(&key(1, vec![], None)).is_none(), "any type admits the new relationship");
        assert!(c.get(&key(2, vec![7], None)).is_some());
        assert!(c.get(&key(3, vec![8], None)).is_none(), "a new relationship may open a path");
        // entries computed after the creation are valid
        c.insert(key(3, vec![8], None), Vec::new());
        assert_eq!(c.get(&key(3, vec![8], None)), Some(Vec::new()));
    }

    #[test]
    fn deletions_invalidate_paths_through_the_relationship() {
        let mut c = PathCache::new(10);
        c.insert(key(1, vec![], None), path(&[10, 11]));
        c.insert(key(2, vec![], None), path(&[12]));
        c.insert(key(3, vec![], None), Vec::new());
        c.relationship_deleted(11);
        assert!(c.get(&key(1, vec![], None)).is_none());
        assert!(c.get(&key(2, vec![], None)).is_some());
        assert_eq!(c.get(&key(3, vec![], None)), Some(Vec::new()), "no path stays no path");
        assert!(!c.by_rel.contains_key(&10), "the removed entry leaves no index behind");
    }

    #[test]
    fn weight_changes() {
        let mut c = PathCache::new(10);
        let cost = |s| key(s, vec![], Some("cost"));
        c.insert(cost(1), path(&[10]));
        c.insert(cost(2), path(&[20]));
        c.insert(key(3, vec![], None), path(&[10]));
        c.insert(key(4, vec![], Some("time")), path(&[10]));
        let (one, three) = (Value::Integer(1), Value::Integer(3));

        // heavier: only weighted paths through it, for that property
        c.relationship_property_set(10, 0, "cost", Some(&one), Some(&three));
        assert!(c.get(&cost(1)).is_none());
        assert!(c.get(&cost(2)).is_some());
        assert!(c.get(&key(3, vec![], None)).is_some(), "hop counts ignore weights");
        assert!(c.get(&key(4, vec![], Some("time"))).is_some(), "another property");

        // lighter: every entry weighted by that property
        c.insert(cost(1), path(&[10]));
        c.relationship_property_set(30, 0, "cost", Some(&three), Some(&one));
        assert!(c.get(&cost(1)).is_none());
        assert!(c.get(&cost(2)).is_none());
        assert!(c.get(&key(4, vec![], Some("time"))).is_some());

        // removed or invalid counts as lighter
        c.insert(cost(2), path(&[20]));
        c.relationship_property_set(20, 0, "cost", Some(&one), Some(&Value::from("x")));
        assert!(c.get(&cost(2)).is_none());
    }

    #[test]
    fn least_recently_used_entries_are_evicted() {
        let mut c = PathCache::new(3);
        for s in 0..3 {
            c.insert(key(s, vec![], None), path(&[s]));
        }
        assert!(c.get(&key(0, vec![], None)).is_some()); // 0 is now the most recent
        c.insert(key(3, vec![], None), path(&[3]));
        assert_eq!(c.len(), 3);
        assert!(c.get(&key(1, vec![], None)).is_none(), "1 was the least recently used");
        assert!(c.get(&key(0, vec![], None)).is_some());
        assert_eq!(c.stats(), (2, 1));
        c.set_capacity(0);
        assert!(c.is_empty());
        c.insert(key(5, vec![], None), path(&[5]));
        assert!(c.is_empty(), "capacity 0 disables the cache");
    }
}
