//! **Layer: Domain/Core**
//!
//! Library persistence: load/save, format-safe filesystem operations,
//! deterministic ID normalization, and backup rotation.

use super::model::{Snippet, Snippets};
use crate::config::{cached_read_toml, invalidate_toml_cache};
use crate::error::{SnipError, SnipResult};
use crate::test_failpoints::mutation_barrier;
use crate::utils::toml_helpers::fix_invalid_toml_escapes;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn write_library_file(path: &Path, content: &str, temp_prefix: &str) -> SnipResult<()> {
    crate::utils::atomic::write_private_atomic(path, content, temp_prefix)?;
    invalidate_toml_cache(path);
    Ok(())
}

/// Normalizes snippet IDs deterministically using SHA-256.
///
/// - Existing unique non-empty IDs remain unchanged.
/// - Missing IDs receive a deterministic provisional ID: `legacy-<sha256hex>`.
/// - For duplicate explicit IDs, the first occurrence keeps the original and
///   later occurrences receive deterministic replacement IDs.
/// - Repeated loads of identical file content produce identical IDs.
/// - Every ID in the normalized collection is unique: an ID that is already
///   present in the file is never handed out again.
///
/// Note: the missing-ID and duplicate-ID branches intentionally share one
/// content-fingerprint occurrence counter, so identical-content snippets
/// consume occurrence indices from the same sequence regardless of which
/// branch processes them. This keeps IDs deterministic for fixed file order;
/// inserting or reordering identical-content snippets can shift provisional
/// IDs of unrelated missing-ID snippets (accepted churn after sync merges).
fn normalize_snippet_ids(snippets: &mut [Snippet]) {
    // Pass 1 — reserve every ID the file already carries. Reserving them up
    // front means a provisional ID can never be minted for a snippet whose
    // sibling already owns that ID, regardless of the order the two appear in
    // the file. Without this, a snippet that already holds
    // `legacy-<hash(content, 0)>` did not consume occurrence index 0, so a
    // later empty-ID sibling with the same content was handed the very same
    // ID — two snippets sharing one identity.
    let mut taken: HashSet<String> = HashSet::new();
    for snippet in snippets.iter() {
        if !snippet.id.is_empty() {
            taken.insert(snippet.id.clone());
        }
    }

    // Pass 2 — hand out IDs. `kept` tracks the IDs already claimed by a
    // snippet that kept its own value, so only the *second* occurrence of a
    // repeated explicit ID is replaced.
    let mut kept: HashSet<String> = HashSet::new();
    let mut content_fingerprints: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    for snippet in snippets.iter_mut() {
        if snippet.id.is_empty() {
            // Compute a deterministic fingerprint from stable user data.
            let key = snippet_content_key(snippet);
            let new_id = next_free_id(&key, &mut content_fingerprints, &taken, |idx| {
                deterministic_legacy_id(snippet, idx)
            });
            taken.insert(new_id.clone());
            snippet.id = new_id;
        } else if !kept.insert(snippet.id.clone()) {
            // Duplicate explicit ID — generate a deterministic replacement.
            let key = snippet_content_key(snippet);
            let original_id = snippet.id.clone();
            let new_id = next_free_id(&key, &mut content_fingerprints, &taken, |idx| {
                deterministic_duplicate_id(&original_id, snippet, idx)
            });
            taken.insert(new_id.clone());
            snippet.id = new_id;
        }
    }
}

/// Returns the first candidate ID that is not already in `taken`.
///
/// The candidate is derived from `key` and the occurrence index for that key.
/// The occurrence index advances on every attempt, so a candidate that is
/// already taken is skipped rather than reused.
fn next_free_id(
    key: &str,
    content_fingerprints: &mut std::collections::HashMap<String, usize>,
    taken: &HashSet<String>,
    mut candidate: impl FnMut(usize) -> String,
) -> String {
    loop {
        let occurrence = content_fingerprints.entry(key.to_string()).or_insert(0);
        let idx = *occurrence;
        *occurrence += 1;
        let id = candidate(idx);
        if !taken.contains(&id) {
            return id;
        }
    }
}

