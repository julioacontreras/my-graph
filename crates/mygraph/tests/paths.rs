//! Variable-length relationships, named paths and shortest paths.

use mygraph::{params, Database, Params, Value};

fn rows(db: &Database, q: &str) -> Vec<Vec<Value>> {
    db.execute(q, Params::new()).unwrap_or_else(|e| panic!("{q}: {e}")).rows
}

fn column(db: &Database, q: &str) -> Vec<Value> {
    rows(db, q).into_iter().map(|mut r| r.remove(0)).collect()
}

fn strs(items: &[&str]) -> Vec<Value> {
    items.iter().map(|s| Value::from(*s)).collect()
}

/// a -NEXT-> b -NEXT-> c -NEXT-> d, plus a -JUMP-> d, d -BACK-> a (a cycle),
/// and e -NEXT-> b. Optionally indexed on `:N(n)`, so the planner reverses
/// and splits paths; results must not change.
fn graph(indexed: bool) -> Database {
    let db = Database::in_memory().unwrap();
    if indexed {
        db.execute("CREATE INDEX ON :N(n)", params! {}).unwrap();
    }
    db.execute(
        "CREATE (a:N {n: 'a'}), (b:N {n: 'b'}), (c:N {n: 'c'}), (d:N {n: 'd'}), (e:N {n: 'e'}),
                (a)-[:NEXT {w: 1}]->(b), (b)-[:NEXT {w: 2}]->(c), (c)-[:NEXT {w: 1}]->(d),
                (a)-[:JUMP]->(d), (d)-[:BACK]->(a), (e)-[:NEXT {w: 5}]->(b)",
        params! {},
    )
    .unwrap();
    db
}

fn both(check: impl Fn(&Database)) {
    check(&graph(false));
    check(&graph(true));
}

#[test]
fn variable_length_ranges() {
    both(|db| {
        let q = |r: &str| column(db, &format!("MATCH (:N {{n: 'a'}})-[:NEXT{r}]->(x) RETURN x.n ORDER BY x.n"));
        assert_eq!(q("*"), strs(&["b", "c", "d"]));
        assert_eq!(q("*1..2"), strs(&["b", "c"]));
        assert_eq!(q("*2"), strs(&["c"]));
        assert_eq!(q("*2.."), strs(&["c", "d"]));
        assert_eq!(q("*..1"), strs(&["b"]));
        assert_eq!(q("*0..1"), strs(&["a", "b"]));
        assert_eq!(q("*4.."), strs(&[]));
    });
}

#[test]
fn variable_length_with_types_properties_and_directions() {
    both(|db| {
        assert_eq!(
            column(db, "MATCH (:N {n: 'a'})-[:NEXT|JUMP*1..2]->(x) RETURN x.n ORDER BY x.n"),
            strs(&["b", "c", "d"])
        );
        assert_eq!(column(db, "MATCH (:N {n: 'a'})-[:NEXT*1..3 {w: 1}]->(x) RETURN x.n"), strs(&["b"]));
        assert_eq!(
            column(db, "MATCH (:N {n: 'd'})<-[:NEXT*]-(x) RETURN x.n ORDER BY x.n"),
            strs(&["a", "b", "c", "e"])
        );
        assert_eq!(column(db, "MATCH (:N {n: 'c'})-[:NEXT*2]-(x) RETURN x.n ORDER BY x.n"), strs(&["a", "e"]));
    });
}

#[test]
fn cycles_terminate_by_relationship_uniqueness() {
    both(|db| {
        // every relationship at most once: b, c, d, a, d (a-b-c-d-a-d) and
        // d, a, b, c, d (a-d-a-b-c-d)
        let ends = column(db, "MATCH (:N {n: 'a'})-[*]->(x) RETURN x.n ORDER BY x.n");
        assert_eq!(ends, strs(&["a", "a", "b", "b", "c", "c", "d", "d", "d", "d"]));
        assert_eq!(column(db, "MATCH (a:N {n: 'a'})-[*]->(a) RETURN count(*)"), vec![Value::Integer(2)]);
    });
}

