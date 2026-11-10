//! `mygraph-server start [--host HOST] [--port PORT] [--data DIR]`
//!
//! Without `--data` the instance is in-memory.

use std::net::TcpListener;
use std::process::ExitCode;
use std::sync::Arc;

use mygraph::Database;

const USAGE: &str = "usage:
  mygraph-server start [--host HOST] [--port PORT] [--data DIR]

defaults: --host 127.0.0.1 --port 7687, in-memory without --data";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("start") {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let (mut host, mut port, mut data) = ("127.0.0.1".to_string(), 7687u16, None::<String>);
    while let Some(a) = args.next() {
        let value = match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--host" | "--port" | "--data" => match args.next() {
                Some(v) => v,
                None => {
                    eprintln!("{a} needs a value\n{USAGE}");
                    return ExitCode::from(2);
                }
            },
            other => {
                eprintln!("unknown argument `{other}`\n{USAGE}");
                return ExitCode::from(2);
            }
        };
        match a.as_str() {
            "--host" => host = value,
            "--port" => match value.parse() {
                Ok(p) => port = p,
                Err(_) => {
                    eprintln!("invalid port `{value}`");
                    return ExitCode::from(2);
                }
            },
            _ => data = Some(value),
        }
    }

    let db = match &data {
        Some(dir) => Database::open(dir.as_str()),
        None => Database::in_memory(),
    };
    let db = match db {
        Ok(db) => Arc::new(db),
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let listener = match TcpListener::bind((host.as_str(), port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: cannot listen on {host}:{port}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let addr = listener.local_addr().map(|a| a.to_string()).unwrap_or_default();
    // The Python test suite waits for this exact line to learn the port.
    println!(
        "MyGraph server v{} listening on {addr} ({})",
        env!("CARGO_PKG_VERSION"),
        data.as_deref().unwrap_or("in-memory")
    );
    match mygraph_server::serve(listener, db) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
