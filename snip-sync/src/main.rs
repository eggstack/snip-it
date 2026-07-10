#![allow(clippy::uninlined_format_args)]

use clap::Parser;
use snip_sync::cli::{Cli, Command};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        None | Some(Command::Serve) => serve()?,
        Some(Command::Init {
            force_cert,
            skip_cert,
        }) => cmd_init(force_cert, skip_cert)?,
        Some(Command::Cert { force, out_dir }) => {
            snip_sync::cert::generate_dev_certs(force, out_dir)?
        }
        Some(Command::Edit) => cmd_edit()?,
        Some(Command::Stop { force }) => cmd_stop(force)?,
        Some(Command::Restart { force }) => cmd_restart(force)?,
        Some(Command::Update { dry_run, locked }) => cmd_update(dry_run, locked)?,
        Some(Command::Croncheck { verbose }) => cmd_croncheck(verbose)?,
        Some(Command::Paths { json }) => cmd_paths(json)?,
        Some(Command::Completions { shell }) => cmd_completions(shell),
        Some(Command::Version) => println!("snip-sync {}", env!("CARGO_PKG_VERSION")),
    }

    Ok(())
}

fn serve() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_target(false).init();

    let tls_enabled = std::env::var("TLS_ENABLED")
        .ok()
        .is_some_and(|v| v == "true" || v == "1");

    tracing::info!("Starting snip-sync server v{}", env!("CARGO_PKG_VERSION"));

    if tls_enabled {
        tracing::warn!(
            "TLS_ENABLED acknowledges TLS termination by an upstream reverse proxy; snip-sync itself still serves plaintext gRPC and HTTP."
        );
    } else {
        if std::env::var_os("SNIP_SYNC_ALLOW_HTTP").is_some_and(|v| v == "true") {
            tracing::warn!(
                "Serving plaintext gRPC and HTTP for local development. For production, put a \
                 TLS-terminating reverse proxy in front of snip-sync."
            );
        } else {
            tracing::error!(
                "snip-sync does not terminate TLS. Put a TLS-terminating reverse proxy in front \
                 of it and set TLS_ENABLED=true, or set SNIP_SYNC_ALLOW_HTTP=true for local development."
            );
            return Err(
                "TLS termination is required for production. Set TLS_ENABLED=true when a reverse proxy terminates TLS, or set SNIP_SYNC_ALLOW_HTTP=true for local development"
                    .into(),
            );
        }
    }

    snip_sync::bootstrap::ensure_layout()?;
    snip_sync::bootstrap::ensure_config_file()?;
    let config = snip_sync::Config::load();

    // Check for stale PID file before starting
    if let Some(old_pid) = snip_sync::process::read_pid() {
        if snip_sync::process::is_running(old_pid) {
            return Err(format!(
                "Server already running with PID {}. Use 'snip-sync stop' first.",
                old_pid
            )
            .into());
        }
        tracing::warn!("Found stale PID file for process {}. Removing.", old_pid);
        snip_sync::process::remove_pid();
    }

    snip_sync::process::write_pid().map_err(|e| format!("Failed to write PID file: {}", e))?;

    let rt = tokio::runtime::Runtime::new()?;
    let result = rt.block_on(serve_inner(config));

    // Clean up PID file on shutdown
    snip_sync::process::remove_pid();

    result
}

async fn serve_inner(config: snip_sync::Config) -> Result<(), Box<dyn std::error::Error>> {
    use axum::extract::State;
    use axum::http::HeaderValue;
    use base64::Engine;
    use snip_proto::snippet_sync_server::SnippetSyncServer;
    use snip_sync::{AppState, Database, Metrics, PremadeManager, RateLimiter, SnipSyncService};
    use std::sync::Arc;
    use std::time::Duration;
    use tower_http::cors::{Any, CorsLayer};

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

    let grpc_addr = resolve_socket_addr(&config.grpc_host, config.grpc_port)?;
    let http_addr = resolve_socket_addr(&config.http_host, config.http_port)?;

    // Bind both listeners before spawning either service. This makes port and
    // address errors fail the command immediately instead of leaving a
    // half-started server running with a misleading successful exit.
    let grpc_listener = tokio::net::TcpListener::bind(grpc_addr).await?;
    let http_listener = tokio::net::TcpListener::bind(http_addr).await?;

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
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(grpc_listener);
        let server = tonic::transport::Server::builder()
            .timeout(timeout)
            .max_frame_size(grpc_max_message_size)
            .add_service(SnippetSyncServer::new(grpc_service))
            .serve_with_incoming(incoming);

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
        tracing::info!("HTTP server listening on http://{}", http_addr);

        if let Err(e) = axum::serve(http_listener, app)
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

fn cmd_init(force_cert: bool, skip_cert: bool) -> Result<(), Box<dyn std::error::Error>> {
    snip_sync::bootstrap::ensure_layout()?;
    snip_sync::bootstrap::ensure_config_file()?;
    if !skip_cert {
        snip_sync::cert::generate_dev_certs(force_cert, None)?;
    }
    println!("Initialization complete.");
    Ok(())
}

fn cmd_edit() -> Result<(), Box<dyn std::error::Error>> {
    snip_sync::bootstrap::ensure_layout()?;
    snip_sync::bootstrap::ensure_config_file()?;
    let config_path = snip_sync::paths::config_path();
    snip_sync::editor::open_in_editor(&config_path)?;
    Ok(())
}

fn cmd_stop(force: bool) -> Result<(), Box<dyn std::error::Error>> {
    let pid = snip_sync::process::read_pid().ok_or("No PID file found. Is the server running?")?;

    if !snip_sync::process::is_running(pid) {
        println!(
            "Process {} is not running. Cleaning up stale PID file.",
            pid
        );
        snip_sync::process::remove_pid();
        return Ok(());
    }

    if !force && !snip_sync::process::validate_process_name(pid) {
        eprintln!(
            "Warning: PID {} does not appear to be a snip-sync process.",
            pid
        );
        eprintln!("Use --force to stop it anyway.");
        return Err("Refusing to stop non-snip-sync process".into());
    }

    #[cfg(not(unix))]
    {
        eprintln!("Stop is only supported on Unix systems.");
        return Err("Unsupported platform".into());
    }

    #[cfg(unix)]
    {
        println!("Sending SIGTERM to process {}...", pid);
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };

        match snip_sync::process::wait_for_exit(pid, std::time::Duration::from_secs(10)) {
            Ok(()) => {
                snip_sync::process::remove_pid();
                println!("Server stopped.");
            }
            Err(e) => {
                eprintln!("Warning: {}", e);
                if force {
                    println!("Sending SIGKILL...");
                    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
                    snip_sync::process::remove_pid();
                    println!("Server killed.");
                } else {
                    return Err(e.into());
                }
            }
        }
    }

    Ok(())
}

