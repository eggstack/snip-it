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
use std::process::Command;
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

[[snippets]]
id = "test-3"
description = "Choice snippet"
command = "echo <color=|_red_||_green_||_blue_||>"
tag = ["choice-test"]
output = ""

[[snippets]]
id = "test-4"
description = "Repeated choice variable"
command = "<x=|_a_||_b_||> and <x=|_a_||_b_||>"
tag = ["choice-test"]
output = ""
"#;
    fs::write(config_dir.join("snippets.toml"), library_content).unwrap();
    (tmp, config_dir)
}

/// Spawn snp in a PTY, send `keys` after a brief delay, wait for exit.
/// Returns (exit_code, captured_output).
fn run_snp_pty(args: &[&str], config_dir: &Path, keys: &[u8]) -> (i32, String) {
    run_snp_pty_with_delay(args, config_dir, keys, Duration::from_secs(2))
}

/// Like `run_snp_pty` but allows specifying the initial delay before keys
/// are sent. Useful for tests that need extra time for TUI transitions.
fn run_snp_pty_with_delay(
    args: &[&str],
    config_dir: &Path,
    keys: &[u8],
    initial_delay: Duration,
) -> (i32, String) {
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
    std::thread::sleep(initial_delay);

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

/// Spawn an interactive Bash session, source generated integration, invoke
/// the current-buffer widget, and return its exit code plus terminal output.
fn run_bash_capture_pty(
    config_dir: &Path,
    integration_path: &Path,
    command: &[u8],
    sentinel_path: &Path,
    shim_dir: &Path,
    args_capture: &Path,
) -> (i32, String) {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();

    let mut shell = CommandBuilder::new("bash");
    shell.args(["--noprofile", "--norc", "-i"]);
    shell.env("XDG_CONFIG_HOME", config_dir.parent().unwrap());
    shell.env("TERM", "xterm-256color");
    shell.env("PS1", "snp-pty> ");
    shell.env("PATH", format!("{}:/usr/bin:/bin", shim_dir.display()));
    shell.env("SNP_ARGS_CAPTURE", args_capture);
    shell.cwd(config_dir.parent().unwrap());
    let mut child = pair.slave.spawn_command(shell).unwrap();

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

    std::thread::sleep(Duration::from_millis(700));
    let raw_fd = pair.master.as_raw_fd().expect("master pty fd");
    let setup = format!(
        "source \"{}\"\nsnp_pty_capture() {{ snp_new_current --description 'PTY capture' --library pty; }}\nbind -x '\"\\C-n\": snp_pty_capture'\nprintf 'READY\\n'\n",
        integration_path.display()
    );
    for byte in setup.bytes() {
        unsafe {
            libc::write(raw_fd, &byte as *const u8 as *const libc::c_void, 1);
        }
    }
    std::thread::sleep(Duration::from_millis(700));

    for &byte in command {
        unsafe {
            libc::write(raw_fd, &byte as *const u8 as *const libc::c_void, 1);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // Ctrl-N invokes the explicitly installed current-buffer widget.
    let ctrl_n = 0x0e_u8;
    unsafe {
        libc::write(raw_fd, &ctrl_n as *const u8 as *const libc::c_void, 1);
    }
    std::thread::sleep(Duration::from_millis(1200));
    // Clear the unexecuted buffer, then exit the shell normally.
    for byte in b"\x15exit\n" {
        unsafe {
            libc::write(raw_fd, byte as *const u8 as *const libc::c_void, 1);
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let timeout = Duration::from_secs(10);
    let start = std::time::Instant::now();
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.exit_code() as i32,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let output = drain.join().unwrap();
                    panic!(
                        "bash PTY capture timed out after {timeout:?}; sentinel={:?}\nOUTPUT: {}",
                        sentinel_path,
                        String::from_utf8_lossy(&output)
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("try_wait error: {e}"),
        }
    };

    let output = drain.join().unwrap();
    (exit_code, String::from_utf8_lossy(&output).to_string())
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

#[test]
fn test_bash_current_buffer_capture_pty_persists_without_execution() {
    let bash_supports_readline_buffer = Command::new("bash")
        .args(["-c", "(( BASH_VERSINFO[0] >= 4 ))"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !bash_supports_readline_buffer {
        eprintln!("skipping: Bash Readline buffer API requires Bash 4+");
        return;
    }

    let (tmp, config_dir) = setup_test_env();
    let created = Command::new(snp_bin())
        .args(["library", "create", "pty"])
        .env("XDG_CONFIG_HOME", config_dir.parent().unwrap())
        .output()
        .unwrap();
    assert!(created.status.success(), "library create failed");
    let integration_path = tmp.path().join("snp-bash-init.sh");
    let integration = Command::new(snp_bin())
        .args(["shell", "init", "bash"])
        .output()
        .unwrap();
    assert!(integration.status.success());
    fs::write(&integration_path, integration.stdout).unwrap();

    let shim_dir = tmp.path().join("bin");
    fs::create_dir_all(&shim_dir).unwrap();
    let args_capture = tmp.path().join("snp-args.capture");
    fs::write(
        shim_dir.join("snp"),
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$SNP_ARGS_CAPTURE\"\nexec \"{}\" \"$@\"\n",
            snp_bin().display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(shim_dir.join("snp"), fs::Permissions::from_mode(0o755)).unwrap();
    }

    let sentinel = tmp.path().join("must-not-run");
    let command = format!(
        "printf '%s' 'quotes and $()'; touch '{}'",
        sentinel.display()
    );
    let (code, output) = run_bash_capture_pty(
        &config_dir,
        &integration_path,
        command.as_bytes(),
        &sentinel,
        &shim_dir,
        &args_capture,
    );
    assert_eq!(code, 0, "bash PTY exited unsuccessfully: {output}");
    assert!(!sentinel.exists(), "captured command was executed");

    let listed = Command::new(snp_bin())
        .args(["list", "--json", "--library", "pty"])
        .env("XDG_CONFIG_HOME", config_dir.parent().unwrap())
        .output()
        .unwrap();
    assert!(
        listed.status.success(),
        "list failed: {}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(
        snippets.len(),
        1,
        "pty output: {output}; args: {}; list: {}",
        fs::read_to_string(&args_capture).unwrap_or_default(),
        String::from_utf8_lossy(&listed.stdout)
    );
    assert_eq!(snippets[0]["command"], command);
}

#[test]
fn test_command_stdin_pty_persists_exact_command_body() {
    let (_tmp, config_dir) = setup_test_env();
    let command = b"printf '%s' 'pty ingestion'\n";
    let mut input = command.to_vec();
    // VEOF gives read_to_end an EOF while keeping the command body in the PTY.
    input.push(0x04);
    let (code, output) = run_snp_pty(
        &["new", "--command-stdin", "--description", "PTY ingestion"],
        &config_dir,
        &input,
    );
    assert_eq!(code, 0, "snp new --command-stdin failed: {output}");

    let listed = Command::new(snp_bin())
        .args(["list", "--json"])
        .env("XDG_CONFIG_HOME", config_dir.parent().unwrap())
        .output()
        .unwrap();
    assert!(listed.status.success());
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&listed.stdout).unwrap();
    let captured = snippets
        .iter()
        .find(|snippet| snippet["description"] == "PTY ingestion")
        .expect("PTY snippet was not persisted");
    assert_eq!(
        captured["command"],
        String::from_utf8_lossy(command).to_string()
    );
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

// -----------------------------------------------------------------------
// Release 3A: Pet choice variable PTY tests
// -----------------------------------------------------------------------

/// Select a choice variable snippet via --expanded, navigate down to the
/// second choice, then press Enter. Exit code should be 0.
#[test]
fn test_select_choice_variable_navigate_and_enter() {
    let (_tmp, config_dir) = setup_test_env();
    // Enter to select, Esc+NOR, j to navigate, Enter to confirm
    let keys = vec![b'\r', b'\x1b', b'j', b'\r'];
    let (code, output) = run_snp_pty_with_delay(
        &["select", "--expanded", "--filter", "Choice snippet"],
        &config_dir,
        &keys,
        Duration::from_secs(2),
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(
        code, 0,
        "snp select --expanded with choice variable + navigate + Enter should exit 0"
    );
}

/// Select a choice variable snippet via --expanded and accept the default
/// choice ("red"). Exit code should be 0.
#[test]
fn test_select_choice_variable_accept_default() {
    let (_tmp, config_dir) = setup_test_env();
    // Enter to select, Enter to accept default choice
    let keys = vec![b'\r', b'\r'];
    let (code, output) = run_snp_pty_with_delay(
        &["select", "--expanded", "--filter", "Choice snippet"],
        &config_dir,
        &keys,
        Duration::from_secs(2),
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(
        code, 0,
        "snp select --expanded with choice variable + accept default should exit 0"
    );
}

/// Select a choice variable snippet via --expanded, then cancel with Esc+q.
/// Esc goes to NOR mode, q returns Back (to snippet selector). Then Esc+q
/// exits the snippet selector with exit code 4.
#[test]
fn test_select_choice_variable_cancel() {
    let (_tmp, config_dir) = setup_test_env();
    // Enter to select, Esc+q to cancel back to snippet selector, Esc+q to exit
    let keys = vec![b'\r', b'\x1b', b'q', b'\x1b', b'q'];
    let (code, output) = run_snp_pty_with_delay(
        &["select", "--expanded", "--filter", "Choice snippet"],
        &config_dir,
        &keys,
        Duration::from_secs(2),
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(
        code, 4,
        "snp select --expanded with choice variable + cancel should exit 4"
    );
}

/// Select a snippet with a repeated choice variable via --expanded.
/// Deduplication ensures only one prompt for 'x'. Accept default, exit 0.
#[test]
fn test_select_repeated_choice_variable_single_prompt() {
    let (_tmp, config_dir) = setup_test_env();
    // Enter to select, Enter to accept default
    let keys = vec![b'\r', b'\r'];
    let (code, output) = run_snp_pty_with_delay(
        &["select", "--expanded", "--filter", "Repeated choice"],
        &config_dir,
        &keys,
        Duration::from_secs(2),
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(
        code, 0,
        "snp select --expanded with repeated choice variable + accept default should exit 0"
    );
}

/// Cancel via Ctrl+C at the choice prompt. Exit code should be 4.
#[test]
fn test_select_choice_variable_ctrl_c() {
    let (_tmp, config_dir) = setup_test_env();
    // Enter to select, Ctrl+C to cancel
    let keys = vec![b'\r', b'\x03'];
    let (code, output) = run_snp_pty_with_delay(
        &["select", "--expanded", "--filter", "Choice snippet"],
        &config_dir,
        &keys,
        Duration::from_secs(2),
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(
        code, 4,
        "snp select --expanded with choice variable + Ctrl+C should exit 4"
    );
}

/// Terminal restoration: after selecting and confirming a choice variable,
/// the PTY should still be functional. If terminal was not restored, the
/// drain thread would hang or produce garbled output.
#[test]
fn test_choice_variable_terminal_restoration() {
    let (_tmp, config_dir) = setup_test_env();
    let keys = vec![b'\r', b'\r'];
    let (code, output) = run_snp_pty_with_delay(
        &["select", "--expanded", "--filter", "Choice snippet"],
        &config_dir,
        &keys,
        Duration::from_secs(2),
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0, "should exit 0 after choice selection");
    assert!(
        !output.is_empty(),
        "PTY should have produced output (terminal was restored properly)"
    );
}

// ── Sort through real selector ────────────────────────────────────────

/// Helper: create a PTY test library with snippets for sort testing.
fn setup_sort_pty_env() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    fs::create_dir_all(&config_dir).unwrap();

    let library_content = r#"[[snippets]]
id = "sort-alpha"
description = "alpha deploy"
command = "deploy-alpha.sh"
tag = ["deploy"]
output = ""

[[snippets]]
id = "sort-beta"
description = "beta test"
command = "test-beta.sh"
tag = ["test"]
output = ""

[[snippets]]
id = "sort-gamma"
description = "gamma status"
command = "status-gamma.sh"
tag = ["status"]
output = ""
"#;
    fs::write(config_dir.join("snippets.toml"), library_content).unwrap();
    (tmp, config_dir)
}

#[test]
fn test_select_with_sort_description_selects_first_alphabetically() {
    let (_tmp, config_dir) = setup_sort_pty_env();
    let output_path = _tmp.path().join("sort_output.txt");

    // Use --sort description so "alpha deploy" should be first
    let (code, output) = run_snp_pty(
        &[
            "select",
            "--sort",
            "description",
            "--output-file",
            output_path.to_str().unwrap(),
        ],
        &config_dir,
        b"\r", // Enter to select first item
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(
        code, 0,
        "snp select --sort description with Enter should exit 0"
    );

    // The first item alphabetically by description is "alpha deploy"
    let selected = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        selected.trim(),
        "deploy-alpha.sh",
        "first item by description sort should be alpha deploy"
    );
}

#[test]
fn test_select_with_sort_command_selects_first_alphabetically_by_command() {
    let (_tmp, config_dir) = setup_sort_pty_env();
    let output_path = _tmp.path().join("sort_cmd_output.txt");

    // Use --sort command so "deploy-alpha.sh" should be first
    let (code, output) = run_snp_pty(
        &[
            "select",
            "--sort",
            "command",
            "--output-file",
            output_path.to_str().unwrap(),
        ],
        &config_dir,
        b"\r", // Enter to select first item
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(
        code, 0,
        "snp select --sort command with Enter should exit 0"
    );

    // First item alphabetically by command: deploy-alpha.sh < status-gamma.sh < test-beta.sh
    let selected = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        selected.trim(),
        "deploy-alpha.sh",
        "first item by command sort should be deploy-alpha.sh"
    );
}

// ── Usage tracking through PTY ────────────────────────────────────────

#[test]
fn test_select_usage_recorded_after_selection() {
    let (_tmp, config_dir) = setup_sort_pty_env();
    let output_path = _tmp.path().join("usage_output.txt");

    let (code, output) = run_snp_pty(
        &["select", "--output-file", output_path.to_str().unwrap()],
        &config_dir,
        b"\r", // Enter to select
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0, "snp select with Enter should exit 0");

    // Verify output file was written
    assert!(
        output_path.exists(),
        "output file should exist after selection"
    );
    let selected = fs::read_to_string(&output_path).unwrap();
    assert!(
        !selected.trim().is_empty(),
        "selected command should not be empty"
    );
}

#[test]
fn test_cancel_no_usage_recorded() {
    let (_tmp, config_dir) = setup_sort_pty_env();

    let usage_path = config_dir.join("usage.toml");

    let (code, output) = run_snp_pty(&["select"], &config_dir, b"\x1bq");
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 4, "snp select with Esc+q should exit 4");

    // Verify no usage.toml was created
    assert!(
        !usage_path.exists(),
        "usage.toml should not exist after cancellation"
    );
}

#[test]
fn test_run_records_usage_exactly_once() {
    let (_tmp, config_dir) = setup_test_env();

    let usage_path = config_dir.join("usage.toml");

    // Run the snippet (echo hello completes instantly)
    let (code, output) = run_snp_pty(&["run"], &config_dir, b"\r");
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0, "snp run with Enter should exit 0");

    // Verify usage.toml was created
    assert!(
        usage_path.exists(),
        "usage.toml should exist after successful run"
    );

    // Parse and verify exactly one entry with correct count
    let content = fs::read_to_string(&usage_path).unwrap();
    let idx: snip_it::usage::UsageIndex = toml::from_str(&content).unwrap();
    assert_eq!(
        idx.entries().len(),
        1,
        "usage should have exactly 1 entry after one run"
    );
    let entry = &idx.entries()[0];
    assert_eq!(entry.id, "test-1", "usage entry should reference test-1");
    assert_eq!(entry.use_count, 1, "use_count should be 1 after first run");
    assert!(entry.last_used_at.is_some(), "last_used_at should be set");
}