#[test]
fn relationship_list_is_in_written_order() {
    both(|db| {
        let r = rows(
            db,
            "MATCH (x)-[r:NEXT*]->(:N {n: 'd'}) WHERE x.n = 'a'
             RETURN size(r), startNode(r[0]).n, endNode(r[2]).n, r[1].w",
        );
        assert_eq!(r, vec![vec![3.into(), "a".into(), "d".into(), 2.into()]]);
        // a bound list from an earlier clause must be matched exactly
        let r = rows(db, "MATCH (:N {n: 'a'})-[r:NEXT*2]->(y) MATCH (:N {n: 'a'})-[r*]->(z) RETURN y.n, z.n");
        assert_eq!(r, vec![vec!["c".into(), "c".into()]]);
    });
}

#[test]
fn where_on_variable_length_matches() {
    both(|db| {
        assert_eq!(column(db, "MATCH (a:N)-[r:NEXT*]->(b:N) WHERE a.n = 'a' AND size(r) = 2 RETURN b.n"), strs(&["c"]));
        assert_eq!(
            column(db, "MATCH (a:N)-[:NEXT*]->(b:N) WHERE b.n = 'd' RETURN a.n ORDER BY a.n"),
            strs(&["a", "b", "c", "e"])
        );
    });
}

fn path_names(v: &Value) -> Vec<String> {
    let p = v.as_path().expect("a path");
    assert_eq!(p.nodes.len(), p.relationships.len() + 1);
    p.nodes.iter().map(|n| n.properties["n"].as_str().unwrap().to_string()).collect()
}

#[test]
fn named_paths() {
    both(|db| {
        let r = rows(db, "MATCH p = (:N {n: 'a'})-[:NEXT*]->(:N {n: 'd'}) RETURN p, length(p)");
        assert_eq!(r.len(), 1);
        assert_eq!(path_names(&r[0][0]), ["a", "b", "c", "d"]);
        assert_eq!(r[0][1], Value::Integer(3));

        // fixed-length, incoming direction, path functions
        let r = rows(
            db,
            "MATCH p = (b:N {n: 'b'})<-[:NEXT]-(x) RETURN x.n, length(p), size(nodes(p)), type(relationships(p)[0])
             ORDER BY x.n",
        );
        assert_eq!(
            r,
            vec![
                vec!["a".into(), 1.into(), 2.into(), "NEXT".into()],
                vec!["e".into(), 1.into(), 2.into(), "NEXT".into()]
            ]
        );
        // a single-node path, and WHERE on the path
        assert_eq!(column(db, "MATCH p = (:N {n: 'e'}) RETURN length(p)"), vec![Value::Integer(0)]);
        assert_eq!(
            column(db, "MATCH p = (:N {n: 'a'})-[*1..3]->(x) WHERE length(p) = 3 RETURN x.n ORDER BY x.n"),
            strs(&["b", "d"]) // a-d-a-b and a-b-c-d
        );
        let shown = column(db, "MATCH p = (:N {n: 'c'})-[:NEXT]->(:N)-[:BACK]->(:N) RETURN p");
        assert_eq!(shown[0].to_string(), "(:N {n: \"c\"})-[:NEXT {w: 1}]->(:N {n: \"d\"})-[:BACK]->(:N {n: \"a\"})");
    });
}

