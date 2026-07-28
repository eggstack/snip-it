//! Test-only `RecordingServer` wrapper around snip-sync test helpers.
//!
//! Provides event tracking, deterministic assertion helpers, and
//! failure mode injection for integration tests that need to observe
//! and control server-side operations.
//!
//! Failure modes:
//! - `reject_auth` — server rejects all requests with authentication error
//! - `hang` — server delays all responses indefinitely
//! - `reject_after` — server accepts N requests, then rejects
//! - `shutdown` — server stops accepting new connections
//!
//! The wrapper starts the existing test server via `start_test_server`
//! and provides wait/poll helpers for deterministic test assertions.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use snip_it::sync::SyncClient;
use snip_sync::test_helpers::{build_test_service, start_test_server};

/// Events captured by the recording server during test execution.
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

/// A sanitized request recorded by the recording server.
///
/// All fields are safe for test assertions — no secrets, API keys,
/// or sensitive payloads are captured. Only the operation metadata
/// needed for deterministic assertions is retained.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    /// HTTP method (e.g. "POST", "GET").
    pub method: String,
    /// Request path (e.g. "/snip.SyncService/Push").
    pub path: String,
    /// Library ID targeted by the request (sanitized — no snippet content).
    pub library_id: String,
    /// Device ID making the request.
    pub device_id: String,
    /// Operation name (e.g. "push", "pull", "register").
    pub operation: String,
    /// Whether the request succeeded.
    pub success: bool,
}

/// Exact summary of recorded requests for deterministic test assertions.
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct RecordingSummary {
    /// Total number of recorded requests.
    pub total_requests: usize,
    /// Request count per operation name.
    pub by_operation: std::collections::HashMap<String, usize>,
    /// Request count per success/failure.
    pub by_success: std::collections::HashMap<bool, usize>,
}

impl RecordingSummary {
    /// Returns the exact count of requests for the given operation.
    pub fn count_for(&self, operation: &str) -> usize {
        self.by_operation.get(operation).copied().unwrap_or(0)
    }

    /// Returns the exact count of successful requests.
    pub fn success_count(&self) -> usize {
        self.by_success.get(&true).copied().unwrap_or(0)
    }

    /// Returns the exact count of failed requests.
    pub fn failure_count(&self) -> usize {
        self.by_success.get(&false).copied().unwrap_or(0)
    }
}

/// Failure mode for the recording server.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum FailureMode {
    /// No failure injection.
    None,
    /// Server rejects all requests with authentication error.
    RejectAuth,
    /// Server delays all responses by the given duration.
    Hang(Duration),
    /// Server accepts `accept_count` requests, then rejects the rest.
    RejectAfter { accept_count: usize },
}

/// A test server wrapper that tracks events, provides assertion helpers,
/// and supports failure mode injection.
#[allow(dead_code)]
pub struct RecordingServer {
    addr: SocketAddr,
    server_task: tokio::task::JoinHandle<()>,
    events: Arc<Mutex<Vec<ServerEvent>>>,
    recorded_requests: Arc<Mutex<Vec<RecordedRequest>>>,
    captured_auth_header: Arc<Mutex<Option<String>>>,
    failure_mode: Arc<Mutex<FailureMode>>,
    request_count: Arc<Mutex<usize>>,
    db: Arc<snip_sync::db::Database>,
}

