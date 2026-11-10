//! MyGraph embedded API.
//!
//! ```
//! use mygraph::{params, Database};
//!
//! let db = Database::in_memory()?;
//! db.execute("CREATE (:User {name: $name, age: $age})", params! { "name" => "Alice", "age" => 25 })?;
//! let result = db.execute("MATCH (u:User) RETURN u.name AS name", params! {})?;
//! assert_eq!(result.value(0, "name").and_then(|v| v.as_str()), Some("Alice"));
//! # Ok::<(), mygraph::MyGraphError>(())
//! ```
//!
//! Concurrency in v0.1: a [`Database`] is `Send + Sync`, but statements are
//! serialized, and an explicit [`Transaction`] holds exclusive access until it
//! is committed or rolled back. MVCC is planned for a later phase.

mod config;
mod result;

use std::sync::{Mutex, MutexGuard};

use mygraph_core::{Catalog, Journal, Op, WriteContext, DEFAULT_DATABASE, DEFAULT_SCHEMA};
use mygraph_cypher::{AdminCommand, ExecResult, Query, Statement};
use mygraph_storage::{RecoveryInfo, Store, StoreConfig};

pub use config::Config;
pub use mygraph_core::{MyGraphError, NodeValue, PropertyMap, RelationshipValue, Result, Value};
pub use mygraph_cypher::QueryStats;
pub use result::QueryResult;

/// Query parameters, referenced in Cypher as `$name`.
pub type Params = std::collections::BTreeMap<String, Value>;

/// Builds [`Params`]: `params! { "name" => "Alice", "age" => 25 }`.
#[macro_export]
macro_rules! params {
    () => { $crate::Params::new() };
    ($($key:expr => $value:expr),+ $(,)?) => {{
        let mut p = $crate::Params::new();
        $( p.insert(::std::string::ToString::to_string(&$key), $crate::Value::from($value)); )+
        p
    }};
}

/// An open MyGraph instance (all of its databases).
pub struct Database {
    inner: Mutex<Inner>,
}

struct Session {
    database: String,
    schema: String,
}

struct Inner {
    catalog: Catalog,
    store: Option<Store>,
    session: Session,
    config: Config,
}

impl Database {
    /// A purely in-memory instance; nothing is written to disk.
    pub fn in_memory() -> Result<Self> {
        Ok(Self::from_parts(Catalog::new(), None, Config::default()))
    }

    /// Opens (or creates) a persistent instance. Accepts a path or a [`Config`].
    pub fn open(config: impl Into<Config>) -> Result<Self> {
        let config = config.into();
        let Some(path) = config.storage_path.clone() else {
            return Ok(Self::from_parts(Catalog::new(), None, config));
        };
        let (store, catalog) = Store::open(&path, &StoreConfig { sync_on_commit: config.sync_on_commit })?;
        Ok(Self::from_parts(catalog, Some(store), config))
    }

    fn from_parts(catalog: Catalog, store: Option<Store>, config: Config) -> Self {
        let session = Session { database: DEFAULT_DATABASE.into(), schema: DEFAULT_SCHEMA.into() };
        Database { inner: Mutex::new(Inner { catalog, store, session, config }) }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock cannot leave a half-applied statement
        // committed: changes are only persisted after a statement succeeds.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Executes a script of one or more `;`-separated statements. Each
    /// statement runs in its own transaction (auto-commit). The whole script is
    /// parsed before anything runs. Returns the last statement's rows, with
    /// stats summed over all statements.
    pub fn execute(&self, query: &str, params: Params) -> Result<QueryResult> {
        let statements = mygraph_cypher::parse(query)?;
        let mut inner = self.lock();
        let mut result = QueryResult::default();
        for stmt in &statements {
            let r = inner.run_statement(stmt, &params)?;
            let stats = std::mem::take(&mut result.stats);
            result = r;
            result.stats += &stats;
        }
        Ok(result)
    }

    /// Starts an explicit transaction on the current database and schema.
    /// Dropping it without calling [`Transaction::commit`] rolls it back.
    pub fn begin_transaction(&self) -> Result<Transaction<'_>> {
        let inner = self.lock();
        let (database, schema) = (inner.session.database.clone(), inner.session.schema.clone());
        inner.catalog.database(&database)?.schema(&schema)?;
        Ok(Transaction { inner, journal: Journal::default(), database, schema, finished: false })
    }

    /// Writes a snapshot and truncates the WAL. No-op for in-memory instances.
    pub fn checkpoint(&self) -> Result<()> {
        self.lock().checkpoint()
    }

