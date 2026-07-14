//! Local-only per-snippet usage metadata.
//!
//! Tracks how often and when each snippet has been used.  The data is stored
//! in `~/.config/snp/usage.toml` and is intentionally isolated from the
//! snippet library — no command bodies are logged and no remote sync is
//! performed.
//!
//! # File format
//!
//! ```toml
//! [[usage]]
//! id = "snippet-uuid"
//! use_count = 5
//! last_used_at = 1700000000
//! ```

use crate::error::{SnipError, SnipResult};
use crate::utils::atomic::write_private_atomic;
use crate::utils::config::get_config_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Return type for [`UsageIndex::get_usage`].
///
/// A plain data struct rather than a reference into the index so callers
/// don't need to hold a borrow on the [`UsageIndex`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageData {
    pub use_count: u64,
    pub last_used_at: Option<i64>,
}

/// Persistent per-snippet usage index.
///
/// Each entry records how many times a snippet was selected and the
/// timestamp of the most recent use.  The index is stored as a TOML
/// array-of-tables at `~/.config/snp/usage.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageIndex {
    #[serde(default)]
    entries: Vec<UsageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageEntry {
    pub id: String,
    #[serde(default)]
    pub use_count: u64,
    #[serde(default)]
    pub last_used_at: Option<i64>,
}

/// Path to the usage TOML file.
fn usage_path() -> PathBuf {
    get_config_dir().join("usage.toml")
}

impl UsageIndex {
    /// Load the usage index from disk.
    ///
    /// Returns an empty index if the file is missing, unreadable, or
    /// contains invalid TOML (fail-open semantics).
    pub fn load() -> Self {
        Self::load_from(&usage_path())
    }

    /// Persist the index to disk via an atomic write.
    pub fn save(&self) -> SnipResult<()> {
        self.save_to(&usage_path())
    }

    /// Record a use of the given snippet.
    ///
    /// Increments `use_count` and sets `last_used_at` to the current
    /// Unix timestamp.  If no entry exists for `snippet_id` a new one
    /// is created.
    pub fn record_use(&mut self, snippet_id: &str) {
        let now = chrono::Utc::now().timestamp();
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == snippet_id) {
            entry.use_count = entry.use_count.saturating_add(1);
            entry.last_used_at = Some(now);
        } else {
            self.entries.push(UsageEntry {
                id: snippet_id.to_string(),
                use_count: 1,
                last_used_at: Some(now),
            });
        }
    }

    /// Return usage data for a single snippet.
    ///
    /// Returns zeroed defaults when no entry exists for `snippet_id`.
    pub fn get_usage(&self, snippet_id: &str) -> UsageData {
        match self.entries.iter().find(|e| e.id == snippet_id) {
            Some(entry) => UsageData {
                use_count: entry.use_count,
                last_used_at: entry.last_used_at,
            },
            None => UsageData::default(),
        }
    }

    /// Remove entries whose `id` is not in `active_ids`.
    ///
    /// Useful for lazily pruning records of snippets that have been
    /// deleted from the library.
    pub fn prune(&mut self, active_ids: &[String]) {
        self.entries.retain(|e| active_ids.contains(&e.id));
    }

    /// Return a reference to all entries.
    pub fn entries(&self) -> &[UsageEntry] {
        &self.entries
    }

    // -- internal helpers (accept explicit paths so tests avoid env-var races) --

    fn load_from(path: &Path) -> Self {
        let data = match fs::read_to_string(path) {
            Ok(d) => d,
            Err(_) => return Self::default(),
        };
        toml::from_str::<UsageIndex>(&data).unwrap_or_default()
    }

    fn save_to(&self, path: &Path) -> SnipResult<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| SnipError::toml_error("serialize usage", e))?;
        write_private_atomic(path, &content, "usage")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a temp dir and return it (callers hold the guard to
    /// keep the directory alive).
    fn temp_dir() -> TempDir {
        TempDir::new().expect("create temp dir")
    }

    #[test]
    fn load_returns_empty_when_file_missing() {
        let dir = temp_dir();
        let path = dir.path().join("nonexistent").join("usage.toml");
        let idx = UsageIndex::load_from(&path);
        assert!(idx.entries.is_empty());
    }

    #[test]
    fn record_use_increments_count() {
        let mut idx = UsageIndex::default();
        idx.record_use("abc-123");
        assert_eq!(idx.get_usage("abc-123").use_count, 1);

        idx.record_use("abc-123");
        assert_eq!(idx.get_usage("abc-123").use_count, 2);

        idx.record_use("abc-123");
        assert_eq!(idx.get_usage("abc-123").use_count, 3);
    }

    #[test]
    fn record_use_sets_timestamp() {
        let mut idx = UsageIndex::default();
        let before = chrono::Utc::now().timestamp();
        idx.record_use("abc-123");
        let after = chrono::Utc::now().timestamp();

        let usage = idx.get_usage("abc-123");
        assert!(usage.last_used_at.is_some());
        let ts = usage.last_used_at.unwrap();
        assert!(
            ts >= before && ts <= after,
            "timestamp {ts} not in [{before}, {after}]"
        );
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = temp_dir();
        let path = dir.path().join("usage.toml");

        let mut idx = UsageIndex::default();
        idx.record_use("aaa-111");
        idx.record_use("bbb-222");
        idx.record_use("aaa-111");
        idx.save_to(&path).expect("save");

        let loaded = UsageIndex::load_from(&path);
        assert_eq!(loaded.get_usage("aaa-111").use_count, 2);
        assert_eq!(loaded.get_usage("bbb-222").use_count, 1);
        assert_eq!(loaded.get_usage("missing").use_count, 0);
    }

    #[test]
    fn corrupt_file_fails_open() {
        let dir = temp_dir();
        let path = dir.path().join("usage.toml");
        fs::write(&path, "{{{{invalid toml").unwrap();

        let idx = UsageIndex::load_from(&path);
        assert!(
            idx.entries.is_empty(),
            "corrupt file should fail open to empty index"
        );
    }

    #[test]
    fn prune_removes_stale_entries() {
        let mut idx = UsageIndex::default();
        idx.record_use("keep-1");
        idx.record_use("keep-2");
        idx.record_use("drop-3");

        let active = vec!["keep-1".to_string(), "keep-2".to_string()];
        idx.prune(&active);

        assert_eq!(idx.get_usage("keep-1").use_count, 1);
        assert_eq!(idx.get_usage("keep-2").use_count, 1);
        assert_eq!(idx.get_usage("drop-3").use_count, 0);
        assert_eq!(idx.entries().len(), 2);
    }

    #[test]
    fn get_usage_returns_default_for_unknown() {
        let idx = UsageIndex::default();
        let usage = idx.get_usage("nonexistent-id");
        assert_eq!(usage, UsageData::default());
        assert_eq!(usage.use_count, 0);
        assert!(usage.last_used_at.is_none());
    }
}