/// Builds a content-based key for a snippet (used for occurrence counting).
fn snippet_content_key(snippet: &Snippet) -> String {
    let mut key = String::new();
    key.push_str(&snippet.description);
    key.push('\0');
    key.push_str(&snippet.command);
    key.push('\0');
    for tag in &snippet.tags {
        key.push_str(tag);
        key.push('\0');
    }
    key.push_str(&snippet.output);
    key.push('\0');
    for folder in &snippet.folders {
        key.push_str(folder);
        key.push('\0');
    }
    key.push_str(if snippet.favorite { "1" } else { "0" });
    key.push('\0');
    key.push_str(&snippet.device_id);
    key
}

/// Generates a deterministic legacy ID for a snippet with a missing ID.
pub(crate) fn deterministic_legacy_id(snippet: &Snippet, occurrence: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"snip-it-legacy-id-v1\0");
    hasher.update(&snippet.description);
    hasher.update(b"\0");
    hasher.update(&snippet.command);
    hasher.update(b"\0");
    for tag in &snippet.tags {
        hasher.update(tag);
        hasher.update(b"\0");
    }
    hasher.update(&snippet.output);
    hasher.update(b"\0");
    for folder in &snippet.folders {
        hasher.update(folder);
        hasher.update(b"\0");
    }
    hasher.update([u8::from(snippet.favorite)]);
    hasher.update(b"\0");
    hasher.update(&snippet.device_id);
    hasher.update(b"\0");
    hasher.update(occurrence.to_le_bytes());
    let hash = hasher.finalize();
    let mut hex = String::with_capacity(hash.len() * 2);
    for b in hash {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    format!("legacy-{hex}")
}

/// Generates a deterministic replacement ID for a duplicate explicit ID.
fn deterministic_duplicate_id(original_id: &str, snippet: &Snippet, occurrence: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"snip-it-duplicate-id-v1\0");
    hasher.update(original_id);
    hasher.update(b"\0");
    hasher.update(&snippet.description);
    hasher.update(b"\0");
    hasher.update(&snippet.command);
    hasher.update(b"\0");
    for tag in &snippet.tags {
        hasher.update(tag);
        hasher.update(b"\0");
    }
    hasher.update(&snippet.output);
    hasher.update(b"\0");
    for folder in &snippet.folders {
        hasher.update(folder);
        hasher.update(b"\0");
    }
    hasher.update([u8::from(snippet.favorite)]);
    hasher.update(b"\0");
    hasher.update(&snippet.device_id);
    hasher.update(b"\0");
    hasher.update(occurrence.to_le_bytes());
    let hash = hasher.finalize();
    let mut hex = String::with_capacity(hash.len() * 2);
    for b in hash {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    format!("legacy-{hex}")
}

/// Loads a snippet library from a TOML file.
///
/// Returns an empty collection if the file doesn't exist or is empty.
/// Returns an error if the file exists but contains malformed TOML;
/// a best-effort backup is created before the error is returned.
/// Missing or duplicate snippet IDs are repaired deterministically.
pub fn load_library(path: &Path) -> SnipResult<Snippets> {
    if !path.exists() {
        return Ok(Snippets::default());
    }

    let content = cached_read_toml(path)?;
    if content.trim().is_empty() {
        return Ok(Snippets::default());
    }

    let fixed_content = fix_invalid_toml_escapes(&content);

    let snippets: Snippets = match toml::from_str(&fixed_content) {
        Ok(s) => s,
        Err(e) => {
            // Best-effort backup of corrupted file before returning the error.
            // The backup failure must not hide the original parse error.
            let backup_path = path.with_extension("toml.corrupt.bak");
            if let Err(backup_err) = fs::copy(path, &backup_path) {
                tracing::warn!(
                    file = %path.display(),
                    error = %e,
                    backup_error = %backup_err,
                    "Failed to parse TOML and could not create backup"
                );
            } else {
                tracing::warn!(
                    file = %path.display(),
                    backup = %backup_path.display(),
                    error = %e,
                    "Failed to parse TOML, backup saved"
                );
            }
            return Err(SnipError::toml_error(
                &format!("parse library file: {}", path.display()),
                e,
            ));
        }
    };

    let mut snippets = snippets;
    normalize_snippet_ids(&mut snippets.snippets);

    Ok(snippets)
}

