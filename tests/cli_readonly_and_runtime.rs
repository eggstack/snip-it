//! Read-only and runtime contracts of the CLI.
//!
//! - `snp select` is classified read-only (`command_behavior()` →
//!   `StartupRecoveryPolicy::SuppressReadOnly`, `StartupServices::Minimal`)
//!   and therefore must not migrate a legacy single-file checkout.
//! - `snp edit --output-stdin` must reject an invalid invocation *before* it
//!   reads stdin, so a broken pipeline fails fast instead of consuming the
//!   piped input.
//!
//! `tests/readonly_library_resolution.rs` covers `get`/`list`/`validate`;
//! `tests/tui_requires_terminal.rs` covers the non-tty failure of the
//! interactive commands.

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use support::helpers::*;

/// Snapshot of every file below `dir`, keyed by relative path.
fn file_snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(bytes) = fs::read(&p) {
                let rel = p.strip_prefix(dir).unwrap_or(&p).to_path_buf();
                out.insert(rel, bytes);
            }
        }
    }
    out
}

/// A legacy single-file checkout: one `snippets.toml`, no `libraries/`.
fn write_legacy_snippets(config_dir: &Path) {
    fs::write(
        config_dir.join("snippets.toml"),
        r#"[[snippets]]
id = "legacy-1"
description = "legacy snippet"
command = "echo legacy"
"#,
    )
    .unwrap();
}

#[test]
fn select_leaves_a_legacy_checkout_untouched() {
    // `snp select` used to resolve the library through the *mutating*
    // resolver: `select_cmd::run` → `run_snippet_selection` →
    // `get_library_path` → `ensure_library_mode()` →
    // `migrate_from_single_file()`, which created `libraries/`, copied the
    // legacy file (leaving the original behind as a second, inert copy),
    // wrote `libraries.toml`, and bumped the generation — from a command
    // classified read-only.
    let (_tmp, config_dir) = setup_test_env();
    write_legacy_snippets(&config_dir);
    assert!(!config_dir.join("libraries").exists());
    let before = file_snapshot(&config_dir);

    let output = snp_in(&config_dir).args(["select"]).output().unwrap();

    assert!(
        !config_dir.join("libraries").exists(),
        "read-only select must not create the libraries directory"
    );
    assert!(
        !config_dir.join("libraries.toml").exists(),
        "read-only select must not write libraries.toml"
    );
    assert!(
        !config_dir.join(".transaction").exists(),
        "read-only select must not create a transaction directory"
    );
    assert_eq!(
        file_snapshot(&config_dir),
        before,
        "read-only select must not modify any file"
    );

    // The legacy library was still resolved (read in place): the command
    // fails on the missing terminal, not on a missing library.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("requires a terminal"),
        "select must reach the selector with the legacy library resolved: {stderr}"
    );
    assert!(
        !stderr.contains("No library found"),
        "select must resolve the legacy library read-only: {stderr}"
    );
}

#[test]
fn select_does_not_bump_the_library_generation() {
    // A migrated library checkout must also stay byte-identical: the
    // read-only resolver never calls `ensure_library_mode()`, so the
    // generation counter is untouched.
    let (_tmp, config_dir) = setup_test_env();
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        config_dir.join("libraries.toml"),
        "[[libraries]]\nfilename = \"work\"\nlibrary_id = \"server-work\"\nis_primary = true\ngeneration = 7\n",
    )
    .unwrap();
    fs::write(
        libraries_dir.join("work.toml"),
        "[[snippets]]\nid = \"work-1\"\ndescription = \"work snippet\"\ncommand = \"echo work\"\n",
    )
    .unwrap();
    let before = file_snapshot(&config_dir);

    let output = snp_in(&config_dir).args(["select"]).output().unwrap();

    assert_eq!(
        file_snapshot(&config_dir),
        before,
        "read-only select must not rewrite the library index"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("requires a terminal"),
        "select must reach the selector: {stderr}"
    );
}

/// Runs `snp` with stdin connected to a pipe that is never closed.
///
/// Returns `None` when the child did not exit within `timeout` (it is
/// killed). A command that reads stdin to EOF therefore never finishes
/// while this helper holds the pipe open.
fn run_with_open_stdin(config_dir: &Path, args: &[&str], timeout: Duration) -> Option<Output> {
    let mut child = snp_in(config_dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn snp");
    let mut stdin = child.stdin.take().unwrap();
    let _ = stdin.write_all(b"piped output\n");
    let _ = stdin.flush();

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    drop(stdin);
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                let _ = child.kill();
                drop(stdin);
                return None;
            }
        }
    }
    drop(stdin);
    Some(child.wait_with_output().unwrap())
}

#[test]
fn edit_output_stdin_without_filter_fails_before_reading_stdin() {
    // `snp edit --output-stdin` used to read stdin to EOF *before* the
    // `--filter` requirement was checked, so an invalid invocation blocked
    // until Ctrl-D and the piped input was consumed and discarded.
    let (_tmp, config_dir) = setup_test_env();

    let output = run_with_open_stdin(
        &config_dir,
        &["edit", "--output-stdin"],
        Duration::from_secs(20),
    )
    .expect("edit --output-stdin must not block on stdin");

    assert!(
        !output.status.success(),
        "an invalid invocation must fail: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--filter is required"),
        "expected the documented requirement error, got: {stderr}"
    );
}

#[test]
fn edit_output_stdin_with_filter_waits_for_stdin() {
    // Control for the test above: with `--filter` present the command is
    // valid and *does* read stdin, so it blocks while the pipe is open.
    // This proves the timeout in the previous test detects a blocking read.
    let (_tmp, config_dir) = setup_test_env();

    let output = run_with_open_stdin(
        &config_dir,
        &["edit", "--output-stdin", "--filter", "anything"],
        Duration::from_secs(3),
    );

    assert!(
        output.is_none(),
        "a valid --output-stdin invocation must read stdin before proceeding"
    );
}
