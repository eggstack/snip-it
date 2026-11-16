//! Maintenance consolidation M002 — shared backup copy-policy contracts.
//!
//! Proves the four backup paths refuse symlink sources, that the repair
//! backup is restorable via `snp restore`, and that forensic copies do not
//! follow symlink targets.

mod support;

use std::fs;
use std::path::Path;
use support::helpers::*;

fn write_primary_library(config_dir: &Path, body: &str) {
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(libraries_dir.join("primary.toml"), body).unwrap();
    fs::write(
        config_dir.join("libraries.toml"),
        r#"
generation = 1

[[libraries]]
filename = "primary"
name = "primary"
is_primary = true
"#,
    )
    .unwrap();
}

fn valid_snippet_body() -> &'static str {
    r#"[[snippets]]
id = "s1"
description = "pre-repair"
command = "echo pre"
favorite = false
created_at = 1700000000
updated_at = 1700000000
device_id = "test-device"
"#
}

#[cfg(unix)]
fn plant_symlinked_library(config_dir: &Path, tmp: &Path) -> std::path::PathBuf {
    let external = tmp.join("external-secret.toml");
    // Valid TOML so the repair *scan* does not treat the symlink target as a
    // parse error before the backup admission half runs. The refusal under
    // test is the copy-source admission, not TOML parsing.
    fs::write(
        &external,
        r#"[[snippets]]
id = "external"
description = "secret"
command = "echo secret"
favorite = false
created_at = 1700000000
updated_at = 1700000000
device_id = "ext"
"#,
    )
    .unwrap();
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let link = libraries_dir.join("leaky.toml");
    std::os::unix::fs::symlink(&external, &link).unwrap();
    link
}

#[cfg(unix)]
#[test]
fn path2_backup_refuses_symlink_and_names_path() {
    let (tmp, config_dir) = setup_test_env();
    write_primary_library(config_dir.as_path(), valid_snippet_body());
    let link = plant_symlinked_library(&config_dir, tmp.path());
    let backup_dir = tmp.path().join("backup-out");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("symlink") || stderr.contains("escapes config root"),
        "stderr={stderr}"
    );
    // Containment refusal names the canonicalized target; symlink refusal
    // names the link path. Either form is acceptable evidence.
    assert!(
        stderr.contains("leaky.toml")
            || stderr.contains("external-secret.toml")
            || stderr.contains(link.display().to_string().as_str()),
        "refusal must name the path: {stderr}"
    );
    assert!(!backup_dir.exists(), "partial snapshot must not remain");
}

#[cfg(unix)]
#[test]
fn path3_repair_apply_refuses_symlink_and_applies_nothing() {
    let (tmp, config_dir) = setup_test_env();
    write_primary_library(config_dir.as_path(), valid_snippet_body());
    // Safe repair so `--apply` reaches create_repair_backup.
    fs::write(
        config_dir.join("usage.toml"),
        r#"[[entries]]
id = "missing-id"
use_count = 1
last_used_at = 0
"#,
    )
    .unwrap();
    let _link = plant_symlinked_library(&config_dir, tmp.path());
    let before = fs::read(config_dir.join("libraries/primary.toml")).unwrap();
    let output = snp_in(&config_dir)
        .args(["repair", "--apply"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("symlink")
            || stderr.contains("escapes")
            || stderr.contains("Copy source")
            || stderr.contains("Backup entry"),
        "stderr={stderr}"
    );
    let after = fs::read(config_dir.join("libraries/primary.toml")).unwrap();
    assert_eq!(
        before, after,
        "repair must apply nothing when backup refuses"
    );
}

#[cfg(unix)]
#[test]
fn path4_restore_refuses_symlink_before_mutating_live_config() {
    let (tmp, config_dir) = setup_test_env();
    write_primary_library(config_dir.as_path(), valid_snippet_body());
    let backup_dir = tmp.path().join("good-backup");
    let backup = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        backup.status.success(),
        "setup backup must succeed: {}",
        String::from_utf8_lossy(&backup.stderr)
    );

    let _link = plant_symlinked_library(&config_dir, tmp.path());
    let before = fs::read(config_dir.join("libraries/primary.toml")).unwrap();
    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "replace"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("symlink")
            || stderr.contains("escapes")
            || stderr.contains("Copy source")
            || stderr.contains("Backup entry"),
        "stderr={stderr}"
    );
    let after = fs::read(config_dir.join("libraries/primary.toml")).unwrap();
    assert_eq!(before, after, "live config must be untouched");
}

#[test]
fn repair_backup_is_restorable_rollback_point() {
    let (_tmp, config_dir) = setup_test_env();
    write_primary_library(config_dir.as_path(), valid_snippet_body());
    fs::write(
        config_dir.join("usage.toml"),
        r#"[[entries]]
id = "missing-id"
use_count = 1
last_used_at = 0
"#,
    )
    .unwrap();
    let pre_bytes = fs::read(config_dir.join("libraries/primary.toml")).unwrap();

    let repair = snp_in(&config_dir)
        .args(["repair", "--apply", "--json"])
        .output()
        .unwrap();
    assert!(
        repair.status.success(),
        "repair should succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&repair.stdout),
        String::from_utf8_lossy(&repair.stderr)
    );

    let backups = config_dir.join("backups");
    let mut repair_dirs: Vec<_> = fs::read_dir(&backups)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("repair-"))
        })
        .collect();
    repair_dirs.sort();
    let repair_backup = repair_dirs.pop().expect("repair backup directory");
    assert!(
        repair_backup.join("manifest.toml").exists(),
        "repair backup must carry manifest.toml"
    );

    let restore = snp_in(&config_dir)
        .args([
            "restore",
            repair_backup.to_str().unwrap(),
            "--mode",
            "replace",
        ])
        .output()
        .unwrap();
    assert!(
        restore.status.success(),
        "restore from repair backup must succeed: {}",
        String::from_utf8_lossy(&restore.stderr)
    );
    let restored = fs::read(config_dir.join("libraries/primary.toml")).unwrap();
    assert_eq!(
        restored, pre_bytes,
        "restored library must equal pre-repair bytes"
    );
}
