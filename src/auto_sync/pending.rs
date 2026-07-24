//! Durable pending generation state with integrity checks.

use crate::auto_sync::pending_lock::{self, PendingTxnLockError};
use crate::auto_sync::policy::MutationKind;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PENDING_FILE_NAME: &str = "auto-sync-pending.toml";
pub const STALE_PENDING_THRESHOLD_MS: u64 = 5 * 60 * 1000;
const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum PendingSnapshot {
    #[default]
    None,
    Mutation {
        kind: MutationKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingState {
    pub generation: u64,
    pub snapshot: PendingSnapshot,
    pub created_at_unix_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct PendingOnDisk {
    schema: u32,
    generation: u64,
    created_at_unix_ms: u64,
    snapshot: PendingSnapshot,
    integrity: String,
}

#[derive(Serialize, Deserialize)]
struct LegacyPendingOnDiskV1 {
    kind: MutationKind,
    created_at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionalClearResult {
    Cleared,
    Missing,
    GenerationChanged { current: u64 },
}

pub fn pending_path(state_dir: &Path) -> PathBuf {
    state_dir.join(PENDING_FILE_NAME)
}

/// Records a logical mutation in the durable pending marker.
///
/// This is the **only** public API that increments the pending generation.
/// It is called by the parent after a successful local commit. Subsequent
/// scheduling (see `crate::auto_sync::schedule::schedule_and_spawn`)
/// must not mutate pending state, change the generation, or replace the
/// snapshot.
///
/// The entire read-modify-write is performed under a short-lived
/// `PendingTxnGuard` to serialize concurrent CLI processes.
pub fn record_pending_mutation(
    state_dir: &Path,
    snapshot: PendingSnapshot,
) -> Result<PendingState, PendingError> {
    let _guard =
        pending_lock::acquire_pending_txn(state_dir, std::time::Duration::from_millis(500))
            .map_err(PendingError::Lock)?;

    let path = pending_path(state_dir);
    let (new_generation, created_at_ms) = match read_state(&path) {
        Ok(existing) => (existing.generation.saturating_add(1), unix_now_ms()),
        Err(PendingError::NotFound) => (1u64, unix_now_ms()),
        Err(e) => return Err(e),
    };

    write_pending_state(state_dir, &path, new_generation, created_at_ms, snapshot)
}

pub fn read_state(path: &Path) -> Result<PendingState, PendingError> {
    let contents = std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            PendingError::NotFound
        } else {
            PendingError::Io(e)
        }
    })?;
    parse(&contents)
}

pub fn read_state_from_dir(state_dir: &Path) -> Result<PendingState, PendingError> {
    read_state(&pending_path(state_dir))
}

/// Atomically clears the pending marker only if the current generation
/// matches the observed generation.
///
/// The read-compare-delete is performed under `PendingTxnGuard` to prevent
/// a newer generation from being deleted by a stale clear.
pub fn clear_if_generation_matches(
    state_dir: &Path,
    observed_generation: u64,
) -> Result<ConditionalClearResult, PendingError> {
    let _guard =
        pending_lock::acquire_pending_txn(state_dir, std::time::Duration::from_millis(500))
            .map_err(PendingError::Lock)?;

    let path = pending_path(state_dir);
    let current = match read_state(&path) {
        Ok(s) => s,
        Err(PendingError::NotFound) => return Ok(ConditionalClearResult::Missing),
        Err(e) => return Err(e),
    };

    if current.generation == observed_generation {
        remove_secure(&path)?;
        Ok(ConditionalClearResult::Cleared)
    } else {
        Ok(ConditionalClearResult::GenerationChanged {
            current: current.generation,
        })
    }
}

pub fn clear(state_dir: &Path) -> Result<(), PendingError> {
    let path = pending_path(state_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(PendingError::Io(e)),
    }
}

pub fn record_success(
    state_dir: &Path,
    observed_generation: u64,
) -> Result<ConditionalClearResult, PendingError> {
    clear_if_generation_matches(state_dir, observed_generation)
}

pub fn record_failure(
    state_dir: &Path,
    observed_generation: u64,
    classification: &str,
) -> Result<(), PendingError> {
    let path = pending_path(state_dir);
    let current = match read_state(&path) {
        Ok(s) => s,
        Err(PendingError::NotFound) => return Ok(()),
        Err(e) => return Err(e),
    };

    if current.generation != observed_generation {
        return Ok(());
    }

    tracing::warn!(
        generation = current.generation,
        classification = classification,
        "auto-sync failure recorded; pending state preserved for recovery"
    );
    Ok(())
}

pub fn clear_for_explicit_sync(state_dir: &Path) -> Result<(), PendingError> {
    clear(state_dir)
}

#[cfg(test)]
pub fn set_local_generation(state_dir: &Path, generation: u64) -> Result<(), PendingError> {
    let path = pending_path(state_dir);
    let snapshot = PendingSnapshot::Mutation {
        kind: MutationKind::SnippetCreate,
    };
    write_pending_state(state_dir, &path, generation, unix_now_ms(), snapshot)?;
    Ok(())
}

#[cfg(test)]
pub fn set_local_generation_with_timestamp(
    state_dir: &Path,
    generation: u64,
    created_at_ms: u64,
) -> Result<(), PendingError> {
    let path = pending_path(state_dir);
    let snapshot = PendingSnapshot::Mutation {
        kind: MutationKind::SnippetCreate,
    };
    write_pending_state(state_dir, &path, generation, created_at_ms, snapshot)?;
    Ok(())
}

#[derive(Debug)]
pub enum PendingError {
    Io(std::io::Error),
    Serialize(toml::ser::Error),
    Deserialize(toml::de::Error),
    IntegrityMismatch { expected: String, got: String },
    NotFound,
    Lock(PendingTxnLockError),
    Corrupted(String),
}

impl std::fmt::Display for PendingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Serialize(e) => write!(f, "toml serialize error: {e}"),
            Self::Deserialize(e) => write!(f, "toml deserialize error: {e}"),
            Self::IntegrityMismatch { expected, got } => {
                write!(f, "integrity mismatch: expected {expected}, got {got}")
            }
            Self::NotFound => write!(f, "pending state not found"),
            Self::Lock(e) => write!(f, "pending txn lock error: {e}"),
            Self::Corrupted(msg) => write!(f, "corrupted pending state: {msg}"),
        }
    }
}

