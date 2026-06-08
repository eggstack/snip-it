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
use crate::error::{SnipError, SnipResult};
use crate::proto::PremadeLibrary;
use crate::proto::snippet_sync_client::SnippetSyncClient;
use crate::proto::{
    CreateLibraryRequest, GetPremadeLibraryRequest, HealthRequest, Library, ListLibrariesRequest,
    ListPremadeLibrariesRequest, RegisterRequest, SyncRequest,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tonic::Code;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint, Uri};

static JITTER_COUNTER: AtomicU32 = AtomicU32::new(0);

const DEFAULT_MAX_RETRIES: u32 = 3; // Total attempts: 1 initial + 3 retries = 4
const DEFAULT_INITIAL_DELAY_MS: u64 = 100; // Initial backoff before first retry
const DEFAULT_MAX_DELAY_MS: u64 = 5000; // Cap exponential backoff at 5 seconds
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;

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
                        break Err(SnipError::runtime_error($name, Some(&e.to_string())));
                    }
                    let jitter_ms = {
                        let counter = JITTER_COUNTER.fetch_add(1, Ordering::Relaxed);
                        let nanos = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.subsec_nanos())
                            .unwrap_or(0);
                        let combined = nanos.wrapping_add(counter.wrapping_mul(31));
                        0.5 + ((combined as u64 % 1000) as f64 / 1000.0)
                    };
                    let actual_delay = (delay_ms as f64 * jitter_ms) as u64;
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
}

impl SyncClient {
    /// Creates a new sync client connected to the server specified in settings.
    pub async fn create(settings: SyncSettings) -> SnipResult<Self> {
        let server_url = settings.server_url.clone();

        let channel = create_tls_channel(&server_url)
            .await
            .map_err(|e| SnipError::runtime_error("Failed to connect", Some(&e.to_string())))?;

        Ok(Self {
            client: SnippetSyncClient::new(channel),
            settings,
        })
    }

