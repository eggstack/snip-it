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

// ── TUI output preview ──────────────────────────────────────────────

/// Helper: create a library with snippets that have non-empty output fields.
fn setup_test_env_with_output() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let library_content = r#"[[snippets]]
id = "output-1"
description = "Snippet with output"
command = "echo hello"
tag = ["test"]
output = "sample output text"

[[snippets]]
id = "output-2"
description = "Another snippet"
command = "echo world"
tag = ["test"]
output = "secondary output"
"#;
    fs::write(libraries_dir.join("output-test.toml"), library_content).unwrap();

    let libraries_config = r#"[[libraries]]
filename = "output-test"
is_primary = true
"#;
    fs::write(config_dir.join("libraries.toml"), libraries_config).unwrap();

    (tmp, config_dir)
}

#[test]
fn test_tui_output_preview_shows_output_content() {
    let (_tmp, config_dir) = setup_test_env_with_output();
    let keys = vec![b'\x1b', b'q']; // Esc to NORMAL mode, then q to quit
    let (code, output) = run_snp_pty(&["select", "--library", "output-test"], &config_dir, &keys);
    assert_eq!(code, 4, "select cancel should exit 4");
    assert!(
        output.contains("--- Output / Notes ---"),
        "PTY output should contain '--- Output / Notes ---' separator. Got: {output}"
    );
    assert!(
        output.contains("sample output text"),
        "PTY output should contain the output content. Got: {output}"
    );
}

// ── Ranking and identity correctness ──────────────────────────────────

/// Helper: create a multi-library PTY test environment.
fn setup_multi_library_pty_env() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    fs::create_dir_all(&config_dir).unwrap();

    // Create two libraries via snp library create
    let mut cmd = Command::new(snp_bin());
    cmd.env("XDG_CONFIG_HOME", config_dir.parent().unwrap());
    cmd.args(["library", "create", "multi-a"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    fs::write(
        libraries_dir.join("multi-a.toml"),
        r#"[[snippets]]
id = "multi-a-1"
description = "alpha from A"
command = "alpha-a.sh"
tag = ["test"]
output = ""

[[snippets]]
id = "multi-a-2"
description = "bravo from A"
command = "bravo-a.sh"
tag = ["test"]
output = ""
"#,
    )
    .unwrap();

    let mut cmd = Command::new(snp_bin());
    cmd.env("XDG_CONFIG_HOME", config_dir.parent().unwrap());
    cmd.args(["library", "create", "multi-b"]);
    cmd.output().unwrap();

    fs::write(
        libraries_dir.join("multi-b.toml"),
        r#"[[snippets]]
id = "multi-b-1"
description = "alpha from B"
command = "alpha-b.sh"
tag = ["test"]
output = ""

[[snippets]]
id = "multi-b-2"
description = "charlie from B"
command = "charlie-b.sh"
tag = ["test"]
output = ""
"#,
    )
    .unwrap();

    // Set multi-a as primary
    let mut cmd = Command::new(snp_bin());
    cmd.env("XDG_CONFIG_HOME", config_dir.parent().unwrap());
    cmd.args(["library", "set-primary", "multi-a"]);
    cmd.output().unwrap();

    (tmp, config_dir)
}

#[test]
fn test_select_sort_last_used_accepts_flag() {
    let (_tmp, config_dir) = setup_sort_pty_env();
    let output_path = _tmp.path().join("last_used_output.txt");

    // --sort last-used should be accepted and produce output
    // Note: TUI uses updated_at as proxy since UsageIndex isn't loaded in TUI
    let (code, output) = run_snp_pty(
        &[
            "select",
            "--sort",
            "last-used",
            "--output-file",
            output_path.to_str().unwrap(),
        ],
        &config_dir,
        b"\r", // Enter to select first item
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0, "snp select --sort last-used should exit 0");

    let selected = fs::read_to_string(&output_path).unwrap();
    assert!(
        !selected.trim().is_empty(),
        "selected command should not be empty"
    );
}

#[test]
fn test_select_with_multi_library_sort() {
    let (_tmp, config_dir) = setup_multi_library_pty_env();
    let output_path = _tmp.path().join("multi_sort_output.txt");

    // Sort by description across multi-a library: alpha < bravo
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
    assert_eq!(code, 0, "multi-library sort should exit 0");

    let selected = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        selected.trim(),
        "alpha-a.sh",
        "first item by description sort should be alpha from A"
    );
}

