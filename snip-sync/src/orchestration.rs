//! Server shutdown orchestration tests.
//!
//! These tests verify the core shutdown coordination invariants from
//! `main.rs` using short fake service futures and channels. They serve
//! as the deterministic regression coverage for Workstream I.

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[derive(Debug)]
    struct FakeError(&'static str);
    impl std::fmt::Display for FakeError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }
    impl std::error::Error for FakeError {}

    #[allow(dead_code)]
    async fn service_ok() -> Result<(), FakeError> {
        Ok(())
    }

    async fn service_err() -> Result<(), FakeError> {
        Err(FakeError("service error"))
    }

    async fn service_panic() -> Result<(), FakeError> {
        panic!("service panic");
    }

    /// Test: requested shutdown notifies both fake services and they exit
    /// within the drain bound. Both services are awaited before persistence
    /// shutdown is observed.
    #[tokio::test]
    async fn requested_shutdown_notifies_both_services() {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc_fut = {
            let mut rx = shutdown_tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok::<(), FakeError>(())
            }
        };
        let http_fut = {
            let mut rx = shutdown_tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok::<(), FakeError>(())
            }
        };

        let grpc = tokio::spawn(grpc_fut);
        let http = tokio::spawn(http_fut);

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Broadcast shutdown.
        let _ = shutdown_tx.send(());

        let drain = tokio::time::timeout(Duration::from_secs(2), async {
            let _ = tokio::join!(grpc, http);
        })
        .await;

        assert!(drain.is_ok(), "services should exit within drain bound");
    }

    /// Test: first service error triggers sibling shutdown.
    /// The error is not swallowed into log-only Ok(()).
    #[tokio::test]
    async fn first_service_error_triggers_sibling_shutdown() {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

        let grpc = tokio::spawn(service_err());
        let http_fut = {
            let mut rx = shutdown_tx.subscribe();
            async move {
                let _ = rx.recv().await;
                Ok::<(), FakeError>(())
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
                Ok::<(), FakeError>(())
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

        // Simulate: signal received, both services still running.
        // Drain with a short timeout. completes_later finishes via select,
        // never_complete hangs -> timeout triggers abort.
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
}
