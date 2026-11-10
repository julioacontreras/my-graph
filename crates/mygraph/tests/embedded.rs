use mygraph::{params, Database, MyGraphError, Value};

fn strings(db: &Database, q: &str) -> Vec<Vec<String>> {
    db.execute(q, params! {})
        .unwrap()
        .rows
        .into_iter()
        .map(|r| r.into_iter().map(|v| v.to_string()).collect())
        .collect()
}

fn count(db: &Database, q: &str) -> i64 {
    db.execute(q, params! {}).unwrap().rows[0][0].as_i64().unwrap()
}

/// Spec §21 acceptance test, verbatim (no `;` between the CREATE block and MATCH).
#[test]
fn spec_v01_acceptance_query() {
    let db = Database::in_memory().unwrap();
    let r = db
        .execute(
            r#"CREATE (a:User {name: "Alice"})
               CREATE (b:User {name: "Bob"})
               CREATE (a)-[:KNOWS]->(b)

               MATCH (u:User)-[:KNOWS]->(f)
               RETURN u.name, f.name;"#,
            params! {},
        )
        .unwrap();
    assert_eq!(r.columns, vec!["u.name", "f.name"]);
    assert_eq!(r.rows, vec![vec![Value::from("Alice"), Value::from("Bob")]]);
    assert_eq!(r.stats.nodes_created, 2);
    assert_eq!(r.stats.relationships_created, 1);

    // Same thing as two separate statements.
    let db = Database::in_memory().unwrap();
    db.execute(
        r#"CREATE (a:User {name: "Alice"}) CREATE (b:User {name: "Bob"}) CREATE (a)-[:KNOWS]->(b);"#,
        params! {},
    )
    .unwrap();
    assert_eq!(strings(&db, "MATCH (u:User)-[:KNOWS]->(f) RETURN u.name, f.name"), vec![vec!["\"Alice\"", "\"Bob\""]]);
    // Direction matters, undirected matches both ways.
    assert_eq!(count(&db, "MATCH (u:User)<-[:KNOWS]-(f) RETURN count(*)"), 1);
    assert_eq!(count(&db, "MATCH (u:User)-[:KNOWS]-(f) RETURN count(*)"), 2);
}

#[test]
fn parameters_and_returned_entities() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE (u:User {name: $name, age: $age})", params! { "name" => "Alice", "age" => 25 }).unwrap();
    let r = db.execute("MATCH (u:User {name: $name}) RETURN u", params! { "name" => "Alice" }).unwrap();
    let node = r.rows[0][0].as_node().unwrap();
    assert_eq!(node.labels, vec!["User"]);
    assert_eq!(node.properties["age"], Value::from(25));
    assert_eq!(r.rows[0][0].to_string(), r#"(:User {age: 25, name: "Alice"})"#);

    let err = db.execute("MATCH (u {name: $missing}) RETURN u", params! {});
    assert!(matches!(err, Err(MyGraphError::Semantic(_))));
}

#[test]
fn set_remove_and_delete() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE (:User {name: 'Alice', age: 25})-[:KNOWS {since: 2024}]->(:User {name: 'Bob'})", params! {})
        .unwrap();

    let r = db
        .execute(
            "MATCH (u:User {name: 'Alice'}) SET u.age = u.age + 1, u.active = true, u:Admin RETURN u.age",
            params! {},
        )
        .unwrap();
    assert_eq!(r.rows[0][0], Value::from(26));
    assert_eq!(r.stats.properties_set, 2);
    assert_eq!(r.stats.labels_added, 1);
    assert_eq!(count(&db, "MATCH (a:Admin:User) RETURN count(a)"), 1);

    db.execute("MATCH (u:User {name: 'Alice'}) REMOVE u.active, u:Admin", params! {}).unwrap();
    assert_eq!(
        strings(&db, "MATCH (u {name: 'Alice'}) RETURN keys(u), labels(u)"),
        vec![vec!["[\"age\", \"name\"]", "[\"User\"]"]]
    );

    db.execute("MATCH (u {name: 'Bob'}) SET u += {city: 'Madrid'}", params! {}).unwrap();
    db.execute("MATCH (u {name: 'Bob'}) SET u = {name: 'Robert'}", params! {}).unwrap();
    assert_eq!(
        strings(&db, "MATCH (u:User) WHERE u.name STARTS WITH 'Rob' RETURN properties(u)"),
        vec![vec!["{name: \"Robert\"}"]]
    );

    db.execute("MATCH ()-[r:KNOWS]->() SET r.since = null", params! {}).unwrap();
    assert_eq!(strings(&db, "MATCH ()-[r]->() RETURN r.since, type(r)"), vec![vec!["null", "\"KNOWS\""]]);

    let err = db.execute("MATCH (u:User {name: 'Alice'}) DELETE u", params! {}).unwrap_err();
    assert!(err.to_string().contains("DETACH DELETE"), "{err}");
    assert_eq!(count(&db, "MATCH (u:User) RETURN count(u)"), 2);

    let r = db.execute("MATCH (u:User {name: 'Alice'}) DETACH DELETE u", params! {}).unwrap();
    assert_eq!((r.stats.nodes_deleted, r.stats.relationships_deleted), (1, 1));
    assert_eq!(count(&db, "MATCH (n) RETURN count(n)"), 1);
    assert_eq!(count(&db, "MATCH ()-[r]->() RETURN count(r)"), 0);
}

