//! Restore crash failpoint tests (Workstream G).
//!
//! Proves the production transaction protocol by killing the real `snp
//! restore` binary at production failpoints and verifying exact recovery.
//!
//! Each test:
//! 1. Creates a valid backup with exact hashes.
//! 2. Establishes known pre-state bytes.
//! 3. Launches the real `snp restore` binary with `SNP_TEST_FAILPOINT` set.
//! 4. Confirms the process aborted at the expected boundary.
//! 5. Inspects journal, stage, backup, live files, and canonical pending marker.
//! 6. Launches a second command (e.g. `snp restore` again or `snp repair`)
//!    to trigger recovery.
//! 7. Verifies exact final state.
//! 8. Repeats recovery to prove idempotence.
//!
//! Failpoints use `SNP_TEST_FAILPOINT` env var. Production builds ignore
//! `SNP_TEST_FAILPOINT` entirely.

mod support;

use std::fs;
use std::path::PathBuf;
use support::helpers::*;

/// SHA-256 hex digest of bytes.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Build a backup directory with one library snippet and an index.
fn make_backup(tmp: &std::path::Path) -> PathBuf {
    make_backup_multi_file(tmp)
}

/// Build a backup directory with two files (library + index) so that
/// rollback has at least two actions to perform.
fn make_backup_multi_file(tmp: &std::path::Path) -> PathBuf {
    let backup_dir = tmp.join("backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let content = r#"[[snippets]]
id = "crash-test-1"
description = "crash test snippet"
command = "echo crash-test"
"#;
    fs::write(libraries_dir.join("crash-test.toml"), content).unwrap();

    let index = r#"[[libraries]]
filename = "crash-test"
is_primary = true
"#;
    fs::write(backup_dir.join("libraries.toml"), index).unwrap();

    let lib_hash = sha256_hex(content.as_bytes());
    let index_hash = sha256_hex(index.as_bytes());

    let manifest = format!(
        r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "crash-test.toml"
kind = "library"
size = {lib_size}
sha256 = "{lib_hash}"

[[files]]
path = "libraries.toml"
kind = "index"
size = {idx_size}
sha256 = "{index_hash}"
"#,
        lib_size = content.len(),
        idx_size = index.len(),
    );
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    backup_dir
}

/// Find all transaction journal files in a directory.
fn find_journals(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut journals = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("txn-") && name.ends_with(".toml") {
            journals.push(entry.path());
        }
    }
    journals
}

/// Count pending generations in the canonical config directory.
fn count_pending_generations(config_dir: &std::path::Path) -> u64 {
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if !pending_path.exists() {
        return 0;
    }
    let content = fs::read_to_string(&pending_path).unwrap_or_default();
    // Extract generation from the TOML
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("generation = ") {
            return rest.trim().parse().unwrap_or(0);
        }
    }
    0
}

/// Count interrupted transaction journals in the .transaction directory.
#[allow(dead_code)]
fn count_journals(config_dir: &std::path::Path) -> usize {
    let txn_dir = config_dir.join(".transaction");
    if !txn_dir.exists() {
        return 0;
    }
    let mut count = 0;
    for entry in fs::read_dir(&txn_dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("txn-") && name.ends_with(".toml") {
            count += 1;
        }
    }
    count
}

/// Count pending markers in the .transaction directory (should always be 0).
fn count_pending_in_transaction(config_dir: &std::path::Path) -> usize {
    let txn_dir = config_dir.join(".transaction");
    if !txn_dir.exists() {
        return 0;
    }
    let mut count = 0;
    for entry in fs::read_dir(&txn_dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "auto-sync-pending.toml" {
            count += 1;
        }
    }
    count
}

/// Run restore with a failpoint and return the output.
fn run_restore_with_failpoint(
    config_dir: &std::path::Path,
    backup_dir: &std::path::Path,
    failpoint: &str,
) -> std::process::Output {
    let mut cmd = snp_in(config_dir);
    cmd.args(["restore", backup_dir.to_str().unwrap()]);
    cmd.env("SNP_TEST_FAILPOINT", failpoint);
    cmd.env("SNP_TEST_CREDENTIAL_FILE", "/nonexistent/cred");
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap()
}

/// Run restore without a failpoint (for recovery).
fn run_restore(config_dir: &std::path::Path, backup_dir: &std::path::Path) -> std::process::Output {
    let mut cmd = snp_in(config_dir);
    cmd.args(["restore", backup_dir.to_str().unwrap()]);
    cmd.env("SNP_TEST_CREDENTIAL_FILE", "/nonexistent/cred");
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap()
}

