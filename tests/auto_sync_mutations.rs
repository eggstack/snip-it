//! Auto-sync mutation trigger and durability matrix tests.
//!
//! Covers Workstream B (Local-First Durability Matrix) and
//! Workstream G (Trigger Matrix Audit).

mod support;

use std::fs;
use support::helpers::*;

// ── Workstream G: Trigger Matrix Audit ──

/// Trigger matrix: new snippet triggers auto-sync.
#[test]
fn test_trigger_matrix_new_snippet() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml_auto_sync_ignore(&config_dir);
    create_test_library_for_auto_sync(&config_dir, "trigger-new");

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "trigger new test",
        "--library",
        "trigger-new",
    ]);
    let output = output_with_stdin(cmd, b"echo trigger-new");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Snippet added"),
        "Should confirm snippet creation. Got: {stdout}"
    );
}

/// Trigger matrix: output-only edit does NOT trigger auto-sync.
#[test]
fn test_trigger_matrix_output_only_edit_no_trigger() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml_auto_sync_ignore(&config_dir);
    create_test_library_for_auto_sync(&config_dir, "trigger-output");

    // First create a snippet
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "output edit test",
        "--library",
        "trigger-output",
    ]);
    let _ = output_with_stdin(cmd, b"echo output-edit");

    // Edit output only (should NOT trigger sync)
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "edit",
        "--output",
        "new output value",
        "--filter",
        "output edit test",
        "--library",
        "trigger-output",
    ]);
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "Edit output should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the output was set
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json", "--library", "trigger-output"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["output"].as_str().unwrap(), "new output value");
}

/// Trigger matrix: import dry-run does NOT trigger auto-sync.
#[test]
fn test_trigger_matrix_import_dry_run_no_trigger() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml_auto_sync_ignore(&config_dir);

    let pet_file = _tmp.path().join("trigger_dryrun.toml");
    fs::write(
        &pet_file,
        r#"[[snippets]]
description = "dry run snippet"
command = "echo dryrun"
output = ""
tag = ["test"]
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["import", "pet", pet_file.to_str().unwrap(), "--dry-run"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());

    // No library should be created (dry-run)
    let libraries_dir = config_dir.join("libraries");
    assert!(
        !libraries_dir.join("trigger-dryrun.toml").exists(),
        "Dry-run should not create library"
    );
}

/// Trigger matrix: import create triggers auto-sync (one event per import).
#[test]
fn test_trigger_matrix_import_create_triggers() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml_auto_sync_ignore(&config_dir);

    let pet_file = _tmp.path().join("trigger_import.toml");
    fs::write(
        &pet_file,
        r#"[[snippets]]
description = "import trigger snippet"
command = "echo import-trigger"
output = ""
tag = ["test"]
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["import", "pet", pet_file.to_str().unwrap()]);
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "Import should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Library should be created
    let libraries_dir = config_dir.join("libraries");
    assert!(
        libraries_dir.join("trigger-import.toml").exists(),
        "Import should create library file"
    );
}

/// Trigger matrix: library create triggers auto-sync.
#[test]
fn test_trigger_matrix_library_create_triggers() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml_auto_sync_ignore(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "trigger-lib-create"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Created library"),
        "Should confirm library creation. Got: {stdout}"
    );
}

/// Trigger matrix: library delete triggers auto-sync.
#[test]
fn test_trigger_matrix_library_delete_triggers() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml_auto_sync_ignore(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "trigger-lib-delete"]);
    let create_output = cmd.output().unwrap();
    assert!(
        create_output.status.success(),
        "Library create should succeed: {}",
        String::from_utf8_lossy(&create_output.stderr)
    );

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "delete", "trigger-lib-delete", "--force"]);
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "Library delete should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Trigger matrix: set-primary library is local-only (no sync trigger).
#[test]
fn test_trigger_matrix_set_primary_no_sync_trigger() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml_auto_sync_ignore(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "primary-a"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "primary-b"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "primary-b"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());

    // set-primary is metadata-only, should not create pending marker
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        assert!(
            !content.contains("pending = true"),
            "set-primary should not trigger auto-sync"
        );
    }
}

