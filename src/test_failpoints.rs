//! **Layer: Test-only**
//!
//! Process-crash failpoints and error injection seams for adversarial
//! transaction testing.
//!
//! When the `test-support` feature is enabled, these seams check environment
//! variables to trigger controlled failures. When the feature is disabled
//! (production builds), every function is a compile-time no-op — the
//! environment variable checks are entirely absent from the binary.
//!
//! Tests launch the real `snp restore` binary with one failpoint active,
//! confirm the process terminated at the expected boundary, then launch
//! a second command to verify recovery.

/// Check whether a test-only failpoint is active and abort if so.
///
/// If `SNP_TEST_FAILPOINT` equals `name`, the process aborts immediately.
/// Only compiled with the `test-support` feature; production builds are
/// a compile-time no-op.
#[cfg(feature = "test-support")]
pub fn maybe_failpoint(name: &str) {
    if std::env::var("SNP_TEST_FAILPOINT").as_deref().ok() == Some(name) {
        tracing::error!(failpoint = name, "test failpoint triggered: aborting");
        std::process::abort();
    }
}

/// Production no-op for failpoint checks.
#[cfg(not(feature = "test-support"))]
#[inline(always)]
pub fn maybe_failpoint(_name: &str) {}

/// Inject a recoverable error for testing rollback and recovery paths.
///
/// When the `test-support` feature is enabled and `SNP_TEST_INJECT_ERROR`
/// equals `name`, returns an `Err`. Production builds are a compile-time
/// no-op.
#[cfg(feature = "test-support")]
#[allow(dead_code)]
pub fn maybe_injected_error(name: &str) -> crate::error::SnipResult<()> {
    if std::env::var("SNP_TEST_INJECT_ERROR").as_deref().ok() == Some(name) {
        return Err(crate::error::SnipError::runtime_error(
            "Injected test failure",
            Some(name),
        ));
    }
    Ok(())
}

/// Production no-op for injected error checks.
#[cfg(not(feature = "test-support"))]
#[inline(always)]
#[allow(dead_code)]
pub fn maybe_injected_error(_name: &str) -> crate::error::SnipResult<()> {
    Ok(())
}

/// Test-only mutation barrier for barrier-controlled concurrency tests.
///
/// When the `test-support` feature is enabled and `SNP_TEST_MUTATION_BARRIER_DIR`
/// is set, checks if the barrier point matches and blocks until released.
/// Production builds are a compile-time no-op.
#[cfg(feature = "test-support")]
#[allow(dead_code)]
pub fn mutation_barrier(point: &str) {
    let Ok(root) = std::env::var("SNP_TEST_MUTATION_BARRIER_DIR") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let expected = root.join("point");
    if std::fs::read_to_string(&expected).ok().as_deref() != Some(point) {
        return;
    }
    let _ = std::fs::write(root.join("entered"), point);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !root.join("release").exists() {
        if std::time::Instant::now() > deadline {
            tracing::error!("mutation barrier timed out at point: {}", point);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Production no-op for mutation barriers.
#[cfg(not(feature = "test-support"))]
#[inline(always)]
#[allow(dead_code)]
pub fn mutation_barrier(_point: &str) {}

/// Failpoint names used by the restore crash tests.
///
/// These are stable identifiers — tests reference them by name.
pub mod failpoints {
    pub const RESTORE_AFTER_PREPARED: &str = "restore-after-prepared";
    pub const RESTORE_AFTER_BACKUPS_DURABLE: &str = "restore-after-backups-durable";
    pub const RESTORE_AFTER_FIRST_INSTALL: &str = "restore-after-first-install";
    pub const RESTORE_AFTER_INDEX_INSTALL: &str = "restore-after-index-install";
    pub const RESTORE_AFTER_ALL_INSTALLS: &str = "restore-after-all-installs";
    pub const RESTORE_AFTER_COMMITTED_LOCAL_BEFORE_PENDING: &str =
        "restore-after-committed-local-before-pending";
    pub const RESTORE_AFTER_PENDING_BEFORE_JOURNAL_UPDATE: &str =
        "restore-after-pending-before-journal-update";
    pub const RESTORE_AFTER_JOURNAL_PENDING_BEFORE_CLEANUP: &str =
        "restore-after-journal-pending-before-cleanup";
    pub const RESTORE_DURING_FIRST_ROLLBACK: &str = "restore-during-first-rollback";
    pub const RESTORE_DURING_SECOND_ROLLBACK: &str = "restore-during-second-rollback";
    pub const CLEANUP_DURING_STAGED_REMOVAL: &str = "cleanup-during-staged-removal";
    pub const CLEANUP_DURING_DIR_REMOVAL: &str = "cleanup-during-dir-removal";
}
