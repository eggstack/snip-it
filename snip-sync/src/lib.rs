// The `format!("{}", var)` patterns below are inherited from the
// pre-existing main.rs implementation. They were not flagged by clippy
// when the code lived in a binary crate's `#[cfg(test)] mod tests`
// (binary test targets are not always checked by `cargo clippy
// --all-targets`), but moving the code into the library makes them
// visible to lib test compilation. The lint suppression keeps the
// refactor minimal; a follow-up should inline the args.
//
// Similarly, `RateLimiter::new()` and a few other constructors predate
// the refactor and never carried a `Default` impl. Suppressing
// `new_without_default` here keeps the diff focused.
#![allow(clippy::uninlined_format_args, clippy::new_without_default)]

pub mod db;
pub mod metrics;
pub mod premade;
pub mod rate_limiter;
#[cfg(any(test, feature = "test-helpers"))]
pub mod test_helpers;

pub use db::Database;
pub use metrics::Metrics;
pub use premade::PremadeManager;
pub use rate_limiter::RateLimiter;

use serde::Deserialize;
use snip_proto::{
    CreateLibraryRequest, CreateLibraryResponse, DeleteLibraryRequest, DeleteLibraryResponse,
    GetPremadeLibraryRequest, GetPremadeLibraryResponse, GetSnippetsRequest, HealthRequest,
    HealthResponse, Library, ListLibrariesRequest, ListLibrariesResponse,
    ListPremadeLibrariesRequest, ListPremadeLibrariesResponse,
    PremadeLibrary as ProtoPremadeLibrary, PushSnippetsRequest, PushSnippetsResponse,
    RegisterRequest, RegisterResponse, Snippet as ProtoSnippet, SnippetList, SyncRequest,
    SyncResponse, snippet_sync_server::SnippetSync,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tonic::{Request, Response, Status};

/// Extract the API key from a gRPC request's `authorization` metadata.
/// Falls back to the body `api_key` field for backward compatibility.
pub fn extract_api_key<T>(request: &Request<T>, body_api_key: &str) -> String {
    request
        .metadata()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| !v.is_empty())
        .unwrap_or(body_api_key)
        .to_string()
}

fn strip_port(addr: &str) -> String {
    if let Some(close_bracket) = addr.find("]:") {
        return addr[1..close_bracket].to_string();
    }
    if let Some(colon_pos) = addr.rfind(':') {
        if !addr.starts_with('[') {
            return addr[..colon_pos].to_string();
        }
    }
    addr.to_string()
}

pub const DEFAULT_MAX_COMMAND_LENGTH: usize = 1024;
pub const DEFAULT_MAX_DESCRIPTION_LENGTH: usize = 1024;
pub const DEFAULT_MAX_TAGS: usize = 50;
pub const DEFAULT_MAX_TAG_LENGTH: usize = 100;
pub const DEFAULT_MAX_ID_LENGTH: usize = 128;
pub const DEFAULT_MAX_DEVICE_ID_LENGTH: usize = 128;
pub const DEFAULT_MAX_API_KEY_LENGTH: usize = 512;
pub const DEFAULT_MAX_SYNC_SNIPPETS: usize = 10000;
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;
pub const DEFAULT_RATE_LIMIT_PER_MINUTE: u32 = 120;
pub const DEFAULT_DB_MAX_CONNECTIONS: u32 = 5;
pub const DEFAULT_GRPC_MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024; // 4 MiB
pub const MAX_REQUEST_LIMIT: i32 = 1000;

#[derive(Deserialize, Default)]
pub struct ConfigFile {
    pub server: Option<ServerConfig>,
}

#[derive(Deserialize, Default)]
pub struct ServerConfig {
    pub grpc_host: Option<String>,
    pub grpc_port: Option<u16>,
    pub http_host: Option<String>,
    pub http_port: Option<u16>,
    pub database: Option<DatabaseConfig>,
    pub premade: Option<PremadeConfig>,
    pub limits: Option<LimitsConfig>,
    pub rate_limit: Option<RateLimitConfig>,
    pub metrics: Option<MetricsConfig>,
    pub cors: Option<CorsConfig>,
}

#[derive(Deserialize, Default)]
pub struct DatabaseConfig {
    pub path: Option<String>,
    pub max_connections: Option<u32>,
}

#[derive(Deserialize, Default)]
pub struct PremadeConfig {
    pub directory: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct LimitsConfig {
    pub max_command_length: Option<usize>,
    pub max_description_length: Option<usize>,
    pub max_tags: Option<usize>,
    pub max_tag_length: Option<usize>,
    pub max_id_length: Option<usize>,
    pub max_device_id_length: Option<usize>,
    pub max_api_key_length: Option<usize>,
    pub request_timeout_secs: Option<u64>,
    pub grpc_max_message_size: Option<usize>,
}

#[derive(Deserialize, Default)]
pub struct RateLimitConfig {
    pub requests_per_minute: Option<u32>,
    pub trusted_proxies: Option<Vec<String>>,
    pub persist: Option<bool>,
}

#[derive(Deserialize, Default)]
pub struct MetricsConfig {
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct CorsConfig {
    pub allowed_origins: Option<String>,
}

#[derive(Clone)]
pub struct Config {
    pub grpc_host: String,
    pub grpc_port: u16,
    pub http_host: String,
    pub http_port: u16,
    pub db_path: String,
    pub db_max_connections: u32,
    pub premade_dir: PathBuf,
    pub max_command_length: usize,
    pub max_description_length: usize,
    pub max_tags: usize,
    pub max_tag_length: usize,
    pub max_id_length: usize,
    pub max_device_id_length: usize,
    pub max_api_key_length: usize,
    pub request_timeout_secs: u64,
    pub grpc_max_message_size: usize,
    pub rate_limit_per_minute: u32,
    pub trusted_proxies: Vec<String>,
    pub persist_rate_limits: bool,
    pub metrics_username: Option<String>,
    pub metrics_password: Option<String>,
    pub cors_allowed_origins: Vec<String>,
}

impl Config {
    pub fn load() -> Self {
        let config_path = std::env::var("CONFIG_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("config.toml"));

        let config_file = if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(content) => match toml::from_str(&content) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!(
                            "Failed to parse config file {}: {}. Using defaults. Fix the config file or remove it to regenerate.",
                            config_path.display(),
                            e
                        );
                        ConfigFile::default()
                    }
                },
                Err(e) => {
                    tracing::error!(
                        "Failed to read config file {}: {}. Using defaults.",
                        config_path.display(),
                        e
                    );
                    ConfigFile::default()
                }
            }
        } else {
            ConfigFile::default()
        };

