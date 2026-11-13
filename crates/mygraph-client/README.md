# mygraph-client

Rust client for the [MyGraph](https://github.com/julioacontreras/my-graph)
graph database server. It speaks [wire protocol v1](https://github.com/julioacontreras/my-graph/blob/main/PROTOCOL.md)
over TCP, is synchronous, and has no dependencies outside MyGraph.

```bash
cargo add mygraph-client
```

You need a running server, for example:

```bash
docker run -p 6660:6660 julioacontreras/mygraph
```

## Usage

```rust
use mygraph_client::{params, Client, Value};

fn main() -> mygraph_client::Result<()> {
    let mut db = Client::connect("localhost:6660")?;

    // Parameters are referenced as $name.
    db.run_with(
        "CREATE (:User {name: $name, age: $age})",
        params! { "name" => "Alice", "age" => 30 },
    )?;

    let result = db.run("MATCH (u:User) RETURN u.name AS name, u.age AS age")?;
    println!("{:?}", result.columns);
    for row in &result.rows {
        println!("{row:?}");
    }
    assert_eq!(result.value(0, "name"), Some(&Value::from("Alice")));

    db.close();
    Ok(())
}
```

### Transactions

With `db` connected as above:

```rust
let mut tx = db.begin()?;
tx.run("CREATE (:User {name: 'Bob'})")?;
tx.commit()?; // dropping `tx` without committing rolls it back

// Or with a closure: commits on Ok, rolls back on Err.
db.transaction(|tx| {
    tx.run("CREATE (:User {name: 'Carol'})")?;
    Ok::<_, mygraph_client::Error>(())
})?;
```

While a transaction is open, other connections wait for it (MyGraph v0.x),
so keep transactions short.

### Databases and schemas

```rust
db.create_database("shop")?;
db.use_database("shop")?;
let r = db.run("RETURN 1")?;
assert_eq!(r.database, "shop");
```

### Options and errors

```rust
use std::time::Duration;
use mygraph_client::{Client, ConnectOptions, Error};

let options = ConnectOptions::default()
    .timeout(Some(Duration::from_secs(5)))
    .auth("user", "secret");
let mut db = Client::connect_with(("localhost", 6660), options)?;

match db.run("MATCH (n RETURN n") {
    Err(Error::Query(e)) if e.is_syntax() => println!("line {:?}: {}", e.line, e.message),
    other => println!("{other:?}"),
}
```

- `Error::Query`: the server could not run the statement. The connection
  stays usable. `code` is the engine error kind (`Syntax`, `NotFound`, ...).
- `Error::Connection` and `Error::Protocol`: the connection is closed;
  open a new client.

Query results contain `mygraph_client::Value`: nulls, booleans, integers,
floats, strings, bytes, lists, maps, and the `Node`, `Relationship` and
`Path` values returned by `MATCH`.

## License

MIT
