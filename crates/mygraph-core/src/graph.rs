//! In-memory property graph for a single schema.
//!
//! All mutations go through [`Graph::apply`], which takes a [`Change`] and
//! returns its inverse. The same `Change` records serve as redo entries for the
//! WAL and as undo entries for rollback.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

use crate::error::{MyGraphError, Result};
use crate::path_cache::{CachedPaths, PathCache, PathKey, SharedPathCache};
use crate::props::{KeyInterner, Properties};
use crate::value::{NodeValue, PropertyMap, RelationshipValue, Value};

pub type NodeId = u64;
pub type RelId = u64;
pub type LabelId = u32;
pub type RelTypeId = u32;
pub type SchemaId = u32;

/// Hasher for integer ids. Ids are allocated sequentially, so a single
/// multiplication spreads them well and is far cheaper than SipHash. Not
/// meant for keys an attacker chooses.
#[derive(Debug, Clone, Copy, Default)]
pub struct IdHasher(u64);

impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(*b)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }

    fn write_u32(&mut self, n: u32) {
        self.write_u64(u64::from(n));
    }
}

/// A hash map keyed by node or relationship ids.
pub type IdMap<K, V> = HashMap<K, V, BuildHasherDefault<IdHasher>>;
/// A hash set of node or relationship ids.
pub type IdSet<K> = HashSet<K, BuildHasherDefault<IdHasher>>;

/// Maps label / relationship-type names to compact ids.
#[derive(Debug, Clone, Default)]
pub struct Interner {
    names: Vec<String>,
    ids: HashMap<String, u32>,
}

impl Interner {
    pub fn intern(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(name.to_string());
        self.ids.insert(name.to_string(), id);
        id
    }

    pub fn get(&self, name: &str) -> Option<u32> {
        self.ids.get(name).copied()
    }

    pub fn name(&self, id: u32) -> &str {
        &self.names[id as usize]
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub schema_id: SchemaId,
    pub labels: Vec<LabelId>,
    pub properties: Properties,
    pub created_at: u64,
    pub updated_at: u64,
    outgoing: Vec<Adjacent>,
    incoming: Vec<Adjacent>,
}

/// One entry of a node's adjacency list. It repeats the relationship's type
/// and the node at its other end, so traversals can filter by type and move
/// to the neighbour without looking the relationship up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Adjacent {
    pub rel: RelId,
    /// The node at the other end (the node itself for a self-loop).
    pub other: NodeId,
    pub rel_type: RelTypeId,
}

impl Node {
    /// Relationships starting at this node, in creation order.
    pub fn outgoing(&self) -> &[Adjacent] {
        &self.outgoing
    }

    /// Relationships ending at this node, in creation order.
    pub fn incoming(&self) -> &[Adjacent] {
        &self.incoming
    }

    pub fn degree(&self) -> usize {
        self.outgoing.len() + self.incoming.len()
    }
}

#[derive(Debug, Clone)]
pub struct Relationship {
    pub id: RelId,
    pub schema_id: SchemaId,
    pub start_node: NodeId,
    pub end_node: NodeId,
    pub rel_type: RelTypeId,
    pub properties: Properties,
}

/// An atomic, invertible graph mutation. Names (not interned ids) are used so
/// records stay meaningful outside the in-memory graph (WAL, replication).
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    CreateNode {
        id: NodeId,
        labels: Vec<String>,
        properties: PropertyMap,
        created_at: u64,
        updated_at: u64,
    },
    DeleteNode {
        id: NodeId,
    },
    CreateRelationship {
        id: RelId,
        start: NodeId,
        end: NodeId,
        rel_type: String,
        properties: PropertyMap,
    },
    DeleteRelationship {
        id: RelId,
    },
    /// `value: None` removes the property. `ts` becomes the node's `updated_at`.
    SetNodeProperty {
        id: NodeId,
        key: String,
        value: Option<Value>,
        ts: u64,
    },
    SetRelationshipProperty {
        id: RelId,
        key: String,
        value: Option<Value>,
    },
    AddLabel {
        id: NodeId,
        label: String,
        ts: u64,
    },
    RemoveLabel {
        id: NodeId,
        label: String,
        ts: u64,
    },
    /// Creates an equality index on `(label, key)` and fills it from existing nodes.
    CreateIndex {
        name: String,
        label: String,
        key: String,
    },
    DropIndex {
        label: String,
        key: String,
    },
    /// Creates a constraint on `(label, key)` after checking the existing nodes.
    /// A `Unique` constraint needs a property index on the same pair.
    CreateConstraint {
        name: String,
        label: String,
        key: String,
        kind: ConstraintKind,
        /// The index was created for this constraint and is dropped with it.
        owns_index: bool,
    },
    DropConstraint {
        name: String,
    },
}

/// What a constraint requires of the nodes carrying its label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConstraintKind {
    /// No two nodes share a value of the property.
    Unique,
    /// Every node has the property.
    NotNull,
}

impl ConstraintKind {
    /// Name used by `SHOW CONSTRAINTS`.
    pub fn as_str(self) -> &'static str {
        match self {
            ConstraintKind::Unique => "UNIQUE",
            ConstraintKind::NotNull => "NOT NULL",
        }
    }
}

