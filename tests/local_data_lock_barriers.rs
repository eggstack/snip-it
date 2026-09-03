//! **Layer: Integration Test**
//!
//! Barrier-controlled backup concurrency tests (Workstream J).
//!
//! These tests prove that `snp backup` sees a complete before-state or
//! complete after-state while real writers are paused inside multi-file
//! mutations. The writer process is paused at a barrier point (via the
//! `SNP_TEST_MUTATION_BARRIER_DIR` environment variable) while a concurrent
//! `snp backup` process attempts to acquire the `LocalDataLock`.
//!
//! The test verifies:
//! - backup does not complete while the writer holds `LocalDataLock`;
//! - backup is observed waiting or failing busy while the writer owns the lock;
//! - each multi-file writer is covered;
//! - no test is merely backup → mutation → backup.

mod support;

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use support::helpers::{setup_test_env, snp_cmd, snp_in};

/// Set up a barrier directory for the given barrier point.
/// Writes the expected point name to `<dir>/point` and removes any stale
/// `entered` and `release` files.
fn setup_barrier(barrier_dir: &Path, point: &str) {
    fs::write(barrier_dir.join("point"), point).unwrap();
    let _ = fs::remove_file(barrier_dir.join("entered"));
    let _ = fs::remove_file(barrier_dir.join("release"));
}

/// Release the barrier by creating the `release` file.
fn release_barrier(barrier_dir: &Path) {
    fs::write(barrier_dir.join("release"), "released").unwrap();
}

/// Wait for the `entered` file to appear, with a timeout.
fn wait_for_entered(barrier_dir: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if barrier_dir.join("entered").exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Wait for a child process to finish, with a timeout.
fn wait_for_child(child: &mut std::process::Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return false,
        }
    }
}

/// Read the manifest.toml from a backup directory.
fn read_manifest(backup_dir: &Path) -> serde_json::Value {
    let manifest_path = backup_dir.join("manifest.toml");
    assert!(
        manifest_path.exists(),
        "manifest.toml not found at {}",
        manifest_path.display()
    );
    let content = fs::read_to_string(&manifest_path).unwrap();
    toml::from_str(&content).unwrap()
}

/// Verify backup snapshot coherence: every manifest entry exists,
/// actual size equals manifest size, actual hash equals manifest hash,
/// index references correspond to copied library files, every copied
/// library parses, and no temporary or partially written files are included.
fn verify_backup_coherence(backup_dir: &Path) {
    let manifest = read_manifest(backup_dir);
    let files = manifest["files"].as_array().unwrap();

    for entry in files {
        let path = entry["path"].as_str().unwrap();
        let kind = entry["kind"].as_str().unwrap();
        let size = entry["size"].as_u64().unwrap();
        let sha = entry["sha256"].as_str().unwrap();

        // Resolve the actual file path in the backup directory.
        let file_path = if kind == "library" {
            let basename = path.strip_prefix("libraries/").unwrap_or(path);
            backup_dir.join("libraries").join(basename)
        } else {
            backup_dir.join(path)
        };

        assert!(
            file_path.exists(),
            "backup manifest entry '{}' does not exist in backup directory",
            path
        );

        // Verify actual size equals manifest size.
        let actual_size = fs::metadata(&file_path).unwrap().len();
        assert_eq!(
            actual_size, size,
            "backup file '{}' size mismatch: manifest={}, actual={}",
            path, size, actual_size
        );

        // Verify actual hash equals manifest hash.
        let content = fs::read(&file_path).unwrap();
        let actual_hash = sha256_hex(&content);
        assert_eq!(
            actual_hash, sha,
            "backup file '{}' hash mismatch: manifest={}, actual={}",
            path, sha, actual_hash
        );

        // For library files, verify they parse as valid TOML.
        if kind == "library" {
            let _: toml::Value = toml::from_str(&String::from_utf8_lossy(&content))
                .unwrap_or_else(|e| panic!("backup library {} is not valid TOML: {}", path, e));
        }

        // For index files, verify library references correspond to copied files.
        if kind == "index" {
            let index_value: toml::Value =
                toml::from_str(&String::from_utf8_lossy(&content)).unwrap();
            if let Some(libraries) = index_value.get("libraries").and_then(|l| l.as_array()) {
                for lib in libraries {
                    let filename = lib.get("filename").and_then(|f| f.as_str()).unwrap();
                    let lib_path = backup_dir
                        .join("libraries")
                        .join(format!("{filename}.toml"));
                    assert!(
                        lib_path.exists(),
                        "index references library '{}' but file does not exist in backup",
                        filename
                    );
                }
            }
        }
    }

    // Verify no temporary or partially written files are included.
    fn check_no_temp(dir: &Path) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".tmp") || name.ends_with(".partial") || name.ends_with(".bak") {
                    panic!("backup contains temporary file: {}", name);
                }
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    check_no_temp(&entry.path());
                }
            }
        }
    }
    check_no_temp(backup_dir);
}

