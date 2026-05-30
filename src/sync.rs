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
use snip_proto::snippet_sync_client::SnippetSyncClient;
use snip_proto::PremadeLibrary;
use snip_proto::{
    CreateLibraryRequest, DeleteLibraryRequest, GetPremadeLibraryRequest, GetSnippetsRequest,
    HealthRequest, Library, ListLibrariesRequest, ListPremadeLibrariesRequest, PushSnippetsRequest,
    PushSnippetsResponse, RegisterRequest, SnippetList, SyncRequest,
};
use std::time::Duration;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint, Uri};
use tonic::Code;

const DEFAULT_MAX_RETRIES: u32 = 3;
const DEFAULT_INITIAL_DELAY_MS: u64 = 100;
const DEFAULT_MAX_DELAY_MS: u64 = 5000;

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

/// Retry an async gRPC operation with exponential backoff.
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
                    tracing::warn!(
                        "{} failed (attempt {}/{}): {}. Retrying in {}ms...",
                        $name,
                        attempt + 1,
                        config.max_retries + 1,
                        e,
                        delay_ms
                    );
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    delay_ms = (delay_ms * 2).min(config.max_delay_ms);
                    attempt += 1;
                }
            }
        }
    }};
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

    pub async fn sync_encrypted(
        &mut self,
        local_snippets: Vec<snip_proto::Snippet>,
        last_sync: i64,
        library_id: &str,
    ) -> SnipResult<snip_proto::SyncResponse> {
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

        let request = SyncRequest {
            api_key: api_key.clone(),
            local_snippets: encrypted_snippets,
            last_sync_timestamp: last_sync,
            library_id: library_id.to_string(),
            limit: self.settings.sync_limit_value(),
        };

        let response = self.sync_with_retry(request).await?;

        let mut decrypted_snippets = Vec::new();
        let mut decrypt_failed_ids = Vec::new();

        for s in &response.snippets {
            match decrypt_snippet(&api_key, s) {
                Ok(ds) => decrypted_snippets.push(ds),
                Err(e) => {
                    decrypt_failed_ids.push(s.id.clone());
                    tracing::warn!("Failed to decrypt snippet {}: {}", s.id, e);
                }
            }
        }

        let total_skipped = encrypt_failed_ids.len() + decrypt_failed_ids.len();
        let mut all_skipped_ids = encrypt_failed_ids;
        all_skipped_ids.extend(decrypt_failed_ids);

        let mut response = response;
        response.snippets = decrypted_snippets;
        response.skipped_count = total_skipped as i32;
        response.skipped_ids = all_skipped_ids;

        Ok(response)
    }

    /// Manual retry logic for sync requests.
    ///
    /// Note: The `retry_grpc!` macro cannot be used here because `self.client.sync()`
    /// borrows `&mut self`, and the macro requires the operation to be a standalone
    /// future expression. This method implements the same exponential backoff strategy.
    async fn sync_with_retry(
        &mut self,
        request: SyncRequest,
    ) -> SnipResult<snip_proto::SyncResponse> {
        let config = default_retry_config();
        let mut delay_ms = config.initial_delay_ms;
        for attempt in 0..=config.max_retries {
            let req = SyncRequest {
                api_key: request.api_key.clone(),
                local_snippets: request.local_snippets.clone(),
                last_sync_timestamp: request.last_sync_timestamp,
                library_id: request.library_id.clone(),
                limit: request.limit,
            };
            match self.client.sync(tonic::Request::new(req)).await {
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
                    tracing::warn!(
                        "Sync request failed (attempt {}/{}): {}. Retrying in {}ms...",
                        attempt + 1,
                        config.max_retries + 1,
                        e,
                        delay_ms
                    );
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    delay_ms = (delay_ms * 2).min(config.max_delay_ms);
                }
            }
        }
        unreachable!()
    }

    pub async fn health_check(&mut self) -> SnipResult<bool> {
        match retry_grpc!(
            self.client.health(tonic::Request::new(HealthRequest {})),
            "Health check"
        ) {
            Ok(response) => Ok(response.into_inner().healthy),
            Err(_) => Ok(false),
        }
    }

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

    pub async fn list_libraries(&mut self) -> SnipResult<Vec<Library>> {
        let api_key = self.settings.api_key.clone();
        let response = retry_grpc!(
            self.client
                .list_libraries(tonic::Request::new(ListLibrariesRequest {
                    api_key: api_key.clone(),
                    limit: 50,
                    offset: 0,
                })),
            "List libraries"
        )?;
        Ok(response.into_inner().libraries)
    }

    pub async fn create_library(&mut self, name: &str) -> SnipResult<Library> {
        let api_key = self.settings.api_key.clone();
        let name_str = name.to_string();
        let response = retry_grpc!(
            self.client
                .create_library(tonic::Request::new(CreateLibraryRequest {
                    api_key: api_key.clone(),
                    name: name_str.clone(),
                })),
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

    #[allow(dead_code)]
    pub async fn delete_library(&mut self, library_id: &str) -> SnipResult<()> {
        let api_key = self.settings.api_key.clone();
        let library_id_str = library_id.to_string();
        let response = retry_grpc!(
            self.client
                .delete_library(tonic::Request::new(DeleteLibraryRequest {
                    api_key: api_key.clone(),
                    library_id: library_id_str.clone(),
                })),
            "Delete library"
        )?;

        let response = response.into_inner();
        if response.success {
            Ok(())
        } else {
            Err(SnipError::runtime_error(
                "Failed to delete library",
                Some(&response.message),
            ))
        }
    }

    #[allow(dead_code)]
    pub async fn get_snippets(
        &mut self,
        library_id: &str,
        limit: i32,
        offset: i32,
    ) -> SnipResult<SnippetList> {
        let api_key = self.settings.api_key.clone();
        let library_id_str = library_id.to_string();
        let response = retry_grpc!(
            self.client
                .get_snippets(tonic::Request::new(GetSnippetsRequest {
                    api_key: api_key.clone(),
                    since: 0,
                    library_id: library_id_str.clone(),
                    limit,
                    offset,
                })),
            "Get snippets"
        )?;
        Ok(response.into_inner())
    }

    #[allow(dead_code)]
    pub async fn push_snippets(
        &mut self,
        snippets: &[snip_proto::Snippet],
        library_id: &str,
    ) -> SnipResult<PushSnippetsResponse> {
        let api_key = self.settings.api_key.clone();
        let library_id_str = library_id.to_string();
        let response = retry_grpc!(
            self.client
                .push_snippets(tonic::Request::new(PushSnippetsRequest {
                    api_key: api_key.clone(),
                    snippets: snippets.to_vec(),
                    library_id: library_id_str.clone(),
                })),
            "Push snippets"
        )?;
        Ok(response.into_inner())
    }

    pub async fn list_premade_libraries(&mut self) -> SnipResult<Vec<PremadeLibrary>> {
        let api_key = self.settings.api_key.clone();
        let response = retry_grpc!(
            self.client
                .list_premade_libraries(tonic::Request::new(ListPremadeLibrariesRequest {
                    api_key: api_key.clone(),
                })),
            "List premade libraries"
        )?;
        Ok(response.into_inner().libraries)
    }

    pub async fn get_premade_library(&mut self, filename: &str) -> SnipResult<String> {
        let api_key = self.settings.api_key.clone();
        let filename_str = filename.to_string();
        let response = retry_grpc!(
            self.client
                .get_premade_library(tonic::Request::new(GetPremadeLibraryRequest {
                    api_key: api_key.clone(),
                    filename: filename_str.clone(),
                })),
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

    let host = uri.host().ok_or("No host in URI")?;

    let tls_config = ClientTlsConfig::new()
        .with_enabled_roots()
        .domain_name(host)
        .assume_http2(true);

    let channel = Endpoint::new(uri)?
        .tls_config(tls_config)?
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .connect()
        .await?;

    Ok(channel)
}

pub fn encrypt_snippet(
    api_key: &str,
    snippet: &snip_proto::Snippet,
) -> SnipResult<snip_proto::Snippet> {
    let data = EncryptedSnippetData {
        description: snippet.description.clone(),
        command: snippet.command.clone(),
        tags: snippet.tags.clone(),
    };

    let json = serde_json::to_string(&data)
        .map_err(|e| SnipError::runtime_error("Serialize snippet data", Some(&e.to_string())))?;

    let encrypted = encryption::encrypt(api_key, &json)
        .map_err(|e| SnipError::runtime_error("Encrypt snippet", Some(&e.to_string())))?;

    Ok(snip_proto::Snippet {
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

pub fn decrypt_snippet(
    api_key: &str,
    snippet: &snip_proto::Snippet,
) -> SnipResult<snip_proto::Snippet> {
    if !snippet.encrypted {
        return Ok(snippet.clone());
    }

    let decrypted = encryption::decrypt(api_key, &snippet.command)
        .map_err(|e| SnipError::runtime_error("Decrypt snippet", Some(&e.to_string())))?;

    let data: EncryptedSnippetData = serde_json::from_str(&decrypted)
        .map_err(|e| SnipError::runtime_error("Deserialize snippet data", Some(&e.to_string())))?;

    Ok(snip_proto::Snippet {
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
    server_snippets: &[snip_proto::Snippet],
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
        let snippets = vec![snip_proto::Snippet {
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
        let snippets = vec![snip_proto::Snippet {
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
        let snippets = vec![snip_proto::Snippet {
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
}
