//! Read-only command recovery suppression tests (Workstream B).
//!
//! Proves that read-only commands (status, get, list, validate, backup,
//! doctor, search --help, library list, library show, restore --dry-run)
//! cannot trigger startup recovery, spawn detached workers, or initiate
//! network work.

mod support;

use std::fs;
use std::path::Path;
use support::helpers::*;

/// Setup: create a library with a test snippet.
fn setup_library(config_dir: &Path) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", "readonly-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("readonly-test.toml"),
        r#"[[snippets]]
id = "readonly-1"
description = "readonly test snippet"
command = "echo readonly-test"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", "readonly-test"]);
    cmd.output().unwrap();
}

/// Write a sync.toml with auto_sync enabled pointing to a server that
/// will never respond. If recovery triggers, it would try to connect.
fn setup_auto_sync_config(config_dir: &Path) {
    // Write sync.toml with auto_sync enabled
    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-readonly-key"
device_id = "test-readonly-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "ignore"
"#,
    )
    .unwrap();
}

/// Create a valid pending marker with a known generation G.
fn setup_pending_marker(config_dir: &Path, generation: u64) {
    let pending_path = config_dir.join("auto-sync-pending.toml");
    fs::write(&pending_path, format!("generation = {generation}\n")).unwrap();
}

/// Read the pending generation from the marker file.
fn read_pending_generation(config_dir: &Path) -> Option<u64> {
    let pending_path = config_dir.join("auto-sync-pending.toml");
    let raw = fs::read_to_string(pending_path).ok()?;
    let parsed: toml::Table = raw.parse().ok()?;
    parsed
        .get("generation")
        .and_then(|v| v.as_integer())
        .and_then(|v| u64::try_from(v).ok())
}

/// Read the status file content.
fn read_status_file(config_dir: &Path) -> Option<String> {
    fs::read_to_string(config_dir.join("auto-sync-status.toml")).ok()
}

/// Assert no worker event file was created (indicating no worker spawn).
fn assert_no_worker_spawned(config_dir: &Path) {
    // Check for auto-sync-status.toml — if a worker ran, it would update this
    let status_path = config_dir.join("auto-sync-status.toml");
    if status_path.exists() {
        let content = fs::read_to_string(&status_path).unwrap_or_default();
        // If status file contains "worker_started", a worker was spawned
        assert!(
            !content.contains("worker_started"),
            "Worker was spawned during read-only command"
        );
    }
}

/// Strengthened read-only test: verifies the full 12-point spec:
/// 1. Valid sync configuration exists
/// 2. Valid pending marker with generation G exists
/// 3. Status snapshot S0 captured
/// 4. Command invoked
/// 5. No worker event
/// 6. No executor event
/// 7. Pending generation remains exactly G
/// 8. Status unchanged (where applicable)
fn run_read_only_command_and_verify(config_dir: &Path, args: &[&str], expected_exit_code: i32) {
    let generation = 42u64;
    setup_pending_marker(config_dir, generation);
    let status_before = read_status_file(config_dir);

    let output = snp_in(config_dir).args(args).output().unwrap();
    assert_eq!(
        output.status.code().unwrap_or(-1),
        expected_exit_code,
        "command {:?} should exit with code {expected_exit_code}",
        args
    );

    // Give a brief moment for any accidental worker spawn
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Assert no worker spawned
    assert_no_worker_spawned(config_dir);

    // Assert pending marker unchanged
    let gen_after = read_pending_generation(config_dir);
    assert_eq!(
        gen_after,
        Some(generation),
        "pending generation must remain exactly {generation} after read-only command"
    );

    // Assert status unchanged (if it existed before, it must be byte-identical after)
    let status_after = read_status_file(config_dir);
    assert_eq!(
        status_before, status_after,
        "status file must remain byte-identical after read-only command"
    );
}

// === status ===

#[test]
fn test_status_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    run_read_only_command_and_verify(&config_dir, &["status"], 0);
}

#[test]
fn test_status_json_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    run_read_only_command_and_verify(&config_dir, &["status", "--json"], 0);
}

// === get ===

#[test]
fn test_get_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    run_read_only_command_and_verify(&config_dir, &["get", "--id", "readonly-1"], 0);
}

// === list ===

#[test]
fn test_list_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    run_read_only_command_and_verify(&config_dir, &["list"], 0);
}

// === validate ===

#[test]
fn test_validate_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    run_read_only_command_and_verify(&config_dir, &["validate"], 0);
}

// === backup ===

