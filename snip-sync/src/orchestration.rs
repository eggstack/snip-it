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

    /// Production decision method used by `serve_inner`. Returns `Ok(())`
    /// only for a requested, unforced, dual-clean shutdown. Returns an
    /// `Err` containing both service classifications and the original
    /// detail for every other case: forced abort, unexpected clean exit,
    /// or any service error/panic.
    pub fn ensure_clean_requested_shutdown(&self) -> Result<(), String> {
        if self.is_clean_requested_shutdown() {
            return Ok(());
        }

        Err(format!(
            "service shutdown was not clean: requested={}, forced={}, grpc={:?}, http={:?}",
            self.requested, self.forced, self.grpc_result, self.http_result,
        ))
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

    let mut grpc_result: Option<ServiceResult> = None;
    let mut http_result: Option<ServiceResult> = None;

    // Keep abort handles for the original service tasks. The JoinSet below
    // owns wrapper tasks that classify those handles; aborting only the
    // wrappers would detach the actual service tasks on a forced drain.
    let grpc_abort = grpc_handle.abort_handle();
    let http_abort = http_handle.abort_handle();

    #[derive(Clone, Copy)]
    enum ServiceKind {
        Grpc,
        Http,
    }

    struct ServiceCompletion {
        kind: ServiceKind,
        result: ServiceResult,
    }

    let mut tasks = tokio::task::JoinSet::<ServiceCompletion>::new();
    tasks.spawn(async move {
        ServiceCompletion {
            kind: ServiceKind::Grpc,
            result: classify_result(grpc_handle.await),
        }
    });
    tasks.spawn(async move {
        ServiceCompletion {
            kind: ServiceKind::Http,
            result: classify_result(http_handle.await),
        }
    });

    let record = |completion: ServiceCompletion,
                  grpc_result: &mut Option<ServiceResult>,
                  http_result: &mut Option<ServiceResult>,
                  phase: &str| {
        if completion.result != ServiceResult::Clean {
            let name = match completion.kind {
                ServiceKind::Grpc => "gRPC",
                ServiceKind::Http => "HTTP",
            };
            tracing::error!("{name} service {phase}: {:?}", completion.result);
        }
        match completion.kind {
            ServiceKind::Grpc => *grpc_result = Some(completion.result),
            ServiceKind::Http => *http_result = Some(completion.result),
        }
    };

    // Phase 1: Wait for the first terminal event — a shutdown signal,
    // gRPC completion, or HTTP completion. No pre-signal lifetime
    // timeout; the server runs indefinitely until a signal or failure.
    tokio::select! {
        biased;
        _ = &mut shutdown_future => {
            tracing::info!("Shutdown signal received");
            requested = true;
        }
        completion = tasks.join_next() => {
            if let Some(Ok(completion)) = completion {
                record(completion, &mut grpc_result, &mut http_result, "completed");
            }
        }
    }

    // Broadcast shutdown to both services.
    let _ = shutdown_sender.send(());

    // Phase 2: drain every remaining task under one bounded timeout.
    let drain_result = tokio::time::timeout(drain_timeout, async {
        while let Some(joined) = tasks.join_next().await {
            if let Ok(completion) = joined {
                record(
                    completion,
                    &mut grpc_result,
                    &mut http_result,
                    "during drain",
                );
            }
        }
    })
    .await;

    // Phase 3: abort the original service tasks and drain the wrapper join
    // records so cancellation is observed through the original handles.
    let forced = drain_result.is_err();
    if forced {
        tracing::warn!(
            "Graceful shutdown timed out after {}s, aborting remaining tasks",
            drain_timeout.as_secs()
        );

        grpc_abort.abort();
        http_abort.abort();
        while let Some(joined) = tasks.join_next().await {
            if let Ok(completion) = joined {
                record(
                    completion,
                    &mut grpc_result,
                    &mut http_result,
                    "after abort",
                );
            }
        }
    }

    ServiceShutdownOutcome {
        requested,
        forced,
        grpc_result: grpc_result
            .unwrap_or_else(|| ServiceResult::Cancelled("not observed".to_owned())),
        http_result: http_result
            .unwrap_or_else(|| ServiceResult::Cancelled("not observed".to_owned())),
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

        // Signal completes immediately so the helper itself must be the
        // sole sender on the broadcast shutdown channel. Services cannot
        // wake from any test-side send.
        let signal = std::future::ready(());
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(outcome.requested);
        assert!(!outcome.forced);
        assert!(outcome.is_clean_requested_shutdown());
        assert!(
            outcome.ensure_clean_requested_shutdown().is_ok(),
            "production decision method must return Ok for clean requested shutdown"
        );
        assert_eq!(outcome.grpc_result, ServiceResult::Clean);
        assert_eq!(outcome.http_result, ServiceResult::Clean);
    }

    // ── Section 7.2: One service completes during drain, sibling times out ──

    #[tokio::test]
    async fn one_service_completes_sibling_times_out() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);
        let grpc_await_count = Arc::new(AtomicUsize::new(0));
        let http_drop_count = Arc::new(AtomicUsize::new(0));

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
        let http: HttpHandle = {
            let drop_count = http_drop_count.clone();
            tokio::spawn(async move {
                struct DropProbe(Arc<AtomicUsize>);
                impl Drop for DropProbe {
                    fn drop(&mut self) {
                        self.0.fetch_add(1, Ordering::SeqCst);
                    }
                }

                let _probe = DropProbe(drop_count);
                std::future::pending::<Result<(), std::io::Error>>().await
            })
        };

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = std::future::ready(());
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
        assert!(matches!(outcome.http_result, ServiceResult::Cancelled(_)));
        assert_eq!(
            http_drop_count.load(Ordering::SeqCst),
            1,
            "the underlying HTTP future must be dropped before the helper returns"
        );
        let err = outcome
            .ensure_clean_requested_shutdown()
            .expect_err("forced shutdown must fail production check");
        assert!(
            err.contains("forced=true"),
            "diagnostic should mention forced: {err}"
        );
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

        let signal = std::future::ready(());
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(outcome.requested);
        assert!(!outcome.forced);
        assert!(!outcome.is_clean_requested_shutdown());
        assert!(matches!(outcome.grpc_result, ServiceResult::Panic(_)));
        assert_eq!(outcome.http_result, ServiceResult::Clean);
        let err = outcome
            .ensure_clean_requested_shutdown()
            .expect_err("panic must fail production check");
        assert!(
            err.contains("grpc"),
            "diagnostic should mention grpc: {err}"
        );
        assert!(
            err.contains("grpc drain panic"),
            "diagnostic should retain original panic detail: {err}"
        );
    }

    // ── Section 7.3: Drain-time service error ─────────────────────

    #[tokio::test]
    async fn drain_time_service_error() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        // gRPC returns cleanly after receiving shutdown.
        let grpc: GrpcHandle = {
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let _ = rx.recv().await;
                Ok(())
            })
        };
        // HTTP returns an error after receiving shutdown.
        let http: HttpHandle = {
            let mut rx = tx.subscribe();
            tokio::spawn(async move {
                let _ = rx.recv().await;
                Err(std::io::Error::other("http service error"))
            })
        };

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = std::future::ready(());
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(outcome.requested);
        assert!(!outcome.forced);
        assert!(!outcome.is_clean_requested_shutdown());
        assert_eq!(outcome.grpc_result, ServiceResult::Clean);
        assert!(matches!(
            outcome.http_result,
            ServiceResult::ServiceError(_)
        ));
        let err = outcome
            .ensure_clean_requested_shutdown()
            .expect_err("drain-time error must fail production check");
        assert!(
            err.contains("http"),
            "diagnostic should mention http: {err}"
        );
        assert!(
            err.contains("http service error"),
            "diagnostic should retain original error detail: {err}"
        );
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

        // Signal held pending via oneshot — helper must wake from gRPC
        // completion, not from a test-side broadcast send.
        let (_signal_tx, signal_rx) = tokio::sync::oneshot::channel::<()>();
        let signal = async move {
            let _ = signal_rx.await;
        };
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(!outcome.requested);
        assert!(!outcome.forced);
        assert!(!outcome.is_clean_requested_shutdown());
        let err = outcome
            .ensure_clean_requested_shutdown()
            .expect_err("unexpected clean service exit must fail production check");
        assert!(
            err.contains("requested=false"),
            "diagnostic should mention requested=false: {err}"
        );
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

        let (_signal_tx, signal_rx) = tokio::sync::oneshot::channel::<()>();
        let signal = async move {
            let _ = signal_rx.await;
        };
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(!outcome.requested);
        assert!(!outcome.forced);
        assert!(!outcome.is_clean_requested_shutdown());
        let err = outcome
            .ensure_clean_requested_shutdown()
            .expect_err("unexpected service error must fail production check");
        assert!(
            err.contains("http"),
            "diagnostic should mention http: {err}"
        );
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

        let signal = std::future::ready(());
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
        let err = outcome
            .ensure_clean_requested_shutdown()
            .expect_err("forced abort must fail production check");
        assert!(
            err.contains("forced=true"),
            "diagnostic should mention forced: {err}"
        );
    }

    // ── Section 7.7: No pre-signal lifetime timeout ────────────────

    #[tokio::test]
    async fn no_pre_signal_lifetime_timeout() {
        let drain_timeout = Duration::from_millis(100);

        // ── Phase A: helper remains pending while no terminal event ──
        // The helper must not start the drain timeout until a terminal
        // event has occurred. We construct the helper with a held-pending
        // oneshot signal and no pre-completing services, then assert that
        // it does not complete within 2× the drain timeout.
        {
            let (tx, _) = tokio::sync::broadcast::channel::<()>(1);
            let (_signal_tx, signal_rx) = tokio::sync::oneshot::channel::<()>();
            let grpc: GrpcHandle = {
                let mut rx = tx.subscribe();
                tokio::spawn(async move {
                    let _ = rx.recv().await;
                    Ok(())
                })
            };
            let http: HttpHandle = {
                let mut rx = tx.subscribe();
                tokio::spawn(async move {
                    let _ = rx.recv().await;
                    Ok(())
                })
            };
            let mut signal = Box::pin(async move {
                let _ = signal_rx.await;
            });
            let helper_fut =
                run_services_until_shutdown(signal.as_mut(), grpc, http, tx, drain_timeout);
            let phase_a_result = tokio::time::timeout(drain_timeout * 2, helper_fut).await;
            assert!(
                phase_a_result.is_err(),
                "helper must remain pending for at least 2× drain timeout when no terminal event has occurred"
            );
        }

        // ── Phase B: signal triggers and helper completes cleanly ──
        // Trigger the oneshot BEFORE polling the helper so the first
        // poll sees the signal ready, then verify a clean requested
        // shutdown.
        {
            let (tx, _) = tokio::sync::broadcast::channel::<()>(1);
            let (signal_tx, signal_rx) = tokio::sync::oneshot::channel::<()>();
            let grpc: GrpcHandle = {
                let mut rx = tx.subscribe();
                tokio::spawn(async move {
                    let _ = rx.recv().await;
                    Ok(())
                })
            };
            let http: HttpHandle = {
                let mut rx = tx.subscribe();
                tokio::spawn(async move {
                    let _ = rx.recv().await;
                    Ok(())
                })
            };
            let mut signal = Box::pin(async move {
                let _ = signal_rx.await;
            });
            let _ = signal_tx.send(());
            let helper_fut =
                run_services_until_shutdown(signal.as_mut(), grpc, http, tx, drain_timeout);
            let outcome = tokio::time::timeout(Duration::from_secs(5), helper_fut)
                .await
                .expect("helper should complete within 5s after signal trigger");
            assert!(outcome.requested);
            assert!(!outcome.forced);
            assert!(outcome.is_clean_requested_shutdown());
            assert!(
                outcome.ensure_clean_requested_shutdown().is_ok(),
                "production decision method must return Ok for clean requested shutdown"
            );
        }
    }

    // ── Regression: completed gRPC service is observed once ───────

    #[tokio::test]
    async fn grpc_service_completion_is_observed_once() {
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

        let signal = std::future::ready(());
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

    // ── Section 7.4 regression: drain-time panic preserves detail ──

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

        let signal = std::future::ready(());
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

        let signal = std::future::ready(());
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

        let signal = std::future::ready(());
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(outcome.requested);
        assert!(!outcome.forced);
        assert!(outcome.is_clean_requested_shutdown());
    }
}
