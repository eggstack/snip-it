//! Server lifetime regression test.
//!
//! Launches `snip-sync serve` in an isolated environment and confirms
//! that a healthy server remains running beyond 30 seconds (the former
//! timeout boundary). The test is marked `#[ignore]` so routine CI
//! stays fast; invoke it explicitly with:
//!
//! ```text
//! cargo test --test snip_sync_lifetime -- --ignored --test-threads=1
//! ```

use std::io::{BufRead, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

fn check_health(addr: SocketAddr) -> bool {
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let request = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if stream.write_all(request).is_err() {
        return false;
    }
    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };
    if n == 0 {
        return false;
    }
    let resp = String::from_utf8_lossy(&buf[..n]);
    resp.starts_with("HTTP/1.1 200") || resp.starts_with("HTTP/1.0 200")
}

fn http_response(addr: SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    String::from_utf8(response).unwrap()
}

fn read_keep_alive_response(reader: &mut std::io::BufReader<TcpStream>) -> String {
    let mut headers = Vec::new();
    loop {
        let mut line = Vec::new();
        reader.read_until(b'\n', &mut line).unwrap();
        if line == b"\r\n" || line.is_empty() {
            break;
        }
        headers.extend_from_slice(&line);
    }
    let header_text = String::from_utf8(headers).unwrap();
    let content_length = header_text
        .lines()
        .find_map(|line| {
            line.strip_prefix("content-length: ")
                .and_then(|length| length.parse::<usize>().ok())
        })
        .unwrap_or(0);
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body).unwrap();
    format!("{header_text}\r\n{}", String::from_utf8_lossy(&body))
}

fn find_snip_sync_binary() -> String {
    if let Some(path) = option_env!("CARGO_BIN_EXE_snip-sync") {
        return path.to_owned();
    }
    let test_bin = std::env::current_exe().expect("current_exe should be set");
    let deps_dir = test_bin.parent().expect("test binary should have a parent");
    let target_debug = deps_dir
        .parent()
        .expect("deps should be under target/debug");
    let snip_sync_bin = target_debug.join("snip-sync");
    if snip_sync_bin.exists() {
        return snip_sync_bin.to_str().unwrap().to_string();
    }
    panic!("Cannot find snip-sync binary. Build it first with: cargo build -p snip-sync");
}

fn reserve_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    drop(listener);
    port
}

/// Wait for a child process to exit within a bounded deadline.
/// If the deadline is exceeded, kill the process and panic.
fn wait_for_exit(child: &mut std::process::Child, deadline: Duration) -> std::process::ExitStatus {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) => {
                if start.elapsed() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("server process did not exit within {deadline:?}");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => panic!("failed to wait on server process: {e}"),
        }
    }
}

fn stop_server(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    child.kill().expect("failed to stop server process");
}

/// Spawn the server on specific ports. Returns (child, http_addr).
fn start_server_on_ports(
    tmp: &tempfile::TempDir,
    grpc_port: u16,
    http_port: u16,
) -> (std::process::Child, SocketAddr) {
    start_server_on_ports_with_http_config(tmp, grpc_port, http_port, false, false)
}

fn start_server_on_ports_with_http_config(
    tmp: &tempfile::TempDir,
    grpc_port: u16,
    http_port: u16,
    expose_metrics: bool,
    allow_all_origins: bool,
) -> (std::process::Child, SocketAddr) {
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    let state_dir = tmp.path().join("state");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&state_dir).unwrap();

    let config_path = config_dir.join("config.toml");
    std::fs::write(&config_path, "").unwrap();

    let exe = find_snip_sync_binary();
    let mut command = Command::new(exe);
    command
        .arg("serve")
        .env("CONFIG_PATH", &config_path)
        .env("DATABASE_URL", data_dir.join("test.db"))
        .env("SNIP_SYNC_ALLOW_HTTP", "true")
        .env("SNIP_SYNC_STATE_DIR", state_dir.to_str().unwrap())
        .env("GRPC_HOST", "127.0.0.1")
        .env("HTTP_HOST", "127.0.0.1")
        .env("GRPC_PORT", grpc_port.to_string())
        .env("HTTP_PORT", http_port.to_string())
        .env(
            "CORS_ALLOWED_ORIGINS",
            if expose_metrics {
                "https://example.com"
            } else {
                ""
            },
        )
        .env(
            "CORS_ALLOW_ALL",
            if allow_all_origins { "true" } else { "false" },
        )
        .env_remove("METRICS_USERNAME")
        .env_remove("METRICS_PASSWORD")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if expose_metrics {
        command.env("METRICS_USERNAME", "user");
        command.env("METRICS_PASSWORD", "pass");
    }
    let mut child = command.spawn().expect("failed to spawn snip-sync serve");

    let _stderr = child.stderr.take().unwrap();

    let http_addr: SocketAddr = format!("127.0.0.1:{http_port}").parse().unwrap();
    let start = std::time::Instant::now();
    let deadline = Duration::from_secs(10);
    loop {
        if start.elapsed() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("server did not become healthy within {deadline:?}");
        }
        if check_health(http_addr) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    (child, http_addr)
}