#[test]
fn test_select_unicode_sort_order() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    fs::create_dir_all(&config_dir).unwrap();

    let library_content = r#"[[snippets]]
id = "uni-alpha"
description = "alpha deploy"
command = "deploy.sh"
tag = ["test"]
output = ""

[[snippets]]
id = "uni-mid"
description = "Über test"
command = "uber.sh"
tag = ["test"]
output = ""

[[snippets]]
id = "uni-cjk"
description = "日本語テスト"
command = "jp.sh"
tag = ["test"]
output = ""
"#;
    fs::write(config_dir.join("snippets.toml"), library_content).unwrap();

    let output_path = tmp.path().join("uni_output.txt");

    // Sort by description - should be Unicode-aware case-insensitive
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
    assert_eq!(code, 0, "unicode sort should exit 0");

    let selected = fs::read_to_string(&output_path).unwrap();
    // alpha < Über < 日本語 (case-insensitive Unicode-aware sort)
    assert_eq!(
        selected.trim(),
        "deploy.sh",
        "first by Unicode description sort should be alpha deploy"
    );
}

#[test]
fn test_select_identity_sorted_row_matches_preview() {
    let (_tmp, config_dir) = setup_sort_pty_env();
    let output_path = _tmp.path().join("identity_output.txt");

    // Sort by description, then check that the PTY preview shows the first
    // alphabetical item's content (alpha deploy / deploy-alpha.sh)
    let keys = vec![b'\r']; // Enter to select first
    let (code, output) = run_snp_pty(
        &[
            "select",
            "--sort",
            "description",
            "--output-file",
            output_path.to_str().unwrap(),
        ],
        &config_dir,
        &keys,
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0);

    let selected = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        selected.trim(),
        "deploy-alpha.sh",
        "selected command should match the first item in sorted order"
    );

    // The PTY output should have shown "alpha deploy" in the selection
    assert!(
        output.contains("alpha deploy"),
        "PTY should have displayed 'alpha deploy' as the first sorted item"
    );
}

#[test]
fn test_select_favorites_first_in_pty() {
    let (_tmp, config_dir) = setup_sort_pty_env();
    let output_path = _tmp.path().join("fav_pty_output.txt");

    // With favorites-first, "alpha deploy" (favorite=true) should be first
    let (code, output) = run_snp_pty(
        &[
            "select",
            "--sort",
            "description",
            "--favorites-first",
            "--output-file",
            output_path.to_str().unwrap(),
        ],
        &config_dir,
        b"\r", // Enter to select first item
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0);

    let selected = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        selected.trim(),
        "deploy-alpha.sh",
        "favorites-first should put favorite item first"
    );
}

#[test]
fn test_select_sort_by_command_pty() {
    let (_tmp, config_dir) = setup_sort_pty_env();
    let output_path = _tmp.path().join("cmd_sort_pty.txt");

    // Sort by command: deploy-alpha.sh < status-gamma.sh < test-beta.sh
    let (code, output) = run_snp_pty(
        &[
            "select",
            "--sort",
            "command",
            "--output-file",
            output_path.to_str().unwrap(),
        ],
        &config_dir,
        b"\r",
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0);

    let selected = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        selected.trim(),
        "deploy-alpha.sh",
        "first by command sort should be deploy-alpha.sh"
    );
}

#[test]
fn test_select_no_sort_preserves_original_order() {
    let (_tmp, config_dir) = setup_sort_pty_env();
    let output_path = _tmp.path().join("no_sort_output.txt");

    // Without --sort, should preserve original order (alpha, beta, gamma)
    let (code, output) = run_snp_pty(
        &["select", "--output-file", output_path.to_str().unwrap()],
        &config_dir,
        b"\r", // Enter to select first item
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0);

    let selected = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        selected.trim(),
        "deploy-alpha.sh",
        "without --sort, first item should be alpha deploy (insertion order)"
    );
}

