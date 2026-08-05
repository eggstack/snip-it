//! Server shutdown orchestration.
//!
//! Provides the single production implementation of service-lifetime
//! coordination used by both `serve_inner` and deterministic tests.

use std::time::Duration;

/// Terminal classification of a service task result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceResult {
    /// Task returned `Ok(())`.
    Clean,
    /// Task returned `Ok(Err(e))` — a service-level error.
    ServiceError(String),
    /// Task returned `Err(JoinError)` where `is_panic()` is true.
    Panic(String),
    /// Task was cancelled (explicitly aborted).
    Cancelled(String),
}

/// Result of the shutdown orchestration.
#[derive(Debug)]
pub struct ServiceShutdownOutcome {
    /// `true` when the shutdown was triggered by a process signal
    /// (Ctrl-C / SIGTERM). `false` when an unexpected service completion
    /// triggered shutdown.
    pub requested: bool,
    /// `true` when the drain timed out and unfinished tasks were forcibly
    /// aborted.
    pub forced: bool,
    /// Terminal result of the gRPC service.
    pub grpc_result: ServiceResult,
    /// Terminal result of the HTTP service.
    pub http_result: ServiceResult,
}

impl ServiceShutdownOutcome {
    /// A requested shutdown succeeds only when both services returned
    /// cleanly without forced abort.
    pub fn is_clean_requested_shutdown(&self) -> bool {
        self.requested
            && !self.forced
            && self.grpc_result == ServiceResult::Clean
            && self.http_result == ServiceResult::Clean
    }
}

/// Classify a `JoinHandle` output into a `ServiceResult`.
fn classify_result(
    result: Result<Result<(), impl std::fmt::Display>, tokio::task::JoinError>,
) -> ServiceResult {
    match result {
        Ok(Ok(())) => ServiceResult::Clean,
        Ok(Err(e)) => ServiceResult::ServiceError(e.to_string()),
        Err(e) if e.is_panic() => ServiceResult::Panic(format!("{e}")),
        Err(e) => ServiceResult::Cancelled(format!("{e}")),
    }
}

