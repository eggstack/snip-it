mod db;
mod metrics;
mod premade;
mod rate_limiter;

use axum::extract::State;
use base64::Engine;
use db::Database;
use metrics::Metrics;
use premade::PremadeManager;
use rate_limiter::RateLimiter;
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
use tower_http::cors::{Any, CorsLayer};

const DEFAULT_MAX_COMMAND_LENGTH: usize = 1024;
const DEFAULT_MAX_DESCRIPTION_LENGTH: usize = 1024;
const DEFAULT_MAX_TAGS: usize = 50;
const DEFAULT_MAX_TAG_LENGTH: usize = 100;
const DEFAULT_MAX_ID_LENGTH: usize = 128;
const DEFAULT_MAX_DEVICE_ID_LENGTH: usize = 128;
const DEFAULT_MAX_SYNC_SNIPPETS: usize = 10000;
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;
const DEFAULT_RATE_LIMIT_PER_MINUTE: u32 = 120;
const DEFAULT_DB_MAX_CONNECTIONS: u32 = 5;
const DEFAULT_GRPC_MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024; // 4 MiB
const MAX_REQUEST_LIMIT: i32 = 1000;

#[derive(Deserialize, Default)]
struct ConfigFile {
    server: Option<ServerConfig>,
}

#[derive(Deserialize, Default)]
struct ServerConfig {
    grpc_host: Option<String>,
    grpc_port: Option<u16>,
    http_host: Option<String>,
    http_port: Option<u16>,
    database: Option<DatabaseConfig>,
    premade: Option<PremadeConfig>,
    limits: Option<LimitsConfig>,
    rate_limit: Option<RateLimitConfig>,
    metrics: Option<MetricsConfig>,
    cors: Option<CorsConfig>,
}

#[derive(Deserialize, Default)]
struct DatabaseConfig {
    path: Option<String>,
    max_connections: Option<u32>,
}

#[derive(Deserialize, Default)]
struct PremadeConfig {
    directory: Option<String>,
}

#[derive(Deserialize, Default)]
struct LimitsConfig {
    max_command_length: Option<usize>,
    max_description_length: Option<usize>,
    max_tags: Option<usize>,
    max_tag_length: Option<usize>,
    max_id_length: Option<usize>,
    max_device_id_length: Option<usize>,
    request_timeout_secs: Option<u64>,
    grpc_max_message_size: Option<usize>,
}

#[derive(Deserialize, Default)]
struct RateLimitConfig {
    requests_per_minute: Option<u32>,
    trusted_proxies: Option<Vec<String>>,
    persist: Option<bool>,
}

#[derive(Deserialize, Default)]
struct MetricsConfig {
    username: Option<String>,
    password: Option<String>,
}

#[derive(Deserialize, Default)]
struct CorsConfig {
    allowed_origins: Option<String>,
}

#[derive(Clone)]
struct Config {
    grpc_host: String,
    grpc_port: u16,
    http_host: String,
    http_port: u16,
    db_path: String,
    db_max_connections: u32,
    premade_dir: PathBuf,
    max_command_length: usize,
    max_description_length: usize,
    max_tags: usize,
    max_tag_length: usize,
    max_id_length: usize,
    max_device_id_length: usize,
    request_timeout_secs: u64,
    grpc_max_message_size: usize,
    rate_limit_per_minute: u32,
    trusted_proxies: Vec<String>,
    persist_rate_limits: bool,
    metrics_username: Option<String>,
    metrics_password: Option<String>,
    cors_allowed_origins: Vec<String>,
}