#[test]
fn projection_aggregation_and_ordering() {
    let db = Database::in_memory().unwrap();
    db.execute(
        "UNWIND [['Ana', 'Madrid', 30], ['Luis', 'Madrid', 40], ['Eva', 'Paris', 20], ['Leo', null, 50]] AS p \
         CREATE (:Person {name: p[0], city: p[1], age: p[2]})",
        params! {},
    )
    .unwrap();

    assert_eq!(
        strings(
            &db,
            "MATCH (p:Person) WHERE p.city IS NOT NULL \
             RETURN p.city AS city, count(*) AS n, avg(p.age) AS avg, collect(p.name) AS names \
             ORDER BY n DESC, city"
        ),
        vec![vec!["\"Madrid\"", "2", "35.0", "[\"Ana\", \"Luis\"]"], vec!["\"Paris\"", "1", "20.0", "[\"Eva\"]"],]
    );
    assert_eq!(
        strings(&db, "MATCH (p:Person) RETURN p.name ORDER BY p.age DESC SKIP 1 LIMIT 2"),
        vec![vec!["\"Luis\""], vec!["\"Ana\""]]
    );
    assert_eq!(
        strings(&db, "MATCH (p:Person) RETURN DISTINCT p.city AS c ORDER BY c"),
        vec![vec!["\"Madrid\""], vec!["\"Paris\""], vec!["null"]]
    );
    assert_eq!(
        strings(&db, "MATCH (p:Person) WITH p.city AS c, max(p.age) AS oldest WHERE oldest > 25 RETURN c, oldest ORDER BY oldest"),
        vec![vec!["\"Madrid\"", "40"], vec!["null", "50"]]
    );
    assert_eq!(strings(&db, "MATCH (p:Nobody) RETURN count(p), sum(p.age), collect(p)"), vec![vec!["0", "0", "[]"]]);
    assert_eq!(
        strings(
            &db,
            "RETURN CASE WHEN 1 < 2 THEN 'yes' ELSE 'no' END AS a, 7 / 2 AS b, 7 / 2.0 AS c, 2 IN [1, 2] AS d"
        ),
        vec![vec!["\"yes\"", "3", "3.5", "true"]]
    );
}

#[test]
fn optional_match_yields_nulls() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE (:User {name: 'Alice'})-[:KNOWS]->(:User {name: 'Bob'})", params! {}).unwrap();
    assert_eq!(
        strings(&db, "MATCH (u:User) OPTIONAL MATCH (u)-[:KNOWS]->(f) RETURN u.name, f.name ORDER BY u.name"),
        vec![vec!["\"Alice\"", "\"Bob\""], vec!["\"Bob\"", "null"]]
    );
}

