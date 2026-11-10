use std::path::{Path, PathBuf};

/// Instance configuration.
///
/// ```
/// use mygraph::Config;
/// let config = Config::default().storage_path("./mygraph_data").sync_on_commit(true);
/// ```
#[derive(Debug, Clone)]
pub struct Config {
    pub(crate) storage_path: Option<PathBuf>,
    pub(crate) cache_size_mb: usize,
    pub(crate) sync_on_commit: bool,
    pub(crate) checkpoint_wal_bytes: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config { storage_path: None, cache_size_mb: 1024, sync_on_commit: true, checkpoint_wal_bytes: 64 * 1024 * 1024 }
    }
}

impl Config {
    /// Data directory. Without one, the instance is in-memory only.
    pub fn storage_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.storage_path = Some(path.into());
        self
    }

    /// Page cache size. Reserved for the page-based store (Phase 3); v0.1
    /// keeps the whole graph in memory and ignores this value.
    pub fn cache_size_mb(mut self, mb: usize) -> Self {
        self.cache_size_mb = mb;
        self
    }

    /// fsync the WAL on every commit (default `true`). With `false`, a crash
    /// may lose the most recent commits but never corrupts the database.
    pub fn sync_on_commit(mut self, sync: bool) -> Self {
        self.sync_on_commit = sync;
        self
    }

    /// WAL size that triggers an automatic checkpoint (default 64 MiB).
    pub fn checkpoint_wal_bytes(mut self, bytes: u64) -> Self {
        self.checkpoint_wal_bytes = bytes;
        self
    }

    pub fn get_storage_path(&self) -> Option<&Path> {
        self.storage_path.as_deref()
    }

    pub fn get_cache_size_mb(&self) -> usize {
        self.cache_size_mb
    }
}

impl From<&str> for Config {
    fn from(p: &str) -> Self {
        Config::default().storage_path(p)
    }
}

impl From<String> for Config {
    fn from(p: String) -> Self {
        Config::default().storage_path(p)
    }
}

impl From<&Path> for Config {
    fn from(p: &Path) -> Self {
        Config::default().storage_path(p)
    }
}

impl From<PathBuf> for Config {
    fn from(p: PathBuf) -> Self {
        Config::default().storage_path(p)
    }
}

impl From<&PathBuf> for Config {
    fn from(p: &PathBuf) -> Self {
        Config::default().storage_path(p.clone())
    }
}
