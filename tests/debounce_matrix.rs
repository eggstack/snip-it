mod support;

use snip_it::auto_sync::execution_lock::try_acquire;
use snip_it::auto_sync::pending::{self, PendingSnapshot};
use snip_it::auto_sync::policy::{AutoSyncPolicy, FailureClass, MutationKind};
use snip_it::auto_sync::schedule::{Caller, ScheduleDecision, schedule_sync};
use snip_it::auto_sync::status;
use snip_it::auto_sync::worker::{Clock, DebounceResult, debounce};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct MockClock {
    instant: Mutex<Instant>,
    unix_ms: Mutex<u64>,
}

impl MockClock {
    fn new(start_instant: Instant, start_unix_ms: u64) -> Self {
        Self {
            instant: Mutex::new(start_instant),
            unix_ms: Mutex::new(start_unix_ms),
        }
    }

    fn advance(&self, duration: Duration) {
        let mut inst = self.instant.lock().unwrap();
        *inst += duration;
        let mut ms = self.unix_ms.lock().unwrap();
        *ms += duration.as_millis() as u64;
    }
}

impl Clock for MockClock {
    fn now_instant(&self) -> Instant {
        *self.instant.lock().unwrap()
    }
    fn now_unix_ms(&self) -> u64 {
        *self.unix_ms.lock().unwrap()
    }
    fn sleep(&self, duration: Duration) {
        self.advance(duration);
    }
}

fn enabled_policy() -> AutoSyncPolicy {
    AutoSyncPolicy {
        sync_configured: true,
        enabled: true,
        ..AutoSyncPolicy::default()
    }
}

fn create_mutation(state_dir: &std::path::Path) -> snip_it::auto_sync::PendingState {
    pending::record_pending_mutation(
        state_dir,
        PendingSnapshot::Mutation {
            kind: MutationKind::SnippetCreate,
        },
    )
    .unwrap()
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn mock_clock_now() -> (Instant, u64) {
    let real_ms = unix_now_ms();
    (Instant::now(), real_ms)
}

#[test]
fn test_zero_debounce_one_mutation_one_attempt() {
    let dir = TempDir::new().unwrap();
    let observed = create_mutation(dir.path());
    let (instant, unix_ms) = mock_clock_now();
    let clock = MockClock::new(instant, unix_ms);
    let start = clock.now_instant();
    let max_lifetime = Duration::from_secs(300);
    let max_delay = Duration::from_secs(300);
    let deadline = start;

    let result = debounce(
        dir.path(),
        observed.clone(),
        deadline,
        start,
        max_lifetime,
        max_delay,
        Duration::ZERO,
        &clock,
    );

    match result {
        DebounceResult::Ready(state) => {
            assert_eq!(state.generation, 1);
        }
        other => panic!("expected Ready(gen=1), got {other:?}"),
    }
}

#[test]
fn test_positive_debounce_coalesces_mutations() {
    let dir = TempDir::new().unwrap();
    let observed = create_mutation(dir.path());

    for i in 0..4 {
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

    let latest = pending::read_state_from_dir(dir.path()).unwrap();
    assert_eq!(latest.generation, 5);

    let (instant, unix_ms) = mock_clock_now();
    let clock = MockClock::new(instant, unix_ms);
    let start = clock.now_instant();
    let max_lifetime = Duration::from_secs(300);
    let max_delay = Duration::from_secs(600);
    let deadline = start + Duration::from_secs(2);

    let result = debounce(
        dir.path(),
        observed,
        deadline,
        start,
        max_lifetime,
        max_delay,
        Duration::from_secs(2),
        &clock,
    );

    match result {
        DebounceResult::Ready(state) => {
            assert_eq!(state.generation, 5);
        }
        other => panic!("expected Ready(gen=5), got {other:?}"),
    }
}

#[test]
fn test_max_delay_forces_attempt() {
    let dir = TempDir::new().unwrap();
    let observed = create_mutation(dir.path());
    let (instant, unix_ms) = mock_clock_now();
    let clock = MockClock::new(instant, unix_ms);
    let start = clock.now_instant();
    let max_lifetime = Duration::from_secs(300);
    let max_delay = Duration::ZERO;
    let deadline = start + Duration::from_secs(60);

    let result = debounce(
        dir.path(),
        observed,
        deadline,
        start,
        max_lifetime,
        max_delay,
        Duration::from_secs(2),
        &clock,
    );

    match result {
        DebounceResult::DeferredMaximumLifetime(state) => {
            assert_eq!(state.generation, 1);
        }
        other => panic!("expected DeferredMaximumLifetime(gen=1), got {other:?}"),
    }
}

#[test]
fn test_marker_removed_during_debounce_cancels() {
    let dir = TempDir::new().unwrap();
    let observed = create_mutation(dir.path());
    let (instant, unix_ms) = mock_clock_now();
    let clock = MockClock::new(instant, unix_ms);
    let start = clock.now_instant();
    let max_lifetime = Duration::from_secs(300);
    let max_delay = Duration::from_secs(300);
    let deadline = start - Duration::from_secs(10);

    pending::clear(dir.path()).unwrap();

    let result = debounce(
        dir.path(),
        observed,
        deadline,
        start,
        max_lifetime,
        max_delay,
        Duration::from_secs(2),
        &clock,
    );

    assert!(matches!(result, DebounceResult::CancelledMarkerRemoved));
}

#[test]
fn test_mutation_during_debounce_promotes_generation() {
    let dir = TempDir::new().unwrap();
    let observed = create_mutation(dir.path());
    assert_eq!(observed.generation, 1);

    let (instant, unix_ms) = mock_clock_now();
    let clock = MockClock::new(instant, unix_ms);
    let start = clock.now_instant();
    let max_lifetime = Duration::from_secs(300);
    let max_delay = Duration::from_secs(300);
    let deadline = start + Duration::from_secs(60);

    pending::record_pending_mutation(
        dir.path(),
        PendingSnapshot::Mutation {
            kind: MutationKind::SnippetUpdate,
        },
    )
    .unwrap();

    let result = debounce(
        dir.path(),
        observed,
        deadline,
        start,
        max_lifetime,
        max_delay,
        Duration::from_secs(2),
        &clock,
    );

    match result {
        DebounceResult::Ready(state) => {
            assert_eq!(state.generation, 2);
        }
        other => panic!("expected Ready(gen=2), got {other:?}"),
    }
}

#[test]
fn test_debounce_returns_final_generation_not_initial() {
    let dir = TempDir::new().unwrap();
    create_mutation(dir.path());
    let initial = pending::read_state_from_dir(dir.path()).unwrap();
    assert_eq!(initial.generation, 1);

    for i in 0..9 {
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

    let (instant, unix_ms) = mock_clock_now();
    let clock = MockClock::new(instant, unix_ms);
    let start = clock.now_instant();
    let max_lifetime = Duration::from_secs(300);
    let max_delay = Duration::from_secs(600);
    let deadline = start + Duration::from_secs(60);

    let result = debounce(
        dir.path(),
        initial,
        deadline,
        start,
        max_lifetime,
        max_delay,
        Duration::from_secs(2),
        &clock,
    );

    match result {
        DebounceResult::Ready(state) => {
            assert_eq!(state.generation, 10);
        }
        other => panic!("expected Ready(gen=10), got {other:?}"),
    }
}

#[test]
fn test_startup_recovery_schedules_worker() {
    let dir = TempDir::new().unwrap();
    create_mutation(dir.path());

    let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::StartupRecovery);
    assert_eq!(
        decision,
        ScheduleDecision::SpawnNow,
        "startup with pending must produce SpawnNow, got {decision:?}"
    );
}

#[test]
fn test_startup_recovery_skips_when_lock_held() {
    let dir = TempDir::new().unwrap();
    create_mutation(dir.path());
    let _lock = try_acquire(dir.path()).unwrap();

    let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::StartupRecovery);
    assert_eq!(
        decision,
        ScheduleDecision::AlreadyActive,
        "startup with pending + lock held must produce AlreadyActive, got {decision:?}"
    );
}

#[test]
fn test_startup_recovery_preserves_stale_pending() {
    let dir = TempDir::new().unwrap();
    create_mutation(dir.path());

    let state = pending::read_state_from_dir(dir.path()).unwrap();
    assert_eq!(state.generation, 1);

    let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::StartupRecovery);
    assert!(
        decision == ScheduleDecision::SpawnNow || decision == ScheduleDecision::AlreadyActive,
        "pending marker must still be scheduled regardless of age, got {decision:?}"
    );
}

