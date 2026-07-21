mod support;

use snip_it::auto_sync::execution_lock;
use snip_it::auto_sync::pending::{self, PendingSnapshot};
use snip_it::auto_sync::policy::MutationKind;
use snip_it::auto_sync::schedule::{self, Caller, ScheduleDecision};
use tempfile::TempDir;

fn enabled_policy() -> snip_it::auto_sync::AutoSyncPolicy {
    snip_it::auto_sync::AutoSyncPolicy {
        sync_configured: true,
        enabled: true,
        ..snip_it::auto_sync::AutoSyncPolicy::default()
    }
}

#[test]
fn test_sequential_lock_acquisition() {
    let dir = TempDir::new().unwrap();
    let lock1 = execution_lock::try_acquire(dir.path()).unwrap();
    drop(lock1);
    let lock2 = execution_lock::try_acquire(dir.path()).unwrap();
    drop(lock2);
}

#[test]
fn test_concurrent_lock_acquisition_blocked() {
    let dir = TempDir::new().unwrap();
    let _lock1 = execution_lock::try_acquire(dir.path()).unwrap();
    let result = execution_lock::try_acquire(dir.path());
    assert!(result.is_err());
}

#[test]
fn test_execution_lock_survives_across_functions() {
    let dir = TempDir::new().unwrap();
    {
        let _lock = execution_lock::try_acquire(dir.path()).unwrap();
        let result = execution_lock::try_acquire(dir.path());
        assert!(result.is_err());
    }
    let lock2 = execution_lock::try_acquire(dir.path()).unwrap();
    drop(lock2);
}

#[test]
fn test_schedule_already_active_when_lock_held() {
    let dir = TempDir::new().unwrap();
    pending::record_pending_mutation(
        dir.path(),
        PendingSnapshot::Mutation {
            kind: MutationKind::SnippetCreate,
        },
    )
    .unwrap();
    let _lock = execution_lock::try_acquire(dir.path()).unwrap();
    let decision = schedule::schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation);
    assert_eq!(decision, ScheduleDecision::AlreadyActive);
}

#[test]
fn test_schedule_spawn_now_when_no_lock() {
    let dir = TempDir::new().unwrap();
    pending::record_pending_mutation(
        dir.path(),
        PendingSnapshot::Mutation {
            kind: MutationKind::SnippetCreate,
        },
    )
    .unwrap();
    let decision = schedule::schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation);
    assert!(decision == ScheduleDecision::SpawnNow || decision == ScheduleDecision::AlreadyActive);
}

#[test]
fn test_20_mutations_during_lock_all_already_active() {
    let dir = TempDir::new().unwrap();
    pending::record_pending_mutation(
        dir.path(),
        PendingSnapshot::Mutation {
            kind: MutationKind::SnippetCreate,
        },
    )
    .unwrap();
    let _lock = execution_lock::try_acquire(dir.path()).unwrap();
    for i in 0..20 {
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let decision = schedule::schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation);
        assert_eq!(
            decision,
            ScheduleDecision::AlreadyActive,
            "mutation {i} while lock held must be AlreadyActive"
        );
    }
}

#[test]
fn test_sequential_mutations_preserve_all_generations() {
    let dir = TempDir::new().unwrap();
    for i in 0..10 {
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: if i % 2 == 0 {
                    MutationKind::SnippetCreate
                } else {
                    MutationKind::SnippetUpdate
                },
            },
        )
        .unwrap();
    }
    let state = pending::read_state_from_dir(dir.path()).unwrap();
    assert_eq!(
        state.generation, 10,
        "10 sequential mutations must produce generation 10"
    );
}

#[test]
fn test_lock_released_after_drop() {
    let dir = TempDir::new().unwrap();
    let lock = execution_lock::try_acquire(dir.path()).unwrap();
    let inspect1 = execution_lock::inspect(&execution_lock::execution_lock_path(dir.path()));
    assert!(inspect1.is_some());
    drop(lock);
    let inspect2 = execution_lock::inspect(&execution_lock::execution_lock_path(dir.path()));
    assert!(inspect2.is_none());
}

#[test]
fn test_stale_lock_with_dead_pid_is_recovered() {
    let dir = TempDir::new().unwrap();
    let lock_path = execution_lock::execution_lock_path(dir.path());
    #[derive(serde::Serialize)]
    struct FakeLock {
        pid: u32,
        started_at_unix_ms: u64,
        nonce: String,
    }
    let fake = FakeLock {
        pid: 999999999,
        started_at_unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
        nonce: "dead".to_string(),
    };
    std::fs::write(&lock_path, toml::to_string_pretty(&fake).unwrap()).unwrap();
    let result = execution_lock::try_acquire(dir.path());
    assert!(result.is_ok());
}
