//! MyGraph TCP server speaking protocol v1 (`docs/PROTOCOL.md`).
//!
//! One OS thread per connection, all sharing one [`Database`]. Each
//! connection has its own [`mygraph::Session`] (current database and schema)
//! and at most one open transaction. In v0.1 statements are serialized, and an
//! open transaction blocks every other connection until it commits or rolls
//! back, exactly as in embedded mode.

mod connection;

use std::io;
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

use mygraph::Database;

pub use connection::handle_connection;

/// Accepts connections forever, one thread each.
pub fn serve(listener: TcpListener, db: Arc<Database>) -> io::Result<()> {
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("accept failed: {e}");
                continue;
            }
        };
        let db = Arc::clone(&db);
        thread::Builder::new().name("mygraph-conn".into()).spawn(move || {
            let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_else(|_| "?".into());
            if let Err(e) = handle_connection(stream, &db) {
                eprintln!("connection {peer}: {e}");
            }
        })?;
    }
    Ok(())
}
