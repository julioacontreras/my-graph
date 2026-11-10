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