impl Config {
    fn load() -> Self {
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

    fn ensure_config_file() {
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
struct AppState {
    config: Config,
    metrics: Metrics,
    db: Arc<Database>,
}

struct SnipSyncService {
    db: Arc<Database>,
    rate_limiter: Arc<RateLimiter>,
    config: Config,
    metrics: Metrics,
    premade_manager: PremadeManager,
}

impl SnipSyncService {
    fn record_request(&self, method: &str) {
        self.metrics
            .requests_total
            .with_label_values(&[method])
            .inc();
        tracing::trace!("Request: {}", method);
    }

    fn record_request_duration(&self, method: &str, start: std::time::Instant) {
        let duration = start.elapsed().as_secs_f64();
        self.metrics
            .request_duration_seconds
            .with_label_values(&[method])
            .observe(duration);
        tracing::debug!("Request {} completed in {:.3}s", method, duration);
    }

    fn record_rate_limit(&self) {
        self.metrics.rate_limit_hits.inc();
    }

    fn record_auth_failure(&self) {
        self.metrics.auth_failures.inc();
    }

    fn record_sync(&self, direction: &str) {
        self.metrics
            .sync_operations_total
            .with_label_values(&[direction])
            .inc();
    }

    fn record_library_op(&self, operation: &str) {
        self.metrics
            .library_operations_total
            .with_label_values(&[operation])
            .inc();
    }

    async fn authenticate_and_rate_limit(&self, api_key: &str) -> Result<String, Status> {
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

    fn validate_snippet(&self, snippet: &ProtoSnippet) -> Result<(), Status> {
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
                .map(|s| s.trim().to_string())
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
        let req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&req.api_key).await?;

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

        let has_more = (offset + snippets.len() as i32) < total;

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
        let req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&req.api_key).await?;

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
        let req = request.into_inner();

        if req.local_snippets.len() > DEFAULT_MAX_SYNC_SNIPPETS {
            return Err(Status::invalid_argument(format!(
                "Too many snippets in sync request (max {}), got {}",
                DEFAULT_MAX_SYNC_SNIPPETS,
                req.local_snippets.len()
            )));
        }

        let user_id = self.authenticate_and_rate_limit(&req.api_key).await?;

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

        let has_more = (offset + snippets.len() as i32) < total;

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
        let req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&req.api_key).await?;

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
        let req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&req.api_key).await?;

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

        let has_more = (offset + libraries.len() as i32) < total;

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
        let req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&req.api_key).await?;

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

        let req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&req.api_key).await?;

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
        let req = request.into_inner();

        let user_id = self.authenticate_and_rate_limit(&req.api_key).await?;

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

