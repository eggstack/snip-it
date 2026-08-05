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

use std::io::{BufReader, Read, Write};
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

/// Spawn the server and return (child, http_addr). Reads stderr until
/// both listen lines appear, then parses the HTTP address.
fn find_snip_sync_binary() -> String {
    // CARGO_BIN_EXE_snip-sync is set when snip-sync is a direct binary
    // dependency. For workspace binaries, we locate it through the
    // target directory relative to the test binary.
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_snip-sync") {
        return path;
    }
    // Fall back: find the binary relative to the test binary's location.
    let test_bin = std::env::current_exe().expect("current_exe should be set");
    let deps_dir = test_bin.parent().expect("test binary should have a parent");
    // The snip-sync binary is in target/debug/ (not deps/).
    let target_debug = deps_dir
        .parent()
        .expect("deps should be under target/debug");
    let snip_sync_bin = target_debug.join("snip-sync");
    if snip_sync_bin.exists() {
        return snip_sync_bin.to_str().unwrap().to_string();
    }
    panic!("Cannot find snip-sync binary. Build it first with: cargo build -p snip-sync");
}

/// Reserve a random available port on localhost, then release it.
fn reserve_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    // Release the port immediately so the server can bind to it.
    drop(listener);
    port
}

/// Spawn the server and return (child, http_addr). Reads stderr until
/// both listen lines appear, then parses the HTTP address.
fn start_server(tmp: &tempfile::TempDir) -> (std::process::Child, SocketAddr) {
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    let state_dir = tmp.path().join("state");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&state_dir).unwrap();

    let config_path = config_dir.join("config.toml");
    std::fs::write(&config_path, "").unwrap();

    let grpc_port = reserve_port();
    let http_port = reserve_port();

    let exe = find_snip_sync_binary();
    let mut child = Command::new(exe)
        .arg("serve")
        .env("CONFIG_PATH", &config_path)
        .env("DATABASE_URL", data_dir.join("test.db"))
        .env("SNIP_SYNC_ALLOW_HTTP", "true")
        .env("GRPC_PORT", grpc_port.to_string())
        .env("HTTP_PORT", http_port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn snip-sync serve");

    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);

    // Poll health endpoint with bounded startup deadline.
    let http_addr: SocketAddr = format!("127.0.0.1:{http_port}").parse().unwrap();
    let start = std::time::Instant::now();
    let deadline = Duration::from_secs(10);
    loop {
        if start.elapsed() > deadline {
            panic!("server did not become healthy within {deadline:?}");
        }
        if check_health(http_addr) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // Drain remaining stderr lines so the pipe doesn't block.
    // We don't parse logs for readiness; health polling is authoritative.
    drop(reader);

    (child, http_addr)
}

#[test]
#[ignore = "runs for 35+ seconds; invoke explicitly with --ignored"]
fn server_remains_healthy_beyond_30_seconds() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut child, http_addr) = start_server(&tmp);

    // Verify the server is healthy immediately.
    assert!(
        check_health(http_addr),
        "server should be healthy immediately after start"
    );

    // Wait beyond the old 30-second timeout boundary.
    std::thread::sleep(Duration::from_secs(35));

    // Verify the server is still healthy.
    assert!(
        check_health(http_addr),
        "server should still be healthy after 35 seconds"
    );

    // Send SIGTERM for a clean shutdown.
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }

    // Wait for exit using a thread with timeout.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = child.wait();
        let _ = tx.send(());
    });

    #[cfg(unix)]
    {
        let exited = rx.recv_timeout(Duration::from_secs(15)).is_ok();
        assert!(
            exited,
            "server should exit within 15s of SIGTERM after healthy run"
        );
    }

    #[cfg(not(unix))]
    {
        let _ = rx.recv_timeout(Duration::from_secs(15));
    }

    drop(tmp);
}

#[test]
#[ignore = "runs for 5+ seconds; invoke explicitly with --ignored"]
fn server_exits_cleanly_on_signal() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut child, _http_addr) = start_server(&tmp);

    // Wait briefly for startup.
    std::thread::sleep(Duration::from_secs(2));

    // Send SIGTERM.
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }

    // Wait for exit with timeout.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = child.wait();
        let _ = tx.send(());
    });

    #[cfg(unix)]
    {
        let exited = rx.recv_timeout(Duration::from_secs(15)).is_ok();
        assert!(exited, "server should exit within 15s of SIGTERM");
    }

    #[cfg(not(unix))]
    {
        let _ = rx.recv_timeout(Duration::from_secs(15));
    }

    drop(tmp);
}
