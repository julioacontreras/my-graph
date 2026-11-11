use mygraph::{params, Database};

let db = Database::open("./mygraph_data")?;
db.execute("CREATE (a:User {name: $a})-[:KNOWS]->(b:User {name: $b})",
           params! { "a" => "Alice", "b" => "Bob" })?;
let r = db.execute("MATCH (u:User)-[:KNOWS]->(f) RETURN u.name, f.name", params! {})?;
