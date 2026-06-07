//! Test-only helpers for the snip-sync server.
//!
//! These utilities are used by integration tests in `tests/` to spin up the
//! server in-process and observe the gRPC metadata of incoming requests. They
//! are not used in production.

use std::path::PathBuf;
use std::sync::Arc;

use tonic::transport::Server;

use crate::db::Database;
use crate::{Config, Metrics, PremadeManager, RateLimiter, SnipSyncService};
use snip_proto::snippet_sync_server::SnippetSyncServer;

/// Builds a `SnipSyncService` pre-configured for in-process testing.
///
/// Uses an in-memory SQLite database, the fallback metrics registry, an
/// in-memory rate limiter, and an empty premade-libraries directory. The
/// `captured_auth_header` field is freshly initialized to `None` so each
/// test can read the first request's `authorization` metadata after the
/// call completes.
pub async fn build_test_service() -> SnipSyncService {
    let db = Arc::new(Database::connect("sqlite::memory:", 5).await.unwrap());
    let config = Config {
        grpc_host: "127.0.0.1".to_string(),
        grpc_port: 0,
        http_host: "127.0.0.1".to_string(),
        http_port: 0,
        db_path: "sqlite::memory:".to_string(),
        db_max_connections: 5,
        premade_dir: PathBuf::from("premade-libraries"),
        max_command_length: 1024,
        max_description_length: 1024,
        max_tags: 50,
        max_tag_length: 100,
        max_id_length: 128,
        max_device_id_length: 128,
        max_api_key_length: 512,
        request_timeout_secs: 30,
        grpc_max_message_size: 4 * 1024 * 1024,
        rate_limit_per_minute: 120,
        trusted_proxies: vec![],
        persist_rate_limits: false,
        metrics_username: None,
        metrics_password: None,
        cors_allowed_origins: vec![],
    };
    let metrics = Metrics::fallback();
    let rate_limiter = Arc::new(RateLimiter::new());
    let premade_manager = PremadeManager::new(PathBuf::from("premade-libraries"));

    SnipSyncService {
        db,
        rate_limiter,
        config,
        metrics,
        premade_manager,
        captured_auth_header: Arc::new(std::sync::Mutex::new(None)),
    }
}

/// Starts a `SnippetSyncServer` on `127.0.0.1:0` (random port) and returns:
///   - the bound `SocketAddr` the client should dial,
///   - the `JoinHandle` for the server task (drop to stop the server),
///   - a clone of the service's `captured_auth_header` slot, so the test can
///     assert on the first request's `authorization` metadata after the call.
pub async fn start_test_server(
    service: SnipSyncService,
) -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<()>,
    Arc<std::sync::Mutex<Option<String>>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    let captured = service.captured_auth_header.clone();
    let svc = SnippetSyncServer::new(service)
        .max_decoding_message_size(4 * 1024 * 1024)
        .max_encoding_message_size(4 * 1024 * 1024);

    let server_task = tokio::spawn(async move {
        let (_tx, rx) = tokio::sync::oneshot::channel::<()>();
        let result = Server::builder()
            .add_service(svc)
            .serve_with_incoming_shutdown(incoming, async move {
                let _ = rx.await;
            })
            .await;
        if let Err(e) = result {
            tracing::error!("test server error: {e}");
        }
    });

    (addr, server_task, captured)
}