    /// The database that statements currently run against.
    pub fn current_database(&self) -> String {
        self.lock().session.database.clone()
    }

    /// The schema that statements currently run against.
    pub fn current_schema(&self) -> String {
        self.lock().session.schema.clone()
    }

    /// What happened during recovery when the instance was opened.
    pub fn recovery_info(&self) -> Option<RecoveryInfo> {
        self.lock().store.as_ref().map(|s| s.recovery_info().clone())
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        // Best effort: a failed checkpoint loses nothing, the WAL is still there.
        let _ = self.lock().checkpoint();
    }
}

fn empty_result() -> QueryResult {
    QueryResult::default()
}

impl Inner {
    fn run_statement(&mut self, stmt: &Statement, params: &Params) -> Result<QueryResult> {
        match stmt {
            Statement::Admin(cmd) => self.run_admin(cmd),
            Statement::Query(q) => {
                let (db, schema) = (self.session.database.clone(), self.session.schema.clone());
                let mut journal = Journal::default();
                match self.execute_in(&db, &schema, q, params, &mut journal) {
                    Ok(r) => {
                        self.persist(&db, &schema, journal)?;
                        Ok(r.into())
                    }
                    Err(e) => {
                        self.rollback_to(&db, &schema, &mut journal, 0)?;
                        Err(e)
                    }
                }
            }
            Statement::Begin | Statement::Commit | Statement::Rollback => Err(MyGraphError::Transaction(
                "use Database::begin_transaction() and Transaction::commit()/rollback() in embedded mode".into(),
            )),
        }
    }

    fn execute_in(
        &mut self,
        db: &str,
        schema: &str,
        q: &Query,
        params: &Params,
        journal: &mut Journal,
    ) -> Result<ExecResult> {
        let (graph, ids) = self.catalog.database_mut(db)?.write_parts(schema)?;
        let mut ctx = WriteContext::new(graph, ids, journal);
        mygraph_cypher::execute(q, &mut ctx, params)
    }

    fn rollback_to(&mut self, db: &str, schema: &str, journal: &mut Journal, mark: usize) -> Result<()> {
        let (graph, _) = self.catalog.database_mut(db)?.write_parts(schema)?;
        journal.rollback_to(graph, mark)
    }

    /// Makes a journal durable. If the WAL write fails, the in-memory changes
    /// are rolled back so memory never runs ahead of disk.
    fn persist(&mut self, db: &str, schema: &str, mut journal: Journal) -> Result<()> {
        if journal.is_empty() {
            return Ok(());
        }
        let Some(store) = self.store.as_mut() else { return Ok(()) };
        let ops = journal
            .changes()
            .iter()
            .map(|c| Op::Graph { database: db.into(), schema: schema.into(), change: c.clone() })
            .collect();
        if let Err(e) = store.commit(ops) {
            self.rollback_to(db, schema, &mut journal, 0)?;
            return Err(e);
        }
        self.maybe_checkpoint()
    }

    fn maybe_checkpoint(&mut self) -> Result<()> {
        let due = self.store.as_ref().is_some_and(|s| s.wal_size() >= self.config.checkpoint_wal_bytes);
        if due {
            self.checkpoint()?;
        }
        Ok(())
    }

    fn checkpoint(&mut self) -> Result<()> {
        match self.store.as_mut() {
            Some(store) => store.checkpoint(&self.catalog),
            None => Ok(()),
        }
    }

    /// Validates, logs, then applies a catalog operation.
    fn apply_logged(&mut self, op: Op) -> Result<()> {
        self.catalog.validate(&op)?;
        if let Some(store) = self.store.as_mut() {
            store.commit(vec![op.clone()])?;
        }
        self.catalog.apply(&op)?;
        self.maybe_checkpoint()
    }

