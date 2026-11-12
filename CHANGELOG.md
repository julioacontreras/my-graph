# Changelog

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
