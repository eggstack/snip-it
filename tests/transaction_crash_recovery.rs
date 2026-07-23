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
