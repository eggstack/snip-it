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
    snippet_sync_server::SnippetSync, CreateLibraryRequest, CreateLibraryResponse,
    DeleteLibraryRequest, DeleteLibraryResponse, GetPremadeLibraryRequest,
    GetPremadeLibraryResponse, GetSnippetsRequest, HealthRequest, HealthResponse, Library,
    ListLibrariesRequest, ListLibrariesResponse, ListPremadeLibrariesRequest,
    ListPremadeLibrariesResponse, PremadeLibrary as ProtoPremadeLibrary, PushSnippetsRequest,
    PushSnippetsResponse, RegisterRequest, RegisterResponse, Snippet as ProtoSnippet, SnippetList,
    SyncRequest, SyncResponse,
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
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;
const DEFAULT_RATE_LIMIT_PER_MINUTE: u32 = 120;
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
    request_timeout_secs: Option<u64>,
}

#[derive(Deserialize, Default)]
struct RateLimitConfig {
    requests_per_minute: Option<u32>,
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
    premade_dir: PathBuf,
    max_command_length: usize,
    max_description_length: usize,
    max_tags: usize,
    max_tag_length: usize,
    request_timeout_secs: u64,
    rate_limit_per_minute: u32,
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
                        tracing::warn!(
                            "Failed to parse config file {}: {}. Using defaults.",
                            config_path.display(),
                            e
                        );
                        ConfigFile::default()
                    }
                },
                Err(e) => {
                    tracing::warn!("Failed to read config file: {}", e);
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
                .unwrap_or_else(|| "snippets.db".to_string()),
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
            request_timeout_secs: std::env::var("REQUEST_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server.limits.as_ref().and_then(|l| l.request_timeout_secs))
                .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS),
            rate_limit_per_minute: std::env::var("RATE_LIMIT_PER_MINUTE")
                .ok()
                .and_then(|v| v.parse().ok())
                .or(server
                    .rate_limit
                    .as_ref()
                    .and_then(|r| r.requests_per_minute))
                .unwrap_or(DEFAULT_RATE_LIMIT_PER_MINUTE),
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
}

struct SnipSyncService {
    db: Arc<Database>,
    rate_limiter: Arc<RateLimiter>,
    config: Config,
    metrics: Metrics,
    premade_manager: PremadeManager,
}

impl SnipSyncService {
    fn record_request(&self, _method: &str) {
        self.metrics.requests_total.inc();
    }

    fn record_rate_limit(&self) {
        self.metrics.rate_limit_hits.inc();
    }

    fn record_auth_failure(&self) {
        self.metrics.auth_failures.inc();
    }

    fn record_sync(&self) {
        self.metrics.sync_operations_total.inc();
    }

    fn record_library_op(&self) {
        self.metrics.library_operations_total.inc();
    }

    fn validate_snippet(&self, snippet: &ProtoSnippet) -> Result<(), Status> {
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

        Ok(())
    }
}