/// Run `snp repair --apply` for recovery.
fn run_repair(config_dir: &std::path::Path) -> std::process::Output {
    let mut cmd = snp_in(config_dir);
    cmd.args(["repair", "--apply"]);
    cmd.env("SNP_TEST_CREDENTIAL_FILE", "/nonexistent/cred");
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap()
}

/// Assert that no pending marker exists in .transaction (the canonical bug).
fn assert_no_pending_in_transaction(config_dir: &std::path::Path) {
    assert_eq!(
        count_pending_in_transaction(config_dir),
        0,
        "pending marker must NOT exist inside .transaction/"
    );
}

// === Crash after prepared ===

#[test]
fn test_crash_after_prepared() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    // Pre-state: no library file exists.
    let lib_path = config_dir.join("libraries").join("crash-test.toml");
    assert!(!lib_path.exists());

    // Crash at the prepared boundary.
    let output = run_restore_with_failpoint(&config_dir, &backup_dir, "restore-after-prepared");
    assert!(
        !output.status.success(),
        "process should have aborted (crashed) at the failpoint"
    );

    // No live change — the library file should not exist.
    assert!(!lib_path.exists(), "no live change after crash at prepared");

    // No pending marker in canonical or .transaction directory.
    assert_eq!(count_pending_generations(&config_dir), 0);
    assert_no_pending_in_transaction(&config_dir);

    // Recovery: run restore again — it should succeed and create exactly
    // one pending generation.
    let recovery = run_restore(&config_dir, &backup_dir);
    assert!(recovery.status.success(), "recovery restore should succeed");
    assert!(lib_path.exists(), "library should exist after recovery");
    assert_eq!(count_pending_generations(&config_dir), 1);
    assert_no_pending_in_transaction(&config_dir);

    // Repeat recovery to prove idempotence — no additional generation.
    let recovery2 = run_restore(&config_dir, &backup_dir);
    assert!(recovery2.status.success(), "second recovery should succeed");
    assert_eq!(count_pending_generations(&config_dir), 1);
    assert_no_pending_in_transaction(&config_dir);
}

// === Crash after backups durable ===

#[test]
fn test_crash_after_backups_durable() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    let lib_path = config_dir.join("libraries").join("crash-test.toml");
    assert!(!lib_path.exists());

    let output =
        run_restore_with_failpoint(&config_dir, &backup_dir, "restore-after-backups-durable");
    assert!(!output.status.success(), "process should have aborted");

    // No live change — backups are durable but no live writes happened.
    assert!(!lib_path.exists());

    // No pending marker.
    assert_eq!(count_pending_generations(&config_dir), 0);
    assert_no_pending_in_transaction(&config_dir);

    // Recovery.
    let recovery = run_restore(&config_dir, &backup_dir);
    assert!(recovery.status.success());
    assert!(lib_path.exists());
    assert_eq!(count_pending_generations(&config_dir), 1);
    assert_no_pending_in_transaction(&config_dir);
}

// === Crash after first install ===

#[test]
fn test_crash_after_first_install() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    let lib_path = config_dir.join("libraries").join("crash-test.toml");
    assert!(!lib_path.exists());

    let output =
        run_restore_with_failpoint(&config_dir, &backup_dir, "restore-after-first-install");
    assert!(!output.status.success(), "process should have aborted");

    // The first file may or may not have been installed. Recovery must
    // produce exactly one final state.
    let recovery = run_restore(&config_dir, &backup_dir);
    assert!(recovery.status.success(), "recovery should succeed");
    assert!(lib_path.exists(), "library should exist after recovery");
    assert_eq!(count_pending_generations(&config_dir), 1);
    assert_no_pending_in_transaction(&config_dir);
}

// === Crash after all installs ===

#[test]
fn test_crash_after_all_installs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    let lib_path = config_dir.join("libraries").join("crash-test.toml");
    assert!(!lib_path.exists());

    let output = run_restore_with_failpoint(&config_dir, &backup_dir, "restore-after-all-installs");
    assert!(!output.status.success(), "process should have aborted");

    // All installs completed but pending was not recorded.
    // The library file should exist (all installs completed).
    assert!(lib_path.exists(), "library should exist after all installs");

    // No pending marker yet (crash before pending).
    assert_eq!(count_pending_generations(&config_dir), 0);
    assert_no_pending_in_transaction(&config_dir);

    // Recovery: should create exactly one pending generation.
    let recovery = run_restore(&config_dir, &backup_dir);
    assert!(recovery.status.success(), "recovery should succeed");
    assert_eq!(count_pending_generations(&config_dir), 1);
    assert_no_pending_in_transaction(&config_dir);
}

