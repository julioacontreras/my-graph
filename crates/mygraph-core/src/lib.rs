//! MyGraph core: property-graph model, values, database/schema catalog and the
//! transactional write context used by the query engine.
//!
//! This crate has no knowledge of Cypher, storage formats or networking.

pub mod catalog;
pub mod error;
pub mod graph;
pub mod transaction;
pub mod value;

pub use catalog::{Catalog, GraphDatabase, IdAllocator, Op, Schema, DEFAULT_DATABASE, DEFAULT_SCHEMA};
pub use error::{MyGraphError, Result};
pub use graph::{Change, Graph, IndexInfo, IndexKey, LabelId, Node, NodeId, RelId, RelTypeId, Relationship, SchemaId};
pub use transaction::{now_millis, Journal, WriteContext};
pub use value::{format_identifier, NodeValue, PathValue, PropertyMap, RelationshipValue, Value};
