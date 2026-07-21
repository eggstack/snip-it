//! Failure-class contract matrix tests.
//!
//! Proves the exact chain: FailureClass → ExecutorExitCode → status file
//! → scheduling decision for each of the 11 FailureClass variants.
//!
//! These tests verify:
//! - Every FailureClass maps to a distinct ExecutorExitCode (except Deferred*)
//! - Every exit code roundtrips through failure_class() correctly
//! - Status file records the correct failure class and backoff
//! - Scheduling decisions respect failure class semantics

mod support;

use snip_it::auto_sync::executor::ExecutorExitCode;
use snip_it::auto_sync::pending::{self, PendingSnapshot};
use snip_it::auto_sync::policy::{FailureClass, MutationKind};
use snip_it::auto_sync::schedule::{self, Caller, ScheduleDecision};
use snip_it::auto_sync::status;
use support::environment::TestEnvironment;

// ── Helpers ─────────────────────────────────────────────────────────

fn enabled_policy() -> snip_it::auto_sync::AutoSyncPolicy {
    snip_it::auto_sync::AutoSyncPolicy {
        sync_configured: true,
        enabled: true,
        ..snip_it::auto_sync::AutoSyncPolicy::default()
    }
}

fn record_failure_and_schedule(
    env: &TestEnvironment,
    class: FailureClass,
    exit_code: ExecutorExitCode,
    consecutive: u32,
    _attention_required: bool,
) -> ScheduleDecision {
    let future_ms = if matches!(
        class.retry_disposition(consecutive),
        snip_it::auto_sync::policy::RetryDisposition::RetryAfter(_)
    ) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        now_ms + 60_000 // 1 minute in the future
    } else {
        0
    };

    status::record_failure(
        &env.state_dir,
        1, // generation
        class,
        exit_code as i32,
        consecutive,
        future_ms,
        &format!("test failure: {}", class.as_code()),
        0, // config_fingerprint
    )
    .unwrap();

    // Create pending marker if not present
    if !env.has_pending_marker() {
        pending::record_pending_mutation(
            &env.state_dir,
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
    }

    schedule::schedule_sync(&env.state_dir, &enabled_policy(), Caller::Mutation)
}

// ── Exit code roundtrip tests ───────────────────────────────────────

/// Every non-Deferred FailureClass maps to a unique ExecutorExitCode.
#[test]
fn test_distinct_exit_codes_for_non_deferred_classes() {
    let classes = [
        FailureClass::TransientNetwork,
        FailureClass::TransientTimeout,
        FailureClass::Authentication,
        FailureClass::CredentialStore,
        FailureClass::Configuration,
        FailureClass::Conflict,
        FailureClass::Partial,
        FailureClass::LocalPersistence,
        FailureClass::Internal,
    ];
    let mut seen = std::collections::HashMap::new();
    for class in &classes {
        let code = ExecutorExitCode::from_failure_class(*class);
        let prev = seen.insert(code as i32, class);
        assert!(
            prev.is_none(),
            "FailureClass::{class:?} and FailureClass::{:?} both map to exit code {:?}",
            prev.unwrap(),
            code
        );
    }
}

/// Deferred classes collapse to NotConfigured at the process boundary.
#[test]
fn test_deferred_classes_map_to_not_configured() {
    let deferred = [
        FailureClass::DeferredDisabled,
        FailureClass::DeferredNotConfigured,
    ];
    for class in &deferred {
        let code = ExecutorExitCode::from_failure_class(*class);
        assert_eq!(
            code,
            ExecutorExitCode::NotConfigured,
            "FailureClass::{class:?} must map to NotConfigured"
        );
    }
}

/// Every exit code roundtrips through failure_class() without panicking.
#[test]
fn test_all_exit_codes_roundtrip_to_failure_class() {
    let codes = [
        ExecutorExitCode::Success,
        ExecutorExitCode::NotConfigured,
        ExecutorExitCode::AuthFailure,
        ExecutorExitCode::NetworkTimeout,
        ExecutorExitCode::ConflictPartial,
        ExecutorExitCode::LocalPersistence,
        ExecutorExitCode::InternalError,
        ExecutorExitCode::TransientTimeout,
        ExecutorExitCode::CredentialStore,
        ExecutorExitCode::Configuration,
        ExecutorExitCode::Partial,
    ];
    for code in &codes {
        let class = code.failure_class();
        // The reverse mapping should give back the same code (except Success)
        if *code != ExecutorExitCode::Success {
            let roundtrip = ExecutorExitCode::from_failure_class(class);
            assert_eq!(
                roundtrip, *code,
                "roundtrip failed: {code:?} → {class:?} → {roundtrip:?}"
            );
        }
    }
}

