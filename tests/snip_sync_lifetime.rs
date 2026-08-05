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

use std::io::{Read, Write};
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

fn find_snip_sync_binary() -> String {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_snip-sync") {
        return path;
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

/// Spawn the server on specific ports. Returns (child, http_addr).
fn start_server_on_ports(
    tmp: &tempfile::TempDir,
    grpc_port: u16,
    http_port: u16,
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
    let mut child = Command::new(exe)
        .arg("serve")
        .env("CONFIG_PATH", &config_path)
        .env("DATABASE_URL", data_dir.join("test.db"))
        .env("SNIP_SYNC_ALLOW_HTTP", "true")
        .env("SNIP_SYNC_STATE_DIR", state_dir.to_str().unwrap())
        .env("GRPC_PORT", grpc_port.to_string())
        .env("HTTP_PORT", http_port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn snip-sync serve");

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
            "server should exit normally (code 0), not via signal: {:?}",
            status.signal()
        );
    }

    drop(tmp);
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
