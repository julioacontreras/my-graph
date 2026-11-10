# MyGraph

Property-graph database written in Rust, queried with Cypher.

If you only want to *use* the database (including AI agents), read
[Quick reference](#quick-reference) first.

## Status: v0.1 (vertical slice)

| Area | Implemented |
|---|---|
| Model | Directed property graph: nodes with multiple labels, typed relationships, properties (`null`, bool, int, float, string, bytes, list, map) |
| Databases / schemas | Multiple databases with isolated schemas; `CREATE/DROP/USE DATABASE`, `CREATE/DROP/USE SCHEMA`, `SHOW DATABASES/SCHEMAS` |
| Cypher | `MATCH`, `OPTIONAL MATCH`, `WHERE`, `CREATE`, `SET` (`=`, `+=`, labels), `REMOVE`, `DELETE`, `DETACH DELETE`, `UNWIND`, `WITH`, `RETURN` (`DISTINCT`, `ORDER BY`, `SKIP`, `LIMIT`), `count/sum/avg/min/max/collect`, `CASE`, parameters, scalar functions; errors carry line/column |
| Transactions | Auto-commit per statement; explicit transactions with commit/rollback (rollback on drop); statement-level atomicity |
| Storage | WAL (CRC32, fsync on commit) + snapshot checkpoints, crash recovery, torn-write detection, exclusive directory lock |
| API | Embedded `Database::in_memory()` / `Database::open(path_or_config)`, `params!` |
| CLI | `mygraph shell` / `mygraph query`, embedded or connected to a server |
| Server | `mygraph-server start`: TCP, [protocol v1](PROTOCOL.md), one session per connection, explicit transactions |
| Python client | [`python/`](python/README.md): `run(cypher)`, CRUD helpers for nodes/relationships, transactions; standard library only |

Not yet: `MERGE`, variable-length paths, indexes beyond the label index,
query planner, MVCC/concurrent writers, page-based storage, authentication
(AUTH is accepted but not checked), TLS, import/export, security.

## Quick reference

Everything below was checked against v0.1. If a feature is not listed here
or in the status table above, assume it is not supported.

### Published packages

| What | Where | Install |
|---|---|---|
| Server (Docker image, amd64 + arm64) | Docker Hub [`julioacontreras/mygraph`](https://hub.docker.com/r/julioacontreras/mygraph), also `ghcr.io/julioacontreras/mygraph` | `docker pull julioacontreras/mygraph:latest` |
| Python client + `mygraph-client` command | PyPI `mygraph-client` | `pip install mygraph-client` |
| Python client from GitHub (no PyPI needed) | this repo, folder `python/` | `pip install "git+https://github.com/julioacontreras/my-graph.git#subdirectory=python"` |
| Rust CLI `mygraph` | this repo | `cargo install --git https://github.com/julioacontreras/my-graph.git mygraph-cli` |

The image contains only the server. It has no shell and no CLI, so
`docker compose exec` cannot run queries. Use one of the clients from
your machine.

### 1. Start the server with Docker Compose

From the repository root (the [`docker-compose.yml`](docker-compose.yml) is
already there):

```bash
docker compose up -d            # start on port 6660, data in volume mygraph-data
docker compose ps               # check that it is running
docker compose logs -f mygraph  # follow the logs
docker compose down             # stop it; the data stays in the volume
docker compose down -v          # stop it and DELETE all data
docker compose pull && docker compose up -d   # update to the latest image
```

Outside the repository, save this as `docker-compose.yml` and run the same
commands:

```yaml
services:
  mygraph:
    image: julioacontreras/mygraph:latest   # or ghcr.io/julioacontreras/mygraph:latest
    ports:
      - "6660:6660"        # change the left side to use another host port
    volumes:
      - mygraph-data:/data
    init: true
    restart: unless-stopped

volumes:
  mygraph-data:
```

Without Compose: `docker run -d -p 6660:6660 -v mygraph-data:/data julioacontreras/mygraph:latest`.

Without Docker: `cargo run -p mygraph-server -- start --port 6660 --data ./data`.
This listens on 127.0.0.1 only. Add `--host 0.0.0.0` to accept remote
connections. Without `--data` all data is kept in memory and is lost when
the server stops.

No authentication is needed. `AUTH` is accepted but not checked.

### 2. Connect with a CLI

Both CLIs connect to `localhost:6660` by default. Use `--host` / `--port`
for another address.

**Python CLI (`mygraph-client`):**

```bash
pip install mygraph-client                       # or the GitHub install above
mygraph-client "MATCH (n) RETURN n LIMIT 10;"    # one query, table output
mygraph-client --json "MATCH (n) RETURN n;"      # one JSON object per line
mygraph-client --host db.example.com --port 7000 "RETURN 1;"
mygraph-client <<'CYPHER'                        # multi-line script from stdin
MATCH (u:User)
RETURN u.name AS name;
CYPHER
```

Without installing it, `python3 -m mygraph` (from `python/`) does the same.

**Rust CLI (`mygraph`):**

```bash
cargo install --git https://github.com/julioacontreras/my-graph.git mygraph-cli
# or, in this repo: cargo build --release -p mygraph-cli  → target/release/mygraph

mygraph query --host localhost --json "MATCH (n) RETURN n LIMIT 10"   # one query
mygraph shell --host localhost                                        # interactive shell
```

When you pass `--host` or `--port`, the Rust CLI connects to a server. The
shell shows the current database and schema in the prompt. Each statement
ends with `;`. Use `BEGIN;`, `COMMIT;` and `ROLLBACK;` for transactions, and
`EXIT;` to leave:

```text
$ mygraph shell --host localhost
MyGraph v0.1.0 (connected to localhost:6660)
End statements with `;`. Type EXIT; to quit.
mygraph[default/public]> MATCH (n) RETURN count(n) AS c;
+---+
| c |
+---+
| 0 |
+---+
1 row
(0.3 ms)
```

Without `--host` or `--port`, the Rust CLI runs embedded, without a
server: `--data DIR` stores data on disk, and without `--data` the data is
kept in memory. Do not open a directory in embedded mode while a server is
using it, because the directory is locked.

**From code:** Python `mygraph.connect("localhost", 6660).run("...", param=value)`
(see [python/README.md](python/README.md)). Rust embedded:
`db.execute("...", params! {...})` (see [Usage](#usage)).

For agents, `--json` is the format to parse. The CLIs **cannot pass
parameters** (`$name` fails with `missing parameter`), so put literal values
in the query, or use the Python or Rust library when the values come from
user input.

### 3. Result format

`mygraph query --json` prints one JSON array with one object per row. The
keys are the column names. A column without `AS` is named after its
expression text, for example `n.name` or `count(n)`. Use `AS` to get
predictable keys.

```bash
mygraph query --host localhost --json "MATCH (u:User)-[r:KNOWS]->(f) RETURN u, r, f.name AS friend"
```

```json
[{"u":{"id":0,"labels":["User"],"properties":{"age":30,"name":"Alice"}},
  "r":{"id":0,"type":"KNOWS","start":0,"end":1,"properties":{"since":2020}},
  "friend":"Bob"}]
```

- Node: `{"id", "labels", "properties"}`. Relationship: `{"id", "type", "start", "end", "properties"}`.
- No rows: `[]`. A write without `RETURN` prints `[]` with `--json`. Without
  `--json` it prints a summary such as
  `nodes created: 2, relationships created: 1, properties set: 4, labels added: 2`.
- Errors go to stderr as `error: ...` with exit status 1. Syntax errors give
  the position, for example `syntax error at line 1, column 10: ...`.

`mygraph-client --json` (Python) prints one JSON object per line, one line
per record.

### 4. Sessions, databases and scripts

- A new connection starts in database `default`, schema `public`.
- `USE DATABASE` / `USE SCHEMA` last only for the session. **Each CLI call
  is a new session**, so put `USE` in the same script as the queries that
  need it:
  `mygraph query --host localhost "USE DATABASE shop; MATCH (n) RETURN count(n) AS c"`.
- One query string can contain several statements separated by `;`. The
  output is the result of the last statement. Outside a transaction each
  statement is committed on its own.
- Node and relationship ids are integers given by the engine. Use
  `id(n)` to match them: `MATCH (n) WHERE id(n) = 0 RETURN n`.
- An open transaction blocks all other connections. Keep it short.

### 5. Cypher examples

```cypher
// Create nodes and a relationship in one pattern
CREATE (a:User {name: 'Alice', age: 30})-[:KNOWS {since: 2020}]->(b:User {name: 'Bob', age: 25})

// Connect two existing nodes (there is no MERGE: run MATCH first to avoid duplicates)
MATCH (a:User {name: 'Alice'}), (c:User {name: 'Carol'}) CREATE (a)-[:KNOWS]->(c)

// Read with a filter, ordering and paging
MATCH (u:User) WHERE u.age >= 18 AND u.name STARTS WITH 'A'
RETURN u.name AS name, u.age AS age ORDER BY age DESC SKIP 0 LIMIT 20

// Traverse a fixed number of hops (variable-length paths are not supported)
MATCH (a:User {name: 'Alice'})-[:KNOWS]->()-[:KNOWS]->(fof) RETURN DISTINCT fof.name AS name

// Aggregate
MATCH (u:User) RETURN count(u) AS users, avg(u.age) AS avg_age, collect(u.name) AS names

// Group with WITH
MATCH (u:User)-[:KNOWS]->(f) WITH u, count(f) AS friends WHERE friends > 1 RETURN u.name AS name, friends

// Optional relationships
MATCH (u:User) OPTIONAL MATCH (u)-[:KNOWS]->(f) RETURN u.name AS name, f.name AS friend

// Update and delete
MATCH (u:User {name: 'Bob'}) SET u.age = 26, u:VIP
MATCH (u:User {name: 'Bob'}) SET u += {city: 'Madrid'} REMOVE u.age
MATCH (u:User {name: 'Bob'}) DETACH DELETE u

// Lists
UNWIND [1, 2, 3] AS x RETURN x * 2 AS y

// Databases and schemas
CREATE DATABASE shop; SHOW DATABASES; USE DATABASE shop; CREATE SCHEMA s1; USE SCHEMA s1; SHOW SCHEMAS
```

**Operators:** `= <> < <= > >=`, `AND OR XOR NOT`, `+ - * / % ^`,
`IN`, `STARTS WITH`, `ENDS WITH`, `CONTAINS`, `IS NULL`, `IS NOT NULL`,
`CASE WHEN ... THEN ... ELSE ... END`. Comments are `//` and `/* */`.
Identifiers with spaces or reserved words go in backticks: `` `my label` ``.

**Functions** (names are case-insensitive):

| Kind | Functions |
|---|---|
| Aggregation | `count(*)`, `count(x)`, `sum`, `avg`, `min`, `max`, `collect` |
| Graph | `id`, `labels`, `type`, `properties`, `keys`, `startNode`, `endNode` |
| String | `toUpper`, `toLower`, `trim`, `ltrim`, `rtrim`, `replace`, `split`, `substring`, `left`, `right`, `reverse`, `size` |
| List | `size`, `head`, `last`, `tail`, `range`, `reverse` |
| Number | `abs`, `ceil`, `floor`, `round`, `sign`, `sqrt` |
| Conversion | `toInteger`, `toFloat`, `toString`, `toBoolean`, `coalesce` |
| Other | `length`, `timestamp` |

**Not supported in v0.1:** `MERGE`, variable-length paths (`-[*1..3]->`)
and regular expressions (`=~`) fail with `syntax error ... not supported yet`.
Indexes and constraints (`CREATE INDEX`) and `CALL` / procedures fail with
an ordinary syntax error. To do an upsert, run `MATCH` first and `CREATE`
only when it returns no rows.

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
cargo run -p mygraph-cli -- query --host localhost --port 6660 --json "MATCH (n) RETURN n LIMIT 10"
cargo run -p mygraph --example embedded
cargo test --workspace

cargo run -p mygraph-server -- start --port 6660 --data ./data
python3 python/examples/crud.py
```

```python
import mygraph

with mygraph.connect("localhost", 6660) as db:
    alice = db.create_node("User", {"name": "Alice"})
    db.find_nodes("User", {"name": "Alice"})
    db.update_node(alice.id, {"age": 26})
    db.delete_node(alice.id)
```

## Workspace

| Crate | Responsibility | Depends on |
|---|---|---|
| `mygraph-core` | Values, graph, catalog (databases/schemas), change journal | — |
| `mygraph-cypher` | Lexer, parser, AST, semantic checks, executor | core |
| `mygraph-storage` | Binary format, WAL, snapshots, recovery | core |
| `mygraph` | Public embedded API | core, cypher, storage |
| `mygraph-cli` | `mygraph` binary | mygraph |
| `mygraph-protocol` | Wire protocol v1: frames, value codec, messages | core |
| `mygraph-server` | `mygraph-server` binary (TCP) | mygraph, protocol |

All mutations are invertible `Change` records applied through one entry point
(`Graph::apply`). The same records are the redo log written to the WAL and,
inverted, the undo log used for rollback.

## v0.1 design decisions

- **Storage:** the whole catalog is kept in memory and persisted as WAL plus
  full snapshots, not in pages with a buffer pool. The `Store`
  API is designed so the page-based store can replace it in Phase 3.
  `cache_size_mb` is accepted but has no effect yet.
- **Concurrency:** statements are serialized. An explicit transaction holds
  exclusive access until it commits or rolls back. MVCC is Phase 5.
- **Checkpoints** happen when the WAL reaches a size threshold (default
  64 MiB), when `checkpoint()` is called, and on close. They are not time-based
  yet.
- **`Value::Map`** uses `BTreeMap` (not `HashMap`) so iteration order and the
  binary encoding are deterministic.
- **Server:** one OS thread per connection over `std::net`, not Tokio.
  Statements are serialized anyway in v0.1. Tokio can come
  with concurrent execution. Each connection gets its own `Session`
  (`Database::session()`), so `USE DATABASE` does not leak between clients.
- **Leniency:** clauses may follow `CREATE` without a `WITH` in between.