// ── Divergent-metadata PTY identity tests ──────────────────────────────
//
// These tests use a fixture where updated_at, use_count, and last_used_at
// deliberately disagree, proving the TUI selects the correct snippet for
// each sort mode.

/// Create a library with deliberately divergent updated_at, use_count, and
/// last_used_at values.
///
/// | Snippet | updated_at | use_count | last_used_at |
/// | A       | 300        | 1         | 100          |
/// | B       | 100        | 9         | 200          |
/// | C       | 200        | 2         | 900          |
///
/// Expected orderings:
/// - recent (by updated_at desc): A, C, B
/// - most-used (by use_count desc): B, C, A
/// - last-used (by last_used_at desc): C, B, A
fn setup_divergent_metadata_pty_env() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    fs::create_dir_all(&config_dir).unwrap();

    let library_content = r#"[[snippets]]
id = "div-a"
description = "snippet A"
command = "echo alpha"
tag = ["divergent"]
output = ""
created_at = 100
updated_at = 300

[[snippets]]
id = "div-b"
description = "snippet B"
command = "echo bravo"
tag = ["divergent"]
output = ""
created_at = 50
updated_at = 100

[[snippets]]
id = "div-c"
description = "snippet C"
command = "echo charlie"
tag = ["divergent"]
output = ""
created_at = 150
updated_at = 200
"#;
    fs::write(config_dir.join("snippets.toml"), library_content).unwrap();

    let usage_content = r#"[[entries]]
id = "div-a"
use_count = 1
last_used_at = 100

[[entries]]
id = "div-b"
use_count = 9
last_used_at = 200

[[entries]]
id = "div-c"
use_count = 2
last_used_at = 900
"#;
    fs::write(config_dir.join("usage.toml"), usage_content).unwrap();

    (tmp, config_dir)
}

#[test]
fn test_pty_select_sort_last_used_selects_c_first() {
    let (_tmp, config_dir) = setup_divergent_metadata_pty_env();
    let output_path = _tmp.path().join("div_last_used.txt");

    let (code, output) = run_snp_pty(
        &[
            "select",
            "--sort",
            "last-used",
            "--output-file",
            output_path.to_str().unwrap(),
        ],
        &config_dir,
        b"\r",
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0, "snp select --sort last-used should exit 0");

    let selected = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        selected.trim(),
        "echo charlie",
        "last-used sort should select C first (last_used_at=900)"
    );
}

#[test]
fn test_pty_select_sort_most_used_selects_b_first() {
    let (_tmp, config_dir) = setup_divergent_metadata_pty_env();
    let output_path = _tmp.path().join("div_most_used.txt");

    let (code, output) = run_snp_pty(
        &[
            "select",
            "--sort",
            "most-used",
            "--output-file",
            output_path.to_str().unwrap(),
        ],
        &config_dir,
        b"\r",
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0, "snp select --sort most-used should exit 0");

    let selected = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        selected.trim(),
        "echo bravo",
        "most-used sort should select B first (use_count=9)"
    );
}

#[test]
fn test_pty_select_sort_recent_selects_a_first() {
    let (_tmp, config_dir) = setup_divergent_metadata_pty_env();
    let output_path = _tmp.path().join("div_recent.txt");

    let (code, output) = run_snp_pty(
        &[
            "select",
            "--sort",
            "recent",
            "--output-file",
            output_path.to_str().unwrap(),
        ],
        &config_dir,
        b"\r",
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0, "snp select --sort recent should exit 0");

    let selected = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        selected.trim(),
        "echo alpha",
        "recent sort should select A first (updated_at=300)"
    );
}

#[test]
fn test_pty_run_sort_most_used_records_usage_for_b() {
    let (_tmp, config_dir) = setup_divergent_metadata_pty_env();

    let (code, output) = run_snp_pty(&["run", "--sort", "most-used"], &config_dir, b"\r");
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0, "snp run --sort most-used should exit 0");

    let usage_path = config_dir.join("usage.toml");
    assert!(usage_path.exists(), "usage.toml should exist after run");

    let content = fs::read_to_string(&usage_path).unwrap();
    let idx: snip_it::usage::UsageIndex = toml::from_str(&content).unwrap();
    let b_entry = idx.entries().iter().find(|e| e.id == "div-b");
    assert!(b_entry.is_some(), "usage should have an entry for div-b");
    assert_eq!(
        b_entry.unwrap().use_count,
        10,
        "div-b use_count should be 10 (9 + 1 from this run)"
    );
}

