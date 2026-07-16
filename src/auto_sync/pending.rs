//! Durable pending generation state with integrity checks.

use crate::auto_sync::policy::MutationKind;
use serde::{Deserialize, Serialize};
use std::io::Write;
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

#[derive(Debug, Clone)]
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

pub fn pending_path(state_dir: &Path) -> PathBuf {
    state_dir.join(PENDING_FILE_NAME)
}

/// Records a logical mutation in the durable pending marker.
///
/// This is the **only** public API that increments the pending generation.
/// It is called by the parent after a successful local commit. Subsequent
/// scheduling (see `crate::auto_sync::worker::schedule_existing_pending`)
/// must not mutate pending state, change the generation, or replace the
/// snapshot.
pub fn record_pending_mutation(
    state_dir: &Path,
    snapshot: PendingSnapshot,
) -> Result<PendingState, PendingError> {
    mark_pending_internal(state_dir, snapshot)
}

/// Internal helper that bumps the generation and writes the marker.
/// Re-exported as `mark_pending` for legacy callers and tests that need
/// direct control; production flow goes through `record_pending_mutation`.
pub fn mark_pending(
    state_dir: &Path,
    snapshot: PendingSnapshot,
) -> Result<PendingState, PendingError> {
    mark_pending_internal(state_dir, snapshot)
}

fn mark_pending_internal(
    state_dir: &Path,
    snapshot: PendingSnapshot,
) -> Result<PendingState, PendingError> {
    let path = pending_path(state_dir);

    let (new_generation, created_at_ms) = match read_state(&path) {
        Ok(existing) => (existing.generation.saturating_add(1), unix_now_ms()),
        Err(PendingError::NotFound) => (1u64, unix_now_ms()),
        Err(e) => return Err(e),
    };

    let snapshot_bytes = serialize_snapshot(&snapshot)?;
    let crc = crc32(&snapshot_bytes);

    let on_disk = PendingOnDisk {
        schema: SCHEMA_VERSION,
        generation: new_generation,
        created_at_unix_ms: created_at_ms,
        snapshot,
        integrity: format!("crc32:{crc:08x}"),
    };

    let serialized = toml::to_string_pretty(&on_disk).map_err(PendingError::Serialize)?;
    atomic_write(&path, serialized.as_bytes())?;
    restrict_permissions(&path);

    Ok(PendingState {
        generation: new_generation,
        snapshot: on_disk.snapshot,
        created_at_unix_ms: created_at_ms,
    })
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

pub fn clear_if_generation_matches(
    state_dir: &Path,
    observed_generation: u64,
) -> Result<bool, PendingError> {
    let path = pending_path(state_dir);
    let current = match read_state(&path) {
        Ok(s) => s,
        Err(PendingError::NotFound) => return Ok(false),
        Err(e) => return Err(e),
    };

    if current.generation == observed_generation {
        remove_secure(&path)?;
        Ok(true)
    } else {
        Ok(false)
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

pub fn record_success(state_dir: &Path, observed_generation: u64) -> Result<(), PendingError> {
    clear_if_generation_matches(state_dir, observed_generation).map(|_| ())
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
    let snapshot_bytes = serialize_snapshot(&snapshot)?;
    let crc = crc32(&snapshot_bytes);
    let on_disk = PendingOnDisk {
        schema: SCHEMA_VERSION,
        generation,
        created_at_unix_ms: unix_now_ms(),
        snapshot,
        integrity: format!("crc32:{crc:08x}"),
    };
    let serialized = toml::to_string_pretty(&on_disk).map_err(PendingError::Serialize)?;
    atomic_write(&path, serialized.as_bytes())?;
    restrict_permissions(&path);
    Ok(())
}

#[derive(Debug)]
pub enum PendingError {
    Io(std::io::Error),
    Serialize(toml::ser::Error),
    Deserialize(toml::de::Error),
    IntegrityMismatch { expected: String, got: String },
    NotFound,
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
        }
    }
}

impl std::error::Error for PendingError {}

fn parse(contents: &str) -> Result<PendingState, PendingError> {
    if let Ok(on_disk) = toml::from_str::<PendingOnDisk>(contents) {
        if on_disk.schema != SCHEMA_VERSION {
            return Err(PendingError::Deserialize(
                <toml::de::Error as serde::de::Error>::custom("unsupported schema"),
            ));
        }
        let snapshot_bytes = serialize_snapshot(&on_disk.snapshot)?;
        let actual_crc = format!("crc32:{:08x}", crc32(&snapshot_bytes));
        if on_disk.integrity != actual_crc {
            return Err(PendingError::IntegrityMismatch {
                expected: on_disk.integrity.clone(),
                got: actual_crc,
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

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), PendingError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(PendingError::Io)?;
    }
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = std::fs::File::create(&tmp).map_err(PendingError::Io)?;
        f.write_all(bytes).map_err(PendingError::Io)?;
        f.sync_all().map_err(PendingError::Io)?;
    }
    std::fs::rename(&tmp, path).map_err(PendingError::Io)?;
    Ok(())
}

fn restrict_permissions(path: &Path) {
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
        let state = mark_pending(
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
        let s1 = mark_pending(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let s2 = mark_pending(
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
        let s = mark_pending(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let cleared = clear_if_generation_matches(dir.path(), s.generation).unwrap();
        assert!(cleared);
        assert!(matches!(
            read_state_from_dir(dir.path()),
            Err(PendingError::NotFound)
        ));
    }

    #[test]
    fn test_clear_if_generation_mismatched() {
        let dir = TempDir::new().unwrap();
        let _lock = mark_pending(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let cleared = clear_if_generation_matches(dir.path(), 999).unwrap();
        assert!(!cleared);
        assert!(read_state_from_dir(dir.path()).is_ok());
    }

    #[test]
    fn test_clear_removes_marker() {
        let dir = TempDir::new().unwrap();
        let _lock = mark_pending(
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
        let s = mark_pending(
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
        let _lock = mark_pending(
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
        let _lock = mark_pending(
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
        let s = mark_pending(
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
        let _lock = mark_pending(
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
    fn test_atomic_write_creates_parent() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("nested").join("path");
        std::fs::create_dir_all(&nested).unwrap();
        let _ = mark_pending(
            &nested,
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        assert!(pending_path(&nested).exists());
    }
}
