//! Interactive TUI commands must fail cleanly when there is no terminal.
//!
//! `select`, `run`, `clip`, and `search` all enter the ratatui selector. Without
//! a tty, `ratatui::init()` used to panic inside terminal setup and the process
//! aborted with SIGABRT (exit 134) — hostile to pipelines and CI. The selector
//! now guards the single `ratatui::init()` call site and returns an actionable
//! error instead.
//!
//! `tests/pty_integration.rs` covers the interactive path through a real pty
//! pair; this target covers the non-tty path.

mod support;

use std::process::Stdio;
use support::helpers::{setup_test_env, snp_in};

/// Spawn a command with piped stdio, which is never a tty.
fn run_without_tty(config_dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut cmd = snp_in(config_dir);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.output().expect("failed to spawn snp")
}

fn seed_snippet(config_dir: &std::path::Path) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["new", "echo guarded", "--description", "guarded"]);
    let out = cmd.output().expect("failed to seed snippet");
    assert!(
        out.status.success(),
        "seeding failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn tui_commands_fail_cleanly_without_a_terminal() {
    let (_tmp, config_dir) = setup_test_env();
    seed_snippet(&config_dir);

    for command in ["select", "run", "clip", "search"] {
        let out = run_without_tty(&config_dir, &[command]);

        let code = out.status.code().unwrap_or_else(|| {
            panic!(
                "{command}: process was killed by a signal (expected a clean exit).\n\
                 stderr: {}",
                String::from_utf8_lossy(&out.stderr)
            )
        });

        // A clean failure is exit 1. The regression was SIGABRT (no exit code).
        assert_eq!(
            code, 1,
            "{command}: expected exit 1, got {code}\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("panicked"),
            "{command}: must not panic without a tty\nstderr: {stderr}"
        );
        assert!(
            stderr.contains("requires a terminal"),
            "{command}: expected an explanatory error, got: {stderr}"
        );
        // The message must point users at the non-interactive spelling.
        assert!(
            stderr.contains("snp get --query") && stderr.contains("snp list --json"),
            "{command}: expected a pointer to the non-TUI alternative, got: {stderr}"
        );
    }
}

#[test]
fn get_works_without_a_terminal() {
    // The documented non-interactive alternative must keep working headless.
    let (_tmp, config_dir) = setup_test_env();
    seed_snippet(&config_dir);

    let out = run_without_tty(&config_dir, &["get", "--query", "guarded", "--field", "command"]);
    assert!(
        out.status.success(),
        "snp get must work without a tty\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "echo guarded");

    let out = run_without_tty(&config_dir, &["list", "--json"]);
    assert!(
        out.status.success(),
        "snp list --json must work without a tty\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
