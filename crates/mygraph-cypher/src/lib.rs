//! MyGraph Cypher engine: lexer → parser → semantic checks → executor.
//!
//! Supported in v0.1: MATCH / OPTIONAL MATCH, WHERE, CREATE, SET, REMOVE,
//! DELETE / DETACH DELETE, UNWIND, WITH, RETURN (DISTINCT, ORDER BY, SKIP,
//! LIMIT), aggregations (count, sum, avg, min, max, collect), parameters,
//! CASE, scalar functions, and the MyGraph administrative extensions
//! (CREATE/DROP/USE DATABASE|SCHEMA, SHOW DATABASES|SCHEMAS).

pub mod ast;
pub mod executor;
pub mod lexer;
pub mod parser;
pub mod semantic;

pub use ast::{AdminCommand, Query, Statement};
pub use executor::{execute, ExecResult, QueryStats};
pub use parser::parse;