        let server = config_file.server.unwrap_or_default();

        Self {
            grpc_host: std::env::var("GRPC_HOST")
                .ok()
                .or_else(|| server.grpc_host.clone())
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            grpc_port: std::env::var("GRPC_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server.grpc_port)
                .unwrap_or(50051),
            http_host: std::env::var("HTTP_HOST")
                .ok()
                .or_else(|| server.http_host.clone())
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            http_port: std::env::var("HTTP_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server.http_port)
                .unwrap_or(50050),
            db_path: std::env::var("DATABASE_URL")
                .ok()
                .or_else(|| server.database.as_ref().and_then(|d| d.path.clone()))
                .unwrap_or_else(|| {
                    dirs::home_dir()
                        .unwrap_or_else(|| PathBuf::from("."))
                        .join(".config")
                        .join("snip-sync")
                        .join("snippets.db")
                        .to_string_lossy()
                        .into_owned()
                }),
            db_max_connections: std::env::var("DB_MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server.database.as_ref().and_then(|d| d.max_connections))
                .unwrap_or(DEFAULT_DB_MAX_CONNECTIONS),
            premade_dir: std::env::var("PREMADE_DIR")
                .map(PathBuf::from)
                .ok()
                .or_else(|| {
                    server
                        .premade
                        .as_ref()
                        .and_then(|p| p.directory.clone())
                        .map(PathBuf::from)
                })
                .unwrap_or_else(|| PathBuf::from("premade-libraries")),
            max_command_length: std::env::var("MAX_COMMAND_LENGTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server.limits.as_ref().and_then(|l| l.max_command_length))
                .unwrap_or(DEFAULT_MAX_COMMAND_LENGTH),
            max_description_length: std::env::var("MAX_DESCRIPTION_LENGTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server
                    .limits
                    .as_ref()
                    .and_then(|l| l.max_description_length))
                .unwrap_or(DEFAULT_MAX_DESCRIPTION_LENGTH),
            max_tags: std::env::var("MAX_TAGS")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server.limits.as_ref().and_then(|l| l.max_tags))
                .unwrap_or(DEFAULT_MAX_TAGS),
            max_tag_length: std::env::var("MAX_TAG_LENGTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server.limits.as_ref().and_then(|l| l.max_tag_length))
                .unwrap_or(DEFAULT_MAX_TAG_LENGTH),
            max_id_length: std::env::var("MAX_ID_LENGTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server.limits.as_ref().and_then(|l| l.max_id_length))
                .unwrap_or(DEFAULT_MAX_ID_LENGTH),
            max_device_id_length: std::env::var("MAX_DEVICE_ID_LENGTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server.limits.as_ref().and_then(|l| l.max_device_id_length))
                .unwrap_or(DEFAULT_MAX_DEVICE_ID_LENGTH),
            max_api_key_length: std::env::var("MAX_API_KEY_LENGTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server.limits.as_ref().and_then(|l| l.max_api_key_length))
                .unwrap_or(DEFAULT_MAX_API_KEY_LENGTH),
            request_timeout_secs: std::env::var("REQUEST_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server.limits.as_ref().and_then(|l| l.request_timeout_secs))
                .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS),
            grpc_max_message_size: std::env::var("GRPC_MAX_MESSAGE_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server.limits.as_ref().and_then(|l| l.grpc_max_message_size))
                .unwrap_or(DEFAULT_GRPC_MAX_MESSAGE_SIZE),
            rate_limit_per_minute: std::env::var("RATE_LIMIT_PER_MINUTE")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server
                    .rate_limit
                    .as_ref()
                    .and_then(|r| r.requests_per_minute))
                .unwrap_or(DEFAULT_RATE_LIMIT_PER_MINUTE),
            trusted_proxies: std::env::var("TRUSTED_PROXIES")
                .ok()
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty() && s.parse::<std::net::IpAddr>().is_ok())
                        .collect()
                })
                .or_else(|| server.rate_limit.as_ref()?.trusted_proxies.clone())
                .unwrap_or_default(),
            persist_rate_limits: std::env::var("PERSIST_RATE_LIMITS")
                .ok()
                .map(|v| v == "true" || v == "1")
                .or(server.rate_limit.as_ref().and_then(|r| r.persist))
                .unwrap_or(false),
            metrics_username: std::env::var("METRICS_USERNAME")
                .ok()
                .or_else(|| server.metrics.as_ref().and_then(|m| m.username.clone())),
            metrics_password: std::env::var("METRICS_PASSWORD")
                .ok()
                .or_else(|| server.metrics.as_ref().and_then(|m| m.password.clone())),
            cors_allowed_origins: std::env::var("CORS_ALLOWED_ORIGINS")
                .ok()
                .or_else(|| server.cors.as_ref().and_then(|c| c.allowed_origins.clone()))
                .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default(),
        }
    }

    pub fn ensure_config_file() {
        let config_path = std::env::var("CONFIG_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("config.toml"));

        if !config_path.exists() {
            let default_config = include_str!("../config.toml");
            if let Err(e) = std::fs::write(&config_path, default_config) {
                tracing::warn!("Failed to create default config file: {}", e);
            } else {
                tracing::info!("Created default config file at {}", config_path.display());
            }
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub metrics: Metrics,
    pub db: Arc<Database>,
}

pub struct SnipSyncService {
    pub db: Arc<Database>,
    pub rate_limiter: Arc<RateLimiter>,
    pub config: Config,
    pub metrics: Metrics,
    pub premade_manager: PremadeManager,
    /// Test-only: most recent `authorization` header value observed by
    /// the gRPC handlers. Used by integration tests to assert that the
    /// sync client actually sends a non-empty bearer token in metadata
    /// (catching the auth-header regression where `request.api_key` was
    /// read instead of the caller's API key).
    pub captured_auth_header: Arc<std::sync::Mutex<Option<String>>>,
}

impl SnipSyncService {
    pub fn record_request(&self, method: &str) {
        self.metrics
            .requests_total
            .with_label_values(&[method])
            .inc();
        tracing::trace!("Request: {}", method);
    }

    pub fn record_request_duration(&self, method: &str, start: std::time::Instant) {
        let duration = start.elapsed().as_secs_f64();
        self.metrics
            .request_duration_seconds
            .with_label_values(&[method])
            .observe(duration);
        tracing::debug!("Request {} completed in {:.3}s", method, duration);
    }

    pub fn record_rate_limit(&self) {
        self.metrics.rate_limit_hits.inc();
    }

    pub fn record_auth_failure(&self) {
        self.metrics.auth_failures.inc();
    }

    pub fn record_sync(&self, direction: &str) {
        self.metrics
            .sync_operations_total
            .with_label_values(&[direction])
            .inc();
    }

    pub fn record_library_op(&self, operation: &str) {
        self.metrics
            .library_operations_total
            .with_label_values(&[operation])
            .inc();
    }

    fn capture_auth_header<T>(&self, request: &Request<T>, body_api_key: &str) -> String {
        let api_key = extract_api_key(request, body_api_key);
        if !api_key.is_empty() {
            let mut slot = self
                .captured_auth_header
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                *slot = Some(format!("Bearer {api_key}"));
            }
        }
        api_key
    }

    pub async fn authenticate_and_rate_limit(&self, api_key: &str) -> Result<String, Status> {
        if api_key.is_empty() {
            return Err(Status::unauthenticated("API key is required"));
        }
        if api_key.len() > self.config.max_api_key_length {
            return Err(Status::invalid_argument(format!(
                "API key exceeds maximum length of {} bytes",
                self.config.max_api_key_length
            )));
        }

        if !self
            .rate_limiter
            .allow(
                api_key,
                self.config.rate_limit_per_minute as usize,
                Duration::from_secs(60),
            )
            .await
        {
            self.record_rate_limit();
            return Err(Status::resource_exhausted("Rate limit exceeded"));
        }

        let user_id = self
            .db
            .get_user_by_api_key(api_key)
            .await
            .map_err(|_| Status::internal("Internal error"))?
            .ok_or_else(|| {
                self.record_auth_failure();
                Status::unauthenticated("Invalid API key")
            })?;

        Ok(user_id)
    }

    pub fn validate_snippet(&self, snippet: &ProtoSnippet) -> Result<(), Status> {
        if snippet.id.is_empty() {
            return Err(Status::invalid_argument("Snippet ID is required"));
        }
        if snippet.id.len() > self.config.max_id_length {
            return Err(Status::invalid_argument(format!(
                "Snippet ID exceeds maximum length of {} bytes",
                self.config.max_id_length
            )));
        }

        if snippet.device_id.is_empty() {
            return Err(Status::invalid_argument("Device ID is required"));
        }
        if snippet.device_id.len() > self.config.max_device_id_length {
            return Err(Status::invalid_argument(format!(
                "Device ID exceeds maximum length of {} bytes",
                self.config.max_device_id_length
            )));
        }

        if snippet.command.len() > self.config.max_command_length {
            return Err(Status::invalid_argument(format!(
                "Command exceeds maximum length of {} bytes",
                self.config.max_command_length
            )));
        }

        if snippet.description.len() > self.config.max_description_length {
            return Err(Status::invalid_argument(format!(
                "Description exceeds maximum length of {} bytes",
                self.config.max_description_length
            )));
        }

        if snippet.tags.len() > self.config.max_tags {
            return Err(Status::invalid_argument(format!(
                "Too many tags (max {})",
                self.config.max_tags
            )));
        }

        for tag in &snippet.tags {
            if tag.len() > self.config.max_tag_length {
                return Err(Status::invalid_argument(format!(
                    "Tag '{}' exceeds maximum length of {} bytes",
                    tag, self.config.max_tag_length
                )));
            }
        }

        let now = chrono::Utc::now().timestamp();
        if snippet.updated_at > now + 300 {
            return Err(Status::invalid_argument(
                "Updated timestamp is more than 5 minutes in the future",
            ));
        }
        if snippet.created_at > now + 300 {
            return Err(Status::invalid_argument(
                "Created timestamp is more than 5 minutes in the future",
            ));
        }
        if snippet.created_at < 0 || snippet.updated_at < 0 {
            return Err(Status::invalid_argument("Timestamps must be non-negative"));
        }
        if snippet.created_at > snippet.updated_at {
            return Err(Status::invalid_argument(
                "created_at must not be greater than updated_at",
            ));
        }

        Ok(())
    }
}

#[tonic::async_trait]
impl SnippetSync for SnipSyncService {
    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let request_id = uuid::Uuid::new_v4();
        let start = std::time::Instant::now();
        self.record_request("health");
        tracing::debug!(request_id = %request_id, "Health check");
        let healthy = self.db.ping().await.is_ok();
        self.record_request_duration("health", start);
        Ok(Response::new(HealthResponse {
            healthy,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }))
    }

    async fn register(
        &self,
        request: Request<RegisterRequest>,
    ) -> Result<Response<RegisterResponse>, Status> {
        let request_id = uuid::Uuid::new_v4();
        let start = std::time::Instant::now();
        self.record_request("register");
        tracing::info!(request_id = %request_id, "Register request");

        // Use peer IP address for rate limiting (device_id is client-controlled)
        // Only trust x-forwarded-for if it comes from a trusted proxy
        let rate_limit_key = if let Some(proxy_ip) = request
            .extensions()
            .get::<axum::extract::ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip().to_string())
            .filter(|ip| self.config.trusted_proxies.contains(ip))
        {
            request
                .metadata()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|header| header.split(',').next())
                .map(|s| strip_port(s.trim()))
                .unwrap_or(proxy_ip)
        } else {
            request
                .extensions()
                .get::<axum::extract::ConnectInfo<SocketAddr>>()
                .map(|info| info.0.ip().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        };

        if !self
            .rate_limiter
            .allow(
                &rate_limit_key,
                self.config.rate_limit_per_minute as usize,
                Duration::from_secs(60),
            )
            .await
        {
            self.record_rate_limit();
            return Err(Status::resource_exhausted(
                "Rate limit exceeded for registration",
            ));
        }

        let _req = request.into_inner();

        let api_key = uuid::Uuid::new_v4().to_string();

        match self.db.create_user(&api_key).await {
            Ok(device_id) => {
                if uuid::Uuid::parse_str(&device_id).is_err() {
                    tracing::error!(
                        request_id = %request_id,
                        "Generated device_id is not a valid UUID: {}",
                        device_id
                    );
                    return Err(Status::internal(
                        "Internal error: invalid device ID generated",
                    ));
                }
                tracing::info!(
                    request_id = %request_id,
                    device_id = %device_id,
                    "Created new user"
                );
                self.record_request_duration("register", start);
                Ok(Response::new(RegisterResponse {
                    success: true,
                    api_key,
                    message: "Account created successfully".to_string(),
                    device_id,
                }))
            }
            Err(e) => {
                tracing::error!(request_id = %request_id, "Failed to create user: {}", e);
                self.record_request_duration("register", start);
                Err(Status::internal("Internal error"))
            }
        }
    }

    async fn get_snippets(
        &self,
        request: Request<GetSnippetsRequest>,
    ) -> Result<Response<SnippetList>, Status> {
        let request_id = uuid::Uuid::new_v4();
        self.record_request("get_snippets");
        tracing::info!(request_id = %request_id, "GetSnippets request");
        let api_key = self.capture_auth_header(&request, &request.get_ref().api_key);
        let req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&api_key).await?;

        let library_id = if req.library_id.is_empty() {
            self.db.get_default_library(&user_id).await.map_err(|e| {
                tracing::error!("Internal error: {}", e);
                Status::internal("Internal error")
            })?
        } else {
            if !self
                .db
                .verify_library_ownership(&user_id, &req.library_id)
                .await
                .map_err(|e| {
                    tracing::error!("Internal error: {}", e);
                    Status::internal("Internal error")
                })?
            {
                tracing::warn!(
                    "User {} attempted to access library {} without ownership",
                    user_id,
                    req.library_id
                );
                return Err(Status::not_found("Library not found"));
            }
            req.library_id
        };

        let limit = if req.limit > 0 {
            req.limit.clamp(1, MAX_REQUEST_LIMIT)
        } else {
            100
        };
        let offset = if req.offset > 0 { req.offset } else { 0 };

        let (snippets, total) = self
            .db
            .get_snippets(&user_id, &library_id, req.since, limit, offset, false)
            .await
            .map_err(|e| {
                tracing::error!("Internal error: {}", e);
                Status::internal("Internal error")
            })?;

        let has_more = offset.saturating_add(snippets.len() as i32) < total;

        let proto_snippets: Vec<ProtoSnippet> = snippets
            .into_iter()
            .map(|s| ProtoSnippet {
                id: s.id,
                description: s.description,
                command: s.command,
                tags: s.tags,
                created_at: s.created_at,
                updated_at: s.updated_at,
                device_id: s.device_id,
                deleted: s.deleted,
                encrypted: s.encrypted,
            })
            .collect();

        Ok(Response::new(SnippetList {
            snippets: proto_snippets,
            total_count: total,
            has_more,
        }))
    }

    async fn push_snippets(
        &self,
        request: Request<PushSnippetsRequest>,
    ) -> Result<Response<PushSnippetsResponse>, Status> {
        let request_id = uuid::Uuid::new_v4();
        self.record_request("push_snippets");
        tracing::info!(request_id = %request_id, "PushSnippets request");
        let api_key = self.capture_auth_header(&request, &request.get_ref().api_key);
        let req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&api_key).await?;

        let library_id = if req.library_id.is_empty() {
            self.db.get_default_library(&user_id).await.map_err(|e| {
                tracing::error!("Internal error: {}", e);
                Status::internal("Internal error")
            })?
        } else {
            if !self
                .db
                .verify_library_ownership(&user_id, &req.library_id)
                .await
                .map_err(|e| {
                    tracing::error!("Internal error: {}", e);
                    Status::internal("Internal error")
                })?
            {
                tracing::warn!(
                    "User {} attempted to push to library {} without ownership",
                    user_id,
                    req.library_id
                );
                return Err(Status::not_found("Library not found"));
            }
            req.library_id
        };

        if req.snippets.len() > DEFAULT_MAX_SYNC_SNIPPETS {
            return Err(Status::invalid_argument(format!(
                "Too many snippets in push request (max {}), got {}",
                DEFAULT_MAX_SYNC_SNIPPETS,
                req.snippets.len()
            )));
        }

        let mut accepted = 0;
        let mut rejected = 0;

        let mut tx = self.db.pool().begin().await.map_err(|e| {
            tracing::error!(request_id = %request_id, "Failed to begin transaction: {}", e);
            Status::internal("Internal error")
        })?;

        for snippet in req.snippets {
            if let Err(e) = self.validate_snippet(&snippet) {
                rejected += 1;
                tracing::warn!(request_id = %request_id, snippet_id = %snippet.id, reason = %e, "Snippet validation failed");
                continue;
            }

            let db_snippet = db::Snippet {
                id: snippet.id.clone(),
                description: snippet.description,
                command: snippet.command,
                tags: snippet.tags,
                created_at: snippet.created_at,
                updated_at: snippet.updated_at,
                device_id: snippet.device_id,
                deleted: snippet.deleted,
                encrypted: snippet.encrypted,
            };

            match self
                .db
                .upsert_snippet_in_tx(&mut tx, &db_snippet, &user_id, &library_id)
                .await
            {
                Ok(_) => accepted += 1,
                Err(e) => {
                    tracing::warn!(
                        request_id = %request_id,
                        snippet_id = %snippet.id,
                        reason = %e,
                        "Snippet upsert failed"
                    );
                    rejected += 1;
                }
            }
        }

        tx.commit().await.map_err(|e| {
            tracing::error!(request_id = %request_id, "Failed to commit transaction: {}", e);
            Status::internal("Internal error")
        })?;

        Ok(Response::new(PushSnippetsResponse {
            success: rejected == 0,
            message: format!("Accepted {}, rejected {}", accepted, rejected),
            accepted_count: accepted,
            rejected_count: rejected,
        }))
    }

    async fn sync(&self, request: Request<SyncRequest>) -> Result<Response<SyncResponse>, Status> {
        let request_id = uuid::Uuid::new_v4();
        let start = std::time::Instant::now();
        self.record_request("sync");
        self.record_sync("bidirectional");
        tracing::info!(request_id = %request_id, "Sync request");
        let api_key = self.capture_auth_header(&request, &request.get_ref().api_key);
        let req = request.into_inner();

        if req.local_snippets.len() > DEFAULT_MAX_SYNC_SNIPPETS {
            return Err(Status::invalid_argument(format!(
                "Too many snippets in sync request (max {}), got {}",
                DEFAULT_MAX_SYNC_SNIPPETS,
                req.local_snippets.len()
            )));
        }

        let user_id = self.authenticate_and_rate_limit(&api_key).await?;

        let library_id = if req.library_id.is_empty() {
            self.db.get_default_library(&user_id).await.map_err(|e| {
                tracing::error!("Internal error: {}", e);
                Status::internal("Internal error")
            })?
        } else {
            if !self
                .db
                .verify_library_ownership(&user_id, &req.library_id)
                .await
                .map_err(|e| {
                    tracing::error!("Internal error: {}", e);
                    Status::internal("Internal error")
                })?
            {
                tracing::warn!(
                    "User {} attempted to sync with library {} without ownership",
                    user_id,
                    req.library_id
                );
                return Err(Status::not_found("Library not found"));
            }
            req.library_id
        };

        let mut skipped_ids = Vec::new();

        let mut tx = self.db.pool().begin().await.map_err(|e| {
            tracing::error!(request_id = %request_id, "Failed to begin transaction: {}", e);
            Status::internal("Internal error")
        })?;

        for snippet in &req.local_snippets {
            if let Err(e) = self.validate_snippet(snippet) {
                tracing::warn!(
                    request_id = %request_id,
                    snippet_id = %snippet.id,
                    reason = %e,
                    "Snippet skipped: validation failed"
                );
                skipped_ids.push(snippet.id.clone());
                continue;
            }

            if snippet.updated_at <= req.last_sync_timestamp {
                continue;
            }

            let db_snippet = db::Snippet {
                id: snippet.id.clone(),
                description: snippet.description.clone(),
                command: snippet.command.clone(),
                tags: snippet.tags.clone(),
                created_at: snippet.created_at,
                updated_at: snippet.updated_at,
                device_id: snippet.device_id.clone(),
                deleted: snippet.deleted,
                encrypted: snippet.encrypted,
            };

            if let Err(e) = self
                .db
                .upsert_snippet_in_tx(&mut tx, &db_snippet, &user_id, &library_id)
                .await
            {
                tracing::warn!(
                    request_id = %request_id,
                    snippet_id = %snippet.id,
                    reason = %e,
                    "Snippet skipped: upsert failed"
                );
                skipped_ids.push(snippet.id.clone());
            }
        }

        tx.commit().await.map_err(|e| {
            tracing::error!(request_id = %request_id, "Failed to commit transaction: {}", e);
            Status::internal("Internal error")
        })?;

        let limit = if req.limit > 0 {
            req.limit.clamp(1, MAX_REQUEST_LIMIT)
        } else {
            1000
        };

        let offset = if req.offset > 0 { req.offset } else { 0 };

        let (snippets, total) = self
            .db
            .get_snippets(
                &user_id,
                &library_id,
                req.last_sync_timestamp,
                limit,
                offset,
                true,
            )
            .await
            .map_err(|e| {
                tracing::error!("Internal error: {}", e);
                Status::internal("Internal error")
            })?;

        let has_more = offset.saturating_add(snippets.len() as i32) < total;

        let timestamp = if has_more {
            // Don't advance timestamp when paginating — client needs to fetch remaining pages
            req.last_sync_timestamp
        } else {
            self.db
                .get_latest_timestamp(&user_id, &library_id)
                .await
                .map_err(|e| {
                    tracing::error!("Internal error: {}", e);
                    Status::internal("Internal error")
                })?
        };

        let proto_snippets: Vec<ProtoSnippet> = snippets
            .into_iter()
            .map(|s| ProtoSnippet {
                id: s.id,
                description: s.description,
                command: s.command,
                tags: s.tags,
                created_at: s.created_at,
                updated_at: s.updated_at,
                device_id: s.device_id,
                deleted: s.deleted,
                encrypted: s.encrypted,
            })
            .collect();

        let skipped_count = skipped_ids.len() as i32;

        tracing::info!(
            request_id = %request_id,
            snippets_synced = proto_snippets.len(),
            skipped_count = skipped_count,
            has_more = has_more,
            "Sync completed"
        );

        self.record_request_duration("sync", start);

        Ok(Response::new(SyncResponse {
            success: true,
            message: if skipped_count > 0 {
                format!("Sync completed, {} snippets skipped", skipped_count)
            } else {
                "Sync completed".to_string()
            },
            snippets: proto_snippets,
            server_timestamp: timestamp,
            skipped_count,
            skipped_ids,
            has_more,
            total_count: total,
        }))
    }

    async fn create_library(
        &self,
        request: Request<CreateLibraryRequest>,
    ) -> Result<Response<CreateLibraryResponse>, Status> {
        let request_id = uuid::Uuid::new_v4();
        self.record_request("create_library");
        self.record_library_op("create");
        tracing::info!(request_id = %request_id, "CreateLibrary request");
        let api_key = self.capture_auth_header(&request, &request.get_ref().api_key);
        let req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&api_key).await?;

        match self.db.create_library(&user_id, &req.name).await {
            Ok(lib_id) => {
                tracing::info!("Created library '{}' for user {}", req.name, user_id);
                Ok(Response::new(CreateLibraryResponse {
                    success: true,
                    library_id: lib_id,
                    message: format!("Library '{}' created successfully", req.name),
                }))
            }
            Err(db::DbError::Conflict(msg)) => Err(Status::already_exists(msg)),
            Err(db::DbError::NotFound(msg)) => Err(Status::invalid_argument(msg)),
            Err(db::DbError::Database(e)) => {
                tracing::error!("Database error creating library: {}", e);
                Err(Status::internal("Internal error"))
            }
        }
    }