async fn security_headers_middleware(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("x-frame-options", "DENY".parse().unwrap());
    headers.insert("cache-control", "no-store".parse().unwrap());
    response
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_target(false).init();

    let tls_enabled = std::env::var("TLS_ENABLED")
        .ok()
        .is_some_and(|v| v == "true" || v == "1");

    tracing::info!("Starting snip-sync server v{}", env!("CARGO_PKG_VERSION"));

    if !tls_enabled {
        if std::env::var_os("SNIP_SYNC_ALLOW_HTTP").is_some_and(|v| v == "true") {
            tracing::warn!(
                "TLS is not enabled. For production, use a reverse proxy with TLS (nginx, traefik, etc.) \
                 (explicitly allowed via SNIP_SYNC_ALLOW_HTTP=true)"
            );
        } else {
            tracing::error!(
                "TLS is not enabled. For production, set TLS_ENABLED=true or use a reverse proxy with TLS. \
                 Set SNIP_SYNC_ALLOW_HTTP=true to allow plaintext HTTP."
            );
            return Err(
                "TLS is required for production. Set TLS_ENABLED=true or SNIP_SYNC_ALLOW_HTTP=true"
                    .into(),
            );
        }
    }

    Config::ensure_config_file();
    let config = Config::load();

    let db = Arc::new(Database::connect(&config.db_path, config.db_max_connections).await?);
    tracing::info!("Database initialized at {}", config.db_path);

    match db.migrate_plaintext_api_keys().await {
        Ok(count) if count > 0 => tracing::info!("Migrated {} API keys to hashed format", count),
        Ok(_) => tracing::debug!("No plaintext API keys to migrate"),
        Err(e) => {
            tracing::error!(
                "API key migration failed: {}. Halting startup to prevent auth lockout.",
                e
            );
            return Err(e.into());
        }
    }

    let grpc_addr = format!("{}:{}", config.grpc_host, config.grpc_port).parse::<SocketAddr>()?;
    let http_addr = format!("{}:{}", config.http_host, config.http_port).parse::<SocketAddr>()?;

    tracing::info!("gRPC server listening on {}", grpc_addr);
    tracing::info!("HTTP server listening on {}", http_addr);

    let rate_limiter = if config.persist_rate_limits {
        tracing::info!("Rate limiter persistence enabled (PERSIST_RATE_LIMITS=true)");
        Arc::new(RateLimiter::new_with_db(db.pool().clone(), true))
    } else {
        Arc::new(RateLimiter::new())
    };
    rate_limiter.load_state().await;
    rate_limiter.start_persistence_task();
    let cors_allowed_origins = config.cors_allowed_origins.clone();

    tracing::info!(
        "Input validation config: max_command={}, max_description={}, max_tags={}, max_tag_length={}, request_timeout={}s",
        config.max_command_length,
        config.max_description_length,
        config.max_tags,
        config.max_tag_length,
        config.request_timeout_secs
    );

    let timeout = Duration::from_secs(config.request_timeout_secs);

    let metrics = match Metrics::new() {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(
                "Failed to create metrics: {}. Metrics will be unavailable.",
                e
            );
            Metrics::fallback()
        }
    };

    if config.metrics_username.is_some() && config.metrics_password.is_some() {
        tracing::info!("Metrics endpoint enabled with authentication");
    } else if config.metrics_username.is_some() || config.metrics_password.is_some() {
        tracing::warn!(
            "Metrics endpoint disabled: both METRICS_USERNAME and METRICS_PASSWORD must be set (only one provided)"
        );
    } else {
        tracing::warn!("Metrics endpoint disabled: METRICS_USERNAME and METRICS_PASSWORD not set");
    }

    let premade_manager = PremadeManager::new(config.premade_dir.clone());
    if premade_manager.is_empty() {
        tracing::warn!(
            "No premade libraries found in {}",
            config.premade_dir.display()
        );
    } else {
        tracing::info!(
            "Premade libraries loaded from {}",
            config.premade_dir.display()
        );
    }

    let state = AppState {
        config: config.clone(),
        metrics: metrics.clone(),
        db: db.clone(),
    };

    let grpc_max_message_size = config.grpc_max_message_size;

    let grpc_service = SnipSyncService {
        db: db.clone(),
        rate_limiter: rate_limiter.clone(),
        config,
        metrics,
        premade_manager,
    };

    let cors_allow_all = std::env::var("CORS_ALLOW_ALL")
        .ok()
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let cors = if cors_allow_all {
        tracing::info!("CORS: allowing all origins (CORS_ALLOW_ALL=true)");
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else if cors_allowed_origins.is_empty() {
        tracing::warn!(
            "CORS: no origins configured. Cross-origin requests will be blocked. \
             Set CORS_ALLOWED_ORIGINS to allow specific origins, or CORS_ALLOW_ALL=true for permissive CORS."
        );
        CorsLayer::new()
    } else {
        let mut cors = CorsLayer::new();
        for origin in &cors_allowed_origins {
            if let Ok(header_value) = origin.parse::<axum::http::HeaderValue>() {
                cors = cors.allow_origin(header_value);
            }
        }
        tracing::info!("CORS allowed origins: {:?}", cors_allowed_origins);
        cors.allow_methods([axum::http::Method::GET])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
            ])
    };

    async fn metrics_handler(
        State(state): State<AppState>,
        headers: axum::http::HeaderMap,
    ) -> Result<String, (axum::http::StatusCode, String)> {
        let (username, password) = match (
            &state.config.metrics_username,
            &state.config.metrics_password,
        ) {
            (Some(u), Some(p)) => (u.as_str(), p.as_str()),
            _ => {
                return Err((axum::http::StatusCode::NOT_FOUND, "Not found".to_string()));
            }
        };

        let auth_header = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Basic "));

        let expected = format!("{}:{}", username, password);
        let valid = if let Some(encoded) = auth_header {
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                use subtle::ConstantTimeEq;
                let expected_bytes = expected.as_bytes();
                if decoded.len() != expected_bytes.len() {
                    false
                } else {
                    bool::from(decoded.ct_eq(expected_bytes))
                }
            } else {
                false
            }
        } else {
            false
        };

        if !valid {
            return Err((
                axum::http::StatusCode::UNAUTHORIZED,
                "Authentication required".to_string(),
            ));
        }

        use prometheus::Encoder;
        let encoder = prometheus::TextEncoder::new();
        let mut buffer = Vec::new();
        if let Err(e) = encoder.encode(&state.metrics.registry.gather(), &mut buffer) {
            return Err((
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Error gathering metrics: {}", e),
            ));
        }
        Ok(String::from_utf8(buffer).unwrap_or_default())
    }

    let app = axum::Router::new()
        .route(
            "/health",
            axum::routing::get(|State(state): State<AppState>| async move {
                let healthy = state.db.ping().await.is_ok();
                let status = if healthy { "healthy" } else { "unhealthy" };
                let code = if healthy {
                    axum::http::StatusCode::OK
                } else {
                    axum::http::StatusCode::SERVICE_UNAVAILABLE
                };
                (
                    code,
                    axum::Json(serde_json::json!({
                        "version": env!("CARGO_PKG_VERSION"),
                        "status": status
                    })),
                )
            }),
        )
        .route("/metrics", axum::routing::get(metrics_handler))
        .layer(axum::middleware::from_fn(security_headers_middleware))
        .layer(cors)
        .with_state(state);

    let grpc_handle = tokio::spawn(async move {
        let server = tonic::transport::Server::builder()
            .timeout(timeout)
            .max_frame_size(grpc_max_message_size as u32)
            .add_service(snip_proto::snippet_sync_server::SnippetSyncServer::new(
                grpc_service,
            ))
            .serve(grpc_addr);

        tracing::info!(
            "gRPC server listening on http://{} (timeout: {}s)",
            grpc_addr,
            timeout.as_secs()
        );

        tokio::select! {
            result = server => {
                if let Err(e) = result {
                    tracing::error!("gRPC server error: {}", e);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown signal received, stopping gRPC server...");
            }
        }
    });

    let http_handle = tokio::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(http_addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(addr = %http_addr, error = %e, "Failed to bind HTTP listener");
                return;
            }
        };
        tracing::info!("HTTP server listening on http://{}", http_addr);

        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
                tracing::info!("Shutdown signal received, stopping HTTP server...");
            })
            .await
        {
            tracing::error!(error = %e, "HTTP server error");
        }
    });

    let _ = tokio::join!(grpc_handle, http_handle);

    tracing::info!("Server shutdown complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use snip_proto::{
        CreateLibraryRequest, GetSnippetsRequest, HealthRequest, ListLibrariesRequest,
        PushSnippetsRequest, RegisterRequest, Snippet as ProtoSnippet, SyncRequest,
    };
    use std::sync::Arc;

    async fn setup_test_service() -> SnipSyncService {
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
        }
    }

    async fn register_test_user(service: &SnipSyncService) -> String {
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
        // Exhaust rate limit
        for _ in 0..120 {
            let req = Request::new(RegisterRequest {
                device_id: "test-device".to_string(),
            });
            let _ = service.register(req).await;
        }
        // Next request should be rate limited
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

        // Create library
        let req = Request::new(CreateLibraryRequest {
            api_key: api_key.clone(),
            name: "test-lib".to_string(),
        });
        let resp = service.create_library(req).await.unwrap();
        let create = resp.into_inner();
        assert!(create.success);
        assert!(!create.library_id.is_empty());

        // List libraries - should have default + test-lib
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

        // Create with same name should fail
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
        let future = chrono::Utc::now().timestamp() + 600; // 10 minutes in future
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

        // Create library with user 1
        let req = Request::new(CreateLibraryRequest {
            api_key: api_key1.clone(),
            name: "user1-lib".to_string(),
        });
        let resp = service.create_library(req).await.unwrap();
        let library_id = resp.into_inner().library_id;

        // Register user 2
        let api_key2 = register_test_user(&service).await;

        // User 2 tries to access user 1's library
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

        // Create library
        let req = Request::new(CreateLibraryRequest {
            api_key: api_key.clone(),
            name: "test-lib".to_string(),
        });
        let resp = service.create_library(req).await.unwrap();
        let library_id = resp.into_inner().library_id;

        // Push snippets
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

        // Get snippets
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

        // Create library
        let req = Request::new(CreateLibraryRequest {
            api_key: api_key.clone(),
            name: "to-delete".to_string(),
        });
        let resp = service.create_library(req).await.unwrap();
        let library_id = resp.into_inner().library_id;

        // Delete library
        let req = Request::new(snip_proto::DeleteLibraryRequest {
            api_key: api_key.clone(),
            library_id: library_id.clone(),
        });
        let resp = service.delete_library(req).await.unwrap();
        assert!(resp.into_inner().success);

        // Verify deleted - should still have default library
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

        // Create library
        let req = Request::new(CreateLibraryRequest {
            api_key: api_key.clone(),
            name: "sync-test".to_string(),
        });
        let resp = service.create_library(req).await.unwrap();
        let library_id = resp.into_inner().library_id;

        // Sync with empty local snippets (pull only)
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
