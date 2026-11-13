//! Rust client for the [MyGraph](https://github.com/julioacontreras/my-graph)
//! graph database server, speaking wire protocol v1 over TCP.
//!
//! ```no_run
//! use mygraph_client::{params, Client};
//!
//! # fn main() -> mygraph_client::Result<()> {
//! let mut db = Client::connect("localhost:6660")?;
//!
//! db.run_with("CREATE (:User {name: $name, age: $age})", params! { "name" => "Alice", "age" => 30 })?;
//!
//! let result = db.run("MATCH (u:User) RETURN u.name AS name, u.age AS age")?;
//! for record in result.records() {
//!     println!("{record:?}");
//! }
//!
//! let mut tx = db.begin()?;
//! tx.run("CREATE (:User {name: 'Bob'})")?;
//! tx.commit()?; // dropping `tx` without committing rolls it back
//!
//! db.close();
//! # Ok(())
//! # }
//! ```
//!
//! Requests on one [`Client`] are strictly sequential (protocol v1 has no
//! pipelining). Use one client per thread for parallel work.

mod error;
mod result;

use std::io::{self, BufReader, BufWriter};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use mygraph_protocol::{read_frame, write_frame, MessageType, Payload};

pub use error::{Error, QueryError, Result};
pub use mygraph_core::{NodeValue, PathValue, PropertyMap, RelationshipValue, Value};
pub use result::{QueryResult, Stats};

/// Default server port.
pub const DEFAULT_PORT: u16 = 6660;

/// Builds a [`PropertyMap`] of query parameters.
///
/// ```
/// let p = mygraph_client::params! { "name" => "Alice", "age" => 30 };
/// assert_eq!(p.len(), 2);
/// ```
#[macro_export]
macro_rules! params {
    () => { $crate::PropertyMap::new() };
    ($($key:expr => $value:expr),+ $(,)?) => {{
        let mut map = $crate::PropertyMap::new();
        $( map.insert(::std::string::String::from($key), $crate::Value::from($value)); )+
        map
    }};
}

/// Options for [`Client::connect_with`].
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// Timeout for connecting and for each read and write. `None` waits forever.
    pub timeout: Option<Duration>,
    /// Sent with `AUTH` after the handshake when set.
    pub user: Option<String>,
    pub password: Option<String>,
    /// Name the client reports in `HELLO`.
    pub client_name: String,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        ConnectOptions {
            timeout: Some(Duration::from_secs(30)),
            user: None,
            password: None,
            client_name: concat!("mygraph-rust/", env!("CARGO_PKG_VERSION")).to_string(),
        }
    }
}

impl ConnectOptions {
    pub fn timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn auth(mut self, user: impl Into<String>, password: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self.password = Some(password.into());
        self
    }

    pub fn client_name(mut self, name: impl Into<String>) -> Self {
        self.client_name = name.into();
        self
    }
}

/// What the server reported in its `HELLO`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub server: String,
    pub version: String,
    pub protocol: i64,
}

struct Conn {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

/// One TCP connection to a MyGraph server, with its own session (current
/// database and schema, at most one open transaction).
pub struct Client {
    conn: Option<Conn>,
    next_id: u32,
    server: ServerInfo,
}

impl Client {
    /// Connects with default options, for example `Client::connect("localhost:6660")`.
    pub fn connect(addr: impl ToSocketAddrs) -> Result<Client> {
        Client::connect_with(addr, ConnectOptions::default())
    }

    pub fn connect_with(addr: impl ToSocketAddrs, options: ConnectOptions) -> Result<Client> {
        let stream = open_stream(addr, options.timeout)?;
        let _ = stream.set_nodelay(true);
        stream.set_read_timeout(options.timeout)?;
        stream.set_write_timeout(options.timeout)?;
        let reader = BufReader::new(stream.try_clone()?);
        let mut client = Client {
            conn: Some(Conn { reader, writer: BufWriter::new(stream) }),
            next_id: 0,
            server: ServerInfo { server: String::new(), version: String::new(), protocol: 0 },
        };
        let hello = client.request(
            MessageType::Hello,
            Payload::new().with("client", options.client_name.as_str()),
            MessageType::Hello,
        )?;
        client.server = ServerInfo {
            server: text(&hello, "server"),
            version: text(&hello, "version"),
            protocol: match hello.get("protocol") {
                Some(Value::Integer(p)) => *p,
                _ => 0,
            },
        };
        if let Some(user) = &options.user {
            let p =
                Payload::new().with("user", user.as_str()).with("password", options.password.as_deref().unwrap_or(""));
            client.request(MessageType::Auth, p, MessageType::Ok)?;
        }
        Ok(client)
    }

