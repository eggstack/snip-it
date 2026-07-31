//! Test helper binary for cross-process process_file_lock concurrency.
//!
//! Enabled only under the `test-support` feature so production builds do
//! not contain this code path. Invoked from
//! `tests/process_lock_concurrency.rs` to verify mutual exclusion across
//! real subprocess boundaries.
//!
//! Usage:
//!   SNP_TEST_LOCK_PATH=<path> SNP_TEST_LOCK_BARRIER_DIR=<dir> \
//!     process_lock_helper wait_acquire <label>
//!
//! Steps:
//!   1. Wait until `<barrier>/release` exists.
//!   2. Attempt to acquire the kernel-backed lock at `<lock_path>`.
//!   3. Write one of three outcomes to `<barrier>/<label>.outcome`:
//!      - ACQUIRED — kernel lock acquired
//!      - BUSY     — kernel lock held by another process
//!      - TIMEOUT  — wait_acquire exhausted its deadline
//!   4. If ACQUIRED, hold the lock until `<barrier>/drop` exists.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use snip_it::process_file_lock::{self, ProcessFileLockError};

fn outcome_path(label: &str) -> PathBuf {
    let barrier = std::env::var("SNP_TEST_LOCK_BARRIER_DIR").expect("barrier dir");
    PathBuf::from(barrier).join(format!("{label}.outcome"))
}

fn lock_path() -> PathBuf {
    PathBuf::from(std::env::var("SNP_TEST_LOCK_PATH").expect("lock path"))
}

fn wait_for_release() {
    let barrier = std::env::var("SNP_TEST_LOCK_BARRIER_DIR").expect("barrier dir");
    let release = PathBuf::from(&barrier).join("release");
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !release.exists() {
        if std::time::Instant::now() >= deadline {
            panic!("barrier release file never appeared");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn record(label: &str, what: &str) {
    let path = outcome_path(label);
    std::fs::write(&path, what).expect("write outcome");
}

fn wait_for_drop() {
    let barrier = std::env::var("SNP_TEST_LOCK_BARRIER_DIR").expect("barrier dir");
    let drop = PathBuf::from(&barrier).join("drop");
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !drop.exists() {
        if std::time::Instant::now() >= deadline {
            panic!("drop file never appeared");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn run_wait_acquire(label: &str) {
    wait_for_release();
    // 200 ms is long enough for one contender to win the kernel lock
    // and short enough that all other contenders see a Busy kernel
    // lock and time out before the first acquirer is killed.
    match process_file_lock::wait_acquire(&lock_path(), "test-helper", Duration::from_millis(200)) {
        Ok(_guard) => {
            record(label, "ACQUIRED");
            let mut stdout = std::io::stdout();
            let _ = writeln!(stdout, "ACQUIRED");
            let _ = stdout.flush();
            wait_for_drop();
        }
        Err(ProcessFileLockError::Timeout { .. }) => {
            record(label, "TIMEOUT");
        }
        Err(ProcessFileLockError::Busy { .. }) => {
            record(label, "BUSY");
        }
        Err(e) => {
            record(label, &format!("ERROR:{e}"));
        }
    }
}

fn main() {
    let label = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "child".to_string());
    let mode = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "wait_acquire".to_string());
    match mode.as_str() {
        "wait_acquire" => run_wait_acquire(&label),
        other => record(&label, &format!("UNKNOWN_MODE:{other}")),
    }
}
