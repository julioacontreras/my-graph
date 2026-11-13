# Changelog

## Unreleased

### Added

- Rust client crate `mygraph-client`, published on crates.io
  (`cargo add mygraph-client`): connect with timeouts and auth, queries
  with parameters (`params!`), transactions (rolled back when dropped),
  ping, and database/schema helpers. Errors separate query errors, which
  keep the connection, from connection and protocol errors.
- `mygraph-core` and `mygraph-protocol` are published too, as dependencies
  of the client.

### Changed

- The `mygraph` CLI uses `mygraph-client` to talk to a server.

## 0.3.1

About half the memory for large graphs. A tree of 1,082,401 nodes and
1,082,400 relationships (the volume test of the benchmark report, where
0.3.0 ran out of memory in a 6 GB container) now loads with a 1.5 GB peak
instead of 2.8 GB.

### Changed

- Node and relationship properties are stored in one exact-size array
  sorted by key, and each key name is stored once per graph and shared.
  Before, every node with properties allocated a whole tree node (~650
  bytes) and its own copy of every key. Memory after loading the tree:
  2,025 MB → 1,055 MB.
- A property index stores a value held by a single node without a set per
  value, so unique properties such as ids cost a few bytes per node.
- Checkpoints write the snapshot to disk in 1 MB chunks, and startup reads
  it the same way, instead of building the whole file in memory (twice
  when writing). A checkpoint of the tree used 413 MB more memory before
  and uses 8 MB now.
- A commit moves its changes into the WAL record instead of copying them.
- Query latencies are unchanged.
- New example `examples/tree_volume.rs` loads the tree and prints memory
  as it goes.

### Upgrade notes

- The snapshot, WAL and protocol formats are unchanged: 0.3.1 and 0.3.0
  read each other's data.

## 0.3.0

Faster searches and weighted shortest paths, following the search engine
plan in `docs/seacrh.md`.

### Added

- Weighted shortest paths: `shortestPath((a)-[*]->(b), {weight: 'cost'})`
  finds the path with the smallest sum of a relationship property
  (bidirectional Dijkstra). Weights must be non-negative numbers; a weight
  needs `*` without a maximum, and is not supported with `allShortestPaths`.
- A per-schema cache of shortest-path searches
  (`Config::path_cache_entries`, default 10,000, 0 disables it). An answer
  is dropped only when a change could affect it, so it stays useful while
  the graph is updated.
- Relationship lookups: `MATCH (a)-[r]->(b) WHERE id(r) = $id` (or `IN`, or
  an already bound `r`) goes straight to the relationship instead of
  scanning every node (137 ms → 0.007 ms at 200k relationships). The Python
  client's relationship helpers use this pattern.
- An automatic relationship-type index: `MATCH (a)-[:RARE]->(b)` starts
  from those relationships when they are fewer than the nodes to scan.
- Read-only scripts (no `CREATE`, `SET`, `REMOVE`, `DELETE`) run
  concurrently, embedded and in the server; writes still run one at a time.
- The benchmark (`examples/bench.rs`) reports p50/p95/p99, random pairs,
  updates per second and concurrent readers.

### Changed

- Unweighted `shortestPath` between random pairs: 0.47 ms → 0.15 ms
  (100k nodes). Adjacency lists carry the relationship type and neighbour,
  and nodes and relationships are stored in hash maps keyed by id.
- About 7% more memory per graph for the richer adjacency lists.

### Upgrade notes

- The snapshot and protocol formats are unchanged: 0.3 and 0.2 read each
  other's data, and the Python client 0.2 works with this server.

## 0.2.0

Search performance and traversals, following the graph database benchmark
(load 28.1 s → under 1 s at 50,677 nodes / 100,525 relationships once the
lookup key is indexed).

### Added

- Property equality indexes: `CREATE INDEX FOR (n:L) ON (n.p)`,
  `CREATE INDEX ON :L(p)`, `DROP INDEX`, `SHOW INDEXES`. **They are not
  created automatically**: create one on every property used to find nodes.
- A rule-based planner: each path starts from its most selective node
  (bound variable, `id()`, index, smallest label). `WHERE` conditions run as
  soon as their variables are bound, so `MATCH (a), (b) WHERE ...` no
  longer builds the full cross product. `id(n) = x` and `id(n) IN list`
  go straight to the node.
- Variable-length relationships (`-[:A|B*1..8]->`, `*`, `*n`, `*n..`, `*..m`).
- Named paths (`p = (a)-->(b)`), `shortestPath` (bidirectional search) and
  `allShortestPaths`, with `length(p)`, `nodes(p)` and `relationships(p)`.
- Path values in results: wire protocol tag 10, `mygraph.Path` in the
  Python client, a JSON shape in the CLI.
- Python client `mygraph-client` 0.2.0 (PyPI, tag `py-v0.2.0`): reads paths.

### Upgrade notes

- Snapshot format version 2 (it stores the index definitions). 0.2 reads
  0.1 data directories; 0.1 cannot read a snapshot written by 0.2.
- Clients older than this version fail to decode a result that contains a
  path. Other results are unchanged.
- In `shortestPath`, `WHERE` is applied after the shortest path is found.
  Neo4j instead looks for the shortest path that satisfies it.

## 0.1.0

First release: property graph, Cypher subset, WAL and snapshots, embedded
API, CLI, TCP server and Python client.
