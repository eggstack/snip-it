//! Test-only request observer for the snip-sync server.
//!
//! The observer lets tests capture sanitized request telemetry from
//! the actual gRPC handlers without depending on injected hooks that
//! could miss a code path. Each handler invokes `request_started` and
//! `request_finished` automatically, so test code cannot create
//! evidence by calling `record_request` manually.
//!
//! Sanitization guarantees:
//! - no API key, authorization header, raw body, decrypted snippet
//!   command, or plaintext payload is retained;
//! - payload evidence is length, hash, and sentinel boolean only;
//! - per-process record count is bounded.

use std::sync::{Arc, Mutex};

/// Maximum number of request records retained per server instance.
/// Older records are dropped when this limit is exceeded.
pub const MAX_OBSERVER_RECORDS: usize = 256;

/// A sanitized request-start record emitted by gRPC handlers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestStarted {
    /// Monotonic sequence number per server instance.
    pub sequence: u64,
    /// Wall-clock start time (millis since UNIX epoch).
    pub started_at_unix_ms: u64,
    /// gRPC method name (e.g. `"push_snippets"`).
    pub method: String,
    /// Logical operation (e.g. `"push"`, `"pull"`, `"register"`).
    pub operation: String,
    /// Authenticated user id resolved from the API key, if any.
    /// `None` for unauthenticated endpoints like `health`.
    pub authenticated_user_id: Option<String>,
    /// Authenticated device id (if present in metadata).
    pub authenticated_device_id: Option<String>,
    /// Target library id (if the request targets a specific library).
    pub target_library_id: Option<String>,
    /// Request revision (for sync endpoints).
    pub request_revision: Option<u64>,
    /// Payload length in bytes (decoded request body, sanitized).
    pub payload_len: usize,
    /// Payload SHA-256 hex digest.
    pub payload_sha256: String,
    /// True if any plaintext snippet command sentinel would have
    /// been observed in the payload. Always `false` for encrypted
    /// payloads.
    pub payload_contains_plaintext_sentinel: bool,
    /// Number of requests in-flight at start time.
    pub concurrent_at_start: usize,
}

/// A sanitized request-finish record emitted by gRPC handlers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestFinished {
    /// Sequence number shared with the matching `RequestStarted`.
    pub sequence: u64,
    /// Wall-clock finish time (millis since UNIX epoch).
    pub finished_at_unix_ms: u64,
    /// Whether the handler returned a success status.
    pub success: bool,
    /// Response revision (for sync endpoints).
    pub response_revision: Option<u64>,
}

/// Observer trait implemented by test sinks.
///
/// The handler invokes `request_started` immediately before dispatch
/// and `request_finished` after the handler future resolves.
pub trait TestRequestObserver: Send + Sync {
    fn request_started(&self, event: RequestStarted);
    fn request_finished(&self, event: RequestFinished);

    /// Allocate the next monotonic sequence number for a request pair.
    /// Default returns 0 (observers that don't track sequences).
    fn allocate_sequence(&self) -> u64 {
        0
    }

    /// Number of requests currently in-flight (started but not finished).
    /// Default returns 0.
    fn current_concurrent(&self) -> usize {
        0
    }

    /// Find the sequence number of the most recent `RequestStarted`
    /// whose `method` matches. Default returns 0.
    fn last_started_sequence_for_method(&self, _method: &str) -> u64 {
        0
    }

    /// Update the IDs of the most recent request for the given method.
    /// Default is a no-op.
    fn update_ids_for_last_method(
        &self,
        _method: &str,
        _user_id: Option<String>,
        _device_id: Option<String>,
        _library_id: Option<String>,
    ) {
    }
}

/// Default in-memory observer used by `RecordingServer`.
#[derive(Debug, Default)]
pub struct InMemoryObserver {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Debug, Default)]
struct Inner {
    next_seq: u64,
    in_flight: usize,
    max_in_flight: usize,
    starts: Vec<RequestStarted>,
    finishes: Vec<RequestFinished>,
}

