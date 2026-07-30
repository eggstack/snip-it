//! Integration tests for `snp edit` byte-change-driven sync notification.
//!
//! Verifies the outcome matrix defined in the post-11L plan:
//!
//! | Editor status | File changed | Outcome                              |
//! |---            |---           |---                                   |
//! | success       | no           | success, no notification             |
//! | success       | yes          | success, exactly one notification    |
//! | failure       | no           | error, no notification               |
//! | failure       | yes          | error, exactly one notification      |

mod support;

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tempfile::TempDir;

use support::environment::TestEnvironment;
use support::helpers::*;

fn write_editor_script(dir: &std::path::Path, body: &str) -> PathBuf {
    let path = dir.join("editor.sh");
    fs::write(&path, body).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

fn run_edit_with_editor(
    config_dir: &std::path::Path,
    editor_path: &std::path::Path,
    library: &str,
) -> (bool, String, String) {
    let mut cmd = snp_in(config_dir);
    cmd.arg("edit");
    cmd.env("EDITOR", editor_path);
    cmd.env("SNP_SKIP_WORKER_SPAWN", "true");
    cmd.env("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    let out = cmd.output().unwrap();
    let _ = library;
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn count_pending_marker_events(pending_marker: &std::path::Path) -> usize {
    let Ok(content) = fs::read_to_string(pending_marker) else {
        return 0;
    };
    // Each notify_mutation call writes a marker with a generation. The
    // current generation reflects the latest mutation count for this
    // file. We only need to know whether at least one mutation was
    // recorded (presence of a marker means at least one).
    let has_marker = content.contains("generation = 1");
    if has_marker { 1 } else { 0 }
}

fn setup_env() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let config_dir = dir.path().join(".config").join("snp");
    fs::create_dir_all(&config_dir).unwrap();

    // Sync configured but no real server — auto-sync will retry and
    // fail in the background, but the pending marker is written
    // synchronously by notify_mutation.
    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:1"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_timeout_seconds = 5
auto_sync_failure = "ignore"
"#,
    )
    .unwrap();

    (dir, config_dir)
}

fn seed_library(config_dir: &std::path::Path, name: &str) -> PathBuf {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", name]);
    cmd.output().unwrap();
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", name]);
    cmd.output().unwrap();
    let mut cmd = snp_in(config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "seed",
        "--library",
        name,
    ]);
    let _ = output_with_stdin(cmd, b"echo seed");
    // Remove any pending marker created by the seed operations so the
    // tests can assert that `snp edit` itself does or does not create
    // a marker.
    let pending = config_dir.join("auto-sync-pending.toml");
    let _ = fs::remove_file(&pending);
    config_dir.join("libraries").join(format!("{name}.toml"))
}

/// Editor exits 0 without writing. No notification should fire.
#[test]
fn test_edit_unchanged_success_no_notification() {
    let (_tmp, config_dir) = setup_env();
    let lib_path = seed_library(&config_dir, "edit-unchanged-success");
    let script_dir = TempDir::new().unwrap();
    let editor = write_editor_script(script_dir.path(), "#!/bin/sh\n# no-op editor\nexit 0\n");
    let (ok, stdout, _stderr) =
        run_edit_with_editor(&config_dir, &editor, "edit-unchanged-success");
    assert!(ok, "snp edit should succeed: stdout={stdout}");
    let pending = config_dir.join("auto-sync-pending.toml");
    assert!(
        !pending.exists(),
        "no pending marker should be created for an unchanged editor session"
    );
    let _ = lib_path;
}

/// Editor writes changes and exits 0. Exactly one notification should fire.
#[test]
fn test_edit_changed_success_notifies() {
    let (_tmp, config_dir) = setup_env();
    let _lib_path = seed_library(&config_dir, "edit-changed-success");
    let script_dir = TempDir::new().unwrap();
    let lib_path = config_dir
        .join("libraries")
        .join("edit-changed-success.toml");
    let editor_body = format!(
        "#!/bin/sh\necho '# edited' >> \"{lib}\"\nexit 0\n",
        lib = lib_path.display()
    );
    let editor = write_editor_script(script_dir.path(), &editor_body);
    let (ok, _stdout, _stderr) = run_edit_with_editor(&config_dir, &editor, "edit-changed-success");
    assert!(ok);
    let pending = config_dir.join("auto-sync-pending.toml");
    assert!(
        pending.exists(),
        "pending marker should exist after a changed editor session"
    );
    let events = count_pending_marker_events(&pending);
    assert_eq!(events, 1, "exactly one mutation notification expected");
}

