//! Write journal and the write context handed to the query executor.
//!
//! v0.1 concurrency model: a single writer at a time (enforced by the caller),
//! with statement- and transaction-level atomicity provided by undo records.
//! MVCC is planned for a later phase.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::catalog::IdAllocator;
use crate::error::Result;
use crate::graph::{Change, Graph, NodeId, RelId};
use crate::value::{PropertyMap, Value};

pub fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Redo/undo log of the changes applied by a transaction.
#[derive(Debug, Default)]
pub struct Journal {
    redo: Vec<Change>,
    undo: Vec<Change>,
}

impl Journal {
    pub fn is_empty(&self) -> bool {
        self.redo.is_empty()
    }

    pub fn len(&self) -> usize {
        self.redo.len()
    }

    /// Changes applied so far, in order (the redo log).
    pub fn changes(&self) -> &[Change] {
        &self.redo
    }

    /// Splits the journal into its redo log and its undo log (both in the
    /// order the changes were applied), without copying them.
    pub fn into_parts(self) -> (Vec<Change>, Vec<Change>) {
        (self.redo, self.undo)
    }

    /// A savepoint usable with [`Journal::rollback_to`].
    pub fn mark(&self) -> usize {
        self.redo.len()
    }

    /// Undoes every change recorded after `mark`, newest first.
    pub fn rollback_to(&mut self, graph: &mut Graph, mark: usize) -> Result<()> {
        while self.undo.len() > mark {
            let undo = self.undo.pop().expect("len checked");
            self.redo.pop();
            graph.apply(&undo)?;
        }
        Ok(())
    }
}

/// Mutable view of one schema graph that records every change in a journal.
pub struct WriteContext<'a> {
    graph: &'a mut Graph,
    ids: &'a mut IdAllocator,
    journal: &'a mut Journal,
}

impl<'a> WriteContext<'a> {
    pub fn new(graph: &'a mut Graph, ids: &'a mut IdAllocator, journal: &'a mut Journal) -> Self {
        WriteContext { graph, ids, journal }
    }

    pub fn graph(&self) -> &Graph {
        self.graph
    }

    /// Applies a change and journals it. Returns `false` if it was a no-op.
    pub fn apply(&mut self, change: Change) -> Result<bool> {
        match self.graph.apply(&change)? {
            Some(inverse) => {
                self.journal.redo.push(change);
                self.journal.undo.push(inverse);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn create_node(&mut self, labels: Vec<String>, properties: PropertyMap) -> Result<NodeId> {
        let id = self.ids.next_node();
        let ts = now_millis();
        self.apply(Change::CreateNode { id, labels, properties, created_at: ts, updated_at: ts })?;
        Ok(id)
    }

    pub fn create_relationship(
        &mut self,
        start: NodeId,
        end: NodeId,
        rel_type: &str,
        properties: PropertyMap,
    ) -> Result<RelId> {
        let id = self.ids.next_relationship();
        self.apply(Change::CreateRelationship { id, start, end, rel_type: rel_type.into(), properties })?;
        Ok(id)
    }

    pub fn delete_node(&mut self, id: NodeId) -> Result<()> {
        self.apply(Change::DeleteNode { id }).map(|_| ())
    }

    pub fn delete_relationship(&mut self, id: RelId) -> Result<()> {
        self.apply(Change::DeleteRelationship { id }).map(|_| ())
    }

    /// Sets a node property; `Value::Null` removes it.
    pub fn set_node_property(&mut self, id: NodeId, key: &str, value: Value) -> Result<()> {
        let value = (!value.is_null()).then_some(value);
        self.apply(Change::SetNodeProperty { id, key: key.into(), value, ts: now_millis() }).map(|_| ())
    }

    /// Sets a relationship property; `Value::Null` removes it.
    pub fn set_relationship_property(&mut self, id: RelId, key: &str, value: Value) -> Result<()> {
        let value = (!value.is_null()).then_some(value);
        self.apply(Change::SetRelationshipProperty { id, key: key.into(), value }).map(|_| ())
    }

    pub fn add_label(&mut self, id: NodeId, label: &str) -> Result<bool> {
        self.apply(Change::AddLabel { id, label: label.into(), ts: now_millis() })
    }

    pub fn remove_label(&mut self, id: NodeId, label: &str) -> Result<bool> {
        self.apply(Change::RemoveLabel { id, label: label.into(), ts: now_millis() })
    }
}
