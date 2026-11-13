//! Unique and not-null constraints: DDL, enforcement at the end of each
//! statement, rollback, and persistence.

use mygraph::{params, Database, MyGraphError, Params, Value};

fn rows(db: &Database, q: &str, p: Params) -> Vec<Vec<Value>> {
    db.execute(q, p).unwrap_or_else(|e| panic!("{q}: {e}")).rows
}

fn count(db: &Database, q: &str) -> i64 {
    rows(db, q, params! {})[0][0].as_i64().unwrap()
}

fn violates(db: &Database, q: &str) {
    match db.execute(q, params! {}) {
        Err(MyGraphError::ConstraintViolation(_)) => {}
        other => panic!("{q}: expected a constraint violation, got {other:?}"),
    }
}

fn s(v: &str) -> Value {
    Value::from(v)
}

#[test]
fn constraint_ddl() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE CONSTRAINT user_email FOR (u:User) REQUIRE u.email IS UNIQUE", params! {}).unwrap();
    db.execute("CREATE CONSTRAINT FOR (u:User) REQUIRE u.name IS NOT NULL", params! {}).unwrap();
    assert_eq!(
        rows(&db, "SHOW CONSTRAINTS", params! {}),
        vec![
            vec![s("constraint_User_name_not_null"), s("NOT NULL"), s("User"), s("name")],
            vec![s("user_email"), s("UNIQUE"), s("User"), s("email")],
        ]
    );
    // The unique constraint created its own index, named after it.
    assert_eq!(
        rows(&db, "SHOW INDEXES", params! {}),
        vec![vec![s("user_email"), s("User"), s("email"), Value::Integer(0)]]
    );
    let err = db.execute("DROP INDEX user_email", params! {}).unwrap_err();
    assert!(matches!(err, MyGraphError::Execution(_)), "{err:?}");

    // Duplicates by name or by definition.
    let again = "CREATE CONSTRAINT user_email FOR (u:User) REQUIRE u.email IS UNIQUE";
    assert!(matches!(db.execute(again, params! {}), Err(MyGraphError::AlreadyExists(_))));
    let other_name = "CREATE CONSTRAINT other FOR (u:User) REQUIRE u.email IS UNIQUE";
    assert!(matches!(db.execute(other_name, params! {}), Err(MyGraphError::AlreadyExists(_))));
    db.execute("CREATE CONSTRAINT other IF NOT EXISTS FOR (u:User) REQUIRE u.email IS UNIQUE", params! {}).unwrap();
    db.execute("CREATE CONSTRAINT ON (u:User) ASSERT u.email IS UNIQUE", params! {}).unwrap();
    assert_eq!(rows(&db, "SHOW CONSTRAINTS", params! {}).len(), 2);

    // Dropping the constraint drops the index it owns.
    db.execute("DROP CONSTRAINT user_email", params! {}).unwrap();
    assert!(rows(&db, "SHOW INDEXES", params! {}).is_empty());
    assert!(matches!(db.execute("DROP CONSTRAINT user_email", params! {}), Err(MyGraphError::NotFound(_))));
    db.execute("DROP CONSTRAINT user_email IF EXISTS", params! {}).unwrap();

    // An existing index is reused and kept.
    db.execute("CREATE INDEX idx FOR (u:User) ON (u.email)", params! {}).unwrap();
    db.execute("CREATE CONSTRAINT c FOR (u:User) REQUIRE u.email IS UNIQUE", params! {}).unwrap();
    assert!(db.execute("DROP INDEX idx", params! {}).is_err());
    db.execute("DROP CONSTRAINT c", params! {}).unwrap();
    assert_eq!(rows(&db, "SHOW INDEXES", params! {})[0][0], s("idx"));

    // Not inside an explicit transaction.
    let mut tx = db.begin_transaction().unwrap();
    assert!(matches!(tx.execute("SHOW CONSTRAINTS", params! {}), Err(MyGraphError::Transaction(_))));
}

#[test]
fn unique_constraint_is_enforced() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE CONSTRAINT FOR (u:User) REQUIRE u.email IS UNIQUE", params! {}).unwrap();
    db.execute("CREATE (:User {email: 'a'}), (:User {email: 'b'}), (:User), (:Other {email: 'a'})", params! {})
        .unwrap();
    violates(&db, "CREATE (:User {email: 'a'})");
    violates(&db, "CREATE (:User {email: 'c'}), (:User {email: 'c'})");
    violates(&db, "MATCH (u:User {email: 'b'}) SET u.email = 'a'");
    violates(&db, "MATCH (o:Other) SET o:User");
    // Equal under Cypher equality: 1 and 1.0.
    db.execute("CREATE (:User {email: 1})", params! {}).unwrap();
    violates(&db, "CREATE (:User {email: 1.0})");
    // Values a unique constraint cannot compare are rejected.
    violates(&db, "CREATE (:User {email: ['x']})");
    // Nothing of a failed statement is kept.
    assert_eq!(count(&db, "MATCH (u:User) RETURN count(u)"), 4);
    assert_eq!(count(&db, "MATCH (o:Other) RETURN count(o)"), 1);

    // Swapping two values in one statement is fine: the check runs at the end.
    db.execute("MATCH (a:User {email: 'a'}), (b:User {email: 'b'}) SET a.email = 'b', b.email = 'a'", params! {})
        .unwrap();
    // Missing values are not duplicates.
    db.execute("CREATE (:User), (:User)", params! {}).unwrap();
    // A deleted node frees its value.
    db.execute("MATCH (u:User {email: 'a'}) DELETE u", params! {}).unwrap();
    db.execute("CREATE (:User {email: 'a'})", params! {}).unwrap();
}