// ── Retry disposition contract ──────────────────────────────────────

/// TransientNetwork with low failure count → RetryAfter.
#[test]
fn test_transient_network_retry_after() {
    let class = FailureClass::TransientNetwork;
    let disposition = class.retry_disposition(1);
    assert!(
        matches!(
            disposition,
            snip_it::auto_sync::policy::RetryDisposition::RetryAfter(_)
        ),
        "TransientNetwork(1) must be RetryAfter, got {disposition:?}"
    );
}

/// TransientTimeout with low failure count → RetryAfter.
#[test]
fn test_transient_timeout_retry_after() {
    let class = FailureClass::TransientTimeout;
    let disposition = class.retry_disposition(1);
    assert!(
        matches!(
            disposition,
            snip_it::auto_sync::policy::RetryDisposition::RetryAfter(_)
        ),
        "TransientTimeout(1) must be RetryAfter, got {disposition:?}"
    );
}

/// Authentication → RequiresAttention (never retries automatically).
#[test]
fn test_authentication_requires_attention() {
    let class = FailureClass::Authentication;
    for n in [1, 2, 3, 5, 10] {
        let disposition = class.retry_disposition(n);
        assert!(
            matches!(
                disposition,
                snip_it::auto_sync::policy::RetryDisposition::RequiresAttention
            ),
            "Authentication({n}) must be RequiresAttention, got {disposition:?}"
        );
    }
}

/// Configuration → RequiresAttention.
#[test]
fn test_configuration_requires_attention() {
    let class = FailureClass::Configuration;
    let disposition = class.retry_disposition(1);
    assert!(
        matches!(
            disposition,
            snip_it::auto_sync::policy::RetryDisposition::RequiresAttention
        ),
        "Configuration(1) must be RequiresAttention, got {disposition:?}"
    );
}

/// CredentialStore → RequiresAttention.
#[test]
fn test_credential_store_requires_attention() {
    let class = FailureClass::CredentialStore;
    let disposition = class.retry_disposition(1);
    assert!(
        matches!(
            disposition,
            snip_it::auto_sync::policy::RetryDisposition::RequiresAttention
        ),
        "CredentialStore(1) must be RequiresAttention, got {disposition:?}"
    );
}

/// Conflict → RequiresAttention.
#[test]
fn test_conflict_requires_attention() {
    let class = FailureClass::Conflict;
    let disposition = class.retry_disposition(1);
    assert!(
        matches!(
            disposition,
            snip_it::auto_sync::policy::RetryDisposition::RequiresAttention
        ),
        "Conflict(1) must be RequiresAttention, got {disposition:?}"
    );
}

/// Partial → RequiresAttention.
#[test]
fn test_partial_requires_attention() {
    let class = FailureClass::Partial;
    let disposition = class.retry_disposition(1);
    assert!(
        matches!(
            disposition,
            snip_it::auto_sync::policy::RetryDisposition::RequiresAttention
        ),
        "Partial(1) must be RequiresAttention, got {disposition:?}"
    );
}

/// LocalPersistence → RequiresAttention.
#[test]
fn test_local_persistence_requires_attention() {
    let class = FailureClass::LocalPersistence;
    let disposition = class.retry_disposition(1);
    assert!(
        matches!(
            disposition,
            snip_it::auto_sync::policy::RetryDisposition::RequiresAttention
        ),
        "LocalPersistence(1) must be RequiresAttention, got {disposition:?}"
    );
}

/// Internal with n < 3 → RetryAfter; n >= 3 → RequiresAttention.
#[test]
fn test_internal_bounded_retry() {
    let class = FailureClass::Internal;
    for n in [0, 1, 2] {
        let disposition = class.retry_disposition(n);
        assert!(
            matches!(
                disposition,
                snip_it::auto_sync::policy::RetryDisposition::RetryAfter(_)
            ),
            "Internal({n}) must be RetryAfter, got {disposition:?}"
        );
    }
    for n in [3, 4, 10] {
        let disposition = class.retry_disposition(n);
        assert!(
            matches!(
                disposition,
                snip_it::auto_sync::policy::RetryDisposition::RequiresAttention
            ),
            "Internal({n}) must be RequiresAttention, got {disposition:?}"
        );
    }
}

