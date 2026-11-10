//! Property indexes and MATCH planning: indexed and planned queries must return
//! exactly what a plain scan in written order returns.

use mygraph::{params, Database, Params, Value};

fn rows(db: &Database, q: &str, p: Params) -> Vec<Vec<Value>> {
    db.execute(q, p).unwrap_or_else(|e| panic!("{q}: {e}")).rows
}

/// Mixed-type data that exercises index key normalization (1 vs 1.0 vs "1").
const SEED: &str = "
    CREATE (:P {k: 1, name: 'int'}), (:P {k: 1.0, name: 'float'}), (:P {k: '1', name: 'str'}),
           (:P {k: true, name: 'bool'}), (:P {k: [1], name: 'list'}), (:P {name: 'none'}),
           (:P:Q {k: 2, name: 'pq'}), (:Q {k: 1, name: 'q only'}), (:P {k: 2.5, name: 'frac'})";

fn seeded(indexed: bool) -> Database {
    let db = Database::in_memory().unwrap();
    if indexed {
        db.execute("CREATE INDEX FOR (n:P) ON (n.k)", params! {}).unwrap();
    }
    db.execute(SEED, params! {}).unwrap();
    db
}

const LOOKUPS: &[&str] = &[
    "MATCH (n:P {k: 1}) RETURN n.name ORDER BY n.name",
    "MATCH (n:P {k: 1.0}) RETURN n.name ORDER BY n.name",
    "MATCH (n:P {k: '1'}) RETURN n.name ORDER BY n.name",
    "MATCH (n:P {k: true}) RETURN n.name ORDER BY n.name",
    "MATCH (n:P {k: [1]}) RETURN n.name ORDER BY n.name",
    "MATCH (n:P {k: 2.5}) RETURN n.name ORDER BY n.name",
    "MATCH (n:P) WHERE n.k = 2 RETURN n.name ORDER BY n.name",
    "MATCH (n:P) WHERE 1 = n.k AND n.name <> 'float' RETURN n.name ORDER BY n.name",
    "MATCH (n:P) WHERE n.k IN [1, '1', 2] RETURN n.name ORDER BY n.name",
    "MATCH (n:P) WHERE n.k = null RETURN n.name",
    "MATCH (n:P) WHERE n.k IN null RETURN n.name",
    "MATCH (n:P:Q {k: 2}) RETURN n.name",
    "MATCH (n:Q {k: 1}) RETURN n.name",
];

#[test]
fn index_returns_the_same_rows_as_a_scan() {
    let (plain, indexed) = (seeded(false), seeded(true));
    for q in LOOKUPS {
        assert_eq!(rows(&indexed, q, params! {}), rows(&plain, q, params! {}), "{q}");
    }
}

#[test]
fn index_follows_property_and_label_changes() {
    let (plain, indexed) = (seeded(false), seeded(true));
    let changes = [
        "MATCH (n:P {name: 'int'}) SET n.k = 7",
        "MATCH (n:P {name: 'str'}) REMOVE n.k",
        "MATCH (n:Q {name: 'q only'}) SET n:P",
        "MATCH (n:P {name: 'pq'}) REMOVE n:P",
        "MATCH (n:P {name: 'bool'}) DELETE n",
        "MATCH (n:P {name: 'none'}) SET n += {k: 1}",
        "MATCH (n:P {name: 'frac'}) SET n = {name: 'frac'}",
    ];
    for c in changes {
        plain.execute(c, params! {}).unwrap();
        indexed.execute(c, params! {}).unwrap();
        for q in LOOKUPS.iter().chain(&["MATCH (n:P {k: 7}) RETURN n.name"]) {
            assert_eq!(rows(&indexed, q, params! {}), rows(&plain, q, params! {}), "after `{c}`: {q}");
        }
    }
}

#[test]
fn rollback_restores_the_index() {
    let db = seeded(true);
    {
        let mut tx = db.begin_transaction().unwrap();
        tx.execute("MATCH (n:P {k: 1}) SET n.k = 99", params! {}).unwrap();
        tx.execute("CREATE (:P {k: 99, name: 'new'})", params! {}).unwrap();
        tx.rollback().unwrap();
    }
    assert!(rows(&db, "MATCH (n:P {k: 99}) RETURN n", params! {}).is_empty());
    assert_eq!(rows(&db, "MATCH (n:P {k: 1}) RETURN count(*)", params! {}), vec![vec![Value::Integer(2)]]);
}

