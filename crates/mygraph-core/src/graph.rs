//! In-memory property graph for a single schema.
//!
//! All mutations go through [`Graph::apply`], which takes a [`Change`] and
//! returns its inverse. The same `Change` records serve as redo entries for the
//! WAL and as undo entries for rollback.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::error::{MyGraphError, Result};
use crate::value::{NodeValue, PropertyMap, RelationshipValue, Value};

pub type NodeId = u64;
pub type RelId = u64;
pub type LabelId = u32;
pub type RelTypeId = u32;
pub type SchemaId = u32;

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
    outgoing: Vec<RelId>,
    incoming: Vec<RelId>,
}

impl Node {
    pub fn outgoing(&self) -> &[RelId] {
        &self.outgoing
    }

    pub fn incoming(&self) -> &[RelId] {
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
}

#[derive(Debug, Clone)]
pub struct Graph {
    schema_id: SchemaId,
    nodes: BTreeMap<NodeId, Node>,
    relationships: BTreeMap<RelId, Relationship>,
    labels: Interner,
    rel_types: Interner,
    label_index: HashMap<LabelId, BTreeSet<NodeId>>,
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
            nodes: BTreeMap::new(),
            relationships: BTreeMap::new(),
            labels: Interner::default(),
            rel_types: Interner::default(),
            label_index: HashMap::new(),
        }
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

    pub fn nodes(&self) -> impl Iterator<Item = &Node> + '_ {
        self.nodes.values()
    }

    pub fn relationships(&self) -> impl Iterator<Item = &Relationship> + '_ {
        self.relationships.values()
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
                self.nodes.get_mut(start).expect("checked").outgoing.push(*id);
                self.nodes.get_mut(end).expect("checked").incoming.push(*id);
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
                    n.outgoing.retain(|r| r != id);
                }
                if let Some(n) = self.nodes.get_mut(&rel.end_node) {
                    n.incoming.retain(|r| r != id);
                }
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
                let old = match value {
                    Some(v) => node.properties.insert(key.clone(), v),
                    None => node.properties.remove(key),
                };
                let old_ts = std::mem::replace(&mut node.updated_at, *ts);
                Ok(Some(Change::SetNodeProperty { id: *id, key: key.clone(), value: old, ts: old_ts }))
            }
            Change::SetRelationshipProperty { id, key, value } => {
                let value = normalize_value(key, value)?;
                let rel = self.relationships.get_mut(id).ok_or_else(|| rel_not_found(*id))?;
                let old = match value {
                    Some(v) => rel.properties.insert(key.clone(), v),
                    None => rel.properties.remove(key),
                };
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
                Ok(Some(Change::AddLabel { id: *id, label: label.clone(), ts: old_ts }))
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
        assert_eq!(g.node(1).unwrap().outgoing(), &[7]);
        assert_eq!(g.node_ids_with_label("User"), vec![1, 2]);
        assert!(g.apply(&Change::DeleteNode { id: 1 }).is_err());

        for c in undo.into_iter().rev() {
            g.apply(&c).unwrap();
        }
        assert_eq!(g.node_count(), 0);
        assert_eq!(g.relationship_count(), 0);
        assert!(g.node_ids_with_label("User").is_empty());
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
