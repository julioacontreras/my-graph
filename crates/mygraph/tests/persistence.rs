use std::fs;
use std::path::Path;

use mygraph::{params, Config, Database, Value};

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != "LOCK" {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

fn names(db: &Database) -> Vec<Value> {
    db.execute("MATCH (u:User) RETURN u.name ORDER BY u.name", params! {})
        .unwrap()
        .rows
        .into_iter()
        .map(|mut r| r.remove(0))
        .collect()
}

#[test]
fn data_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let db = Database::open(dir.path()).unwrap();
        db.execute(
            "CREATE DATABASE social; USE DATABASE social; CREATE SCHEMA s2; \
             CREATE (a:User {name: 'Alice', tags: ['x', 'y'], meta: {score: 1.5}})-[:KNOWS {since: 2024}]->(b:User {name: 'Bob'})",
            params! {},
        )
        .unwrap();
    }
    let db = Database::open(dir.path()).unwrap();
    db.execute("USE DATABASE social", params! {}).unwrap();
    let r = db
        .execute("MATCH (a)-[k:KNOWS]->(b) RETURN a.name, a.tags, a.meta.score, k.since, b.name", params! {})
        .unwrap();
    assert_eq!(
        r.rows[0].iter().map(|v| v.to_string()).collect::<Vec<_>>(),
        vec!["\"Alice\"", "[\"x\", \"y\"]", "1.5", "2024", "\"Bob\""]
    );
    assert_eq!(db.execute("SHOW SCHEMAS", params! {}).unwrap().len(), 2);

    // Ids keep increasing after a restart (no reuse).
    let r = db.execute("CREATE (c:User {name: 'Carol'}) RETURN id(c)", params! {}).unwrap();
    assert_eq!(r.rows[0][0], Value::from(2));
}

/// Simulates a crash: copy the data directory while the instance is still
/// open (no final checkpoint), then recover from the copy using the WAL.
#[test]
fn recovers_committed_data_after_crash() {
    let dir = tempfile::tempdir().unwrap();
    let crash = tempfile::tempdir().unwrap();
    let db = Database::open(Config::default().storage_path(dir.path())).unwrap();
    db.execute("CREATE (:User {name: 'Alice'}), (:User {name: 'Bob'})", params! {}).unwrap();
    db.checkpoint().unwrap();
    db.execute("MATCH (u:User {name: 'Bob'}) SET u.name = 'Bobby'", params! {}).unwrap();
    db.execute("CREATE (:User {name: 'Carol'})", params! {}).unwrap();
    {
        let mut tx = db.begin_transaction().unwrap();
        tx.execute("CREATE (:User {name: 'Uncommitted'})", params! {}).unwrap();
        copy_dir(dir.path(), crash.path()); // crash while the transaction is open
    }
    drop(db);

    let recovered = Database::open(crash.path()).unwrap();
    let info = recovered.recovery_info().unwrap();
    assert_eq!(info.replayed_records, 2);
    assert_eq!(names(&recovered), vec![Value::from("Alice"), Value::from("Bobby"), Value::from("Carol")]);
}

#[test]
fn torn_wal_write_is_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let crash = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.execute("CREATE (:User {name: 'Alice'})", params! {}).unwrap();
    db.execute("CREATE (:User {name: 'Bob'})", params! {}).unwrap();
    copy_dir(dir.path(), crash.path());
    drop(db);

    // Chop the last record in half, as if the process died mid-write.
    let wal = crash.path().join("wal.log");
    let len = fs::metadata(&wal).unwrap().len();
    fs::OpenOptions::new().write(true).open(&wal).unwrap().set_len(len - 10).unwrap();

    let recovered = Database::open(crash.path()).unwrap();
    assert!(recovered.recovery_info().unwrap().truncated_wal_bytes > 0);
    assert_eq!(names(&recovered), vec![Value::from("Alice")]);
    // And it keeps working afterwards.
    recovered.execute("CREATE (:User {name: 'Dan'})", params! {}).unwrap();
    drop(recovered);
    assert_eq!(names(&Database::open(crash.path()).unwrap()), vec![Value::from("Alice"), Value::from("Dan")]);
}

#[test]
fn second_instance_cannot_open_same_directory() {
    let dir = tempfile::tempdir().unwrap();
    let _db = Database::open(dir.path()).unwrap();
    assert!(Database::open(dir.path()).is_err());
}

#[test]
fn automatic_checkpoint_truncates_wal() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(Config::default().storage_path(dir.path()).checkpoint_wal_bytes(1024)).unwrap();
    for i in 0..50 {
        db.execute("CREATE (:User {name: $n})", params! { "n" => format!("user{i}") }).unwrap();
    }
    assert!(fs::metadata(dir.path().join("wal.log")).unwrap().len() < 1024);
    drop(db);
    let db = Database::open(dir.path()).unwrap();
    assert_eq!(db.execute("MATCH (u:User) RETURN count(u)", params! {}).unwrap().rows[0][0], Value::from(50));
}
