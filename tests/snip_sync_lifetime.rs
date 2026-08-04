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

use std::io::{BufRead, BufReader, Read, Write};
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
fn start_server(tmp: &tempfile::TempDir) -> (std::process::Child, SocketAddr) {
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    let state_dir = tmp.path().join("state");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&state_dir).unwrap();

    let config_path = config_dir.join("config.toml");
    std::fs::write(&config_path, "").unwrap();

    let exe = std::env::var("CARGO_BIN_EXE_snip-sync")
        .expect("CARGO_BIN_EXE_snip-sync not set; run via cargo test");
    let mut child = Command::new(exe)
        .arg("serve")
        .env("CONFIG_PATH", &config_path)
        .env("DATABASE_URL", data_dir.join("test.db"))
        .env("SNIP_SYNC_ALLOW_HTTP", "true")
        .env("GRPC_PORT", "0")
        .env("HTTP_PORT", "0")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn snip-sync serve");

    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);

    let mut http_addr: Option<SocketAddr> = None;
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.contains("HTTP server listening on")
            && let Some(addr_str) = line.split("HTTP server listening on ").nth(1)
            && let Ok(addr) = addr_str.trim().parse::<SocketAddr>()
        {
            http_addr = Some(addr);
        }
        // Once we have the HTTP address, we can stop reading.
        if http_addr.is_some() {
            break;
        }
    }

    let http_addr = http_addr.expect("could not parse HTTP listen address from stderr");
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