#[tonic::async_trait]
impl SnippetSync for SnipSyncService {
    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        self.record_request("health");
        Ok(Response::new(HealthResponse {
            healthy: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }))
    }

    async fn register(
        &self,
        request: Request<RegisterRequest>,
    ) -> Result<Response<RegisterResponse>, Status> {
        self.record_request("register");
        let req = request.into_inner();

        if !self
            .rate_limiter
            .allow(
                &req.device_id,
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

        let api_key = uuid::Uuid::new_v4().to_string();

        match self.db.create_user(&api_key).await {
            Ok(device_id) => {
                tracing::info!("Created new user with device_id: {}", device_id);
                Ok(Response::new(RegisterResponse {
                    success: true,
                    api_key,
                    message: "Account created successfully".to_string(),
                    device_id,
                }))
            }
            Err(e) => {
                tracing::error!("Failed to create user: {}", e);
                Err(Status::internal(format!("Failed to create user: {}", e)))
            }
        }
    }

    async fn get_snippets(
        &self,
        request: Request<GetSnippetsRequest>,
    ) -> Result<Response<SnippetList>, Status> {
        self.record_request("get_snippets");
        let req = request.into_inner();

        let user_id = self
            .db
            .get_user_by_api_key(&req.api_key)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| {
                tracing::warn!("Invalid API key attempted");
                self.record_auth_failure();
                Status::unauthenticated("Invalid API key")
            })?;

        let library_id = if req.library_id.is_empty() {
            self.db
                .get_default_library(&user_id)
                .await
                .map_err(|e| Status::internal(e.to_string()))?
        } else {
            if !self
                .db
                .verify_library_ownership(&user_id, &req.library_id)
                .await
                .map_err(|e| Status::internal(e.to_string()))?
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
            .get_snippets(&user_id, &library_id, req.since, limit, offset)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

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
        self.record_request("push_snippets");
        let req = request.into_inner();

        if !self
            .rate_limiter
            .allow(
                &req.api_key,
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
            .get_user_by_api_key(&req.api_key)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| {
                self.record_auth_failure();
                Status::unauthenticated("Invalid API key")
            })?;

        let library_id = if req.library_id.is_empty() {
            self.db
                .get_default_library(&user_id)
                .await
                .map_err(|e| Status::internal(e.to_string()))?
        } else {
            if !self
                .db
                .verify_library_ownership(&user_id, &req.library_id)
                .await
                .map_err(|e| Status::internal(e.to_string()))?
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

        let mut accepted = 0;
        let mut rejected = 0;

        for snippet in req.snippets {
            if let Err(e) = self.validate_snippet(&snippet) {
                rejected += 1;
                tracing::warn!("Snippet validation failed: {}", e);
                continue;
            }

            let db_snippet = db::Snippet {
                id: snippet.id,
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
                .upsert_snippet(&db_snippet, &user_id, &library_id)
                .await
            {
                Ok(_) => accepted += 1,
                Err(_) => rejected += 1,
            }
        }

        Ok(Response::new(PushSnippetsResponse {
            success: rejected == 0,
            message: format!("Accepted {}, rejected {}", accepted, rejected),
            accepted_count: accepted,
            rejected_count: rejected,
        }))
    }

    async fn sync(&self, request: Request<SyncRequest>) -> Result<Response<SyncResponse>, Status> {
        self.record_request("sync");
        self.record_sync();
        let req = request.into_inner();

        if !self
            .rate_limiter
            .allow(
                &req.api_key,
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
            .get_user_by_api_key(&req.api_key)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| {
                self.record_auth_failure();
                Status::unauthenticated("Invalid API key")
            })?;

        let library_id = if req.library_id.is_empty() {
            self.db
                .get_default_library(&user_id)
                .await
                .map_err(|e| Status::internal(e.to_string()))?
        } else {
            if !self
                .db
                .verify_library_ownership(&user_id, &req.library_id)
                .await
                .map_err(|e| Status::internal(e.to_string()))?
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

        for snippet in &req.local_snippets {
            if snippet.updated_at > req.last_sync_timestamp {
                if let Err(e) = self.validate_snippet(snippet) {
                    tracing::warn!("Snippet validation failed: {}", e);
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
                    .upsert_snippet(&db_snippet, &user_id, &library_id)
                    .await
                {
                    tracing::warn!("Failed to upsert snippet: {}", e);
                }
            }
        }

        let limit = if req.limit > 0 {
            req.limit.clamp(1, MAX_REQUEST_LIMIT)
        } else {
            1000
        };

        let (snippets, _) = self
            .db
            .get_snippets(&user_id, &library_id, req.last_sync_timestamp, limit, 0)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let timestamp = self
            .db
            .get_latest_timestamp(&user_id, &library_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

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

        Ok(Response::new(SyncResponse {
            success: true,
            message: "Sync completed".to_string(),
            snippets: proto_snippets,
            server_timestamp: timestamp,
            skipped_count: 0,
            skipped_ids: vec![],
        }))
    }

    async fn create_library(
        &self,
        request: Request<CreateLibraryRequest>,
    ) -> Result<Response<CreateLibraryResponse>, Status> {
        self.record_request("create_library");
        self.record_library_op();
        let req = request.into_inner();

        if !self
            .rate_limiter
            .allow(
                &req.api_key,
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
            .get_user_by_api_key(&req.api_key)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| {
                self.record_auth_failure();
                Status::unauthenticated("Invalid API key")
            })?;

        match self.db.create_library(&user_id, &req.name).await {
            Ok(lib_id) => {
                tracing::info!("Created library '{}' for user {}", req.name, user_id);
                Ok(Response::new(CreateLibraryResponse {
                    success: true,
                    library_id: lib_id,
                    message: format!("Library '{}' created successfully", req.name),
                }))
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("already exists") {
                    Err(Status::already_exists(msg))
                } else {
                    Err(Status::invalid_argument(msg))
                }
            }
        }
    }

    async fn list_libraries(
        &self,
        request: Request<ListLibrariesRequest>,
    ) -> Result<Response<ListLibrariesResponse>, Status> {
        self.record_request("list_libraries");
        self.record_library_op();
        let req = request.into_inner();

        let user_id = self
            .db
            .get_user_by_api_key(&req.api_key)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| {
                self.record_auth_failure();
                Status::unauthenticated("Invalid API key")
            })?;

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
            .map_err(|e| Status::internal(e.to_string()))?;

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
        self.record_request("delete_library");
        self.record_library_op();
        let req = request.into_inner();

        if !self
            .rate_limiter
            .allow(
                &req.api_key,
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
            .get_user_by_api_key(&req.api_key)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| {
                self.record_auth_failure();
                Status::unauthenticated("Invalid API key")
            })?;

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
            Err(e) => Err(Status::not_found(e.to_string())),
        }
    }

    async fn list_premade_libraries(
        &self,
        request: Request<ListPremadeLibrariesRequest>,
    ) -> Result<Response<ListPremadeLibrariesResponse>, Status> {
        self.record_request("list_premade_libraries");

        let req = request.into_inner();

        let user_id = self
            .db
            .get_user_by_api_key(&req.api_key)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| {
                self.record_auth_failure();
                Status::unauthenticated("Invalid API key")
            })?;

        if !self
            .rate_limiter
            .allow(
                &req.api_key,
                self.config.rate_limit_per_minute as usize,
                Duration::from_secs(60),
            )
            .await
        {
            self.record_rate_limit();
            return Err(Status::resource_exhausted("Rate limit exceeded"));
        }

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

        tracing::debug!("User {} listed premade libraries", user_id);

        Ok(Response::new(ListPremadeLibrariesResponse {
            libraries: proto_libraries,
        }))
    }

    async fn get_premade_library(
        &self,
        request: Request<GetPremadeLibraryRequest>,
    ) -> Result<Response<GetPremadeLibraryResponse>, Status> {
        self.record_request("get_premade_library");

        let req = request.into_inner();

        let user_id = self
            .db
            .get_user_by_api_key(&req.api_key)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| {
                self.record_auth_failure();
                Status::unauthenticated("Invalid API key")
            })?;

        if !self
            .rate_limiter
            .allow(
                &req.api_key,
                self.config.rate_limit_per_minute as usize,
                Duration::from_secs(60),
            )
            .await
        {
            self.record_rate_limit();
            return Err(Status::resource_exhausted("Rate limit exceeded"));
        }

        if req.filename.is_empty() {
            return Err(Status::invalid_argument("Filename is required"));
        }

        let sanitized: String = req
            .filename
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_target(false).init();

    tracing::info!("Starting snip-sync server v{}", env!("CARGO_PKG_VERSION"));
    tracing::warn!(
        "TLS is not enabled. For production, use a reverse proxy with TLS (nginx, traefik, etc.)"
    );

    Config::ensure_config_file();
    let config = Config::load();

    let db = Arc::new(Database::connect(&config.db_path).await?);
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

    let rate_limiter = Arc::new(RateLimiter::new());
    let cors_allowed_origins = config.cors_allowed_origins.clone();

    tracing::info!("Input validation config: max_command={}, max_description={}, max_tags={}, max_tag_length={}, request_timeout={}s",
        config.max_command_length, config.max_description_length, config.max_tags, config.max_tag_length, config.request_timeout_secs);

    if cors_allowed_origins.is_empty() {
        tracing::warn!("CORS: no origins configured, requests from any origin will be allowed");
    }

    let timeout = Duration::from_secs(config.request_timeout_secs);

    let metrics = Metrics::new().expect("Failed to create metrics");

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
    };

    let grpc_service = SnipSyncService {
        db: db.clone(),
        rate_limiter: rate_limiter.clone(),
        config,
        metrics,
        premade_manager,
    };

    let cors = if cors_allowed_origins.is_empty() {
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
        cors.allow_methods(Any).allow_headers(Any)
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
                decoded.len() == expected_bytes.len() && bool::from(decoded.ct_eq(expected_bytes))
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
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "status": "healthy"
                }))
            }),
        )
        .route("/metrics", axum::routing::get(metrics_handler))
        .layer(cors)
        .with_state(state);

    let grpc_handle = tokio::spawn(async move {
        let server = tonic::transport::Server::builder()
            .timeout(timeout)
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
        let listener = tokio::net::TcpListener::bind(http_addr)
            .await
            .expect("Failed to bind HTTP");
        tracing::info!("HTTP server listening on http://{}", http_addr);

        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
                tracing::info!("Shutdown signal received, stopping HTTP server...");
            })
            .await
            .expect("HTTP server failed");
    });

    let _ = tokio::join!(grpc_handle, http_handle);

    tracing::info!("Server shutdown complete");
    Ok(())
}