#[test]
fn shortest_paths() {
    both(|db| {
        let one = |q: &str| rows(db, q);
        let r = one("MATCH (a:N {n: 'a'}), (d:N {n: 'd'}), p = shortestPath((a)-[*]->(d)) RETURN length(p)");
        assert_eq!(r, vec![vec![Value::Integer(1)]]);
        let r = one("MATCH (a:N {n: 'a'}), (d:N {n: 'd'}), p = shortestPath((a)-[:NEXT*]->(d)) RETURN p");
        assert_eq!(path_names(&r[0][0]), ["a", "b", "c", "d"]);
        // max length shorter than the distance: no path
        assert!(one("MATCH (a:N {n: 'a'}), (d:N {n: 'd'}), p = shortestPath((a)-[:NEXT*..2]->(d)) RETURN p").is_empty());
        // unreachable, and OPTIONAL MATCH keeps the row
        assert!(one("MATCH (d:N {n: 'd'}), (e:N {n: 'e'}), p = shortestPath((d)-[:NEXT*]->(e)) RETURN p").is_empty());
        let r = one("MATCH (d:N {n: 'd'}), (e:N {n: 'e'}) OPTIONAL MATCH p = shortestPath((d)-[:NEXT*]->(e)) RETURN p");
        assert_eq!(r, vec![vec![Value::Null]]);
        // undirected, written with the end nodes after the shortest pattern
        let r = one("MATCH p = shortestPath((x)-[*]-(y)), (x:N {n: 'e'}), (y:N {n: 'd'}) RETURN length(p)");
        assert_eq!(r, vec![vec![Value::Integer(3)]]);
        // same start and end node
        assert!(one("MATCH (a:N {n: 'a'}), p = shortestPath((a)-[*]->(a)) RETURN p").is_empty());
        let r = one("MATCH (a:N {n: 'a'}), p = shortestPath((a)-[*0..]->(a)) RETURN length(p)");
        assert_eq!(r, vec![vec![Value::Integer(0)]]);
        // relationship variable of a shortest path
        let r = one("MATCH (a:N {n: 'a'}), (c:N {n: 'c'}), shortestPath((a)-[r:NEXT*]->(c)) RETURN size(r), r[0].w");
        assert_eq!(r, vec![vec![2.into(), 1.into()]]);
    });
}

#[test]
fn all_shortest_paths() {
    let db = Database::in_memory().unwrap();
    db.execute(
        "CREATE (s:V {n: 's'}), (a:V {n: 'a'}), (b:V {n: 'b'}), (t:V {n: 't'}), (long:V {n: 'l'}),
                (s)-[:E]->(a), (s)-[:E]->(b), (a)-[:E]->(t), (b)-[:E]->(t),
                (s)-[:E]->(long), (long)-[:E]->(long2:V {n: 'l2'}), (long2)-[:E]->(t)",
        params! {},
    )
    .unwrap();
    let r = rows(&db, "MATCH (s:V {n: 's'}), (t:V {n: 't'}), p = allShortestPaths((s)-[*]->(t)) RETURN p");
    let mut paths: Vec<Vec<String>> = r.iter().map(|row| path_names(&row[0])).collect();
    paths.sort();
    assert_eq!(paths, vec![vec!["s", "a", "t"], vec!["s", "b", "t"]]);
    let r = rows(&db, "MATCH (s:V {n: 's'}), (t:V {n: 't'}), p = shortestPath((s)-[*]->(t)) RETURN length(p)");
    assert_eq!(r, vec![vec![Value::Integer(2)]]);
}

#[test]
fn invalid_path_syntax() {
    let db = graph(false);
    for q in [
        "MATCH (a)-[*3..1]->(b) RETURN a",
        "MATCH p = shortestPath((a)-->(b)-->(c)) RETURN p",
        "MATCH p = shortestPath((a)-[*2..]->(b)) RETURN p",
        "CREATE (a)-[:R*2]->(b)",
        "CREATE p = (a)-[:R]->(b)",
        "MATCH (n:N) CREATE (m {p: n})",
    ] {
        assert!(db.execute(q, params! {}).is_err(), "{q} should fail");
    }
    // a path cannot be stored as a property
    assert!(db.execute("MATCH p = (n:N {n: 'a'}) SET n.p = p", params! {}).is_err());
}