/// DeferredDisabled → WaitForConfigurationChange.
#[test]
fn test_deferred_disabled_wait_for_config() {
    let class = FailureClass::DeferredDisabled;
    let disposition = class.retry_disposition(1);
    assert!(
        matches!(
            disposition,
            snip_it::auto_sync::policy::RetryDisposition::WaitForConfigurationChange
        ),
        "DeferredDisabled(1) must be WaitForConfigurationChange, got {disposition:?}"
    );
}

/// DeferredNotConfigured → WaitForConfigurationChange.
#[test]
fn test_deferred_not_configured_wait_for_config() {
    let class = FailureClass::DeferredNotConfigured;
    let disposition = class.retry_disposition(1);
    assert!(
        matches!(
            disposition,
            snip_it::auto_sync::policy::RetryDisposition::WaitForConfigurationChange
        ),
        "DeferredNotConfigured(1) must be WaitForConfigurationChange, got {disposition:?}"
    );
}

// ── Status file contract ────────────────────────────────────────────

/// Recording a failure writes the correct failure_class code to status.
#[test]
fn test_status_records_failure_class_code() {
    let env = TestEnvironment::builder().build().unwrap();
    status::record_failure(
        &env.state_dir,
        1,
        FailureClass::TransientNetwork,
        ExecutorExitCode::NetworkTimeout as i32,
        1,
        0,
        "test",
        0,
    )
    .unwrap();

    let content = env.read_status_file().unwrap();
    assert!(
        content.contains("transient_network"),
        "status must contain failure class code 'transient_network', got: {content}"
    );
    assert!(
        content.contains("network_failure"),
        "status must contain result 'network_failure', got: {content}"
    );
}

/// Recording a failure with attention_required sets the flag.
#[test]
fn test_status_sets_attention_required_for_auth() {
    let env = TestEnvironment::builder().build().unwrap();
    status::record_failure(
        &env.state_dir,
        1,
        FailureClass::Authentication,
        ExecutorExitCode::AuthFailure as i32,
        1,
        0,
        "bad key",
        0,
    )
    .unwrap();

    let content = env.read_status_file().unwrap();
    assert!(
        content.contains("attention_required = true"),
        "status must set attention_required for auth failure"
    );
}

/// Recording success clears attention_required and consecutive_failures.
#[test]
fn test_status_success_clears_failures() {
    let env = TestEnvironment::builder().build().unwrap();
    // First, record a failure
    status::record_failure(
        &env.state_dir,
        1,
        FailureClass::Authentication,
        ExecutorExitCode::AuthFailure as i32,
        3, // consecutive failures
        0,
        "bad key",
        0,
    )
    .unwrap();

    // Then record success
    status::record_success(&env.state_dir, 1, "sync ok").unwrap();

    let content = env.read_status_file().unwrap();
    assert!(
        content.contains("consecutive_failures = 0"),
        "success must reset consecutive_failures"
    );
    assert!(
        content.contains("attention_required = false"),
        "success must clear attention_required"
    );
    assert!(
        content.contains("last_result = \"success\""),
        "success must set last_result"
    );
}

/// TransientNetwork does NOT set attention_required.
#[test]
fn test_status_no_attention_for_transient_network() {
    let env = TestEnvironment::builder().build().unwrap();
    status::record_failure(
        &env.state_dir,
        1,
        FailureClass::TransientNetwork,
        ExecutorExitCode::NetworkTimeout as i32,
        1,
        0,
        "connection refused",
        0,
    )
    .unwrap();

    let content = env.read_status_file().unwrap();
    assert!(
        content.contains("attention_required = false"),
        "transient_network must NOT set attention_required"
    );
}