/// Spawn the server on random ports. Returns (child, http_addr, grpc_port, http_port).
fn start_server(tmp: &tempfile::TempDir) -> (std::process::Child, SocketAddr, u16, u16) {
    let grpc_port = reserve_port();
    let http_port = reserve_port();
    let (child, http_addr) = start_server_on_ports(tmp, grpc_port, http_port);
    (child, http_addr, grpc_port, http_port)
}

#[test]
#[ignore = "runs for 35+ seconds; invoke explicitly with --ignored"]
fn server_remains_healthy_beyond_30_seconds() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut child, http_addr, _grpc, _http) = start_server(&tmp);

    assert!(
        check_health(http_addr),
        "server should be healthy immediately"
    );

    std::thread::sleep(Duration::from_secs(35));

    assert!(
        check_health(http_addr),
        "server should still be healthy after 35s"
    );

    stop_server(&mut child);

    let status = wait_for_exit(&mut child, Duration::from_secs(15));

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            status.code(),
            Some(0),
            "server should exit normally (code 0), not via signal: {:?}",
            status.signal()
        );
    }

    drop(tmp);
}

#[test]
fn http_health_head_metrics_disabled_and_fallback_contract() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut child, http_addr, _grpc, _http) = start_server(&tmp);

    let health = http_response(
        http_addr,
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    assert!(health.contains("\"status\":\"healthy\""), "{health}");
    assert!(
        health.contains("x-content-type-options: nosniff"),
        "{health}"
    );
    assert!(health.contains("x-frame-options: DENY"), "{health}");
    assert!(health.contains("cache-control: no-store"), "{health}");

    let head = http_response(
        http_addr,
        "HEAD /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(
        !head.contains("\"status\""),
        "HEAD must not include a body: {head}"
    );
    let content_length = |response: &str| {
        response
            .lines()
            .find_map(|line| line.strip_prefix("content-length: ").map(str::to_owned))
    };
    assert_eq!(content_length(&head), content_length(&health));

    let queried = http_response(
        http_addr,
        "GET /health?probe=1 HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    assert!(queried.starts_with("HTTP/1.1 200"), "{queried}");

    let unsupported = http_response(
        http_addr,
        "PUT /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    assert!(unsupported.starts_with("HTTP/1.1 405"), "{unsupported}");
    assert!(
        unsupported
            .lines()
            .find(|line| line.starts_with("allow:"))
            .is_some_and(|line| line.replace(' ', "") == "allow:GET,HEAD"),
        "{unsupported}"
    );

    let body = http_response(
        http_addr,
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 3\r\nConnection: close\r\n\r\nx=y",
    );
    assert!(
        !body.starts_with("HTTP/1.1 200"),
        "body-bearing request was not rejected: {body}"
    );
    assert!(
        !body.contains("\"status\":\"healthy\""),
        "body-bearing request reached handler: {body}"
    );

    let metrics = http_response(
        http_addr,
        "GET /metrics HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    assert!(metrics.starts_with("HTTP/1.1 404"), "{metrics}");

    let missing = http_response(
        http_addr,
        "GET /missing HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");
    assert!(
        missing.contains("x-content-type-options: nosniff"),
        "{missing}"
    );
    assert!(missing.contains("x-frame-options: DENY"), "{missing}");
    assert!(missing.contains("cache-control: no-store"), "{missing}");

    let stream = TcpStream::connect_timeout(&http_addr, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut reader = std::io::BufReader::new(stream);
    reader
        .get_mut()
        .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .unwrap();
    assert!(read_keep_alive_response(&mut reader).starts_with("HTTP/1.1 200"));
    std::thread::sleep(Duration::from_secs(2));
    reader
        .get_mut()
        .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    assert!(read_keep_alive_response(&mut reader).starts_with("HTTP/1.1 200"));

    stop_server(&mut child);
    let status = wait_for_exit(&mut child, Duration::from_secs(15));
    assert!(status.success(), "server should exit normally: {status:?}");
}

#[test]
fn http_metrics_auth_and_cors_preflight_contract() {
    let tmp = tempfile::tempdir().unwrap();
    let grpc_port = reserve_port();
    let http_port = reserve_port();
    let (mut child, http_addr) =
        start_server_on_ports_with_http_config(&tmp, grpc_port, http_port, true, false);

    let unauthorized = http_response(
        http_addr,
        "GET /metrics HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );
    assert!(unauthorized.starts_with("HTTP/1.1 401"), "{unauthorized}");
    let invalid = http_response(
        http_addr,
        "GET /metrics HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Basic !!!\r\nConnection: close\r\n\r\n",
    );
    assert!(invalid.starts_with("HTTP/1.1 401"), "{invalid}");
    let authorized = http_response(
        http_addr,
        "GET /metrics HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Basic dXNlcjpwYXNz\r\nConnection: close\r\n\r\n",
    );
    assert!(authorized.starts_with("HTTP/1.1 200"), "{authorized}");
    assert!(
        authorized.contains("snip_sync_auth_failures_total"),
        "{authorized}"
    );
    assert!(
        authorized.contains("x-content-type-options: nosniff"),
        "{authorized}"
    );
    let head_metrics = http_response(
        http_addr,
        "HEAD /metrics HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Basic dXNlcjpwYXNz\r\nConnection: close\r\n\r\n",
    );
    assert!(head_metrics.starts_with("HTTP/1.1 200"), "{head_metrics}");
    assert!(
        !head_metrics.contains("snip_sync_auth_failures_total"),
        "HEAD must suppress metrics bytes: {head_metrics}"
    );
    let content_length = |response: &str| {
        response
            .lines()
            .find_map(|line| line.strip_prefix("content-length: ").map(str::to_owned))
    };
    assert_eq!(content_length(&head_metrics), content_length(&authorized));

    let preflight = http_response(
        http_addr,
        "OPTIONS /health HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://example.com\r\nAccess-Control-Request-Method: GET\r\nAccess-Control-Request-Headers: content-type,authorization\r\nConnection: close\r\n\r\n",
    );
    assert!(preflight.starts_with("HTTP/1.1 200"), "{preflight}");
    assert!(
        preflight.contains("access-control-allow-origin: https://example.com"),
        "{preflight}"
    );
    assert!(
        preflight.contains("access-control-allow-methods: GET"),
        "{preflight}"
    );
    assert!(
        preflight.contains("access-control-allow-headers: content-type,authorization"),
        "{preflight}"
    );

    stop_server(&mut child);
    let status = wait_for_exit(&mut child, Duration::from_secs(15));
    assert!(status.success(), "server should exit normally: {status:?}");
}

#[test]
fn http_allow_all_cors_is_available_for_loopback_server() {
    let tmp = tempfile::tempdir().unwrap();
    let grpc_port = reserve_port();
    let http_port = reserve_port();
    let (mut child, http_addr) =
        start_server_on_ports_with_http_config(&tmp, grpc_port, http_port, false, true);

    let response = http_response(
        http_addr,
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://arbitrary.example\r\nConnection: close\r\n\r\n",
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(
        response.contains("access-control-allow-origin: *"),
        "{response}"
    );

    let preflight = http_response(
        http_addr,
        "OPTIONS /health HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://arbitrary.example\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Headers: x-custom\r\nConnection: close\r\n\r\n",
    );
    assert!(preflight.starts_with("HTTP/1.1 200"), "{preflight}");
    assert!(
        preflight.contains("access-control-allow-origin: *"),
        "{preflight}"
    );
    assert!(
        preflight.contains("access-control-allow-methods: *"),
        "{preflight}"
    );
    assert!(
        preflight.contains("access-control-allow-headers: *"),
        "{preflight}"
    );

    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let status = wait_for_exit(&mut child, Duration::from_secs(15));
    assert!(status.success(), "server should exit normally: {status:?}");
}

#[test]
#[ignore = "runs for 10+ seconds; invoke explicitly with --ignored"]
fn server_exits_cleanly_on_signal() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut child, http_addr, grpc_port, http_port) = start_server(&tmp);

    std::thread::sleep(Duration::from_secs(2));

    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }

    let status = wait_for_exit(&mut child, Duration::from_secs(15));

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            status.code(),
            Some(0),
            "server should exit normally (code 0) after SIGTERM, not via signal: {:?}",
            status.signal()
        );
    }

    // Wait briefly for PID file removal and lock release.
    std::thread::sleep(Duration::from_secs(1));

    // Start replacement on the SAME ports with the SAME state dir.
    let (mut replacement, _repl_addr) = start_server_on_ports(&tmp, grpc_port, http_port);
    assert!(
        check_health(http_addr),
        "replacement server should be healthy after singleton lock release"
    );

    #[cfg(unix)]
    unsafe {
        libc::kill(replacement.id() as i32, libc::SIGTERM);
    }
    let _ = wait_for_exit(&mut replacement, Duration::from_secs(15));

    drop(tmp);
}