#[test]
fn bidirectional_shortest_path_agrees_with_breadth_first_search() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE INDEX ON :V(i)", params! {}).unwrap();
    db.execute("UNWIND range(0, 199) AS i CREATE (:V {i: i})", params! {}).unwrap();
    // deterministic pseudo-random edges
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = |n: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % n) as i64
    };
    let edges: Vec<Value> =
        (0..360).map(|_| Value::List(vec![Value::Integer(next(200)), Value::Integer(next(200))])).collect();
    db.execute(
        "UNWIND $edges AS e MATCH (a:V {i: e[0]}), (b:V {i: e[1]}) CREATE (a)-[:E]->(b)",
        [("edges".to_string(), Value::List(edges))].into(),
    )
    .unwrap();
    for _ in 0..80 {
        let (a, b) = (next(200), next(200));
        for dir in ["->", "-"] {
            let p: Params = [("a".to_string(), Value::Integer(a)), ("b".to_string(), Value::Integer(b))].into();
            let one = db
                .execute(
                    &format!("MATCH (a:V {{i: $a}}), (b:V {{i: $b}}), p = shortestPath((a)-[*]{dir}(b)) RETURN p"),
                    p.clone(),
                )
                .unwrap()
                .rows;
            let all = db
                .execute(
                    &format!("MATCH (a:V {{i: $a}}), (b:V {{i: $b}}), p = allShortestPaths((a)-[*]{dir}(b)) RETURN length(p)"),
                    p,
                )
                .unwrap()
                .rows;
            assert_eq!(one.len(), usize::from(!all.is_empty() && a != b), "{a} {dir} {b}");
            if let Some(row) = one.first() {
                let path = row[0].as_path().unwrap();
                assert_eq!(Value::Integer(path.relationships.len() as i64), all[0][0], "{a} {dir} {b}");
                assert_eq!(path.nodes.first().unwrap().properties["i"], Value::Integer(a));
                assert_eq!(path.nodes.last().unwrap().properties["i"], Value::Integer(b));
                for (i, r) in path.relationships.iter().enumerate() {
                    let (u, v) = (path.nodes[i].id, path.nodes[i + 1].id);
                    let forward = r.start_node == u && r.end_node == v;
                    assert!(forward || (dir == "-" && r.start_node == v && r.end_node == u), "broken path");
                }
            }
        }
    }
}

/// Total weight of the single path a `... p = shortestPath(...)` query finds,
/// or `None` when it finds none. `q` must bind the path as `p`.
fn path_cost(db: &Database, q: &str, p: Params) -> Option<f64> {
    let q = format!("{q} UNWIND relationships(p) AS r RETURN count(*) AS n, sum(r.cost) AS c");
    let rows = db.execute(&q, p).unwrap_or_else(|e| panic!("{q}: {e}")).rows;
    match rows.first().map(|r| (&r[0], &r[1])) {
        Some((Value::Integer(0), _)) | None => None,
        Some((_, Value::Integer(c))) => Some(*c as f64),
        Some((_, Value::Float(c))) => Some(*c),
        other => panic!("unexpected row {other:?}"),
    }
}

/// s -[1]-> x -[2]-> t, s -[10]-> t (direct but heavy), t -[1]-> s, and an
/// isolated node u. Every relationship is `:R` with a `cost`.
fn weighted() -> Database {
    let db = Database::in_memory().unwrap();
    db.execute(
        "CREATE (s:W {n: 's'}), (x:W {n: 'x'}), (t:W {n: 't'}), (:W {n: 'u'}),
                (s)-[:R {cost: 1}]->(x), (x)-[:R {cost: 2}]->(t), (s)-[:R {cost: 10}]->(t), (t)-[:R {cost: 1}]->(s)",
        params! {},
    )
    .unwrap();
    db
}

const LIGHTEST_S_T: &str =
    "MATCH (s:W {n: 's'}), (t:W {n: 't'}), p = shortestPath((s)-[:R*]->(t), {weight: 'cost'}) RETURN p";