/// A constraint, as listed by `SHOW CONSTRAINTS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintInfo {
    pub name: String,
    pub label: String,
    pub key: String,
    pub kind: ConstraintKind,
    pub owns_index: bool,
}

/// Hashable form of an indexable property value. Values that are equal under
/// Cypher equality map to the same key (`1` and `1.0` included). Lists, maps,
/// byte arrays and NaN are not indexed; lookups with them fall back to a scan.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IndexKey {
    Bool(bool),
    Int(i64),
    Float(u64),
    Str(String),
}

impl IndexKey {
    pub fn from_value(v: &Value) -> Option<IndexKey> {
        Some(match v {
            Value::Boolean(b) => IndexKey::Bool(*b),
            Value::Integer(i) => IndexKey::Int(*i),
            Value::Float(f) => Self::from_float(*f)?,
            Value::String(s) => IndexKey::Str(s.clone()),
            _ => return None,
        })
    }

    pub fn from_float(f: f64) -> Option<IndexKey> {
        if f.is_nan() {
            return None;
        }
        // i64::MIN as f64 is exact; i64::MAX as f64 rounds up to 2^63, which is out of range
        if f.fract() == 0.0 && f >= i64::MIN as f64 && f < i64::MAX as f64 {
            return Some(IndexKey::Int(f as i64));
        }
        Some(IndexKey::Float(f.to_bits()))
    }
}

/// The nodes holding one indexed value. Most indexed keys are unique, so a
/// single id is stored inline instead of in a `BTreeSet` (one tree node each).
#[derive(Debug, Clone)]
enum Postings {
    One(NodeId),
    Many(BTreeSet<NodeId>),
}

impl Postings {
    fn insert(&mut self, id: NodeId) {
        match self {
            Postings::One(x) if *x == id => {}
            Postings::One(x) => *self = Postings::Many(BTreeSet::from([*x, id])),
            Postings::Many(set) => {
                set.insert(id);
            }
        }
    }

    /// Removes `id`; returns `true` when no id is left.
    fn remove(&mut self, id: NodeId) -> bool {
        match self {
            Postings::One(x) => *x == id,
            Postings::Many(set) => {
                set.remove(&id);
                match set.len() {
                    0 => true,
                    1 => {
                        *self = Postings::One(*set.first().expect("len 1"));
                        false
                    }
                    _ => false,
                }
            }
        }
    }

    fn len(&self) -> usize {
        match self {
            Postings::One(_) => 1,
            Postings::Many(set) => set.len(),
        }
    }

    /// Ids in ascending order.
    fn to_vec(&self) -> Vec<NodeId> {
        match self {
            Postings::One(x) => vec![*x],
            Postings::Many(set) => set.iter().copied().collect(),
        }
    }
}

/// An equality index over one `(label, property)` pair.
#[derive(Debug, Clone, Default)]
struct PropIndex {
    name: String,
    entries: HashMap<IndexKey, Postings>,
}

impl PropIndex {
    fn insert(&mut self, value: &Value, id: NodeId) {
        if let Some(k) = IndexKey::from_value(value) {
            match self.entries.entry(k) {
                std::collections::hash_map::Entry::Occupied(mut e) => e.get_mut().insert(id),
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(Postings::One(id));
                }
            }
        }
    }

    fn remove(&mut self, value: &Value, id: NodeId) {
        if let Some(k) = IndexKey::from_value(value) {
            if self.entries.get_mut(&k).is_some_and(|p| p.remove(id)) {
                self.entries.remove(&k);
            }
        }
    }
}

/// Description of an index, as listed by `SHOW INDEXES`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexInfo {
    pub name: String,
    pub label: String,
    pub key: String,
    pub entries: usize,
}

#[derive(Debug, Clone)]
pub struct Graph {
    schema_id: SchemaId,
    nodes: IdMap<NodeId, Node>,
    relationships: IdMap<RelId, Relationship>,
    labels: Interner,
    rel_types: Interner,
    prop_keys: KeyInterner,
    label_index: HashMap<LabelId, BTreeSet<NodeId>>,
    /// Relationship ids by type (automatic, like the label index).
    rel_type_index: HashMap<RelTypeId, BTreeSet<RelId>>,
    /// Property indexes, grouped by label so node changes find theirs quickly.
    prop_index: HashMap<LabelId, BTreeMap<String, PropIndex>>,
    /// Constraints by name.
    constraints: BTreeMap<String, ConstraintInfo>,
    /// Results of shortest-path searches, kept valid by [`Graph::apply`].
    path_cache: SharedPathCache,
}

fn node_not_found(id: NodeId) -> MyGraphError {
    MyGraphError::NotFound(format!("node {id}"))
}

fn rel_not_found(id: RelId) -> MyGraphError {
    MyGraphError::NotFound(format!("relationship {id}"))
}

fn check_storable(key: &str, value: &Value) -> Result<()> {
    if value.is_storable() {
        Ok(())
    } else {
        Err(MyGraphError::Type(format!("property `{key}` cannot store a value of type {}", value.type_name())))
    }
}

