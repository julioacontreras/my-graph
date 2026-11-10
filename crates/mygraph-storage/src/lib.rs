//! MyGraph persistence.
//!
//! v0.1 storage model: the whole catalog lives in memory; durability comes
//! from a write-ahead log plus periodic full snapshots (checkpoints).
//!
//! ```text
//! <data dir>/
//! ├── LOCK           exclusive lock held while the instance is open
//! ├── snapshot.dat   last checkpoint (versioned, CRC32-protected)
//! └── wal.log        committed transactions since that checkpoint
//! ```
//!
//! Recovery = load snapshot, then replay WAL records with LSN > snapshot LSN.
//! The page-based store with buffer pool (spec §6) replaces this in Phase 3;
//! the `Store` API is meant to survive that change.

mod codec;
pub mod snapshot;
pub mod wal;

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use mygraph_core::{Catalog, MyGraphError, Op, Result};

use wal::Wal;
pub use wal::WalRecord;

#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// fsync the WAL on every commit.
    pub sync_on_commit: bool,
}

impl Default for StoreConfig {
    fn default() -> Self {
        StoreConfig { sync_on_commit: true }
    }
}

/// Information gathered while opening a store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryInfo {
    pub snapshot_lsn: u64,
    pub replayed_records: usize,
    pub truncated_wal_bytes: u64,
}

pub struct Store {
    root: PathBuf,
    wal: Wal,
    last_lsn: u64,
    recovery: RecoveryInfo,
    _lock: File,
}

impl Store {
    /// Opens (creating if needed) a data directory and recovers its catalog.
    pub fn open(root: &Path, config: &StoreConfig) -> Result<(Store, Catalog)> {
        fs::create_dir_all(root)?;
        let lock = OpenOptions::new().create(true).truncate(false).write(true).open(root.join("LOCK"))?;
        lock.try_lock().map_err(|_| {
            MyGraphError::Storage(format!("data directory {} is in use by another MyGraph instance", root.display()))
        })?;

        let (mut catalog, snapshot_lsn) =
            snapshot::read(&root.join("snapshot.dat"))?.unwrap_or_else(|| (Catalog::new(), 0));
        let (wal, records) = Wal::open(&root.join("wal.log"), config.sync_on_commit)?;

        let mut last_lsn = snapshot_lsn;
        let mut replayed = 0;
        for rec in records {
            if rec.lsn <= snapshot_lsn {
                continue;
            }
            if rec.lsn != last_lsn + 1 {
                return Err(MyGraphError::Corruption(format!(
                    "WAL sequence gap: expected LSN {}, found {}",
                    last_lsn + 1,
                    rec.lsn
                )));
            }
            for op in &rec.ops {
                catalog
                    .apply(op)
                    .map_err(|e| MyGraphError::Corruption(format!("WAL record {} cannot be replayed: {e}", rec.lsn)))?;
            }
            last_lsn = rec.lsn;
            replayed += 1;
        }
        let recovery =
            RecoveryInfo { snapshot_lsn, replayed_records: replayed, truncated_wal_bytes: wal.truncated_bytes() };
        Ok((Store { root: root.to_path_buf(), wal, last_lsn, recovery, _lock: lock }, catalog))
    }

    /// Durably records one committed transaction.
    pub fn commit(&mut self, ops: Vec<Op>) -> Result<()> {
        if ops.is_empty() {
            return Ok(());
        }
        let lsn = self.last_lsn + 1;
        self.wal.append(&WalRecord { lsn, ops })?;
        self.last_lsn = lsn;
        Ok(())
    }

    /// Writes a snapshot of `catalog` (which must reflect every committed
    /// record) and truncates the WAL.
    pub fn checkpoint(&mut self, catalog: &Catalog) -> Result<()> {
        snapshot::write(&self.root.join("snapshot.dat"), catalog, self.last_lsn)?;
        self.wal.reset()
    }

    pub fn wal_size(&self) -> u64 {
        self.wal.size()
    }

    pub fn last_lsn(&self) -> u64 {
        self.last_lsn
    }

    pub fn recovery_info(&self) -> &RecoveryInfo {
        &self.recovery
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mygraph_core::{Change, PropertyMap, Value, DEFAULT_DATABASE, DEFAULT_SCHEMA};

    fn node_op(id: u64) -> Op {
        Op::Graph {
            database: DEFAULT_DATABASE.into(),
            schema: DEFAULT_SCHEMA.into(),
            change: Change::CreateNode {
                id,
                labels: vec!["User".into()],
                properties: PropertyMap::from([("n".into(), Value::from(id as i64))]),
                created_at: 0,
                updated_at: 0,
            },
        }
    }

    fn node_count(c: &Catalog) -> usize {
        c.database(DEFAULT_DATABASE).unwrap().schema(DEFAULT_SCHEMA).unwrap().graph.node_count()
    }

    #[test]
    fn wal_replay_and_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = StoreConfig::default();
        {
            let (mut store, mut cat) = Store::open(dir.path(), &cfg).unwrap();
            for id in 0..3 {
                let op = node_op(id);
                cat.apply(&op).unwrap();
                store.commit(vec![op]).unwrap();
            }
        }
        {
            let (mut store, mut cat) = Store::open(dir.path(), &cfg).unwrap();
            assert_eq!(store.recovery_info().replayed_records, 3);
            assert_eq!(node_count(&cat), 3);
            store.checkpoint(&cat).unwrap();
            let op = node_op(3);
            cat.apply(&op).unwrap();
            store.commit(vec![op]).unwrap();
        }
        let (store, cat) = Store::open(dir.path(), &cfg).unwrap();
        assert_eq!(store.recovery_info().snapshot_lsn, 3);
        assert_eq!(store.recovery_info().replayed_records, 1);
        assert_eq!(node_count(&cat), 4);
        assert_eq!(cat.database(DEFAULT_DATABASE).unwrap().ids.next_node_id, 4);
    }

    #[test]
    fn torn_wal_tail_is_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = StoreConfig::default();
        {
            let (mut store, _) = Store::open(dir.path(), &cfg).unwrap();
            store.commit(vec![node_op(0)]).unwrap();
            store.commit(vec![node_op(1)]).unwrap();
        }
        let wal_path = dir.path().join("wal.log");
        let len = fs::metadata(&wal_path).unwrap().len();
        OpenOptions::new().write(true).open(&wal_path).unwrap().set_len(len - 3).unwrap();

        let (store, cat) = Store::open(dir.path(), &cfg).unwrap();
        assert_eq!(node_count(&cat), 1);
        assert!(store.recovery_info().truncated_wal_bytes > 0);
    }

    #[test]
    fn corrupted_snapshot_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        {
            let (mut store, cat) = Store::open(dir.path(), &StoreConfig::default()).unwrap();
            store.checkpoint(&cat).unwrap();
        }
        let path = dir.path().join("snapshot.dat");
        let mut bytes = fs::read(&path).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        fs::write(&path, bytes).unwrap();
        assert!(matches!(Store::open(dir.path(), &StoreConfig::default()), Err(MyGraphError::Corruption(_))));
    }

    #[test]
    fn directory_is_locked_while_open() {
        let dir = tempfile::tempdir().unwrap();
        let _first = Store::open(dir.path(), &StoreConfig::default()).unwrap();
        assert!(Store::open(dir.path(), &StoreConfig::default()).is_err());
    }
}
