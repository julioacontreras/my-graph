# MyGraph

Property-graph database written in Rust, queried with Cypher. The reference
design is [MyGraph-Especificacion-Tecnica-v1.0.md](MyGraph-Especificacion-Tecnica-v1.0.md).

## Status: v0.1 (vertical slice, spec §21)

| Area | Implemented |
|---|---|
| Model | Directed property graph: nodes with multiple labels, typed relationships, properties (`null`, bool, int, float, string, bytes, list, map) |
| Databases / schemas | Multiple databases with isolated schemas; `CREATE/DROP/USE DATABASE`, `CREATE/DROP/USE SCHEMA`, `SHOW DATABASES/SCHEMAS` |
| Cypher | `MATCH`, `OPTIONAL MATCH`, `WHERE`, `CREATE`, `SET` (`=`, `+=`, labels), `REMOVE`, `DELETE`, `DETACH DELETE`, `UNWIND`, `WITH`, `RETURN` (`DISTINCT`, `ORDER BY`, `SKIP`, `LIMIT`), `count/sum/avg/min/max/collect`, `CASE`, parameters, scalar functions; errors carry line/column |
| Transactions | Auto-commit per statement; explicit transactions with commit/rollback (rollback on drop); statement-level atomicity |
| Storage | WAL (CRC32, fsync on commit) + snapshot checkpoints, crash recovery, torn-write detection, exclusive directory lock |
| API | Embedded `Database::in_memory()` / `Database::open(path_or_config)`, `params!` |
| CLI | `mygraph shell` / `mygraph query` in embedded mode |

Not yet: `MERGE`, variable-length paths, indexes beyond the label index,
query planner, MVCC/concurrent writers, page-based storage, server/protocol,
import/export, security. See the roadmap in spec §17.

## Usage

```rust
use mygraph::{params, Database};

let db = Database::open("./mygraph_data")?;
db.execute("CREATE (a:User {name: $a})-[:KNOWS]->(b:User {name: $b})", params! { "a" => "Alice", "b" => "Bob" })?;
let r = db.execute("MATCH (u:User)-[:KNOWS]->(f) RETURN u.name, f.name", params! {})?;

let mut tx = db.begin_transaction()?;
tx.execute("MATCH (u:User {name: 'Bob'}) SET u.age = 30", params! {})?;
tx.commit()?;
```

```bash
cargo run -p mygraph-cli -- shell --data ./data
cargo run -p mygraph-cli -- query --json "MATCH (n) RETURN n LIMIT 10"
cargo run -p mygraph --example embedded
cargo test --workspace
```

## Workspace

| Crate | Responsibility | Depends on |
|---|---|---|
| `mygraph-core` | Values, graph, catalog (databases/schemas), change journal | — |
| `mygraph-cypher` | Lexer, parser, AST, semantic checks, executor | core |
| `mygraph-storage` | Binary format, WAL, snapshots, recovery | core |
| `mygraph` | Public embedded API | core, cypher, storage |
| `mygraph-cli` | `mygraph` binary | mygraph |

All mutations are invertible `Change` records applied through one entry point
(`Graph::apply`). The same records are the redo log written to the WAL and,
inverted, the undo log used for rollback.

## v0.1 design decisions and deviations from the spec

- **Storage:** the whole catalog is kept in memory and persisted as WAL plus
  full snapshots, not in pages with a buffer pool (spec §6). The `Store`
  API is designed so the page-based store can replace it in Phase 3.
  `cache_size_mb` is accepted but has no effect yet.
- **Concurrency:** statements are serialized. An explicit transaction holds
  exclusive access until it commits or rolls back. MVCC is Phase 5.
- **Checkpoints** happen when the WAL reaches a size threshold (default
  64 MiB), when `checkpoint()` is called, and on close. They are not time-based
  yet.
- **`Value::Map`** uses `BTreeMap` (not `HashMap`) so iteration order and the
  binary encoding are deterministic.
- **Leniency:** clauses may follow `CREATE` without `WITH` (as in the spec §21
  example).
