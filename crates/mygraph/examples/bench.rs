//! Micro-benchmark modelled on the graph benchmark report (load in batches of
//! 1,000, then point lookups and traversals).
//!
//! ```text
//! cargo run --release -p mygraph --example bench -- [nodes] [--index]
//! ```
//!
//! `nodes` defaults to 50,000 (about 100,000 relationships). `--index` creates
//! a property index on `:Item(key)` before loading.

use std::time::{Duration, Instant};

use mygraph::{Database, Params, Value};

const BATCH: usize = 1_000;

fn params(pairs: Vec<(&str, Value)>) -> Params {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn map(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Deterministic pseudo-random generator (xorshift), so runs are comparable.
struct Rng(u64);

impl Rng {
    fn next(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn time_query(db: &Database, name: &str, query: &str, p: Params) {
    // one cold run, then 10 warm runs; report the warm median like the report does
    let rows = match db.execute(query, p.clone()) {
        Ok(r) => r.rows.len(),
        Err(e) => {
            println!("{name:<34} unsupported: {e}");
            return;
        }
    };
    let runs: Vec<Duration> = (0..10)
        .map(|_| {
            let t = Instant::now();
            db.execute(query, p.clone()).expect("query ran once already");
            t.elapsed()
        })
        .collect();
    println!("{name:<34} p50 {:>10.3} ms   rows {rows}", median(runs).as_secs_f64() * 1e3);
}

fn main() -> mygraph::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: usize = args.iter().find_map(|a| a.parse().ok()).unwrap_or(50_000);
    let with_index = args.iter().any(|a| a == "--index");

    let db = Database::in_memory()?;
    if with_index {
        db.execute("CREATE INDEX FOR (n:Item) ON (n.key)", Params::new())?;
    }

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let types = ["CALLS", "READS", "WRITES", "INCLUDES"];

    let start = Instant::now();
    for chunk in (0..n).collect::<Vec<_>>().chunks(BATCH) {
        let rows = chunk
            .iter()
            .map(|&i| map(vec![("key", Value::from(format!("k{i}"))), ("group", Value::Integer((i % 100) as i64))]))
            .collect();
        db.execute(
            "UNWIND $rows AS r CREATE (:Item {key: r.key, group: r.group})",
            params(vec![("rows", Value::List(rows))]),
        )?;
    }
    let nodes_time = start.elapsed();

    // 2 relationships per node: a chain (so long traversals exist) plus a random edge
    let mut edges: Vec<(usize, usize, &str)> = Vec::with_capacity(2 * n);
    for i in 0..n {
        if i + 1 < n {
            edges.push((i, i + 1, types[i % types.len()]));
        }
        edges.push((i, rng.next(n as u64) as usize, types[rng.next(types.len() as u64) as usize]));
    }
    let edge_start = Instant::now();
    for t in types {
        let of_type: Vec<_> = edges.iter().filter(|e| e.2 == t).collect();
        for chunk in of_type.chunks(BATCH) {
            let rows = chunk
                .iter()
                .map(|(a, b, _)| map(vec![("a", Value::from(format!("k{a}"))), ("b", Value::from(format!("k{b}")))]))
                .collect();
            db.execute(
                &format!("UNWIND $rows AS e MATCH (a:Item {{key: e.a}}), (b:Item {{key: e.b}}) CREATE (a)-[:{t}]->(b)"),
                params(vec![("rows", Value::List(rows))]),
            )?;
        }
    }
    let edge_time = edge_start.elapsed();
    let total = start.elapsed();
    println!(
        "load: {n} nodes in {:.2} s, {} relationships in {:.2} s, total {:.2} s ({:.0} elem/s){}",
        nodes_time.as_secs_f64(),
        edges.len(),
        edge_time.as_secs_f64(),
        total.as_secs_f64(),
        (n + edges.len()) as f64 / total.as_secs_f64(),
        if with_index { " [index on :Item(key)]" } else { "" }
    );

    let mid = format!("k{}", n / 2);
    let p = |k: &str| params(vec![("k", Value::from(k))]);
    time_query(&db, "lookup inline {key}", "MATCH (n:Item {key: $k}) RETURN n.group", p(&mid));
    time_query(&db, "lookup WHERE n.key =", "MATCH (n:Item) WHERE n.key = $k RETURN n.group", p(&mid));
    time_query(
        &db,
        "lookup by id()",
        "MATCH (n) WHERE id(n) = $id RETURN n.key",
        params(vec![("id", Value::Integer((n / 2) as i64))]),
    );
    time_query(
        &db,
        "2 hops, anchor on last node",
        "MATCH (a:Item)-[:CALLS|READS]->(b)-[]->(c:Item {key: $k}) RETURN count(*) AS c",
        p(&mid),
    );
    time_query(
        &db,
        "cross product + WHERE",
        "MATCH (a:Item), (b:Item) WHERE a.key = $k AND b.key = $k2 RETURN a.group, b.group",
        params(vec![("k", Value::from(mid.as_str())), ("k2", Value::from("k7"))]),
    );
    time_query(
        &db,
        "impact *1..8",
        "MATCH (a:Item {key: $k})-[:CALLS|READS|WRITES|INCLUDES*1..8]->(b) RETURN count(DISTINCT b) AS c",
        p(&mid),
    );
    time_query(
        &db,
        "shortestPath",
        "MATCH (a:Item {key: $a}), (b:Item {key: $b}), p = shortestPath((a)-[*..30]->(b)) RETURN length(p) AS len",
        params(vec![("a", Value::from("k0")), ("b", Value::from(format!("k{}", n - 1)))]),
    );
    time_query(&db, "scan all CALLS", "MATCH (a)-[:CALLS]->(b) RETURN count(*) AS c", Params::new());
    time_query(&db, "scan all CALLS, group", "MATCH (a)-[:CALLS]->(b) RETURN b.key, count(*) AS c", Params::new());
    time_query(
        &db,
        "top 10 by in-degree",
        "MATCH (a)-[:CALLS]->(b) RETURN b.key, count(*) AS c ORDER BY c DESC LIMIT 10",
        Params::new(),
    );
    Ok(())
}