/// Drops `null` entries (a null property is an absent property) and validates types.
fn clean_properties(props: &PropertyMap) -> Result<PropertyMap> {
    let mut out = PropertyMap::new();
    for (k, v) in props {
        if v.is_null() {
            continue;
        }
        check_storable(k, v)?;
        out.insert(k.clone(), v.clone());
    }
    Ok(out)
}

fn normalize_value(key: &str, value: &Option<Value>) -> Result<Option<Value>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            check_storable(key, v)?;
            Ok(Some(v.clone()))
        }
    }
}

impl Graph {
    pub fn new(schema_id: SchemaId) -> Self {
        Graph {
            schema_id,
            nodes: IdMap::default(),
            relationships: IdMap::default(),
            labels: Interner::default(),
            rel_types: Interner::default(),
            prop_keys: KeyInterner::default(),
            label_index: HashMap::new(),
            rel_type_index: HashMap::new(),
            prop_index: HashMap::new(),
            constraints: BTreeMap::new(),
            path_cache: SharedPathCache::default(),
        }
    }

    /// The cached answer to a shortest-path search, if still valid.
    pub fn cached_paths(&self, key: &PathKey) -> Option<CachedPaths> {
        self.path_cache.lock().get(key)
    }

    /// Caches the answer to a shortest-path search run against this graph
    /// as it is now. Callers must only cache searches fully described by
    /// `key` (no relationship property filters, no excluded relationships).
    pub fn cache_paths(&self, key: PathKey, paths: CachedPaths) {
        self.path_cache.lock().insert(key, paths);
    }

    /// Maximum number of cached shortest-path searches; 0 disables the cache.
    /// Takes `&self` so read-only queries can apply the configured size.
    pub fn set_path_cache_capacity(&self, entries: usize) {
        self.path_cache.set_capacity(entries);
    }

    /// (entries, hits, misses) of the shortest-path cache.
    pub fn path_cache_stats(&self) -> (usize, u64, u64) {
        let c = self.path_cache.lock();
        let (hits, misses) = c.stats();
        (c.len(), hits, misses)
    }

    fn path_cache(&mut self) -> &mut PathCache {
        self.path_cache.get_mut()
    }

    pub fn schema_id(&self) -> SchemaId {
        self.schema_id
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn relationship(&self, id: RelId) -> Option<&Relationship> {
        self.relationships.get(&id)
    }

    /// Every node, in id order. Sorts, so O(n log n): for scans and snapshots.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> + '_ {
        let mut all: Vec<&Node> = self.nodes.values().collect();
        all.sort_unstable_by_key(|n| n.id);
        all.into_iter()
    }

    /// Every relationship, in id order. Sorts, so O(n log n): for scans and
    /// snapshots (replaying them in id order rebuilds the same adjacency).
    pub fn relationships(&self) -> impl Iterator<Item = &Relationship> + '_ {
        let mut all: Vec<&Relationship> = self.relationships.values().collect();
        all.sort_unstable_by_key(|r| r.id);
        all.into_iter()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn relationship_count(&self) -> usize {
        self.relationships.len()
    }

    pub fn label_id(&self, name: &str) -> Option<LabelId> {
        self.labels.get(name)
    }

    pub fn label_name(&self, id: LabelId) -> &str {
        self.labels.name(id)
    }

    pub fn rel_type_id(&self, name: &str) -> Option<RelTypeId> {
        self.rel_types.get(name)
    }

    pub fn rel_type_name(&self, id: RelTypeId) -> &str {
        self.rel_types.name(id)
    }

    /// Ids of the nodes carrying `label`, in ascending order (label index lookup).
    pub fn node_ids_with_label(&self, label: &str) -> Vec<NodeId> {
        self.labels
            .get(label)
            .and_then(|l| self.label_index.get(&l))
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Number of nodes carrying `label`.
    pub fn label_count(&self, label: &str) -> usize {
        self.labels.get(label).and_then(|l| self.label_index.get(&l)).map_or(0, BTreeSet::len)
    }

    /// Ids of the relationships of type `rel_type`, in ascending order.
    pub fn relationship_ids_with_type(&self, rel_type: &str) -> impl Iterator<Item = RelId> + '_ {
        self.rel_types.get(rel_type).and_then(|t| self.rel_type_index.get(&t)).into_iter().flatten().copied()
    }

    /// Number of relationships of type `rel_type`.
    pub fn rel_type_count(&self, rel_type: &str) -> usize {
        self.rel_types.get(rel_type).and_then(|t| self.rel_type_index.get(&t)).map_or(0, BTreeSet::len)
    }

    pub fn has_index(&self, label: &str, key: &str) -> bool {
        self.labels.get(label).and_then(|l| self.prop_index.get(&l)).is_some_and(|m| m.contains_key(key))
    }

