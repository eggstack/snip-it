//! Restore manifest-gate contract.
//!
//! Proves that the bytes a restore *stages* are the bytes the manifest
//! describes. The manifest gate verifies every artifact's checksum, but the
//! staged bytes used to come from a later, independent read, so an artifact
//! substituted between the two reads was installed while the manifest still
//! "verified".
//!
//! The test uses the mutation-barrier seam to substitute the artifact
//! deterministically between the manifest gate and the staging step.

mod support;

use std::fs;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use support::helpers::*;

/// The barrier point the restore blocks on, right after the manifest gate.
const BARRIER_POINT: &str = "restore-after-manifest-gate";

fn setup_library(config_dir: &Path) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", "gate-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("gate-test.toml"),
        r#"[[snippets]]
id = "gate-1"
description = "gate test snippet"
command = "echo gate-test"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", "gate-test"]);
    cmd.output().unwrap();
}

/// Wait for the restore to reach the barrier.
fn wait_for_barrier(barrier_dir: &Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if barrier_dir.join("entered").exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the restore never reached the barrier at {BARRIER_POINT}");
}

#[test]
fn test_restore_rejects_an_artifact_substituted_after_the_manifest_gate() {
    let (tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);

    let backup_dir = tmp.path().join("backup-gate");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backup must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let backed_up_library = backup_dir.join("libraries").join("gate-test.toml");
    assert!(
        backed_up_library.exists(),
        "the backup must contain the library file"
    );

    let live_library = config_dir.join("libraries").join("gate-test.toml");
    let live_before = fs::read_to_string(&live_library).unwrap();

    // Arm the barrier so the restore blocks right after the manifest gate.
    let barrier_dir = tmp.path().join("barrier");
    fs::create_dir_all(&barrier_dir).unwrap();
    fs::write(barrier_dir.join("point"), BARRIER_POINT).unwrap();

    let child = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "replace"])
        .env("SNP_TEST_MUTATION_BARRIER_DIR", &barrier_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    wait_for_barrier(&barrier_dir);

    // Substitute the artifact *after* the manifest gate has verified it.
    fs::write(
        &backed_up_library,
        r#"[[snippets]]
id = "gate-1"
description = "substituted snippet"
command = "echo substituted"
"#,
    )
    .unwrap();

    // Let the restore continue.
    fs::write(barrier_dir.join("release"), "release").unwrap();

    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "the restore must fail when the artifact no longer matches the manifest"
    );
    assert!(
        stderr.contains("Checksum mismatch"),
        "the restore must report a checksum mismatch, got: {stderr}"
    );

    let live_after = fs::read_to_string(&live_library).unwrap();
    assert_eq!(
        live_after, live_before,
        "the substituted artifact must never reach the live library"
    );
}
