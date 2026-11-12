//! The shortest-path cache must never change an answer: a database with the
//! cache and one without it get the same random mix of searches and changes,
//! and must agree on every result (or error).

use mygraph::{params, Config, Database, Params, Value};

struct Rng(u64);

impl Rng {
    fn next(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

/// Runs `q` and summarises the outcome: the rows, or the error message.
fn outcome(db: &Database, q: &str, p: Params) -> Result<Vec<Vec<Value>>, String> {
    db.execute(q, p).map(|r| r.rows).map_err(|e| e.to_string())
}

const NODES: u64 = 40;

fn setup(cache: bool) -> Database {
    let entries = if cache { 1000 } else { 0 };
    let db = Database::open(Config::default().path_cache_entries(entries)).unwrap();
    db.execute("CREATE INDEX ON :V(i)", params! {}).unwrap();
    db.execute("UNWIND range(0, 39) AS i CREATE (:V {i: i})", params! {}).unwrap();
    db
}

/// Searches whose answer does not depend on which of several equally short
/// paths is found: lengths, costs and counts only.
const SEARCHES: &[&str] = &[
    "MATCH (a:V {i: $a}), (b:V {i: $b}), p = shortestPath((a)-[*]->(b), {weight: 'cost'})
     UNWIND relationships(p) AS r RETURN count(r), sum(r.cost)",
    "MATCH (a:V {i: $a}), (b:V {i: $b}), p = shortestPath((a)-[:T1*]-(b), {weight: 'cost'})
     UNWIND relationships(p) AS r RETURN sum(r.cost)",
    "MATCH (a:V {i: $a}), (b:V {i: $b}), p = shortestPath((a)-[*]->(b)) RETURN length(p)",
    "MATCH (a:V {i: $a}), (b:V {i: $b}), p = shortestPath((a)-[:T2*..4]-(b)) RETURN length(p)",
    "MATCH (a:V {i: $a}), (b:V {i: $b}), p = allShortestPaths((a)-[*]->(b)) RETURN count(p), min(length(p))",
];

#[test]
fn cached_and_uncached_answers_agree() {
    let (cached, plain) = (setup(true), setup(false));
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    // a few pairs, asked repeatedly, so the cache gets hits
    let pairs: Vec<(i64, i64)> = (0..8).map(|_| (rng.next(NODES) as i64, rng.next(NODES) as i64)).collect();
    let mut next_rel = 0i64;
    let mut searches = 0;
    for step in 0..1500 {
        let op = rng.next(100);
        let (a, b) = (rng.next(NODES) as i64, rng.next(NODES) as i64);
        let ty = if rng.next(2) == 0 { "T1" } else { "T2" };
        let cost = rng.next(10) as i64;
        let rel = if next_rel > 0 { rng.next(next_rel as u64) as i64 } else { 0 };
        let (q, p): (String, Params) = match op {
            0..=49 => {
                let (a, b) = pairs[rng.next(pairs.len() as u64) as usize];
                let q = SEARCHES[rng.next(SEARCHES.len() as u64) as usize];
                searches += 1;
                (q.to_string(), params! { "a" => a, "b" => b })
            }
            50..=69 => {
                next_rel += 1;
                (
                    format!("MATCH (a:V {{i: $a}}), (b:V {{i: $b}}) CREATE (a)-[:{ty} {{cost: $c}}]->(b)"),
                    params! { "a" => a, "b" => b, "c" => cost },
                )
            }
            70..=79 => ("MATCH ()-[r]->() WHERE id(r) = $id DELETE r".into(), params! { "id" => rel }),
            80..=93 => {
                ("MATCH ()-[r]->() WHERE id(r) = $id SET r.cost = $c".into(), params! { "id" => rel, "c" => cost })
            }
            94..=95 => {
                // an invalid weight, fixed again a little later by the SET above
                let bad = ["-1", "'x'", "null"][rng.next(3) as usize];
                (format!("MATCH ()-[r]->() WHERE id(r) = $id SET r.cost = {bad}"), params! { "id" => rel })
            }
            96..=97 => ("MATCH (n:V {i: $a}) SET n.tag = $c".into(), params! { "a" => a, "c" => cost }),
            _ => {
                // changes rolled back: the cache must forget what it saw in between
                for db in [&cached, &plain] {
                    let mut tx = db.begin_transaction().unwrap();
                    let (pa, pb) = pairs[0];
                    tx.execute("MATCH ()-[r]->() WHERE id(r) = $id SET r.cost = 0", params! { "id" => rel }).unwrap();
                    tx.execute(
                        "MATCH (a:V {i: $a}), (b:V {i: $b}) CREATE (a)-[:T1 {cost: 0}]->(b)",
                        params! { "a" => pa, "b" => pb },
                    )
                    .unwrap();
                    tx.execute(SEARCHES[0], params! { "a" => pa, "b" => pb }).ok();
                    tx.rollback().unwrap();
                }
                next_rel += 1; // the rolled-back relationship used an id
                continue;
            }
        };
        let (x, y) = (outcome(&cached, &q, p.clone()), outcome(&plain, &q, p.clone()));
        assert_eq!(x, y, "step {step}: {q} {p:?}");
    }
    assert!(searches > 500);
}

#[test]
fn deleting_an_end_node_and_its_relationships() {
    let (cached, plain) = (setup(true), setup(false));
    for db in [&cached, &plain] {
        db.execute(
            "MATCH (a:V {i: 0}), (b:V {i: 1}), (c:V {i: 2})
             CREATE (a)-[:T1 {cost: 1}]->(b), (b)-[:T1 {cost: 1}]->(c), (a)-[:T1 {cost: 5}]->(c)",
            params! {},
        )
        .unwrap();
    }
    let q = "MATCH (a:V {i: 0}), (c:V {i: 2}), p = shortestPath((a)-[*]->(c), {weight: 'cost'})
             UNWIND relationships(p) AS r RETURN sum(r.cost)";
    for change in
        ["", "MATCH (b:V {i: 1}) DETACH DELETE b", "MATCH (a:V {i: 0}), (c:V {i: 2}) CREATE (a)-[:T2 {cost: 0}]->(c)"]
    {
        for db in [&cached, &plain] {
            if !change.is_empty() {
                db.execute(change, params! {}).unwrap();
            }
        }
        assert_eq!(outcome(&cached, q, params! {}), outcome(&plain, q, params! {}), "after `{change}`");
    }
    assert_eq!(outcome(&cached, q, params! {}), Ok(vec![vec![Value::Integer(0)]]));
}

#[test]
fn weight_changes_without_new_relationships() {
    // with no creations in between, only the weight rules keep answers right
    let (cached, plain) = (setup(true), setup(false));
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut rels = 0i64;
    for _ in 0..160 {
        let (a, b, c) = (rng.next(NODES) as i64, rng.next(NODES) as i64, 1 + rng.next(9) as i64);
        let ty = if rng.next(2) == 0 { "T1" } else { "T2" };
        for db in [&cached, &plain] {
            db.execute(
                &format!("MATCH (a:V {{i: $a}}), (b:V {{i: $b}}) CREATE (a)-[:{ty} {{cost: $c}}]->(b)"),
                params! { "a" => a, "b" => b, "c" => c },
            )
            .unwrap();
        }
        rels += 1;
    }
    let pairs: Vec<(i64, i64)> = (0..6).map(|_| (rng.next(NODES) as i64, rng.next(NODES) as i64)).collect();
    let mut cheaper = 0;
    for step in 0..2000 {
        let rel = rng.next(rels as u64) as i64;
        let (q, p): (String, Params) = match rng.next(10) {
            0..=5 => {
                let (a, b) = pairs[rng.next(pairs.len() as u64) as usize];
                (SEARCHES[rng.next(2) as usize].to_string(), params! { "a" => a, "b" => b })
            }
            6 => (
                "MATCH ()-[r]->() WHERE id(r) = $id SET r.cost = r.cost + $d".into(),
                params! { "id" => rel, "d" => 1 + rng.next(5) as i64 },
            ),
            7 => {
                cheaper += 1;
                ("MATCH ()-[r]->() WHERE id(r) = $id SET r.cost = 0".into(), params! { "id" => rel })
            }
            8 => (
                "MATCH ()-[r]->() WHERE id(r) = $id SET r.cost = $c".into(),
                params! { "id" => rel, "c" => rng.next(10) as i64 },
            ),
            _ => ("MATCH ()-[r]->() WHERE id(r) = $id DELETE r".into(), params! { "id" => rel }),
        };
        let (x, y) = (outcome(&cached, &q, p.clone()), outcome(&plain, &q, p.clone()));
        assert_eq!(x, y, "step {step}: {q} {p:?}");
    }
    assert!(cheaper > 100);
}
