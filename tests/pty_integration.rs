//! PTY-backed end-to-end tests for snippet selection exit codes.
//!
//! These tests verify the cancellation contract:
//! - `snp select` → exit 4 on primary selector cancel
//! - `snp run` → exit 0 on cancel (normal completion)
//! - `snp select --output-file` → exit 4 + file not created on cancel
//! - `snp select --expanded` → exit 4 on primary cancel (before variable prompt)
//!
//! Uses `portable-pty` to create a pseudo-terminal pair. Keys are written
//! directly to the master fd (bypassing `UnixMasterWriter::drop` which
//! sends VEOF and interferes with crossterm's event loop).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

fn snp_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_snp"))
}

fn setup_test_env() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    fs::create_dir_all(&config_dir).unwrap();

    let library_content = r#"[[snippets]]
id = "test-1"
description = "Test snippet"
command = "echo hello"
tag = ["test"]
output = ""

[[snippets]]
id = "test-2"
description = "Variable snippet"
command = "echo <name=world>"
tag = ["test"]
output = ""
"#;
    fs::write(config_dir.join("snippets.toml"), library_content).unwrap();
    (tmp, config_dir)
}

/// Spawn snp in a PTY, send `keys` after a brief delay, wait for exit.
/// Returns (exit_code, captured_output).
fn run_snp_pty(args: &[&str], config_dir: &Path, keys: &[u8]) -> (i32, String) {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();

    let mut cmd = CommandBuilder::new(snp_bin());
    for a in args {
        cmd.arg(a);
    }
    cmd.env("XDG_CONFIG_HOME", config_dir.parent().unwrap());
    cmd.env("TERM", "xterm-256color");
    cmd.cwd(config_dir.parent().unwrap());

    let mut child = pair.slave.spawn_command(cmd).unwrap();

    // Drain thread: read master output so the slave's stdout doesn't block.
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut output = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => output.extend_from_slice(&buf[..n]),
            }
        }
        output
    });

    // Give the TUI time to start up and render
    std::thread::sleep(Duration::from_secs(2));

    // Write keys directly to the master pty fd.
    // Send each byte separately with a small delay so crossterm's parser
    // processes them as individual events (e.g. Esc then 'q', not as a
    // combined escape sequence like ESC-q).
    let raw_fd = pair.master.as_raw_fd().expect("master pty fd");
    for &byte in keys {
        let written = unsafe { libc::write(raw_fd, &byte as *const u8 as *const libc::c_void, 1) };
        eprintln!("Wrote byte {written} ({byte:#04x}) to master fd {raw_fd}");
        std::thread::sleep(Duration::from_millis(50));
    }

    // Wait for child with a generous timeout
    let timeout = Duration::from_secs(10);
    let start = std::time::Instant::now();
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break status.exit_code() as i32;
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    // Drain output before panicking
                    let output = drain.join().unwrap();
                    let output_str = String::from_utf8_lossy(&output);
                    panic!("snp process timed out after {timeout:?}\nOUTPUT: {output_str}");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("try_wait error: {e}"),
        }
    };

    let output = drain.join().unwrap();
    let output_str = String::from_utf8_lossy(&output).to_string();
    (exit_code, output_str)
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[test]
fn test_select_cancel_returns_exit_4() {
    let (_tmp, config_dir) = setup_test_env();
    // Esc to switch to normal mode, then q to quit
    let (code, output) = run_snp_pty(&["select"], &config_dir, b"\x1bq");
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 4, "snp select with Esc+q should exit 4");
}

#[test]
fn test_select_ctrl_c_returns_exit_4() {
    let (_tmp, config_dir) = setup_test_env();
    let (code, output) = run_snp_pty(&["select"], &config_dir, b"\x03");
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 4, "snp select with Ctrl-C should exit 4");
}

#[test]
fn test_select_enter_returns_exit_0() {
    let (_tmp, config_dir) = setup_test_env();
    let (code, output) = run_snp_pty(&["select", "--filter", "Test snippet"], &config_dir, b"\r");
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0, "snp select with Enter should exit 0");
}