#[test]
fn not_null_constraint_is_enforced() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE CONSTRAINT ON (u:User) ASSERT EXISTS (u.name)", params! {}).unwrap();
    db.execute("CREATE (:User {name: 'a'}), (:Other)", params! {}).unwrap();
    violates(&db, "CREATE (:User)");
    violates(&db, "CREATE (:User {name: null})");
    violates(&db, "MATCH (u:User) REMOVE u.name");
    violates(&db, "MATCH (u:User) SET u.name = null");
    violates(&db, "MATCH (u:User) SET u = {}");
    violates(&db, "MATCH (o:Other) SET o:User");
    // A statement may create the node and set the property later.
    db.execute("CREATE (u:User) SET u.name = 'b'", params! {}).unwrap();
    // Removing the label lifts the requirement.
    db.execute("MATCH (u:User {name: 'b'}) REMOVE u:User REMOVE u.name", params! {}).unwrap();
    assert_eq!(count(&db, "MATCH (u:User) RETURN count(u)"), 1);
}

#[test]
fn creating_a_constraint_checks_existing_data() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE (:User {email: 'a'}), (:User {email: 'a'}), (:User)", params! {}).unwrap();
    violates(&db, "CREATE CONSTRAINT FOR (u:User) REQUIRE u.email IS UNIQUE");
    violates(&db, "CREATE CONSTRAINT FOR (u:User) REQUIRE u.email IS NOT NULL");
    // The failed DDL left nothing behind, not even the index it would own.
    assert!(rows(&db, "SHOW CONSTRAINTS", params! {}).is_empty());
    assert!(rows(&db, "SHOW INDEXES", params! {}).is_empty());
}

#[test]
fn violations_in_a_transaction_keep_earlier_statements() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE CONSTRAINT FOR (u:User) REQUIRE u.email IS UNIQUE", params! {}).unwrap();
    let mut tx = db.begin_transaction().unwrap();
    tx.execute("CREATE (:User {email: 'a'})", params! {}).unwrap();
    let err = tx.execute("CREATE (:User {email: 'b'}), (:User {email: 'a'})", params! {}).unwrap_err();
    assert!(matches!(err, MyGraphError::ConstraintViolation(_)));
    tx.execute("CREATE (:User {email: 'b'})", params! {}).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        rows(&db, "MATCH (u:User) RETURN u.email ORDER BY u.email", params! {}),
        vec![vec![s("a")], vec![s("b")]]
    );
}

#[test]
fn merge_is_an_upsert_on_a_unique_key() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE CONSTRAINT FOR (u:User) REQUIRE u.email IS UNIQUE", params! {}).unwrap();
    let q = "UNWIND $emails AS e MERGE (u:User {email: e}) ON CREATE SET u.n = 1 ON MATCH SET u.n = u.n + 1";
    let emails = Value::List(vec![s("a"), s("b"), s("a")]);
    db.execute(q, params! { "emails" => emails.clone() }).unwrap();
    db.execute(q, params! { "emails" => emails }).unwrap();
    assert_eq!(
        rows(&db, "MATCH (u:User) RETURN u.email, u.n ORDER BY u.email", params! {}),
        vec![vec![s("a"), Value::Integer(4)], vec![s("b"), Value::Integer(2)]]
    );
}

#[test]
fn constraints_are_per_schema() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE CONSTRAINT FOR (u:User) REQUIRE u.email IS UNIQUE", params! {}).unwrap();
    db.execute("CREATE SCHEMA other; USE SCHEMA other", params! {}).unwrap();
    assert!(rows(&db, "SHOW CONSTRAINTS", params! {}).is_empty());
    db.execute("CREATE (:User {email: 'a'}), (:User {email: 'a'})", params! {}).unwrap();
}

#[test]
fn constraints_survive_wal_replay_and_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    {
        let db = Database::open(dir.path()).unwrap();
        db.execute(
            "CREATE CONSTRAINT u FOR (n:A) REQUIRE n.x IS UNIQUE; \
             CREATE CONSTRAINT n FOR (n:A) REQUIRE n.y IS NOT NULL; \
             CREATE CONSTRAINT gone FOR (n:B) REQUIRE n.x IS UNIQUE; \
             DROP CONSTRAINT gone; \
             CREATE (:A {x: 1, y: 1})",
            params! {},
        )
        .unwrap();
        // dropping the database would checkpoint; leak it to force a WAL replay
        std::mem::forget(db);
    }
    let check = |db: &Database| {
        assert_eq!(
            rows(db, "SHOW CONSTRAINTS", params! {}),
            vec![vec![s("n"), s("NOT NULL"), s("A"), s("y")], vec![s("u"), s("UNIQUE"), s("A"), s("x")]]
        );
        assert_eq!(rows(db, "SHOW INDEXES", params! {}).len(), 1);
        violates(db, "CREATE (:A {x: 1, y: 2})");
        violates(db, "CREATE (:A {x: 2})");
        // The owned index is still owned after a restart.
        assert!(db.execute("DROP INDEX u", params! {}).is_err());
    };
    std::fs::remove_file(dir.path().join("LOCK")).ok();
    let db = Database::open(dir.path()).unwrap();
    assert!(db.recovery_info().unwrap().replayed_records > 0);
    check(&db);
    db.checkpoint().unwrap();
    drop(db);
    let db = Database::open(dir.path()).unwrap();
    assert_eq!(db.recovery_info().unwrap().replayed_records, 0);
    check(&db);
    db.execute("DROP CONSTRAINT u", params! {}).unwrap();
    assert!(rows(&db, "SHOW INDEXES", params! {}).is_empty());
}