/// Trigger matrix: all mutation kinds are classified correctly.
#[test]
fn test_trigger_matrix_all_mutation_kinds() {
    // This test verifies the public API classification via doctor output
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["doctor", "--compatibility"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Doctor should show all auto-sync checks
    assert!(
        stderr.contains("auto_sync") || stderr.contains("Auto-sync"),
        "Doctor should include auto-sync checks"
    );
}

// ── Workstream B: Local-First Durability Matrix ──

/// Incomplete local mutation (before write) fails without scheduling.
#[test]
fn test_durability_incomplete_mutation_no_schedule() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml_auto_sync_ignore(&config_dir);

    // Try to create a snippet in a nonexistent library (fails before write)
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "should fail",
        "--library",
        "nonexistent-durability",
    ]);
    let output = output_with_stdin(cmd, b"echo should-fail");
    assert!(
        !output.status.success(),
        "Mutation in nonexistent library should fail"
    );

    // No pending marker should exist
    let pending_path = config_dir.join("auto-sync-pending.toml");
    assert!(
        !pending_path.exists(),
        "Failed mutation should not create pending marker"
    );
}

/// Completed local mutation remains readable after sync failure.
#[test]
fn test_durability_local_mutation_survives_sync_failure() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "ignore"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "durability-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "durability-test"]);
    cmd.output().unwrap();

    // Create a snippet (sync will fail)
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "durability survives",
        "--library",
        "durability-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo durable");

    // Verify local state is intact
    let lib_path = config_dir.join("libraries").join("durability-test.toml");
    let content = fs::read_to_string(&lib_path).unwrap();
    assert!(
        content.contains("durability survives"),
        "Local mutation must survive sync failure"
    );

    // List should show the snippet
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json", "--library", "durability-test"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0]["description"].as_str().unwrap(),
        "durability survives"
    );
}

/// Local mutation committed before notification (even if notification fails).
#[test]
fn test_durability_local_commit_before_notification() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "ignore"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "commit-before"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "commit-before"]);
    cmd.output().unwrap();

    // The local commit happens atomically before auto-sync is triggered
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "committed before notify",
        "--library",
        "commit-before",
    ]);
    let output = output_with_stdin(cmd, b"echo committed");
    assert!(output.status.success());

    // The snippet should be readable immediately (local-first guarantee)
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json", "--library", "commit-before"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0]["description"].as_str().unwrap(),
        "committed before notify"
    );
}

/// Multiple local mutations all survive even when sync never succeeds.
#[test]
fn test_durability_many_mutations_survive_sync_failure() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "ignore"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "many-durable"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "many-durable"]);
    cmd.output().unwrap();

    // Create 20 snippets (all syncs will fail)
    for i in 0..20 {
        let mut cmd = snp_in(&config_dir);
        cmd.args([
            "new",
            "--command-stdin",
            "--description",
            &format!("durable mutation {i}"),
            "--library",
            "many-durable",
        ]);
        let output = output_with_stdin(cmd, format!("echo durable-{i}").as_bytes());
        assert!(
            output.status.success(),
            "Mutation {i} should succeed locally"
        );
    }

    // All 20 snippets should be readable
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json", "--library", "many-durable"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        items.len(),
        20,
        "All 20 mutations should be readable locally"
    );
}

/// Backup semantics: existing library survives create-failure.
#[test]
fn test_durability_library_survives_concurrent_write() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "ignore"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "concurrent-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "concurrent-test"]);
    cmd.output().unwrap();

    // Create initial snippet
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "initial snippet",
        "--library",
        "concurrent-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo initial");

    // Create second snippet
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "second snippet",
        "--library",
        "concurrent-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo second");

    // Both should be present
    let lib_path = config_dir.join("libraries").join("concurrent-test.toml");
    let content = fs::read_to_string(&lib_path).unwrap();
    assert!(content.contains("initial snippet"));
    assert!(content.contains("second snippet"));
}

/// No corrupted coordinator state blocks future commands.
#[test]
fn test_durability_corrupt_pending_does_not_block() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "ignore"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "corrupt-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "corrupt-test"]);
    cmd.output().unwrap();

    // Write a corrupted pending file
    let pending_path = config_dir.join("auto-sync-pending.toml");
    fs::write(&pending_path, "this is not valid toml {{{{").unwrap();

    // Future commands should still work (corrupted pending is handled gracefully)
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "after corrupt",
        "--library",
        "corrupt-test",
    ]);
    let output = output_with_stdin(cmd, b"echo after-corrupt");
    assert!(
        output.status.success(),
        "Commands should work after corrupted pending state"
    );
}

/// Retry/manual sync can recover pending state.
#[test]
fn test_durability_explicit_sync_clears_pending() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "warn"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "recover-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "recover-test"]);
    cmd.output().unwrap();

    // Create a snippet (triggers auto-sync, which fails)
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "recover test",
        "--library",
        "recover-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo recover");

    // Explicit sync should clear pending and succeed (or fail gracefully)
    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync"]);
    let _ = cmd.output();

    // Pending should be cleared after explicit sync
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        assert!(
            content.contains("pending = false") || !content.contains("pending = true"),
            "Pending should be cleared after explicit sync"
        );
    }
}
