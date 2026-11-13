//! Run a server (`docker run -p 6660:6660 julioacontreras/mygraph`), then:
//! `cargo run -p mygraph-client --example quickstart`

use mygraph_client::{params, Client};

fn main() -> mygraph_client::Result<()> {
    let mut db = Client::connect("localhost:6660")?;
    println!("connected to {} {}", db.server_info().server, db.server_info().version);

    db.run_with("CREATE (:User {name: $name, age: $age})", params! { "name" => "Alice", "age" => 30 })?;
    db.run_with("CREATE (:User {name: $name, age: $age})", params! { "name" => "Bob", "age" => 25 })?;
    db.run("MATCH (a:User {name: 'Alice'}), (b:User {name: 'Bob'}) CREATE (a)-[:FOLLOWS]->(b)")?;

    let result = db.run("MATCH (a:User)-[:FOLLOWS]->(b:User) RETURN a.name AS from, b.name AS to")?;
    for record in result.records() {
        println!("{record:?}");
    }

    db.run("MATCH (u:User) DETACH DELETE u")?;
    db.close();
    Ok(())
}
