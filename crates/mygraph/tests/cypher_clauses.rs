//! Regular expressions, UNION, MERGE and CALL.

use mygraph::{params, Database, MyGraphError, Params, Value};

fn rows(db: &Database, q: &str, p: Params) -> Vec<Vec<Value>> {
    db.execute(q, p).unwrap().rows
}

fn s(v: &str) -> Value {
    Value::from(v)
}

// ---- =~ ----

#[test]
fn regex_matches_the_whole_string() {
    let db = Database::in_memory().unwrap();
    db.execute(
        "CREATE (:User {name: 'alice', email: 'alice@example.com'}), (:User {name: 'Bob', email: 'bob@test.org'}), (:User {name: 'carol'})",
        params! {},
    )
    .unwrap();
    let q = |w: &str| rows(&db, &format!("MATCH (u:User) WHERE {w} RETURN u.name ORDER BY u.name"), params! {});
    assert_eq!(q(r"u.email =~ '.*@example\\.com'"), vec![vec![s("alice")]]);
    // A partial match is not enough.
    assert_eq!(q("u.name =~ 'ali'"), Vec::<Vec<Value>>::new());
    assert_eq!(q("u.name =~ '(?i)b.*'"), vec![vec![s("Bob")]]);
    // A missing property gives null, so the row is filtered out.
    assert_eq!(q("u.email =~ '.*'").len(), 2);
    assert_eq!(
        rows(&db, "MATCH (u:User) WHERE u.name =~ $re RETURN u.name", params! { "re" => "c.*" }),
        vec![vec![s("carol")]]
    );
    assert_eq!(rows(&db, "RETURN 1 =~ '1' AS a, null =~ 'x' AS b", params! {}), vec![vec![Value::Null, Value::Null]]);
}

#[test]
fn invalid_regex_is_an_error() {
    let db = Database::in_memory().unwrap();
    let err = db.execute("RETURN 'a' =~ '(' AS x", params! {}).unwrap_err();
    assert!(matches!(err, MyGraphError::Type(_)), "{err:?}");
}

// ---- UNION ----

fn union_db() -> Database {
    let db = Database::in_memory().unwrap();
    db.execute(
        "CREATE (:Person {name: 'Ann'}), (:Person {name: 'Bo'}), (:Company {name: 'Bo'}), (:Company {name: 'Acme'})",
        params! {},
    )
    .unwrap();
    db
}

#[test]
fn union_removes_duplicates_and_union_all_keeps_them() {
    let db = union_db();
    let r = db
        .execute("MATCH (p:Person) RETURN p.name AS name UNION MATCH (c:Company) RETURN c.name AS name", params! {})
        .unwrap();
    assert_eq!(r.columns, vec!["name"]);
    let mut names: Vec<Value> = r.rows.into_iter().map(|mut r| r.remove(0)).collect();
    names.sort_by_key(|v| v.to_string());
    assert_eq!(names, vec![s("Acme"), s("Ann"), s("Bo")]);

    let r = rows(
        &db,
        "MATCH (p:Person) RETURN p.name AS name UNION ALL MATCH (c:Company) RETURN c.name AS name",
        params! {},
    );
    assert_eq!(r.len(), 4);
    // Each part keeps its own ORDER BY / LIMIT.
    let r =
        rows(&db, "MATCH (p:Person) RETURN p.name AS n ORDER BY n DESC LIMIT 1 UNION ALL RETURN 'x' AS n", params! {});
    assert_eq!(r, vec![vec![s("Bo")], vec![s("x")]]);
}

#[test]
fn union_parts_must_return_the_same_columns() {
    let db = union_db();
    for q in [
        "RETURN 1 AS a UNION RETURN 2 AS b",
        "RETURN 1 AS a, 2 AS b UNION RETURN 2 AS b, 1 AS a",
        "MATCH (n:Person) RETURN * UNION RETURN 1 AS x",
    ] {
        assert!(matches!(db.execute(q, params! {}), Err(MyGraphError::Semantic(_))), "{q}");
    }
    for q in ["RETURN 1 AS a UNION ALL RETURN 1 AS a UNION RETURN 1 AS a", "CREATE (:X) UNION RETURN 1 AS a"] {
        assert!(matches!(db.execute(q, params! {}), Err(MyGraphError::Syntax { .. })), "{q}");
    }
}

#[test]
fn union_with_writes_sums_stats_and_rolls_back_together() {
    let db = union_db();
    let r = db
        .execute(
            "CREATE (a:Tag {name: 't1'}) RETURN a.name AS n UNION CREATE (b:Tag {name: 't2'}) RETURN b.name AS n",
            params! {},
        )
        .unwrap();
    assert_eq!(r.stats.nodes_created, 2);
    assert!(db.execute("CREATE (:Tag {name: 't3'}) RETURN 1 AS n UNION RETURN 1 / 0 AS n", params! {}).is_err());
    assert_eq!(rows(&db, "MATCH (t:Tag) RETURN count(t)", params! {}), vec![vec![Value::from(2)]]);
}