    /// Encrypts local snippets, sends them to the server, and decrypts the response.
    ///
    /// Snippets that fail encryption/decryption are counted as skipped.
    /// Handles server-side pagination by fetching all pages before returning.
    pub async fn sync_encrypted(
        &mut self,
        local_snippets: Vec<crate::proto::Snippet>,
        last_sync: i64,
        library_id: &str,
    ) -> SnipResult<crate::proto::SyncResponse> {
        let api_key = self.settings.api_key.clone();

        let mut encrypted_snippets = Vec::new();
        let mut encrypt_failed_ids = Vec::new();

        for s in &local_snippets {
            match encrypt_snippet(&api_key, s) {
                Ok(es) => encrypted_snippets.push(es),
                Err(e) => {
                    encrypt_failed_ids.push(s.id.clone());
                    tracing::warn!("Failed to encrypt snippet {}: {}", s.id, e);
                }
            }
        }

        let mut request = SyncRequest {
            api_key: String::new(), // Auth via gRPC metadata, not body
            local_snippets: Vec::new(),
            last_sync_timestamp: last_sync,
            library_id: library_id.to_string(),
            limit: self.settings.sync_limit_value(),
            offset: 0,
        };

        let local_snippets_for_push = encrypted_snippets.clone();
        let encrypt_failed_count = encrypt_failed_ids.len();
        let mut all_server_snippets = Vec::new();
        let mut all_skipped_ids = encrypt_failed_ids;
        let mut server_decrypt_failed_count = 0usize;
        let mut final_timestamp;
        let mut final_message;
        let mut final_total_count;
        let mut offset = 0;

        loop {
            if offset == 0 {
                request.local_snippets = local_snippets_for_push.clone();
            } else {
                request.local_snippets.clear();
            }
            request.offset = offset;

            let mut response = self.sync_with_retry(request.clone(), &api_key).await?;

            // Decrypt server snippets from this page
            for s in &response.snippets {
                match decrypt_snippet(&api_key, s) {
                    Ok(ds) => all_server_snippets.push(ds),
                    Err(e) => {
                        server_decrypt_failed_count += 1;
                        all_skipped_ids.push(s.id.clone());
                        tracing::warn!("Failed to decrypt snippet {}: {}", s.id, e);
                    }
                }
            }

            final_timestamp = response.server_timestamp;
            final_message = std::mem::take(&mut response.message);
            final_total_count = response.total_count;

            if !response.has_more || response.snippets.is_empty() {
                break;
            }

            offset = offset.saturating_add(response.snippets.len() as i32);
        }

        let total_skipped = all_skipped_ids.len();
        let all_skipped_local = encrypt_failed_count > 0 && encrypted_snippets.is_empty();
        // Failure only if ALL server snippets failed to decrypt (not just some)
        let all_skipped_server = server_decrypt_failed_count > 0
            && all_server_snippets.is_empty()
            && server_decrypt_failed_count >= encrypt_failed_count;
        let overall_success = !(all_skipped_local || all_skipped_server);

        Ok(crate::proto::SyncResponse {
            success: overall_success,
            message: final_message,
            snippets: all_server_snippets,
            server_timestamp: final_timestamp,
            skipped_count: total_skipped as i32,
            skipped_ids: all_skipped_ids,
            has_more: false,
            total_count: final_total_count,
        })
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
        loop {
            let mut grpc_req = tonic::Request::new(request.clone());
            add_api_key_metadata(&mut grpc_req, api_key);
            match self.client.sync(grpc_req).await {
                Ok(response) => return Ok(response.into_inner()),
                Err(e) => {
                    if !SyncRetryConfig::is_retryable_grpc_error(&e)
                        || attempt >= config.max_retries
                    {
                        return Err(SnipError::runtime_error(
                            "Sync request",
                            Some(&e.to_string()),
                        ));
                    }
                    let is_rate_limited = e.code() == Code::ResourceExhausted;
                    tracing::warn!(
                        "Sync request failed (attempt {}/{}): {}. Retrying in {}ms...",
                        attempt + 1,
                        config.max_retries + 1,
                        e,
                        delay_ms
                    );
                    let jitter_ms = {
                        let counter = JITTER_COUNTER.fetch_add(1, Ordering::Relaxed);
                        let nanos = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.subsec_nanos())
                            .unwrap_or(0);
                        let combined = nanos.wrapping_add(counter.wrapping_mul(31));
                        0.5 + ((combined as u64 % 1000) as f64 / 1000.0)
                    };
                    let actual_delay = (delay_ms as f64 * jitter_ms) as u64;
                    tokio::time::sleep(Duration::from_millis(actual_delay)).await;
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
        match retry_grpc!(
            self.client.health(tonic::Request::new(HealthRequest {})),
            "Health check"
        ) {
            Ok(response) => Ok(response.into_inner().healthy),
            Err(e) => {
                tracing::debug!(error = %e, "Health check failed");
                Ok(false)
            }
        }
    }

    /// Registers a new device with the server and returns the API key and device ID.
    pub async fn register(server_url: String) -> SnipResult<(String, String)> {
        let channel = create_tls_channel(&server_url)
            .await
            .map_err(|e| SnipError::runtime_error("Failed to connect", Some(&e.to_string())))?;

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
            Err(SnipError::runtime_error(
                "Registration failed",
                Some(&response.message),
            ))
        }
    }

    /// Lists all libraries on the sync server.
    pub async fn list_libraries(&mut self) -> SnipResult<Vec<Library>> {
        let api_key = self.settings.api_key.clone();
        let mut all_libraries = Vec::new();
        let mut offset = 0i32;
        const PAGE_LIMIT: i32 = 50;
        loop {
            let response = retry_grpc!(
                async {
                    let mut req = tonic::Request::new(ListLibrariesRequest {
                        api_key: api_key.clone(),
                        limit: PAGE_LIMIT,
                        offset,
                    });
                    add_api_key_metadata(&mut req, &api_key);
                    self.client.list_libraries(req).await
                },
                "List libraries"
            )?;
            let inner = response.into_inner();
            let count = inner.libraries.len() as i32;
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
        let api_key = self.settings.api_key.clone();
        let name_str = name.to_string();
        let response = retry_grpc!(
            async {
                let mut req = tonic::Request::new(CreateLibraryRequest {
                    api_key: api_key.clone(),
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
            Err(SnipError::runtime_error(
                "Failed to create library",
                Some(&response.message),
            ))
        }
    }

    /// Lists all premade libraries available on the server.
    pub async fn list_premade_libraries(&mut self) -> SnipResult<Vec<PremadeLibrary>> {
        let api_key = self.settings.api_key.clone();
        let response = retry_grpc!(
            async {
                let mut req = tonic::Request::new(ListPremadeLibrariesRequest {
                    api_key: api_key.clone(),
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
        let api_key = self.settings.api_key.clone();
        let filename_str = filename.to_string();
        let response = retry_grpc!(
            async {
                let mut req = tonic::Request::new(GetPremadeLibraryRequest {
                    api_key: api_key.clone(),
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
            Err(SnipError::runtime_error(
                "Failed to get premade library",
                Some(&response.message),
            ))
        }
    }
}

async fn create_tls_channel(
    server_url: &str,
) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
    let uri: Uri = server_url.parse()?;
    let scheme = uri.scheme_str().unwrap_or("https").to_ascii_lowercase();
    let host = if scheme == "https" {
        Some(uri.host().ok_or("No host in URI")?.to_string())
    } else {
        None
    };

    let connect_timeout_secs = std::env::var("SNP_SYNC_CONNECT_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CONNECT_TIMEOUT_SECS);
    let request_timeout_secs = std::env::var("SNP_SYNC_REQUEST_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS);

    let endpoint = Endpoint::new(uri)?
        .connect_timeout(Duration::from_secs(connect_timeout_secs))
        .timeout(Duration::from_secs(request_timeout_secs));

    let endpoint = if let Some(host) = host {
        let tls_config = ClientTlsConfig::new()
            .with_enabled_roots()
            .domain_name(host)
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

    let json = serde_json::to_string(&data)
        .map_err(|e| SnipError::runtime_error("Serialize snippet data", Some(&e.to_string())))?;

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

/// Decrypts a snippet's encrypted fields received from the sync server.
pub fn decrypt_snippet(
    api_key: &str,
    snippet: &crate::proto::Snippet,
) -> SnipResult<crate::proto::Snippet> {
    if !snippet.encrypted {
        return Ok(snippet.clone());
    }

    let decrypted = encryption::decrypt(api_key, &snippet.command)?;

    let data: EncryptedSnippetData = serde_json::from_str(&decrypted)
        .map_err(|e| SnipError::runtime_error("Deserialize snippet data", Some(&e.to_string())))?;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