/// Saves a snippet library to a TOML file using atomic write.
///
/// Creates a backup before saving and sorts snippets by `updated_at` descending.
///
/// The serialized output is written verbatim from `toml::to_string_pretty`.
/// snip-it does not post-process the TOML body because the serializer already
/// picks correct quoting and escapes for every character, including tabs,
/// trailing whitespace, and CRLF. The earlier `quote_strings_containing_backslashes`
/// post-processing pass silently corrupted those byte sequences (its regex
/// could not tell TOML triple-quoted multi-line strings from ordinary
/// double-quoted strings, and its single-quoted output preserved TOML escape
/// sequences like `\t` as literal two-character pairs). The helper remains
/// available for callers that hand-write TOML and need the same conversion.
pub fn save_library(path: &Path, snippets: &Snippets) -> SnipResult<()> {
    // Check for interrupted transactions before any mutation.
    // This prevents new writes from proceeding over an unresolved restore.
    // The gate needs both the canonical sync state directory (where the
    // pending marker lives) and the transaction directory (where journals
    // and locks live).
    let sync_state_dir = crate::config::derive_sync_state_dir();
    let transaction_dir = crate::local_data::transaction_dir();
    crate::transaction::gate_mutation_on_interrupted_transactions(
        &sync_state_dir,
        &transaction_dir,
    )?;

    // Acquire the local-data lock to serialize against backup snapshot capture.
    // This ensures backup sees either the complete before-state or complete
    // after-state, never a mixed state.
    let _local_lock = crate::local_data::acquire_local_data_lock(&transaction_dir)?;

    save_library_internal(path, snippets, &_local_lock)?;

    Ok(())
}

/// Borrowed, recency-sorted serialization view of [`Snippets`].
///
/// Lets saves sort by `updated_at` without cloning snippet payloads.
/// Serializes to the exact same TOML shape as `Snippets`.
#[derive(serde::Serialize)]
struct SortedSnippetsView<'a> {
    snippets: Vec<&'a Snippet>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    folders: &'a Vec<String>,
}

/// Internal library save that skips the mutation gate and lock acquisition.
///
/// This is valid only when the caller already holds the local-data lock
/// (via `guard`) and is operating within an active transaction or has
/// already passed the mutation gate. Restore uses this to avoid
/// self-recovery: once restore has persisted `Committing`, the global
/// gate would see the caller's own journal as interrupted and roll it
/// back while restore continues.
pub fn save_library_internal(
    path: &Path,
    snippets: &Snippets,
    _guard: &crate::local_data::LocalDataLock,
) -> SnipResult<()> {
    if let Err(e) = backup_library(path) {
        tracing::warn!(error = %e, "Failed to create backup before save");
    }

    // Serialize from a borrowed, recency-sorted view so the full snippet
    // payloads are not cloned on every save.
    let mut order: Vec<usize> = (0..snippets.snippets.len()).collect();
    if !snippets
        .snippets
        .windows(2)
        .all(|pair| pair[0].updated_at >= pair[1].updated_at)
    {
        order.sort_by_key(|&i| std::cmp::Reverse(snippets.snippets[i].updated_at));
    }
    let sorted_view = SortedSnippetsView {
        snippets: order.iter().map(|&i| &snippets.snippets[i]).collect(),
        folders: &snippets.folders,
    };

    let toml_str = toml::to_string_pretty(&sorted_view)
        .map_err(|e| SnipError::toml_error("serialize snippets", e))?;

    let temp_prefix = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("snippets");
    crate::utils::atomic::write_private_atomic(path, &toml_str, temp_prefix)?;

    mutation_barrier("snippet-save-after-write-before-cache-invalidate");

    invalidate_toml_cache(path);

    Ok(())
}