#[test]
fn index_ddl() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE (:A {x: 1}), (:A {x: 2}), (:A {y: 3})", params! {}).unwrap();
    db.execute("CREATE INDEX a_x FOR (n:A) ON (n.x)", params! {}).unwrap();
    db.execute("CREATE INDEX ON :A(y)", params! {}).unwrap();
    db.execute("CREATE INDEX ON :A(y)", params! {}).unwrap(); // Memgraph form is idempotent
    db.execute("CREATE INDEX IF NOT EXISTS FOR (n:A) ON (n.x)", params! {}).unwrap();
    assert!(db.execute("CREATE INDEX FOR (n:A) ON (n.x)", params! {}).is_err());
    assert!(db.execute("CREATE INDEX a_x FOR (n:B) ON (n.z)", params! {}).is_err(), "duplicate name");
    assert!(db.execute("CREATE INDEX FOR (n:A) ON (m.x)", params! {}).is_err(), "wrong variable");

    let shown = rows(&db, "SHOW INDEXES", params! {});
    assert_eq!(
        shown,
        vec![
            vec![Value::from("a_x"), Value::from("A"), Value::from("x"), Value::Integer(2)],
            vec![Value::from("index_A_y"), Value::from("A"), Value::from("y"), Value::Integer(1)],
        ]
    );

    db.execute("DROP INDEX a_x", params! {}).unwrap();
    db.execute("DROP INDEX ON :A(y)", params! {}).unwrap();
    db.execute("DROP INDEX a_x IF EXISTS", params! {}).unwrap();
    assert!(db.execute("DROP INDEX a_x", params! {}).is_err());
    assert!(rows(&db, "SHOW INDEXES", params! {}).is_empty());
    assert_eq!(rows(&db, "MATCH (n:A {x: 2}) RETURN n.x", params! {}), vec![vec![Value::Integer(2)]]);
}

#[test]
fn indexes_are_per_schema() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE INDEX ON :A(x); CREATE SCHEMA other; USE SCHEMA other", params! {}).unwrap();
    assert!(rows(&db, "SHOW INDEXES", params! {}).is_empty());
}

#[test]
fn index_survives_wal_replay_and_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    {
        let db = Database::open(dir.path()).unwrap();
        db.execute("CREATE INDEX ON :A(x); CREATE (:A {x: 1}), (:A {x: 2})", params! {}).unwrap();
        // dropping the database would checkpoint; leak it to force a WAL replay
        std::mem::forget(db);
    }
    let check = |db: &Database| {
        assert_eq!(rows(db, "SHOW INDEXES", params! {}).len(), 1);
        assert_eq!(rows(db, "MATCH (n:A {x: 2}) RETURN n.x", params! {}), vec![vec![Value::Integer(2)]]);
    };
    // the leaked instance still holds the lock file
    std::fs::remove_file(dir.path().join("LOCK")).ok();
    let db = Database::open(dir.path()).unwrap();
    assert!(db.recovery_info().unwrap().replayed_records > 0);
    check(&db);
    db.checkpoint().unwrap();
    drop(db);
    let db = Database::open(dir.path()).unwrap();
    assert_eq!(db.recovery_info().unwrap().replayed_records, 0);
    check(&db);
}

/// A chain a0 -> a1 -> ... -> a9 plus a few branches, with an index on `:N(i)`.
fn chain(indexed: bool) -> Database {
    let db = Database::in_memory().unwrap();
    if indexed {
        db.execute("CREATE INDEX ON :N(i)", params! {}).unwrap();
    }
    db.execute("UNWIND range(0, 9) AS i CREATE (:N {i: i, even: i % 2 = 0})", params! {}).unwrap();
    db.execute("MATCH (a:N), (b:N) WHERE b.i = a.i + 1 CREATE (a)-[:NEXT]->(b)", params! {}).unwrap();
    db.execute("MATCH (a:N {i: 3}), (b:N {i: 7}) CREATE (a)-[:JUMP]->(b), (b)-[:JUMP]->(a)", params! {}).unwrap();
    db
}

