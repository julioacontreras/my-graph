//! End-to-end tests against a real server on an ephemeral port.

use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

use mygraph_client::{params, Client, Error, Value};

fn start() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let db = Arc::new(mygraph::Database::in_memory().unwrap());
    thread::spawn(move || mygraph_server::serve(listener, db));
    addr
}

#[test]
fn handshake_and_ping() {
    let mut c = Client::connect(start()).unwrap();
    assert_eq!(c.server_info().protocol, 1);
    c.ping().unwrap();
    c.close();
}

#[test]
fn crud_with_params() {
    let mut c = Client::connect(start()).unwrap();
    let r = c
        .run_with("CREATE (u:User {name: $name, age: $age}) RETURN u", params! { "name" => "Alice", "age" => 25 })
        .unwrap();
    assert_eq!(r.stats.nodes_created, 1);
    let node = r.single().and_then(Value::as_node).expect("node");
    assert_eq!(node.labels, vec!["User".to_string()]);
    assert_eq!(node.properties.get("age"), Some(&Value::Integer(25)));

    let r = c.run("MATCH (u:User) RETURN u.name AS name").unwrap();
    assert_eq!(r.columns, vec!["name"]);
    assert_eq!(r.value(0, "name"), Some(&Value::from("Alice")));
    assert!(!r.database.is_empty());
}

#[test]
fn query_errors_keep_the_connection() {
    let mut c = Client::connect(start()).unwrap();
    let e = c.run("MATCH (n RETURN n").unwrap_err();
    let q = e.as_query().expect("query error");
    assert!(q.is_syntax(), "{q:?}");
    assert!(q.line.is_some());
    assert!(!c.is_closed());
    c.run("RETURN 1").unwrap();
}

#[test]
fn transactions_commit_and_roll_back() {
    let mut c = Client::connect(start()).unwrap();

    let mut tx = c.begin().unwrap();
    tx.run("CREATE (:T {v: 1})").unwrap();
    tx.commit().unwrap();

    let mut tx = c.begin().unwrap();
    tx.run("CREATE (:T {v: 2})").unwrap();
    tx.rollback().unwrap();

    {
        let mut tx = c.begin().unwrap();
        tx.run("CREATE (:T {v: 3})").unwrap();
    } // dropped: rolled back

    let r: Result<(), Error> = c.transaction(|tx| {
        tx.run("CREATE (:T {v: 4})")?;
        Ok(())
    });
    r.unwrap();

    let r = c.run("MATCH (t:T) RETURN t.v AS v ORDER BY v").unwrap();
    let vs: Vec<_> = r.rows.iter().map(|row| row[0].clone()).collect();
    assert_eq!(vs, vec![Value::Integer(1), Value::Integer(4)]);
    assert!(!r.in_transaction);
}

#[test]
fn databases_and_schemas() {
    let mut c = Client::connect(start()).unwrap();
    c.create_database("shop").unwrap();
    c.use_database("shop").unwrap();
    let r = c.run("RETURN 1").unwrap();
    assert_eq!(r.database, "shop");
}

#[test]
fn connect_failure_is_a_connection_error() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    match Client::connect(("127.0.0.1", port)) {
        Err(Error::Connection(_)) => {}
        other => panic!("{other:?}"),
    }
}
