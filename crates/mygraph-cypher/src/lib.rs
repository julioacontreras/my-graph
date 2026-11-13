//! MyGraph Cypher engine: lexer → parser → semantic checks → executor.
//!
//! Supported: MATCH / OPTIONAL MATCH, WHERE, CREATE, MERGE (ON CREATE / ON
//! MATCH), SET, REMOVE, DELETE / DETACH DELETE, UNWIND, WITH, RETURN
//! (DISTINCT, ORDER BY, SKIP, LIMIT), UNION / UNION ALL, CALL of built-in
//! procedures, aggregations (count, sum, avg, min, max, collect), parameters,
//! CASE, `=~` regular expressions, scalar functions, index and constraint DDL,
//! and the MyGraph administrative extensions (CREATE/DROP/USE
//! DATABASE|SCHEMA, SHOW DATABASES|SCHEMAS).

pub mod ast;
pub mod executor;
pub mod lexer;
pub mod parser;
pub mod semantic;

pub use ast::{AdminCommand, IndexTarget, Query, Statement};
pub use executor::{execute, execute_read, ExecResult, QueryStats};
pub use parser::parse;
