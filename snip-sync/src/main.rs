// Pre-existing `format!` patterns in this binary's metric handler. The
// lib already suppresses `clippy::uninlined_format_args` for the same
// reason; mirror that here so the binary compiles cleanly.
#![allow(clippy::uninlined_format_args)]

use axum::extract::State;
use axum::http::HeaderValue;
use base64::Engine;
use snip_proto::snippet_sync_server::SnippetSyncServer;
use snip_sync::{
    AppState, Config, Database, Metrics, PremadeManager, RateLimiter, SnipSyncService,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};

async fn security_headers_middleware(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    response
}

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
            // Pad to expected length so ct_eq always compares the same number of
            // bytes, preventing a timing side-channel that reveals the password length.
            let mut padded = decoded.clone();
            padded.resize(expected_bytes.len(), 0);
            bool::from(padded.ct_eq(expected_bytes)) && decoded.len() == expected_bytes.len()
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
    let (_persist_shutdown_tx, persist_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    rate_limiter.start_persistence_task(async {
        let _ = persist_shutdown_rx.await;
    });
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
        captured_auth_header: Arc::new(std::sync::Mutex::new(None)),
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
            .max_frame_size(grpc_max_message_size)
            .add_service(SnippetSyncServer::new(grpc_service))
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

    let shutdown_timeout = tokio::time::timeout(Duration::from_secs(30), async {
        let (grpc_result, http_result) = tokio::join!(grpc_handle, http_handle);
        (grpc_result, http_result)
    })
    .await;

    match shutdown_timeout {
        Ok((grpc_result, http_result)) => {
            if let Err(e) = grpc_result {
                tracing::error!("gRPC server task failed: {}", e);
            }
            if let Err(e) = http_result {
                tracing::error!("HTTP server task failed: {}", e);
            }
        }
        Err(_) => {
            tracing::warn!("Shutdown timed out after 30s, forcing exit");
        }
    }

    tracing::info!("Server shutdown complete");
    Ok(())
}