#[test]
fn schemas_and_databases_are_isolated() {
    let db = Database::in_memory().unwrap();
    db.execute(
        "CREATE DATABASE ecommerce; USE DATABASE ecommerce; CREATE SCHEMA products; CREATE SCHEMA orders; \
         USE SCHEMA products; CREATE (:Product {name: 'Keyboard', price: 99.99});",
        params! {},
    )
    .unwrap();
    assert_eq!(db.current_database(), "ecommerce");
    assert_eq!(db.current_schema(), "products");
    assert_eq!(count(&db, "MATCH (p:Product) RETURN count(p)"), 1);

    db.execute("USE SCHEMA orders", params! {}).unwrap();
    assert_eq!(count(&db, "MATCH (p:Product) RETURN count(p)"), 0);

    db.execute("USE DATABASE default", params! {}).unwrap();
    assert_eq!(db.current_schema(), "public");
    assert_eq!(count(&db, "MATCH (n) RETURN count(n)"), 0);

    let dbs = db.execute("SHOW DATABASES", params! {}).unwrap();
    let names: Vec<String> = dbs.rows.iter().map(|r| r[0].to_string()).collect();
    assert_eq!(names, vec!["\"default\"", "\"ecommerce\""]);

    db.execute("USE DATABASE ecommerce; SHOW SCHEMAS", params! {}).unwrap();
    let schemas = db.execute("SHOW SCHEMAS", params! {}).unwrap();
    assert_eq!(schemas.len(), 3);

    assert!(matches!(db.execute("CREATE DATABASE ecommerce", params! {}), Err(MyGraphError::AlreadyExists(_))));
    db.execute("CREATE DATABASE IF NOT EXISTS ecommerce", params! {}).unwrap();
    db.execute("DROP DATABASE ecommerce", params! {}).unwrap();
    assert_eq!(db.current_database(), "default");
    assert!(db.execute("USE DATABASE ecommerce", params! {}).is_err());
}

#[test]
fn failed_statement_is_rolled_back() {
    let db = Database::in_memory().unwrap();
    let err = db.execute("CREATE (a:Temp {x: 1}) WITH a RETURN 1 / 0", params! {});
    assert!(err.is_err());
    assert_eq!(count(&db, "MATCH (n:Temp) RETURN count(n)"), 0);

    // A script is parsed entirely before it runs.
    let err = db.execute("CREATE (:Temp); RETURN (", params! {});
    assert!(matches!(err, Err(MyGraphError::Syntax { .. })));
    assert_eq!(count(&db, "MATCH (n:Temp) RETURN count(n)"), 0);
}

#[test]
fn explicit_transactions() {
    let db = Database::in_memory().unwrap();
    {
        let mut tx = db.begin_transaction().unwrap();
        tx.execute("CREATE (:User {name: 'Alice'})", params! {}).unwrap();
        // A failing statement only undoes itself.
        assert!(tx.execute("CREATE (:User {name: 'Bob'}) WITH 1 AS x RETURN x / 0", params! {}).is_err());
        assert_eq!(tx.execute("MATCH (u:User) RETURN count(u)", params! {}).unwrap().rows[0][0], Value::from(1));
        assert!(tx.execute("CREATE DATABASE nope", params! {}).is_err());
        tx.commit().unwrap();
    }
    assert_eq!(count(&db, "MATCH (u:User) RETURN count(u)"), 1);

    let mut tx = db.begin_transaction().unwrap();
    tx.execute("MATCH (u:User) SET u.name = 'Changed' CREATE (:User {name: 'Tmp'})", params! {}).unwrap();
    tx.rollback().unwrap();
    assert_eq!(strings(&db, "MATCH (u:User) RETURN u.name"), vec![vec!["\"Alice\""]]);

    {
        let mut tx = db.begin_transaction().unwrap();
        tx.execute("MATCH (u:User) DETACH DELETE u", params! {}).unwrap();
        // dropped without commit
    }
    assert_eq!(count(&db, "MATCH (u:User) RETURN count(u)"), 1);
}

#[test]
fn syntax_errors_have_positions() {
    let db = Database::in_memory().unwrap();
    match db.execute("MATCH (n)\nRETURN n.name +", params! {}) {
        Err(MyGraphError::Syntax { line, column, .. }) => assert_eq!((line, column), (2, 16)),
        other => panic!("{other:?}"),
    }
    assert!(matches!(db.execute("MATCH (n) RETURN m", params! {}), Err(MyGraphError::Semantic(_))));
}

#[test]
fn sessions_have_independent_current_database() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE DATABASE other", params! {}).unwrap();
    let mut a = db.session();
    let mut b = db.session();
    a.execute("USE DATABASE other", params! {}).unwrap();
    a.execute("CREATE (:Only {x: 1})", params! {}).unwrap();
    assert_eq!(a.current_database(), "other");
    assert_eq!(b.current_database(), db.current_database());
    let r = b.execute("MATCH (n:Only) RETURN count(n) AS c", params! {}).unwrap();
    assert_eq!(r.value(0, "c"), Some(&mygraph::Value::Integer(0)));
    let mut tx = a.begin_transaction().unwrap();
    tx.execute("CREATE (:Only {x: 2})", params! {}).unwrap();
    tx.commit().unwrap();
    let r = a.execute("MATCH (n:Only) RETURN count(n) AS c", params! {}).unwrap();
    assert_eq!(r.value(0, "c"), Some(&mygraph::Value::Integer(2)));
}
