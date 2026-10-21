#![cfg(target_os = "linux")]
//! Regression tests for the server-side findings of the `snip-sync` audit.
//!
//! The lock-owner scenario is exercised through the real `snip-sync stop`
//! binary: a separate child process holds the kernel lock, so the parent
//! never becomes a signal target.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use snip_sync::server_lock::{ServerLock, ServerLockError, server_lock_path};

/// Set on the child process so it knows it is the lock holder.
const LOCK_HOLDER_ENV: &str = "SNIP_SYNC_TEST_LOCK_HOLDER";
/// State directory the child process locks.
const STATE_DIR_ENV: &str = "SNIP_SYNC_TEST_STATE_DIR";
/// Touched by the child once it owns the kernel lock.
const READY_FILE_NAME: &str = "lock-holder.ready";

fn snip_sync_bin() -> &'static str {
    env!("CARGO_BIN_EXE_snip-sync")
}

fn state_dir(root: &Path) -> PathBuf {
    root.join("snip-sync")
}

fn output_text(output: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Child process: acquire the server lock and hold it until terminated.
#[test]
fn lock_holder_child_process_holds_the_server_lock() {
    if std::env::var(LOCK_HOLDER_ENV).is_err() {
        // The parent test drives the scenario; nothing to do here.
        return;
    }
    let state_dir = PathBuf::from(std::env::var(STATE_DIR_ENV).expect("state dir"));
    let deadline = Instant::now() + Duration::from_secs(20);
    let guard = loop {
        match ServerLock::try_acquire(&state_dir) {
            Ok(guard) => break guard,
            Err(ServerLockError::Busy { .. }) => {
                assert!(
                    Instant::now() < deadline,
                    "could not acquire the server lock"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => panic!("unexpected server lock error: {error}"),
        }
    };
    fs::write(state_dir.join(READY_FILE_NAME), b"ready").unwrap();
    let _guard = guard;
    loop {
        std::thread::sleep(Duration::from_millis(200));
    }
}

struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Spawn the lock holder and wait until it owns the kernel lock.
fn spawn_lock_holder(state_dir: &Path) -> (ChildGuard, u32) {
    let exe = std::env::current_exe().expect("test binary path");
    let child = Command::new(exe)
        .arg("--exact")
        .arg("lock_holder_child_process_holds_the_server_lock")
        .arg("--nocapture")
        .env(LOCK_HOLDER_ENV, "1")
        .env(STATE_DIR_ENV, state_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the lock holder");
    let pid = child.id();
    let guard = ChildGuard(Some(child));

    let ready = state_dir.join(READY_FILE_NAME);
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready.exists() {
        assert!(
            Instant::now() < deadline,
            "the lock holder never acquired the server lock"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    (guard, pid)
}

/// `snip-sync stop` must be able to act on the lock owner even when the
/// in-place owner record inside the lock file is observed empty (the record
/// is rewritten in place, so a concurrent reader can catch it mid-write).
#[test]
fn stop_identifies_the_lock_owner_when_the_lock_file_record_is_torn() {
    let root = tempfile::tempdir().unwrap();
    let dir = state_dir(root.path());
    fs::create_dir_all(&dir).unwrap();

    let (_holder, owner_pid) = spawn_lock_holder(&dir);

    // Simulate the in-place record being observed while it is rewritten.
    fs::write(server_lock_path(&dir), "").unwrap();

    let output = Command::new(snip_sync_bin())
        .env("SNIP_SYNC_STATE_DIR", &dir)
        .env("HOME", root.path())
        .env("XDG_STATE_HOME", root.path())
        .args(["stop"])
        .output()
        .unwrap();
    let (stdout, stderr) = output_text(&output);

    assert!(
        !stderr.contains("no usable owner metadata"),
        "stop dead-ended on missing owner metadata; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains(&owner_pid.to_string()) || stderr.contains(&owner_pid.to_string()),
        "stop did not identify the lock owner PID {owner_pid}; stdout={stdout} stderr={stderr}"
    );
}

/// The owner record published beside the lock file is what makes the busy
/// path actionable; it must survive a truncated in-place record.
#[test]
fn busy_lock_reports_the_owner_after_the_in_place_record_is_torn() {
    let root = tempfile::tempdir().unwrap();
    let dir = state_dir(root.path());
    fs::create_dir_all(&dir).unwrap();

    let (_holder, owner_pid) = spawn_lock_holder(&dir);
    fs::write(server_lock_path(&dir), "").unwrap();

    match ServerLock::try_acquire(&dir) {
        Err(ServerLockError::Busy { owner }) => {
            let owner = owner.expect("the busy path must resolve an owner");
            assert_eq!(owner.pid, owner_pid);
        }
        other => panic!("expected Busy, got {other:?}"),
    }
}
