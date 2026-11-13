//! MyGraph core: property-graph model, values, database/schema catalog and the
//! transactional write context used by the query engine.
//!
//! This crate has no knowledge of Cypher, storage formats or networking.

pub mod catalog;
pub mod error;
pub mod graph;
pub mod path_cache;
pub mod props;
pub mod transaction;
pub mod value;

pub use catalog::{Catalog, GraphDatabase, IdAllocator, Op, Schema, DEFAULT_DATABASE, DEFAULT_SCHEMA};
pub use error::{MyGraphError, Result};
pub use graph::{
    Adjacent, Change, ConstraintInfo, ConstraintKind, Graph, IdHasher, IdMap, IdSet, IndexInfo, IndexKey, LabelId,
    Node, NodeId, RelId, RelTypeId, Relationship, SchemaId,
};
pub use path_cache::{CachedPaths, PathCache, PathKey, DEFAULT_PATH_CACHE_ENTRIES};
pub use props::{KeyInterner, Properties};
pub use transaction::{now_millis, Journal, WriteContext};
pub use value::{format_identifier, NodeValue, PathValue, PropertyMap, RelationshipValue, Value};