#[test]
fn test_backoff_active_defers_scheduling() {
    let dir = TempDir::new().unwrap();
    create_mutation(dir.path());

    let future_ms = unix_now_ms() + 60_000;
    status::record_failure(
        dir.path(),
        1,
        FailureClass::TransientNetwork,
        4,
        1,
        future_ms,
        "connection failed",
        0,
    )
    .unwrap();

    let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation);
    assert!(
        matches!(decision, ScheduleDecision::DeferredUntil(_)),
        "active backoff must produce DeferredUntil, got {decision:?}"
    );
}

#[test]
fn test_backoff_expired_allows_spawn() {
    let dir = TempDir::new().unwrap();
    create_mutation(dir.path());

    let past_ms = unix_now_ms().saturating_sub(60_000);
    status::record_failure(
        dir.path(),
        1,
        FailureClass::TransientNetwork,
        4,
        1,
        past_ms,
        "connection failed",
        0,
    )
    .unwrap();

    let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation);
    assert_eq!(
        decision,
        ScheduleDecision::SpawnNow,
        "expired backoff must produce SpawnNow, got {decision:?}"
    );
}

#[test]
fn test_attention_required_blocks_scheduling() {
    let dir = TempDir::new().unwrap();
    create_mutation(dir.path());

    status::record_failure(
        dir.path(),
        1,
        FailureClass::Authentication,
        3,
        1,
        0,
        "bad api key",
        0,
    )
    .unwrap();

    let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation);
    assert!(
        matches!(
            decision,
            ScheduleDecision::RequiresAttention(FailureClass::Authentication)
        ),
        "auth failure must produce RequiresAttention(Authentication), got {decision:?}"
    );
}