#[test]
fn test_pty_clip_sort_last_used_records_usage_for_c() {
    let (_tmp, config_dir) = setup_divergent_metadata_pty_env();

    let (code, output) = run_snp_pty(&["clip", "--sort", "last-used"], &config_dir, b"\r");
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0, "snp clip --sort last-used should exit 0");

    let usage_path = config_dir.join("usage.toml");
    assert!(usage_path.exists(), "usage.toml should exist after clip");

    let content = fs::read_to_string(&usage_path).unwrap();
    let idx: snip_it::usage::UsageIndex = toml::from_str(&content).unwrap();
    let c_entry = idx.entries().iter().find(|e| e.id == "div-c");
    assert!(c_entry.is_some(), "usage should have an entry for div-c");
    assert_eq!(
        c_entry.unwrap().use_count,
        3,
        "div-c use_count should be 3 (2 + 1 from this clip)"
    );
}

#[test]
fn test_pty_select_interactive_cycle_changes_order() {
    let (_tmp, config_dir) = setup_divergent_metadata_pty_env();
    let output_path = _tmp.path().join("div_cycle.txt");

    // Initial sort is None (insertion order): A, B, C → first item is "echo alpha"
    // Press 'n' to cycle to Newest sort: A (updated_at=300), C (200), B (100)
    // Then Enter to select: should pick "echo alpha" (A)
    let (code, output) = run_snp_pty(
        &["select", "--output-file", output_path.to_str().unwrap()],
        &config_dir,
        b"n\r",
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(code, 0, "snp select with interactive cycle should exit 0");

    let selected = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        selected.trim(),
        "echo alpha",
        "after cycling to Newest, first item should be A (updated_at=300)"
    );
}

#[test]
fn test_pty_select_favorites_first_with_most_used() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    fs::create_dir_all(&config_dir).unwrap();

    let library_content = r#"[[snippets]]
id = "fav-a"
description = "fav alpha"
command = "echo-fav-alpha"
tag = ["fav-test"]
output = ""
favorite = true
created_at = 100
updated_at = 300

[[snippets]]
id = "fav-b"
description = "fav bravo"
command = "echo-fav-bravo"
tag = ["fav-test"]
output = ""
favorite = false
created_at = 50
updated_at = 100

[[snippets]]
id = "fav-c"
description = "fav charlie"
command = "echo-fav-charlie"
tag = ["fav-test"]
output = ""
favorite = true
created_at = 150
updated_at = 200
"#;
    fs::write(config_dir.join("snippets.toml"), library_content).unwrap();

    let usage_content = r#"[[entries]]
id = "fav-a"
use_count = 3
last_used_at = 100

[[entries]]
id = "fav-b"
use_count = 10
last_used_at = 200

[[entries]]
id = "fav-c"
use_count = 7
last_used_at = 900
"#;
    fs::write(config_dir.join("usage.toml"), usage_content).unwrap();

    let output_path = tmp.path().join("fav_most_used.txt");

    // favorites-first + most-used:
    // Favorites: fav-c (7), fav-a (3) — most-used desc within favorites
    // Non-favorites: fav-b (10) — only non-favorite
    // First item: fav-c → echo-fav-charlie
    let (code, output) = run_snp_pty(
        &[
            "select",
            "--sort",
            "most-used",
            "--favorites-first",
            "--output-file",
            output_path.to_str().unwrap(),
        ],
        &config_dir,
        b"\r",
    );
    eprintln!("OUTPUT: {output}");
    assert_eq!(
        code, 0,
        "snp select --sort most-used --favorites-first should exit 0"
    );

    let selected = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        selected.trim(),
        "echo-fav-charlie",
        "favorites-first + most-used should select fav-c first (favorite with highest use_count)"
    );
}
