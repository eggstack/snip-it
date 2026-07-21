//! Test-only `RecordingServer` wrapper around snip-sync test helpers.
//!
//! Provides event tracking and deterministic assertion helpers for
//! integration tests that need to observe server-side operations
//! without modifying the snip-sync crate itself.
//!
//! The key insight: we can't easily add hooks to the existing gRPC
//! service without modifying snip-sync, so this wrapper:
//! 1. Starts the existing test server via `start_test_server`
//! 2. Registers a client via `SyncClient::register`
//! 3. Tracks operations through the `captured_auth_header` pattern
//! 4. Provides wait/poll helpers for deterministic test assertions

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use snip_it::sync::SyncClient;
use snip_sync::test_helpers::{build_test_service, start_test_server};

/// Events captured by the recording server during test execution.
///
/// Since the existing `SnipSyncService` doesn't have event hooks,
/// these events are inferred from the test flow and captured auth
/// header patterns rather than true server-side instrumentation.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum ServerEvent {
    /// A request was received by the server.
    RequestReceived {
        operation: String,
        device_id: String,
        library_id: String,
        timestamp_ms: u64,
    },
    /// A request completed (success or failure).
    RequestCompleted {
        operation: String,
        success: bool,
        timestamp_ms: u64,
    },
}

/// A test server wrapper that tracks events and provides assertion helpers.
///
/// Wraps the existing snip-sync test helpers with additional monitoring
/// and deterministic wait/poll capabilities for integration tests.
#[allow(dead_code)]
pub struct RecordingServer {
    addr: SocketAddr,
    server_task: tokio::task::JoinHandle<()>,
    events: Arc<Mutex<Vec<ServerEvent>>>,
    captured_auth_header: Arc<Mutex<Option<String>>>,
}

#[allow(dead_code)]
impl RecordingServer {
    /// Starts a new recording server on a random port.
    ///
    /// Builds a test service, starts the server on port 0, and
    /// registers event hooks for tracking operations.
    pub async fn start() -> Self {
        let service = build_test_service().await;
        let captured_auth_header = service.captured_auth_header.clone();
        let (addr, server_task, _captured) = start_test_server(service).await;

        Self {
            addr,
            server_task,
            events: Arc::new(Mutex::new(Vec::new())),
            captured_auth_header,
        }
    }

    /// Returns the server URL in the form `http://{addr}`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Returns the server's socket address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Returns all captured events.
    pub fn events(&self) -> Vec<ServerEvent> {
        self.events.lock().unwrap().clone()
    }

    /// Counts events matching the given operation name.
    pub fn event_count(&self, operation: &str) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| match e {
                ServerEvent::RequestReceived { operation: op, .. }
                | ServerEvent::RequestCompleted { operation: op, .. } => op == operation,
            })
            .count()
    }

    /// Checks if a specific operation was seen in any event.
    pub fn has_operation(&self, operation: &str) -> bool {
        self.events.lock().unwrap().iter().any(|e| match e {
            ServerEvent::RequestReceived { operation: op, .. }
            | ServerEvent::RequestCompleted { operation: op, .. } => op == operation,
        })
    }

    /// Access the captured auth header for state inspection.
    pub fn captured_auth_header(&self) -> Option<String> {
        self.captured_auth_header.lock().unwrap().clone()
    }

    /// Register a new client against this server.
    ///
    /// Returns `(api_key, device_id)` on success. This uses the
    /// production `SyncClient::register` path over gRPC.
    pub async fn register_client(&self) -> (String, String) {
        SyncClient::register(self.url())
            .await
            .expect("register should succeed against recording server")
    }

    /// Build a `SyncClient` configured for this server.
    pub async fn build_client(&self, api_key: &str) -> SyncClient {
        use snip_it::config::{AutoSyncFailureMode, SyncDirection, SyncSettings};

        let settings = SyncSettings {
            enabled: true,
            server_url: self.url(),
            api_key: api_key.to_string(),
            device_id: String::new(),
            sync_interval_minutes: 30,
            auto_sync: false,
            auto_sync_debounce_seconds: 2,
            auto_sync_failure: AutoSyncFailureMode::Warn,
            auto_sync_max_delay_seconds: None,
            auto_sync_timeout_seconds: None,
            sync_direction: SyncDirection::Bidirectional,
            clipboard_auto_clear_seconds: None,
            sync_limit: None,
            credential_revision: 0,
        };
        SyncClient::create(settings)
            .await
            .expect("SyncClient::create should succeed against recording server")
    }

    /// Wait until the captured auth header is set (confirming a client connected).
    ///
    /// Returns `true` if the auth header appeared within the timeout,
    /// `false` if the timeout elapsed without observing a connection.
    pub async fn wait_for_auth(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        let poll_interval = Duration::from_millis(10);

        loop {
            let auth = self.captured_auth_header.lock().unwrap().clone();
            if auth.is_some() {
                return true;
            }
            if start.elapsed() >= timeout {
                return false;
            }
            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Wait for a specific operation to appear in the event log.
    ///
    /// Polls with short intervals for deterministic test assertions.
    /// Returns `true` if the operation appeared within the timeout.
    pub async fn wait_for_operation(&self, operation: &str, timeout: Duration) -> bool {
        let start = Instant::now();
        let poll_interval = Duration::from_millis(10);

        loop {
            if self.has_operation(operation) {
                return true;
            }
            if start.elapsed() >= timeout {
                return false;
            }
            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Wait until the request count for an operation reaches the expected count.
    ///
    /// Returns `true` if the count was reached within the timeout.
    pub async fn wait_for_request_count(
        &self,
        operation: &str,
        expected: usize,
        timeout: Duration,
    ) -> bool {
        let start = Instant::now();
        let poll_interval = Duration::from_millis(10);

        loop {
            if self.event_count(operation) >= expected {
                return true;
            }
            if start.elapsed() >= timeout {
                return false;
            }
            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Record a request received event.
    ///
    /// Called by the test to track when a request is sent to the server.
    pub fn record_request_received(&self, operation: &str, device_id: &str, library_id: &str) {
        let event = ServerEvent::RequestReceived {
            operation: operation.to_string(),
            device_id: device_id.to_string(),
            library_id: library_id.to_string(),
            timestamp_ms: now_millis(),
        };
        self.events.lock().unwrap().push(event);
    }

    /// Record a request completed event.
    ///
    /// Called by the test to track when a request finishes.
    pub fn record_request_completed(&self, operation: &str, success: bool) {
        let event = ServerEvent::RequestCompleted {
            operation: operation.to_string(),
            success,
            timestamp_ms: now_millis(),
        };
        self.events.lock().unwrap().push(event);
    }

    /// Stop the server.
    ///
    /// Aborts the server task to stop accepting new connections.
    pub fn shutdown(self) {
        self.server_task.abort();
    }
}

#[allow(dead_code)]
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