    async fn list_libraries(
        &self,
        request: Request<ListLibrariesRequest>,
    ) -> Result<Response<ListLibrariesResponse>, Status> {
        let request_id = uuid::Uuid::new_v4();
        self.record_request("list_libraries");
        self.record_library_op("list");
        tracing::info!(request_id = %request_id, "ListLibraries request");
        let api_key = self.capture_auth_header(&request, &request.get_ref().api_key);
        let req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&api_key).await?;

        let limit = if req.limit > 0 {
            req.limit.clamp(1, MAX_REQUEST_LIMIT)
        } else {
            50
        };
        let offset = if req.offset > 0 { req.offset } else { 0 };

        let (libraries, total) = self
            .db
            .list_libraries(&user_id, limit, offset)
            .await
            .map_err(|e| {
                tracing::error!("Internal error: {}", e);
                Status::internal("Internal error")
            })?;

        let has_more = offset.saturating_add(libraries.len() as i32) < total;

        let proto_libraries: Vec<Library> = libraries
            .into_iter()
            .map(|l| Library {
                id: l.id,
                name: l.name,
                created_at: l.created_at,
                snippet_count: l.snippet_count,
            })
            .collect();

        Ok(Response::new(ListLibrariesResponse {
            libraries: proto_libraries,
            total_count: total,
            has_more,
        }))
    }

    async fn delete_library(
        &self,
        request: Request<DeleteLibraryRequest>,
    ) -> Result<Response<DeleteLibraryResponse>, Status> {
        let request_id = uuid::Uuid::new_v4();
        self.record_request("delete_library");
        self.record_library_op("delete");
        tracing::info!(request_id = %request_id, "DeleteLibrary request");
        let api_key = self.capture_auth_header(&request, &request.get_ref().api_key);
        let req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&api_key).await?;

        // Prevent deleting default library
        if req.library_id.is_empty() || req.library_id == "default" {
            return Err(Status::invalid_argument("Cannot delete default library"));
        }

        match self.db.delete_library(&user_id, &req.library_id).await {
            Ok(_) => {
                tracing::info!("Deleted library {} for user {}", req.library_id, user_id);
                Ok(Response::new(DeleteLibraryResponse {
                    success: true,
                    message: "Library deleted successfully".to_string(),
                }))
            }
            Err(e) => {
                tracing::error!("Failed to delete library: {}", e);
                Err(Status::not_found("Library not found"))
            }
        }
    }

    async fn list_premade_libraries(
        &self,
        request: Request<ListPremadeLibrariesRequest>,
    ) -> Result<Response<ListPremadeLibrariesResponse>, Status> {
        let request_id = uuid::Uuid::new_v4();
        self.record_request("list_premade_libraries");
        tracing::info!(request_id = %request_id, "ListPremadeLibraries request");

        let api_key = self.capture_auth_header(&request, &request.get_ref().api_key);
        let _req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&api_key).await?;

        let libraries = self.premade_manager.list();

        let proto_libraries: Vec<ProtoPremadeLibrary> = libraries
            .into_iter()
            .map(|l| ProtoPremadeLibrary {
                name: l.name,
                filename: l.filename,
                description: l.description,
                snippet_count: l.snippet_count,
            })
            .collect();

        tracing::info!(
            request_id = %request_id,
            user_id = %user_id,
            count = proto_libraries.len(),
            "ListPremadeLibraries completed"
        );

        let count = proto_libraries.len() as i32;

        Ok(Response::new(ListPremadeLibrariesResponse {
            libraries: proto_libraries,
            has_more: false,
            total_count: count,
        }))
    }

    async fn get_premade_library(
        &self,
        request: Request<GetPremadeLibraryRequest>,
    ) -> Result<Response<GetPremadeLibraryResponse>, Status> {
        let request_id = uuid::Uuid::new_v4();
        self.record_request("get_premade_library");
        tracing::info!(request_id = %request_id, "GetPremadeLibrary request");
        let api_key = self.capture_auth_header(&request, &request.get_ref().api_key);
        let req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&api_key).await?;

        if req.filename.is_empty() {
            return Err(Status::invalid_argument("Filename is required"));
        }

        if req.filename.contains("..") || req.filename.contains('/') || req.filename.contains('\\')
        {
            return Err(Status::invalid_argument("Invalid filename"));
        }

        let sanitized: String = req
            .filename
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
            .collect();

        if sanitized.is_empty() || sanitized.len() > 64 {
            return Err(Status::invalid_argument("Invalid filename"));
        }

        let content = match self.premade_manager.get(&sanitized) {
            Ok(c) => c,
            Err(e) => return Err(e),
        };

        tracing::info!("User {} fetched premade library '{}'", user_id, sanitized);

        Ok(Response::new(GetPremadeLibraryResponse {
            success: true,
            name: sanitized.clone(),
            content,
            message: "Library fetched successfully".to_string(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snip_proto::{
        CreateLibraryRequest, GetSnippetsRequest, HealthRequest, ListLibrariesRequest,
        PushSnippetsRequest, RegisterRequest, Snippet as ProtoSnippet, SyncRequest,
    };

    pub async fn setup_test_service() -> SnipSyncService {
        let db = Arc::new(Database::connect("sqlite::memory:", 5).await.unwrap());
        let config = Config {
            grpc_host: "127.0.0.1".to_string(),
            grpc_port: 50051,
            http_host: "127.0.0.1".to_string(),
            http_port: 50050,
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

    pub async fn register_test_user(service: &SnipSyncService) -> String {
        let req = Request::new(RegisterRequest {
            device_id: "test-device".to_string(),
        });
        let resp = service.register(req).await.unwrap();
        resp.into_inner().api_key
    }

    #[tokio::test]
    async fn test_health_check() {
        let service = setup_test_service().await;
        let req = Request::new(HealthRequest {});
        let resp = service.health(req).await.unwrap();
        let health = resp.into_inner();
        assert!(health.healthy);
        assert_eq!(health.version, env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn test_register_creates_user() {
        let service = setup_test_service().await;
        let req = Request::new(RegisterRequest {
            device_id: "test-device".to_string(),
        });
        let resp = service.register(req).await.unwrap();
        let reg = resp.into_inner();
        assert!(reg.success);
        assert!(!reg.api_key.is_empty());
        assert!(!reg.device_id.is_empty());
        assert!(uuid::Uuid::parse_str(&reg.device_id).is_ok());
    }

    #[tokio::test]
    async fn test_register_rate_limiting() {
        let service = setup_test_service().await;
        for _ in 0..120 {
            let req = Request::new(RegisterRequest {
                device_id: "test-device".to_string(),
            });
            let _ = service.register(req).await;
        }
        let req = Request::new(RegisterRequest {
            device_id: "test-device".to_string(),
        });
        let resp = service.register(req).await;
        assert!(resp.is_err());
        assert_eq!(resp.unwrap_err().code(), tonic::Code::ResourceExhausted);
    }

    #[tokio::test]
    async fn test_create_and_list_libraries() {
        let service = setup_test_service().await;
        let api_key = register_test_user(&service).await;

        let req = Request::new(CreateLibraryRequest {
            api_key: api_key.clone(),
            name: "test-lib".to_string(),
        });
        let resp = service.create_library(req).await.unwrap();
        let create = resp.into_inner();
        assert!(create.success);
        assert!(!create.library_id.is_empty());

        let req = Request::new(ListLibrariesRequest {
            api_key: api_key.clone(),
            limit: 100,
            offset: 0,
        });
        let resp = service.list_libraries(req).await.unwrap();
        let list = resp.into_inner();
        assert_eq!(list.libraries.len(), 2);
        assert!(list.libraries.iter().any(|l| l.name == "test-lib"));
    }

    #[tokio::test]
    async fn test_create_duplicate_library_name() {
        let service = setup_test_service().await;
        let api_key = register_test_user(&service).await;

        let req = Request::new(CreateLibraryRequest {
            api_key: api_key.clone(),
            name: "test-lib".to_string(),
        });
        let _ = service.create_library(req).await.unwrap();

        let req = Request::new(CreateLibraryRequest {
            api_key: api_key.clone(),
            name: "test-lib".to_string(),
        });
        let resp = service.create_library(req).await;
        assert!(resp.is_err());
        assert_eq!(resp.unwrap_err().code(), tonic::Code::AlreadyExists);
    }

    #[tokio::test]
    async fn test_invalid_api_key() {
        let service = setup_test_service().await;
        let req = Request::new(GetSnippetsRequest {
            api_key: "invalid-key".to_string(),
            library_id: String::new(),
            limit: 100,
            offset: 0,
            since: 0,
        });
        let resp = service.get_snippets(req).await;
        assert!(resp.is_err());
        assert_eq!(resp.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn test_validate_snippet_empty_id() {
        let service = setup_test_service().await;
        let snippet = ProtoSnippet {
            id: String::new(),
            description: "test".to_string(),
            command: "echo hello".to_string(),
            tags: vec![],
            created_at: 0,
            updated_at: 0,
            device_id: "device".to_string(),
            deleted: false,
            encrypted: false,
        };
        let result = service.validate_snippet(&snippet);
        assert!(result.is_err());
        assert!(result.unwrap_err().message().contains("ID is required"));
    }

    #[tokio::test]
    async fn test_validate_snippet_oversized_command() {
        let service = setup_test_service().await;
        let snippet = ProtoSnippet {
            id: "test-id".to_string(),
            description: "test".to_string(),
            command: "x".repeat(2000),
            tags: vec![],
            created_at: 0,
            updated_at: 0,
            device_id: "device".to_string(),
            deleted: false,
            encrypted: false,
        };
        let result = service.validate_snippet(&snippet);
        assert!(result.is_err());
        assert!(result.unwrap_err().message().contains("maximum length"));
    }

    #[tokio::test]
    async fn test_validate_snippet_future_timestamp() {
        let service = setup_test_service().await;
        let future = chrono::Utc::now().timestamp() + 600;
        let snippet = ProtoSnippet {
            id: "test-id".to_string(),
            description: "test".to_string(),
            command: "echo hello".to_string(),
            tags: vec![],
            created_at: future,
            updated_at: future,
            device_id: "device".to_string(),
            deleted: false,
            encrypted: false,
        };
        let result = service.validate_snippet(&snippet);
        assert!(result.is_err());
        assert!(result.unwrap_err().message().contains("future"));
    }

    #[tokio::test]
    async fn test_validate_snippet_negative_timestamp() {
        let service = setup_test_service().await;
        let snippet = ProtoSnippet {
            id: "test-id".to_string(),
            description: "test".to_string(),
            command: "echo hello".to_string(),
            tags: vec![],
            created_at: -1,
            updated_at: -1,
            device_id: "device".to_string(),
            deleted: false,
            encrypted: false,
        };
        let result = service.validate_snippet(&snippet);
        assert!(result.is_err());
        assert!(result.unwrap_err().message().contains("non-negative"));
    }

    #[tokio::test]
    async fn test_validate_snippet_too_many_tags() {
        let service = setup_test_service().await;
        let snippet = ProtoSnippet {
            id: "test-id".to_string(),
            description: "test".to_string(),
            command: "echo hello".to_string(),
            tags: (0..100).map(|i| format!("tag{}", i)).collect(),
            created_at: 0,
            updated_at: 0,
            device_id: "device".to_string(),
            deleted: false,
            encrypted: false,
        };
        let result = service.validate_snippet(&snippet);
        assert!(result.is_err());
        assert!(result.unwrap_err().message().contains("Too many tags"));
    }

    #[tokio::test]
    async fn test_validate_snippet_valid() {
        let service = setup_test_service().await;
        let snippet = ProtoSnippet {
            id: "test-id".to_string(),
            description: "test".to_string(),
            command: "echo hello".to_string(),
            tags: vec!["bash".to_string()],
            created_at: 0,
            updated_at: 0,
            device_id: "device".to_string(),
            deleted: false,
            encrypted: false,
        };
        assert!(service.validate_snippet(&snippet).is_ok());
    }

    #[tokio::test]
    async fn test_cross_user_library_access_denied() {
        let service = setup_test_service().await;
        let api_key1 = register_test_user(&service).await;

        let req = Request::new(CreateLibraryRequest {
            api_key: api_key1.clone(),
            name: "user1-lib".to_string(),
        });
        let resp = service.create_library(req).await.unwrap();
        let library_id = resp.into_inner().library_id;

        let api_key2 = register_test_user(&service).await;

        let req = Request::new(GetSnippetsRequest {
            api_key: api_key2,
            library_id,
            limit: 100,
            offset: 0,
            since: 0,
        });
        let resp = service.get_snippets(req).await;
        assert!(resp.is_err());
        assert_eq!(resp.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_push_and_get_snippets() {
        let service = setup_test_service().await;
        let api_key = register_test_user(&service).await;

        let req = Request::new(CreateLibraryRequest {
            api_key: api_key.clone(),
            name: "test-lib".to_string(),
        });
        let resp = service.create_library(req).await.unwrap();
        let library_id = resp.into_inner().library_id;

        let snippet = ProtoSnippet {
            id: "snippet-1".to_string(),
            description: "Test snippet".to_string(),
            command: "echo hello".to_string(),
            tags: vec!["test".to_string()],
            created_at: chrono::Utc::now().timestamp(),
            updated_at: chrono::Utc::now().timestamp(),
            device_id: "device-1".to_string(),
            deleted: false,
            encrypted: false,
        };
        let req = Request::new(PushSnippetsRequest {
            api_key: api_key.clone(),
            library_id: library_id.clone(),
            snippets: vec![snippet],
        });
        let resp = service.push_snippets(req).await.unwrap();
        let push = resp.into_inner();
        assert_eq!(push.accepted_count, 1);
        assert_eq!(push.rejected_count, 0);

        let req = Request::new(GetSnippetsRequest {
            api_key: api_key.clone(),
            library_id,
            limit: 100,
            offset: 0,
            since: 0,
        });
        let resp = service.get_snippets(req).await.unwrap();
        let list = resp.into_inner();
        assert_eq!(list.snippets.len(), 1);
        assert_eq!(list.snippets[0].id, "snippet-1");
        assert_eq!(list.snippets[0].command, "echo hello");
    }

    #[tokio::test]
    async fn test_delete_library() {
        let service = setup_test_service().await;
        let api_key = register_test_user(&service).await;

        let req = Request::new(CreateLibraryRequest {
            api_key: api_key.clone(),
            name: "to-delete".to_string(),
        });
        let resp = service.create_library(req).await.unwrap();
        let library_id = resp.into_inner().library_id;

        let req = Request::new(snip_proto::DeleteLibraryRequest {
            api_key: api_key.clone(),
            library_id: library_id.clone(),
        });
        let resp = service.delete_library(req).await.unwrap();
        assert!(resp.into_inner().success);

        let req = Request::new(ListLibrariesRequest {
            api_key: api_key.clone(),
            limit: 100,
            offset: 0,
        });
        let resp = service.list_libraries(req).await.unwrap();
        let list = resp.into_inner();
        assert_eq!(list.libraries.len(), 1);
        assert_eq!(list.libraries[0].name, "default");
    }

    #[tokio::test]
    async fn test_delete_default_library_blocked() {
        let service = setup_test_service().await;
        let api_key = register_test_user(&service).await;

        let req = Request::new(snip_proto::DeleteLibraryRequest {
            api_key: api_key.clone(),
            library_id: "default".to_string(),
        });
        let resp = service.delete_library(req).await;
        assert!(resp.is_err());
        assert!(resp.unwrap_err().message().contains("Cannot delete"));
    }

    #[tokio::test]
    async fn test_sync_basic() {
        let service = setup_test_service().await;
        let api_key = register_test_user(&service).await;

        let req = Request::new(CreateLibraryRequest {
            api_key: api_key.clone(),
            name: "sync-test".to_string(),
        });
        let resp = service.create_library(req).await.unwrap();
        let library_id = resp.into_inner().library_id;

        let req = Request::new(SyncRequest {
            api_key: api_key.clone(),
            local_snippets: vec![],
            last_sync_timestamp: 0,
            library_id,
            limit: 1000,
            offset: 0,
        });
        let resp = service.sync(req).await.unwrap();
        let sync = resp.into_inner();
        assert_eq!(sync.snippets.len(), 0);
    }
}