// ---- MERGE ----

fn count(db: &Database, q: &str) -> i64 {
    db.execute(q, params! {}).unwrap().rows[0][0].as_i64().unwrap()
}

#[test]
fn merge_node_is_idempotent_with_on_create_and_on_match() {
    let db = Database::in_memory().unwrap();
    let q =
        "MERGE (u:User {email: $e}) ON CREATE SET u.visits = 1 ON MATCH SET u.visits = u.visits + 1 RETURN u.visits";
    let r = db.execute(q, params! { "e" => "a@x" }).unwrap();
    assert_eq!(r.rows, vec![vec![Value::from(1)]]);
    assert_eq!((r.stats.nodes_created, r.stats.labels_added, r.stats.properties_set), (1, 1, 2));
    let r = db.execute(q, params! { "e" => "a@x" }).unwrap();
    assert_eq!(r.rows, vec![vec![Value::from(2)]]);
    assert_eq!((r.stats.nodes_created, r.stats.properties_set), (0, 1));
    assert_eq!(count(&db, "MATCH (u:User) RETURN count(u)"), 1);
}

#[test]
fn merge_returns_every_match() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE (:City {country: 'ES'}), (:City {country: 'ES'})", params! {}).unwrap();
    let r = db.execute("MERGE (c:City {country: 'ES'}) ON MATCH SET c.seen = true RETURN c.seen", params! {}).unwrap();
    assert_eq!(r.rows.len(), 2);
    assert_eq!(r.stats.nodes_created, 0);
}

#[test]
fn merge_sees_rows_merged_earlier_in_the_same_clause() {
    let db = Database::in_memory().unwrap();
    let r = db
        .execute("UNWIND ['Madrid', 'Paris', 'Madrid'] AS name MERGE (c:City {name: name}) RETURN count(*)", params! {})
        .unwrap();
    assert_eq!(r.rows, vec![vec![Value::from(3)]]);
    assert_eq!(r.stats.nodes_created, 2);
}

#[test]
fn merge_relationship_between_bound_nodes() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE (:P {n: 'a'}), (:P {n: 'b'})", params! {}).unwrap();
    let q = "MATCH (a:P {n: 'a'}), (b:P {n: 'b'}) MERGE (a)-[r:KNOWS]->(b) ON CREATE SET r.since = 2026";
    assert_eq!(db.execute(q, params! {}).unwrap().stats.relationships_created, 1);
    assert_eq!(db.execute(q, params! {}).unwrap().stats.relationships_created, 0);
    // Undirected: the existing a->b relationship matches from b's side too.
    let q = "MATCH (a:P {n: 'a'}), (b:P {n: 'b'}) MERGE (b)-[:KNOWS]-(a)";
    assert_eq!(db.execute(q, params! {}).unwrap().stats.relationships_created, 0);
    // A new undirected relationship is created left to right.
    db.execute("MATCH (a:P {n: 'a'}), (b:P {n: 'b'}) MERGE (b)-[:LIKES]-(a)", params! {}).unwrap();
    assert_eq!(count(&db, "MATCH (:P {n: 'b'})-[:LIKES]->(:P {n: 'a'}) RETURN count(*)"), 1);
    // The whole pattern is merged: unbound nodes are created along with the relationship.
    let r = db.execute("MATCH (a:P {n: 'a'}) MERGE (a)-[:OWNS]->(c:Car {plate: '1'})", params! {}).unwrap();
    assert_eq!((r.stats.nodes_created, r.stats.relationships_created), (1, 1));
}

#[test]
fn merge_errors() {
    let db = Database::in_memory().unwrap();
    for q in [
        "MERGE (u:User {email: null})",
        "MATCH (a) MERGE (a:User)",
        "MERGE (a)-[:R*1..2]->(b)",
        "MERGE (a)-[:R|S]->(b)",
        "MERGE (a)-[]->(b)",
        "MERGE p = (a)-[:R]->(b)",
    ] {
        assert!(matches!(db.execute(q, params! {}), Err(MyGraphError::Semantic(_))), "{q}");
    }
    assert!(matches!(db.execute("MERGE (a:A), (b:B)", params! {}), Err(MyGraphError::Syntax { .. })));
    assert!(matches!(db.execute("MERGE (a:A) ON DELETE SET a.x = 1", params! {}), Err(MyGraphError::Syntax { .. })));
}

