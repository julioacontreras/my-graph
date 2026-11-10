# Changelog

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