#[test]
fn test_run_cancel_returns_exit_0() {
    let (_tmp, config_dir) = setup_test_env();
    // Esc to normal mode, then q to quit
    let (code, output) = run_snp_pty(&["run"], &config_dir, b"\x1bq");
    eprintln!("OUTPUT: {output}");
    assert_eq!(
        code, 0,
        "snp run with Esc+q should exit 0 (normal completion)"
    );
}

#[test]
fn test_select_output_file_cancel_cleanup() {
    let (tmp, config_dir) = setup_test_env();
    let output_path = tmp.path().join("test_output.txt");

    let (code, output) = run_snp_pty(
        &["select", "--output-file", output_path.to_str().unwrap()],
        &config_dir,
        b"\x1bq",
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 4, "snp select --output-file with Esc+q should exit 4");
    assert!(
        !output_path.exists(),
        "Output file should not exist after cancellation"
    );
}

#[test]
fn test_select_expanded_cancel_returns_exit_4() {
    let (_tmp, config_dir) = setup_test_env();
    let (code, output) = run_snp_pty(
        &["select", "--expanded", "--filter", "Variable"],
        &config_dir,
        b"\x1bq",
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 4, "snp select --expanded with Esc+q should exit 4");
}

#[test]
fn test_clip_cancel_returns_exit_0() {
    let (_tmp, config_dir) = setup_test_env();
    let (code, output) = run_snp_pty(&["clip"], &config_dir, b"\x1bq");
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0, "snp clip with Esc+q should exit 0");
}

#[test]
fn test_search_cancel_returns_exit_0() {
    let (_tmp, config_dir) = setup_test_env();
    let (code, output) = run_snp_pty(&["search"], &config_dir, b"\x1bq");
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0, "snp search with Esc+q should exit 0");
}

/// Diagnostic: verify that data written to master is readable on slave stdin.
#[test]
fn test_pty_stdin_readable() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();

    // Run a program that reads a single byte from stdin and echoes it
    let mut cmd = CommandBuilder::new("/bin/sh");
    cmd.arg("-c");
    cmd.arg("read BYTE; echo GOT_$BYTE");
    let mut child = pair.slave.spawn_command(cmd).unwrap();

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut output = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => output.extend_from_slice(&buf[..n]),
            }
        }
        output
    });

    std::thread::sleep(Duration::from_millis(500));

    // Write 'q' to master
    let raw_fd = pair.master.as_raw_fd().expect("master pty fd");
    let n = unsafe { libc::write(raw_fd, b"q\n".as_ptr() as *const libc::c_void, 2) };
    eprintln!("Wrote {n} bytes to master");

    // Wait for child with timeout
    let timeout = Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                eprintln!("Child exited: {}", status.exit_code());
                break;
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    panic!("child timed out");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("try_wait error: {e}"),
        }
    }

    let output = drain.join().unwrap();
    let output_str = String::from_utf8_lossy(&output);
    eprintln!("PTY output: {output_str:?}");

    assert!(
        output_str.contains("GOT_q"),
        "Should have received 'q' on stdin, got: {output_str:?}"
    );
}

/// Diagnostic: verify the PTY mechanism works with a simple command.
#[test]
fn test_pty_basic_echo() {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();

    let mut cmd = CommandBuilder::new("/bin/sh");
    cmd.arg("-c");
    cmd.arg("echo HELLO_FROM_PTY");
    let mut child = pair.slave.spawn_command(cmd).unwrap();

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut output = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => output.extend_from_slice(&buf[..n]),
            }
        }
        output
    });

    // Wait for child to exit with timeout
    let timeout = Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                eprintln!("Child exited: {}", status.exit_code());
                break;
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    panic!("child timed out");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("try_wait error: {e}"),
        }
    }

    let output = drain.join().unwrap();
    let output_str = String::from_utf8_lossy(&output);
    eprintln!("PTY output: {output_str:?}");

    assert!(
        output_str.contains("HELLO_FROM_PTY"),
        "PTY output should contain echoed text, got: {output_str:?}"
    );
}