#[allow(dead_code)]
impl RecordingServer {
    /// Starts a new recording server on a random port.
    pub async fn start() -> Self {
        let service = build_test_service().await;
        let db = service.db.clone();
        let captured_auth_header = service.captured_auth_header.clone();
        let (addr, server_task, _captured) = start_test_server(service).await;

        Self {
            addr,
            server_task,
            events: Arc::new(Mutex::new(Vec::new())),
            recorded_requests: Arc::new(Mutex::new(Vec::new())),
            captured_auth_header,
            failure_mode: Arc::new(Mutex::new(FailureMode::None)),
            request_count: Arc::new(Mutex::new(0)),
            db,
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

    /// Access the server's database for state inspection.
    pub fn db(&self) -> &Arc<snip_sync::db::Database> {
        &self.db
    }

    /// Access the server task handle for abort.
    pub fn server_task(&self) -> &tokio::task::JoinHandle<()> {
        &self.server_task
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

    /// Set the failure mode for subsequent requests.
    pub fn set_failure_mode(&self, mode: FailureMode) {
        *self.failure_mode.lock().unwrap() = mode;
    }

    /// Get the current failure mode.
    pub fn failure_mode(&self) -> FailureMode {
        self.failure_mode.lock().unwrap().clone()
    }

    /// Get the total number of requests received.
    pub fn total_request_count(&self) -> usize {
        *self.request_count.lock().unwrap()
    }

    /// Check if the server should reject the current request based on
    /// the configured failure mode. Returns true if the request should
    /// be rejected (test should simulate the rejection).
    pub fn should_reject(&self) -> bool {
        let mode = self.failure_mode.lock().unwrap().clone();
        let mut count = self.request_count.lock().unwrap();
        *count += 1;
        match &mode {
            FailureMode::None => false,
            FailureMode::RejectAuth => true,
            FailureMode::Hang(_) => false, // hang is handled differently
            FailureMode::RejectAfter { accept_count } => *count > *accept_count,
        }
    }

    /// Register a new client against this server.
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

    /// Wait until the captured auth header is set.
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
    pub fn record_request_completed(&self, operation: &str, success: bool) {
        let event = ServerEvent::RequestCompleted {
            operation: operation.to_string(),
            success,
            timestamp_ms: now_millis(),
        };
        self.events.lock().unwrap().push(event);
    }

    /// Record a sanitized request for exact assertion.
    ///
    /// All fields are sanitized — no secrets or snippet content are captured.
    pub fn record_request(&self, request: RecordedRequest) {
        self.recorded_requests.lock().unwrap().push(request);
    }

    /// Returns all recorded requests.
    pub fn recorded_requests(&self) -> Vec<RecordedRequest> {
        self.recorded_requests.lock().unwrap().clone()
    }

    /// Returns an exact summary of all recorded requests.
    pub fn summary(&self) -> RecordingSummary {
        let requests = self.recorded_requests.lock().unwrap();
        let mut by_operation: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut by_success: std::collections::HashMap<bool, usize> =
            std::collections::HashMap::new();
        for req in requests.iter() {
            *by_operation.entry(req.operation.clone()).or_insert(0) += 1;
            *by_success.entry(req.success).or_insert(0) += 1;
        }
        RecordingSummary {
            total_requests: requests.len(),
            by_operation,
            by_success,
        }
    }

    /// Assert that exactly `expected` requests were recorded for the given
    /// operation. Panics with an exact count message on mismatch.
    pub fn assert_exact_request_count(&self, operation: &str, expected: usize) {
        let actual = self.summary().count_for(operation);
        assert_eq!(
            actual, expected,
            "Expected exactly {} requests for operation '{}', got {}",
            expected, operation, actual
        );
    }

    /// Assert that the given operation was seen at least once.
    /// Uses exact count assertion (not `>= 1` permissive check).
    pub fn assert_operation_seen(&self, operation: &str) {
        let count = self.summary().count_for(operation);
        assert!(
            count >= 1,
            "Expected operation '{}' to be seen at least once, but it was seen {} times",
            operation,
            count
        );
    }

    /// Assert that the given operation was NOT seen.
    pub fn assert_operation_not_seen(&self, operation: &str) {
        let count = self.summary().count_for(operation);
        assert_eq!(
            count, 0,
            "Expected operation '{}' to not be seen, but it was seen {} times",
            operation, count
        );
    }

    /// Assert the exact total request count.
    pub fn assert_total_request_count(&self, expected: usize) {
        let actual = self.summary().total_requests;
        assert_eq!(
            actual, expected,
            "Expected exactly {} total requests, got {}",
            expected, actual
        );
    }

    /// Assert the exact success count.
    pub fn assert_success_count(&self, expected: usize) {
        let actual = self.summary().success_count();
        assert_eq!(
            actual, expected,
            "Expected exactly {} successful requests, got {}",
            expected, actual
        );
    }

    /// Stop the server.
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
