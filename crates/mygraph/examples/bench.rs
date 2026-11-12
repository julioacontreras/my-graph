//! Micro-benchmark modelled on the graph benchmark report (load in batches of
//! 1,000, then point lookups, traversals, shortest paths and updates).
//!
//! ```text
//! cargo run --release -p mygraph --example bench -- [nodes] [--index] [--runs N]
//! ```
//!
//! `nodes` defaults to 50,000 (about 100,000 relationships). `--index` creates
//! a property index on `:Item(key)` before loading; use it for large graphs,
//! otherwise loading the relationships scans every node per row. `--runs`
//! sets the number of timed runs per query (default 30).
//!
//! Every relationship has an integer `cost` between 1 and 100, used by the
//! weighted shortest-path queries. Queries the engine does not support yet are
//! reported as `unsupported` instead of stopping the run.

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

/// Latency percentiles of a set of runs.
struct Stats {
    p50: Duration,
    p95: Duration,
    p99: Duration,
    total: Duration,
    count: usize,
}

impl Stats {
    fn of(mut v: Vec<Duration>) -> Stats {
        assert!(!v.is_empty(), "no runs to summarise");
        v.sort();
        let pick = |q: f64| v[((q * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1];
        Stats { p50: pick(0.50), p95: pick(0.95), p99: pick(0.99), total: v.iter().sum(), count: v.len() }
    }

    fn per_second(&self) -> f64 {
        self.count as f64 / self.total.as_secs_f64()
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn report(name: &str, s: &Stats, extra: &str) {
    println!(
        "{name:<36} p50 {:>9.3}  p95 {:>9.3}  p99 {:>9.3} ms  {:>9.0}/s  {extra}",
        ms(s.p50),
        ms(s.p95),
        ms(s.p99),
        s.per_second()
    );
}

/// Runs `query` once cold, then `runs` warm times with the same parameters.
fn time_query(db: &Database, runs: usize, name: &str, query: &str, p: Params) {
    let rows = match db.execute(query, p.clone()) {
        Ok(r) => r.rows.len(),
        Err(e) => {
            println!("{name:<36} unsupported: {e}");
            return;
        }
    };
    let times = (0..runs)
        .map(|_| {
            let t = Instant::now();
            db.execute(query, p.clone()).expect("query ran once already");
            t.elapsed()
        })
        .collect();
    report(name, &Stats::of(times), &format!("rows {rows}"));
}

/// Runs `query` once per parameter set (each run is a different input).
/// Returns `None` when the engine rejects the query.
fn time_each(db: &Database, name: &str, query: &str, inputs: Vec<Params>) -> Option<Stats> {
    let mut times = Vec::with_capacity(inputs.len());
    let mut found = 0;
    for p in inputs {
        let t = Instant::now();
        match db.execute(query, p) {
            Ok(r) => {
                times.push(t.elapsed());
                found += usize::from(!r.rows.is_empty());
            }
            Err(e) => {
                println!("{name:<36} unsupported: {e}");
                return None;
            }
        }
    }
    let stats = Stats::of(times);
    report(name, &stats, &format!("non-empty {found}/{}", stats.count));
    Some(stats)
}

fn main() -> mygraph::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let with_index = args.iter().any(|a| a == "--index");
    let runs: usize = args
        .iter()
        .position(|a| a == "--runs")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let n: usize = args
        .iter()
        .enumerate()
        .filter(|(i, _)| *i == 0 || args[i - 1] != "--runs")
        .find_map(|(_, a)| a.parse().ok())
        .unwrap_or(50_000);

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
    let mut edges: Vec<(usize, usize, &str, i64)> = Vec::with_capacity(2 * n);
    for i in 0..n {
        if i + 1 < n {
            edges.push((i, i + 1, types[i % types.len()], 1 + rng.next(100) as i64));
        }
        let to = rng.next(n as u64) as usize;
        let t = types[rng.next(types.len() as u64) as usize];
        edges.push((i, to, t, 1 + rng.next(100) as i64));
    }
    let edge_start = Instant::now();
    for t in types {
        let of_type: Vec<_> = edges.iter().filter(|e| e.2 == t).collect();
        for chunk in of_type.chunks(BATCH) {
            let rows = chunk
                .iter()
                .map(|(a, b, _, c)| {
                    map(vec![
                        ("a", Value::from(format!("k{a}"))),
                        ("b", Value::from(format!("k{b}"))),
                        ("c", Value::Integer(*c)),
                    ])
                })
                .collect();
            db.execute(
                &format!(
                    "UNWIND $rows AS e MATCH (a:Item {{key: e.a}}), (b:Item {{key: e.b}}) CREATE (a)-[:{t} {{cost: e.c}}]->(b)"
                ),
                params(vec![("rows", Value::List(rows))]),
            )?;
        }
    }
    let edge_time = edge_start.elapsed();
    let total = start.elapsed();
    let rel_count = edges.len() as u64;
    println!(
        "load: {n} nodes in {:.2} s, {rel_count} relationships in {:.2} s, total {:.2} s ({:.0} elem/s){}",
        nodes_time.as_secs_f64(),
        edge_time.as_secs_f64(),
        total.as_secs_f64(),
        (n + edges.len()) as f64 / total.as_secs_f64(),
        if with_index { " [index on :Item(key)]" } else { "" }
    );
    println!("runs per query: {runs}\n");

    println!("-- lookups and pattern queries (same input every run)");
    let mid = format!("k{}", n / 2);
    let p = |k: &str| params(vec![("k", Value::from(k))]);
    time_query(&db, runs, "lookup inline {key}", "MATCH (n:Item {key: $k}) RETURN n.group", p(&mid));
    time_query(&db, runs, "lookup WHERE n.key =", "MATCH (n:Item) WHERE n.key = $k RETURN n.group", p(&mid));
    time_query(
        &db,
        runs,
        "lookup by id()",
        "MATCH (n) WHERE id(n) = $id RETURN n.key",
        params(vec![("id", Value::Integer((n / 2) as i64))]),
    );
    time_query(
        &db,
        runs,
        "relationship by id()",
        "MATCH ()-[r]->() WHERE id(r) = $id RETURN r.cost",
        params(vec![("id", Value::Integer((rel_count / 2) as i64))]),
    );
    time_query(
        &db,
        runs,
        "2 hops, anchor on last node",
        "MATCH (a:Item)-[:CALLS|READS]->(b)-[]->(c:Item {key: $k}) RETURN count(*) AS c",
        p(&mid),
    );
    time_query(
        &db,
        runs,
        "cross product + WHERE",
        "MATCH (a:Item), (b:Item) WHERE a.key = $k AND b.key = $k2 RETURN a.group, b.group",
        params(vec![("k", Value::from(mid.as_str())), ("k2", Value::from("k7"))]),
    );
    time_query(
        &db,
        runs,
        "impact *1..8",
        "MATCH (a:Item {key: $k})-[:CALLS|READS|WRITES|INCLUDES*1..8]->(b) RETURN count(DISTINCT b) AS c",
        p(&mid),
    );
    time_query(
        &db,
        runs,
        "shortestPath chain ends",
        "MATCH (a:Item {key: $a}), (b:Item {key: $b}), p = shortestPath((a)-[*..30]->(b)) RETURN length(p) AS len",
        params(vec![("a", Value::from("k0")), ("b", Value::from(format!("k{}", n - 1)))]),
    );
    time_query(&db, runs, "scan all CALLS", "MATCH (a)-[:CALLS]->(b) RETURN count(*) AS c", Params::new());
    time_query(
        &db,
        runs,
        "scan all CALLS, group",
        "MATCH (a)-[:CALLS]->(b) RETURN b.key, count(*) AS c",
        Params::new(),
    );
    time_query(
        &db,
        runs,
        "top 10 by in-degree",
        "MATCH (a)-[:CALLS]->(b) RETURN b.key, count(*) AS c ORDER BY c DESC LIMIT 10",
        Params::new(),
    );

    println!("\n-- shortest paths between random pairs (a new pair every run)");
    let mut pair_rng = Rng(0xD1B5_4A32_D192_ED03);
    let pairs: Vec<(String, String)> =
        (0..runs).map(|_| (format!("k{}", pair_rng.next(n as u64)), format!("k{}", pair_rng.next(n as u64)))).collect();
    let pair_params = |pairs: &[(String, String)]| -> Vec<Params> {
        pairs
            .iter()
            .map(|(a, b)| params(vec![("a", Value::from(a.as_str())), ("b", Value::from(b.as_str()))]))
            .collect()
    };
    time_each(
        &db,
        "shortestPath hops *..30",
        "MATCH (a:Item {key: $a}), (b:Item {key: $b}), p = shortestPath((a)-[*..30]->(b)) RETURN length(p) AS len",
        pair_params(&pairs),
    );
    let weighted = "MATCH (a:Item {key: $a}), (b:Item {key: $b}), p = shortestPath((a)-[*]->(b), {weight: 'cost'}) \
                    RETURN length(p) AS len";
    let weighted_ok = time_each(&db, "shortestPath weighted", weighted, pair_params(&pairs)).is_some();
    if weighted_ok {
        let same = vec![pairs[0].clone(); runs];
        time_each(&db, "shortestPath weighted, same pair", weighted, pair_params(&same));
    }

    println!("\n-- updates");
    let mut upd_rng = Rng(0x2545_F491_4F6C_DD1D);
    let update = "MATCH ()-[r]->() WHERE id(r) = $id SET r.cost = $c";
    let updates: Vec<Params> = (0..runs)
        .map(|_| {
            params(vec![
                ("id", Value::Integer(upd_rng.next(rel_count) as i64)),
                ("c", Value::Integer(1 + upd_rng.next(100) as i64)),
            ])
        })
        .collect();
    time_each(&db, "SET r.cost by relationship id", update, updates.clone());

    if weighted_ok {
        // an update before each query: what a path cache has to cope with
        let mut times = Vec::with_capacity(runs);
        let query = pair_params(&pairs[..1]).remove(0);
        for u in updates {
            db.execute(update, u)?;
            let t = Instant::now();
            db.execute(weighted, query.clone())?;
            times.push(t.elapsed());
        }
        report("weighted, same pair after each update", &Stats::of(times), "");
    }

    println!("\n-- concurrent shortestPath (hops *..30), random pairs, 2 s per row");
    let hops =
        "MATCH (a:Item {key: $a}), (b:Item {key: $b}), p = shortestPath((a)-[*..30]->(b)) RETURN length(p) AS len";
    for writer in [false, true] {
        for threads in [1usize, 2, 4, 8] {
            let (qps, writes) = concurrent(&db, hops, n as u64, rel_count, threads, writer);
            let w = if writer { format!(", 1 writer: {writes:.0} SET/s") } else { String::new() };
            println!("{threads} reader thread(s){w:<28} {qps:>9.0} queries/s");
        }
    }
    Ok(())
}

/// Runs `query` on random `k{n}` pairs from `threads` threads for 2 s, plus
/// optionally a thread setting random relationship costs. Returns (queries
/// per second, writes per second).
fn concurrent(db: &Database, query: &str, n: u64, rels: u64, threads: usize, writer: bool) -> (f64, f64) {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    let stop = AtomicBool::new(false);
    let (queries, writes) = (AtomicU64::new(0), AtomicU64::new(0));
    let window = Duration::from_secs(2);
    std::thread::scope(|s| {
        for t in 0..threads {
            let (stop, queries) = (&stop, &queries);
            s.spawn(move || {
                let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (t as u64 + 1).wrapping_mul(0xD1B5_4A32_D192_ED03));
                while !stop.load(Ordering::Relaxed) {
                    let (a, b) = (format!("k{}", rng.next(n)), format!("k{}", rng.next(n)));
                    db.execute(query, params(vec![("a", Value::from(a)), ("b", Value::from(b))])).expect("query");
                    queries.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        if writer {
            let (stop, writes) = (&stop, &writes);
            s.spawn(move || {
                let mut rng = Rng(0x2545_F491_4F6C_DD1D);
                while !stop.load(Ordering::Relaxed) {
                    let p = params(vec![
                        ("id", Value::Integer(rng.next(rels) as i64)),
                        ("c", Value::Integer(1 + rng.next(100) as i64)),
                    ]);
                    db.execute("MATCH ()-[r]->() WHERE id(r) = $id SET r.cost = $c", p).expect("update");
                    writes.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        std::thread::sleep(window);
        stop.store(true, Ordering::Relaxed);
    });
    let secs = window.as_secs_f64();
    (queries.into_inner() as f64 / secs, writes.into_inner() as f64 / secs)
}