/// Run two service task handles until a shutdown signal or unexpected
/// service completion, then drain with a bounded timeout.
///
/// This is the single production orchestration implementation used by
/// both `serve_inner` and deterministic orchestration tests.
///
/// # Arguments
/// * `shutdown_future` — a future that completes when a process signal
///   is received.
/// * `grpc_handle` — the gRPC service JoinHandle.
/// * `http_handle` — the HTTP service JoinHandle.
/// * `shutdown_sender` — broadcast sender to notify services of shutdown.
/// * `drain_timeout` — maximum time to wait for services to drain after
///   shutdown.
pub async fn run_services_until_shutdown<F>(
    mut shutdown_future: std::pin::Pin<&mut F>,
    grpc_handle: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
    http_handle: tokio::task::JoinHandle<Result<(), std::io::Error>>,
    shutdown_sender: tokio::sync::broadcast::Sender<()>,
    drain_timeout: Duration,
) -> ServiceShutdownOutcome
where
    F: std::future::Future<Output = ()>,
{
    let mut requested = false;

    // Track whether each handle has been consumed (output received).
    // A consumed handle must never be awaited or aborted again.
    let mut grpc_consumed = false;
    let mut http_consumed = false;
    let mut grpc_result: Option<ServiceResult> = None;
    let mut http_result: Option<ServiceResult> = None;

    // We need mutable references to the handles for tokio::select!, but
    // we also need to move them for await. Use a small wrapper to allow
    // both.
    let mut grpc_handle = Some(grpc_handle);
    let mut http_handle = Some(http_handle);

    // Phase 1: Wait for the first terminal event — a shutdown signal,
    // gRPC completion, or HTTP completion. No pre-signal lifetime
    // timeout; the server runs indefinitely until a signal or failure.
    tokio::select! {
        biased;
        _ = &mut shutdown_future => {
            tracing::info!("Shutdown signal received");
            requested = true;
        }
        result = grpc_handle.as_mut().unwrap() => {
            let classified = classify_result(result);
            if classified != ServiceResult::Clean {
                tracing::error!("gRPC service: {classified:?}");
            }
            grpc_result = Some(classified);
            grpc_consumed = true;
        }
        result = http_handle.as_mut().unwrap() => {
            let classified = classify_result(result);
            if classified != ServiceResult::Clean {
                tracing::error!("HTTP service: {classified:?}");
            }
            http_result = Some(classified);
            http_consumed = true;
        }
    }

    // Broadcast shutdown to both services.
    let _ = shutdown_sender.send(());

    // Phase 2: Bounded drain. After the initial select!, at most one
    // handle has been consumed. For remaining pending handles, select
    // whichever finishes next. Each completion updates consumed state
    // immediately so Phase 3 never re-awaits a completed handle.
    let drain_result = tokio::time::timeout(drain_timeout, async {
        if !grpc_consumed && !http_consumed {
            // Both still pending — select whichever finishes first.
            tokio::select! {
                biased;
                result = grpc_handle.as_mut().unwrap() => {
                    let classified = classify_result(result);
                    if classified != ServiceResult::Clean {
                        tracing::error!("gRPC service during drain: {classified:?}");
                    }
                    grpc_result = Some(classified);
                    grpc_consumed = true;
                }
                result = http_handle.as_mut().unwrap() => {
                    let classified = classify_result(result);
                    if classified != ServiceResult::Clean {
                        tracing::error!("HTTP service during drain: {classified:?}");
                    }
                    http_result = Some(classified);
                    http_consumed = true;
                }
            }
        }

        // Await whichever remaining handle is still pending.
        if !grpc_consumed {
            let result = grpc_handle.as_mut().unwrap().await;
            let classified = classify_result(result);
            if classified != ServiceResult::Clean {
                tracing::error!("gRPC service during drain: {classified:?}");
            }
            grpc_result = Some(classified);
            grpc_consumed = true;
        }
        if !http_consumed {
            let result = http_handle.as_mut().unwrap().await;
            let classified = classify_result(result);
            if classified != ServiceResult::Clean {
                tracing::error!("HTTP service during drain: {classified:?}");
            }
            http_result = Some(classified);
            http_consumed = true;
        }
    })
    .await;

    // Phase 3: Handle timeout — abort still-pending handles and await
    // each exactly once. Only handles not yet consumed are aborted;
    // completed handles are never touched again.
    let forced = drain_result.is_err();
    if forced {
        tracing::warn!(
            "Graceful shutdown timed out after {}s, aborting remaining tasks",
            drain_timeout.as_secs()
        );

        if !grpc_consumed {
            if let Some(ref mut h) = grpc_handle {
                h.abort();
                let result = h.await;
                grpc_result = Some(match result {
                    Err(e) if e.is_panic() => ServiceResult::Panic(format!("aborted: {e}")),
                    Err(e) => ServiceResult::Cancelled(format!("aborted: {e}")),
                    Ok(Ok(())) => ServiceResult::Clean,
                    Ok(Err(e)) => ServiceResult::ServiceError(e.to_string()),
                });
            }
            grpc_consumed = true;
        }

        if !http_consumed {
            if let Some(ref mut h) = http_handle {
                h.abort();
                let result = h.await;
                http_result = Some(match result {
                    Err(e) if e.is_panic() => ServiceResult::Panic(format!("aborted: {e}")),
                    Err(e) => ServiceResult::Cancelled(format!("aborted: {e}")),
                    Ok(Ok(())) => ServiceResult::Clean,
                    Ok(Err(e)) => ServiceResult::ServiceError(e.to_string()),
                });
            }
            http_consumed = true;
        }
    }

    // Both handles must be consumed before returning.
    debug_assert!(grpc_consumed, "gRPC handle must be consumed");
    debug_assert!(http_consumed, "HTTP handle must be consumed");

    ServiceShutdownOutcome {
        requested,
        forced,
        grpc_result: grpc_result.expect("gRPC result must be set before returning"),
        http_result: http_result.expect("HTTP result must be set before returning"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    type GrpcHandle = tokio::task::JoinHandle<Result<(), tonic::transport::Error>>;
    type HttpHandle = tokio::task::JoinHandle<Result<(), std::io::Error>>;

    // ── Section 7.1: Requested clean shutdown ──────────────────────

    #[tokio::test]
    async fn requested_clean_shutdown() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc: GrpcHandle = tokio::spawn({
            let mut rx = tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok(())
            }
        });
        let http: HttpHandle = tokio::spawn({
            let mut rx = tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok(())
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = {
            let tx = tx.clone();
            async move {
                let _ = tx.send(());
            }
        };
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(outcome.requested);
        assert!(!outcome.forced);
        assert!(outcome.is_clean_requested_shutdown());
        assert_eq!(outcome.grpc_result, ServiceResult::Clean);
        assert_eq!(outcome.http_result, ServiceResult::Clean);
    }

    // ── Section 7.2: One service completes during drain, sibling times out ──

    #[tokio::test]
    async fn one_service_completes_sibling_times_out() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);
        let grpc_await_count = Arc::new(AtomicUsize::new(0));

        let grpc: GrpcHandle = {
            let count = grpc_await_count.clone();
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let _ = rx.recv().await;
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        };
        // HTTP never completes — it ignores the shutdown signal.
        let http: HttpHandle =
            tokio::spawn(async { std::future::pending::<Result<(), std::io::Error>>().await });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = {
            let tx = tx.clone();
            async move {
                let _ = tx.send(());
            }
        };
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_millis(100)).await;

        assert!(outcome.requested);
        assert!(outcome.forced);
        assert_eq!(outcome.grpc_result, ServiceResult::Clean);
        assert_eq!(
            grpc_await_count.load(Ordering::SeqCst),
            1,
            "gRPC future should have completed exactly once"
        );
        assert!(matches!(
            outcome.http_result,
            ServiceResult::Cancelled(_) | ServiceResult::Panic(_)
        ));
    }

    // ── Section 7.4: Drain-time service panic ──────────────────────

    #[tokio::test]
    async fn drain_time_service_panic() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc: GrpcHandle = {
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let _ = rx.recv().await;
                panic!("grpc drain panic");
            })
        };
        let http: HttpHandle = {
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let _ = rx.recv().await;
                Ok(())
            })
        };

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = {
            let tx = tx.clone();
            async move {
                let _ = tx.send(());
            }
        };
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(outcome.requested);
        assert!(!outcome.forced);
        assert!(!outcome.is_clean_requested_shutdown());
        assert!(matches!(outcome.grpc_result, ServiceResult::Panic(_)));
        assert_eq!(outcome.http_result, ServiceResult::Clean);
    }

    // ── Section 7.5: Unexpected service completion ─────────────────

    #[tokio::test]
    async fn unexpected_grpc_exit() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc: GrpcHandle = tokio::spawn(async { Ok(()) });
        let http: HttpHandle = {
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let _ = rx.recv().await;
                Ok(())
            })
        };

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = std::future::pending::<()>();
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(!outcome.requested);
        assert!(!outcome.forced);
        assert!(!outcome.is_clean_requested_shutdown());
    }

    #[tokio::test]
    async fn unexpected_http_error() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc: GrpcHandle = {
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let _ = rx.recv().await;
                Ok(())
            })
        };
        let http: HttpHandle = tokio::spawn(async { Err(std::io::Error::other("http boom")) });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = std::future::pending::<()>();
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(!outcome.requested);
        assert!(!outcome.forced);
        assert!(!outcome.is_clean_requested_shutdown());
    }

    // ── Section 7.6: Both services refuse to drain ─────────────────

    #[tokio::test]
    async fn both_refuse_to_drain() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc: GrpcHandle = tokio::spawn(async {
            std::future::pending::<Result<(), tonic::transport::Error>>().await
        });
        let http: HttpHandle =
            tokio::spawn(async { std::future::pending::<Result<(), std::io::Error>>().await });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = {
            let tx = tx.clone();
            async move {
                let _ = tx.send(());
            }
        };
        tokio::pin!(signal);

        let start = std::time::Instant::now();
        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_millis(100)).await;
        let elapsed = start.elapsed();

        assert!(outcome.requested);
        assert!(outcome.forced);
        assert!(!outcome.is_clean_requested_shutdown());
        assert!(
            elapsed < Duration::from_secs(2),
            "test should complete quickly, took {elapsed:?}"
        );
    }

    // ── Section 7.7: No pre-signal lifetime timeout ────────────────

    #[tokio::test]
    async fn no_pre_signal_lifetime_timeout() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc: GrpcHandle = tokio::spawn({
            let mut rx = tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok(())
            }
        });
        let http: HttpHandle = tokio::spawn({
            let mut rx = tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok(())
            }
        });

        tokio::time::sleep(Duration::from_millis(200)).await;

        let signal = {
            let tx = tx.clone();
            async move {
                let _ = tx.send(());
            }
        };
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_millis(100)).await;

        assert!(outcome.requested);
        assert!(!outcome.forced);
        assert!(outcome.is_clean_requested_shutdown());
    }

    // ── Section 7.2 regression: gRPC result consumed exactly once ──

    #[tokio::test]
    async fn grpc_handle_consumed_exactly_once() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);
        let poll_count = Arc::new(AtomicUsize::new(0));

        let grpc: GrpcHandle = {
            let count = poll_count.clone();
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let _ = rx.recv().await;
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        };
        let http: HttpHandle =
            tokio::spawn(async { std::future::pending::<Result<(), std::io::Error>>().await });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = {
            let tx = tx.clone();
            async move {
                let _ = tx.send(());
            }
        };
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_millis(100)).await;

        assert!(outcome.forced);
        assert_eq!(
            poll_count.load(Ordering::SeqCst),
            1,
            "gRPC future should have been polled exactly once"
        );
    }

    // ── Section 7.3 regression: drain-time panic preserves detail ──

    #[tokio::test]
    async fn drain_panic_preserved_in_outcome() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc: GrpcHandle = {
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let _ = rx.recv().await;
                panic!("deliberate grpc panic");
            })
        };
        let http: HttpHandle = {
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let _ = rx.recv().await;
                Ok(())
            })
        };

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = {
            let tx = tx.clone();
            async move {
                let _ = tx.send(());
            }
        };
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(outcome.requested);
        assert!(!outcome.forced);
        assert!(!outcome.is_clean_requested_shutdown());
        match &outcome.grpc_result {
            ServiceResult::Panic(msg) => {
                assert!(
                    msg.contains("deliberate grpc panic"),
                    "panic message should be preserved"
                );
            }
            other => panic!("expected Panic, got {other:?}"),
        }
    }

    // ── Section 7.6 regression: HTTP aborted while gRPC clean ──────

    #[tokio::test]
    async fn http_aborted_while_grpc_clean() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc: GrpcHandle = {
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let _ = rx.recv().await;
                Ok(())
            })
        };
        let http: HttpHandle =
            tokio::spawn(async { std::future::pending::<Result<(), std::io::Error>>().await });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = {
            let tx = tx.clone();
            async move {
                let _ = tx.send(());
            }
        };
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_millis(100)).await;

        assert!(outcome.requested);
        assert!(outcome.forced);
        assert_eq!(outcome.grpc_result, ServiceResult::Clean);
        assert!(matches!(
            outcome.http_result,
            ServiceResult::Cancelled(_) | ServiceResult::Panic(_)
        ));
    }

    // ── Section 7.1 regression: both complete during drain ──────────

    #[tokio::test]
    async fn both_complete_during_drain() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc: GrpcHandle = {
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let _ = rx.recv().await;
                tokio::time::sleep(Duration::from_millis(10)).await;
                Ok(())
            })
        };
        let http: HttpHandle = {
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let _ = rx.recv().await;
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok(())
            })
        };

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = {
            let tx = tx.clone();
            async move {
                let _ = tx.send(());
            }
        };
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(outcome.requested);
        assert!(!outcome.forced);
        assert!(outcome.is_clean_requested_shutdown());
    }
}