#[test]
fn weighted_shortest_path_prefers_the_lighter_route() {
    let db = weighted();
    // by hops the direct relationship wins; by weight the detour does
    let hops = rows(&db, "MATCH (s:W {n: 's'}), (t:W {n: 't'}), p = shortestPath((s)-[:R*]->(t)) RETURN p");
    assert_eq!(path_names(&hops[0][0]), ["s", "t"]);
    let r = rows(&db, LIGHTEST_S_T);
    assert_eq!(path_names(&r[0][0]), ["s", "x", "t"]);
    let q = "MATCH (s:W {n: 's'}), (t:W {n: 't'}), p = shortestPath((s)-[:R*]->(t), {weight: 'cost'})";
    assert_eq!(path_cost(&db, q, params! {}), Some(3.0));

    // the result follows weight changes
    db.execute("MATCH (:W {n: 's'})-[r:R]->(:W {n: 't'}) SET r.cost = 2.5", params! {}).unwrap();
    assert_eq!(path_names(&rows(&db, LIGHTEST_S_T)[0][0]), ["s", "t"]);
    assert_eq!(path_cost(&db, q, params! {}), Some(2.5));

    // undirected: t -> s directly weighs 1 (t-[1]->s), against t<-[2.5]-s
    let q = "MATCH (s:W {n: 's'}), (t:W {n: 't'}), p = shortestPath((t)-[*]-(s), {weight: 'cost'})";
    assert_eq!(path_cost(&db, q, params! {}), Some(1.0));
    // against the relationships' direction
    let q = "MATCH (s:W {n: 's'}), (x:W {n: 'x'}), p = shortestPath((x)<-[*]-(s), {weight: 'cost'})";
    assert_eq!(path_cost(&db, q, params! {}), Some(1.0));
}

#[test]
fn weighted_shortest_path_edge_cases() {
    let db = weighted();
    let one = |q: &str| rows(&db, q);
    // no path, and OPTIONAL MATCH keeps the row
    let q = "MATCH (s:W {n: 's'}), (u:W {n: 'u'}), p = shortestPath((s)-[*]->(u), {weight: 'cost'}) RETURN p";
    assert!(one(q).is_empty());
    let q =
        "MATCH (s:W {n: 's'}), (u:W {n: 'u'}) OPTIONAL MATCH p = shortestPath((s)-[*]->(u), {weight: 'cost'}) RETURN p";
    assert_eq!(one(q), vec![vec![Value::Null]]);
    // same start and end node
    assert!(one("MATCH (s:W {n: 's'}), p = shortestPath((s)-[*]->(s), {weight: 'cost'}) RETURN p").is_empty());
    let q = "MATCH (s:W {n: 's'}), p = shortestPath((s)-[*0..]->(s), {weight: 'cost'}) RETURN length(p)";
    assert_eq!(one(q), vec![vec![Value::Integer(0)]]);
    // relationships of another type are not followed
    let q = "MATCH (s:W {n: 's'}), (t:W {n: 't'}), p = shortestPath((s)-[:OTHER*]->(t), {weight: 'cost'}) RETURN p";
    assert!(one(q).is_empty());
    // the relationship variable holds the list of relationships
    let q = "MATCH (s:W {n: 's'}), (t:W {n: 't'}), shortestPath((s)-[rs*]->(t), {weight: 'cost'}) RETURN size(rs)";
    assert_eq!(one(q), vec![vec![Value::Integer(2)]]);
    // a relationship of weight 0
    db.execute("MATCH (:W {n: 's'})-[r:R]->(:W {n: 'x'}) SET r.cost = 0", params! {}).unwrap();
    let q = "MATCH (s:W {n: 's'}), (t:W {n: 't'}), p = shortestPath((s)-[*]->(t), {weight: 'cost'})";
    assert_eq!(path_cost(&db, q, params! {}), Some(2.0));
}

#[test]
fn weighted_shortest_path_rejects_bad_weights() {
    let q = "MATCH (s:W {n: 's'}), (t:W {n: 't'}), p = shortestPath((s)-[*]->(t), {weight: 'cost'}) RETURN p";
    for bad in ["-1", "'heavy'", "null", "[1]"] {
        let db = weighted();
        db.execute(&format!("MATCH (:W {{n: 's'}})-[r:R]->(:W {{n: 'x'}}) SET r.cost = {bad}"), params! {}).unwrap();
        let err = db.execute(q, params! {}).expect_err(bad).to_string();
        assert!(err.contains("weight"), "{bad}: {err}");
    }
    // a relationship the search never reaches does not matter
    let db = weighted();
    db.execute("MATCH (u:W {n: 'u'}) CREATE (u)-[:R {cost: -5}]->(u)", params! {}).unwrap();
    assert_eq!(path_names(&rows(&db, LIGHTEST_S_T)[0][0]), ["s", "x", "t"]);
}

