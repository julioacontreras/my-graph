//! Read-only scripts share the database; writers run alone. Readers running
//! next to a writer must never see a statement half applied.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use mygraph::{params, Database, Value};

#[test]
fn readers_never_see_half_applied_statements() {
    let db = Database::in_memory().unwrap();
    db.execute("UNWIND range(0, 9) AS i CREATE (:Acc {i: i, v: 100})", params! {}).unwrap();
    db.execute("MATCH (a:Acc {i: 0}), (b:Acc {i: 1}) CREATE (a)-[:CUR {cost: 1}]->(b)", params! {}).unwrap();
    let done = AtomicBool::new(false);
    let reads = AtomicU64::new(0);
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    // both invariants hold after every write statement
                    let r = db.execute("MATCH (a:Acc) RETURN sum(a.v) AS total", params! {}).unwrap();
                    assert_eq!(r.rows, vec![vec![Value::Integer(1000)]]);
                    let r = db.execute("MATCH ()-[c:CUR]->() RETURN count(c) AS n", params! {}).unwrap();
                    assert_eq!(r.rows, vec![vec![Value::Integer(1)]]);
                    // a path search through the moving relationship
                    db.execute(
                        "MATCH (a:Acc {i: 0}), (b:Acc) WHERE b.i <> 0
                         OPTIONAL MATCH p = shortestPath((a)-[*]-(b), {weight: 'cost'}) RETURN count(p)",
                        params! {},
                    )
                    .unwrap();
                    reads.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        s.spawn(|| {
            for step in 0..400i64 {
                let (x, y) = (step % 10, (step * 7 + 3) % 10);
                if x != y {
                    db.execute(
                        "MATCH (a:Acc {i: $x}), (b:Acc {i: $y}) SET a.v = a.v - 5, b.v = b.v + 5",
                        params! { "x" => x, "y" => y },
                    )
                    .unwrap();
                }
                // move the single CUR relationship: delete and create in one statement
                db.execute(
                    "MATCH ()-[c:CUR]->() DELETE c WITH count(*) AS gone
                     MATCH (a:Acc {i: $x}), (b:Acc {i: $y}) CREATE (a)-[:CUR {cost: 1}]->(b)",
                    params! { "x" => (x + 1) % 10, "y" => y },
                )
                .unwrap();
            }
            done.store(true, Ordering::Relaxed);
        });
    });
    assert!(reads.load(Ordering::Relaxed) > 0);
}

/// Counts the quick reads `reader` completes while a slow read-only query
/// runs on another thread. Serialized readers would complete almost none.
fn reads_during_a_slow_read(db: &Database, reader: impl Fn() + Sync) -> u64 {
    let (running, finished) = (AtomicBool::new(false), AtomicBool::new(false));
    let overlapped = AtomicU64::new(0);
    std::thread::scope(|s| {
        s.spawn(|| {
            running.store(true, Ordering::SeqCst);
            // a cross product of 700 x 700 nodes: slow on purpose
            db.execute("MATCH (a:N), (b:N) WHERE a.i + b.i = -1 RETURN count(*)", params! {}).unwrap();
            finished.store(true, Ordering::SeqCst);
        });
        s.spawn(|| {
            while !running.load(Ordering::SeqCst) {
                std::hint::spin_loop();
            }
            while !finished.load(Ordering::SeqCst) {
                reader();
                if !finished.load(Ordering::SeqCst) {
                    overlapped.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
    });
    overlapped.into_inner()
}

#[test]
fn readers_do_not_wait_for_each_other() {
    let db = Database::in_memory().unwrap();
    db.execute("UNWIND range(1, 700) AS i CREATE (:N {i: i})", params! {}).unwrap();
    // through the default session, as embedded users call it from many threads
    let n = reads_during_a_slow_read(&db, || {
        db.execute("MATCH (n:N {i: 7}) RETURN n.i", params! {}).unwrap();
    });
    assert!(n > 50, "only {n} reads overlapped the slow one");
    // and through separate sessions, as the server does
    let n = reads_during_a_slow_read(&db, || {
        db.session().execute("MATCH (n:N {i: 7}) RETURN n.i", params! {}).unwrap();
    });
    assert!(n > 50, "only {n} reads overlapped the slow one");
}

#[test]
fn mixed_scripts_take_the_exclusive_path() {
    let db = Database::in_memory().unwrap();
    let r = db.execute("CREATE (:A {x: 1}); MATCH (a:A) RETURN a.x AS x", params! {}).unwrap();
    assert_eq!(r.rows, vec![vec![Value::Integer(1)]]);
    let r = db.execute("MATCH (a:A) RETURN a.x AS x; MATCH (a:A) SET a.x = 2 RETURN a.x AS x", params! {}).unwrap();
    assert_eq!(r.rows, vec![vec![Value::Integer(2)]]);
}