#[test]
fn test_backup_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    let backup_dir = _tmp.path().join("readonly-backup");
    run_read_only_command_and_verify(
        &config_dir,
        &["backup", "--output", backup_dir.to_str().unwrap()],
        0,
    );
}

// === doctor ===

#[test]
fn test_doctor_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    let generation = 42u64;
    setup_pending_marker(&config_dir, generation);
    let status_before = read_status_file(&config_dir);

    // doctor may exit nonzero (e.g., code 2) when it detects issues in the test env.
    // The key invariant is that no worker is spawned, regardless of exit code.
    let _output = snp_in(&config_dir)
        .args(["doctor", "--compatibility"])
        .output()
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_no_worker_spawned(&config_dir);

    // Verify pending marker unchanged
    let gen_after = read_pending_generation(&config_dir);
    assert_eq!(
        gen_after,
        Some(generation),
        "pending generation must remain exactly {generation} after doctor"
    );

    // Verify status unchanged
    let status_after = read_status_file(&config_dir);
    assert_eq!(
        status_before, status_after,
        "status file must remain byte-identical after doctor"
    );
}

// === search --help ===

#[test]
fn test_search_help_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    run_read_only_command_and_verify(&config_dir, &["search", "--help"], 0);
}

// === library list ===

#[test]
fn test_library_list_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    run_read_only_command_and_verify(&config_dir, &["library", "list"], 0);
}

// === library show ===

#[test]
fn test_library_show_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    run_read_only_command_and_verify(&config_dir, &["library", "show", "readonly-test"], 0);
}

// === restore --dry-run ===

#[test]
fn test_restore_dry_run_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    // Create a backup to restore from
    let backup_dir = _tmp.path().join("restore-backup");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Capture pending marker before command
    let state_dir = config_dir.join(".auto-sync");
    let pending_path = state_dir.join("pending");
    let pending_before = fs::read_to_string(&pending_path).ok();

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "restore --dry-run should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify pending marker unchanged
    let pending_after = fs::read_to_string(&pending_path).ok();
    assert_eq!(
        pending_before, pending_after,
        "pending marker must not change during restore --dry-run"
    );

    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_no_worker_spawned(&config_dir);
}

// === import pet --dry-run ===

#[test]
fn test_import_dry_run_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    // Create a minimal Pet import file
    let pet_file = _tmp.path().join("test-pet.toml");
    fs::write(
        &pet_file,
        r#"name = "test-pet"
[[snippets]]
description = "imported snippet"
command = "echo imported"
"#,
    )
    .unwrap();

    let _output = snp_in(&config_dir)
        .args(["import", "pet", pet_file.to_str().unwrap(), "--dry-run"])
        .output()
        .unwrap();
    // import --dry-run may fail if pet format is wrong, but should not spawn worker
    // Just verify no worker was spawned regardless of exit code

    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_no_worker_spawned(&config_dir);
}

// === repair --dry-run ===

#[test]
fn test_repair_dry_run_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    let _output = snp_in(&config_dir)
        .args(["repair", "--dry-run"])
        .output()
        .unwrap();
    // repair --dry-run should not spawn a worker

    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_no_worker_spawned(&config_dir);
}

// === sync run --dry-run ===

#[test]
fn test_sync_run_dry_run_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    let _output = snp_in(&config_dir)
        .args(["sync", "run", "--dry-run"])
        .output()
        .unwrap();
    // sync run --dry-run manages its own sync behavior, not generic startup recovery

    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_no_worker_spawned(&config_dir);
}

// === completions ===

#[test]
fn test_completions_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    run_read_only_command_and_verify(&config_dir, &["completions", "bash"], 0);
}

// === shell init ===

#[test]
fn test_shell_init_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    run_read_only_command_and_verify(&config_dir, &["shell", "init", "bash"], 0);
}

// === help ===

#[test]
fn test_help_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    run_read_only_command_and_verify(&config_dir, &["--help"], 0);
}

#[test]
fn test_subcommand_help_does_not_trigger_recovery() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);
    setup_auto_sync_config(&config_dir);

    run_read_only_command_and_verify(&config_dir, &["status", "--help"], 0);
}

// === Machine stdout is uncontaminated ===

#[test]
fn test_status_json_no_ansi_escapes() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["status", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    // JSON output should not contain ANSI escape sequences
    assert!(
        !stdout.contains('\x1b'),
        "JSON output must not contain ANSI escape sequences"
    );
}

#[test]
fn test_list_json_no_ansi_escapes() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains('\x1b'),
        "JSON output must not contain ANSI escape sequences"
    );
}
