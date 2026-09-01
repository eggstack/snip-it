//! **Layer: Sync-Client**
//!
//! gRPC sync client for communicating with snip-sync server.
//!
//! Handles bidirectional synchronization of snippets with encryption in transit.
//! Uses TLS for secure communication and AES-256-GCM for snippet encryption.
//!
//! # Sync Flow
//!
//! 1. Connect to server with TLS
//! 2. Encrypt local snippets with user's API key
//! 3. Send encrypted snippets with last sync timestamp
//! 4. Receive encrypted remote snippets
//! 5. Decrypt and merge with local storage

use crate::config::SyncSettings;
use crate::encryption;
use crate::error::{SnipError, SnipResult, SyncFailureKind};
use prost::Message;
use snip_proto::PremadeLibrary;
use snip_proto::snippet_sync_client::SnippetSyncClient;
use snip_proto::{
    CreateLibraryRequest, GetPremadeLibraryRequest, HealthRequest, Library, ListLibrariesRequest,
    ListPremadeLibrariesRequest, PushSnippetsRequest, RegisterRequest,
    SearchPremadeLibrariesRequest, SyncRequest,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tonic::Code;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint, Uri};
use zeroize::Zeroizing;

static JITTER_COUNTER: AtomicU32 = AtomicU32::new(0);

const DEFAULT_MAX_RETRIES: u32 = 3; // Total attempts: 1 initial + 3 retries = 4
const DEFAULT_INITIAL_DELAY_MS: u64 = 100; // Initial backoff before first retry
const DEFAULT_MAX_DELAY_MS: u64 = 5000; // Cap exponential backoff at 5 seconds
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;

/// Conservative client-side byte ceiling for sync requests.
///
/// The server defaults to a 4 MiB gRPC message limit. This ceiling leaves
/// headroom for gRPC framing and transport overhead while keeping a safe
/// margin below the server default.
pub(crate) const DEFAULT_CLIENT_REQUEST_CEILING: usize = 3_584 * 1024; // 3.5 MiB

/// Bounds network and retry work for one automatic-sync invocation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SyncRunLimits {
    pub deadline: std::time::Instant,
    pub request_timeout: Duration,
}

impl SyncRunLimits {
    pub(crate) fn remaining(self) -> Option<Duration> {
        self.deadline
            .checked_duration_since(std::time::Instant::now())
    }
}

/// Configuration for gRPC retry behavior with exponential backoff.
#[derive(Debug, Clone)]
pub struct SyncRetryConfig {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for SyncRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            initial_delay_ms: DEFAULT_INITIAL_DELAY_MS,
            max_delay_ms: DEFAULT_MAX_DELAY_MS,
        }
    }
}

impl SyncRetryConfig {
    /// Returns `true` if the gRPC error status code is retryable.
    pub fn is_retryable_grpc_error(status: &tonic::Status) -> bool {
        !matches!(
            status.code(),
            Code::InvalidArgument
                | Code::NotFound
                | Code::AlreadyExists
                | Code::PermissionDenied
                | Code::Unauthenticated
        )
    }
}

fn default_retry_config() -> SyncRetryConfig {
    SyncRetryConfig::default()
}

fn retry_jitter_multiplier() -> f64 {
    let counter = JITTER_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let combined = nanos.wrapping_add(counter.wrapping_mul(31));
    0.5 + ((combined as u64 % 1000) as f64 / 1000.0)
}

/// Retry an async gRPC operation with exponential backoff and jitter.
macro_rules! retry_grpc {
    ($op:expr, $name:expr) => {{
        let config = default_retry_config();
        let mut delay_ms = config.initial_delay_ms;
        let mut attempt = 0u32;
        loop {
            match $op.await {
                Ok(val) => break Ok(val),
                Err(e) => {
                    if !SyncRetryConfig::is_retryable_grpc_error(&e)
                        || attempt >= config.max_retries
                    {
                        break Err(grpc_error_to_snip_error($name, &e));
                    }
                    let actual_delay = (delay_ms as f64 * retry_jitter_multiplier()) as u64;
                    tracing::warn!(
                        "{} failed (attempt {}/{}): {}. Retrying in {}ms...",
                        $name,
                        attempt + 1,
                        config.max_retries + 1,
                        e,
                        actual_delay
                    );
                    tokio::time::sleep(Duration::from_millis(actual_delay)).await;
                    delay_ms = (delay_ms * 2).min(config.max_delay_ms);
                    attempt += 1;
                }
            }
        }
    }};
}

/// Retry a request while honoring the per-invocation automatic-sync deadline.
macro_rules! retry_grpc_limited {
    ($client:expr, $op:expr, $name:expr) => {{
        let config = default_retry_config();
        let mut delay_ms = config.initial_delay_ms;
        let mut attempt = 0u32;
        loop {
            let response = match $client.limits.and_then(SyncRunLimits::remaining) {
                Some(remaining) if !remaining.is_zero() => {
                    match tokio::time::timeout(remaining, $op).await {
                        Ok(response) => response,
                        Err(_) => {
                            break Err(SnipError::sync_failure(
                                crate::error::SyncFailureKind::Timeout,
                                Some("automatic sync deadline expired"),
                            ));
                        }
                    }
                }
                None if $client.limits.is_none() => $op.await,
                _ => {
                    break Err(SnipError::sync_failure(
                        crate::error::SyncFailureKind::Timeout,
                        Some("automatic sync deadline expired"),
                    ));
                }
            };
            match response {
                Ok(val) => break Ok(val),
                Err(e) => {
                    if !SyncRetryConfig::is_retryable_grpc_error(&e)
                        || attempt >= config.max_retries
                    {
                        break Err(grpc_error_to_snip_error($name, &e));
                    }
                    let actual_delay = (delay_ms as f64 * retry_jitter_multiplier()) as u64;
                    let delay = Duration::from_millis(actual_delay);
                    if $client.limits.is_some_and(|limits| {
                        limits
                            .remaining()
                            .is_none_or(|remaining| remaining <= delay)
                    }) {
                        break Err(SnipError::sync_failure(
                            crate::error::SyncFailureKind::Timeout,
                            Some("automatic sync deadline expired during retry backoff"),
                        ));
                    }
                    tokio::time::sleep(delay).await;
                    delay_ms = (delay_ms * 2).min(config.max_delay_ms);
                    attempt += 1;
                }
            }
        }
    }};
}

/// Convert a gRPC error status into a typed `SnipError`.
///
/// Distinguishes `NotFound` (library not found), `DeadlineExceeded` (timeout),
/// and `InvalidArgument` (clock skew) from other non-retryable errors.
fn grpc_error_to_snip_error(operation: &str, status: &tonic::Status) -> SnipError {
    if status.code() == Code::NotFound {
        SnipError::sync_failure(
            crate::error::SyncFailureKind::LibraryNotFound,
            Some(&status.to_string()),
        )
    } else if status.code() == Code::DeadlineExceeded {
        SnipError::sync_failure(
            crate::error::SyncFailureKind::Timeout,
            Some(&status.to_string()),
        )
    } else if status.code() == Code::InvalidArgument {
        let msg = status.message();
        if msg.starts_with("CLOCK_SKEW:") {
            SnipError::sync_failure(crate::error::SyncFailureKind::ClockSkew, Some(msg))
        } else {
            SnipError::runtime_error(operation, Some(msg))
        }
    } else {
        SnipError::runtime_error(operation, Some(&status.to_string()))
    }
}