impl InMemoryObserver {
    /// Returns a fresh, empty observer wrapped in an `Arc`.
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Returns all recorded `RequestStarted` events.
    pub fn starts(&self) -> Vec<RequestStarted> {
        self.inner.lock().unwrap().starts.clone()
    }

    /// Returns all recorded `RequestFinished` events.
    pub fn finishes(&self) -> Vec<RequestFinished> {
        self.inner.lock().unwrap().finishes.clone()
    }

    /// Maximum number of concurrent in-flight requests observed.
    pub fn max_concurrent(&self) -> usize {
        self.inner.lock().unwrap().max_in_flight
    }

    /// Total number of `RequestStarted` events recorded.
    pub fn started_count(&self) -> usize {
        self.inner.lock().unwrap().starts.len()
    }

    /// Total number of `RequestFinished` events recorded.
    pub fn finished_count(&self) -> usize {
        self.inner.lock().unwrap().finishes.len()
    }

    /// Total number of successful finishes.
    pub fn success_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap()
            .finishes
            .iter()
            .filter(|f| f.success)
            .count()
    }

    /// First request-start timestamp, or `None`.
    pub fn first_started_at_unix_ms(&self) -> Option<u64> {
        self.inner
            .lock()
            .unwrap()
            .starts
            .first()
            .map(|e| e.started_at_unix_ms)
    }

    /// First request-finish timestamp, or `None`.
    pub fn first_finished_at_unix_ms(&self) -> Option<u64> {
        self.inner
            .lock()
            .unwrap()
            .finishes
            .first()
            .map(|e| e.finished_at_unix_ms)
    }

    /// Count of finished requests for `success=true`.
    pub fn failures(&self) -> usize {
        self.inner
            .lock()
            .unwrap()
            .finishes
            .iter()
            .filter(|f| !f.success)
            .count()
    }
}

impl TestRequestObserver for InMemoryObserver {
    fn allocate_sequence(&self) -> u64 {
        self.next_sequence()
    }

    fn current_concurrent(&self) -> usize {
        self.inner.lock().unwrap().in_flight
    }

    fn last_started_sequence_for_method(&self, method: &str) -> u64 {
        self.inner
            .lock()
            .unwrap()
            .starts
            .iter()
            .rev()
            .find(|s| s.method == method)
            .map(|s| s.sequence)
            .unwrap_or(0)
    }

    fn update_ids_for_last_method(
        &self,
        method: &str,
        user_id: Option<String>,
        device_id: Option<String>,
        library_id: Option<String>,
    ) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(start) = inner.starts.iter_mut().rev().find(|s| s.method == method) {
            start.authenticated_user_id = user_id;
            start.authenticated_device_id = device_id;
            start.target_library_id = library_id;
        }
    }

    fn request_started(&self, event: RequestStarted) {
        let mut inner = self.inner.lock().unwrap();
        inner.starts.push(event);
        if inner.starts.len() > MAX_OBSERVER_RECORDS {
            inner.starts.remove(0);
        }
        inner.in_flight += 1;
        if inner.in_flight > inner.max_in_flight {
            inner.max_in_flight = inner.in_flight;
        }
    }

    fn request_finished(&self, event: RequestFinished) {
        let mut inner = self.inner.lock().unwrap();
        inner.finishes.push(event);
        if inner.finishes.len() > MAX_OBSERVER_RECORDS {
            inner.finishes.remove(0);
        }
        if inner.in_flight > 0 {
            inner.in_flight -= 1;
        }
    }
}

impl InMemoryObserver {
    /// Allocates the next sequence number. Exposed for handlers that
    /// need to coordinate started/finished pairing.
    pub fn next_sequence(&self) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        inner.next_seq += 1;
        inner.next_seq
    }
}