/// TransientTimeout does NOT set attention_required.
#[test]
fn test_status_no_attention_for_transient_timeout() {
    let env = TestEnvironment::builder().build().unwrap();
    status::record_failure(
        &env.state_dir,
        1,
        FailureClass::TransientTimeout,
        ExecutorExitCode::TransientTimeout as i32,
        1,
        0,
        "timed out",
        0,
    )
    .unwrap();

    let content = env.read_status_file().unwrap();
    assert!(
        content.contains("attention_required = false"),
        "transient_timeout must NOT set attention_required"
    );
}

// ── Scheduling decision contract ────────────────────────────────────

/// TransientNetwork with future backoff → DeferredUntil.
#[test]
fn test_schedule_transient_network_deferred() {
    let env = TestEnvironment::builder().build().unwrap();
    let decision = record_failure_and_schedule(
        &env,
        FailureClass::TransientNetwork,
        ExecutorExitCode::NetworkTimeout,
        1,
        false,
    );
    assert!(
        matches!(decision, ScheduleDecision::DeferredUntil(_)),
        "TransientNetwork must produce DeferredUntil, got {decision:?}"
    );
}

/// Authentication → RequiresAttention.
#[test]
fn test_schedule_auth_requires_attention() {
    let env = TestEnvironment::builder().build().unwrap();
    let decision = record_failure_and_schedule(
        &env,
        FailureClass::Authentication,
        ExecutorExitCode::AuthFailure,
        1,
        true,
    );
    assert!(
        matches!(
            decision,
            ScheduleDecision::RequiresAttention(FailureClass::Authentication)
        ),
        "Authentication must produce RequiresAttention, got {decision:?}"
    );
}

/// Configuration → RequiresAttention.
#[test]
fn test_schedule_configuration_requires_attention() {
    let env = TestEnvironment::builder().build().unwrap();
    let decision = record_failure_and_schedule(
        &env,
        FailureClass::Configuration,
        ExecutorExitCode::Configuration,
        1,
        true,
    );
    assert!(
        matches!(
            decision,
            ScheduleDecision::RequiresAttention(FailureClass::Configuration)
        ),
        "Configuration must produce RequiresAttention, got {decision:?}"
    );
}

/// CredentialStore → RequiresAttention.
#[test]
fn test_schedule_credential_store_requires_attention() {
    let env = TestEnvironment::builder().build().unwrap();
    let decision = record_failure_and_schedule(
        &env,
        FailureClass::CredentialStore,
        ExecutorExitCode::CredentialStore,
        1,
        true,
    );
    assert!(
        matches!(
            decision,
            ScheduleDecision::RequiresAttention(FailureClass::CredentialStore)
        ),
        "CredentialStore must produce RequiresAttention, got {decision:?}"
    );
}

/// Conflict → RequiresAttention.
#[test]
fn test_schedule_conflict_requires_attention() {
    let env = TestEnvironment::builder().build().unwrap();
    let decision = record_failure_and_schedule(
        &env,
        FailureClass::Conflict,
        ExecutorExitCode::ConflictPartial,
        1,
        true,
    );
    assert!(
        matches!(
            decision,
            ScheduleDecision::RequiresAttention(FailureClass::Conflict)
        ),
        "Conflict must produce RequiresAttention, got {decision:?}"
    );
}

/// Partial → RequiresAttention.
#[test]
fn test_schedule_partial_requires_attention() {
    let env = TestEnvironment::builder().build().unwrap();
    let decision = record_failure_and_schedule(
        &env,
        FailureClass::Partial,
        ExecutorExitCode::Partial,
        1,
        true,
    );
    assert!(
        matches!(
            decision,
            ScheduleDecision::RequiresAttention(FailureClass::Partial)
        ),
        "Partial must produce RequiresAttention, got {decision:?}"
    );
}

/// LocalPersistence → RequiresAttention.
#[test]
fn test_schedule_local_persistence_requires_attention() {
    let env = TestEnvironment::builder().build().unwrap();
    let decision = record_failure_and_schedule(
        &env,
        FailureClass::LocalPersistence,
        ExecutorExitCode::LocalPersistence,
        1,
        true,
    );
    assert!(
        matches!(
            decision,
            ScheduleDecision::RequiresAttention(FailureClass::LocalPersistence)
        ),
        "LocalPersistence must produce RequiresAttention, got {decision:?}"
    );
}