/// Creates a timestamped backup of a library file.
///
/// Stores backups in a `backups/` subdirectory, keeping at most 10 per library.
/// Returns `None` if the source file doesn't exist.
pub fn backup_library(path: &Path) -> SnipResult<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }

    let backup_dir = path
        .parent()
        .ok_or_else(|| {
            SnipError::runtime_error(
                "backup path has no parent",
                Some(&path.display().to_string()),
            )
        })?
        .join("backups");
    fs::create_dir_all(&backup_dir)
        .map_err(|e| SnipError::io_error("create backup directory", backup_dir.clone(), e))?;

    // Clean up old backups (keep at most 10 per library)
    cleanup_old_backups(&backup_dir, path)?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%f");
    let file_stem = path.file_stem().ok_or_else(|| {
        SnipError::runtime_error(
            "backup path has no file stem",
            Some(&path.display().to_string()),
        )
    })?;
    let backup_name = format!("{}.{}.toml.bak", file_stem.to_string_lossy(), timestamp);
    let backup_path = backup_dir.join(backup_name);

    fs::copy(path, &backup_path)
        .map_err(|e| SnipError::io_error("create backup", backup_path.clone(), e))?;

    // `fs::copy` writes the copy without fsyncing it, so a power loss right
    // after the save could leave the rollback point truncated. Flush the
    // backup before the atomic write replaces the original.
    sync_backup_file(&backup_path);

    Ok(Some(backup_path))
}

/// Flush a freshly copied backup to disk.
///
/// `fs::copy` does not fsync the destination. The flush is best-effort: a
/// backup that cannot be synced is still a usable rollback point, so the
/// failure is logged rather than propagated to the caller.
fn sync_backup_file(path: &Path) {
    match fs::OpenOptions::new().write(true).open(path) {
        Ok(file) => {
            if let Err(e) = file.sync_all() {
                tracing::warn!(
                    backup = %path.display(),
                    error = %e,
                    "Failed to fsync backup file"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                backup = %path.display(),
                error = %e,
                "Failed to open backup file for fsync"
            );
        }
    }
}

/// Returns `true` when `name` is a backup file that belongs to `file_stem`.
///
/// Backups are named `{file_stem}.{timestamp}.toml.bak`. Matching on the
/// plain `{file_stem}.` prefix would also accept another library's backups
/// whenever one library name is a prefix of another (`foo` vs `foo.bar`), so
/// the rotation would delete an unrelated library's rollback point. The
/// segment between the stem and the `.toml.bak` suffix must therefore be the
/// timestamp itself — digits and underscores, never another `.` separator.
pub(crate) fn is_backup_name_for_stem(name: &str, file_stem: &str) -> bool {
    let Some(rest) = name.strip_prefix(file_stem) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('.') else {
        return false;
    };
    let Some(timestamp) = rest.strip_suffix(".toml.bak") else {
        return false;
    };
    !timestamp.is_empty() && timestamp.chars().all(|c| c.is_ascii_digit() || c == '_')
}

fn cleanup_old_backups(backup_dir: &Path, original_path: &Path) -> SnipResult<()> {
    const MAX_BACKUPS_PER_LIBRARY: usize = 10;

    let file_stem = match original_path.file_stem() {
        Some(s) => s.to_string_lossy().to_string(),
        None => return Ok(()),
    };

    let mut backups: Vec<_> = fs::read_dir(backup_dir)
        .map_err(|e| SnipError::io_error("read backup directory", backup_dir.to_path_buf(), e))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            // Only this library's own backups count towards its rotation.
            // A plain prefix match would also capture
            // `<stem>.<other>.<ts>.toml.bak` backups that belong to a
            // different library whose name starts with this one
            // (`foo` vs `foo.bar`).
            is_backup_name_for_stem(&name, &file_stem)
        })
        .map(|entry| {
            (
                entry.path(),
                entry.file_name().to_string_lossy().into_owned(),
            )
        })
        .collect();

    backups.sort_by(|a, b| b.1.cmp(&a.1));

    if backups.len() > MAX_BACKUPS_PER_LIBRARY {
        for (path, _) in backups.into_iter().skip(MAX_BACKUPS_PER_LIBRARY) {
            if let Err(e) = fs::remove_file(&path) {
                tracing::warn!(
                    backup = %path.display(),
                    error = %e,
                    "Failed to remove old backup"
                );
            }
        }
    }

    Ok(())
}