    /// Ids of the nodes with `label` whose `key` equals `value`, in ascending
    /// order. `None` when there is no such index or `value` is not indexable,
    /// in which case the caller must scan.
    pub fn nodes_by_property(&self, label: &str, key: &str, value: &Value) -> Option<Vec<NodeId>> {
        let idx = self.labels.get(label).and_then(|l| self.prop_index.get(&l)).and_then(|m| m.get(key))?;
        if value.is_null() {
            return Some(Vec::new()); // `= null` never matches
        }
        // Integers and floats are compared as f64. Beyond 2^53 several integers
        // share one f64, so the hashed key could miss a match: scan instead.
        const EXACT: f64 = 9_007_199_254_740_992.0; // 2^53
        match value {
            Value::Integer(i) if (*i as f64).abs() > EXACT => return None,
            Value::Float(f) if f.abs() >= EXACT => return None,
            _ => {}
        }
        let k = IndexKey::from_value(value)?;
        Some(idx.entries.get(&k).map(Postings::to_vec).unwrap_or_default())
    }

    /// How many ids [`Graph::nodes_by_property`] would return, without
    /// collecting them. `None` in the same cases.
    pub fn count_by_property(&self, label: &str, key: &str, value: &Value) -> Option<usize> {
        let idx = self.labels.get(label).and_then(|l| self.prop_index.get(&l)).and_then(|m| m.get(key))?;
        if value.is_null() {
            return Some(0);
        }
        const EXACT: f64 = 9_007_199_254_740_992.0; // 2^53, see nodes_by_property
        match value {
            Value::Integer(i) if (*i as f64).abs() > EXACT => return None,
            Value::Float(f) if f.abs() >= EXACT => return None,
            _ => {}
        }
        let k = IndexKey::from_value(value)?;
        Some(idx.entries.get(&k).map_or(0, Postings::len))
    }

    /// Every property index, ordered by label then key.
    pub fn indexes(&self) -> Vec<IndexInfo> {
        let mut out: Vec<IndexInfo> = self
            .prop_index
            .iter()
            .flat_map(|(l, m)| {
                m.iter().map(|(k, idx)| IndexInfo {
                    name: idx.name.clone(),
                    label: self.labels.name(*l).to_string(),
                    key: k.clone(),
                    entries: idx.entries.values().map(Postings::len).sum(),
                })
            })
            .collect();
        out.sort_by(|a, b| (&a.label, &a.key).cmp(&(&b.label, &b.key)));
        out
    }

    /// The `(label, key)` of the index called `name`.
    pub fn index_by_name(&self, name: &str) -> Option<(String, String)> {
        self.indexes().into_iter().find(|i| i.name == name).map(|i| (i.label, i.key))
    }

    /// Every constraint, ordered by name.
    pub fn constraints(&self) -> Vec<ConstraintInfo> {
        self.constraints.values().cloned().collect()
    }

    pub fn constraint(&self, name: &str) -> Option<&ConstraintInfo> {
        self.constraints.get(name)
    }

    /// The name of a constraint of `kind` on `(label, key)`, if there is one.
    pub fn constraint_on(&self, label: &str, key: &str, kind: ConstraintKind) -> Option<&str> {
        self.constraints.values().find(|c| c.label == label && c.key == key && c.kind == kind).map(|c| c.name.as_str())
    }

    /// Names of the labels that at least one node carries, sorted.
    pub fn label_names(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .label_index
            .iter()
            .filter(|(_, set)| !set.is_empty())
            .map(|(l, _)| self.labels.name(*l).to_string())
            .collect();
        out.sort();
        out
    }

    /// Names of the relationship types that at least one relationship has, sorted.
    pub fn rel_type_names(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .rel_type_index
            .iter()
            .filter(|(_, set)| !set.is_empty())
            .map(|(t, _)| self.rel_types.name(*t).to_string())
            .collect();
        out.sort();
        out
    }

    /// Every property key ever stored in this graph, sorted.
    pub fn property_keys(&self) -> Vec<String> {
        let mut out: Vec<String> = self.prop_keys.names().map(str::to_string).collect();
        out.sort();
        out
    }

    /// Checks the constraints against the nodes touched by `changes` (the
    /// changes of one statement). Checking after the whole statement lets a
    /// statement swap two unique values.
    pub fn check_constraints(&self, changes: &[Change]) -> Result<()> {
        if self.constraints.is_empty() {
            return Ok(());
        }
        let mut seen: IdSet<NodeId> = IdSet::default();
        for change in changes {
            let id = match change {
                Change::CreateNode { id, .. } | Change::SetNodeProperty { id, .. } | Change::AddLabel { id, .. } => *id,
                _ => continue,
            };
            if !seen.insert(id) {
                continue;
            }
            let Some(node) = self.nodes.get(&id) else { continue };
            for c in self.constraints.values() {
                let Some(lid) = self.labels.get(&c.label) else { continue };
                if node.labels.contains(&lid) {
                    self.check_node(c, lid, id, node.properties.get(&c.key))?;
                }
            }
        }
        Ok(())
    }