    pub fn server_info(&self) -> &ServerInfo {
        &self.server
    }

    pub fn is_closed(&self) -> bool {
        self.conn.is_none()
    }

    /// Runs a Cypher script (one or more `;`-separated statements) and
    /// returns the last statement's rows.
    pub fn run(&mut self, query: &str) -> Result<QueryResult> {
        self.run_with(query, PropertyMap::new())
    }

    /// Like [`run`](Self::run), with parameters referenced as `$name`.
    /// See the [`params!`] macro.
    pub fn run_with(&mut self, query: &str, params: PropertyMap) -> Result<QueryResult> {
        let p = Payload::new().with("query", query).with("params", Value::Map(params));
        let r = self.request(MessageType::Query, p, MessageType::QueryResult)?;
        to_result(&r)
    }

    /// Begins an explicit transaction. Every statement on this client runs in
    /// it until [`Transaction::commit`] or [`Transaction::rollback`]; dropping
    /// it unfinished rolls it back. In MyGraph v0.x an open transaction blocks
    /// other connections, so keep it short.
    pub fn begin(&mut self) -> Result<Transaction<'_>> {
        self.request(MessageType::Begin, Payload::new(), MessageType::Ok)?;
        Ok(Transaction { client: self, finished: false })
    }

    /// Runs `f` in a transaction: commits if it returns `Ok`, rolls back if it
    /// returns `Err`.
    pub fn transaction<T, E>(
        &mut self,
        f: impl FnOnce(&mut Transaction<'_>) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<Error>,
    {
        let mut tx = self.begin()?;
        match f(&mut tx) {
            Ok(v) => {
                tx.commit()?;
                Ok(v)
            }
            Err(e) => {
                let _ = tx.rollback();
                Err(e)
            }
        }
    }

    pub fn ping(&mut self) -> Result<()> {
        self.request(MessageType::Ping, Payload::new(), MessageType::Pong).map(|_| ())
    }

    pub fn use_database(&mut self, name: &str) -> Result<()> {
        self.run(&format!("USE DATABASE {}", quote(name))).map(|_| ())
    }

    pub fn use_schema(&mut self, name: &str) -> Result<()> {
        self.run(&format!("USE SCHEMA {}", quote(name))).map(|_| ())
    }

    pub fn create_database(&mut self, name: &str) -> Result<()> {
        self.run(&format!("CREATE DATABASE IF NOT EXISTS {}", quote(name))).map(|_| ())
    }

    pub fn drop_database(&mut self, name: &str) -> Result<()> {
        self.run(&format!("DROP DATABASE IF EXISTS {}", quote(name))).map(|_| ())
    }

    pub fn create_schema(&mut self, name: &str) -> Result<()> {
        self.run(&format!("CREATE SCHEMA IF NOT EXISTS {}", quote(name))).map(|_| ())
    }

    pub fn drop_schema(&mut self, name: &str) -> Result<()> {
        self.run(&format!("DROP SCHEMA IF EXISTS {}", quote(name))).map(|_| ())
    }

    /// Says `GOODBYE` and closes the connection. An open transaction is rolled
    /// back by the server. Dropping the client does the same.
    pub fn close(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if self.conn.is_some() {
            let _ = self.request(MessageType::Goodbye, Payload::new(), MessageType::Ok);
            self.conn = None;
        }
    }

    fn request(&mut self, kind: MessageType, payload: Payload, expect: MessageType) -> Result<Payload> {
        let conn = self
            .conn
            .as_mut()
            .ok_or_else(|| Error::Connection(io::Error::new(io::ErrorKind::NotConnected, "the client is closed")))?;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = self.next_id;
        let outcome = (|| {
            let bytes = payload.encode().map_err(|e| Error::Protocol(e.to_string()))?;
            write_frame(&mut conn.writer, kind as u8, id, &bytes)?;
            let frame = read_frame(&mut conn.reader)?
                .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "server closed the connection"))?;
            if frame.request_id != id {
                return Err(Error::Protocol(format!(
                    "response id {} does not match request id {id}",
                    frame.request_id
                )));
            }
            let reply = Payload::decode(&frame.payload).map_err(|e| Error::Protocol(e.to_string()))?;
            Ok((frame.message_type, reply))
        })();
        let (message_type, reply) = match outcome {
            Ok(r) => r,
            Err(e) => {
                // The stream position is unknown now; the connection cannot be reused.
                self.conn = None;
                return Err(e);
            }
        };
        match MessageType::from_u8(message_type) {
            Some(t) if t == expect => Ok(reply),
            Some(MessageType::Error) => {
                let code = text(&reply, "code");
                let message = match reply.get("message") {
                    Some(Value::String(s)) => s.clone(),
                    _ => "unknown error".into(),
                };
                if code == "Protocol" {
                    self.conn = None;
                    return Err(Error::Protocol(message));
                }
                let pos = |k: &str| match reply.get(k) {
                    Some(Value::Integer(i)) => u32::try_from(*i).ok(),
                    _ => None,
                };
                Err(Error::Query(QueryError { code, message, line: pos("line"), column: pos("column") }))
            }
            _ => {
                self.conn = None;
                Err(Error::Protocol(format!(
                    "unexpected message type 0x{message_type:02x} (expected 0x{:02x})",
                    expect as u8
                )))
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("server", &self.server).field("closed", &self.is_closed()).finish()
    }
}

/// An explicit transaction, from [`Client::begin`]. Dropping it without
/// calling [`commit`](Self::commit) rolls it back.
pub struct Transaction<'a> {
    client: &'a mut Client,
    finished: bool,
}

impl Transaction<'_> {
    pub fn run(&mut self, query: &str) -> Result<QueryResult> {
        self.client.run(query)
    }

