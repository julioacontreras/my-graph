//! End-to-end tests: a real server on an ephemeral port, driven over TCP with
//! the protocol crate.

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use mygraph::{Database, PropertyMap, Value};
use mygraph_protocol::{read_frame, write_frame, MessageType, Payload};

struct Client {
    stream: TcpStream,
    next_id: u32,
}

impl Client {
    fn connect(addr: &str) -> Client {
        let mut c = Client { stream: TcpStream::connect(addr).unwrap(), next_id: 1 };
        let (t, p) = c.request(MessageType::Hello, Payload::new().with("client", "test"));
        assert_eq!(t, MessageType::Hello);
        assert_eq!(p.get("protocol"), Some(&Value::Integer(1)));
        c
    }

    fn request(&mut self, t: MessageType, p: Payload) -> (MessageType, Payload) {
        let id = self.next_id;
        self.next_id += 1;
        write_frame(&mut self.stream, t as u8, id, &p.encode().unwrap()).unwrap();
        let f = read_frame(&mut self.stream).unwrap().expect("response");
        assert_eq!(f.request_id, id);
        (MessageType::from_u8(f.message_type).unwrap(), Payload::decode(&f.payload).unwrap())
    }

    fn query(&mut self, q: &str, params: PropertyMap) -> (MessageType, Payload) {
        self.request(MessageType::Query, Payload::new().with("query", q).with("params", Value::Map(params)))
    }

    fn rows(&mut self, q: &str) -> Vec<Value> {
        let (t, p) = self.query(q, PropertyMap::new());
        assert_eq!(t, MessageType::QueryResult, "{p:?}");
        match p.get("rows") {
            Some(Value::List(rows)) => rows.clone(),
            other => panic!("rows: {other:?}"),
        }
    }
}

fn start() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let db = Arc::new(Database::in_memory().unwrap());
    thread::spawn(move || mygraph_server::serve(listener, db));
    addr
}

#[test]
fn crud_over_the_wire() {
    let addr = start();
    let mut c = Client::connect(&addr);
    let params = PropertyMap::from([("name".into(), Value::from("Alice")), ("age".into(), Value::from(25))]);
    let (t, p) = c.query("CREATE (u:User {name: $name, age: $age}) RETURN u", params);
    assert_eq!(t, MessageType::QueryResult);
    let Some(Value::List(rows)) = p.get("rows") else { panic!() };
    let Value::List(row) = &rows[0] else { panic!() };
    let node = row[0].as_node().expect("node value");
    assert_eq!(node.labels, vec!["User".to_string()]);
    assert_eq!(node.properties.get("age"), Some(&Value::Integer(25)));
    let Some(Value::Map(stats)) = p.get("stats") else { panic!() };
    assert_eq!(stats.get("nodes_created"), Some(&Value::Integer(1)));

    c.rows("MATCH (u:User) SET u.age = 26");
    assert_eq!(c.rows("MATCH (u:User) RETURN u.age"), vec![Value::List(vec![Value::Integer(26)])]);
    c.rows("MATCH (u:User) DETACH DELETE u");
    assert_eq!(c.rows("MATCH (u:User) RETURN count(u)"), vec![Value::List(vec![Value::Integer(0)])]);
}

#[test]
fn errors_keep_the_connection_open() {
    let addr = start();
    let mut c = Client::connect(&addr);
    let (t, p) = c.query("MATCH (n RETURN n", PropertyMap::new());
    assert_eq!(t, MessageType::Error);
    assert_eq!(p.get("code"), Some(&Value::from("Syntax")));
    assert!(p.get("line").is_some());
    let (t, _) = c.request(MessageType::Ping, Payload::new());
    assert_eq!(t, MessageType::Pong);
}

#[test]
fn transactions_commit_rollback_and_disconnect() {
    let addr = start();
    let mut c = Client::connect(&addr);
    assert_eq!(c.request(MessageType::Begin, Payload::new()).0, MessageType::Ok);
    c.rows("CREATE (:T {v: 1})");
    assert_eq!(c.request(MessageType::Rollback, Payload::new()).0, MessageType::Ok);
    assert_eq!(c.rows("MATCH (n:T) RETURN count(n)"), vec![Value::List(vec![Value::Integer(0)])]);

    // Cypher-text BEGIN/COMMIT behave like the messages.
    c.rows("BEGIN");
    c.rows("CREATE (:T {v: 2})");
    c.rows("COMMIT");
    assert_eq!(c.request(MessageType::Commit, Payload::new()).0, MessageType::Error);

    // A transaction left open by a closed connection is rolled back.
    let mut d = Client::connect(&addr);
    d.request(MessageType::Begin, Payload::new());
    d.rows("CREATE (:T {v: 3})");
    drop(d);
    assert_eq!(c.rows("MATCH (n:T) RETURN count(n)"), vec![Value::List(vec![Value::Integer(1)])]);
    assert_eq!(c.request(MessageType::Goodbye, Payload::new()).0, MessageType::Ok);
}

#[test]
fn use_database_is_per_connection() {
    let addr = start();
    let mut a = Client::connect(&addr);
    let mut b = Client::connect(&addr);
    a.rows("CREATE DATABASE other");
    let (_, p) = a.query("USE DATABASE other", PropertyMap::new());
    assert_eq!(p.get("database"), Some(&Value::from("other")));
    let (_, p) = b.query("RETURN 1", PropertyMap::new());
    assert_ne!(p.get("database"), Some(&Value::from("other")));
}

#[test]
fn handshake_is_required() {
    let addr = start();
    let mut s = TcpStream::connect(&addr).unwrap();
    write_frame(&mut s, MessageType::Ping as u8, 1, &Payload::new().encode().unwrap()).unwrap();
    let f = read_frame(&mut s).unwrap().unwrap();
    assert_eq!(f.message_type, MessageType::Error as u8);
    assert!(read_frame(&mut s).unwrap().is_none());
}