fn cmd_restart(force: bool) -> Result<(), Box<dyn std::error::Error>> {
    match snip_sync::process::read_pid() {
        Some(pid) if snip_sync::process::is_running(pid) => {
            println!("Stopping existing server (PID {})...", pid);
            cmd_stop(force)?;
        }
        _ => {
            println!("No running server found.");
        }
    }
    println!("Starting server...");
    serve()
}

fn cmd_update(dry_run: bool, locked: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Check that cargo is available on PATH
    let cargo_status = std::process::Command::new("cargo")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match cargo_status {
        Ok(s) if s.success() => {}
        _ => {
            return Err(
                "cargo is not available on PATH. Install Rust via https://rustup.rs first.".into(),
            );
        }
    }

    let mut args = vec!["install", "snip-sync"];
    if dry_run {
        args.push("--dry-run");
    }
    if locked {
        args.push("--locked");
    }

    println!("Running: cargo {}", args.join(" "));
    if dry_run {
        println!("(dry run — no changes will be made)");
    }

    let status = std::process::Command::new("cargo")
        .args(&args)
        .status()
        .map_err(|e| format!("Failed to run cargo: {}", e))?;

    if status.success() {
        println!("Update complete.");
    } else {
        return Err(format!("cargo install failed with status: {}", status).into());
    }
    Ok(())
}

fn resolve_socket_addr(host: &str, port: u16) -> Result<SocketAddr, Box<dyn std::error::Error>> {
    let host = host.trim();
    let address = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    address
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| format!("Could not resolve {address}").into())
}

fn check_health(http_host: &str, http_port: u16) -> bool {
    let address = match resolve_socket_addr(http_host, http_port) {
        Ok(address) => address,
        Err(_) => return false,
    };
    let mut stream = match TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let request = format!("GET /health HTTP/1.1\r\nHost: {http_host}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = [0u8; 4096];
    let bytes_read = match stream.read(&mut response) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    if bytes_read == 0 {
        return false;
    }
    let response = String::from_utf8_lossy(&response[..bytes_read]);
    response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200")
}

fn cmd_croncheck(verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    snip_sync::bootstrap::ensure_layout()?;
    snip_sync::bootstrap::ensure_config_file()?;
    let lock_path = snip_sync::paths::state_dir().join("croncheck.lock");
    let _lock = match snip_sync::process::try_lock(&lock_path)? {
        Some(lock) => lock,
        None => {
            if verbose {
                println!("Another croncheck is already running; skipping this check.");
            }
            return Ok(());
        }
    };

    let config = snip_sync::Config::load();

    if check_health(&config.http_host, config.http_port) {
        if verbose {
            println!(
                "Server is healthy on {}:{}",
                config.http_host, config.http_port
            );
        } else {
            println!("ok");
        }
        return Ok(());
    }

    if verbose {
        println!(
            "Server is unhealthy or not running on {}:{}.",
            config.http_host, config.http_port
        );
        println!("Starting detached server...");
    }

    let child = std::process::Command::new(std::env::current_exe()?)
        .arg("serve")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn server: {}", e))?;

    if verbose {
        println!("Spawned server process (PID {}).", child.id());
    }

    // Wait briefly for the server to come up. A failed health check is an
    // error: cron callers must be able to alert instead of receiving a false
    // success when `serve` exits during startup.
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if check_health(&config.http_host, config.http_port) {
            if verbose {
                println!("Server started successfully.");
            }
            println!("ok");
            return Ok(());
        }
    }

    Err("Server did not become healthy within 5 seconds; inspect the service logs".into())
}

fn cmd_paths(json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let paths = snip_sync::paths::Paths::resolve();
    if json {
        let map = serde_json::json!({
            "config_dir": paths.config_dir,
            "config_path": paths.config_path,
            "data_dir": paths.data_dir,
            "state_dir": paths.state_dir,
            "cert_dir": paths.cert_dir,
            "pid_path": paths.pid_path,
            "db_path": paths.db_path,
            "premade_dir": paths.premade_dir,
        });
        println!("{}", serde_json::to_string_pretty(&map)?);
    } else {
        paths.print();
    }
    Ok(())
}

fn cmd_completions(shell: clap_complete::Shell) {
    let mut cmd = <Cli as clap::CommandFactory>::command();
    let bin_name = "snip-sync".to_string();
    clap_complete::generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
}
