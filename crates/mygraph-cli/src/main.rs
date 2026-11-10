//! `mygraph` command line.
//!
//! ```text
//! mygraph shell [--data DIR | --host HOST --port PORT]                 interactive shell
//! mygraph query [--data DIR | --host HOST --port PORT] [--json] QUERY  run one script and print the result
//! ```
//!
//! Without `--data` the embedded instance is in-memory. With `--host` and/or
//! `--port` the CLI connects to a running `mygraph-server` instead
//! (defaults: localhost, 6660).

mod remote;

use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::time::Instant;

use mygraph::{params, Database, QueryResult, Transaction, Value};

const USAGE: &str = "usage:
  mygraph shell [--data DIR | --host HOST --port PORT]
  mygraph query [--data DIR | --host HOST --port PORT] [--json] \"<cypher>\"

  --host/--port connect to a running mygraph-server (defaults: localhost, 6660)";

const DEFAULT_HOST: &str = "localhost";
const DEFAULT_PORT: u16 = 6660;

struct Args {
    command: String,
    data: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    json: bool,
    rest: Vec<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let command = it.next().ok_or(USAGE)?;
    let mut args = Args { command, data: None, host: None, port: None, json: false, rest: Vec::new() };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--data" => args.data = Some(it.next().ok_or("--data needs a directory")?),
            "--host" => args.host = Some(it.next().ok_or("--host needs a host name")?),
            "--port" => {
                let p = it.next().ok_or("--port needs a number")?;
                args.port = Some(p.parse().map_err(|_| format!("invalid port `{p}`"))?);
            }
            "--json" => args.json = true,
            "-h" | "--help" => return Err(USAGE.into()),
            _ => args.rest.push(a),
        }
    }
    if args.data.is_some() && (args.host.is_some() || args.port.is_some()) {
        return Err("--data cannot be combined with --host/--port".into());
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    if args.host.is_some() || args.port.is_some() {
        let host = args.host.clone().unwrap_or_else(|| DEFAULT_HOST.into());
        return run_remote(&args, &host, args.port.unwrap_or(DEFAULT_PORT));
    }
    let db = match &args.data {
        Some(dir) => Database::open(dir.as_str()),
        None => Database::in_memory(),
    };
    let db = match db {
        Ok(db) => db,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    match args.command.as_str() {
        "shell" => {
            shell(&db, args.data.as_deref());
            ExitCode::SUCCESS
        }
        "query" => {
            let q = args.rest.join(" ");
            match db.execute(&q, params! {}) {
                Ok(r) => {
                    if args.json {
                        println!("{}", to_json(&r));
                    } else {
                        print!("{}", render_table(&r));
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        other => {
            eprintln!("unknown command `{other}`\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run_remote(args: &Args, host: &str, port: u16) -> ExitCode {
    if args.command != "shell" && args.command != "query" {
        eprintln!("unknown command `{}`\n{USAGE}", args.command);
        return ExitCode::from(2);
    }
    let mut client = match remote::Client::connect(host, port) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let code = if args.command == "shell" {
        remote_shell(&mut client, host, port);
        ExitCode::SUCCESS
    } else {
        match client.query(&args.rest.join(" ")) {
            Ok(reply) => {
                if args.json {
                    println!("{}", to_json(&reply.result));
                } else {
                    print!("{}", render_table(&reply.result));
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        }
    };
    client.close();
    code
}

fn remote_shell(client: &mut remote::Client, host: &str, port: u16) {
    println!("MyGraph v{} (connected to {host}:{port})", env!("CARGO_PKG_VERSION"));
    println!("End statements with `;`. Type EXIT; to quit.");
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut location = String::from("?");
    let mut in_tx = false;
    if let Ok(r) = client.query("RETURN 1") {
        location = format!("{}/{}", r.database, r.schema);
    }
    let mut buffer = String::new();
    loop {
        let prompt = match (in_tx, buffer.is_empty()) {
            (_, false) => "      ...> ".to_string(),
            (true, true) => format!("mygraph[{location}]*> "),
            (false, true) => format!("mygraph[{location}]> "),
        };
        print!("{prompt}");
        let _ = io::stdout().flush();
        let Some(Ok(line)) = lines.next() else { break };
        buffer.push_str(&line);
        buffer.push('\n');
        if !line.trim_end().ends_with(';') {
            continue;
        }
        let stmt = std::mem::take(&mut buffer);
        let cmd = stmt.trim().trim_end_matches(';').trim().to_ascii_uppercase();
        if cmd == "EXIT" || cmd == "QUIT" {
            break;
        }
        let started = Instant::now();
        match client.query(&stmt) {
            Ok(reply) => {
                let r = &reply.result;
                if r.columns.is_empty() && !r.stats.contains_updates() {
                    println!("OK");
                } else {
                    print!("{}", render_table(r));
                    println!("({:.1} ms)", started.elapsed().as_secs_f64() * 1000.0);
                }
                location = format!("{}/{}", reply.database, reply.schema);
                in_tx = reply.in_transaction;
            }
            Err(e) => println!("error: {e}"),
        }
    }
    if in_tx {
        println!("open transaction rolled back by the server");
    }
}

fn shell(db: &Database, data: Option<&str>) {
    println!("MyGraph v{} (embedded, {})", env!("CARGO_PKG_VERSION"), data.unwrap_or("in-memory"));
    println!("End statements with `;`. Type EXIT; to quit.");
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut tx: Option<Transaction<'_>> = None;
    let mut location = format!("{}/{}", db.current_database(), db.current_schema());
    let mut buffer = String::new();
    loop {
        let prompt = match (&tx, buffer.is_empty()) {
            (_, false) => "      ...> ".to_string(),
            (Some(_), true) => format!("mygraph[{location}]*> "),
            (None, true) => format!("mygraph[{location}]> "),
        };
        print!("{prompt}");
        let _ = io::stdout().flush();
        let Some(Ok(line)) = lines.next() else { break };
        buffer.push_str(&line);
        buffer.push('\n');
        if !line.trim_end().ends_with(';') {
            continue;
        }
        let stmt = std::mem::take(&mut buffer);
        let cmd = stmt.trim().trim_end_matches(';').trim().to_ascii_uppercase();
        let started = Instant::now();
        let outcome = match cmd.as_str() {
            "EXIT" | "QUIT" => break,
            "BEGIN" if tx.is_some() => Err("a transaction is already open".to_string()),
            "BEGIN" => db
                .begin_transaction()
                .map(|t| {
                    tx = Some(t);
                    None
                })
                .map_err(|e| e.to_string()),
            "COMMIT" | "ROLLBACK" => match tx.take() {
                None => Err("no open transaction".to_string()),
                Some(t) => {
                    let r = if cmd == "COMMIT" { t.commit() } else { t.rollback() };
                    r.map(|_| None).map_err(|e| e.to_string())
                }
            },
            _ => match tx.as_mut() {
                Some(t) => t.execute(&stmt, params! {}),
                None => db.execute(&stmt, params! {}),
            }
            .map(Some)
            .map_err(|e| e.to_string()),
        };
        match outcome {
            Ok(Some(r)) if r.columns.is_empty() && !r.stats.contains_updates() => println!("OK"),
            Ok(Some(r)) => {
                print!("{}", render_table(&r));
                println!("({:.1} ms)", started.elapsed().as_secs_f64() * 1000.0);
            }
            Ok(None) => println!("OK"),
            Err(e) => println!("error: {e}"),
        }
        if tx.is_none() {
            location = format!("{}/{}", db.current_database(), db.current_schema());
        }
    }
    if let Some(t) = tx {
        let _ = t.rollback();
        println!("open transaction rolled back");
    }
}

fn cell(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn render_table(r: &QueryResult) -> String {
    let mut out = String::new();
    if !r.columns.is_empty() {
        let rows: Vec<Vec<String>> = r.rows.iter().map(|row| row.iter().map(cell).collect()).collect();
        let widths: Vec<usize> = r
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| rows.iter().map(|row| row[i].chars().count()).chain([c.chars().count()]).max().unwrap())
            .collect();
        let sep: String = widths.iter().map(|w| format!("+{}", "-".repeat(w + 2))).collect::<String>() + "+\n";
        let line = |cells: &[String]| {
            cells
                .iter()
                .zip(&widths)
                .map(|(c, w)| format!("| {c}{} ", " ".repeat(w - c.chars().count())))
                .collect::<String>()
                + "|\n"
        };
        out.push_str(&sep);
        out.push_str(&line(&r.columns));
        out.push_str(&sep);
        for row in &rows {
            out.push_str(&line(row));
        }
        out.push_str(&sep);
        out.push_str(&format!("{} row{}\n", r.len(), if r.len() == 1 { "" } else { "s" }));
    }
    let s = &r.stats;
    if s.contains_updates() {
        let parts: Vec<String> = [
            ("nodes created", s.nodes_created),
            ("nodes deleted", s.nodes_deleted),
            ("relationships created", s.relationships_created),
            ("relationships deleted", s.relationships_deleted),
            ("properties set", s.properties_set),
            ("labels added", s.labels_added),
            ("labels removed", s.labels_removed),
        ]
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(k, n)| format!("{k}: {n}"))
        .collect();
        out.push_str(&parts.join(", "));
        out.push('\n');
    }
    out
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_map<'a>(entries: impl Iterator<Item = (&'a String, &'a Value)>) -> String {
    let body: Vec<String> = entries.map(|(k, v)| format!("{}:{}", json_string(k), json_value(v))).collect();
    format!("{{{}}}", body.join(","))
}

fn json_value(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) if f.is_finite() => format!("{f}"),
        Value::Float(_) => "null".into(),
        Value::String(s) => json_string(s),
        Value::Bytes(b) => format!("[{}]", b.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")),
        Value::List(items) => format!("[{}]", items.iter().map(json_value).collect::<Vec<_>>().join(",")),
        Value::Map(m) => json_map(m.iter()),
        Value::Node(n) => format!(
            "{{\"id\":{},\"labels\":[{}],\"properties\":{}}}",
            n.id,
            n.labels.iter().map(|l| json_string(l)).collect::<Vec<_>>().join(","),
            json_map(n.properties.iter())
        ),
        Value::Relationship(r) => format!(
            "{{\"id\":{},\"type\":{},\"start\":{},\"end\":{},\"properties\":{}}}",
            r.id,
            json_string(&r.rel_type),
            r.start_node,
            r.end_node,
            json_map(r.properties.iter())
        ),
        Value::Path(p) => {
            let nodes: Vec<String> = p.nodes.iter().map(|n| json_value(&Value::Node(Box::new(n.clone())))).collect();
            let rels: Vec<String> =
                p.relationships.iter().map(|r| json_value(&Value::Relationship(Box::new(r.clone())))).collect();
            format!("{{\"nodes\":[{}],\"relationships\":[{}]}}", nodes.join(","), rels.join(","))
        }
    }
}

fn to_json(r: &QueryResult) -> String {
    let rows: Vec<String> = r.rows.iter().map(|row| json_map(r.columns.iter().zip(row.iter()))).collect();
    format!("[{}]", rows.join(","))
}