const PLANNED: &[&str] = &[
    // anchored on the last node (reversed path)
    "MATCH (a:N)-[:NEXT]->(b:N)-[:NEXT]->(c:N {i: 5}) RETURN a.i, b.i, c.i",
    // anchored in the middle (split path)
    "MATCH (a:N)-[:NEXT]->(b:N {i: 4})-[r]->(c) RETURN a.i, b.i, type(r), c.i ORDER BY c.i",
    "MATCH (a)<-[:NEXT]-(b:N {i: 4})<-[:NEXT]-(c) RETURN a.i, c.i",
    "MATCH (a)-[]-(b:N {i: 3})-[]-(c) RETURN a.i, c.i ORDER BY a.i, c.i",
    // cross products with WHERE pushed into the match
    "MATCH (a:N), (b:N) WHERE a.i = 2 AND b.i = 8 RETURN a.i, b.i",
    "MATCH (a:N), (b:N) WHERE a.i + b.i = 9 AND a.even RETURN a.i, b.i ORDER BY a.i",
    "MATCH (a:N), (b:N)-[:JUMP]->(c) WHERE a.i = c.i RETURN a.i, b.i ORDER BY a.i",
    // id() and IN lookups
    "MATCH (n) WHERE id(n) = 4 RETURN n.i",
    "MATCH (n) WHERE id(n) IN [1, 3, 3, 99] RETURN n.i ORDER BY n.i",
    "MATCH (n) WHERE id(n) = 4.0 RETURN n.i",
    "MATCH (n) WHERE id(n) = -1 RETURN n.i",
    "MATCH (n:N) WHERE n.i IN [2, 5] RETURN n.i ORDER BY n.i",
    // bound variables from an earlier clause
    "MATCH (a:N {i: 1}) MATCH (b:N)-[:NEXT]->(a) RETURN b.i",
    "UNWIND [3, 6] AS x MATCH (a:N)-[:NEXT]->(b:N {i: x}) RETURN a.i, b.i ORDER BY b.i",
    // inline properties that depend on another variable keep the written order
    "MATCH (a:N {i: 2})-[:NEXT]->(b:N {even: a.even}) RETURN b.i",
    "MATCH (a:N)-[:NEXT]->(b:N {i: a.i + 1}) WHERE a.i = 6 RETURN b.i",
    // OPTIONAL MATCH with a filter
    "MATCH (a:N {i: 9}) OPTIONAL MATCH (a)-[:NEXT]->(b) WHERE b.i > 0 RETURN a.i, b",
    "MATCH (a:N {i: 0}) OPTIONAL MATCH (a)-[:NEXT]->(b) WHERE b.even RETURN a.i, b.i",
    // relationship uniqueness still holds across split paths
    "MATCH (a:N {i: 3})-[r1:JUMP]->(b)-[r2:JUMP]->(c) RETURN a.i, b.i, c.i",
    "MATCH (x)-[r1]->(y {i: 7})-[r2]->(z) WHERE r1 <> r2 RETURN x.i, z.i ORDER BY x.i, z.i",
    // a filter that is false for every row
    "MATCH (a:N), (b:N) WHERE false RETURN a, b",
];

#[test]
fn planned_matches_equal_written_order() {
    // without an index the planner only reorders by label size and WHERE
    // pushdown; with one it also reverses and splits paths
    let (plain, indexed) = (chain(false), chain(true));
    for q in PLANNED {
        let expected = rows(&plain, q, params! {});
        assert_eq!(rows(&indexed, q, params! {}), expected, "{q}");
    }
    // and the plain results are what Cypher says they should be
    assert_eq!(rows(&plain, PLANNED[0], params! {}), vec![vec![3.into(), 4.into(), 5.into()]]);
    assert_eq!(rows(&plain, PLANNED[4], params! {}), vec![vec![2.into(), 8.into()]]);
    assert_eq!(rows(&plain, PLANNED[18], params! {}), vec![vec![3.into(), 7.into(), 3.into()]]);
    assert_eq!(rows(&plain, PLANNED[16], params! {}), vec![vec![9.into(), Value::Null]]);
}

#[test]
fn where_type_errors_are_still_reported() {
    let db = chain(true);
    assert!(db.execute("MATCH (a:N) WHERE a.i RETURN a", params! {}).is_err());
    assert!(db.execute("MATCH (a:N), (b:N) WHERE a.i = 1 AND b.i RETURN a", params! {}).is_err());
}