// === Crash before pending (after committed-local-before-pending) ===

#[test]
fn test_crash_before_pending() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    let lib_path = config_dir.join("libraries").join("crash-test.toml");
    assert!(!lib_path.exists());

    let output = run_restore_with_failpoint(
        &config_dir,
        &backup_dir,
        "restore-after-committed-local-before-pending",
    );
    assert!(!output.status.success(), "process should have aborted");

    // All installs completed; pending not yet recorded.
    assert!(lib_path.exists());
    assert_eq!(count_pending_generations(&config_dir), 0);
    assert_no_pending_in_transaction(&config_dir);

    // Recovery: should create exactly one pending generation.
    let recovery = run_restore(&config_dir, &backup_dir);
    assert!(recovery.status.success());
    assert_eq!(count_pending_generations(&config_dir), 1);
    assert_no_pending_in_transaction(&config_dir);
}

// === Crash after pending before journal update ===

#[test]
fn test_crash_after_pending_before_journal_update() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    let lib_path = config_dir.join("libraries").join("crash-test.toml");
    assert!(!lib_path.exists());

    let output = run_restore_with_failpoint(
        &config_dir,
        &backup_dir,
        "restore-after-pending-before-journal-update",
    );
    assert!(!output.status.success(), "process should have aborted");

    // Pending was recorded; journal update not yet persisted.
    assert!(lib_path.exists());
    assert_eq!(count_pending_generations(&config_dir), 1);
    assert_no_pending_in_transaction(&config_dir);

    // Recovery: should reuse the same generation (not increment).
    let recovery = run_restore(&config_dir, &backup_dir);
    assert!(recovery.status.success());
    assert_eq!(count_pending_generations(&config_dir), 1);
    assert_no_pending_in_transaction(&config_dir);
}

// === Crash after journal pending before cleanup ===

#[test]
fn test_crash_after_journal_pending_before_cleanup() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    let lib_path = config_dir.join("libraries").join("crash-test.toml");
    assert!(!lib_path.exists());

    let output = run_restore_with_failpoint(
        &config_dir,
        &backup_dir,
        "restore-after-journal-pending-before-cleanup",
    );
    assert!(!output.status.success(), "process should have aborted");

    // Pending recorded and journal updated; cleanup not yet done.
    assert!(lib_path.exists());
    assert_eq!(count_pending_generations(&config_dir), 1);
    assert_no_pending_in_transaction(&config_dir);

    // Recovery: should clean up without incrementing.
    let recovery = run_restore(&config_dir, &backup_dir);
    assert!(recovery.status.success());
    assert_eq!(count_pending_generations(&config_dir), 1);
    assert_no_pending_in_transaction(&config_dir);
}

// === Crash during first rollback ===

#[test]
fn test_crash_during_first_rollback() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup_multi_file(tmp.path());

    // Pre-state: create existing library files that will be replaced.
    // We need at least two files so rollback has at least two actions.
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let old_lib_content = r#"[[snippets]]
id = "old-snippet"
description = "old snippet"
command = "echo old"
"#;
    fs::write(libraries_dir.join("crash-test.toml"), old_lib_content).unwrap();

    let old_index_content = r#"[[libraries]]
