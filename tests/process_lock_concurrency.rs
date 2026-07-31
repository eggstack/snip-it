//! Cross-process concurrency tests for the kernel-backed process file lock.
//!
//! Spawns real subprocesses (the `process_lock_helper` binary) that all
//! attempt to acquire the same lock file at the same time. Verifies the
//! kernel-backed mutual exclusion works as expected:
//!
//! - Exactly one contender wins a simultaneous race.
//! - A killed owner releases through kernel process teardown.
//! - Repeated acquire/release cycles leave only the canonical lock file.

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use snip_it::process_file_lock;

fn helper_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_process_lock_helper"))
}

fn spawn_contender(label: &str, lock_path: &Path, barrier: &Path) -> std::process::Child {
    Command::new(helper_bin())
        .arg(label)
        .arg("wait_acquire")
        .env("SNP_TEST_LOCK_PATH", lock_path)
        .env("SNP_TEST_LOCK_BARRIER_DIR", barrier)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn helper")
}

fn wait_for_outcome(barrier: &Path, label: &str, timeout: Duration) -> String {
    let path = barrier.join(format!("{label}.outcome"));
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Ok(content) = std::fs::read_to_string(&path) {
            return content.trim().to_string();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("outcome file never appeared: {path:?}");
}

fn setup_barrier(label: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::TempDir::with_prefix(label).unwrap();
    let barrier = dir.path().to_path_buf();
    (dir, barrier)
}

#[test]
fn exactly_one_of_eight_contenders_acquires() {
    let lock_dir = tempfile::TempDir::new().unwrap();
    let lock_path = lock_dir.path().join("k.lock");
    let (_barrier_dir, barrier) = setup_barrier("eight");

    let contenders = 8;
    let mut children = Vec::with_capacity(contenders);
    for i in 0..contenders {
        children.push(spawn_contender(&format!("c{i}"), &lock_path, &barrier));
    }

    // Give the children time to block on the barrier.
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(barrier.join("release"), "").unwrap();

    // Collect every contender's outcome before killing anyone. The
    // helper's wait_acquire timeout is intentionally short so the
    // seven non-winners time out without holding any lock. Killing the
    // single winner afterwards releases the kernel lock, but no other
    // contender is still racing for it.
    let mut outcomes: Vec<String> = Vec::with_capacity(contenders);
    for i in 0..contenders {
        outcomes.push(wait_for_outcome(
            &barrier,
            &format!("c{i}"),
            Duration::from_secs(5),
        ));
    }
    for child in &mut children {
        let _ = child.kill();
        let _ = child.wait();
    }

    let mut acquired = 0;
    let mut busy_or_timeout = 0;
    let mut other = Vec::new();
    for outcome in outcomes {
        match outcome.as_str() {
            "ACQUIRED" => acquired += 1,
            "BUSY" | "TIMEOUT" => busy_or_timeout += 1,
            other_outcome => other.push(other_outcome.to_string()),
        }
    }

    assert!(
        other.is_empty(),
        "unexpected outcomes from contenders: {other:?}"
    );
    assert_eq!(acquired, 1, "exactly one of {contenders} must acquire");
    assert_eq!(
        busy_or_timeout,
        contenders - 1,
        "every other contender must report BUSY or TIMEOUT"
    );
}

#[test]
fn killed_owner_releases_for_next_acquirer() {
    let lock_dir = tempfile::TempDir::new().unwrap();
    let lock_path = lock_dir.path().join("k.lock");

    // Child acquires lock and signals ACQUIRED.
    let (_barrier_dir, barrier) = setup_barrier("kill");
    let mut child = spawn_contender("victim", &lock_path, &barrier);
    std::thread::sleep(Duration::from_millis(150));
    std::fs::write(barrier.join("release"), "").unwrap();

    let outcome = wait_for_outcome(&barrier, "victim", Duration::from_secs(10));
    assert_eq!(outcome, "ACQUIRED");

    // Kill the child without a graceful drop.
    let _ = child.kill();
    let _ = child.wait();

    // The kernel releases the lock when the file descriptor closes.
    // Give the kernel a moment.
    std::thread::sleep(Duration::from_millis(200));

    // A new acquirer succeeds without any quarantine / cleanup.
    let guard = process_file_lock::try_acquire(&lock_path, "next-acquirer").unwrap();
    drop(guard);
}

#[test]
fn repeated_acquire_release_cycles_leave_no_quarantine() {
    let lock_dir = tempfile::TempDir::new().unwrap();
    let lock_path = lock_dir.path().join("k.lock");

    for i in 0..100 {
        let guard = process_file_lock::try_acquire(&lock_path, &format!("cycle-{i}")).unwrap();
        drop(guard);
    }

    // After 100 cycles, only the canonical lock file should remain.
    let entries: Vec<_> = std::fs::read_dir(lock_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "only the canonical lock file may remain; found {} entries",
        entries.len()
    );
    let quarantines: Vec<_> = entries
        .iter()
        .filter(|e| e.file_name().to_string_lossy().contains(".quarantine."))
        .collect();
    assert!(quarantines.is_empty(), "no .quarantine.* files may remain");
}

#[test]
fn second_acquirer_succeeds_after_release() {
    let lock_dir = tempfile::TempDir::new().unwrap();
    let lock_path = lock_dir.path().join("k.lock");
    let (_barrier_dir, barrier) = setup_barrier("sequential");

    // First acquirer.
    let mut first = spawn_contender("first", &lock_path, &barrier);
    std::thread::sleep(Duration::from_millis(150));
    std::fs::write(barrier.join("release"), "").unwrap();
    let outcome = wait_for_outcome(&barrier, "first", Duration::from_secs(10));
    assert_eq!(outcome, "ACQUIRED");

    // Tell the first to drop.
    std::fs::write(barrier.join("drop"), "").unwrap();
    let _ = first.wait();

    // Second acquirer wins.
    let mut second = spawn_contender("second", &lock_path, &barrier);
    std::thread::sleep(Duration::from_millis(150));
    std::fs::write(barrier.join("release"), "").unwrap();
    let outcome = wait_for_outcome(&barrier, "second", Duration::from_secs(10));
    assert_eq!(outcome, "ACQUIRED");

    std::fs::write(barrier.join("drop"), "").unwrap();
    let _ = second.kill();
    let _ = second.wait();
}
