//! Minimal client for `mygraph-server` (wire protocol v1).

use std::io::{BufReader, BufWriter};
use std::net::TcpStream;

use mygraph::{PropertyMap, QueryResult, QueryStats, Value};
use mygraph_protocol::{read_frame, write_frame, MessageType, Payload};

pub struct Client {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
    next_id: u32,
}

/// A query result plus the session state the server reports with it.
pub struct Reply {
    pub result: QueryResult,
    pub database: String,
    pub schema: String,
    pub in_transaction: bool,
}

impl Client {
    pub fn connect(host: &str, port: u16) -> Result<Self, String> {
        let stream = TcpStream::connect((host, port)).map_err(|e| format!("cannot connect to {host}:{port}: {e}"))?;
        let _ = stream.set_nodelay(true);
        let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut c = Client { reader, writer: BufWriter::new(stream), next_id: 0 };
        c.request(MessageType::Hello, Payload::new().with("client", "mygraph-cli"), MessageType::Hello)?;
        Ok(c)
    }

    pub fn query(&mut self, query: &str) -> Result<Reply, String> {
        let p = Payload::new().with("query", query).with("params", Value::Map(PropertyMap::new()));
        let r = self.request(MessageType::Query, p, MessageType::QueryResult)?;
        let text = |k: &str| match r.get(k) {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        Ok(Reply {
            database: text("database"),
            schema: text("schema"),
            in_transaction: matches!(r.get("in_transaction"), Some(Value::Boolean(true))),
            result: to_result(&r)?,
        })
    }

    pub fn close(mut self) {
        let _ = self.request(MessageType::Goodbye, Payload::new(), MessageType::Ok);
    }

    fn request(&mut self, kind: MessageType, payload: Payload, expect: MessageType) -> Result<Payload, String> {
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        let bytes = payload.encode().map_err(|e| e.to_string())?;
        write_frame(&mut self.writer, kind as u8, id, &bytes).map_err(|e| format!("send failed: {e}"))?;
        let frame = read_frame(&mut self.reader)
            .map_err(|e| format!("receive failed: {e}"))?
            .ok_or("server closed the connection")?;
        let reply = Payload::decode(&frame.payload).map_err(|e| e.to_string())?;
        match MessageType::from_u8(frame.message_type) {
            Some(t) if t == expect => Ok(reply),
            Some(MessageType::Error) => {
                let msg = match reply.get("message") {
                    Some(Value::String(s)) => s.clone(),
                    _ => "unknown error".into(),
                };
                Err(msg)
            }
            _ => Err(format!("unexpected message type 0x{:02x}", frame.message_type)),
        }
    }
}

fn to_result(p: &Payload) -> Result<QueryResult, String> {
    let columns = match p.get("columns") {
        Some(Value::List(cs)) => cs
            .iter()
            .map(|c| match c {
                Value::String(s) => Ok(s.clone()),
                _ => Err("column names must be strings".to_string()),
            })
            .collect::<Result<_, _>>()?,
        _ => Vec::new(),
    };
    let rows = match p.get("rows") {
        Some(Value::List(rs)) => rs
            .iter()
            .map(|r| match r {
                Value::List(cells) => Ok(cells.clone()),
                _ => Err("rows must be lists".to_string()),
            })
            .collect::<Result<_, _>>()?,
        _ => Vec::new(),
    };
    let mut stats = QueryStats::default();
    if let Some(Value::Map(m)) = p.get("stats") {
        let n = |k: &str| match m.get(k) {
            Some(Value::Integer(i)) => *i as u64,
            _ => 0,
        };
        stats.nodes_created = n("nodes_created");
        stats.nodes_deleted = n("nodes_deleted");
        stats.relationships_created = n("relationships_created");
        stats.relationships_deleted = n("relationships_deleted");
        stats.properties_set = n("properties_set");
        stats.labels_added = n("labels_added");
        stats.labels_removed = n("labels_removed");
    }
    Ok(QueryResult { columns, rows, stats })
}
