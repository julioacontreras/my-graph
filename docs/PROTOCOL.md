# MyGraph wire protocol v1

Implemented by `crates/mygraph-protocol` (Rust) and `python/mygraph/_protocol.py`.
It is the "own versioned protocol with explicit framing" of spec §9.2. The
transport does not depend on the Cypher grammar.

## Connection

- TCP, default port **6660**.
- Strict request/response: the client sends one frame and waits for its
  response before sending the next (no pipelining in v1).
- The first message must be `HELLO`. Anything else gets `ERROR` and the
  connection is closed.
- Each connection has its own session: current database/schema and at most
  one open transaction. When a connection ends, its open transaction is
  rolled back.

## Frame

All integers are little-endian.

| Offset | Size | Field |
|---|---|---|
| 0 | u8 | protocol version (`1`) |
| 1 | u8 | message type |
| 2 | u32 | request id: chosen by the client, echoed in the response |
| 6 | u32 | payload length in bytes, at most 64 MiB |
| 10 | … | payload |

If the server receives an unknown version, it sends `ERROR` with code
`Protocol` and closes the connection. A payload over the limit closes the
connection.

## Payload

A payload is always one **map** encoded without its tag: `u32` count, then
for each entry the key (as a string, below) followed by a tagged value.

### Values

| Tag | Type | Encoding after the tag byte |
|---|---|---|
| 0 | null | — |
| 1 | boolean | u8 (0 / 1) |
| 2 | integer | i64 |
| 3 | float | f64 (IEEE-754 bits) |
| 4 | string | u32 byte length + UTF-8 bytes |
| 5 | bytes | u32 length + bytes |
| 6 | list | u32 count + values |
| 7 | map | u32 count + (string key, value) pairs |
| 8 | node | u64 id, u32 label count + strings, map of properties |
| 9 | relationship | u64 id, u64 start node id, u64 end node id, string type, map of properties |

Tags 0–7 are the same as the storage codec. Nodes and relationships appear only
in results, so clients must not send them. Lists and maps may nest up to 64
levels deep.

## Messages

| Type | Code | Direction | Payload |
|---|---|---|---|
| HELLO | `0x01` | C→S | `{client: string}` |
| HELLO | `0x01` | S→C | `{server: string, version: string, protocol: int}` |
| AUTH | `0x02` | C→S | `{user: string, password: string}` → `OK` (not checked in v1) |
| QUERY | `0x10` | C→S | `{query: string, params?: map}` → `QUERY_RESULT` or `ERROR` |
| QUERY_RESULT | `0x11` | S→C | `{columns: list<string>, rows: list<list>, stats: map<string,int>, database: string, schema: string, in_transaction: bool}` |
| BEGIN | `0x20` | C→S | `{}` → `OK` |
| COMMIT | `0x21` | C→S | `{}` → `OK` |
| ROLLBACK | `0x22` | C→S | `{}` → `OK` |
| OK | `0x23` | S→C | `{}` |
| PING | `0x30` | C→S | `{}` → `PONG` |
| PONG | `0x31` | S→C | `{}` |
| GOODBYE | `0x40` | C→S | `{}` → `OK`, then the server closes |
| ERROR | `0x7F` | S→C | `{code: string, message: string, line?: int, column?: int}` |

Notes:

- `QUERY` takes a script of one or more `;`-separated statements and returns the
  last statement's rows, with `stats` summed over all statements. Outside a
  transaction each statement is auto-committed. Inside one, a failing
  statement is undone and the earlier ones are kept.
- `QUERY` with the text `BEGIN`, `COMMIT` or `ROLLBACK` does the same as the
  dedicated messages. Administrative statements (`CREATE DATABASE`, `USE`, …)
  are not allowed inside a transaction.
- `database` / `schema` in `QUERY_RESULT` are the session's current ones after
  the query, so `USE DATABASE` is visible to the client.
- `stats` keys: `nodes_created`, `nodes_deleted`, `relationships_created`,
  `relationships_deleted`, `properties_set`, `labels_added`, `labels_removed`.
- An `ERROR` in response to a request does not close the connection, except
  for the handshake and version errors above.

### Error codes

`Syntax` (with `line` / `column`), `Semantic`, `Type`, `Execution`, `NotFound`,
`AlreadyExists`, `InvalidName`, `Unsupported`, `Transaction`, `Storage`,
`Corruption`, `Io`. These are the `MyGraphError` variants. `Protocol` means
the request itself was malformed.

## Concurrency in v0.1

Statements from all connections are serialized. An open transaction blocks
every other connection until it commits or rolls back (the same as embedded
mode). MVCC is planned for phase 5.