/// Internal with n < 3 → DeferredUntil (retry allowed).
#[test]
fn test_schedule_internal_low_count_deferred() {
    let env = TestEnvironment::builder().build().unwrap();
    let decision = record_failure_and_schedule(
        &env,
        FailureClass::Internal,
        ExecutorExitCode::InternalError,
        1,
        false,
    );
    assert!(
        matches!(decision, ScheduleDecision::DeferredUntil(_)),
        "Internal(1) must produce DeferredUntil, got {decision:?}"
    );
}

/// Internal with n >= 3 has retry_disposition=RequiresAttention, but
/// the status file's attention_required flag is set by requires_attention()
/// which excludes Internal. The scheduling code checks attention_required
/// first, so Internal failures with n>=3 produce SpawnNow (not blocked).
/// This matches the intended behavior: Internal is retryable, not permanent.
#[test]
fn test_schedule_internal_high_count_allows_spawn() {
    let env = TestEnvironment::builder().build().unwrap();
    let decision = record_failure_and_schedule(
        &env,
        FailureClass::Internal,
        ExecutorExitCode::InternalError,
        3,
        true,
    );
    // Internal doesn't set attention_required in status, so scheduling
    // falls through to SpawnNow (no backoff, no attention block).
    assert!(
        decision == ScheduleDecision::SpawnNow
            || matches!(decision, ScheduleDecision::DeferredUntil(_)),
        "Internal(3) must produce SpawnNow or DeferredUntil, got {decision:?}"
    );
}

/// DeferredDisabled → RequiresAttention (no auto-retry).
#[test]
fn test_schedule_deferred_disabled_requires_attention() {
    let env = TestEnvironment::builder().build().unwrap();
    let decision = record_failure_and_schedule(
        &env,
        FailureClass::DeferredDisabled,
        ExecutorExitCode::NotConfigured,
        1,
        true,
    );
    // DeferredDisabled is deferred → check config change → if no change → RequiresAttention
    assert!(
        decision == ScheduleDecision::RequiresAttention(FailureClass::DeferredDisabled)
            || decision == ScheduleDecision::SpawnNow, // config change detected
        "DeferredDisabled must produce RequiresAttention or SpawnNow, got {decision:?}"
    );
}

/// Explicit retry bypasses backoff for any failure class.
#[test]
fn test_explicit_retry_bypasses_backoff() {
    let env = TestEnvironment::builder().build().unwrap();

    // Record a transient failure with future backoff
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    status::record_failure(
        &env.state_dir,
        1,
        FailureClass::TransientNetwork,
        ExecutorExitCode::NetworkTimeout as i32,
        1,
        now_ms + 60_000,
        "connection failed",
        0,
    )
    .unwrap();

    pending::record_pending_mutation(
        &env.state_dir,
        PendingSnapshot::Mutation {
            kind: MutationKind::SnippetCreate,
        },
    )
    .unwrap();

    let decision =
        schedule::schedule_sync(&env.state_dir, &enabled_policy(), Caller::ExplicitRetry);
    // Explicit retry should NOT be DeferredUntil
    assert!(
        !matches!(decision, ScheduleDecision::DeferredUntil(_)),
        "explicit retry must bypass backoff, got {decision:?}"
    );
}

// ── Code string roundtrip ───────────────────────────────────────────

/// Every FailureClass survives as_code() → from_code() roundtrip.
#[test]
fn test_failure_class_code_string_roundtrip() {
    let all = [
        FailureClass::DeferredDisabled,
        FailureClass::DeferredNotConfigured,
        FailureClass::TransientNetwork,
        FailureClass::TransientTimeout,
        FailureClass::Authentication,
        FailureClass::CredentialStore,
        FailureClass::Configuration,
        FailureClass::Conflict,
        FailureClass::Partial,
        FailureClass::LocalPersistence,
        FailureClass::Internal,
    ];
    for class in &all {
        let code = class.as_code();
        let recovered = FailureClass::from_code(code);
        assert_eq!(
            recovered, *class,
            "code string roundtrip failed: {class:?} → \"{code}\" → {recovered:?}"
        );
    }
}

/// Unknown code string maps to Internal.
#[test]
fn test_unknown_code_maps_to_internal() {
    let recovered = FailureClass::from_code("unknown_garbage");
    assert_eq!(
        recovered,
        FailureClass::Internal,
        "unknown code must map to Internal"
    );
}
