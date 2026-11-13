//! Volume test modelled on the benchmark report's tree: a root and `levels`
//! levels below it, `fanout` children per node (defaults 4 and 32, i.e.
//! 1,082,401 nodes and 1,082,400 relationships), loaded in batches of 10,000
//! into an on-disk database, then traversed and modified.
//!
//! ```text
//! cargo run --release -p mygraph --example tree_volume -- [--levels N] [--fanout N] [--dir PATH]
//! ```
//!
//! Prints the resident memory (RSS, from `ps`) as the load progresses. For the
//! peak, run it under `/usr/bin/time -l` (macOS) or `/usr/bin/time -v` (Linux).

use std::path::PathBuf;
use std::time::Instant;

use mygraph::{Config, Database, Params, Value};

const BATCH: usize = 10_000;

fn params(pairs: Vec<(&str, Value)>) -> Params {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn map(pairs: Vec<(&str, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Resident memory of this process in MB, or 0 if `ps` is unavailable.
fn rss_mb() -> f64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map_or(0.0, |kb| kb / 1024.0)
}

fn dir_size_mb(dir: &std::path::Path) -> f64 {
    std::fs::read_dir(dir)
        .map(|it| it.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum::<u64>())
        .unwrap_or(0) as f64
        / (1024.0 * 1024.0)
}

fn arg<T: std::str::FromStr>(args: &[String], name: &str) -> Option<T> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok())
}

fn main() -> mygraph::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let levels: u32 = arg(&args, "--levels").unwrap_or(4);
    let fanout: u64 = arg(&args, "--fanout").unwrap_or(32);
    let tmp = tempfile::tempdir()?;
    let dir: PathBuf = arg(&args, "--dir").unwrap_or_else(|| tmp.path().to_path_buf());

    // Nodes numbered breadth-first: node i (i > 0) has parent (i - 1) / fanout.
    let mut per_level = vec![1u64];
    for l in 1..=levels {
        per_level.push(fanout.pow(l));
    }
    let total: u64 = per_level.iter().sum();
    let level_of = |i: u64| {
        let mut start = 0;
        for (l, n) in per_level.iter().enumerate() {
            if i < start + n {
                return l as i64;
            }
            start += n;
        }
        unreachable!()
    };

    let db = Database::open(Config::default().storage_path(&dir))?;
    db.execute("CREATE INDEX FOR (n:Node) ON (n.id)", Params::new())?;
    println!("tree: {levels} levels x {fanout} children = {total} nodes, {} relationships", total - 1);
    println!("start: rss {:.0} MB", rss_mb());

    let start = Instant::now();
    let mut next_report = 100_000;
    for lo in (0..total).step_by(BATCH) {
        let hi = (lo + BATCH as u64).min(total);
        let rows = (lo..hi)
            .map(|i| {
                map(vec![
                    ("id", Value::Integer(i as i64)),
                    ("level", Value::Integer(level_of(i))),
                    ("name", Value::from(format!("node-{i}"))),
                ])
            })
            .collect();
        db.execute(
            "UNWIND $rows AS r CREATE (:Node {id: r.id, level: r.level, name: r.name})",
            params(vec![("rows", Value::List(rows))]),
        )?;
        if hi >= next_report {
            println!("  nodes {hi:>9}: rss {:>6.0} MB, disk {:>5.0} MB", rss_mb(), dir_size_mb(&dir));
            next_report += 100_000;
        }
    }
    let nodes_time = start.elapsed();
    println!("nodes loaded in {:.1} s: rss {:.0} MB", nodes_time.as_secs_f64(), rss_mb());

    let rel_start = Instant::now();
    next_report = 100_000;
    for lo in (1..total).step_by(BATCH) {
        let hi = (lo + BATCH as u64).min(total);
        let rows = (lo..hi)
            .map(|i| map(vec![("p", Value::Integer(((i - 1) / fanout) as i64)), ("c", Value::Integer(i as i64))]))
            .collect();
        db.execute(
            "UNWIND $rows AS e MATCH (p:Node {id: e.p}), (c:Node {id: e.c}) CREATE (p)-[:CHILD]->(c)",
            params(vec![("rows", Value::List(rows))]),
        )?;
        if hi >= next_report {
            println!("  rels  {hi:>9}: rss {:>6.0} MB, disk {:>5.0} MB", rss_mb(), dir_size_mb(&dir));
            next_report += 100_000;
        }
    }
    println!(
        "relationships loaded in {:.1} s; total load {:.1} s: rss {:.0} MB",
        rel_start.elapsed().as_secs_f64(),
        start.elapsed().as_secs_f64(),
        rss_mb()
    );

    let q = |name: &str, query: &str| -> mygraph::Result<()> {
        let t = Instant::now();
        let r = db.execute(query, Params::new())?;
        let first = r.rows.first().and_then(|row| row.first()).cloned();
        println!("{name:<28} {:>9.1} ms  rows {:>8}  first {first:?}", t.elapsed().as_secs_f64() * 1e3, r.len());
        Ok(())
    };
    let depth = levels;
    q("T1 full traversal", &format!("MATCH (:Node {{id: 0}})-[:CHILD*1..{depth}]->(d) RETURN count(d)"))?;
    q("T2 level-1 subtree", &format!("MATCH (:Node {{id: 1}})-[:CHILD*1..{}]->(d) RETURN count(d)", depth - 1))?;
    q("T3 leaf to root", &format!("MATCH (:Node {{id: {}}})<-[:CHILD*1..{depth}]-(a) RETURN count(a)", total - 1))?;
    q("T4 grandchildren", "MATCH (:Node {id: 0})-[:CHILD]->()-[:CHILD]->(g) RETURN count(g)")?;
    q("T5 count per level", "MATCH (n:Node) RETURN n.level AS level, count(n) AS n ORDER BY level")?;
    q("TU1 mark subtree", "MATCH (:Node {id: 33})-[:CHILD*1..2]->(d) SET d.marked = true RETURN count(d)")?;
    q("TU1r unmark", "MATCH (:Node {id: 33})-[:CHILD*1..2]->(d) REMOVE d.marked RETURN count(d)")?;
    println!("after queries: rss {:.0} MB, disk {:.0} MB", rss_mb(), dir_size_mb(&dir));

    let t = Instant::now();
    db.checkpoint()?;
    println!("checkpoint {:.1} s: rss {:.0} MB, disk {:.0} MB", t.elapsed().as_secs_f64(), rss_mb(), dir_size_mb(&dir));
    drop(db);

    let t = Instant::now();
    let db = Database::open(Config::default().storage_path(&dir))?;
    let n = db.execute("MATCH (n:Node) RETURN count(n)", Params::new())?;
    println!(
        "reopen {:.1} s: {:?} nodes, rss {:.0} MB",
        t.elapsed().as_secs_f64(),
        n.rows.first().and_then(|r| r.first()),
        rss_mb()
    );
    Ok(())
}
