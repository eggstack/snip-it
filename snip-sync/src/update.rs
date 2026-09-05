//! Binary-first self-update support for `snip-sync`.
//!
//! crates.io selects the stable version, while the exact component GitHub tag
//! supplies the binary and checksum. The running server is updated only after
//! the candidate has passed integrity and identity checks.

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CRATE_NAME: &str = "snip-sync";
const BINARY_NAME: &str = "snip-sync";
const CRATES_API_URL: &str = "https://crates.io/api/v1/crates/{crate}";
const RELEASE_BASE_URL: &str = "https://github.com/eggstack/snip-it/releases/download";
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct CratesResponse {
    #[serde(rename = "crate")]
    crate_info: CrateInfo,
}

#[derive(Debug, Deserialize)]
struct CrateInfo {
    max_version: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostTarget {
    Prebuilt(&'static str),
    SourceOnly(&'static str),
}

#[derive(Debug)]
enum FetchError {
    NotFound,
    Failed(String),
}

pub fn run(dry_run: bool, _locked: bool) -> Result<(), String> {
    let executable = current_executable()?;
    let current = current_version()?;
    let lifecycle = snip_sync::startup::update_lifecycle()?;

    println!("Checking for snip-sync updates...");
    let latest = latest_crates_version()?;
    if latest <= current {
        println!("snip-sync {current} is already up to date.");
        return Ok(());
    }

    println!("Update available: snip-sync {current} -> {latest}");
    if dry_run {
        print_dry_run(&latest, lifecycle);
        return Ok(());
    }

    ensure_destination_writable(&executable)?;
    let workdir = tempfile::tempdir()
        .map_err(|e| format!("could not create update staging directory: {e}"))?;
    let target = host_target(std::env::consts::OS, std::env::consts::ARCH);
    let (candidate, source) = match target {
        Some(HostTarget::Prebuilt(target)) => {
            match download_candidate(&latest, target, workdir.path())? {
                DownloadedCandidate::Ready(path) => (path, "GitHub release binary"),
                DownloadedCandidate::MissingAsset => {
                    println!("No prebuilt asset is published for {target}; using Cargo fallback.");
                    (cargo_candidate(&latest, workdir.path())?, "Cargo fallback")
                }
            }
        }
        Some(HostTarget::SourceOnly(target)) => {
            println!("Target {target} is source-only; using Cargo fallback.");
            (cargo_candidate(&latest, workdir.path())?, "Cargo fallback")
        }
        None => {
            println!("This host has no supported prebuilt target; using Cargo fallback.");
            (cargo_candidate(&latest, workdir.path())?, "Cargo fallback")
        }
    };
    validate_candidate(&candidate, &latest)?;

    #[cfg(windows)]
    if matches!(
        lifecycle,
        snip_sync::startup::UpdateLifecycle::ManagedRunning(_)
            | snip_sync::startup::UpdateLifecycle::DirectRunning
    ) {
        stop_running_server(&executable)?;
    }

    replace_installed_executable(&candidate, &executable, workdir.path(), lifecycle)?;

    #[cfg(not(windows))]
    if let Err(error) = snip_sync::startup::restart_after_update(&executable, lifecycle, false) {
        return Err(format_partial_restart_error(&error));
    }

    println!("Updated snip-sync {current} -> {latest} (source: {source}).");
    Ok(())
}

fn current_version() -> Result<Version, String> {
    Version::parse(env!("CARGO_PKG_VERSION")).map_err(|e| format!("invalid current version: {e}"))
}

fn print_dry_run(latest: &Version, lifecycle: snip_sync::startup::UpdateLifecycle) {
    match host_target(std::env::consts::OS, std::env::consts::ARCH) {
        Some(HostTarget::Prebuilt(target)) => println!(
            "Dry run: would try snip-sync-{target} at exact tag {}.",
            component_tag(latest)
        ),
        Some(HostTarget::SourceOnly(target)) => println!(
            "Dry run: target {target} is source-only; would run exact-version Cargo fallback."
        ),
        None => {
            println!("Dry run: host target is unsupported; would run exact-version Cargo fallback.")
        }
    }
    println!(
        "Dry run: server lifecycle state is {lifecycle:?}; no files, processes, or services were changed."
    );
}

fn current_executable() -> Result<PathBuf, String> {
    let path = std::env::current_exe()
        .map_err(|e| format!("could not locate the running executable: {e}"))?;
    fs::canonicalize(&path).map_err(|e| {
        format!(
            "could not resolve the running executable {}: {e}",
            path.display()
        )
    })
}

fn update_endpoint(name: &str, default: &str) -> String {
    #[cfg(feature = "test-helpers")]
    {
        std::env::var(name).unwrap_or_else(|_| default.to_owned())
    }
    #[cfg(not(feature = "test-helpers"))]
    {
        let _ = name;
        default.to_owned()
    }
}

fn validate_https_url(url: &str) -> Result<(), FetchError> {
    #[cfg(feature = "test-helpers")]
    if url.starts_with("http://") {
        return Ok(());
    }
    if !url.starts_with("https://") {
        return Err(FetchError::Failed(format!(
            "insecure or unsupported URL scheme rejected: {url}"
        )));
    }
    Ok(())
}

fn curl_protocol() -> &'static str {
    #[cfg(feature = "test-helpers")]
    {
        "=http,https"
    }
    #[cfg(not(feature = "test-helpers"))]
    {
        "=https"
    }
}

fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    validate_https_url(url)?;
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            curl_protocol(),
            "--tlsv1.2",
            "--connect-timeout",
            "10",
            "--max-time",
            "60",
            "--max-filesize",
            &MAX_METADATA_BYTES.to_string(),
            "--user-agent",
            "snip-it-update",
            "--write-out",
            "\n%{http_code}",
            url,
        ])
        .output()
        .map_err(|e| FetchError::Failed(format!("could not run curl: {e}")))?;
    if output.stdout.len() < 4 {
        return Err(FetchError::Failed(format!(
            "curl returned no HTTP status ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let status_start = output.stdout.len() - 3;
    let status = String::from_utf8_lossy(&output.stdout[status_start..])
        .parse::<u16>()
        .map_err(|_| FetchError::Failed("curl returned an invalid HTTP status".into()))?;
    let body = &output.stdout[..status_start - 1];
    match status {
        200..=299 => Ok(body.to_vec()),
        404 => Err(FetchError::NotFound),
        _ => Err(FetchError::Failed(format!(
            "HTTP {status}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

fn fetch_file(url: &str, path: &Path) -> Result<(), FetchError> {
    validate_https_url(url)?;
    let output_path = path
        .to_str()
        .ok_or_else(|| FetchError::Failed("staging path is not UTF-8".into()))?;
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            curl_protocol(),
            "--tlsv1.2",
            "--connect-timeout",
            "10",
            "--max-time",
            "60",
            "--max-filesize",
            &MAX_BINARY_BYTES.to_string(),
            "--user-agent",
            "snip-it-update",
            "--output",
            output_path,
            "--write-out",
            "%{http_code}",
            url,
        ])
        .output()
        .map_err(|e| FetchError::Failed(format!("could not run curl: {e}")))?;
    let status = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u16>()
        .map_err(|_| FetchError::Failed("curl returned an invalid HTTP status".into()))?;
    match status {
        200..=299 => Ok(()),
        404 => Err(FetchError::NotFound),
        _ => Err(FetchError::Failed(format!(
            "HTTP {status}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

fn fetch_error_message(error: FetchError) -> String {
    match error {
        FetchError::NotFound => "requested update metadata was not found".into(),
        FetchError::Failed(message) => message,
    }
}

fn latest_crates_version() -> Result<Version, String> {
    let template = update_endpoint("SNIP_UPDATE_CRATES_API_URL", CRATES_API_URL);
    let url = template.replace("{crate}", CRATE_NAME);
    let body = fetch_bytes(&url).map_err(fetch_error_message)?;
    let response: CratesResponse = serde_json::from_slice(&body)
        .map_err(|e| format!("could not parse crates.io response: {e}"))?;
    let version = Version::parse(&response.crate_info.max_version).map_err(|e| {
        format!(
            "crates.io returned invalid version {:?}: {e}",
            response.crate_info.max_version
        )
    })?;
    if !version.pre.is_empty() {
        return Err(format!(
            "crates.io returned prerelease version {}; a stable release is required",
            version
        ));
    }
    Ok(version)
}

pub(crate) fn host_target(os: &str, arch: &str) -> Option<HostTarget> {
    match (os, arch) {
        ("linux", "x86_64") => Some(HostTarget::Prebuilt("x86_64-unknown-linux-gnu")),
        ("linux", "aarch64") => Some(HostTarget::Prebuilt("aarch64-unknown-linux-gnu")),
        ("linux", "arm") => Some(HostTarget::SourceOnly("armv7-unknown-linux-gnueabihf")),
        ("macos", "x86_64") => Some(HostTarget::Prebuilt("x86_64-apple-darwin")),
        ("macos", "aarch64") => Some(HostTarget::Prebuilt("aarch64-apple-darwin")),
        ("windows", "x86_64") => Some(HostTarget::Prebuilt("x86_64-pc-windows-msvc")),
        ("windows", "aarch64") => Some(HostTarget::SourceOnly("aarch64-pc-windows-msvc")),
        _ => None,
    }
}

fn component_tag(version: &Version) -> String {
    format!("snip-sync-v{version}")
}

fn asset_name(target: &str) -> String {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    format!("{BINARY_NAME}-{target}{suffix}")
}

enum DownloadedCandidate {
    Ready(PathBuf),
    MissingAsset,
}

fn download_candidate(
    version: &Version,
    target: &str,
    staging: &Path,
) -> Result<DownloadedCandidate, String> {
    let asset = asset_name(target);
    let base = update_endpoint("SNIP_UPDATE_RELEASE_BASE_URL", RELEASE_BASE_URL)
        .trim_end_matches('/')
        .to_owned();
    let binary_url = format!("{base}/{}/{asset}", component_tag(version));
    let candidate = staging.join(&asset);
    match fetch_file(&binary_url, &candidate) {
        Err(FetchError::NotFound) => return Ok(DownloadedCandidate::MissingAsset),
        Err(error) => {
            return Err(format!(
                "could not download {asset}: {}",
                fetch_error_message(error)
            ));
        }
        Ok(()) => {}
    }
    let checksum_url = format!("{binary_url}.sha256");
    let checksum = fetch_bytes(&checksum_url).map_err(|error| {
        format!(
            "could not download checksum for {asset}: {}",
            fetch_error_message(error)
        )
    })?;
    verify_checksum(&candidate, &checksum, &asset)?;
    make_executable(&candidate)?;
    Ok(DownloadedCandidate::Ready(candidate))
}

fn verify_checksum(path: &Path, sidecar: &[u8], expected_name: &str) -> Result<(), String> {
    let text = std::str::from_utf8(sidecar)
        .map_err(|_| "checksum sidecar is not valid UTF-8".to_string())?;
    let mut lines = text.lines();
    let line = lines
        .next()
        .ok_or_else(|| "checksum sidecar is empty".to_string())?;
    if lines.next().is_some() {
        return Err("checksum sidecar must contain exactly one line".into());
    }
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() != 2
        || fields[1] != expected_name
        || fields[0].len() != 64
        || !fields[0].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!(
            "checksum sidecar has invalid format; expected '<64-hex-digest>  {expected_name}'"
        ));
    }
    let mut file =
        File::open(path).map_err(|e| format!("could not open downloaded candidate: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|e| format!("could not hash downloaded candidate: {e}"))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let actual = digest_hex(hasher.finalize().as_ref());
    if !actual.eq_ignore_ascii_case(fields[0]) {
        return Err(format!(
            "SHA-256 mismatch for {expected_name}: expected {}, got {actual}",
            fields[0]
        ));
    }
    Ok(())
}

fn digest_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn cargo_candidate(version: &Version, staging: &Path) -> Result<PathBuf, String> {
    let root = staging.join("cargo-root");
    let version_arg = format!("={version}");
    let root_arg = root
        .to_str()
        .ok_or_else(|| "Cargo staging path is not valid UTF-8".to_string())?;
    println!(
        "Running: cargo install {CRATE_NAME} --version {version_arg} --locked --root {root_arg}"
    );
    let status = Command::new("cargo")
        .args(["install", CRATE_NAME, "--version", &version_arg, "--locked", "--root", root_arg])
        .status()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound {
            format!("Cargo is not installed; update manually with `cargo install {CRATE_NAME} --version '={version}' --locked`")
        } else { format!("could not run cargo: {e}") })?;
    if !status.success() {
        return Err(format!("cargo exited with status {status}"));
    }
    let binary = if cfg!(windows) {
        root.join("bin").join("snip-sync.exe")
    } else {
        root.join("bin").join(BINARY_NAME)
    };
    if !binary.is_file() {
        return Err(format!(
            "Cargo completed but did not produce {}",
            binary.display()
        ));
    }
    Ok(binary)
}

fn validate_candidate(path: &Path, version: &Version) -> Result<(), String> {
    let expected = format!("{BINARY_NAME} {version}");
    let mut child = Command::new(path)
        .arg("version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("verified candidate could not run: {e}"))?;
    wait_for_candidate(&mut child, Duration::from_secs(10))?;
    let output = child
        .wait_with_output()
        .map_err(|e| format!("could not collect candidate version output: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "candidate `version` command failed with {}",
            output.status
        ));
    }
    let identity = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if identity != expected {
        return Err(format!(
            "candidate identity mismatch: expected {expected:?}, got {identity:?}"
        ));
    }
    Ok(())
}

fn wait_for_candidate(child: &mut Child, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if child
            .try_wait()
            .map_err(|e| format!("could not inspect candidate process: {e}"))?
            .is_some()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("candidate `version` command timed out".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("could not mark candidate executable: {e}"))?;
    }
    Ok(())
}

fn ensure_destination_writable(destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "installed executable has no parent directory".to_string())?;
    let name = format!(
        ".snip-sync.update-check-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let probe = parent.join(name);
    match OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(probe);
            Ok(())
        }
        Err(error) => Err(format!(
            "cannot replace installed executable before stopping any service ({}): {error}",
            destination.display()
        )),
    }
}

fn replace_installed_executable(
    candidate: &Path,
    destination: &Path,
    _workdir: &Path,
    _lifecycle: snip_sync::startup::UpdateLifecycle,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        let parent = destination
            .parent()
            .ok_or_else(|| "installed executable has no parent directory".to_string())?;
        let staged = parent.join(format!(".snip-sync.update-{}", std::process::id()));
        let _ = fs::remove_file(&staged);
        if let Err(error) = fs::copy(candidate, &staged) {
            let _ = fs::remove_file(&staged);
            return Err(format!(
                "could not stage candidate beside installed executable: {error}"
            ));
        }
        if let Ok(permissions) = fs::metadata(destination).map(|m| m.permissions()) {
            if let Err(error) = fs::set_permissions(&staged, permissions) {
                let _ = fs::remove_file(&staged);
                return Err(format!(
                    "could not preserve executable permissions: {error}"
                ));
            }
        } else {
            if let Err(error) = make_executable(&staged) {
                let _ = fs::remove_file(&staged);
                return Err(error);
            }
        }
        let file = match OpenOptions::new().read(true).open(&staged) {
            Ok(file) => file,
            Err(error) => {
                let _ = fs::remove_file(&staged);
                return Err(format!("could not open staged executable: {error}"));
            }
        };
        if let Err(error) = file.sync_all() {
            let _ = fs::remove_file(&staged);
            return Err(format!("could not durably stage executable: {error}"));
        }
        drop(file);
        fs::rename(&staged, destination).map_err(|e| {
            let _ = fs::remove_file(&staged);
            format!("could not replace installed executable: {e}")
        })?;
        Ok(())
    }
    #[cfg(windows)]
    {
        schedule_windows_self_replace(candidate, destination, _workdir, _lifecycle)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (candidate, destination, _workdir, _lifecycle);
        Err("executable replacement is not supported on this platform".into())
    }
}

#[cfg(windows)]
fn stop_running_server(executable: &Path) -> Result<(), String> {
    let status = Command::new(executable)
        .args(["stop", "--force"])
        .status()
        .map_err(|e| format!("failed to stop running server before Windows replacement: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("could not stop running server before Windows replacement".into())
    }
}

#[cfg(windows)]
fn schedule_windows_self_replace(
    candidate: &Path,
    destination: &Path,
    workdir: &Path,
    lifecycle: snip_sync::startup::UpdateLifecycle,
) -> Result<(), String> {
    let helper = workdir.join("snip-sync-self-replace-helper.exe");
    let current = std::env::current_exe()
        .map_err(|e| format!("could not locate updater helper source: {e}"))?;
    fs::copy(current, &helper)
        .map_err(|e| format!("could not stage Windows replacement helper: {e}"))?;
    let managed_restart = matches!(
        lifecycle,
        snip_sync::startup::UpdateLifecycle::ManagedRunning(_)
    );
    let restart = matches!(
        lifecycle,
        snip_sync::startup::UpdateLifecycle::ManagedRunning(_)
            | snip_sync::startup::UpdateLifecycle::DirectRunning
    );
    let mut args = vec![
        "__self-replace".to_owned(),
        "--candidate".to_owned(),
        candidate
            .to_str()
            .ok_or_else(|| "candidate path is not UTF-8".to_string())?
            .to_owned(),
        "--destination".to_owned(),
        destination
            .to_str()
            .ok_or_else(|| "destination path is not UTF-8".to_string())?
            .to_owned(),
    ];
    if restart {
        args.push("--restart".to_owned());
    }
    if managed_restart {
        args.push("--managed-restart".to_owned());
    }
    Command::new(&helper)
        .args(&args)
        .spawn()
        .map_err(|e| format!("could not start Windows replacement helper: {e}"))?;
    println!(
        "Verified candidate staged; Windows replacement will complete after this process exits."
    );
    Ok(())
}

pub fn run_self_replace_helper(
    candidate: &Path,
    destination: &Path,
    restart: bool,
    managed_restart: bool,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let wide = |path: &Path| {
            path.as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>()
        };
        let source = wide(candidate);
        let target = wide(destination);
        let ok = unsafe {
            MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if ok == 0 {
            return Err(format!(
                "Windows replacement failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let _ = fs::remove_file(candidate);
        println!("Windows executable replacement complete.");
        if restart {
            let mut command = Command::new(destination);
            if managed_restart {
                command.arg("restart");
            } else {
                command.args(["croncheck", "--verbose"]);
            }
            let status = command.status().map_err(|e| {
                format!("new snip-sync binary installed, but restart could not be started: {e}")
            })?;
            if !status.success() {
                return Err(format_partial_restart_error(
                    "new snip-sync binary installed, but restart failed",
                ));
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (candidate, destination, restart, managed_restart);
        Err("internal Windows replacement command is only supported on Windows".into())
    }
}

fn format_partial_restart_error(error: &str) -> String {
    format!(
        "new snip-sync version is installed on disk, but restart failed: {error}. The old process may still be active; run `snip-sync restart` manually"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_mapping_matches_release_contract() {
        assert_eq!(
            host_target("linux", "x86_64"),
            Some(HostTarget::Prebuilt("x86_64-unknown-linux-gnu"))
        );
        assert_eq!(
            host_target("linux", "arm"),
            Some(HostTarget::SourceOnly("armv7-unknown-linux-gnueabihf"))
        );
        assert_eq!(
            host_target("windows", "aarch64"),
            Some(HostTarget::SourceOnly("aarch64-pc-windows-msvc"))
        );
        assert_eq!(host_target("freebsd", "x86_64"), None);
    }

    #[test]
    fn component_tag_and_checksum_contract_are_exact() {
        let version = Version::new(1, 2, 3);
        assert_eq!(component_tag(&version), "snip-sync-v1.2.3");
        let expected = if cfg!(windows) {
            "snip-sync-x86_64-unknown-linux-gnu.exe"
        } else {
            "snip-sync-x86_64-unknown-linux-gnu"
        };
        assert_eq!(asset_name("x86_64-unknown-linux-gnu"), expected);
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(file.path(), b"hello").unwrap();
        let mut hasher = Sha256::new();
        hasher.update(b"hello");
        let digest = digest_hex(hasher.finalize().as_ref());
        verify_checksum(
            file.path(),
            format!("{digest}  candidate\n").as_bytes(),
            "candidate",
        )
        .unwrap();
        assert!(
            verify_checksum(
                file.path(),
                format!("{digest} candidate\nextra\n").as_bytes(),
                "candidate"
            )
            .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_helper_replaces_a_temporary_copy() {
        let directory = tempfile::tempdir().unwrap();
        let candidate = directory.path().join("candidate.exe");
        let destination = directory.path().join("destination.exe");
        fs::write(&candidate, b"new executable").unwrap();
        fs::write(&destination, b"old executable").unwrap();
        run_self_replace_helper(&candidate, &destination, false, false).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"new executable");
        assert!(!candidate.exists());
    }
}
