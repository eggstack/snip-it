mod support;

use std::time::Duration;

use snip_it::auto_sync::executor::ExecutorExitCode;
use snip_it::auto_sync::policy::FailureClass;
use snip_it::auto_sync::worker::WorkerOutcome;
use tempfile::TempDir;

#[test]
fn test_worker_nothing_to_do_without_pending() {
    let dir = TempDir::new().unwrap();
    let outcome = snip_it::auto_sync::worker::run(dir.path());
    assert_eq!(outcome, WorkerOutcome::NothingToDo);
}

#[test]
fn test_terminate_child_reap() {
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let status = child.wait().unwrap();
    assert!(!status.success());
}

#[test]
fn test_force_kill_child_reap() {
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    unsafe {
        libc::kill(child.id() as i32, libc::SIGKILL);
    }
    let status = child.wait().unwrap();
    assert!(!status.success());
}

#[test]
fn test_child_exits_before_deadline() {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break None,
        }
    };
    assert!(result.is_some());
    assert!(result.unwrap().success());
}

#[test]
fn test_child_timeout_returns_none() {
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_millis(200);
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break None,
        }
    };
    assert!(result.is_none());
    unsafe {
        libc::kill(child.id() as i32, libc::SIGKILL);
    }
    let _ = child.wait();
}

#[test]
fn test_executor_exit_code_success_roundtrip() {
    let raw = ExecutorExitCode::Success.to_exit_status();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("exit {raw}"))
        .status()
        .unwrap();
    assert_eq!(
        ExecutorExitCode::from_exit_status(status),
        ExecutorExitCode::Success
    );
}

#[test]
fn test_executor_exit_code_all_values_roundtrip() {
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
        let raw = code.to_exit_status();
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("exit {raw}"))
            .status()
            .unwrap();
        let reconstructed = ExecutorExitCode::from_exit_status(status);
        assert_eq!(
            *code, reconstructed,
            "roundtrip failed for {code:?} (raw={raw})"
        );
    }
}

#[test]
fn test_executor_exit_code_signal_death_maps_to_internal() {
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    unsafe {
        libc::kill(child.id() as i32, libc::SIGKILL);
    }
    let status = child.wait().unwrap();
    assert_eq!(
        ExecutorExitCode::from_exit_status(status),
        ExecutorExitCode::InternalError,
        "signal death must map to InternalError"
    );
}

#[test]
fn test_failure_class_backoff_progression() {
    let class = FailureClass::TransientNetwork;
    let d0 = class.retry_disposition(0);
    let d1 = class.retry_disposition(1);
    let d2 = class.retry_disposition(2);
    let d3 = class.retry_disposition(3);
    for d in [&d0, &d1, &d2, &d3] {
        assert!(
            matches!(
                d,
                snip_it::auto_sync::policy::RetryDisposition::RetryAfter(_)
            ),
            "TransientNetwork must be RetryAfter, got {d:?}"
        );
    }
}

#[test]
fn test_failure_class_is_deferred() {
    assert!(FailureClass::DeferredDisabled.is_deferred());
    assert!(FailureClass::DeferredNotConfigured.is_deferred());
    assert!(FailureClass::Configuration.is_deferred());
    assert!(FailureClass::Authentication.is_deferred());
    assert!(FailureClass::CredentialStore.is_deferred());
    assert!(!FailureClass::TransientNetwork.is_deferred());
    assert!(!FailureClass::Internal.is_deferred());
}

#[test]
fn test_failure_class_allows_automatic_retry() {
    assert!(FailureClass::TransientNetwork.allows_automatic_retry());
    assert!(FailureClass::TransientTimeout.allows_automatic_retry());
    assert!(FailureClass::Internal.allows_automatic_retry());
    assert!(!FailureClass::Authentication.allows_automatic_retry());
    assert!(!FailureClass::Configuration.allows_automatic_retry());
    assert!(!FailureClass::Conflict.allows_automatic_retry());
}