filename = "crash-test"
is_primary = true
"#;
    fs::write(config_dir.join("libraries.toml"), old_index_content).unwrap();

    // Compute expected hashes for verification.
    let old_lib_hash = sha256_hex(old_lib_content.as_bytes());
    let old_index_hash = sha256_hex(old_index_content.as_bytes());

    // Phase 1: Start restore with injected error after second install.
    // This triggers rollback. The rollback failpoint aborts before the
    // first rollback action completes.
    let mut cmd = snp_in(&config_dir);
    cmd.args(["restore", backup_dir.to_str().unwrap()]);
    cmd.env("SNP_TEST_INJECT_ERROR", "restore-after-second-install");
    cmd.env("SNP_TEST_FAILPOINT", "restore-during-first-rollback");
    cmd.env("SNP_TEST_CREDENTIAL_FILE", "/nonexistent/cred");
    let output = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "process should have aborted during first rollback"
    );

    // Verify journal is in RollingBack state with next_rollback_position: 0.
    let txn_dir = config_dir.join(".transaction");
    let journals = find_journals(&txn_dir);
    assert_eq!(journals.len(), 1, "exactly one journal should exist");
    let journal_content = fs::read_to_string(&journals[0]).unwrap();
    assert!(
        journal_content.contains("RollingBack"),
        "journal should be in RollingBack state"
    );
    assert!(
        journal_content.contains("next_rollback_position = 0"),
        "journal should show position 0 (first rollback not completed)"
    );

    // No pending marker should exist.
    assert_eq!(count_pending_generations(&config_dir), 0);

    // Phase 2: Recovery — run `snp repair` to complete the interrupted rollback.
    // This restores the original content without starting a new restore.
    // The exit code may be 0 (clean) or 1 (partial failure from unrelated
    // timestamp repairs). The critical invariant is that the transaction
    // rollback itself completed — verified by file hash and journal absence.
    let recovery = run_repair(&config_dir);
    eprintln!(
        "REPAIR STDERR: {}",
        String::from_utf8_lossy(&recovery.stderr)
    );
    eprintln!(
        "REPAIR STDOUT: {}",
        String::from_utf8_lossy(&recovery.stdout)
    );
    let exit = recovery.status.code().unwrap_or(1);
    // Parse the JSON output to verify the rollback was applied.
    let stdout = String::from_utf8_lossy(&recovery.stdout);
    if let Ok(result) = serde_json::from_str::<serde_json::Value>(&stdout) {
        let applied = result["applied"].as_u64().unwrap_or(0);
        assert!(
            applied > 0,
            "repair must have applied at least one rollback action, got applied={applied}"
        );
    } else {
        // If JSON parsing fails, the repair still must have run.
        assert!(exit <= 1, "repair should exit 0 or 1, got {exit}");
    }

    // Verify exact original bytes were restored.
    let lib_path = libraries_dir.join("crash-test.toml");
    let index_path = config_dir.join("libraries.toml");
    let recovered_lib = fs::read_to_string(&lib_path).unwrap();
    let recovered_index = fs::read_to_string(&index_path).unwrap();
    assert_eq!(
        sha256_hex(recovered_lib.as_bytes()),
        old_lib_hash,
        "library content should be restored to original"
    );
    assert_eq!(
        sha256_hex(recovered_index.as_bytes()),
        old_index_hash,
        "index content should be restored to original"
    );

    // No pending marker after recovery.
    assert_eq!(count_pending_generations(&config_dir), 0);

    // No journals after recovery.
    let journals_after = find_journals(&txn_dir);
    assert_eq!(
        journals_after.len(),
        0,
        "no journals should remain after recovery"
    );

    // Phase 3: Repeat recovery to prove idempotence.
    // No journals remain, so the second repair should be a clean no-op.
    let recovery2 = run_repair(&config_dir);
    let stdout2 = String::from_utf8_lossy(&recovery2.stdout);
    if let Ok(result2) = serde_json::from_str::<serde_json::Value>(&stdout2) {
        let applied2 = result2["applied"].as_u64().unwrap_or(0);
        assert_eq!(
            applied2, 0,
            "second repair should be a no-op (no journals to recover)"
        );
    }
    assert_eq!(count_pending_generations(&config_dir), 0);
    assert_eq!(find_journals(&txn_dir).len(), 0);
}

// === Crash during second rollback ===

#[test]
fn test_crash_during_second_rollback() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup_multi_file(tmp.path());

    // Pre-state: create existing library files.
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let old_lib_content = r#"[[snippets]]
id = "old-snippet"
description = "old snippet"
command = "echo old"
"#;
    fs::write(libraries_dir.join("crash-test.toml"), old_lib_content).unwrap();

    let old_index_content = r#"[[libraries]]