    /// Checks one node against one constraint on its label.
    fn check_node(&self, c: &ConstraintInfo, lid: LabelId, id: NodeId, value: Option<&Value>) -> Result<()> {
        let violation = |what: String| {
            Err(MyGraphError::ConstraintViolation(format!(
                "node {id} {what} (constraint `{}`: :{}({}) {})",
                c.name,
                c.label,
                c.key,
                c.kind.as_str()
            )))
        };
        match (c.kind, value) {
            (ConstraintKind::NotNull, None) => violation(format!("has no property `{}`", c.key)),
            (ConstraintKind::NotNull, Some(_)) | (ConstraintKind::Unique, None) => Ok(()),
            (ConstraintKind::Unique, Some(v)) => {
                let Some(k) = IndexKey::from_value(v) else {
                    return violation(format!(
                        "has a {} in `{}`, which a unique constraint cannot check",
                        v.type_name(),
                        c.key
                    ));
                };
                let shared = self
                    .prop_index
                    .get(&lid)
                    .and_then(|m| m.get(&c.key))
                    .and_then(|idx| idx.entries.get(&k))
                    .map_or(0, Postings::len);
                if shared > 1 {
                    return violation(format!("has the value {v} in `{}`, which another node already has", c.key));
                }
                Ok(())
            }
        }
    }

    fn index_node(&mut self, label: LabelId, id: NodeId) {
        let Some(indexes) = self.prop_index.get_mut(&label) else { return };
        let Some(node) = self.nodes.get(&id) else { return };
        for (key, idx) in indexes.iter_mut() {
            if let Some(v) = node.properties.get(key) {
                idx.insert(v, id);
            }
        }
    }

    fn unindex_node(&mut self, label: LabelId, id: NodeId, props: &Properties) {
        let Some(indexes) = self.prop_index.get_mut(&label) else { return };
        for (key, idx) in indexes.iter_mut() {
            if let Some(v) = props.get(key) {
                idx.remove(v, id);
            }
        }
    }

    pub fn node_has_label(&self, node: &Node, label: &str) -> bool {
        self.labels.get(label).is_some_and(|l| node.labels.contains(&l))
    }

    pub fn node_label_names(&self, node: &Node) -> Vec<String> {
        node.labels.iter().map(|l| self.labels.name(*l).to_string()).collect()
    }

    pub fn node_value(&self, id: NodeId) -> Option<NodeValue> {
        self.nodes.get(&id).map(|n| NodeValue {
            id,
            labels: self.node_label_names(n),
            properties: n.properties.to_map(),
        })
    }

    pub fn relationship_value(&self, id: RelId) -> Option<RelationshipValue> {
        self.relationships.get(&id).map(|r| RelationshipValue {
            id,
            start_node: r.start_node,
            end_node: r.end_node,
            rel_type: self.rel_types.name(r.rel_type).to_string(),
            properties: r.properties.to_map(),
        })
    }