impl std::error::Error for PendingError {}

fn write_pending_state(
    _state_dir: &Path,
    path: &Path,
    generation: u64,
    created_at_ms: u64,
    snapshot: PendingSnapshot,
) -> Result<PendingState, PendingError> {
    let on_disk = build_pending_on_disk(generation, created_at_ms, snapshot.clone());
    let serialized = toml::to_string_pretty(&on_disk).map_err(PendingError::Serialize)?;
    let _tmp =
        pending_lock::atomic_write_unique(path, serialized.as_bytes()).map_err(PendingError::Io)?;
    restrict_permissions(path);
    pending_lock::fsync_parent_dir(path);

    Ok(PendingState {
        generation,
        snapshot: on_disk.snapshot,
        created_at_unix_ms: created_at_ms,
    })
}

fn build_pending_on_disk(
    generation: u64,
    created_at_ms: u64,
    snapshot: PendingSnapshot,
) -> PendingOnDisk {
    let integrity = compute_integrity(SCHEMA_VERSION, generation, created_at_ms, &snapshot);
    PendingOnDisk {
        schema: SCHEMA_VERSION,
        generation,
        created_at_unix_ms: created_at_ms,
        snapshot,
        integrity,
    }
}

/// Computes CRC32 integrity over all behavior-driving fields.
///
/// This covers schema, generation, created_at_unix_ms, and the serialized
/// snapshot — ensuring any corruption to these fields is detected.
fn compute_integrity(
    schema: u32,
    generation: u64,
    created_at_ms: u64,
    snapshot: &PendingSnapshot,
) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&schema.to_le_bytes());
    bytes.extend_from_slice(&generation.to_le_bytes());
    bytes.extend_from_slice(&created_at_ms.to_le_bytes());
    if let Ok(snapshot_bytes) = serialize_snapshot(snapshot) {
        bytes.extend_from_slice(&snapshot_bytes);
    }
    let hash = crc32(&bytes);
    format!("crc32:{hash:08x}")
}