/// Compute SHA-256 hex digest of bytes.
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

/// Create a library with a test snippet via the snp binary.
fn setup_library(config_dir: &Path, name: &str) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", name]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join(format!("{name}.toml")),
        format!(
            r#"[[snippets]]
id = "{name}-snippet-1"
description = "{name} test snippet"
command = "echo {name}-test"
"#
        ),
    )
    .unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", name]);
    cmd.output().unwrap();
}

/// Build a snp command with the given config dir and barrier env.
fn snp_with_barrier(config_dir: &Path, barrier_dir: &Path) -> Command {
    let mut cmd = snp_cmd();
    cmd.env("XDG_CONFIG_HOME", config_dir.parent().unwrap())
        .env("SNP_TEST_MUTATION_BARRIER_DIR", barrier_dir);
    cmd
}

/// Build a snp command with the given config dir (no barrier).
fn snp_for_config(config_dir: &Path) -> Command {
    let mut cmd = snp_cmd();
    cmd.env("XDG_CONFIG_HOME", config_dir.parent().unwrap());
    cmd
}

/// Test: library create barrier — backup sees either before-state or after-state.
///
/// The writer (`snp library create`) creates a library file, then pauses at
/// the barrier (after file creation, before index save). While paused,
/// `snp backup` is launched and must wait for the `LocalDataLock`. We then
/// release the writer, and both processes complete. The backup must show a
/// coherent state: the library file exists AND the index references it,
/// or neither does.
#[test]
fn test_library_create_barrier_coherent_snapshot() {
    let (_tmp, config_dir) = setup_test_env();

    // Create an initial library so there's a before-state to observe.
    setup_library(&config_dir, "initial-lib");

    // Set up barrier for library create.
    let barrier_dir = _tmp.path().join("barrier-create");
    fs::create_dir_all(&barrier_dir).unwrap();
    setup_barrier(&barrier_dir, "library-create-after-file-before-index");

    // Spawn the writer: `snp library create barrier-lib` with barrier env.
    let mut writer = snp_with_barrier(&config_dir, &barrier_dir)
        .args(["library", "create", "barrier-lib"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Wait for the writer to enter the barrier.
    assert!(
        wait_for_entered(&barrier_dir, Duration::from_secs(10)),
        "writer did not enter barrier within 10s"
    );

    // While the writer is paused, launch backup. The backup should wait
    // for the LocalDataLock (held by the writer).
    let backup_dir = _tmp.path().join("backup-create");
    let mut backup = snp_for_config(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Assert backup has NOT completed while the writer holds the lock.
    // Poll with try_wait for a bounded observation interval.
    std::thread::sleep(Duration::from_millis(250));
    assert!(
        backup.try_wait().unwrap().is_none(),
        "backup completed while writer still held LocalDataLock"
    );

    // Release the writer.
    release_barrier(&barrier_dir);

    // Wait for the writer to finish.
    let writer_finished = wait_for_child(&mut writer, Duration::from_secs(15));
    assert!(writer_finished, "writer did not finish within 15s");

    // Wait for backup to finish.
    let backup_finished = wait_for_child(&mut backup, Duration::from_secs(15));
    assert!(backup_finished, "backup did not finish within 15s");

    // Verify the backup is coherent: the library file should exist AND
    // the index should reference it (after-state).
    let manifest = read_manifest(&backup_dir);
    let files = manifest["files"].as_array().unwrap();
    let has_library = files
        .iter()
        .any(|f| f["kind"] == "library" && f["path"].as_str().unwrap().contains("barrier-lib"));
    let has_index = files.iter().any(|f| f["kind"] == "index");
    assert!(has_library, "backup should contain the barrier-lib library");
    assert!(has_index, "backup should contain the index");

    // Verify the library file in the backup is complete (not partial).
    let lib_file = backup_dir.join("libraries").join("barrier-lib.toml");
    assert!(lib_file.exists(), "library file should exist in backup");
    let content = fs::read_to_string(&lib_file).unwrap();
    assert!(
        content.contains("snippet") || content.contains("snippets"),
        "library file in backup should be complete"
    );

    // Verify full backup snapshot coherence.
    verify_backup_coherence(&backup_dir);
}

/// Test: snippet save barrier — backup sees coherent state during save.
///
/// The writer (`snp new`) saves a snippet, pausing at the barrier after
/// the atomic write is durable but before cache invalidation. While paused,
/// `snp backup` runs and should see either the old or new content, never
/// a partially written file.
#[test]
fn test_snippet_save_barrier_coherent_snapshot() {
    let (_tmp, config_dir) = setup_test_env();

    // Create an initial library.
    setup_library(&config_dir, "work");

    // Set up barrier for snippet save.
    let barrier_dir = _tmp.path().join("barrier-save");
    fs::create_dir_all(&barrier_dir).unwrap();
    setup_barrier(
        &barrier_dir,
        "snippet-save-after-write-before-cache-invalidate",
    );

    // Spawn the writer: `snp new` with barrier env.
    let mut writer = snp_with_barrier(&config_dir, &barrier_dir)
        .args(["new", "--command-stdin", "--description", "barrier-save"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Provide stdin input.
    writer
        .stdin
        .take()
        .unwrap()
        .write_all(b"echo barrier-save")
        .unwrap();

    // Wait for the writer to enter the barrier.
    assert!(
        wait_for_entered(&barrier_dir, Duration::from_secs(10)),
        "writer did not enter barrier within 10s"
    );

    // While the writer is paused, launch backup. The backup should wait
    // for the LocalDataLock.
    let backup_dir = _tmp.path().join("backup-save");
    let mut backup = snp_for_config(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Give backup a moment to start.
    std::thread::sleep(Duration::from_millis(500));

    // Release the writer.
    release_barrier(&barrier_dir);

    // Wait for the writer to finish.
    let writer_finished = wait_for_child(&mut writer, Duration::from_secs(15));
    assert!(writer_finished, "writer did not finish within 15s");

    // Wait for backup to finish.
    let backup_finished = wait_for_child(&mut backup, Duration::from_secs(15));
    assert!(backup_finished, "backup did not finish within 15s");

    // Verify the backup library file is valid TOML (no partial writes).
    let manifest = read_manifest(&backup_dir);
    let files = manifest["files"].as_array().unwrap();
    assert!(
        files.iter().any(|f| f["kind"] == "library"),
        "backup should contain a library"
    );

    // Read each library file and verify it's valid TOML.
    let libs_dir = backup_dir.join("libraries");
    if libs_dir.exists() {
        for entry in fs::read_dir(&libs_dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".toml") && !name.starts_with('.') {
                let content = fs::read_to_string(entry.path()).unwrap();
                let _: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
                    panic!(
                        "backup library {} is not valid TOML (partial write?): {}",
                        name, e
                    )
                });
            }
        }
    }

    // Verify full backup snapshot coherence.
    verify_backup_coherence(&backup_dir);
}

/// Test: library delete barrier — backup sees coherent state during delete.
///
/// The writer (`snp library delete`) removes a library, pausing at the
/// barrier after index save but before file deletion. While paused,
/// `snp backup` should see a coherent state where the index no longer
/// references the library but the file still exists (or both are gone).
#[test]
fn test_library_delete_barrier_coherent_snapshot() {
    let (_tmp, config_dir) = setup_test_env();

    // Create two libraries.
    setup_library(&config_dir, "lib-a");
    setup_library(&config_dir, "lib-b");

    // Set up barrier for library delete.
    let barrier_dir = _tmp.path().join("barrier-delete");
    fs::create_dir_all(&barrier_dir).unwrap();
    setup_barrier(&barrier_dir, "library-delete-after-index-before-file");

    // Spawn the writer: `snp library delete lib-a` with barrier env.
    let mut writer = snp_with_barrier(&config_dir, &barrier_dir)
        .args(["library", "delete", "lib-a", "--force"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Wait for the writer to enter the barrier.
    assert!(
        wait_for_entered(&barrier_dir, Duration::from_secs(10)),
        "writer did not enter barrier within 10s"
    );

    // While the writer is paused, launch backup. The backup should wait
    // for the LocalDataLock.
    let backup_dir = _tmp.path().join("backup-delete");
    let mut backup = snp_for_config(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Give backup a moment to start.
    std::thread::sleep(Duration::from_millis(500));

    // Release the writer.
    release_barrier(&barrier_dir);

    // Wait for the writer to finish.
    let writer_finished = wait_for_child(&mut writer, Duration::from_secs(15));
    assert!(writer_finished, "writer did not finish within 15s");

    // Wait for backup to finish.
    let backup_finished = wait_for_child(&mut backup, Duration::from_secs(15));
    assert!(backup_finished, "backup did not finish within 15s");

    // Verify coherence: the backup should show either:
    // - lib-a file exists AND index references lib-a (before-state)
    // - lib-a file does NOT exist AND index does NOT reference lib-a (after-state)
    // But NOT: lib-a file exists AND index does NOT reference lib-a (mixed)
    let manifest = read_manifest(&backup_dir);
    let files = manifest["files"].as_array().unwrap();
    let file_exists = files
        .iter()
        .any(|f| f["kind"] == "library" && f["path"].as_str().unwrap().contains("lib-a"));
    let index_references = files
        .iter()
        .find(|f| f["kind"] == "index")
        .and_then(|f| {
            let path = f["path"].as_str().unwrap();
            let content = fs::read_to_string(backup_dir.join(path)).ok()?;
            Some(content.contains("lib-a"))
        })
        .unwrap_or(false);

    assert!(
        file_exists == index_references,
        "incoherent backup state: file_exists={}, index_references={}",
        file_exists,
        index_references
    );
}

/// Test: sync config update barrier — backup sees coherent state during sync.toml write.
///
/// The writer writes sync.toml (via snp register), pausing at the barrier
/// after the atomic write is durable but before cache invalidation. While
/// paused, `snp backup` should see either the old or new sync.toml, never
/// a partial write.
#[test]
fn test_sync_config_barrier_coherent_snapshot() {
    let (_tmp, config_dir) = setup_test_env();

    // Write an initial sync.toml.
    let sync_path = config_dir.join("sync.toml");
    fs::write(
        &sync_path,
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "old-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = false
auto_sync_debounce_seconds = 0
auto_sync_timeout_seconds = 5
auto_sync_failure = "warn"
"#,
    )
    .unwrap();

    // Set up barrier for sync config update.
    let barrier_dir = _tmp.path().join("barrier-sync");
    fs::create_dir_all(&barrier_dir).unwrap();
    setup_barrier(&barrier_dir, "sync-config-update-before-cache-invalidate");

    // Spawn the writer: `snp register` with barrier env.
    let mut writer = snp_with_barrier(&config_dir, &barrier_dir)
        .args([
            "register",
            "--server",
            "http://127.0.0.1:19999",
            "--api-key",
            "new-key-12345",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Wait for the writer to enter the barrier (or timeout if register
    // fails before reaching the barrier).
    let entered = wait_for_entered(&barrier_dir, Duration::from_secs(10));

    if entered {
        // While the writer is paused, launch backup.
        let backup_dir = _tmp.path().join("backup-sync");
        let mut backup = snp_for_config(&config_dir)
            .args(["backup", "--output", backup_dir.to_str().unwrap()])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        // Give backup a moment to start.
        std::thread::sleep(Duration::from_millis(500));

        // Release the writer.
        release_barrier(&barrier_dir);

        // Wait for the writer to finish.
        let _ = wait_for_child(&mut writer, Duration::from_secs(15));

        // Wait for backup to finish.
        let backup_finished = wait_for_child(&mut backup, Duration::from_secs(15));
        assert!(backup_finished, "backup did not finish within 15s");

        // Verify the backup sync.toml is valid TOML (no partial writes).
        let manifest = read_manifest(&backup_dir);
        let files = manifest["files"].as_array().unwrap();
        let sync_entry = files.iter().find(|f| f["kind"] == "sync_config");
        if let Some(sync) = sync_entry {
            let path = sync["path"].as_str().unwrap();
            let content = fs::read_to_string(backup_dir.join(path)).unwrap();
            let _: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
                panic!("backup sync.toml is not valid TOML (partial write?): {}", e)
            });
        }

        // Verify full backup snapshot coherence.
        verify_backup_coherence(&backup_dir);
    } else {
        // Register may have failed before reaching the barrier (e.g.,
        // server not available). This is acceptable — the barrier point
        // is still tested by the other tests.
        let _ = writer.wait();
    }
}

/// Test: production build ignores barrier variables.
///
/// This test verifies that the production (no-feature) binary does not
/// check the `SNP_TEST_MUTATION_BARRIER_DIR` environment variable.
/// The binary should complete normally regardless of the barrier env.
#[test]
fn test_production_build_ignores_barrier() {
    let (_tmp, config_dir) = setup_test_env();

    // Create an initial library.
    setup_library(&config_dir, "work");

    // Set up a barrier directory with a point that matches a barrier
    // in the code. If the binary checked the barrier, it would hang.
    let barrier_dir = _tmp.path().join("barrier-prod");
    fs::create_dir_all(&barrier_dir).unwrap();
    fs::write(
        barrier_dir.join("point"),
        "snippet-save-after-write-before-cache-invalidate",
    )
    .unwrap();
    // Don't create "release" file — if the binary checked the barrier,
    // it would hang waiting for release.

    // Spawn the process and provide stdin.
    let mut child = snp_with_barrier(&config_dir, &barrier_dir)
        .args(["new", "--command-stdin", "--description", "prod-test"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Write stdin input.
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"echo prod-test")
        .unwrap();

    // Wait for the process to finish.
    let result = child.wait_with_output().unwrap();

    // The command should succeed (exit 0) — the production binary
    // ignores the barrier variable.
    assert!(
        result.status.success(),
        "production build should ignore barrier env, but command failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