    pub fn run_with(&mut self, query: &str, params: PropertyMap) -> Result<QueryResult> {
        self.client.run_with(query, params)
    }

    pub fn commit(mut self) -> Result<()> {
        self.finish(MessageType::Commit)
    }

    pub fn rollback(mut self) -> Result<()> {
        self.finish(MessageType::Rollback)
    }

    fn finish(&mut self, kind: MessageType) -> Result<()> {
        self.finished = true;
        self.client.request(kind, Payload::new(), MessageType::Ok).map(|_| ())
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.finish(MessageType::Rollback);
        }
    }
}

/// Backtick-quotes an identifier for use in a Cypher statement.
pub fn quote(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

fn open_stream(addr: impl ToSocketAddrs, timeout: Option<Duration>) -> Result<TcpStream> {
    let Some(timeout) = timeout else {
        return Ok(TcpStream::connect(addr)?);
    };
    let mut last = None;
    for a in addr.to_socket_addrs()? {
        match TcpStream::connect_timeout(&a, timeout) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(Error::Connection(
        last.unwrap_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "address resolved to nothing")),
    ))
}

fn text(p: &Payload, key: &str) -> String {
    match p.get(key) {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn to_result(p: &Payload) -> Result<QueryResult> {
    let columns = match p.get("columns") {
        Some(Value::List(cs)) => cs
            .iter()
            .map(|c| match c {
                Value::String(s) => Ok(s.clone()),
                _ => Err(Error::Protocol("column names must be strings".into())),
            })
            .collect::<Result<_>>()?,
        _ => Vec::new(),
    };
    let rows = match p.get("rows") {
        Some(Value::List(rs)) => rs
            .iter()
            .map(|r| match r {
                Value::List(cells) => Ok(cells.clone()),
                _ => Err(Error::Protocol("rows must be lists".into())),
            })
            .collect::<Result<_>>()?,
        _ => Vec::new(),
    };
    let stats = match p.get("stats") {
        Some(Value::Map(m)) => Stats::from_map(m),
        _ => Stats::default(),
    };
    Ok(QueryResult {
        columns,
        rows,
        stats,
        database: text(p, "database"),
        schema: text(p, "schema"),
        in_transaction: matches!(p.get("in_transaction"), Some(Value::Boolean(true))),
    })
}