fn parse(contents: &str) -> Result<PendingState, PendingError> {
    if let Ok(on_disk) = toml::from_str::<PendingOnDisk>(contents) {
        if on_disk.schema != SCHEMA_VERSION {
            return Err(PendingError::Deserialize(
                <toml::de::Error as serde::de::Error>::custom("unsupported schema"),
            ));
        }
        let expected = compute_integrity(
            on_disk.schema,
            on_disk.generation,
            on_disk.created_at_unix_ms,
            &on_disk.snapshot,
        );
        if on_disk.integrity != expected {
            return Err(PendingError::IntegrityMismatch {
                expected: on_disk.integrity.clone(),
                got: expected,
            });
        }
        return Ok(PendingState {
            generation: on_disk.generation,
            snapshot: on_disk.snapshot,
            created_at_unix_ms: on_disk.created_at_unix_ms,
        });
    }

    if let Ok(v1) = toml::from_str::<LegacyPendingOnDiskV1>(contents) {
        tracing::debug!("migrating legacy pending marker v1 to v2");
        return Ok(PendingState {
            generation: 1,
            snapshot: PendingSnapshot::Mutation { kind: v1.kind },
            created_at_unix_ms: v1.created_at_unix_ms,
        });
    }

    Err(PendingError::Deserialize(
        <toml::de::Error as serde::de::Error>::custom("failed to parse pending state"),
    ))
}

fn serialize_snapshot(snapshot: &PendingSnapshot) -> Result<Vec<u8>, PendingError> {
    let value = toml::Value::try_from(snapshot).map_err(PendingError::Serialize)?;
    Ok(value.to_string().into_bytes())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0xFFFF_FFFF;
    for &b in bytes {
        hash ^= b as u32;
        for _ in 0..8 {
            if hash & 1 != 0 {
                hash = (hash >> 1) ^ 0xEDB8_8320;
            } else {
                hash >>= 1;
            }
        }
    }
    !hash
}

