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
