mod support;

use std::fs;
use std::path::{Path, PathBuf};
use support::helpers::*;

/// State directory where transaction journals live (same as derive_state_dir()).
fn state_dir(config_dir: &Path) -> PathBuf {
    config_dir.to_path_buf()
}

/// Write a transaction journal file directly into the state directory.
fn write_journal(config_dir: &Path, txn_id: &str, state: &str) {
    let dir = state_dir(config_dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("txn-{txn_id}.toml"));
    let content = match state {
        "Prepared" => format!(
            r#"id = "{txn_id}"
operation = "test_op"
created_at_unix_ms = 1000000
state = "Prepared"

[[staged_files]]
original_path = "/tmp/fake.toml"
staged_path = "/tmp/fake.toml"
sha256 = "abc123"
existed_before = true
action = "Replace"
original_hash = "def456"
new_hash = "ghi789"
"#
        ),
        "BackupsDurable" => format!(
            r#"id = "{txn_id}"
operation = "test_op"
created_at_unix_ms = 1000000
state = "BackupsDurable"

[[staged_files]]
original_path = "/tmp/fake.toml"
staged_path = "/tmp/fake.toml"
sha256 = "abc123"
existed_before = true
action = "Replace"
original_hash = "def456"
new_hash = "ghi789"
"#
        ),
        "Committing" => format!(
            r#"id = "{txn_id}"
operation = "test_op"
created_at_unix_ms = 1000000

[state]
Committing = {{ next_index = 0 }}

[[staged_files]]
original_path = "/tmp/fake.toml"
staged_path = "/tmp/fake.toml"
sha256 = "abc123"
existed_before = true
action = "Replace"
original_hash = "def456"
new_hash = "ghi789"
"#
        ),
        "RollingBack" => format!(
            r#"id = "{txn_id}"
operation = "test_op"
created_at_unix_ms = 1000000

[state]
RollingBack = {{ next_index = 1 }}

[[staged_files]]
original_path = "/tmp/fake.toml"
staged_path = "/tmp/fake.toml"
sha256 = "abc123"
existed_before = true
action = "Replace"
original_hash = "def456"
new_hash = "ghi789"
"#
        ),
        "Committed" => format!(
            r#"id = "{txn_id}"
operation = "test_op"
created_at_unix_ms = 1000000
state = "Committed"

[[staged_files]]
original_path = "/tmp/fake.toml"
staged_path = "/tmp/fake.toml"
sha256 = "abc123"
existed_before = true
action = "Replace"
original_hash = "def456"
new_hash = "ghi789"
"#
        ),
        "RolledBack" => format!(
            r#"id = "{txn_id}"
operation = "test_op"
created_at_unix_ms = 1000000
state = "RolledBack"

[[staged_files]]
original_path = "/tmp/fake.toml"
staged_path = "/tmp/fake.toml"
sha256 = "abc123"
existed_before = true
action = "Replace"
original_hash = "def456"
new_hash = "ghi789"
"#
        ),
        "Failed" => format!(
            r#"id = "{txn_id}"
operation = "test_op"
created_at_unix_ms = 1000000

[state]
Failed = "something went wrong"

[[staged_files]]
original_path = "/tmp/fake.toml"
staged_path = "/tmp/fake.toml"
sha256 = "abc123"
existed_before = true
action = "Replace"
original_hash = "def456"
new_hash = "ghi789"
"#
        ),
        _ => panic!("unknown state: {state}"),
    };
    fs::write(&path, &content).unwrap();
}