fn restrict_permissions(#[cfg_attr(not(unix), allow(unused_variables))] path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

fn remove_secure(path: &Path) -> Result<(), PendingError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(PendingError::Io(e)),
    }
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_mark_creates_pending_state() {
        let dir = TempDir::new().unwrap();
        let state = record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        assert_eq!(state.generation, 1);
    }

    #[test]
    fn test_mark_increments_generation() {
        let dir = TempDir::new().unwrap();
        let s1 = record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let s2 = record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetUpdate,
            },
        )
        .unwrap();
        assert_eq!(s1.generation, 1);
        assert_eq!(s2.generation, 2);
    }

    #[test]
    fn test_clear_if_generation_matches_succeeds() {
        let dir = TempDir::new().unwrap();
        let s = record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let result = clear_if_generation_matches(dir.path(), s.generation).unwrap();
        assert_eq!(result, ConditionalClearResult::Cleared);
        assert!(matches!(
            read_state_from_dir(dir.path()),
            Err(PendingError::NotFound)
        ));
    }

    #[test]
    fn test_clear_if_generation_mismatched() {
        let dir = TempDir::new().unwrap();
        let _lock = record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let result = clear_if_generation_matches(dir.path(), 999).unwrap();
        assert_eq!(
            result,
            ConditionalClearResult::GenerationChanged { current: 1 }
        );
        assert!(read_state_from_dir(dir.path()).is_ok());
    }

    #[test]
    fn test_clear_removes_marker() {
        let dir = TempDir::new().unwrap();
        let _lock = record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        clear(dir.path()).unwrap();
        assert!(matches!(
            read_state_from_dir(dir.path()),
            Err(PendingError::NotFound)
        ));
    }

    #[test]
    fn test_record_failure_preserves_state() {
        let dir = TempDir::new().unwrap();
        let s = record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        record_failure(dir.path(), s.generation, "network").unwrap();
        let current = read_state_from_dir(dir.path()).unwrap();
        assert_eq!(current.generation, s.generation);
    }

    #[test]
    fn test_pending_file_permissions() {
        let dir = TempDir::new().unwrap();
        let _lock = record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(pending_path(dir.path())).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn test_legacy_v1_migration() {
        let dir = TempDir::new().unwrap();
        let path = pending_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"kind = "snippet_create"
created_at_unix_ms = 1700000000000"#,
        )
        .unwrap();

        let state = read_state_from_dir(dir.path()).unwrap();
        assert_eq!(state.generation, 1);
        assert_eq!(
            state.snapshot,
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate
            }
        );
    }

    #[test]
    fn test_no_secrets_in_disk_format() {
        let dir = TempDir::new().unwrap();
        let _lock = record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let raw = std::fs::read_to_string(pending_path(dir.path())).unwrap();
        for forbidden in [
            "command",
            "description",
            "password",
            "secret",
            "api_key",
            "apikey",
            "token",
            "credential",
        ] {
            assert!(
                !raw.contains(forbidden),
                "pending state must not contain {forbidden}; raw = {raw}"
            );
        }
    }

    #[test]
    fn test_crc32_basic() {
        assert_eq!(crc32(b""), 0);
        assert_ne!(crc32(b"hello"), 0);
        assert_eq!(crc32(b"hello"), crc32(b"hello"));
    }

    #[test]
    fn test_stale_threshold_is_five_minutes() {
        assert_eq!(STALE_PENDING_THRESHOLD_MS, 300_000);
    }

    #[test]
    fn test_record_success_clears_state() {
        let dir = TempDir::new().unwrap();
        let s = record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        record_success(dir.path(), s.generation).unwrap();
        assert!(matches!(
            read_state_from_dir(dir.path()),
            Err(PendingError::NotFound)
        ));
    }

    #[test]
    fn test_record_failure_with_mismatched_generation_is_noop() {
        let dir = TempDir::new().unwrap();
        let _lock = record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        record_failure(dir.path(), 999, "network").unwrap();
        let current = read_state_from_dir(dir.path()).unwrap();
        assert_eq!(current.generation, 1);
    }

    #[test]
    fn test_pending_path_inside_state_dir() {
        let dir = TempDir::new().unwrap();
        let p = pending_path(dir.path());
        assert!(p.starts_with(dir.path()));
        assert!(p.ends_with(PENDING_FILE_NAME));
    }

    #[test]
    fn test_schema_version_constant() {
        assert_eq!(SCHEMA_VERSION, 2);
    }

    #[test]
    fn test_full_state_integrity_detects_generation_corruption() {
        let dir = TempDir::new().unwrap();
        record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();

        let path = pending_path(dir.path());
        let raw = std::fs::read_to_string(&path).unwrap();
        let corrupted = raw.replace("generation = 1", "generation = 999");
        std::fs::write(&path, corrupted).unwrap();

        let result = read_state(&path);
        assert!(matches!(
            result,
            Err(PendingError::IntegrityMismatch { .. })
        ));
    }

    #[test]
    fn test_full_state_integrity_detects_timestamp_corruption() {
        let dir = TempDir::new().unwrap();
        record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();

        let path = pending_path(dir.path());
        let raw = std::fs::read_to_string(&path).unwrap();
        let corrupted = raw.replace("created_at_unix_ms", "created_at_unix_ms_corrupted");
        std::fs::write(&path, corrupted).unwrap();

        let result = read_state(&path);
        assert!(matches!(
            result,
            Err(PendingError::IntegrityMismatch { .. }) | Err(PendingError::Deserialize(_))
        ));
    }

    #[test]
    fn test_conditional_clear_result_missing() {
        let dir = TempDir::new().unwrap();
        let result = clear_if_generation_matches(dir.path(), 1).unwrap();
        assert_eq!(result, ConditionalClearResult::Missing);
    }

    #[test]
    fn test_conditional_clear_result_cleared() {
        let dir = TempDir::new().unwrap();
        let s = record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let result = clear_if_generation_matches(dir.path(), s.generation).unwrap();
        assert_eq!(result, ConditionalClearResult::Cleared);
    }

    #[test]
    fn test_conditional_clear_result_generation_changed() {
        let dir = TempDir::new().unwrap();
        record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetUpdate,
            },
        )
        .unwrap();
        let result = clear_if_generation_matches(dir.path(), 1).unwrap();
        assert_eq!(
            result,
            ConditionalClearResult::GenerationChanged { current: 2 }
        );
    }

    #[test]
    fn test_set_local_generation_bypasses_txn_lock() {
        let dir = TempDir::new().unwrap();
        set_local_generation(dir.path(), 42).unwrap();
        let state = read_state_from_dir(dir.path()).unwrap();
        assert_eq!(state.generation, 42);
    }
}