    /// Applies a change and returns its inverse, or `None` when the change was a no-op.
    pub fn apply(&mut self, change: &Change) -> Result<Option<Change>> {
        match change {
            Change::CreateNode { id, labels, properties, created_at, updated_at } => {
                if self.nodes.contains_key(id) {
                    return Err(MyGraphError::AlreadyExists(format!("node {id}")));
                }
                let properties = Properties::from_map(clean_properties(properties)?, &mut self.prop_keys);
                let mut label_ids = Vec::with_capacity(labels.len());
                for name in labels {
                    let lid = self.labels.intern(name);
                    if !label_ids.contains(&lid) {
                        label_ids.push(lid);
                        self.label_index.entry(lid).or_default().insert(*id);
                    }
                }
                self.nodes.insert(
                    *id,
                    Node {
                        id: *id,
                        schema_id: self.schema_id,
                        labels: label_ids,
                        properties,
                        created_at: *created_at,
                        updated_at: *updated_at,
                        outgoing: Vec::new(),
                        incoming: Vec::new(),
                    },
                );
                for l in self.nodes[id].labels.clone() {
                    self.index_node(l, *id);
                }
                Ok(Some(Change::DeleteNode { id: *id }))
            }
            Change::DeleteNode { id } => {
                let node = self.nodes.get(id).ok_or_else(|| node_not_found(*id))?;
                if node.degree() > 0 {
                    return Err(MyGraphError::Execution(format!(
                        "cannot delete node {id} because it still has relationships; use DETACH DELETE"
                    )));
                }
                let node = self.nodes.remove(id).expect("checked above");
                for l in &node.labels {
                    if let Some(set) = self.label_index.get_mut(l) {
                        set.remove(id);
                    }
                    self.unindex_node(*l, *id, &node.properties);
                }
                Ok(Some(Change::CreateNode {
                    id: *id,
                    labels: node.labels.iter().map(|l| self.labels.name(*l).to_string()).collect(),
                    properties: node.properties.into_map(),
                    created_at: node.created_at,
                    updated_at: node.updated_at,
                }))
            }
            Change::CreateRelationship { id, start, end, rel_type, properties } => {
                if self.relationships.contains_key(id) {
                    return Err(MyGraphError::AlreadyExists(format!("relationship {id}")));
                }
                if !self.nodes.contains_key(start) {
                    return Err(node_not_found(*start));
                }
                if !self.nodes.contains_key(end) {
                    return Err(node_not_found(*end));
                }
                let properties = Properties::from_map(clean_properties(properties)?, &mut self.prop_keys);
                let type_id = self.rel_types.intern(rel_type);
                let adj = |other| Adjacent { rel: *id, other, rel_type: type_id };
                self.nodes.get_mut(start).expect("checked").outgoing.push(adj(*end));
                self.nodes.get_mut(end).expect("checked").incoming.push(adj(*start));
                self.rel_type_index.entry(type_id).or_default().insert(*id);
                self.path_cache().relationship_created(type_id);
                self.relationships.insert(
                    *id,
                    Relationship {
                        id: *id,
                        schema_id: self.schema_id,
                        start_node: *start,
                        end_node: *end,
                        rel_type: type_id,
                        properties,
                    },
                );
                Ok(Some(Change::DeleteRelationship { id: *id }))
            }
            Change::DeleteRelationship { id } => {
                let rel = self.relationships.remove(id).ok_or_else(|| rel_not_found(*id))?;
                if let Some(n) = self.nodes.get_mut(&rel.start_node) {
                    n.outgoing.retain(|a| a.rel != *id);
                }
                if let Some(n) = self.nodes.get_mut(&rel.end_node) {
                    n.incoming.retain(|a| a.rel != *id);
                }
                if let Some(set) = self.rel_type_index.get_mut(&rel.rel_type) {
                    set.remove(id);
                }
                self.path_cache().relationship_deleted(*id);
                Ok(Some(Change::CreateRelationship {
                    id: *id,
                    start: rel.start_node,
                    end: rel.end_node,
                    rel_type: self.rel_types.name(rel.rel_type).to_string(),
                    properties: rel.properties.into_map(),
                }))
            }
            Change::SetNodeProperty { id, key, value, ts } => {
                let value = normalize_value(key, value)?;
                let node = self.nodes.get_mut(id).ok_or_else(|| node_not_found(*id))?;
                let old = match &value {
                    Some(v) => node.properties.insert(key, v.clone(), &mut self.prop_keys),
                    None => node.properties.remove(key),
                };
                let old_ts = std::mem::replace(&mut node.updated_at, *ts);
                for l in &node.labels {
                    if let Some(idx) = self.prop_index.get_mut(l).and_then(|m| m.get_mut(key)) {
                        if let Some(o) = &old {
                            idx.remove(o, *id);
                        }
                        if let Some(v) = &value {
                            idx.insert(v, *id);
                        }
                    }
                }
                Ok(Some(Change::SetNodeProperty { id: *id, key: key.clone(), value: old, ts: old_ts }))
            }
            Change::SetRelationshipProperty { id, key, value } => {
                let value = normalize_value(key, value)?;
                let rel = self.relationships.get_mut(id).ok_or_else(|| rel_not_found(*id))?;
                let old = match &value {
                    Some(v) => rel.properties.insert(key, v.clone(), &mut self.prop_keys),
                    None => rel.properties.remove(key),
                };
                let rel_type = rel.rel_type;
                self.path_cache.get_mut().relationship_property_set(*id, rel_type, key, old.as_ref(), value.as_ref());
                Ok(Some(Change::SetRelationshipProperty { id: *id, key: key.clone(), value: old }))
            }
            Change::AddLabel { id, label, ts } => {
                let node = self.nodes.get_mut(id).ok_or_else(|| node_not_found(*id))?;
                let lid = self.labels.intern(label);
                if node.labels.contains(&lid) {
                    return Ok(None);
                }
                node.labels.push(lid);
                let old_ts = std::mem::replace(&mut node.updated_at, *ts);
                self.label_index.entry(lid).or_default().insert(*id);
                self.index_node(lid, *id);
                Ok(Some(Change::RemoveLabel { id: *id, label: label.clone(), ts: old_ts }))
            }
            Change::RemoveLabel { id, label, ts } => {
                let node = self.nodes.get_mut(id).ok_or_else(|| node_not_found(*id))?;
                let Some(lid) = self.labels.get(label) else { return Ok(None) };
                if !node.labels.contains(&lid) {
                    return Ok(None);
                }
                node.labels.retain(|l| *l != lid);
                let old_ts = std::mem::replace(&mut node.updated_at, *ts);
                if let Some(set) = self.label_index.get_mut(&lid) {
                    set.remove(id);
                }
                let props = std::mem::take(&mut self.nodes.get_mut(id).expect("checked").properties);
                self.unindex_node(lid, *id, &props);
                self.nodes.get_mut(id).expect("checked").properties = props;
                Ok(Some(Change::AddLabel { id: *id, label: label.clone(), ts: old_ts }))
            }
            Change::CreateIndex { name, label, key } => {
                if self.has_index(label, key) {
                    return Err(MyGraphError::AlreadyExists(format!("index on :{label}({key})")));
                }
                if self.index_by_name(name).is_some() {
                    return Err(MyGraphError::AlreadyExists(format!("index `{name}`")));
                }
                let lid = self.labels.intern(label);
                let mut idx = PropIndex { name: name.clone(), entries: HashMap::new() };
                for nid in self.label_index.get(&lid).into_iter().flatten() {
                    if let Some(v) = self.nodes.get(nid).and_then(|n| n.properties.get(key)) {
                        idx.insert(v, *nid);
                    }
                }
                self.prop_index.entry(lid).or_default().insert(key.clone(), idx);
                Ok(Some(Change::DropIndex { label: label.clone(), key: key.clone() }))
            }
            Change::DropIndex { label, key } => {
                if let Some(name) = self.constraint_on(label, key, ConstraintKind::Unique) {
                    return Err(MyGraphError::Execution(format!(
                        "index on :{label}({key}) is used by constraint `{name}`; drop the constraint first"
                    )));
                }
                let removed =
                    self.labels.get(label).and_then(|l| self.prop_index.get_mut(&l)).and_then(|m| m.remove(key));
                let idx = removed.ok_or_else(|| MyGraphError::NotFound(format!("index on :{label}({key})")))?;
                Ok(Some(Change::CreateIndex { name: idx.name, label: label.clone(), key: key.clone() }))
            }
            Change::CreateConstraint { name, label, key, kind, owns_index } => {
                if self.constraints.contains_key(name) {
                    return Err(MyGraphError::AlreadyExists(format!("constraint `{name}`")));
                }
                if let Some(other) = self.constraint_on(label, key, *kind) {
                    return Err(MyGraphError::AlreadyExists(format!(
                        "constraint `{other}` on :{label}({key}) {}",
                        kind.as_str()
                    )));
                }
                if *kind == ConstraintKind::Unique && !self.has_index(label, key) {
                    return Err(MyGraphError::Execution(format!(
                        "a unique constraint needs an index on :{label}({key})"
                    )));
                }
                let info = ConstraintInfo {
                    name: name.clone(),
                    label: label.clone(),
                    key: key.clone(),
                    kind: *kind,
                    owns_index: *owns_index,
                };
                if let Some(lid) = self.labels.get(label) {
                    for nid in self.label_index.get(&lid).into_iter().flatten() {
                        let value = self.nodes.get(nid).and_then(|n| n.properties.get(key));
                        self.check_node(&info, lid, *nid, value)?;
                    }
                }
                self.constraints.insert(name.clone(), info);
                Ok(Some(Change::DropConstraint { name: name.clone() }))
            }
            Change::DropConstraint { name } => {
                let c = self
                    .constraints
                    .remove(name)
                    .ok_or_else(|| MyGraphError::NotFound(format!("constraint `{name}`")))?;
                Ok(Some(Change::CreateConstraint {
                    name: c.name,
                    label: c.label,
                    key: c.key,
                    kind: c.kind,
                    owns_index: c.owns_index,
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_node(id: NodeId, label: &str) -> Change {
        Change::CreateNode {
            id,
            labels: vec![label.into()],
            properties: PropertyMap::from([("name".into(), Value::from("x"))]),
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn apply_and_inverse_roundtrip() {
        let mut g = Graph::new(0);
        let mut undo = vec![];
        undo.push(g.apply(&create_node(1, "User")).unwrap().unwrap());
        undo.push(g.apply(&create_node(2, "User")).unwrap().unwrap());
        let rel = Change::CreateRelationship {
            id: 7,
            start: 1,
            end: 2,
            rel_type: "KNOWS".into(),
            properties: PropertyMap::new(),
        };
        undo.push(g.apply(&rel).unwrap().unwrap());
        let adj = Adjacent { rel: 7, other: 2, rel_type: g.rel_type_id("KNOWS").unwrap() };
        assert_eq!(g.node(1).unwrap().outgoing(), &[adj]);
        assert_eq!(g.node(2).unwrap().incoming(), &[Adjacent { other: 1, ..adj }]);
        assert_eq!(g.relationship_ids_with_type("KNOWS").collect::<Vec<_>>(), vec![7]);
        assert_eq!(g.rel_type_count("KNOWS"), 1);
        assert_eq!(g.node_ids_with_label("User"), vec![1, 2]);
        assert!(g.apply(&Change::DeleteNode { id: 1 }).is_err());

        for c in undo.into_iter().rev() {
            g.apply(&c).unwrap();
        }
        assert_eq!(g.node_count(), 0);
        assert_eq!(g.relationship_count(), 0);
        assert!(g.node_ids_with_label("User").is_empty());
        assert_eq!(g.rel_type_count("KNOWS"), 0);
    }

    #[test]
    fn index_changes_are_invertible() {
        let mut g = Graph::new(0);
        g.apply(&create_node(1, "User")).unwrap();
        let create = Change::CreateIndex { name: "i".into(), label: "User".into(), key: "name".into() };
        let inv = g.apply(&create).unwrap().unwrap();
        assert_eq!(g.nodes_by_property("User", "name", &Value::from("x")), Some(vec![1]));
        assert!(g.apply(&create).is_err(), "duplicate index");
        g.apply(&create_node(2, "User")).unwrap();
        g.apply(&Change::SetNodeProperty { id: 1, key: "name".into(), value: Some(1.0.into()), ts: 2 }).unwrap();
        assert_eq!(g.nodes_by_property("User", "name", &Value::from("x")), Some(vec![2]));
        assert_eq!(g.nodes_by_property("User", "name", &Value::from(1)), Some(vec![1]));
        assert_eq!(g.nodes_by_property("User", "name", &Value::from(vec![1])), None, "lists are not indexed");
        assert_eq!(g.nodes_by_property("User", "name", &Value::Integer(1 << 60)), None, "beyond 2^53: scan");

        let recreate = g.apply(&inv).unwrap().unwrap();
        assert!(!g.has_index("User", "name"));
        assert_eq!(recreate, create);
        g.apply(&recreate).unwrap();
        assert_eq!(g.indexes()[0].entries, 2);
    }

    #[test]
    fn constraint_changes_are_invertible() {
        let mut g = Graph::new(0);
        g.apply(&create_node(1, "User")).unwrap();
        let unique = Change::CreateConstraint {
            name: "u".into(),
            label: "User".into(),
            key: "name".into(),
            kind: ConstraintKind::Unique,
            owns_index: false,
        };
        assert!(g.apply(&unique).is_err(), "a unique constraint needs an index");
        g.apply(&Change::CreateIndex { name: "i".into(), label: "User".into(), key: "name".into() }).unwrap();
        let inv = g.apply(&unique).unwrap().unwrap();
        assert_eq!(inv, Change::DropConstraint { name: "u".into() });
        assert!(g.apply(&Change::DropIndex { label: "User".into(), key: "name".into() }).is_err());

        // Both nodes are named "x".
        let second = create_node(2, "User");
        g.apply(&second).unwrap();
        assert!(matches!(g.check_constraints(&[second]), Err(MyGraphError::ConstraintViolation(_))));
        assert!(g.check_constraints(&[create_node(1, "User")]).is_err(), "node 1 shares its value too");
        g.apply(&Change::DeleteNode { id: 2 }).unwrap();

        let recreate = g.apply(&inv).unwrap().unwrap();
        assert_eq!(recreate, unique);
        assert!(g.constraints().is_empty());
        g.apply(&recreate).unwrap();
        assert_eq!(g.constraint_on("User", "name", ConstraintKind::Unique), Some("u"));

        let not_null = Change::CreateConstraint {
            name: "n".into(),
            label: "User".into(),
            key: "age".into(),
            kind: ConstraintKind::NotNull,
            owns_index: false,
        };
        assert!(matches!(g.apply(&not_null), Err(MyGraphError::ConstraintViolation(_))));
        assert_eq!(g.constraints().len(), 1, "a failed create changes nothing");
    }

    #[test]
    fn lists_labels_types_and_keys() {
        let mut g = Graph::new(0);
        g.apply(&create_node(1, "B")).unwrap();
        g.apply(&create_node(2, "A")).unwrap();
        g.apply(&Change::CreateRelationship {
            id: 1,
            start: 1,
            end: 2,
            rel_type: "R".into(),
            properties: PropertyMap::from([("w".into(), Value::from(1))]),
        })
        .unwrap();
        assert_eq!(g.label_names(), vec!["A", "B"]);
        assert_eq!(g.rel_type_names(), vec!["R"]);
        assert_eq!(g.property_keys(), vec!["name", "w"]);
        g.apply(&Change::DeleteRelationship { id: 1 }).unwrap();
        g.apply(&Change::RemoveLabel { id: 1, label: "B".into(), ts: 2 }).unwrap();
        assert_eq!(g.label_names(), vec!["A"], "labels without nodes are not listed");
        assert!(g.rel_type_names().is_empty());
    }

    #[test]
    fn index_postings_grow_and_shrink() {
        let mut g = Graph::new(0);
        g.apply(&Change::CreateIndex { name: "i".into(), label: "User".into(), key: "name".into() }).unwrap();
        let mut undo = vec![];
        for id in [3, 1, 2] {
            undo.push(g.apply(&create_node(id, "User")).unwrap().unwrap()); // all named "x"
        }
        assert_eq!(g.nodes_by_property("User", "name", &Value::from("x")), Some(vec![1, 2, 3]));
        assert_eq!(g.count_by_property("User", "name", &Value::from("x")), Some(3));
        let set = |id, v: &str| Change::SetNodeProperty { id, key: "name".into(), value: Some(v.into()), ts: 2 };
        undo.push(g.apply(&set(1, "y")).unwrap().unwrap());
        undo.push(g.apply(&set(2, "y")).unwrap().unwrap());
        assert_eq!(g.nodes_by_property("User", "name", &Value::from("x")), Some(vec![3]));
        assert_eq!(g.nodes_by_property("User", "name", &Value::from("y")), Some(vec![1, 2]));
        assert_eq!(g.indexes()[0].entries, 3);
        for c in undo.into_iter().rev() {
            g.apply(&c).unwrap();
        }
        assert_eq!(g.nodes_by_property("User", "name", &Value::from("x")), Some(vec![]));
        assert_eq!(g.indexes()[0].entries, 0);
    }

    #[test]
    fn set_property_inverse_restores_old_value() {
        let mut g = Graph::new(0);
        g.apply(&create_node(1, "User")).unwrap();
        let set = Change::SetNodeProperty { id: 1, key: "name".into(), value: Some("y".into()), ts: 5 };
        let inv = g.apply(&set).unwrap().unwrap();
        assert_eq!(g.node(1).unwrap().properties.get("name").cloned().unwrap(), Value::from("y"));
        g.apply(&inv).unwrap();
        assert_eq!(g.node(1).unwrap().properties.get("name").cloned().unwrap(), Value::from("x"));
        assert_eq!(g.node(1).unwrap().updated_at, 1);
    }
}