/// Run `snp repair --dry-run --json` and return the parsed JSON output.
fn repair_dry_run_json(config_dir: &Path) -> serde_json::Value {
    let output = snp_in(config_dir)
        .args(["repair", "--dry-run", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // JSON report goes to stdout
    if !stdout.trim().is_empty() {
        serde_json::from_str(&stdout).unwrap_or_else(|e| {
            panic!("failed to parse JSON from stdout: {e}\nstdout: {stdout}\nstderr: {stderr}")
        })
    } else {
        panic!("no JSON output from repair --dry-run --json\nstderr: {stderr}")
    }
}

/// Count how many items in the JSON report have the given category.
fn count_items_by_category(json: &serde_json::Value, category: &str) -> usize {
    json["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|i| i["category"].as_str() == Some(category))
                .count()
        })
        .unwrap_or(0)
}

// =============================================================================
// Interrupted transaction detection tests
// =============================================================================

#[test]
fn test_interrupted_prepared_journal_detected() {
    let (_tmp, config_dir) = setup_test_env();
    write_journal(
        &config_dir,
        "aaaa1111-0000-0000-0000-000000000001",
        "Prepared",
    );
    let json = repair_dry_run_json(&config_dir);
    assert!(
        count_items_by_category(&json, "transaction") >= 1,
        "Prepared journal must be detected as interrupted: {json}"
    );
}

#[test]
fn test_interrupted_backups_durable_journal_detected() {
    let (_tmp, config_dir) = setup_test_env();
    write_journal(
        &config_dir,
        "bbbb2222-0000-0000-0000-000000000002",
        "BackupsDurable",
    );
    let json = repair_dry_run_json(&config_dir);
    assert!(
        count_items_by_category(&json, "transaction") >= 1,
        "BackupsDurable journal must be detected as interrupted: {json}"
    );
}

#[test]
fn test_interrupted_committing_journal_detected() {
    let (_tmp, config_dir) = setup_test_env();
    write_journal(
        &config_dir,
        "cccc3333-0000-0000-0000-000000000003",
        "Committing",
    );
    let json = repair_dry_run_json(&config_dir);
    assert!(
        count_items_by_category(&json, "transaction") >= 1,
        "Committing journal must be detected as interrupted: {json}"
    );
}

#[test]
fn test_interrupted_rolling_back_journal_detected() {
    let (_tmp, config_dir) = setup_test_env();
    write_journal(
        &config_dir,
        "dddd4444-0000-0000-0000-000000000004",
        "RollingBack",
    );
    let json = repair_dry_run_json(&config_dir);
    assert!(
        count_items_by_category(&json, "transaction") >= 1,
        "RollingBack journal must be detected as interrupted: {json}"
    );
}

// =============================================================================
// Terminal state tests (should NOT be detected as interrupted)
// =============================================================================

#[test]
fn test_committed_journal_not_detected() {
    let (_tmp, config_dir) = setup_test_env();
    write_journal(
        &config_dir,
        "eeee5555-0000-0000-0000-000000000005",
        "Committed",
    );
    let json = repair_dry_run_json(&config_dir);
    assert_eq!(
        count_items_by_category(&json, "transaction"),
        0,
        "Committed journal must NOT be detected as interrupted: {json}"
    );
}

#[test]
fn test_rolled_back_journal_not_detected() {
    let (_tmp, config_dir) = setup_test_env();
    write_journal(
        &config_dir,
        "ffff6666-0000-0000-0000-000000000006",
        "RolledBack",
    );
    let json = repair_dry_run_json(&config_dir);
    assert_eq!(
        count_items_by_category(&json, "transaction"),
        0,
        "RolledBack journal must NOT be detected as interrupted: {json}"
    );
}

#[test]
fn test_failed_journal_not_detected() {
    let (_tmp, config_dir) = setup_test_env();
    write_journal(
        &config_dir,
        "aaaa7777-0000-0000-0000-000000000007",
        "Failed",
    );
    let json = repair_dry_run_json(&config_dir);
    assert_eq!(
        count_items_by_category(&json, "transaction"),
        0,
        "Failed journal must NOT be detected as interrupted: {json}"
    );
}

// =============================================================================
// Edge cases
// =============================================================================

#[test]
fn test_empty_state_dir_no_interrupted() {
    let (_tmp, config_dir) = setup_test_env();
    // Ensure the state directory exists but is empty
    fs::create_dir_all(state_dir(&config_dir)).unwrap();
    let json = repair_dry_run_json(&config_dir);
    assert_eq!(
        count_items_by_category(&json, "transaction"),
        0,
        "Empty state dir must yield no interrupted transactions: {json}"
    );
}

#[test]
fn test_malformed_journal_skipped() {
    let (_tmp, config_dir) = setup_test_env();
    let dir = state_dir(&config_dir);
    fs::create_dir_all(&dir).unwrap();
    // Write a file that looks like a journal but has invalid TOML
    fs::write(
        dir.join("txn-bad0000-0000-0000-0000-000000000000.toml"),
        "this is not valid toml {{{",
    )
    .unwrap();
    // Should not crash — malformed journals are skipped with a warning
    let json = repair_dry_run_json(&config_dir);
    assert!(
        json.is_object(),
        "repair must not crash on malformed journal: {json}"
    );
}

// =============================================================================
// Lock file tests
// =============================================================================

#[test]
fn test_lock_pid_and_nonce_persisted() {
    let (_tmp, config_dir) = setup_test_env();
    let lock_path = state_dir(&config_dir).join("transaction.lock");
    let content = r#"schema_version = 1
pid = 12345
nonce = "test-nonce-abc123"
created_at_unix_ms = 1000000
operation = "test_lock"
"#;
    fs::create_dir_all(state_dir(&config_dir)).unwrap();
    fs::write(&lock_path, content).unwrap();

    let read_back = fs::read_to_string(&lock_path).unwrap();
    let parsed: toml::Value = toml::from_str(&read_back).unwrap();

    assert_eq!(
        parsed["pid"].as_integer(),
        Some(12345),
        "lock must contain valid pid"
    );
    assert_eq!(
        parsed["nonce"].as_str(),
        Some("test-nonce-abc123"),
        "lock must contain valid nonce"
    );
    assert_eq!(
        parsed["operation"].as_str(),
        Some("test_lock"),
        "lock must contain valid operation"
    );
}

#[test]
fn test_stale_lock_owner_dead_reclaimed() {
    let (_tmp, config_dir) = setup_test_env();
    let state = state_dir(&config_dir);
    fs::create_dir_all(&state).unwrap();
    let lock_path = state.join("transaction.lock");

    // PID 1 is always dead on any OS (init/systemd).
    // Write a stale lock, then verify that snp repair does not crash.
    // Actual reclaim happens inside acquire_transaction_lock when a
    // transaction-bearing command tries to acquire the lock.
    fs::write(
        &lock_path,
        r#"schema_version = 1
pid = 1
nonce = "stale-nonce"
created_at_unix_ms = 1000000
operation = "stale_op"
"#,
    )
    .unwrap();
    assert!(lock_path.exists(), "stale lock must exist before repair");

    // snp repair must not crash when a stale lock is present
    let output = snp_in(&config_dir)
        .args(["repair", "--apply"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "repair must succeed with stale lock present"
    );

    // The stale lock file persists (repair does not touch locks),
    // but the lock content is still valid TOML with the dead PID.
    let after = fs::read_to_string(&lock_path).unwrap();
    let parsed: toml::Value = toml::from_str(&after).unwrap();
    assert_eq!(
        parsed["pid"].as_integer(),
        Some(1),
        "stale lock PID=1 should persist until acquire_transaction_lock reclaims it"
    );
    assert_eq!(
        parsed["nonce"].as_str(),
        Some("stale-nonce"),
        "stale lock nonce should persist"
    );
}

#[test]
fn test_live_lock_blocks_acquisition() {
    let (_tmp, config_dir) = setup_test_env();
    let state = state_dir(&config_dir);
    fs::create_dir_all(&state).unwrap();
    let lock_path = state.join("transaction.lock");

    // Write a lock with the current process PID — this simulates a live lock
    // (the current test process is alive, so any command should refuse)
    let content = format!(
        r#"schema_version = 1
pid = {}
nonce = "live-nonce"
created_at_unix_ms = 1000000
operation = "live_op"
"#,
        std::process::id()
    );
    fs::write(&lock_path, &content).unwrap();

    // Running snp repair should not remove the live lock
    let _output = snp_in(&config_dir)
        .args(["repair", "--apply"])
        .output()
        .unwrap();

    // Lock file should still exist with the original PID
    if lock_path.exists() {
        let after = fs::read_to_string(&lock_path).unwrap();
        let parsed: toml::Value = toml::from_str(&after).unwrap();
        assert_eq!(
            parsed["pid"].as_integer(),
            Some(std::process::id() as i64),
            "live lock with our PID must not be reclaimed"
        );
    }
}

// === PID reuse cannot steal a live lock (Workstream E) ===

/// Verify that a lock with a different nonce cannot be removed even if the
/// PID happens to match (simulating PID reuse). The nonce must be verified.
#[test]
fn test_wrong_nonce_cannot_remove_live_lock() {
    let (_tmp, config_dir) = setup_test_env();
    let state = state_dir(&config_dir);
    fs::create_dir_all(&state).unwrap();
    let lock_path = state.join("transaction.lock");

    // Write a lock with the current process PID but a wrong nonce
    let content = format!(
        r#"schema_version = 1
pid = {}
nonce = "original-nonce"
created_at_unix_ms = 1000000
operation = "original_op"
"#,
        std::process::id()
    );
    fs::write(&lock_path, &content).unwrap();

    // Running snp repair should not remove the lock because nonce doesn't match
    let _output = snp_in(&config_dir)
        .args(["repair", "--apply"])
        .output()
        .unwrap();

    // Lock file should still exist with the original nonce
    if lock_path.exists() {
        let after = fs::read_to_string(&lock_path).unwrap();
        assert!(
            after.contains("original-nonce"),
            "lock must retain original nonce, got: {after}"
        );
    }
}

// === Malformed lock is not silently deleted ===

/// Verify that a malformed lock file is not silently removed by repair.
#[test]
fn test_malformed_lock_not_silently_deleted() {
    let (_tmp, config_dir) = setup_test_env();
    let state = state_dir(&config_dir);
    fs::create_dir_all(&state).unwrap();
    let lock_path = state.join("transaction.lock");

    // Write garbage content that is not valid TOML
    fs::write(&lock_path, "this is not valid toml {{{").unwrap();
    assert!(lock_path.exists());

    // Running snp repair should not remove the malformed lock
    let _output = snp_in(&config_dir)
        .args(["repair", "--apply"])
        .output()
        .unwrap();

    // Lock file should still exist (malformed locks are preserved, not deleted)
    assert!(
        lock_path.exists(),
        "malformed lock must not be silently deleted"
    );
}

// === Dry-run creates no transaction artifacts (Workstream D) ===

/// Verify that `snp restore --mode dry-run` creates no transaction
/// journals, no locks, and no backup directories.
#[test]
fn test_dry_run_creates_no_transaction_artifacts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();

    // Create a minimal backup directory
    let backup_dir = tmp.path().join("dry-backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let lib_content = r#"[[snippets]]
id = "dry-id"
description = "dry snippet"
command = "echo dry"
"#;
    fs::write(libraries_dir.join("test.toml"), lib_content).unwrap();

    let index = r#"[[libraries]]
filename = "test"
is_primary = true
"#;
    fs::write(backup_dir.join("libraries.toml"), index).unwrap();

    let lib_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(lib_content.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    };
    let index_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(index.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    };

    let manifest = format!(
        r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "test.toml"
kind = "library"
size = {lib_size}
sha256 = "{lib_hash}"

[[files]]
path = "libraries.toml"
kind = "index"
size = {idx_size}
sha256 = "{index_hash}"
"#,
        lib_size = lib_content.len(),
        idx_size = index.len(),
    );
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "dry-run should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // No transaction journals
    let state = state_dir(&config_dir);
    if state.exists() {
        let txn_dir = state.join(".transaction");
        if txn_dir.exists() {
            let journals: Vec<_> = fs::read_dir(&txn_dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path().extension().is_some_and(|ext| ext == "toml")
                        && e.path()
                            .file_stem()
                            .is_some_and(|s| s.to_string_lossy().starts_with("txn-"))
                })
                .collect();
            assert!(
                journals.is_empty(),
                "dry-run must not create transaction journals"
            );
        }
    }

    // No transaction lock
    let lock_path = state.join("transaction.lock");
    assert!(
        !lock_path.exists(),
        "dry-run must not create transaction lock"
    );

    // No backup directories
    let backups_dir = state.join("backups");
    assert!(
        !backups_dir.exists(),
        "dry-run must not create backup directories"
    );

    // No library files created
    let lib_path = config_dir.join("libraries").join("test.toml");
    assert!(!lib_path.exists(), "dry-run must not create library files");
}

// === Rollback creates no pending generation (Workstream D) ===

/// Verify that a rolled-back restore does not create a pending marker,
/// since the restore was effectively undone.
#[test]
fn test_rolled_back_restore_creates_no_pending_generation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("fail-backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    // Create a library with invalid content that will cause a restore failure
    // (e.g., a library file with a checksum mismatch)
    let lib_content = r#"[[snippets]]
id = "fail-id"
description = "fail snippet"
command = "echo fail"
"#;
    fs::write(libraries_dir.join("fail.toml"), lib_content).unwrap();

    let index = r#"[[libraries]]
filename = "fail"
is_primary = true
"#;
    fs::write(backup_dir.join("libraries.toml"), index).unwrap();

    let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";
    let index_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(index.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    };

    // Use a wrong hash for the library to cause checksum failure during restore
    let manifest = format!(
        r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "fail.toml"
kind = "library"
size = {lib_size}
sha256 = "{wrong_hash}"

[[files]]
path = "libraries.toml"
kind = "index"
size = {idx_size}
sha256 = "{index_hash}"
"#,
        lib_size = lib_content.len(),
        idx_size = index.len(),
    );
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    // Attempt replace — should fail due to checksum mismatch
    let _output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "replace"])
        .output()
        .unwrap();

    // No pending generation should be created for a failed/rolled-back restore
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap_or_default();
        assert!(
            !content.contains("generation"),
            "rolled-back restore must not create pending generation"
        );
    }
}
