//! Instance catalog: databases, their schemas, and id allocation.

use std::collections::BTreeMap;

use crate::error::{MyGraphError, Result};
use crate::graph::{Change, Graph, NodeId, RelId, SchemaId};

pub const DEFAULT_DATABASE: &str = "default";
pub const DEFAULT_SCHEMA: &str = "public";

/// Allocates node and relationship ids for a database. Ids are never reused
/// within a running instance (rolled-back ids are simply skipped).
#[derive(Debug, Clone, Default)]
pub struct IdAllocator {
    pub next_node_id: u64,
    pub next_relationship_id: u64,
}

impl IdAllocator {
    pub fn next_node(&mut self) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;
        id
    }

    pub fn next_relationship(&mut self) -> RelId {
        let id = self.next_relationship_id;
        self.next_relationship_id += 1;
        id
    }

    fn observe(&mut self, change: &Change) {
        match change {
            Change::CreateNode { id, .. } if *id >= self.next_node_id => self.next_node_id = id + 1,
            Change::CreateRelationship { id, .. } if *id >= self.next_relationship_id => {
                self.next_relationship_id = id + 1
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
pub struct Schema {
    pub id: SchemaId,
    pub name: String,
    pub graph: Graph,
}

/// A database: an isolated set of schemas sharing one id space.
#[derive(Debug, Clone)]
pub struct GraphDatabase {
    pub id: u32,
    pub name: String,
    pub next_schema_id: SchemaId,
    pub ids: IdAllocator,
    schemas: BTreeMap<String, Schema>,
}

pub fn validate_name(kind: &str, name: &str) -> Result<()> {
    let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.len() <= 63;
    if valid {
        Ok(())
    } else {
        Err(MyGraphError::InvalidName(format!("{kind} name `{name}` must match [A-Za-z_][A-Za-z0-9_]* (max 63 chars)")))
    }
}

impl GraphDatabase {
    /// A new database containing the default `public` schema.
    pub fn new(id: u32, name: &str) -> Self {
        let mut db = Self::empty(id, name);
        db.add_schema(DEFAULT_SCHEMA).expect("default schema name is valid");
        db
    }

    /// A database without schemas (used when restoring from disk).
    pub fn empty(id: u32, name: &str) -> Self {
        GraphDatabase {
            id,
            name: name.to_string(),
            next_schema_id: 0,
            ids: IdAllocator::default(),
            schemas: BTreeMap::new(),
        }
    }

    pub fn schema(&self, name: &str) -> Result<&Schema> {
        self.schemas
            .get(name)
            .ok_or_else(|| MyGraphError::NotFound(format!("schema `{name}` in database `{}`", self.name)))
    }

    pub fn schemas(&self) -> impl Iterator<Item = &Schema> + '_ {
        self.schemas.values()
    }

    pub fn add_schema(&mut self, name: &str) -> Result<SchemaId> {
        validate_name("schema", name)?;
        if self.schemas.contains_key(name) {
            return Err(MyGraphError::AlreadyExists(format!("schema `{name}`")));
        }
        let id = self.next_schema_id;
        self.next_schema_id += 1;
        self.schemas.insert(name.to_string(), Schema { id, name: name.to_string(), graph: Graph::new(id) });
        Ok(id)
    }

    /// Inserts a fully built schema (restore path).
    pub fn insert_schema(&mut self, schema: Schema) {
        if schema.id >= self.next_schema_id {
            self.next_schema_id = schema.id + 1;
        }
        self.schemas.insert(schema.name.clone(), schema);
    }

    pub fn drop_schema(&mut self, name: &str) -> Result<()> {
        self.schemas.remove(name).map(|_| ()).ok_or_else(|| MyGraphError::NotFound(format!("schema `{name}`")))
    }

    /// Mutable access to a schema graph together with the database id allocator.
    pub fn write_parts(&mut self, schema: &str) -> Result<(&mut Graph, &mut IdAllocator)> {
        let db_name = &self.name;
        let s = self
            .schemas
            .get_mut(schema)
            .ok_or_else(|| MyGraphError::NotFound(format!("schema `{schema}` in database `{db_name}`")))?;
        Ok((&mut s.graph, &mut self.ids))
    }

    /// Applies a change (replay path), keeping the id allocator ahead of seen ids.
    pub fn apply_change(&mut self, schema: &str, change: &Change) -> Result<Option<Change>> {
        let (graph, ids) = self.write_parts(schema)?;
        let inverse = graph.apply(change)?;
        ids.observe(change);
        Ok(inverse)
    }
}

/// A catalog-level operation: the unit recorded in the WAL.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    CreateDatabase { name: String },
    DropDatabase { name: String },
    CreateSchema { database: String, name: String },
    DropSchema { database: String, name: String },
    Graph { database: String, schema: String, change: Change },
}

#[derive(Debug, Clone)]
pub struct Catalog {
    databases: BTreeMap<String, GraphDatabase>,
    next_database_id: u32,
}

impl Default for Catalog {
    fn default() -> Self {
        Self::new()
    }
}

impl Catalog {
    /// A fresh catalog with the `default` database and its `public` schema.
    pub fn new() -> Self {
        let mut c = Self::empty();
        c.apply(&Op::CreateDatabase { name: DEFAULT_DATABASE.into() }).expect("default database name is valid");
        c
    }

    pub fn empty() -> Self {
        Catalog { databases: BTreeMap::new(), next_database_id: 0 }
    }

    pub fn next_database_id(&self) -> u32 {
        self.next_database_id
    }

    /// Restore path: inserts a database and reserves its id.
    pub fn insert_database(&mut self, db: GraphDatabase) {
        if db.id >= self.next_database_id {
            self.next_database_id = db.id + 1;
        }
        self.databases.insert(db.name.clone(), db);
    }

    pub fn set_next_database_id(&mut self, id: u32) {
        self.next_database_id = self.next_database_id.max(id);
    }

    pub fn database(&self, name: &str) -> Result<&GraphDatabase> {
        self.databases.get(name).ok_or_else(|| MyGraphError::NotFound(format!("database `{name}`")))
    }

    pub fn database_mut(&mut self, name: &str) -> Result<&mut GraphDatabase> {
        self.databases.get_mut(name).ok_or_else(|| MyGraphError::NotFound(format!("database `{name}`")))
    }

    pub fn databases(&self) -> impl Iterator<Item = &GraphDatabase> + '_ {
        self.databases.values()
    }

    /// Checks that `op` can be applied, without modifying anything.
    /// Graph-level changes are validated when applied.
    pub fn validate(&self, op: &Op) -> Result<()> {
        match op {
            Op::CreateDatabase { name } => {
                validate_name("database", name)?;
                if self.databases.contains_key(name) {
                    return Err(MyGraphError::AlreadyExists(format!("database `{name}`")));
                }
            }
            Op::DropDatabase { name } => {
                self.database(name)?;
            }
            Op::CreateSchema { database, name } => {
                validate_name("schema", name)?;
                if self.database(database)?.schema(name).is_ok() {
                    return Err(MyGraphError::AlreadyExists(format!("schema `{name}`")));
                }
            }
            Op::DropSchema { database, name } | Op::Graph { database, schema: name, .. } => {
                self.database(database)?.schema(name)?;
            }
        }
        Ok(())
    }

    pub fn apply(&mut self, op: &Op) -> Result<()> {
        self.validate(op)?;
        match op {
            Op::CreateDatabase { name } => {
                let id = self.next_database_id;
                self.next_database_id += 1;
                self.databases.insert(name.clone(), GraphDatabase::new(id, name));
            }
            Op::DropDatabase { name } => {
                self.databases.remove(name);
            }
            Op::CreateSchema { database, name } => {
                self.database_mut(database)?.add_schema(name)?;
            }
            Op::DropSchema { database, name } => {
                self.database_mut(database)?.drop_schema(name)?;
            }
            Op::Graph { database, schema, change } => {
                self.database_mut(database)?.apply_change(schema, change)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_catalog_has_default_database_and_schema() {
        let c = Catalog::new();
        assert!(c.database(DEFAULT_DATABASE).unwrap().schema(DEFAULT_SCHEMA).is_ok());
    }

    #[test]
    fn rejects_invalid_and_duplicate_names() {
        let mut c = Catalog::new();
        assert!(c.apply(&Op::CreateDatabase { name: "bad name".into() }).is_err());
        assert!(c.apply(&Op::CreateDatabase { name: DEFAULT_DATABASE.into() }).is_err());
        c.apply(&Op::CreateSchema { database: DEFAULT_DATABASE.into(), name: "s1".into() }).unwrap();
        assert!(c.apply(&Op::CreateSchema { database: DEFAULT_DATABASE.into(), name: "s1".into() }).is_err());
    }
}