#[test]
fn merge_is_rolled_back_with_its_transaction() {
    let db = Database::in_memory().unwrap();
    let mut tx = db.begin_transaction().unwrap();
    tx.execute("MERGE (:T {k: 1})", params! {}).unwrap();
    tx.rollback().unwrap();
    assert_eq!(count(&db, "MATCH (t:T) RETURN count(t)"), 0);
}

#[test]
fn merge_uses_the_property_index() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE INDEX FOR (u:User) ON (u.email)", params! {}).unwrap();
    db.execute("UNWIND range(1, 2000) AS i CREATE (:User {email: toString(i)})", params! {}).unwrap();
    let r = db
        .execute("UNWIND range(1, 2000) AS i MERGE (u:User {email: toString(i)}) RETURN count(*)", params! {})
        .unwrap();
    assert_eq!(r.stats.nodes_created, 0);
    assert_eq!(count(&db, "MATCH (u:User) RETURN count(u)"), 2000);
}

// ---- CALL ----

fn call_db() -> Database {
    let db = Database::in_memory().unwrap();
    db.execute(
        "CREATE (a:User {name: 'a', email: 'x'})-[:KNOWS {since: 1}]->(b:User:Admin {name: 'b'}), (:Ghost)",
        params! {},
    )
    .unwrap();
    db.execute("MATCH (g:Ghost) DELETE g", params! {}).unwrap();
    db.execute("CREATE CONSTRAINT u FOR (u:User) REQUIRE u.email IS UNIQUE", params! {}).unwrap();
    db
}

fn column(r: Vec<Vec<Value>>) -> Vec<Value> {
    r.into_iter().map(|mut row| row.remove(0)).collect()
}

#[test]
fn standalone_calls_return_every_output() {
    let db = call_db();
    let r = db.execute("CALL db.labels()", params! {}).unwrap();
    assert_eq!(r.columns, vec!["label"]);
    // :Ghost has no nodes left, so it is not listed.
    assert_eq!(column(r.rows), vec![s("Admin"), s("User")]);
    assert_eq!(column(rows(&db, "CALL db.relationshipTypes()", params! {})), vec![s("KNOWS")]);
    assert_eq!(column(rows(&db, "CALL db.propertyKeys()", params! {})), vec![s("email"), s("name"), s("since")]);
    assert_eq!(rows(&db, "CALL db.indexes()", params! {}), vec![vec![s("u"), s("User"), s("email"), Value::from(1)]]);
    let r = db.execute("CALL db.constraints()", params! {}).unwrap();
    assert_eq!(r.columns, vec!["name", "type", "label", "property"]);
    assert_eq!(r.rows, vec![vec![s("u"), s("UNIQUE"), s("User"), s("email")]]);
    let names = column(rows(&db, "CALL dbms.procedures() YIELD name", params! {}));
    assert!(names.contains(&s("db.labels")) && names.contains(&s("dbms.procedures")), "{names:?}");
    // Names are case-insensitive.
    assert_eq!(rows(&db, "call DB.LABELS()", params! {}).len(), 2);
}

#[test]
fn call_with_yield_where_and_following_clauses() {
    let db = call_db();
    let r = db.execute("CALL db.labels() YIELD label AS l WHERE l STARTS WITH 'U'", params! {}).unwrap();
    assert_eq!(r.columns, vec!["l"]);
    assert_eq!(r.rows, vec![vec![s("User")]]);
    let r = rows(
        &db,
        "MATCH (u:User) WITH count(u) AS users CALL db.labels() YIELD label RETURN users, label ORDER BY label",
        params! {},
    );
    assert_eq!(r, vec![vec![Value::from(2), s("Admin")], vec![Value::from(2), s("User")]]);
    let r = rows(
        &db,
        "CALL db.labels() YIELD label MATCH (n) WHERE label IN labels(n) RETURN label, count(n) ORDER BY label",
        params! {},
    );
    assert_eq!(r, vec![vec![s("Admin"), Value::from(1)], vec![s("User"), Value::from(2)]]);
    assert_eq!(rows(&db, "CALL db.indexes() YIELD * RETURN name, entries", params! {}).len(), 1);
}

#[test]
fn call_errors() {
    let db = call_db();
    for q in [
        "CALL db.nothing()",
        "CALL db.labels(1)",
        "CALL db.labels() YIELD nope",
        "CALL db.labels() YIELD label, label",
        "MATCH (label) CALL db.labels() YIELD label RETURN label",
    ] {
        assert!(matches!(db.execute(q, params! {}), Err(MyGraphError::Semantic(_))), "{q}");
    }
    for q in ["MATCH (n) CALL db.labels()", "CALL { RETURN 1 }"] {
        assert!(matches!(db.execute(q, params! {}), Err(MyGraphError::Syntax { .. })), "{q}");
    }
}