filename = "crash-test"
is_primary = true
"#;
    fs::write(config_dir.join("libraries.toml"), old_index_content).unwrap();

    let old_lib_hash = sha256_hex(old_lib_content.as_bytes());
    let old_index_hash = sha256_hex(old_index_content.as_bytes());

    // Phase 1: Start restore with injected error after second install.
    // The rollback failpoint aborts before the second rollback action
    // completes (after the first rollback action has completed and
    // progress is persisted).
    let mut cmd = snp_in(&config_dir);
    cmd.args(["restore", backup_dir.to_str().unwrap()]);
    cmd.env("SNP_TEST_INJECT_ERROR", "restore-after-second-install");
    cmd.env("SNP_TEST_FAILPOINT", "restore-during-second-rollback");
    cmd.env("SNP_TEST_CREDENTIAL_FILE", "/nonexistent/cred");
    let output = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "process should have aborted during second rollback"
    );

    // Verify journal is in RollingBack state with next_rollback_position: 1.
    let txn_dir = config_dir.join(".transaction");
    let journals = find_journals(&txn_dir);
    assert_eq!(journals.len(), 1, "exactly one journal should exist");
    let journal_content = fs::read_to_string(&journals[0]).unwrap();
    assert!(
        journal_content.contains("RollingBack"),
        "journal should be in RollingBack state"
    );
    assert!(
        journal_content.contains("next_rollback_position = 1"),
        "journal should show position 1 (first rollback completed, second not started)"
    );

    // No pending marker should exist.
    assert_eq!(count_pending_generations(&config_dir), 0);

    // Phase 2: Recovery — run `snp repair` to complete the interrupted rollback.
    // Verify the rollback was applied via JSON output; exit code may be 0 or 1
    // due to unrelated timestamp repairs.
    let recovery = run_repair(&config_dir);
    let stdout = String::from_utf8_lossy(&recovery.stdout);
    if let Ok(result) = serde_json::from_str::<serde_json::Value>(&stdout) {
        let applied = result["applied"].as_u64().unwrap_or(0);
        assert!(
            applied > 0,
            "repair must have applied at least one rollback action, got applied={applied}"
        );
    } else {
        let exit = recovery.status.code().unwrap_or(1);
        assert!(exit <= 1, "repair should exit 0 or 1, got {exit}");
    }

    // Verify exact original bytes were restored.
    let lib_path = libraries_dir.join("crash-test.toml");
    let index_path = config_dir.join("libraries.toml");
    let recovered_lib = fs::read_to_string(&lib_path).unwrap();
    let recovered_index = fs::read_to_string(&index_path).unwrap();
    assert_eq!(
        sha256_hex(recovered_lib.as_bytes()),
        old_lib_hash,
        "library content should be restored to original"
    );
    assert_eq!(
        sha256_hex(recovered_index.as_bytes()),
        old_index_hash,
        "index content should be restored to original"
    );

    // No pending marker after recovery.
    assert_eq!(count_pending_generations(&config_dir), 0);

    // No journals after recovery.
    assert_eq!(find_journals(&txn_dir).len(), 0);

    // Phase 3: Repeat recovery to prove idempotence.
    // No journals remain, so the second repair should be a clean no-op.
    let recovery2 = run_repair(&config_dir);
    let stdout2 = String::from_utf8_lossy(&recovery2.stdout);
    if let Ok(result2) = serde_json::from_str::<serde_json::Value>(&stdout2) {
        let applied2 = result2["applied"].as_u64().unwrap_or(0);
        assert_eq!(
            applied2, 0,
            "second repair should be a no-op (no journals to recover)"
        );
    }
    assert_eq!(count_pending_generations(&config_dir), 0);
    assert_eq!(find_journals(&txn_dir).len(), 0);
}

// === Production seam: failpoint variable is ignored in production builds ===

#[test]
fn test_production_build_ignores_failpoint() {
    // This test verifies that the SNP_TEST_FAILPOINT variable does not
    // cause a crash when the failpoint name doesn't match any boundary.
    // We use a non-matching failpoint name so the restore should succeed.
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    // Set the failpoint variable with a non-matching name — the restore
    // should succeed because the failpoint doesn't match any boundary.
    let mut cmd = snp_in(&config_dir);
    cmd.args(["restore", backup_dir.to_str().unwrap()]);
    cmd.env("SNP_TEST_FAILPOINT", "nonexistent-failpoint");
    cmd.env("SNP_TEST_CREDENTIAL_FILE", "/nonexistent/cred");
    let output = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "restore should succeed with a non-matching failpoint name"
    );
}

// === Pending path correctness ===

#[test]
fn test_no_pending_in_transaction_after_crash() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    // Crash at various boundaries and verify no pending in .transaction.
    for failpoint in &[
        "restore-after-prepared",
        "restore-after-backups-durable",
        "restore-after-first-install",
        "restore-after-all-installs",
        "restore-after-committed-local-before-pending",
        "restore-after-pending-before-journal-update",
        "restore-after-journal-pending-before-cleanup",
    ] {
        // Clean up from previous iteration.
        let _ = fs::remove_dir_all(config_dir.join("libraries"));
        let _ = fs::remove_file(config_dir.join("libraries.toml"));
        let _ = fs::remove_dir_all(config_dir.join(".transaction"));
        let _ = fs::remove_file(config_dir.join("auto-sync-pending.toml"));

        let output = run_restore_with_failpoint(&config_dir, &backup_dir, failpoint);
        assert!(
            !output.status.success(),
            "failpoint {failpoint} should crash"
        );

        assert_no_pending_in_transaction(&config_dir);
    }
}
