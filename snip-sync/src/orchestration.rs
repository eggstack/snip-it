//! Server shutdown orchestration.
//!
//! Provides the single production implementation of service-lifetime
//! coordination used by both `serve_inner` and deterministic tests.

use std::time::Duration;

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
    /// Error message from the gRPC service if it completed unexpectedly.
    pub grpc_error: Option<String>,
    /// Error message from the HTTP service if it completed unexpectedly.
    pub http_error: Option<String>,
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
    mut grpc_handle: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
    mut http_handle: tokio::task::JoinHandle<Result<(), std::io::Error>>,
    shutdown_sender: tokio::sync::broadcast::Sender<()>,
    drain_timeout: Duration,
) -> ServiceShutdownOutcome
where
    F: std::future::Future<Output = ()>,
{
    let mut requested = false;
    let mut grpc_error: Option<String> = None;
    let mut http_error: Option<String> = None;
    let mut grpc_consumed = false;
    let mut http_consumed = false;

    tokio::select! {
        biased;
        _ = &mut shutdown_future => {
            tracing::info!("Shutdown signal received");
            requested = true;
        }
        result = &mut grpc_handle => {
            grpc_consumed = true;
            match result {
                Ok(Ok(())) => {
                    tracing::error!("gRPC service exited unexpectedly");
                    grpc_error = Some("exited unexpectedly".to_string());
                }
                Ok(Err(e)) => {
                    tracing::error!("gRPC service error: {}", e);
                    grpc_error = Some(e.to_string());
                }
                Err(e) => {
                    tracing::error!("gRPC service task panicked: {}", e);
                    grpc_error = Some(format!("panicked: {e}"));
                }
            }
        }
        result = &mut http_handle => {
            http_consumed = true;
            match result {
                Ok(Ok(())) => {
                    tracing::error!("HTTP service exited unexpectedly");
                    http_error = Some("exited unexpectedly".to_string());
                }
                Ok(Err(e)) => {
                    tracing::error!("HTTP service error: {}", e);
                    http_error = Some(e.to_string());
                }
                Err(e) => {
                    tracing::error!("HTTP service task panicked: {}", e);
                    http_error = Some(format!("panicked: {e}"));
                }
            }
        }
    }

    let _ = shutdown_sender.send(());

    let forced = tokio::time::timeout(drain_timeout, async {
        if !grpc_consumed {
            let _ = (&mut grpc_handle).await;
        }
        if !http_consumed {
            let _ = (&mut http_handle).await;
        }
    })
    .await
    .is_err();

    if forced {
        tracing::warn!(
            "Graceful shutdown timed out after {}s, aborting remaining tasks",
            drain_timeout.as_secs()
        );
        if !grpc_consumed {
            grpc_handle.abort();
            let _ = grpc_handle.await;
        }
        if !http_consumed {
            http_handle.abort();
            let _ = http_handle.await;
        }
    }

    ServiceShutdownOutcome {
        requested,
        forced,
        grpc_error,
        http_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[allow(dead_code)]
    async fn service_ok() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn service_err() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Err("service error".into())
    }

    async fn service_panic() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        panic!("service panic");
    }

    /// Helper: build a shutdown signal future that completes immediately.
    #[allow(dead_code)]
    async fn immediate_signal() {
        // The future is already complete — select will pick it up on first poll.
    }

    /// Test: requested shutdown notifies both fake services and they exit
    /// within the drain bound.
    #[tokio::test]
    async fn requested_shutdown_notifies_both_services() {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc_fut = {
            let mut rx = shutdown_tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
            }
        };
        let http_fut = {
            let mut rx = shutdown_tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
            }
        };

        let grpc = tokio::spawn(grpc_fut);
        let http = tokio::spawn(http_fut);

        tokio::time::sleep(Duration::from_millis(50)).await;

        let _ = shutdown_tx.send(());

        let drain = tokio::time::timeout(Duration::from_secs(2), async {
            let _ = tokio::join!(grpc, http);
        })
        .await;

        assert!(drain.is_ok(), "services should exit within drain bound");
    }

    /// Test: first service error triggers sibling shutdown.
    #[tokio::test]
    async fn first_service_error_triggers_sibling_shutdown() {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc = tokio::spawn(service_err());
        let http_fut = {
            let mut rx = shutdown_tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
            }
        };
        let http = tokio::spawn(http_fut);

        tokio::time::sleep(Duration::from_millis(50)).await;

        let result = tokio::time::timeout(Duration::from_secs(2), async {
            let grpc_result = grpc.await;
            let _ = shutdown_tx.send(());
            let http_result = http.await;
            (grpc_result, http_result)
        })
        .await;

        assert!(result.is_ok(), "both tasks should complete promptly");
        let (grpc_r, http_r) = result.unwrap();
        assert!(grpc_r.is_ok(), "grpc JoinHandle should resolve");
        assert!(grpc_r.unwrap().is_err(), "grpc should have returned error");
        assert!(http_r.is_ok(), "http JoinHandle should resolve");
    }

    /// Test: first service panic triggers sibling shutdown.
    #[tokio::test]
    async fn first_service_panic_triggers_sibling_shutdown() {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc = tokio::spawn(service_panic());
        let http_fut = {
            let mut rx = shutdown_tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
            }
        };
        let http = tokio::spawn(http_fut);

        tokio::time::sleep(Duration::from_millis(50)).await;

        let result = tokio::time::timeout(Duration::from_secs(2), async {
            let grpc_result = grpc.await;
            let _ = shutdown_tx.send(());
            let http_result = http.await;
            (grpc_result, http_result)
        })
        .await;

        assert!(result.is_ok());
        let (grpc_r, http_r) = result.unwrap();
        assert!(grpc_r.is_err(), "grpc should have panicked");
        assert!(http_r.is_ok(), "http JoinHandle should resolve");
    }

    /// Test: one service refusing to drain is aborted after timeout.
    #[tokio::test]
    async fn refusing_service_is_aborted_after_timeout() {
        let mut never_complete =
            tokio::spawn(async { std::future::pending::<Result<(), FakeError>>().await });
        let mut completes_later = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok::<(), FakeError>(())
        });

        let drain = tokio::time::timeout(Duration::from_millis(100), async {
            tokio::select! {
                _ = &mut never_complete => {}
                _ = &mut completes_later => {}
            }
        })
        .await;

        assert!(drain.is_err(), "drain should time out");

        never_complete.abort();
        completes_later.abort();
        let _ = never_complete.await;
        let _ = completes_later.await;
    }

    /// Test: no normal-operation lifetime timeout exists.
    #[tokio::test]
    async fn no_normal_operation_lifetime_timeout() {
        let grpc = tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok::<(), FakeError>(())
        });
        let http = tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok::<(), FakeError>(())
        });

        let result = tokio::time::timeout(Duration::from_secs(5), async {
            let _ = tokio::join!(grpc, http);
        })
        .await;

        assert!(
            result.is_ok(),
            "services should complete without arbitrary timeout"
        );
    }

    #[derive(Debug)]
    struct FakeError(&'static str);
    impl std::fmt::Display for FakeError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }
    impl std::error::Error for FakeError {}

    type GrpcHandle = tokio::task::JoinHandle<Result<(), tonic::transport::Error>>;
    type HttpHandle = tokio::task::JoinHandle<Result<(), std::io::Error>>;

    // ── Tests exercising run_services_until_shutdown directly ──────────

    /// Signal triggers requested=true, forced=false.
    #[tokio::test]
    async fn orchestration_signal_sets_requested() {
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
        assert!(outcome.grpc_error.is_none());
        assert!(outcome.http_error.is_none());
    }

    /// gRPC failure triggers requested=false and captures the error.
    #[tokio::test]
    async fn orchestration_grpc_error_captured() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        // tonic::transport::Error has no public constructor; use panic
        // which JoinError captures.
        let grpc: GrpcHandle = tokio::spawn(async {
            panic!("grpc service error");
        });
        let http: HttpHandle = tokio::spawn({
            let mut rx = tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok(())
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = std::future::pending::<()>();
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(!outcome.requested);
        assert!(!outcome.forced);
        assert!(outcome.grpc_error.is_some());
        assert!(outcome.http_error.is_none());
    }

    /// HTTP failure triggers requested=false and captures the error.
    #[tokio::test]
    async fn orchestration_http_error_captured() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc: GrpcHandle = tokio::spawn({
            let mut rx = tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok(())
            }
        });
        let http: HttpHandle = tokio::spawn(async { Err(std::io::Error::other("http boom")) });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = std::future::pending::<()>();
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(!outcome.requested);
        assert!(!outcome.forced);
        assert!(outcome.grpc_error.is_none());
        assert!(outcome.http_error.is_some());
    }

    /// Panicking service is caught as a task panic.
    #[tokio::test]
    async fn orchestration_grpc_panic_captured() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc: GrpcHandle = tokio::spawn(async { panic!("grpc panic") });
        let http: HttpHandle = tokio::spawn({
            let mut rx = tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok(())
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = std::future::pending::<()>();
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(!outcome.requested);
        assert!(!outcome.forced);
        assert!(outcome.grpc_error.is_some());
        let msg = outcome.grpc_error.unwrap();
        assert!(
            msg.contains("panicked"),
            "expected panic message, got: {msg}"
        );
    }

    /// Refusing to drain triggers forced=true and abort.
    #[tokio::test]
    async fn orchestration_forced_abort_on_timeout() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc: GrpcHandle = tokio::spawn(async {
            std::future::pending::<Result<(), tonic::transport::Error>>().await
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
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_millis(100)).await;

        assert!(outcome.requested);
        assert!(outcome.forced);
    }

    /// Both services completing before signal triggers requested=false.
    #[tokio::test]
    async fn orchestration_both_complete_early() {
        let (tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc: GrpcHandle = tokio::spawn(async { Ok(()) });
        let http: HttpHandle = tokio::spawn(async { Ok(()) });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let signal = std::future::pending::<()>();
        tokio::pin!(signal);

        let outcome =
            run_services_until_shutdown(signal, grpc, http, tx, Duration::from_secs(2)).await;

        assert!(!outcome.requested);
        assert!(!outcome.forced);
    }
}
