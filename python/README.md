# mygraph-client (Python)

Python client for the MyGraph server. It talks to the server over TCP using
MyGraph protocol v1 ([docs/PROTOCOL.md](../docs/PROTOCOL.md)) and needs only
the standard library (Python ≥ 3.10).

## Install

```bash
pip install -e python/        # from the repository root
```

## Start a server

```bash
cargo run -p mygraph-server -- start                      # 127.0.0.1:6660, in memory
cargo run -p mygraph-server -- start --port 6660 --data ./data
```

## Usage

```python
import mygraph

with mygraph.connect("localhost", 6660) as db:
    # Any Cypher, with parameters as a dict and/or keyword arguments
    r = db.run("MATCH (u:User) WHERE u.age > $min RETURN u.name AS name", min=18)
    for record in r:                # records are dicts
        print(record["name"])
    r.value("name"); r.values("name"); r.single(); r.stats.nodes_created

    # Create
    alice = db.create_node("User", {"name": "Alice", "age": 25})          # -> Node
    bob = db.create_node(["User", "Admin"], {"name": "Bob"})
    knows = db.create_relationship(alice.id, bob.id, "KNOWS", {"since": 2020})

    # Search
    db.get_node(alice.id)                                                 # Node | None
    db.find_nodes("User", {"name": "Alice"}, order_by="age", limit=10)    # list[Node]
    db.count_nodes("User")
    db.find_relationships("KNOWS", start_id=alice.id)                     # list[Relationship]

    # Update
    db.update_node(alice.id, {"age": 26})                  # merge: SET n += $props (None removes a key)
    db.update_node(alice.id, {"name": "Alicia"}, replace=True)   # SET n = $props
    db.update_node(alice.id, add_labels="VIP", remove_labels="Admin")
    db.update_relationship(knows.id, {"since": 2021})

    # Delete
    db.delete_relationship(knows.id)
    db.delete_node(alice.id)                      # DETACH DELETE by default
    db.delete_nodes("User", {"name": "Bob"})     # returns how many

    # Transactions: commit on exit, rollback on exception
    with db.transaction() as tx:
        tx.create_node("User", {"name": "Carol"})
        tx.run("MATCH (u:User {name: 'Carol'}) SET u.vip = true")

    # Databases and schemas (the current one is per connection)
    db.create_database("shop"); db.use_database("shop"); db.databases()
```

`Node` has `id`, `labels`, `properties` (and `node["key"]`). `Relationship`
has `id`, `start_node`, `end_node`, `type`, `properties`.

Helpers write labels, types and property keys as backtick-quoted
identifiers, and always send values as parameters, so user input is never
spliced into the query text.

## Errors

| Exception | When |
|---|---|
| `CypherSyntaxError` | Invalid Cypher (`.line`, `.column`) |
| `NotFoundError`, `AlreadyExistsError` | Unknown or duplicate database/schema |
| `TransactionError` | e.g. COMMIT with no open transaction |
| `QueryError` | Base class of the above; `.code` holds the engine error kind |
| `ProtocolError` | The server or the client broke protocol v1 |
| `ConnectionClosedError` | Cannot connect, or the connection was lost/closed |

An error in a statement does not close the connection.

## Notes for MyGraph v0.1

- An open transaction blocks every other connection until it commits or rolls
  back. Keep transactions short.
- While a transaction is open, everything sent on that client runs inside it.
- A client serializes its requests with a lock. Threads that share one
  client also share its current database and transaction, so use one client
  per thread for independent work.
- `AUTH` is accepted but not checked yet (no users or roles on the server).

## Tests

```bash
cd python
python3 -m unittest discover -s tests -t .
```

The integration tests build `mygraph-server` with cargo and start it on a
free port. Set `MYGRAPH_SERVER_BIN` to use an existing binary instead.
