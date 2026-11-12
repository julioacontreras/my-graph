//! In-memory property graph for a single schema.
//!
//! All mutations go through [`Graph::apply`], which takes a [`Change`] and
//! returns its inverse. The same `Change` records serve as redo entries for the
//! WAL and as undo entries for rollback.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

use crate::error::{MyGraphError, Result};
use crate::path_cache::{CachedPaths, PathCache, PathKey, SharedPathCache};
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
    pub properties: PropertyMap,
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
    pub properties: PropertyMap,
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

/// An equality index over one `(label, property)` pair.
#[derive(Debug, Clone, Default)]
struct PropIndex {
    name: String,
    entries: HashMap<IndexKey, BTreeSet<NodeId>>,
}

impl PropIndex {
    fn insert(&mut self, value: &Value, id: NodeId) {
        if let Some(k) = IndexKey::from_value(value) {
            self.entries.entry(k).or_default().insert(id);
        }
    }

    fn remove(&mut self, value: &Value, id: NodeId) {
        if let Some(k) = IndexKey::from_value(value) {
            if let Some(set) = self.entries.get_mut(&k) {
                set.remove(&id);
                if set.is_empty() {
                    self.entries.remove(&k);
                }
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
    label_index: HashMap<LabelId, BTreeSet<NodeId>>,
    /// Relationship ids by type (automatic, like the label index).
    rel_type_index: HashMap<RelTypeId, BTreeSet<RelId>>,
    /// Property indexes, grouped by label so node changes find theirs quickly.
    prop_index: HashMap<LabelId, BTreeMap<String, PropIndex>>,
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
            label_index: HashMap::new(),
            rel_type_index: HashMap::new(),
            prop_index: HashMap::new(),
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
        Some(idx.entries.get(&k).map(|s| s.iter().copied().collect()).unwrap_or_default())
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
        Some(idx.entries.get(&k).map_or(0, BTreeSet::len))
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
                    entries: idx.entries.values().map(BTreeSet::len).sum(),
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

    fn index_node(&mut self, label: LabelId, id: NodeId) {
        let Some(indexes) = self.prop_index.get_mut(&label) else { return };
        let Some(node) = self.nodes.get(&id) else { return };
        for (key, idx) in indexes.iter_mut() {
            if let Some(v) = node.properties.get(key) {
                idx.insert(v, id);
            }
        }
    }

    fn unindex_node(&mut self, label: LabelId, id: NodeId, props: &PropertyMap) {
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
            properties: n.properties.clone(),
        })
    }

    pub fn relationship_value(&self, id: RelId) -> Option<RelationshipValue> {
        self.relationships.get(&id).map(|r| RelationshipValue {
            id,
            start_node: r.start_node,
            end_node: r.end_node,
            rel_type: self.rel_types.name(r.rel_type).to_string(),
            properties: r.properties.clone(),
        })
    }

    /// Applies a change and returns its inverse, or `None` when the change was a no-op.
    pub fn apply(&mut self, change: &Change) -> Result<Option<Change>> {
        match change {
            Change::CreateNode { id, labels, properties, created_at, updated_at } => {
                if self.nodes.contains_key(id) {
                    return Err(MyGraphError::AlreadyExists(format!("node {id}")));
                }
                let properties = clean_properties(properties)?;
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
                    properties: node.properties,
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
                let properties = clean_properties(properties)?;
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
                    properties: rel.properties,
                }))
            }
            Change::SetNodeProperty { id, key, value, ts } => {
                let value = normalize_value(key, value)?;
                let node = self.nodes.get_mut(id).ok_or_else(|| node_not_found(*id))?;
                let old = match &value {
                    Some(v) => node.properties.insert(key.clone(), v.clone()),
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
                    Some(v) => rel.properties.insert(key.clone(), v.clone()),
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
                let removed =
                    self.labels.get(label).and_then(|l| self.prop_index.get_mut(&l)).and_then(|m| m.remove(key));
                let idx = removed.ok_or_else(|| MyGraphError::NotFound(format!("index on :{label}({key})")))?;
                Ok(Some(Change::CreateIndex { name: idx.name, label: label.clone(), key: key.clone() }))
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
    fn set_property_inverse_restores_old_value() {
        let mut g = Graph::new(0);
        g.apply(&create_node(1, "User")).unwrap();
        let set = Change::SetNodeProperty { id: 1, key: "name".into(), value: Some("y".into()), ts: 5 };
        let inv = g.apply(&set).unwrap().unwrap();
        assert_eq!(g.node(1).unwrap().properties["name"], Value::from("y"));
        g.apply(&inv).unwrap();
        assert_eq!(g.node(1).unwrap().properties["name"], Value::from("x"));
        assert_eq!(g.node(1).unwrap().updated_at, 1);
    }
}