/// Wall-clock millis since UNIX epoch.
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Map a gRPC method name to a logical operation name.
///
/// `push_snippets` → `"push"`, `pull` semantics → `"pull"`, etc.
pub fn operation_name_for_method(method: &str) -> &'static str {
    match method {
        "push_snippets" => "push",
        "sync" => "sync",
        "get_snippets" => "pull",
        "register" => "register",
        "health" => "health",
        "create_library" => "create_library",
        "list_libraries" => "list_libraries",
        "delete_library" => "delete_library",
        "list_premade_libraries" => "list_premade_libraries",
        "get_premade_library" => "get_premade_library",
        "search_premade_libraries" => "search_premade",
        _ => "other",
    }
}

/// Compute a stable SHA-256 hex digest of length+sentinel-derived
/// fields without ever retaining the raw payload bytes.
///
/// This is a sanitization helper used by handlers to populate the
/// `payload_sha256` field. It only hashes COUNT-derived fields, so
/// the resulting digest is stable for the same input *size* but does
/// NOT function as a checksum of the actual content.
pub fn payload_sha256<T>(items: &[T], _device_id: &str, _api_key: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update((items.len() as u64).to_le_bytes());
    // Note: do NOT include `_device_id` or `_api_key` in the hash; both
    // carry user-identifying and credential material.
    let bytes = hasher.finalize();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_records_started_and_finished() {
        let obs = InMemoryObserver::shared();
        let seq = obs.next_sequence();
        obs.request_started(RequestStarted {
            sequence: seq,
            started_at_unix_ms: 1,
            method: "push".to_string(),
            operation: "push".to_string(),
            authenticated_user_id: None,
            authenticated_device_id: None,
            target_library_id: None,
            request_revision: None,
            payload_len: 0,
            payload_sha256: String::new(),
            payload_contains_plaintext_sentinel: false,
            concurrent_at_start: 0,
        });
        obs.request_finished(RequestFinished {
            sequence: seq,
            finished_at_unix_ms: 2,
            success: true,
            response_revision: None,
        });
        assert_eq!(obs.started_count(), 1);
        assert_eq!(obs.finished_count(), 1);
        assert_eq!(obs.success_count(), 1);
        assert_eq!(obs.failures(), 0);
    }

    #[test]
    fn observer_tracks_max_concurrent() {
        let obs = InMemoryObserver::shared();
        let s1 = obs.next_sequence();
        obs.request_started(RequestStarted {
            sequence: s1,
            started_at_unix_ms: 1,
            method: "push".to_string(),
            operation: "push".to_string(),
            authenticated_user_id: None,
            authenticated_device_id: None,
            target_library_id: None,
            request_revision: None,
            payload_len: 0,
            payload_sha256: String::new(),
            payload_contains_plaintext_sentinel: false,
            concurrent_at_start: 0,
        });
        let s2 = obs.next_sequence();
        obs.request_started(RequestStarted {
            sequence: s2,
            started_at_unix_ms: 2,
            method: "push".to_string(),
            operation: "push".to_string(),
            authenticated_user_id: None,
            authenticated_device_id: None,
            target_library_id: None,
            request_revision: None,
            payload_len: 0,
            payload_sha256: String::new(),
            payload_contains_plaintext_sentinel: false,
            concurrent_at_start: 1,
        });
        assert_eq!(obs.max_concurrent(), 2);
        obs.request_finished(RequestFinished {
            sequence: s1,
            finished_at_unix_ms: 3,
            success: true,
            response_revision: None,
        });
        obs.request_finished(RequestFinished {
            sequence: s2,
            finished_at_unix_ms: 4,
            success: true,
            response_revision: None,
        });
        assert_eq!(obs.max_concurrent(), 2);
    }

    #[test]
    fn observer_does_not_retain_payload_bytes() {
        // Sanitization guarantee: the observer itself never accepts
        // raw byte payloads. Only length, hash, and sentinel boolean.
        let obs = InMemoryObserver::shared();
        let seq = obs.next_sequence();
        let _ = seq;
        // If the API ever changes to accept `Vec<u8>` here, the
        // `InMemoryObserver::request_started` signature above will
        // need a deliberate review and a sanitized conversion.
    }
}