/// Editor exits nonzero without writing. No notification, error reported.
#[test]
fn test_edit_unchanged_failure_no_notification() {
    let (_tmp, config_dir) = setup_env();
    let _lib_path = seed_library(&config_dir, "edit-unchanged-failure");
    let script_dir = TempDir::new().unwrap();
    let editor = write_editor_script(
        script_dir.path(),
        "#!/bin/sh\n# no-op, exit nonzero\nexit 7\n",
    );
    let (ok, _stdout, stderr) =
        run_edit_with_editor(&config_dir, &editor, "edit-unchanged-failure");
    assert!(!ok, "snp edit should report editor failure");
    let pending = config_dir.join("auto-sync-pending.toml");
    assert!(
        !pending.exists(),
        "no pending marker should exist after a failed but unchanged editor session"
    );
    assert!(
        stderr.to_lowercase().contains("not modified")
            || stderr.to_lowercase().contains("not changed"),
        "error should describe unchanged state: {stderr}"
    );
}

/// Editor writes changes and exits nonzero. Exactly one notification
/// should fire, and the error message should describe the saved changes.
#[test]
fn test_edit_changed_failure_notifies_and_describes_changes() {
    let (_tmp, config_dir) = setup_env();
    let _lib_path = seed_library(&config_dir, "edit-changed-failure");
    let script_dir = TempDir::new().unwrap();
    let lib_path = config_dir
        .join("libraries")
        .join("edit-changed-failure.toml");
    let editor_body = format!(
        "#!/bin/sh\necho '# changed' >> \"{lib}\"\nexit 5\n",
        lib = lib_path.display()
    );
    let editor = write_editor_script(script_dir.path(), &editor_body);
    let (ok, _stdout, stderr) = run_edit_with_editor(&config_dir, &editor, "edit-changed-failure");
    assert!(!ok, "snp edit should report editor failure");
    let pending = config_dir.join("auto-sync-pending.toml");
    assert!(
        pending.exists(),
        "pending marker should exist even when editor exits nonzero, because bytes changed"
    );
    let events = count_pending_marker_events(&pending);
    assert_eq!(
        events, 1,
        "exactly one mutation notification expected even with editor failure"
    );
    assert!(
        stderr.to_lowercase().contains("saved changes")
            || stderr.to_lowercase().contains("modified"),
        "error message should describe the saved changes: {stderr}"
    );
}

/// Editor truncates the file to zero bytes then exits nonzero. Bytes
/// changed, so notification fires; error must still be returned.
#[test]
fn test_edit_truncate_then_fail_notifies() {
    let (_tmp, config_dir) = setup_env();
    let _lib_path = seed_library(&config_dir, "edit-truncate-failure");
    let script_dir = TempDir::new().unwrap();
    let lib_path = config_dir
        .join("libraries")
        .join("edit-truncate-failure.toml");
    let editor_body = format!(
        "#!/bin/sh\n: > \"{lib}\"\nexit 9\n",
        lib = lib_path.display()
    );
    let editor = write_editor_script(script_dir.path(), &editor_body);
    let (ok, _stdout, _stderr) =
        run_edit_with_editor(&config_dir, &editor, "edit-truncate-failure");
    assert!(!ok);
    let pending = config_dir.join("auto-sync-pending.toml");
    assert!(
        pending.exists(),
        "pending marker must exist when truncation changes bytes even with editor failure"
    );
}

/// TestEnvironment variant: end-to-end via the higher-level helper.
#[test]
fn test_edit_unchanged_success_noop_no_marker_via_env() {
    let env = TestEnvironment::builder().build().unwrap();
    env.create_library("env-edit");
    let lib_path = env.config_dir.join("libraries").join("env-edit.toml");
    fs::write(&lib_path, "[[Snippets]]\nid = \"a\"\ndescription = \"x\"\ncommand = \"echo a\"\noutput = \"\"\nfolders = []\nfavorite = false\ntags = []\ncreated_at = 1\nupdated_at = 1\ndevice_id = \"dev\"\n").unwrap();
    let _ = fs::remove_file(env.pending_marker_path());
    let script_dir = TempDir::new().unwrap();
    let editor = write_editor_script(script_dir.path(), "#!/bin/sh\nexit 0\n");
    let mut cmd = env.snp_cmd();
    cmd.arg("edit");
    cmd.env("EDITOR", &editor);
    cmd.env("SNP_SKIP_WORKER_SPAWN", "true");
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "snp edit must succeed");
    assert!(
        !env.has_pending_marker(),
        "no mutation notification expected for no-op editor"
    );
    // Force any stdio buffers to drain before cleanup.
    let _ = std::io::stdout().write_all(b"");
    let _ = lib_path;
}
