//! Contract tests for the library manager's mutation gate.
//!
//! Every local mutation must pass
//! `gate_mutation_on_interrupted_transactions()` before it writes, so a
//! pending restore can never be undermined by a write that lands in the
//! config tree it is about to roll back.
//!
//! These tests redirect the process config directory with `ScopedConfigDir`,
//! which is only compiled into `test-support` builds. The target is gated in
//! `Cargo.toml` with `required-features = ["test-support"]`.

use snip_it::ScopedConfigDir;
use snip_it::config::{SyncSettings, derive_sync_state_dir, save_sync_settings};
use tempfile::TempDir;

/// Point the resolved config directory at a private temp dir.
fn scoped_config_dir(tmp: &TempDir) -> ScopedConfigDir {
    let config_home = tmp.path().join(".config");
    std::fs::create_dir_all(&config_home).expect("create config home");
    ScopedConfigDir::new(config_home.join("snp"))
}

/// A journal file that cannot be parsed is classified as corrupt and blocks
/// every local mutation (fail-closed).
fn write_corrupt_journal(config_dir: &std::path::Path) {
    let transaction_dir = config_dir.join(".transaction");
    std::fs::create_dir_all(&transaction_dir).expect("create transaction dir");
    std::fs::write(
        transaction_dir.join("txn-corrupt.toml"),
        "this is not [valid toml",
    )
    .expect("write corrupt journal");
}

/// `save_sync_settings` must refuse to write while a transaction is
/// unresolved, and must not leave a `sync.toml` behind.
#[test]
fn test_save_sync_settings_refuses_while_transaction_journal_is_corrupt() {
    let tmp = TempDir::new().unwrap();
    let _config_dir = scoped_config_dir(&tmp);
    let config_dir = derive_sync_state_dir();
    write_corrupt_journal(&config_dir);

    let settings = SyncSettings::default();
    let err = save_sync_settings(&settings).unwrap_err();
    assert!(
        err.to_string().contains("corrupt transaction journal"),
        "unexpected error: {err}"
    );
    assert!(
        !config_dir.join("sync.toml").exists(),
        "sync.toml must not be written while a transaction is unresolved"
    );
}

/// Control case: with no pending transaction the settings are written.
#[test]
fn test_save_sync_settings_writes_when_no_transaction_is_pending() {
    let tmp = TempDir::new().unwrap();
    let _config_dir = scoped_config_dir(&tmp);
    let config_dir = derive_sync_state_dir();

    let settings = SyncSettings::default();
    save_sync_settings(&settings).unwrap();

    assert!(config_dir.join("sync.toml").exists());
}
