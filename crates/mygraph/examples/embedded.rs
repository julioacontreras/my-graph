//! Run with: cargo run -p mygraph --example embedded
use mygraph::{params, Config, Database};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join("mygraph_example");
    let db = Database::open(Config::default().storage_path(&dir))?;

    db.execute("MATCH (n) DETACH DELETE n", params! {})?;
    db.execute(
        "CREATE (a:User {name: $a})-[:KNOWS {since: 2024}]->(b:User {name: $b})",
        params! { "a" => "Alice", "b" => "Bob" },
    )?;

    let mut tx = db.begin_transaction()?;
    tx.execute("MATCH (u:User {name: 'Bob'}) SET u.age = 30", params! {})?;
    tx.commit()?;

    let result = db.execute(
        "MATCH (u:User)-[k:KNOWS]->(f) RETURN u.name AS user, f.name AS friend, f.age AS age, k.since AS since",
        params! {},
    )?;
    for record in result.records() {
        println!("{record:?}");
    }
    println!("data stored in {}", dir.display());
    Ok(())
}
