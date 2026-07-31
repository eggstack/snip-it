//! Lifecycle contracts for the single detached auto-sync helper.

use snip_it::auto_sync::worker::WorkerOutcome;
use tempfile::TempDir;

#[test]
fn helper_without_pending_exits_without_work() {
    let dir = TempDir::new().unwrap();
    assert_eq!(
        snip_it::auto_sync::worker::run(dir.path()),
        WorkerOutcome::NothingToDo
    );
}