    fn run_admin(&mut self, cmd: &AdminCommand) -> Result<QueryResult> {
        let current_db = self.session.database.clone();
        match cmd {
            AdminCommand::CreateDatabase { name, if_not_exists } => {
                if !(*if_not_exists && self.catalog.database(name).is_ok()) {
                    self.apply_logged(Op::CreateDatabase { name: name.clone() })?;
                }
            }
            AdminCommand::DropDatabase { name, if_exists } => {
                if !(*if_exists && self.catalog.database(name).is_err()) {
                    self.apply_logged(Op::DropDatabase { name: name.clone() })?;
                    if *name == current_db {
                        self.session = Session { database: DEFAULT_DATABASE.into(), schema: DEFAULT_SCHEMA.into() };
                    }
                }
            }
            AdminCommand::UseDatabase(name) => {
                let db = self.catalog.database(name)?;
                let schema = if db.schema(DEFAULT_SCHEMA).is_ok() {
                    DEFAULT_SCHEMA.to_string()
                } else {
                    db.schemas().next().map_or_else(|| DEFAULT_SCHEMA.to_string(), |s| s.name.clone())
                };
                self.session = Session { database: name.clone(), schema };
            }
            AdminCommand::CreateSchema { name, if_not_exists } => {
                let exists = self.catalog.database(&current_db)?.schema(name).is_ok();
                if !(*if_not_exists && exists) {
                    self.apply_logged(Op::CreateSchema { database: current_db, name: name.clone() })?;
                }
            }
            AdminCommand::DropSchema { name, if_exists } => {
                let exists = self.catalog.database(&current_db)?.schema(name).is_ok();
                if !(*if_exists && !exists) {
                    self.apply_logged(Op::DropSchema { database: current_db, name: name.clone() })?;
                }
            }
            AdminCommand::UseSchema(name) => {
                self.catalog.database(&current_db)?.schema(name)?;
                self.session.schema = name.clone();
            }
            AdminCommand::ShowDatabases => {
                return Ok(QueryResult {
                    columns: vec!["name".into(), "schemas".into(), "current".into()],
                    rows: self
                        .catalog
                        .databases()
                        .map(|db| {
                            vec![
                                Value::from(db.name.as_str()),
                                Value::Integer(db.schemas().count() as i64),
                                Value::Boolean(db.name == current_db),
                            ]
                        })
                        .collect(),
                    stats: QueryStats::default(),
                });
            }
            AdminCommand::ShowSchemas => {
                let db = self.catalog.database(&current_db)?;
                return Ok(QueryResult {
                    columns: vec!["name".into(), "nodes".into(), "relationships".into(), "current".into()],
                    rows: db
                        .schemas()
                        .map(|s| {
                            vec![
                                Value::from(s.name.as_str()),
                                Value::Integer(s.graph.node_count() as i64),
                                Value::Integer(s.graph.relationship_count() as i64),
                                Value::Boolean(s.name == self.session.schema),
                            ]
                        })
                        .collect(),
                    stats: QueryStats::default(),
                });
            }
        }
        Ok(empty_result())
    }
}

/// An explicit read/write transaction bound to one database and schema.
///
/// Each statement inside the transaction is atomic: a failing statement is
/// undone, earlier statements are kept. `commit` makes everything durable;
/// `rollback` (or dropping the transaction) undoes everything.
pub struct Transaction<'db> {
    inner: MutexGuard<'db, Inner>,
    journal: Journal,
    database: String,
    schema: String,
    finished: bool,
}

impl Transaction<'_> {
    pub fn execute(&mut self, query: &str, params: Params) -> Result<QueryResult> {
        let statements = mygraph_cypher::parse(query)?;
        if statements.iter().any(|s| !matches!(s, Statement::Query(_))) {
            return Err(MyGraphError::Transaction(
                "administrative and transaction-control statements are not allowed inside a transaction".into(),
            ));
        }
        let mut result = QueryResult::default();
        for stmt in &statements {
            let Statement::Query(q) = stmt else { unreachable!() };
            let mark = self.journal.mark();
            match self.inner.execute_in(&self.database, &self.schema, q, &params, &mut self.journal) {
                Ok(r) => {
                    let stats = std::mem::take(&mut result.stats);
                    result = r.into();
                    result.stats += &stats;
                }
                Err(e) => {
                    self.inner.rollback_to(&self.database, &self.schema, &mut self.journal, mark)?;
                    return Err(e);
                }
            }
        }
        Ok(result)
    }

    pub fn commit(mut self) -> Result<()> {
        self.finished = true;
        let journal = std::mem::take(&mut self.journal);
        let (db, schema) = (self.database.clone(), self.schema.clone());
        self.inner.persist(&db, &schema, journal)
    }

    pub fn rollback(mut self) -> Result<()> {
        self.finished = true;
        self.undo_all()
    }

    fn undo_all(&mut self) -> Result<()> {
        let mut journal = std::mem::take(&mut self.journal);
        let (db, schema) = (self.database.clone(), self.schema.clone());
        self.inner.rollback_to(&db, &schema, &mut journal, 0)
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.undo_all();
        }
    }
}