#[test]
fn invalid_weighted_shortest_path_syntax() {
    let db = weighted();
    for q in [
        "MATCH p = allShortestPaths((a)-[*]->(b), {weight: 'cost'}) RETURN p",
        "MATCH p = shortestPath((a)-[*..3]->(b), {weight: 'cost'}) RETURN p",
        "MATCH p = shortestPath((a)-[r]->(b), {weight: 'cost'}) RETURN p",
        "MATCH p = shortestPath((a)-[*]->(b), {weight: cost}) RETURN p",
        "MATCH p = shortestPath((a)-[*]->(b), {weight: 1}) RETURN p",
        "MATCH p = shortestPath((a)-[*]->(b), {cost: 'cost'}) RETURN p",
        "MATCH p = shortestPath((a)-[*]->(b), {}) RETURN p",
    ] {
        assert!(db.execute(q, params! {}).is_err(), "{q} should fail");
    }
}

/// Reference: Dijkstra over an explicit edge list, O(V^2).
fn reference_distance(n: usize, edges: &[(usize, usize, i64)], s: usize, t: usize, undirected: bool) -> Option<i64> {
    let mut dist = vec![i64::MAX; n];
    let mut done = vec![false; n];
    dist[s] = 0;
    while let Some(u) = (0..n).filter(|&i| !done[i] && dist[i] != i64::MAX).min_by_key(|&i| dist[i]) {
        done[u] = true;
        for &(a, b, w) in edges {
            for (from, to) in [(a, b), (b, a)].into_iter().take(if undirected { 2 } else { 1 }) {
                if from == u && dist[u] + w < dist[to] {
                    dist[to] = dist[u] + w;
                }
            }
        }
    }
    (dist[t] != i64::MAX).then_some(dist[t])
}

#[test]
fn weighted_shortest_path_agrees_with_a_reference_dijkstra() {
    let db = Database::in_memory().unwrap();
    db.execute("CREATE INDEX ON :V(i)", params! {}).unwrap();
    let n = 120;
    db.execute("UNWIND range(0, 119) AS i CREATE (:V {i: i})", params! {}).unwrap();
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = |m: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % m
    };
    let edges: Vec<(usize, usize, i64)> =
        (0..300).map(|_| (next(n as u64) as usize, next(n as u64) as usize, next(20) as i64)).collect();
    let list = edges
        .iter()
        .map(|&(a, b, w)| Value::List(vec![(a as i64).into(), (b as i64).into(), w.into()]))
        .collect::<Vec<_>>();
    db.execute(
        "UNWIND $edges AS e MATCH (a:V {i: e[0]}), (b:V {i: e[1]}) CREATE (a)-[:E {cost: e[2]}]->(b)",
        [("edges".to_string(), Value::List(list))].into(),
    )
    .unwrap();
    for _ in 0..100 {
        let (s, t) = (next(n as u64) as usize, next(n as u64) as usize);
        if s == t {
            continue;
        }
        for (dir, undirected) in [("->", false), ("-", true)] {
            let q = format!(
                "MATCH (a:V {{i: $s}}), (b:V {{i: $t}}), p = shortestPath((a)-[*]{dir}(b), {{weight: 'cost'}})"
            );
            let p: Params = [("s".to_string(), Value::from(s as i64)), ("t".to_string(), Value::from(t as i64))].into();
            let found = path_cost(&db, &q, p);
            let expected = reference_distance(n, &edges, s, t, undirected).map(|d| d as f64);
            // a found path of weight 0 has relationships, so path_cost sees it
            assert_eq!(found, expected, "{s} {dir} {t}");
        }
    }
}
