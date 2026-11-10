//! One client connection: handshake, then a request/response loop.

use std::io::{self, BufReader, BufWriter};
use std::net::TcpStream;

use mygraph::{Database, MyGraphError, Params, QueryResult, QueryStats, Transaction, Value};
use mygraph_protocol::{error_payload, read_frame, write_frame, MessageType, Payload, PROTOCOL_VERSION};

struct Conn {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

impl Conn {
    fn send(&mut self, t: MessageType, request_id: u32, payload: &Payload) -> io::Result<()> {
        let bytes = payload.encode().map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        write_frame(&mut self.writer, t as u8, request_id, &bytes)
    }

    fn send_error(&mut self, request_id: u32, code: &str, message: impl Into<String>) -> io::Result<()> {
        let p = Payload::new().with("code", code).with("message", message.into());
        self.send(MessageType::Error, request_id, &p)
    }

    fn ok(&mut self, request_id: u32) -> io::Result<()> {
        self.send(MessageType::Ok, request_id, &Payload::new())
    }
}

/// Serves one connection until the client says GOODBYE or disconnects. An
/// open transaction is rolled back when the connection ends.
pub fn handle_connection(stream: TcpStream, db: &Database) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let mut conn = Conn { reader: BufReader::new(stream.try_clone()?), writer: BufWriter::new(stream) };
    let mut session = db.session();
    let mut tx: Option<Transaction<'_>> = None;
    let mut greeted = false;

    while let Some(frame) = read_frame(&mut conn.reader)? {
        let id = frame.request_id;
        if frame.version != PROTOCOL_VERSION {
            let msg = format!("unsupported protocol version {} (server speaks {PROTOCOL_VERSION})", frame.version);
            conn.send_error(id, "Protocol", msg)?;
            return Ok(());
        }
        let Some(kind) = MessageType::from_u8(frame.message_type) else {
            conn.send_error(id, "Protocol", format!("unknown message type 0x{:02x}", frame.message_type))?;
            continue;
        };
        let payload = match Payload::decode(&frame.payload) {
            Ok(p) => p,
            Err(e) => {
                conn.send_error(id, "Protocol", e.to_string())?;
                continue;
            }
        };
        if !greeted && kind != MessageType::Hello {
            conn.send_error(id, "Protocol", "the first message must be HELLO")?;
            return Ok(());
        }

        match kind {
            MessageType::Hello => {
                greeted = true;
                let p = Payload::new()
                    .with("server", "mygraph-server")
                    .with("version", env!("CARGO_PKG_VERSION"))
                    .with("protocol", PROTOCOL_VERSION as i64);
                conn.send(MessageType::Hello, id, &p)?;
            }
            // Authentication is not implemented yet (spec §11): any credentials are accepted.
            MessageType::Auth => conn.ok(id)?,
            MessageType::Ping => conn.send(MessageType::Pong, id, &Payload::new())?,
            MessageType::Goodbye => {
                conn.ok(id)?;
                return Ok(());
            }
            MessageType::Begin => match begin(&session, &mut tx) {
                Ok(()) => conn.ok(id)?,
                Err(e) => conn.send(MessageType::Error, id, &error_payload(&e))?,
            },
            MessageType::Commit | MessageType::Rollback => match finish(&mut tx, kind == MessageType::Commit) {
                Ok(()) => conn.ok(id)?,
                Err(e) => conn.send(MessageType::Error, id, &error_payload(&e))?,
            },
            MessageType::Query => {
                let (query, params) = match query_args(&payload) {
                    Ok(a) => a,
                    Err(msg) => {
                        conn.send_error(id, "Protocol", msg)?;
                        continue;
                    }
                };
                // Transaction control sent as Cypher text is handled like the
                // dedicated messages, so `run("BEGIN")` works from any client.
                let control = query.trim().trim_end_matches(';').trim().to_ascii_uppercase();
                let outcome = match control.as_str() {
                    "BEGIN" => begin(&session, &mut tx).map(|_| QueryResult::default()),
                    "COMMIT" | "ROLLBACK" => finish(&mut tx, control == "COMMIT").map(|_| QueryResult::default()),
                    _ => match tx.as_mut() {
                        Some(t) => t.execute(&query, params),
                        None => session.execute(&query, params),
                    },
                };
                match outcome {
                    Ok(r) => {
                        let p = result_payload(r, session.current_database(), session.current_schema(), tx.is_some());
                        conn.send(MessageType::QueryResult, id, &p)?
                    }
                    Err(e) => conn.send(MessageType::Error, id, &error_payload(&e))?,
                }
            }
            MessageType::QueryResult | MessageType::Ok | MessageType::Pong | MessageType::Error => {
                conn.send_error(id, "Protocol", format!("{kind:?} is a server-to-client message"))?
            }
        }
    }
    Ok(())
}

fn begin<'db>(session: &mygraph::Session<'db>, tx: &mut Option<Transaction<'db>>) -> mygraph::Result<()> {
    if tx.is_some() {
        return Err(MyGraphError::Transaction("a transaction is already open".into()));
    }
    *tx = Some(session.begin_transaction()?);
    Ok(())
}

fn finish(tx: &mut Option<Transaction<'_>>, commit: bool) -> mygraph::Result<()> {
    let t = tx.take().ok_or_else(|| MyGraphError::Transaction("no open transaction".into()))?;
    if commit {
        t.commit()
    } else {
        t.rollback()
    }
}

fn query_args(p: &Payload) -> Result<(String, Params), String> {
    let query = match p.get("query") {
        Some(Value::String(s)) => s.clone(),
        _ => return Err("QUERY needs a string `query`".into()),
    };
    let params = match p.get("params") {
        None | Some(Value::Null) => Params::new(),
        Some(Value::Map(m)) => m.clone(),
        Some(_) => return Err("QUERY `params` must be a map".into()),
    };
    Ok((query, params))
}

fn stats_map(s: &QueryStats) -> Value {
    let fields = [
        ("nodes_created", s.nodes_created),
        ("nodes_deleted", s.nodes_deleted),
        ("relationships_created", s.relationships_created),
        ("relationships_deleted", s.relationships_deleted),
        ("properties_set", s.properties_set),
        ("labels_added", s.labels_added),
        ("labels_removed", s.labels_removed),
    ];
    Value::Map(fields.into_iter().map(|(k, v)| (k.to_string(), Value::Integer(v as i64))).collect())
}

fn result_payload(r: QueryResult, database: &str, schema: &str, in_transaction: bool) -> Payload {
    let stats = stats_map(&r.stats);
    Payload::new()
        .with("columns", Value::List(r.columns.into_iter().map(Value::String).collect()))
        .with("rows", Value::List(r.rows.into_iter().map(Value::List).collect()))
        .with("stats", stats)
        .with("database", database)
        .with("schema", schema)
        .with("in_transaction", in_transaction)
}