/// Add the API key as gRPC `authorization` metadata to a request.
pub(crate) fn add_api_key_metadata<T>(request: &mut tonic::Request<T>, api_key: &str) {
    debug_assert!(!api_key.is_empty(), "api_key must not be empty");
    if !api_key.is_empty()
        && let Ok(val) = format!("Bearer {api_key}").parse()
    {
        request.metadata_mut().insert("authorization", val);
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct EncryptedSnippetData {
    description: String,
    command: String,
    tags: Vec<String>,
}

/// Client for syncing snippets with a remote server.
///
/// Wraps a gRPC client with encryption handling for secure sync operations.
pub struct SyncClient {
    client: SnippetSyncClient<Channel>,
    settings: SyncSettings,
    limits: Option<SyncRunLimits>,
}

impl SyncClient {
    fn ensure_budget(&self) -> SnipResult<()> {
        if self
            .limits
            .is_some_and(|limits| limits.remaining().is_none_or(|duration| duration.is_zero()))
        {
            return Err(SnipError::sync_failure(
                crate::error::SyncFailureKind::Timeout,
                Some("automatic sync deadline expired before network operation"),
            ));
        }
        Ok(())
    }

    /// Creates a new sync client connected to the server specified in settings.
    pub async fn create(settings: SyncSettings) -> SnipResult<Self> {
        Self::create_with_limits(settings, None).await
    }

    pub(crate) async fn create_with_limits(
        settings: SyncSettings,
        limits: Option<SyncRunLimits>,
    ) -> SnipResult<Self> {
        let server_url = settings.server_url.clone();

        let channel = create_tls_channel(&server_url, limits.map(|l| l.request_timeout))
            .await
            .map_err(|e| {
                SnipError::sync_failure(
                    crate::error::SyncFailureKind::ConnectFailed,
                    Some(&e.to_string()),
                )
            })?;

        Ok(Self {
            client: SnippetSyncClient::new(channel),
            settings,
            limits,
        })
    }

    /// Encrypts local snippets, sends them to the server in bounded batches,
    /// and decrypts the response.
    ///
    /// Snippets that fail encryption/decryption are counted as skipped.
    /// Handles server-side pagination by fetching all pages before returning.
    /// Uploads are batched to stay within the gRPC message size ceiling.
    ///
    /// # Upload strategy
    ///
    /// All upload batches are sent before any response page is requested.
    /// For zero batches: an empty-upload `Sync(offset=0)` fetches the
    /// authoritative first response page (pull-only path).
    /// For one batch: `Sync(batch, offset=0)` carries the upload and returns
    /// the first response page in one RPC. For two or more batches: each batch
    /// is sent via `PushSnippets` (upload only), then a final empty-upload
    /// `Sync(offset=0)` fetches the authoritative first response page.
    /// This ordering ensures that `has_more == false` on the first response
    /// cannot truncate uploads, and the final response describes server state
    /// after all successful uploads.
    pub async fn sync_encrypted(
        &mut self,
        local_snippets: Vec<crate::proto::Snippet>,
        last_sync: i64,
        library_id: &str,
    ) -> SnipResult<crate::proto::SyncResponse> {
        self.sync_encrypted_inner(
            local_snippets,
            last_sync,
            library_id,
            DEFAULT_CLIENT_REQUEST_CEILING,
        )
        .await
    }

    /// Encrypt and sync with a custom byte ceiling (for testing).
    ///
    /// Delegates to the real `sync_encrypted` logic but uses the provided
    /// `byte_ceiling` instead of `DEFAULT_CLIENT_REQUEST_CEILING`. This
    /// allows integration tests to force multi-batch uploads with fewer
    /// snippets by setting a lower ceiling.
    pub async fn sync_encrypted_with_ceiling(
        &mut self,
        local_snippets: Vec<crate::proto::Snippet>,
        last_sync: i64,
        library_id: &str,
        byte_ceiling: usize,
    ) -> SnipResult<crate::proto::SyncResponse> {
        self.sync_encrypted_inner(local_snippets, last_sync, library_id, byte_ceiling)
            .await
    }

    /// Unified sync implementation shared by `sync_encrypted` and
    /// `sync_encrypted_with_ceiling`. Handles zero, one, and multiple
    /// upload batches, including the pull-only path (zero local snippets
    /// or all encryption failures).
    async fn sync_encrypted_inner(
        &mut self,
        local_snippets: Vec<crate::proto::Snippet>,
        last_sync: i64,
        library_id: &str,
        byte_ceiling: usize,
    ) -> SnipResult<crate::proto::SyncResponse> {
        let _key_cache_guard = encryption::key_cache_guard();
        self.ensure_budget()?;
        let api_key = self.settings.api_key.as_str();
        let (encrypted_snippets, encrypt_failed_ids) = encrypt_snippets(api_key, &local_snippets);

        self.sync_prepared_encrypted_inner(
            encrypted_snippets,
            encrypt_failed_ids,
            last_sync,
            library_id,
            byte_ceiling,
        )
        .await
    }

    /// The single zero/one/many batch transport implementation. Owns the
    /// complete logic shared by `sync_encrypted`/`sync_encrypted_with_ceiling`
    /// (real encryption) and the test-only injected-encryption caller
    /// (`sync_encrypted_with_test_encrypt`). Accepts pre-encrypted
    /// snippets plus the IDs that already failed encryption so test
    /// injection can drive the same path.
    async fn sync_prepared_encrypted_inner(
        &mut self,
        encrypted_snippets: Vec<crate::proto::Snippet>,
        encrypt_failed_ids: Vec<String>,
        last_sync: i64,
        library_id: &str,
        byte_ceiling: usize,
    ) -> SnipResult<crate::proto::SyncResponse> {
        let api_key = Zeroizing::new(self.settings.api_key.clone());

        let encrypt_failed_count = encrypt_failed_ids.len();
        let mut all_skipped_ids = encrypt_failed_ids;

        let batches = build_upload_batches(
            encrypted_snippets,
            library_id,
            last_sync,
            self.settings.sync_limit_value(),
            byte_ceiling,
        )?;

        let mut all_server_snippets = Vec::new();
        let mut server_decrypt_failed_count = 0usize;
        let mut offset = 0;
        let total_batches = batches.len();

        match total_batches {
            0 => {
                // Zero batches: pull-only path. Send an empty-upload
                // SyncRequest at offset zero to retrieve remote snippets.
                let request = SyncRequest {
                    api_key: String::new(),
                    local_snippets: Vec::new(),
                    last_sync_timestamp: last_sync,
                    library_id: library_id.to_string(),
                    limit: self.settings.sync_limit_value(),
                    offset: 0,
                };
                let response = self.sync_with_retry(request, &api_key).await?;
                let has_more = response.has_more;
                let snippets_len = response.snippets.len();
                let server_timestamp = response.server_timestamp;
                let message = response.message.clone();
                let total_count = response.total_count;

                Self::accumulate_page(
                    &response,
                    &api_key,
                    &mut all_server_snippets,
                    &mut server_decrypt_failed_count,
                    &mut all_skipped_ids,
                );
                offset = i32::try_from(snippets_len).unwrap_or(i32::MAX);

                return self
                    .paginate_remaining(
                        last_sync,
                        library_id,
                        &api_key,
                        &mut offset,
                        &mut all_server_snippets,
                        &mut server_decrypt_failed_count,
                        &mut all_skipped_ids,
                        server_timestamp,
                        message,
                        total_count,
                        has_more,
                        snippets_len,
                        encrypt_failed_count,
                    )
                    .await;
            }
            1 => {
                // Single batch: send via Sync (efficient — carries upload
                // and returns the first response page in one RPC).
                let batch = &batches[0];
                let request = SyncRequest {
                    api_key: String::new(),
                    local_snippets: batch.clone(),
                    last_sync_timestamp: last_sync,
                    library_id: library_id.to_string(),
                    limit: self.settings.sync_limit_value(),
                    offset,
                };
                let response = self.sync_with_retry(request, &api_key).await?;
                let has_more = response.has_more;
                let snippets_len = response.snippets.len();
                let server_timestamp = response.server_timestamp;
                let message = response.message.clone();
                let total_count = response.total_count;

                Self::accumulate_page(
                    &response,
                    &api_key,
                    &mut all_server_snippets,
                    &mut server_decrypt_failed_count,
                    &mut all_skipped_ids,
                );
                offset = offset.saturating_add(i32::try_from(snippets_len).unwrap_or(i32::MAX));

                return self
                    .paginate_remaining(
                        last_sync,
                        library_id,
                        &api_key,
                        &mut offset,
                        &mut all_server_snippets,
                        &mut server_decrypt_failed_count,
                        &mut all_skipped_ids,
                        server_timestamp,
                        message,
                        total_count,
                        has_more,
                        snippets_len,
                        encrypt_failed_count,
                    )
                    .await;
            }
            _ => {
                // Multi-batch: upload via PushSnippets (upload only, no
                // response page). Preserve the original typed error instead
                // of flattening to SyncRequestFailed.
                for (batch_idx, batch) in batches.iter().enumerate() {
                    self.push_snippets_batch(batch, library_id)
                        .await
                        .map_err(|e| add_batch_context(e, batch_idx + 1, total_batches))?;
                }

                // All uploads complete. Request the authoritative first
                // response page via an empty-upload Sync at offset zero.
                let request = SyncRequest {
                    api_key: String::new(),
                    local_snippets: Vec::new(),
                    last_sync_timestamp: last_sync,
                    library_id: library_id.to_string(),
                    limit: self.settings.sync_limit_value(),
                    offset: 0,
                };
                let response = self.sync_with_retry(request, &api_key).await?;
                let has_more = response.has_more;
                let snippets_len = response.snippets.len();
                let server_timestamp = response.server_timestamp;
                let message = response.message.clone();
                let total_count = response.total_count;

                Self::accumulate_page(
                    &response,
                    &api_key,
                    &mut all_server_snippets,
                    &mut server_decrypt_failed_count,
                    &mut all_skipped_ids,
                );
                offset = i32::try_from(snippets_len).unwrap_or(i32::MAX);

                return self
                    .paginate_remaining(
                        last_sync,
                        library_id,
                        &api_key,
                        &mut offset,
                        &mut all_server_snippets,
                        &mut server_decrypt_failed_count,
                        &mut all_skipped_ids,
                        server_timestamp,
                        message,
                        total_count,
                        has_more,
                        snippets_len,
                        encrypt_failed_count,
                    )
                    .await;
            }
        }
    }

    /// Paginate remaining response pages after the first response page has
    /// been accumulated. Stops when `has_more` is false or an empty page is
    /// returned. Consumes the accumulated vectors when building the final
    /// response.
    #[allow(clippy::too_many_arguments)]
    async fn paginate_remaining(
        &mut self,
        last_sync: i64,
        library_id: &str,
        api_key: &str,
        offset: &mut i32,
        all_server_snippets: &mut Vec<crate::proto::Snippet>,
        server_decrypt_failed_count: &mut usize,
        all_skipped_ids: &mut Vec<String>,
        mut server_timestamp: i64,
        mut message: String,
        mut total_count: i32,
        first_has_more: bool,
        first_snippets_len: usize,
        encrypt_failed_count: usize,
    ) -> SnipResult<crate::proto::SyncResponse> {
        if !first_has_more || first_snippets_len == 0 {
            // Take ownership of the accumulated vectors for the final response.
            let snippets = std::mem::take(all_server_snippets);
            let skipped = std::mem::take(all_skipped_ids);
            return Self::build_sync_response(
                snippets,
                skipped,
                encrypt_failed_count,
                *server_decrypt_failed_count,
                server_timestamp,
                message,
                total_count,
            );
        }

        // Hard bound: a buggy or malicious server answering `has_more = true`
        // with non-empty pages forever must not spin manual sync
        // indefinitely. 10_000 pages at the default page size of 1000
        // snippets is far beyond any real library.
        const MAX_SYNC_PAGES: usize = 10_000;
        let mut pages_fetched = 0usize;

        loop {
            pages_fetched += 1;
            if pages_fetched > MAX_SYNC_PAGES {
                return Err(SnipError::sync_failure(
                    SyncFailureKind::SyncRequestFailed,
                    Some(&format!(
                        "server pagination did not terminate after {MAX_SYNC_PAGES} pages; \
                         aborting incremental fetch"
                    )),
                ));
            }
            let request = SyncRequest {
                api_key: String::new(),
                local_snippets: Vec::new(),
                last_sync_timestamp: last_sync,
                library_id: library_id.to_string(),
                limit: self.settings.sync_limit_value(),
                offset: *offset,
            };

            let response = self.sync_with_retry(request, api_key).await?;
            let has_more = response.has_more;
            let snippets_len = response.snippets.len();
            server_timestamp = response.server_timestamp;
            message = response.message.clone();
            total_count = response.total_count;

            Self::accumulate_page(
                &response,
                api_key,
                all_server_snippets,
                server_decrypt_failed_count,
                all_skipped_ids,
            );

            if !has_more || snippets_len == 0 {
                let snippets = std::mem::take(all_server_snippets);
                let skipped = std::mem::take(all_skipped_ids);
                return Self::build_sync_response(
                    snippets,
                    skipped,
                    encrypt_failed_count,
                    *server_decrypt_failed_count,
                    server_timestamp,
                    message,
                    total_count,
                );
            }

            let next_offset =
                offset.saturating_add(i32::try_from(snippets_len).unwrap_or(i32::MAX));
            // Detect i32 saturation: a library larger than ~2.1B snippets
            // cannot be paginated through a 32-bit offset. Surface an
            // explicit error instead of looping with an offset stuck at
            // i32::MAX (which would otherwise terminate only via the
            // MAX_SYNC_PAGES bound, masking the real cause).
            if next_offset == i32::MAX {
                return Err(SnipError::sync_failure(
                    SyncFailureKind::RequestTooLarge,
                    Some("library exceeds i32::MAX snippets; pagination offset saturated"),
                ));
            }
            *offset = next_offset;
        }
    }

    /// Decrypt and accumulate server snippets from a single response page.
    fn accumulate_page(
        response: &crate::proto::SyncResponse,
        api_key: &str,
        all_server_snippets: &mut Vec<crate::proto::Snippet>,
        server_decrypt_failed_count: &mut usize,
        all_skipped_ids: &mut Vec<String>,
    ) {
        for s in &response.snippets {
            match decrypt_snippet(api_key, s) {
                Ok(ds) => all_server_snippets.push(ds),
                Err(e) => {
                    *server_decrypt_failed_count += 1;
                    all_skipped_ids.push(s.id.clone());
                    tracing::warn!("Failed to decrypt snippet {}: {}", s.id, e);
                }
            }
        }
    }

    /// Build the final aggregated SyncResponse from accumulated state.
    fn build_sync_response(
        all_server_snippets: Vec<crate::proto::Snippet>,
        all_skipped_ids: Vec<String>,
        encrypt_failed_count: usize,
        server_decrypt_failed_count: usize,
        server_timestamp: i64,
        message: String,
        total_count: i32,
    ) -> SnipResult<crate::proto::SyncResponse> {
        let total_skipped = all_skipped_ids.len();
        let all_skipped_local = encrypt_failed_count > 0 && all_server_snippets.is_empty();
        let all_skipped_server = server_decrypt_failed_count > 0 && all_server_snippets.is_empty();
        let overall_success = !(all_skipped_local || all_skipped_server);

        Ok(crate::proto::SyncResponse {
            success: overall_success,
            message,
            snippets: all_server_snippets,
            server_timestamp,
            skipped_count: i32::try_from(total_skipped).unwrap_or(i32::MAX),
            skipped_ids: all_skipped_ids,
            has_more: false,
            total_count,
        })
    }

    /// Upload a batch of encrypted snippets via the PushSnippets RPC.
    async fn push_snippets_batch(
        &mut self,
        snippets: &[crate::proto::Snippet],
        library_id: &str,
    ) -> SnipResult<()> {
        self.ensure_budget()?;
        let api_key = Zeroizing::new(self.settings.api_key.clone());
        let request = PushSnippetsRequest {
            api_key: String::new(),
            library_id: library_id.to_string(),
            snippets: snippets.to_vec(),
        };

        let config = default_retry_config();
        let mut delay_ms = config.initial_delay_ms;
        let mut attempt = 0u32;
        loop {
            let remaining = self.limits.and_then(SyncRunLimits::remaining);
            if self
                .limits
                .is_some_and(|limits| limits.remaining().is_none_or(|duration| duration.is_zero()))
            {
                return Err(SnipError::sync_failure(
                    crate::error::SyncFailureKind::Timeout,
                    Some("automatic sync deadline expired before PushSnippets"),
                ));
            }
            let mut grpc_req = tonic::Request::new(request.clone());
            add_api_key_metadata(&mut grpc_req, &api_key);
            let request_future = self.client.push_snippets(grpc_req);
            let response = match self.limits {
                Some(_) => match remaining {
                    Some(duration) if !duration.is_zero() => {
                        match tokio::time::timeout(duration, request_future).await {
                            Ok(response) => response,
                            Err(_) => {
                                return Err(SnipError::sync_failure(
                                    crate::error::SyncFailureKind::Timeout,
                                    Some("automatic sync deadline expired"),
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(SnipError::sync_failure(
                            crate::error::SyncFailureKind::Timeout,
                            Some("automatic sync deadline expired"),
                        ));
                    }
                },
                None => request_future.await,
            };
            match response {
                Ok(response) => {
                    let inner = response.into_inner();
                    if !inner.success {
                        return Err(SnipError::sync_failure(
                            crate::error::SyncFailureKind::SyncRequestFailed,
                            Some(&inner.message),
                        ));
                    }
                    return Ok(());
                }
                Err(e) => {
                    if !SyncRetryConfig::is_retryable_grpc_error(&e)
                        || attempt >= config.max_retries
                    {
                        return Err(grpc_error_to_snip_error("PushSnippets", &e));
                    }
                    let actual_delay = (delay_ms as f64 * retry_jitter_multiplier()) as u64;
                    tracing::warn!(
                        "PushSnippets failed (attempt {}/{}): {}. Retrying in {}ms...",
                        attempt + 1,
                        config.max_retries + 1,
                        e,
                        actual_delay
                    );
                    let delay = Duration::from_millis(actual_delay);
                    if self.limits.is_some_and(|limits| {
                        limits
                            .remaining()
                            .is_none_or(|remaining| remaining <= delay)
                    }) {
                        return Err(SnipError::sync_failure(
                            crate::error::SyncFailureKind::Timeout,
                            Some("automatic sync deadline expired during retry backoff"),
                        ));
                    }
                    tokio::time::sleep(delay).await;
                    delay_ms = (delay_ms * 2).min(config.max_delay_ms);
                    attempt += 1;
                }
            }
        }
    }

    /// Manual retry logic for sync requests.
    ///
    /// Note: The `retry_grpc!` macro cannot be used here because `self.client.sync()`
    /// borrows `&mut self`, and the macro requires the operation to be a standalone
    /// future expression. This method implements the same exponential backoff strategy.
    /// The request is cloned on retry to avoid re-cloning on every attempt.
    ///
    /// `api_key` is passed explicitly rather than read from `request.api_key` so
    /// callers can leave the body field empty (avoiding leaking the key over the
    /// wire in the request body) while still authenticating via `authorization`
    /// metadata.
    async fn sync_with_retry(
        &mut self,
        request: SyncRequest,
        api_key: &str,
    ) -> SnipResult<crate::proto::SyncResponse> {
        let config = default_retry_config();
        let mut delay_ms = config.initial_delay_ms;
        let mut attempt = 0;
        let request = std::sync::Arc::new(request);
        loop {
            let remaining = self.limits.and_then(SyncRunLimits::remaining);
            if self
                .limits
                .is_some_and(|limits| limits.remaining().is_none_or(|duration| duration.is_zero()))
            {
                return Err(SnipError::sync_failure(
                    crate::error::SyncFailureKind::Timeout,
                    Some("automatic sync deadline expired before retry"),
                ));
            }
            let mut grpc_req = tonic::Request::new((*request).clone());
            add_api_key_metadata(&mut grpc_req, api_key);
            let request_future = self.client.sync(grpc_req);
            let response = match self.limits {
                Some(_) => match remaining {
                    Some(duration) if !duration.is_zero() => {
                        match tokio::time::timeout(duration, request_future).await {
                            Ok(response) => response,
                            Err(_) => {
                                return Err(SnipError::sync_failure(
                                    crate::error::SyncFailureKind::Timeout,
                                    Some("automatic sync deadline expired"),
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(SnipError::sync_failure(
                            crate::error::SyncFailureKind::Timeout,
                            Some("automatic sync deadline expired"),
                        ));
                    }
                },
                None => request_future.await,
            };
            match response {
                Ok(response) => return Ok(response.into_inner()),
                Err(e) => {
                    if !SyncRetryConfig::is_retryable_grpc_error(&e)
                        || attempt >= config.max_retries
                    {
                        return Err(grpc_error_to_snip_error("Sync request", &e));
                    }
                    let is_rate_limited = e.code() == Code::ResourceExhausted;
                    let actual_delay = (delay_ms as f64 * retry_jitter_multiplier()) as u64;
                    tracing::warn!(
                        "Sync request failed (attempt {}/{}): {}. Retrying in {}ms...",
                        attempt + 1,
                        config.max_retries + 1,
                        e,
                        actual_delay
                    );
                    let delay = Duration::from_millis(actual_delay);
                    if self.limits.is_some_and(|limits| {
                        limits
                            .remaining()
                            .is_none_or(|remaining| remaining <= delay)
                    }) {
                        return Err(SnipError::sync_failure(
                            crate::error::SyncFailureKind::Timeout,
                            Some("automatic sync deadline expired during retry backoff"),
                        ));
                    }
                    tokio::time::sleep(delay).await;
                    let backoff_multiplier = if is_rate_limited { 4.0 } else { 2.0 };
                    let max_delay = if is_rate_limited {
                        120_000u64
                    } else {
                        config.max_delay_ms
                    };
                    delay_ms = ((delay_ms as f64 * backoff_multiplier) as u64).min(max_delay);
                    attempt += 1;
                }
            }
        }
    }

    /// Checks server health and returns `true` if the server is reachable.
    pub async fn health_check(&mut self) -> SnipResult<bool> {
        self.ensure_budget()?;
        match retry_grpc_limited!(
            self,
            self.client.health(tonic::Request::new(HealthRequest {})),
            "Health check"
        ) {
            Ok(response) => Ok(response.into_inner().healthy),
            Err(e) => {
                tracing::debug!(error = %e, "Health check failed");
                if matches!(
                    e,
                    SnipError::SyncFailure {
                        kind: crate::error::SyncFailureKind::Timeout,
                        ..
                    }
                ) {
                    Err(e)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// Registers a new device with the server and returns the API key and device ID.
    pub async fn register(server_url: String) -> SnipResult<(String, String)> {
        let channel = create_tls_channel(&server_url, None).await.map_err(|e| {
            SnipError::sync_failure(
                crate::error::SyncFailureKind::ConnectFailed,
                Some(&e.to_string()),
            )
        })?;

        let mut client = SnippetSyncClient::new(channel);

        let response = retry_grpc!(
            client.register(tonic::Request::new(RegisterRequest {
                device_id: String::new(),
            })),
            "Register request"
        )?;

        let response = response.into_inner();
        if response.success {
            Ok((response.api_key, response.device_id))
        } else {
            Err(SnipError::sync_failure(
                crate::error::SyncFailureKind::RegistrationFailed,
                Some(&response.message),
            ))
        }
    }

    /// Lists all libraries on the sync server.
    pub async fn list_libraries(&mut self) -> SnipResult<Vec<Library>> {
        self.ensure_budget()?;
        let api_key = Zeroizing::new(self.settings.api_key.clone());
        let mut all_libraries = Vec::new();
        let mut offset = 0i32;
        const PAGE_LIMIT: i32 = 50;
        loop {
            let response = retry_grpc_limited!(
                self,
                async {
                    let mut req = tonic::Request::new(ListLibrariesRequest {
                        api_key: String::new(),
                        limit: PAGE_LIMIT,
                        offset,
                    });
                    add_api_key_metadata(&mut req, &api_key);
                    self.client.list_libraries(req).await
                },
                "List libraries"
            )?;
            let inner = response.into_inner();
            let count = i32::try_from(inner.libraries.len()).unwrap_or(i32::MAX);
            all_libraries.extend(inner.libraries);
            // Server signals end-of-stream with `!has_more` (preferred).
            // `count < PAGE_LIMIT` is a fallback when the server returns
            // fewer than the limit (which is the last page by construction).
            // The `count == 0 && has_more` guard is paranoia against a
            // buggy server that returns empty pages without setting
            // `has_more = false` — without it, we'd loop forever.
            if !inner.has_more || count < PAGE_LIMIT || count == 0 {
                break;
            }
            offset = offset.saturating_add(count);
        }
        Ok(all_libraries)
    }

    /// Creates a new library on the sync server.
    pub async fn create_library(&mut self, name: &str) -> SnipResult<Library> {
        self.ensure_budget()?;
        let api_key = Zeroizing::new(self.settings.api_key.clone());
        let name_str = name.to_string();
        let response = retry_grpc_limited!(
            self,
            async {
                let mut req = tonic::Request::new(CreateLibraryRequest {
                    api_key: String::new(),
                    name: name_str.clone(),
                });
                add_api_key_metadata(&mut req, &api_key);
                self.client.create_library(req).await
            },
            "Create library"
        )?;

        let response = response.into_inner();
        if response.success {
            Ok(Library {
                id: response.library_id,
                name: name_str,
                created_at: chrono::Utc::now().timestamp(),
                snippet_count: 0,
            })
        } else {
            Err(SnipError::sync_failure(
                crate::error::SyncFailureKind::CreateLibraryFailed,
                Some(&response.message),
            ))
        }
    }

    /// Lists all premade libraries available on the server.
    pub async fn list_premade_libraries(&mut self) -> SnipResult<Vec<PremadeLibrary>> {
        let api_key = Zeroizing::new(self.settings.api_key.clone());
        let response = retry_grpc!(
            async {
                let mut req = tonic::Request::new(ListPremadeLibrariesRequest {
                    api_key: String::new(),
                });
                add_api_key_metadata(&mut req, &api_key);
                self.client.list_premade_libraries(req).await
            },
            "List premade libraries"
        )?;
        Ok(response.into_inner().libraries)
    }

    /// Downloads a premade library's content from the server.
    pub async fn get_premade_library(&mut self, filename: &str) -> SnipResult<String> {
        let api_key = Zeroizing::new(self.settings.api_key.clone());
        let filename_str = filename.to_string();
        let response = retry_grpc!(
            async {
                let mut req = tonic::Request::new(GetPremadeLibraryRequest {
                    api_key: String::new(),
                    filename: filename_str.clone(),
                });
                add_api_key_metadata(&mut req, &api_key);
                self.client.get_premade_library(req).await
            },
            "Get premade library"
        )?;

        let response = response.into_inner();
        if response.success {
            Ok(response.content)
        } else {
            Err(SnipError::sync_failure(
                crate::error::SyncFailureKind::GetPremadeLibraryFailed,
                Some(&response.message),
            ))
        }
    }

    /// Searches premade libraries on the server by query string.
    pub async fn search_premade_libraries(
        &mut self,
        query: &str,
    ) -> SnipResult<Vec<PremadeLibrary>> {
        let api_key = Zeroizing::new(self.settings.api_key.clone());
        let query_str = query.to_string();
        let response = retry_grpc!(
            async {
                let mut req = tonic::Request::new(SearchPremadeLibrariesRequest {
                    api_key: String::new(),
                    query: query_str.clone(),
                });
                add_api_key_metadata(&mut req, &api_key);
                self.client.search_premade_libraries(req).await
            },
            "Search premade libraries"
        )?;
        Ok(response.into_inner().libraries)
    }
}

/// Returns `true` if plaintext HTTP should be allowed for the given URI.
///
/// Allows `http://` connections only when:
/// - the host is a loopback address (`localhost`, `127.0.0.1`, `[::1]`,
///   or any `127.x.x.x`), **or**
/// - the `SNIP_SYNC_ALLOW_HTTP` env var is set to a truthy value
///   (`true`/`1`/`yes`/`on`, case-insensitive).
fn allow_plaintext_http(uri: &Uri) -> bool {
    let scheme = uri.scheme_str().unwrap_or("https");
    if scheme != "http" {
        return true; // HTTPS or other schemes are fine.
    }
    let Some(host) = uri.host() else {
        return false;
    };
    if is_loopback(host) {
        return true;
    }
    // SNIP_SYNC_ALLOW_HTTP overrides the loopback check (for testing).
    matches!(
        std::env::var("SNIP_SYNC_ALLOW_HTTP").as_deref(),
        Ok(v) if matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on")
    )
}

/// Returns `true` if `host` resolves to a loopback address.
fn is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let stripped = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if stripped.eq_ignore_ascii_case("::ffff:127.0.0.1")
        || stripped.eq_ignore_ascii_case("::ffff:7f00:1")
        || stripped.eq_ignore_ascii_case("::ffff:7f00:0001")
    {
        return true;
    }
    stripped
        .parse::<std::net::IpAddr>()
        .is_ok_and(|addr| addr.is_loopback())
}

async fn create_tls_channel(
    server_url: &str,
    request_timeout: Option<Duration>,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    let uri: Uri = server_url.parse()?;
    let scheme = uri.scheme_str().unwrap_or("https").to_ascii_lowercase();
    let uri_host = uri.host().ok_or("No host in URI")?;

    if scheme == "http" && !allow_plaintext_http(&uri) {
        return Err(
            "Refusing plaintext gRPC to non-loopback host. Use https:// or set \
             SNIP_SYNC_ALLOW_HTTP=true for local development."
                .into(),
        );
    }

    let host = if scheme == "https" {
        Some(uri_host.to_string())
    } else {
        None
    };

    let connect_timeout_secs = std::env::var("SNP_SYNC_CONNECT_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CONNECT_TIMEOUT_SECS);
    let configured_request_timeout = std::env::var("SNP_SYNC_REQUEST_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS);
    let request_timeout = request_timeout
        .unwrap_or_else(|| Duration::from_secs(configured_request_timeout))
        .min(Duration::from_secs(configured_request_timeout));

    let endpoint = Endpoint::new(uri)?
        .connect_timeout(Duration::from_secs(connect_timeout_secs))
        .timeout(request_timeout);

    let endpoint = if let Some(host) = host {
        let tls_config = ClientTlsConfig::new()
            .with_enabled_roots()
            .domain_name(host)
            // Skip ALPN h2 negotiation: assume the endpoint speaks HTTP/2
            // (gRPC). Servers without proper ALPN support still work; a
            // non-H2 endpoint surfaces as a confusing framing error.
            .assume_http2(true);
        endpoint.tls_config(tls_config)?
    } else {
        endpoint
    };

    let channel = endpoint.connect().await?;
    Ok(channel)
}

/// Encrypts a snippet's sensitive fields (description, command, tags) for sync.
pub fn encrypt_snippet(
    api_key: &str,
    snippet: &crate::proto::Snippet,
) -> SnipResult<crate::proto::Snippet> {
    let data = EncryptedSnippetData {
        description: snippet.description.clone(),
        command: snippet.command.clone(),
        tags: snippet.tags.clone(),
    };

    let json = serde_json::to_string(&data).map_err(|e| {
        SnipError::sync_failure(
            crate::error::SyncFailureKind::EncryptionFailed,
            Some(&e.to_string()),
        )
    })?;

    let encrypted = encryption::encrypt(api_key, &json)?;

    Ok(crate::proto::Snippet {
        id: snippet.id.clone(),
        description: String::new(),
        command: encrypted,
        tags: vec![],
        created_at: snippet.created_at,
        updated_at: snippet.updated_at,
        device_id: snippet.device_id.clone(),
        deleted: snippet.deleted,
        encrypted: true,
    })
}

/// Encrypt a batch of snippets, returning the successfully encrypted
/// snippets and the IDs that failed encryption.
fn encrypt_snippets(
    api_key: &str,
    snippets: &[crate::proto::Snippet],
) -> (Vec<crate::proto::Snippet>, Vec<String>) {
    encrypt_snippets_with(snippets, |s| encrypt_snippet(api_key, s))
}

/// Encrypt a batch of snippets using the provided encrypt function,
/// returning the successfully encrypted snippets and the IDs that
/// failed. Used by tests to inject encryption failures.
fn encrypt_snippets_with(
    snippets: &[crate::proto::Snippet],
    encrypt_fn: impl Fn(&crate::proto::Snippet) -> SnipResult<crate::proto::Snippet>,
) -> (Vec<crate::proto::Snippet>, Vec<String>) {
    let mut encrypted = Vec::new();
    let mut failed_ids = Vec::new();
    for s in snippets {
        match encrypt_fn(s) {
            Ok(es) => encrypted.push(es),
            Err(e) => {
                failed_ids.push(s.id.clone());
                tracing::warn!("Failed to encrypt snippet {}: {}", s.id, e);
            }
        }
    }
    (encrypted, failed_ids)
}

/// Decrypts a snippet's encrypted fields received from the sync server.
pub fn decrypt_snippet(
    api_key: &str,
    snippet: &crate::proto::Snippet,
) -> SnipResult<crate::proto::Snippet> {
    if !snippet.encrypted {
        return Ok(snippet.clone());
    }

    let decrypted = encryption::decrypt(api_key, &snippet.command)?;

    let data: EncryptedSnippetData = serde_json::from_str(&decrypted).map_err(|e| {
        SnipError::sync_failure(
            crate::error::SyncFailureKind::DecryptionFailed,
            Some(&e.to_string()),
        )
    })?;

    Ok(crate::proto::Snippet {
        id: snippet.id.clone(),
        description: data.description,
        command: data.command,
        tags: data.tags,
        created_at: snippet.created_at,
        updated_at: snippet.updated_at,
        device_id: snippet.device_id.clone(),
        deleted: snippet.deleted,
        encrypted: false,
    })
}

/// Detects if any server snippets have a device_id that doesn't match the
/// expected local device_id, indicating a potential conflict from another device.
pub fn detect_device_conflict(
    server_snippets: &[crate::proto::Snippet],
    expected_device_id: &str,
) -> Vec<String> {
    if expected_device_id.is_empty() {
        tracing::debug!("device_id is empty; skipping cross-device conflict check");
        return Vec::new();
    }
    let mut conflicting_ids = Vec::new();
    for s in server_snippets {
        if !s.device_id.is_empty() && s.device_id != expected_device_id {
            tracing::warn!(
                "Device conflict detected: snippet {} has device_id '{}', expected '{}'",
                s.id,
                s.device_id,
                expected_device_id
            );
            conflicting_ids.push(s.id.clone());
        }
    }
    conflicting_ids
}

/// Measure the encoded size of a `SyncRequest` carrying the given batch.
/// `SyncRequest` is the larger of the two request envelopes used during sync;
/// batches that fit `SyncRequest` also fit `PushSnippetsRequest`.
fn sync_request_encoded_len(
    batch: &[crate::proto::Snippet],
    library_id: &str,
    last_sync: i64,
    sync_limit: i32,
) -> usize {
    let request = SyncRequest {
        api_key: String::new(),
        local_snippets: batch.to_vec(),
        last_sync_timestamp: last_sync,
        library_id: library_id.to_string(),
        limit: sync_limit,
        offset: 0,
    };
    request.encoded_len()
}

fn varint_encoded_len(mut value: usize) -> usize {
    let mut length = 1;
    while value >= 128 {
        value >>= 7;
        length += 1;
    }
    length
}

fn sync_request_base_encoded_len(library_id: &str, last_sync: i64, sync_limit: i32) -> usize {
    sync_request_encoded_len(&[], library_id, last_sync, sync_limit)
}

fn snippet_field_encoded_len(snippet: &crate::proto::Snippet) -> usize {
    let encoded_len = snippet.encoded_len();
    // `local_snippets` is a repeated message field: one-byte field tag,
    // length varint, then the encoded message.
    1 + varint_encoded_len(encoded_len) + encoded_len
}

/// Build byte-bounded upload batches from encrypted snippets.
///
/// Uses Prost encoded length to measure the actual request size. Each batch is
/// measured against `SyncRequest` (the larger envelope used for the first
/// batch); batches that fit `SyncRequest` also fit `PushSnippetsRequest`.
///
/// Returns `RequestTooLarge` if any individual snippet exceeds the ceiling
/// before any batch is sent. Batches are built incrementally: tentatively
/// append one snippet, measure, keep or finalize. After an overflow split the
/// new singleton is immediately re-validated so an oversized item following a
/// small item is caught before any remote mutation.
pub(crate) fn build_upload_batches(
    mut encrypted_snippets: Vec<crate::proto::Snippet>,
    library_id: &str,
    last_sync: i64,
    sync_limit: i32,
    byte_ceiling: usize,
) -> SnipResult<Vec<Vec<crate::proto::Snippet>>> {
    // Sort by snippet ID for deterministic ordering across retries.
    encrypted_snippets.sort_by(|a, b| a.id.cmp(&b.id));

    let mut batches: Vec<Vec<crate::proto::Snippet>> = Vec::new();
    let mut current_batch: Vec<crate::proto::Snippet> = Vec::new();
    let base_encoded_size = sync_request_base_encoded_len(library_id, last_sync, sync_limit);
    let mut current_encoded_size = base_encoded_size;

    for snippet in encrypted_snippets {
        let snippet_encoded_size = snippet_field_encoded_len(&snippet);
        current_batch.push(snippet);
        current_encoded_size += snippet_encoded_size;

        if current_encoded_size > byte_ceiling {
            if current_batch.len() == 1 {
                // Single item exceeds ceiling — fail before any send.
                let oversized = &current_batch[0];
                return Err(SnipError::sync_failure(
                    SyncFailureKind::RequestTooLarge,
                    Some(&format!(
                        "snippet '{}' encoded size {} bytes exceeds request ceiling {} bytes; \
                         the local snippet is unchanged — raise both server and client message \
                         limits or reduce/split the snippet",
                        oversized.id, current_encoded_size, byte_ceiling,
                    )),
                ));
            }
            // Remove the overflow snippet and finalize the prior batch.
            let overflow = current_batch.pop().expect("current batch is non-empty");
            batches.push(current_batch);
            current_batch = vec![overflow];
            current_encoded_size = base_encoded_size + snippet_encoded_size;

            // Re-validate the new singleton immediately so an oversized
            // item following a small item is caught before any remote
            // mutation.
            let singleton_size = current_encoded_size;
            if singleton_size > byte_ceiling {
                let oversized = &current_batch[0];
                return Err(SnipError::sync_failure(
                    SyncFailureKind::RequestTooLarge,
                    Some(&format!(
                        "snippet '{}' encoded size {} bytes exceeds request ceiling {} bytes; \
                         the local snippet is unchanged — raise both server and client message \
                         limits or reduce/split the snippet",
                        oversized.id, singleton_size, byte_ceiling,
                    )),
                ));
            }
        }
    }

    if !current_batch.is_empty() {
        batches.push(current_batch);
    }

    Ok(batches)
}

/// Add batch context to a sync error while preserving its classification.
///
/// `SyncFailure` kinds (e.g. `ClockSkew`, `Timeout`) keep their kind; other
/// variants must NOT be re-wrapped as `SyncRequestFailed` — that would
/// classify persistent server-side faults (e.g. gRPC `internal`, which maps
/// to `Runtime` → `FailureClass::Internal`) as `Transient` and retry them
/// forever instead of escalating to attention-required.
fn add_batch_context(error: SnipError, batch: usize, total: usize) -> SnipError {
    let ctx = format!("batch {batch}/{total}");
    match error {
        SnipError::SyncFailure { kind, detail } => {
            let new_detail = match detail {
                Some(d) => format!("{ctx}: {d}"),
                None => ctx,
            };
            SnipError::SyncFailure {
                kind,
                detail: Some(new_detail),
            }
        }
        SnipError::Runtime { message, detail } => SnipError::Runtime {
            message: format!("{ctx}: {message}"),
            detail,
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl SyncClient {
        /// Test-only: drive the prepared transport with an injected
        /// encrypt function. Used by the all-encryption-failed regression
        /// to exercise the zero-batch pull path when every local snippet
        /// fails encryption. Compiled only for unit tests.
        async fn sync_encrypted_with_test_encrypt<F>(
            &mut self,
            local_snippets: Vec<crate::proto::Snippet>,
            last_sync: i64,
            library_id: &str,
            byte_ceiling: usize,
            encrypt_fn: F,
        ) -> SnipResult<crate::proto::SyncResponse>
        where
            F: Fn(&crate::proto::Snippet) -> SnipResult<crate::proto::Snippet>,
        {
            self.ensure_budget()?;
            let (encrypted_snippets, encrypt_failed_ids) =
                encrypt_snippets_with(&local_snippets, encrypt_fn);
            self.sync_prepared_encrypted_inner(
                encrypted_snippets,
                encrypt_failed_ids,
                last_sync,
                library_id,
                byte_ceiling,
            )
            .await
        }
    }

    #[test]
    fn test_default_retry_config() {
        let config = SyncRetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_delay_ms, 100);
        assert_eq!(config.max_delay_ms, 5000);
    }

    #[test]
    fn test_non_retryable_errors() {
        let non_retryable = [
            tonic::Status::invalid_argument("test"),
            tonic::Status::not_found("test"),
            tonic::Status::already_exists("test"),
            tonic::Status::permission_denied("test"),
            tonic::Status::unauthenticated("test"),
        ];
        for status in &non_retryable {
            assert!(
                !SyncRetryConfig::is_retryable_grpc_error(status),
                "Expected {:?} to be non-retryable",
                status.code()
            );
        }

        let retryable = [
            tonic::Status::internal("test"),
            tonic::Status::unavailable("test"),
            tonic::Status::deadline_exceeded("test"),
            tonic::Status::resource_exhausted("rate limited"), // 429 - should be retryable
        ];
        for status in &retryable {
            assert!(
                SyncRetryConfig::is_retryable_grpc_error(status),
                "Expected {:?} to be retryable",
                status.code()
            );
        }
    }

    #[test]
    fn deadline_exceeded_is_a_typed_timeout() {
        let error = grpc_error_to_snip_error(
            "sync",
            &tonic::Status::deadline_exceeded("budget exhausted"),
        );
        assert!(matches!(
            error,
            SnipError::SyncFailure {
                kind: crate::error::SyncFailureKind::Timeout,
                ..
            }
        ));
        assert_eq!(
            crate::sync_failure::FailureClass::from_error(&error),
            crate::sync_failure::FailureClass::Transient
        );
    }

    #[test]
    fn test_detect_device_conflict_empty_device_id() {
        let snippets = vec![crate::proto::Snippet {
            id: "1".to_string(),
            description: String::new(),
            command: String::new(),
            tags: vec![],
            created_at: 0,
            updated_at: 0,
            device_id: "other-device".to_string(),
            deleted: false,
            encrypted: false,
        }];
        assert!(detect_device_conflict(&snippets, "").is_empty());
    }

    #[test]
    fn test_detect_device_conflict_no_conflict() {
        let snippets = vec![crate::proto::Snippet {
            id: "1".to_string(),
            description: String::new(),
            command: String::new(),
            tags: vec![],
            created_at: 0,
            updated_at: 0,
            device_id: "device-a".to_string(),
            deleted: false,
            encrypted: false,
        }];
        assert!(detect_device_conflict(&snippets, "device-a").is_empty());
    }

    #[test]
    fn test_detect_device_conflict_with_mismatch() {
        let snippets = vec![crate::proto::Snippet {
            id: "1".to_string(),
            description: String::new(),
            command: String::new(),
            tags: vec![],
            created_at: 0,
            updated_at: 0,
            device_id: "device-b".to_string(),
            deleted: false,
            encrypted: false,
        }];
        let conflicts = detect_device_conflict(&snippets, "device-a");
        assert_eq!(conflicts, vec!["1".to_string()]);
    }

    #[test]
    fn test_encrypt_decrypt_snippet_roundtrip() {
        let api_key = "test-api-key-for-encryption";
        let snippet = crate::proto::Snippet {
            id: "test-id".to_string(),
            description: "Test Description".to_string(),
            command: "echo hello world".to_string(),
            tags: vec!["bash".to_string(), "test".to_string()],
            created_at: 1000,
            updated_at: 2000,
            device_id: "device-1".to_string(),
            deleted: false,
            encrypted: false,
        };

        let encrypted = encrypt_snippet(api_key, &snippet).unwrap();
        assert!(encrypted.encrypted);
        assert_eq!(encrypted.id, "test-id");
        assert_eq!(encrypted.description, "");
        assert!(encrypted.tags.is_empty());
        assert!(!encrypted.command.is_empty());
        assert_ne!(encrypted.command, "echo hello world");

        let decrypted = decrypt_snippet(api_key, &encrypted).unwrap();
        assert!(!decrypted.encrypted);
        assert_eq!(decrypted.description, "Test Description");
        assert_eq!(decrypted.command, "echo hello world");
        assert_eq!(decrypted.tags, vec!["bash", "test"]);
        assert_eq!(decrypted.created_at, 1000);
        assert_eq!(decrypted.updated_at, 2000);
        assert_eq!(decrypted.device_id, "device-1");
    }

    #[test]
    fn test_decrypt_non_encrypted_passthrough() {
        let api_key = "test-api-key";
        let snippet = crate::proto::Snippet {
            id: "test-id".to_string(),
            description: "desc".to_string(),
            command: "cmd".to_string(),
            tags: vec![],
            created_at: 0,
            updated_at: 0,
            device_id: "device".to_string(),
            deleted: false,
            encrypted: false,
        };

        let result = decrypt_snippet(api_key, &snippet).unwrap();
        assert_eq!(result.description, "desc");
        assert_eq!(result.command, "cmd");
    }

    #[test]
    fn test_decrypt_wrong_key_fails() {
        let snippet = crate::proto::Snippet {
            id: "test-id".to_string(),
            description: "desc".to_string(),
            command: "cmd".to_string(),
            tags: vec![],
            created_at: 0,
            updated_at: 0,
            device_id: "device".to_string(),
            deleted: false,
            encrypted: false,
        };

        let encrypted = encrypt_snippet("correct-key", &snippet).unwrap();
        let result = decrypt_snippet("wrong-key", &encrypted);
        assert!(result.is_err());
    }

    #[test]
    fn test_encrypt_decrypt_with_special_characters() {
        let api_key = "test-key-special-chars";
        let snippet = crate::proto::Snippet {
            id: "id".to_string(),
            description: "Unicode: 你好世界 🌍".to_string(),
            command: "echo 'hello \"world\"' && echo $HOME".to_string(),
            tags: vec!["tag with spaces".to_string()],
            created_at: 0,
            updated_at: 0,
            device_id: "device".to_string(),
            deleted: false,
            encrypted: false,
        };

        let encrypted = encrypt_snippet(api_key, &snippet).unwrap();
        let decrypted = decrypt_snippet(api_key, &encrypted).unwrap();
        assert_eq!(decrypted.description, "Unicode: 你好世界 🌍");
        assert_eq!(decrypted.command, "echo 'hello \"world\"' && echo $HOME");
        assert_eq!(decrypted.tags, vec!["tag with spaces"]);
    }

    // ── Batching tests ─────────────────────────────────────────────

    fn make_encrypted_snippet(id: &str, command_size: usize) -> crate::proto::Snippet {
        crate::proto::Snippet {
            id: id.to_string(),
            description: String::new(),
            command: "x".repeat(command_size),
            tags: vec![],
            created_at: 1000,
            updated_at: 2000,
            device_id: "device-1".to_string(),
            deleted: false,
            encrypted: true,
        }
    }

    #[test]
    fn test_build_upload_batches_empty_list() {
        let batches =
            build_upload_batches(Vec::new(), "lib", 0, 1000, DEFAULT_CLIENT_REQUEST_CEILING)
                .unwrap();
        assert!(batches.is_empty());
    }

    #[test]
    fn test_build_upload_batches_single_small_item() {
        let snippets = vec![make_encrypted_snippet("a", 100)];
        let batches =
            build_upload_batches(snippets, "lib", 0, 1000, DEFAULT_CLIENT_REQUEST_CEILING).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 1);
        assert_eq!(batches[0][0].id, "a");
    }

    #[test]
    fn test_build_upload_batches_fits_one_request() {
        let snippets: Vec<_> = (0..5)
            .map(|i| make_encrypted_snippet(&format!("s{i}"), 50))
            .collect();
        let batches =
            build_upload_batches(snippets, "lib", 0, 1000, DEFAULT_CLIENT_REQUEST_CEILING).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 5);
    }

    #[test]
    fn test_build_upload_batches_exact_boundary_fit() {
        // Use a very small ceiling to force multiple batches.
        let snippets: Vec<_> = (0..3)
            .map(|i| make_encrypted_snippet(&format!("s{i}"), 200))
            .collect();
        // Measure the size of one snippet in a SyncRequest.
        let one_request = SyncRequest {
            api_key: String::new(),
            local_snippets: vec![snippets[0].clone()],
            last_sync_timestamp: 0,
            library_id: "lib".to_string(),
            limit: 1000,
            offset: 0,
        };
        let single_size = one_request.encoded_len();
        // Set ceiling to exactly fit one snippet.
        let batches = build_upload_batches(snippets, "lib", 0, 1000, single_size).unwrap();
        assert_eq!(batches.len(), 3);
        for batch in &batches {
            assert_eq!(batch.len(), 1);
        }
    }

    #[test]
    fn test_build_upload_batches_one_byte_over_starts_new_batch() {
        let snippets: Vec<_> = (0..3)
            .map(|i| make_encrypted_snippet(&format!("s{i}"), 200))
            .collect();
        let one_request = SyncRequest {
            api_key: String::new(),
            local_snippets: vec![snippets[0].clone()],
            last_sync_timestamp: 0,
            library_id: "lib".to_string(),
            limit: 1000,
            offset: 0,
        };
        let single_size = one_request.encoded_len();
        // Ceiling allows one but not two.
        let batches = build_upload_batches(snippets, "lib", 0, 1000, single_size + 1).unwrap();
        // Two snippets fit in one batch (their combined size <= single_size + 1),
        // third goes to a new batch.
        assert!(batches.len() >= 2);
    }

    #[test]
    fn test_build_upload_batches_oversized_single_item() {
        // Use a ceiling smaller than a typical snippet request.
        let snippets = vec![make_encrypted_snippet("huge", 100_000)];
        let one_request = SyncRequest {
            api_key: String::new(),
            local_snippets: vec![snippets[0].clone()],
            last_sync_timestamp: 0,
            library_id: "lib".to_string(),
            limit: 1000,
            offset: 0,
        };
        let single_size = one_request.encoded_len();
        // Set ceiling to half the single item size so it's definitely too large.
        let ceiling = single_size / 2;
        let result = build_upload_batches(snippets, "lib", 0, 1000, ceiling);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            SnipError::SyncFailure {
                kind: SyncFailureKind::RequestTooLarge,
                ..
            }
        ));
        let msg = err.to_string();
        assert!(msg.contains("huge"));
        assert!(msg.contains("exceeds request ceiling"));
    }

    #[test]
    fn test_build_upload_batches_oversized_after_small_fails() {
        // A small item followed by an oversized item must fail before any send.
        let snippets = vec![
            make_encrypted_snippet("small", 100),
            make_encrypted_snippet("huge", 100_000),
        ];
        let one_request = SyncRequest {
            api_key: String::new(),
            local_snippets: vec![snippets[0].clone()],
            last_sync_timestamp: 0,
            library_id: "lib".to_string(),
            limit: 1000,
            offset: 0,
        };
        let small_size = one_request.encoded_len();
        // Ceiling fits the small item but not the oversized one.
        let ceiling = small_size + 10;
        let result = build_upload_batches(snippets, "lib", 0, 1000, ceiling);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            SnipError::SyncFailure {
                kind: SyncFailureKind::RequestTooLarge,
                ..
            }
        ));
        let msg = err.to_string();
        assert!(
            msg.contains("huge"),
            "error should name the oversized snippet"
        );
    }

    #[test]
    fn test_build_upload_batches_oversized_between_small_items_fails() {
        // An oversized item sandwiched between two small items must fail.
        let small = make_encrypted_snippet("small", 100);
        let huge = make_encrypted_snippet("huge", 100_000);
        let small2 = make_encrypted_snippet("small2", 100);
        let one_request = SyncRequest {
            api_key: String::new(),
            local_snippets: vec![small.clone()],
            last_sync_timestamp: 0,
            library_id: "lib".to_string(),
            limit: 1000,
            offset: 0,
        };
        let small_size = one_request.encoded_len();
        let ceiling = small_size + 10;
        // Sort order: small, small2, huge — huge comes last so it's the
        // overflow singleton that gets re-validated.
        let mut snippets = vec![small, huge, small2];
        snippets.sort_by(|a, b| a.id.cmp(&b.id));
        let result = build_upload_batches(snippets, "lib", 0, 1000, ceiling);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            SnipError::SyncFailure {
                kind: SyncFailureKind::RequestTooLarge,
                ..
            }
        ));
    }

    #[test]
    fn test_build_upload_batches_stable_id_ordering() {
        let snippets: Vec<_> = (0..10)
            .map(|i| make_encrypted_snippet(&format!("z{i}"), 100))
            .collect();
        let batches1 = build_upload_batches(
            snippets.clone(),
            "lib",
            0,
            1000,
            DEFAULT_CLIENT_REQUEST_CEILING,
        )
        .unwrap();
        let batches2 =
            build_upload_batches(snippets, "lib", 0, 1000, DEFAULT_CLIENT_REQUEST_CEILING).unwrap();
        let ids1: Vec<_> = batches1.iter().flatten().map(|s| &s.id).collect();
        let ids2: Vec<_> = batches2.iter().flatten().map(|s| &s.id).collect();
        assert_eq!(ids1, ids2);
    }

    #[test]
    fn test_build_upload_batches_metadata_overhead_included() {
        // Two snippets that individually fit but together with metadata exceed ceiling.
        let snippets: Vec<_> = (0..2)
            .map(|i| make_encrypted_snippet(&format!("s{i}"), 500))
            .collect();
        let one_request = SyncRequest {
            api_key: String::new(),
            local_snippets: vec![snippets[0].clone()],
            last_sync_timestamp: 0,
            library_id: "lib".to_string(),
            limit: 1000,
            offset: 0,
        };
        let single_size = one_request.encoded_len();
        // Ceiling fits one but not two.
        let batches = build_upload_batches(snippets, "lib", 0, 1000, single_size + 1).unwrap();
        // First batch has at most one snippet, second has the rest.
        assert!(batches.len() >= 2);
    }

    #[test]
    fn test_incremental_batch_size_matches_prost_encoded_length() {
        let snippets = vec![
            make_encrypted_snippet("a", 200),
            make_encrypted_snippet("b", 200),
            make_encrypted_snippet("c", 200),
        ];
        let estimated = sync_request_base_encoded_len("lib", 0, 1000)
            + snippets
                .iter()
                .map(snippet_field_encoded_len)
                .sum::<usize>();
        let request = SyncRequest {
            api_key: String::new(),
            local_snippets: snippets,
            last_sync_timestamp: 0,
            library_id: "lib".to_string(),
            limit: 1000,
            offset: 0,
        };
        assert_eq!(estimated, request.encoded_len());
    }

    #[test]
    fn test_build_upload_batches_no_batch_exceeds_ceiling() {
        let snippets: Vec<_> = (0..20)
            .map(|i| make_encrypted_snippet(&format!("s{i}"), 300))
            .collect();
        let ceiling = DEFAULT_CLIENT_REQUEST_CEILING.min(2000);
        let batches = build_upload_batches(snippets, "lib", 0, 1000, ceiling).unwrap();
        for (i, batch) in batches.iter().enumerate() {
            let request = SyncRequest {
                api_key: String::new(),
                local_snippets: batch.clone(),
                last_sync_timestamp: 0,
                library_id: "lib".to_string(),
                limit: 1000,
                offset: 0,
            };
            assert!(
                request.encoded_len() <= ceiling,
                "batch {} encoded size {} exceeds ceiling {}",
                i,
                request.encoded_len(),
                ceiling
            );
        }
    }

    #[test]
    fn test_build_upload_batches_every_batch_fits_both_envelopes() {
        // Every batch must fit SyncRequest (by construction) and
        // therefore also fits PushSnippetsRequest (which is smaller).
        let snippets: Vec<_> = (0..10)
            .map(|i| make_encrypted_snippet(&format!("s{i}"), 200))
            .collect();
        let ceiling = DEFAULT_CLIENT_REQUEST_CEILING.min(2000);
        let batches = build_upload_batches(snippets, "lib", 0, 1000, ceiling).unwrap();
        assert!(batches.len() > 1, "need multiple batches for this test");
        for (i, batch) in batches.iter().enumerate() {
            let sync_req = SyncRequest {
                api_key: String::new(),
                local_snippets: batch.clone(),
                last_sync_timestamp: 0,
                library_id: "lib".to_string(),
                limit: 1000,
                offset: 0,
            };
            assert!(
                sync_req.encoded_len() <= ceiling,
                "batch {} SyncRequest size {} exceeds ceiling {}",
                i,
                sync_req.encoded_len(),
                ceiling
            );
            // PushSnippetsRequest is smaller; if it fits SyncRequest it
            // also fits PushSnippetsRequest.
            let push_req = PushSnippetsRequest {
                api_key: String::new(),
                library_id: "lib".to_string(),
                snippets: batch.clone(),
            };
            assert!(
                push_req.encoded_len() <= DEFAULT_CLIENT_REQUEST_CEILING,
                "batch {} PushSnippetsRequest size {} exceeds default ceiling",
                i,
                push_req.encoded_len(),
            );
        }
    }

    #[test]
    fn test_build_upload_batches_splits_many_large_snippets() {
        // Verify that many large snippets are split into multiple batches
        // under the default ceiling.
        let snippets: Vec<_> = (0..400)
            .map(|i| make_encrypted_snippet(&format!("mb-{i:04}", i = i), 10_000))
            .collect();
        let one_size = {
            let req = SyncRequest {
                api_key: String::new(),
                local_snippets: vec![snippets[0].clone()],
                last_sync_timestamp: 0,
                library_id: "lib".to_string(),
                limit: 1000,
                offset: 0,
            };
            req.encoded_len()
        };
        let batches =
            build_upload_batches(snippets, "lib", 0, 1000, DEFAULT_CLIENT_REQUEST_CEILING).unwrap();
        assert!(
            batches.len() > 1,
            "400 large snippets should produce multiple batches, got {} (one snippet = {} bytes, ceiling = {} bytes)",
            batches.len(),
            one_size,
            DEFAULT_CLIENT_REQUEST_CEILING,
        );
        // Every batch must fit the SyncRequest ceiling.
        for (i, batch) in batches.iter().enumerate() {
            let request = SyncRequest {
                api_key: String::new(),
                local_snippets: batch.clone(),
                last_sync_timestamp: 0,
                library_id: "lib".to_string(),
                limit: 1000,
                offset: 0,
            };
            assert!(
                request.encoded_len() <= DEFAULT_CLIENT_REQUEST_CEILING,
                "batch {} encoded size {} exceeds ceiling {}",
                i,
                request.encoded_len(),
                DEFAULT_CLIENT_REQUEST_CEILING
            );
        }
    }

    // ── Clock skew error mapping tests ─────────────────────────────

    #[test]
    fn test_clock_skew_invalid_argument_is_typed() {
        let error = grpc_error_to_snip_error(
            "sync",
            &tonic::Status::invalid_argument(
                "CLOCK_SKEW: updated_at is 742 seconds ahead of server time; synchronize the client clock and retry",
            ),
        );
        assert!(matches!(
            error,
            SnipError::SyncFailure {
                kind: SyncFailureKind::ClockSkew,
                ..
            }
        ));
        assert_eq!(
            crate::sync_failure::FailureClass::from_error(&error),
            crate::sync_failure::FailureClass::Configuration
        );
    }

    #[test]
    fn test_non_clock_skew_invalid_argument_is_generic() {
        let error =
            grpc_error_to_snip_error("sync", &tonic::Status::invalid_argument("bad snippet id"));
        assert!(matches!(error, SnipError::Runtime { .. }));
    }

    #[test]
    fn test_timestamp_text_without_clock_skew_marker_is_generic() {
        let error = grpc_error_to_snip_error(
            "sync",
            &tonic::Status::invalid_argument("updated_at field is malformed"),
        );
        assert!(matches!(error, SnipError::Runtime { .. }));
    }

    #[test]
    fn test_request_too_large_failure_class() {
        let err = SnipError::sync_failure(
            SyncFailureKind::RequestTooLarge,
            Some("snippet 'x' too large"),
        );
        assert_eq!(
            crate::sync_failure::FailureClass::from_error(&err),
            crate::sync_failure::FailureClass::Configuration
        );
    }

    #[test]
    fn test_clock_skew_failure_class() {
        let err = SnipError::sync_failure(
            SyncFailureKind::ClockSkew,
            Some("updated_at is 742 seconds ahead"),
        );
        assert_eq!(
            crate::sync_failure::FailureClass::from_error(&err),
            crate::sync_failure::FailureClass::Configuration
        );
    }

    // ── Typed batch-context unit tests ─────────────────────────────
    // These tests exercise the private `add_batch_context` helper used
    // by the multi-batch loop. They previously lived in
    // `tests/sync_multibatch.rs` and were moved here to keep the helper
    // private.

    #[test]
    fn test_batch_context_preserves_clock_skew() {
        let err = SnipError::sync_failure(
            SyncFailureKind::ClockSkew,
            Some("updated_at is 742 seconds ahead"),
        );
        let contextualized = add_batch_context(err, 2, 5);

        match &contextualized {
            SnipError::SyncFailure { kind, detail } => {
                assert!(
                    matches!(kind, SyncFailureKind::ClockSkew),
                    "expected ClockSkew, got {kind:?}"
                );
                let detail = detail.as_deref().expect("detail should be present");
                assert!(
                    detail.contains("batch 2/5"),
                    "detail should include batch context: {detail}"
                );
                assert!(
                    detail.contains("742 seconds"),
                    "detail should preserve original message: {detail}"
                );
            }
            other => panic!("expected SyncFailure(ClockSkew), got {other:?}"),
        }

        assert_eq!(
            crate::sync_failure::FailureClass::from_error(&contextualized),
            crate::sync_failure::FailureClass::Configuration
        );
    }

    #[test]
    fn test_batch_context_preserves_timeout() {
        let err = SnipError::sync_failure(SyncFailureKind::Timeout, Some("deadline exceeded"));
        let contextualized = add_batch_context(err, 1, 3);

        match &contextualized {
            SnipError::SyncFailure { kind, detail } => {
                assert!(
                    matches!(kind, SyncFailureKind::Timeout),
                    "expected Timeout, got {kind:?}"
                );
                let detail = detail.as_deref().expect("detail should be present");
                assert!(
                    detail.contains("batch 1/3"),
                    "detail should include batch context: {detail}"
                );
                assert!(
                    detail.contains("deadline exceeded"),
                    "detail should preserve original message: {detail}"
                );
            }
            other => panic!("expected SyncFailure(Timeout), got {other:?}"),
        }
    }

    #[test]
    fn test_batch_context_preserves_runtime_variant() {
        let err = SnipError::runtime_error("test op", Some("something broke"));
        let contextualized = add_batch_context(err, 3, 4);

        match &contextualized {
            SnipError::Runtime { message, detail } => {
                assert!(
                    message.contains("batch 3/4"),
                    "message should include batch context: {message}"
                );
                assert!(
                    message.contains("test op"),
                    "message should preserve the original error message: {message}"
                );
                assert_eq!(detail.as_deref(), Some("something broke"));
            }
            other => panic!("expected Runtime variant to be preserved, got {other:?}"),
        }

        // Re-wrapping as SyncRequestFailed would classify this as Transient
        // and retry a persistent fault forever; Runtime must stay Internal.
        assert_eq!(
            crate::sync_failure::FailureClass::from_error(&contextualized),
            crate::sync_failure::FailureClass::Internal
        );
    }

    // ── All-encryption-failed regression ───────────────────────────
    // Drives the same prepared-zero-batch pull path as production but
    // with an encrypt closure that always fails. Verifies:
    // - no panic,
    // - all failed local IDs in skipped_ids,
    // - skipped_count matches,
    // - remote snippets still returned,
    // - zero prepared batches still contact the real in-process server.

    #[tokio::test(flavor = "multi_thread")]
    async fn test_all_encryption_failed_accounting() {
        let service = snip_sync::test_helpers::build_test_service().await;
        let (addr, server_task, _captured) =
            snip_sync::test_helpers::start_test_server(service).await;
        let server_url = format!("http://{addr}");

        let (api_key, device_id) = SyncClient::register(server_url.clone())
            .await
            .expect("register should succeed");

        // Seed the server with some snippets via a normal sync.
        let mut setup_client = build_sync_client(&server_url, &api_key).await;
        let now = chrono::Utc::now().timestamp();
        let seed_snippets: Vec<crate::proto::Snippet> = (0..5)
            .map(|i| crate::proto::Snippet {
                id: format!("remote-{i}"),
                description: format!("remote snippet {i}"),
                command: format!("echo remote-{i}"),
                tags: vec![],
                created_at: now,
                updated_at: now,
                device_id: device_id.clone(),
                deleted: false,
                encrypted: false,
            })
            .collect();
        let seed_resp = setup_client
            .sync_encrypted(seed_snippets, 0, "")
            .await
            .expect("seed sync should succeed");
        assert!(seed_resp.success);

        // Now attempt a sync where all local encryption fails.
        let local_snippets: Vec<crate::proto::Snippet> = (0..3)
            .map(|i| crate::proto::Snippet {
                id: format!("local-fail-{i}"),
                description: format!("local snippet {i}"),
                command: format!("echo local-{i}"),
                tags: vec![],
                created_at: now,
                updated_at: now,
                device_id: device_id.clone(),
                deleted: false,
                encrypted: false,
            })
            .collect();

        let mut client = build_sync_client(&server_url, &api_key).await;
        let response = client
            .sync_encrypted_with_test_encrypt(local_snippets, 0, "", TEST_BYTE_CEILING, |_s| {
                Err(SnipError::runtime_error(
                    "encrypt",
                    Some("injected failure"),
                ))
            })
            .await
            .expect("sync should not panic even when all encryption fails");

        // All local snippets should be in skipped_ids.
        assert_eq!(
            response.skipped_ids.len(),
            3,
            "all 3 local snippet IDs should be in skipped_ids"
        );
        for i in 0..3 {
            let expected_id = format!("local-fail-{i}");
            assert!(
                response.skipped_ids.contains(&expected_id),
                "skipped_ids should contain {expected_id}"
            );
        }
        assert_eq!(response.skipped_count, 3, "skipped_count should be 3");

        // Remote snippets should still be returned.
        assert_eq!(
            response.snippets.len(),
            5,
            "remote snippets should still be returned despite local encryption failure"
        );

        // Overall success is true because server returned snippets even
        // though all local encryption failed. The success flag reflects
        // whether any usable data was exchanged, not whether local
        // encryption succeeded.
        assert!(
            response.success,
            "success should be true when server returned snippets"
        );

        server_task.abort();
    }

    const TEST_BYTE_CEILING: usize = 100 * 1024; // 100 KiB

    async fn build_sync_client(server_url: &str, api_key: &str) -> SyncClient {
        use crate::config::{AutoSyncFailureMode, SyncDirection, SyncSettings};

        let settings = SyncSettings {
            enabled: true,
            server_url: server_url.to_string(),
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
            .expect("SyncClient::create should succeed against a plaintext loopback server")
    }
}
