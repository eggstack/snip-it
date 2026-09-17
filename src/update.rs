//! Binary-first self-update support for the `snp` client.
//!
//! The selected version comes from crates.io. Release assets are addressed by
//! the exact component tag and are fully verified before the installed
//! executable is changed. Cargo is a staging fallback for source-only hosts
//! and a definite missing (404) release asset only.

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;

const CRATES_API_URL: &str = "https://crates.io/api/v1/crates/{crate}";
const RELEASE_BASE_URL: &str = "https://github.com/eggstack/snip-it/releases/download";
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstallMethod {
    Cargo,
    Homebrew,
    Direct,
}

impl fmt::Display for InstallMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cargo => "Cargo",
            Self::Homebrew => "Homebrew",
            Self::Direct => "direct executable",
        })
    }
}

#[derive(Debug, Deserialize)]
struct CratesResponse {
    #[serde(rename = "crate")]
    crate_info: CrateInfo,
}

#[derive(Debug, Deserialize)]
struct CrateInfo {
    max_version: String,
}

#[derive(Clone, Copy)]
struct Package {
    crate_name: &'static str,
    binary_name: &'static str,
    tag_prefix: &'static str,
    formula: &'static str,
}

const CLIENT: Package = Package {
    crate_name: "snip-it",
    binary_name: "snp",
    tag_prefix: "v",
    formula: "snip-it",
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostTarget {
    Prebuilt(&'static str),
    SourceOnly(&'static str),
}

#[derive(Debug)]
enum FetchError {
    NotFound,
    Failed(String),
}

pub async fn run(dry_run: bool, _locked: bool) -> Result<(), String> {
    let executable = current_executable()?;
    let method = detect_install_method(&executable, &CLIENT);
    let current = current_version()?;

    println!("Checking for snp updates ({method})...");
    let latest = latest_crates_version(CLIENT.crate_name).await?;
    if latest <= current {
        println!("snp {current} is already up to date.");
        return Ok(());
    }

    println!("Update available: snp {current} -> {latest}");
    if dry_run {
        print_dry_run(CLIENT, &latest, method);
        return Ok(());
    }

    if method == InstallMethod::Homebrew {
        return update_with_homebrew(CLIENT.formula);
    }

    ensure_destination_writable(&executable)?;
    let workdir = tempfile::tempdir()
        .map_err(|e| format!("could not create update staging directory: {e}"))?;
    let target = host_target(std::env::consts::OS, std::env::consts::ARCH);
    let (candidate, source) = match target {
        Some(HostTarget::Prebuilt(target)) => {
            match download_candidate(CLIENT, &latest, target, workdir.path()).await? {
                DownloadedCandidate::Ready(path) => (path, "GitHub release binary"),
                DownloadedCandidate::MissingAsset => {
                    println!("No prebuilt asset is published for {target}; using Cargo fallback.");
                    (
                        cargo_candidate(CLIENT, &latest, workdir.path())?,
                        "Cargo fallback",
                    )
                }
            }
        }
        Some(HostTarget::SourceOnly(target)) => {
            println!("Target {target} is source-only; using Cargo fallback.");
            (
                cargo_candidate(CLIENT, &latest, workdir.path())?,
                "Cargo fallback",
            )
        }
        None => {
            println!("This host has no supported prebuilt target; using Cargo fallback.");
            (
                cargo_candidate(CLIENT, &latest, workdir.path())?,
                "Cargo fallback",
            )
        }
    };

    validate_candidate(&candidate, CLIENT.binary_name, &latest)?;
    replace_installed_executable(&candidate, &executable, workdir.path())?;
    println!("Updated snp {current} -> {latest} (source: {source}).");
    Ok(())
}

fn current_version() -> Result<Version, String> {
    Version::parse(env!("CARGO_PKG_VERSION")).map_err(|e| format!("invalid current version: {e}"))
}

fn print_dry_run(package: Package, latest: &Version, method: InstallMethod) {
    if method == InstallMethod::Homebrew {
        println!(
            "Dry run: Homebrew-managed installation would run `brew upgrade {}`.",
            package.formula
        );
        println!("Dry run: no changes were made.");
        return;
    }
    match host_target(std::env::consts::OS, std::env::consts::ARCH) {
        Some(HostTarget::Prebuilt(target)) => println!(
            "Dry run: would try the prebuilt asset {} at exact tag {}.",
            asset_name(package, target),
            component_tag(package, latest)
        ),
        Some(HostTarget::SourceOnly(target)) => println!(
            "Dry run: target {target} is source-only; would run exact-version Cargo fallback."
        ),
        None => {
            println!("Dry run: host target is unsupported; would run exact-version Cargo fallback.")
        }
    }
    println!("Dry run: no changes were made.");
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

fn detect_install_method(executable: &Path, package: &Package) -> InstallMethod {
    if let Some(prefix) = homebrew_formula_prefix(package.formula)
        && executable.starts_with(&prefix)
    {
        return InstallMethod::Homebrew;
    }
    if is_cargo_install_path(executable) {
        InstallMethod::Cargo
    } else {
        InstallMethod::Direct
    }
}

fn cargo_bin_dir() -> Option<PathBuf> {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".cargo"))
        })?;
    let cargo_bin = cargo_home.join("bin");
    Some(fs::canonicalize(&cargo_bin).unwrap_or(cargo_bin))
}

fn is_cargo_install_path(executable: &Path) -> bool {
    if let Some(cargo_bin) = cargo_bin_dir()
        && executable.starts_with(cargo_bin)
    {
        return true;
    }
    let Some(bin_dir) = executable.parent() else {
        return false;
    };
    bin_dir.file_name().is_some_and(|name| name == "bin")
        && bin_dir.parent().is_some_and(|root| {
            root.join(".crates2.json").is_file() || root.join(".crates.toml").is_file()
        })
}

fn homebrew_formula_prefix(formula: &str) -> Option<PathBuf> {
    let output = Command::new("brew")
        .args(["--prefix", formula])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let prefix = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!prefix.is_empty()).then(|| {
        let prefix = PathBuf::from(prefix);
        fs::canonicalize(&prefix).unwrap_or(prefix)
    })
}

async fn latest_crates_version(crate_name: &str) -> Result<Version, String> {
    let template = update_endpoint("SNIP_UPDATE_CRATES_API_URL", CRATES_API_URL);
    let url = template.replace("{crate}", crate_name);
    let body = fetch_bytes(&url).await.map_err(fetch_error_message)?;
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

fn update_endpoint(name: &str, default: &str) -> String {
    #[cfg(feature = "test-support")]
    {
        std::env::var(name).unwrap_or_else(|_| default.to_owned())
    }
    #[cfg(not(feature = "test-support"))]
    {
        let _ = name;
        default.to_owned()
    }
}

fn release_base_url() -> String {
    update_endpoint("SNIP_UPDATE_RELEASE_BASE_URL", RELEASE_BASE_URL)
}

/// Maximum redirects followed for one logical fetch. GitHub/crates.io update
/// endpoints do not need deep chains; the bound contains loops and bad
/// endpoints rather than implementing browser policy.
const MAX_REDIRECTS: usize = 10;
/// Wall-clock bound for one complete logical fetch, including redirect
/// traversal and final body consumption. This is the parity point with the
/// former `curl --max-time 60`: a redirect chain must not receive a fresh
/// budget per hop, and a streamed download must not run indefinitely after
/// response headers arrive.
const FETCH_OVERALL_TIMEOUT: Duration = Duration::from_secs(60);
const FETCH_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const FETCH_READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Whether plaintext HTTP update endpoints are accepted in this build.
/// Production builds accept HTTPS only; the `test-support` feature widens the
/// policy so fixture servers can be injected without TLS.
fn build_allow_http() -> bool {
    cfg!(feature = "test-support")
}

fn scheme_allowed(url: &str, allow_http: bool) -> bool {
    if allow_http && url.starts_with("http://") {
        return true;
    }
    url.starts_with("https://")
}

fn check_url_scheme(url: &str, allow_http: bool) -> Result<(), FetchError> {
    if scheme_allowed(url, allow_http) {
        Ok(())
    } else {
        Err(FetchError::Failed(format!(
            "insecure or unsupported URL scheme rejected: {url}"
        )))
    }
}

/// Build the updater HTTP client.
///
/// HTTP/1.1 only (selected Cargo features), native-root TLS with packaged
/// WebPKI fallback, no automatic decompression (release bytes must stay
/// byte-for-byte), no automatic redirects (the adapter validates every hop
/// against the scheme policy itself), and no retries — the updater must not
/// hide transport/release defects.
fn update_http_client() -> eggfetch_core::Client {
    http_client_with(FETCH_CONNECT_TIMEOUT, FETCH_READ_TIMEOUT)
}

fn http_client_with(connect: Duration, read: Duration) -> eggfetch_core::Client {
    eggfetch_core::Client::builder()
        .user_agent("snip-it-update")
        .automatic_decompression(false)
        .follow_redirects(false)
        .timeout(
            eggfetch_core::Timeout::builder()
                .connect(connect)
                .read(read)
                .build(),
        )
        .build()
}

fn is_redirect_status(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn redirect_location(response: &eggfetch_core::Response) -> Result<String, FetchError> {
    response
        .headers()
        .get("location")
        .ok_or_else(|| FetchError::Failed("redirect response is missing a Location header".into()))?
        .to_str()
        .map(str::to_owned)
        .map_err(|_| FetchError::Failed("redirect Location header is not valid UTF-8".into()))
}

/// Resolve the next hop for a redirect response and validate it against the
/// scheme policy BEFORE any further network I/O. Relative `Location` values
/// resolve against the response URL via type inference, so this module needs
/// no direct `url` dependency.
fn redirect_target(
    response: &eggfetch_core::Response,
    location: &str,
    allow_http: bool,
) -> Result<String, FetchError> {
    let resolved = response
        .url()
        .join(location)
        .map_err(|e| FetchError::Failed(format!("invalid redirect location {location:?}: {e}")))?;
    let target: String = resolved.into();
    check_url_scheme(&target, allow_http)?;
    Ok(target)
}

/// GET with safe redirect handling: eggfetch automatic redirects stay
/// disabled so every hop passes the scheme policy before it is requested.
async fn safe_get(
    client: &eggfetch_core::Client,
    initial_url: &str,
    max_body_bytes: usize,
    allow_http: bool,
    overall_timeout: Duration,
) -> Result<eggfetch_core::Response, FetchError> {
    let fetch = async {
        let mut current = initial_url.to_owned();
        for _ in 0..=MAX_REDIRECTS {
            check_url_scheme(&current, allow_http)?;
            let response = client
                .get(&current)
                .map_err(|e| FetchError::Failed(format!("invalid update URL {current:?}: {e}")))?
                .max_decoded_body_size(max_body_bytes)
                .send()
                .await
                .map_err(|e| transport_error("update request failed", e))?;
            if !is_redirect_status(response.status().as_u16()) {
                return Ok(response);
            }
            let location = redirect_location(&response)?;
            current = redirect_target(&response, &location, allow_http)?;
        }
        Err(FetchError::Failed(format!(
            "too many redirects (>{MAX_REDIRECTS}) while fetching update"
        )))
    };
    match tokio::time::timeout(overall_timeout, fetch).await {
        Err(_) => Err(FetchError::Failed(format!(
            "update request timed out after {} seconds",
            overall_timeout.as_secs()
        ))),
        Ok(result) => result,
    }
}

fn transport_error(context: &str, error: eggfetch_core::Error) -> FetchError {
    if matches!(error, eggfetch_core::Error::DecodedBodyTooLarge) {
        return FetchError::Failed(format!("{context} exceeds the size limit: {error}"));
    }
    FetchError::Failed(format!("{context}: {error}"))
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    let client = update_http_client();
    fetch_bytes_with(
        &client,
        url,
        MAX_METADATA_BYTES as usize,
        build_allow_http(),
        FETCH_OVERALL_TIMEOUT,
    )
    .await
}

async fn fetch_bytes_with(
    client: &eggfetch_core::Client,
    url: &str,
    max_body_bytes: usize,
    allow_http: bool,
    overall_timeout: Duration,
) -> Result<Vec<u8>, FetchError> {
    let mut response = safe_get(client, url, max_body_bytes, allow_http, overall_timeout).await?;
    let status = response.status().as_u16();
    match status {
        200..=299 => response
            .bytes()
            .await
            .map(|body| body.to_vec())
            .map_err(|e| transport_error("could not read update metadata", e)),
        404 => Err(FetchError::NotFound),
        _ => Err(FetchError::Failed(format!(
            "HTTP {status} while fetching update metadata"
        ))),
    }
}

async fn fetch_file(url: &str, path: &Path) -> Result<(), FetchError> {
    let client = update_http_client();
    fetch_file_with(
        &client,
        url,
        path,
        MAX_BINARY_BYTES as usize,
        build_allow_http(),
        FETCH_OVERALL_TIMEOUT,
    )
    .await
}

async fn fetch_file_with(
    client: &eggfetch_core::Client,
    url: &str,
    path: &Path,
    max_body_bytes: usize,
    allow_http: bool,
    overall_timeout: Duration,
) -> Result<(), FetchError> {
    let mut response = safe_get(client, url, max_body_bytes, allow_http, overall_timeout).await?;
    let status = response.status().as_u16();
    match status {
        200..=299 => {}
        404 => return Err(FetchError::NotFound),
        _ => {
            return Err(FetchError::Failed(format!(
                "HTTP {status} while downloading update binary"
            )));
        }
    }
    // The destination is created only after the final status is classified,
    // so 404/5xx responses never truncate or create the staging file.
    let mut stream = response
        .bytes_stream()
        .map_err(|e| FetchError::Failed(format!("could not stream update binary: {e}")))?;
    let mut output = File::create(path)
        .map_err(|e| FetchError::Failed(format!("could not create staging file: {e}")))?;
    if let Err(error) = stream_binary_to_file(&mut stream, &mut output).await {
        drop(output);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    drop(output);
    Ok(())
}

/// Stream one final 2xx body to disk without buffering the whole release
/// asset in memory. Ordinary blocking writes are sufficient for a one-shot
/// updater; no async file I/O is introduced.
async fn stream_binary_to_file(
    stream: &mut eggfetch_core::BoxBytesStream,
    output: &mut File,
) -> Result<(), FetchError> {
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| transport_error("could not download update binary", e))?;
        output
            .write_all(&chunk)
            .map_err(|e| FetchError::Failed(format!("could not write staging file: {e}")))?;
    }
    Ok(())
}

fn fetch_error_message(error: FetchError) -> String {
    match error {
        FetchError::NotFound => "requested update metadata was not found".into(),
        FetchError::Failed(message) => message,
    }
}

fn host_target(os: &str, arch: &str) -> Option<HostTarget> {
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

fn component_tag(package: Package, version: &Version) -> String {
    format!("{}{}", package.tag_prefix, version)
}

fn asset_name(package: Package, target: &str) -> String {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    format!("{}-{target}{suffix}", package.binary_name)
}

#[derive(Debug)]
enum DownloadedCandidate {
    Ready(PathBuf),
    MissingAsset,
}

async fn download_candidate(
    package: Package,
    version: &Version,
    target: &str,
    staging: &Path,
) -> Result<DownloadedCandidate, String> {
    let asset = asset_name(package, target);
    let tag = component_tag(package, version);
    let base = release_base_url().trim_end_matches('/').to_owned();
    let binary_url = format!("{base}/{tag}/{asset}");
    let checksum_url = format!("{binary_url}.sha256");
    let candidate = staging.join(&asset);
    match fetch_file(&binary_url, &candidate).await {
        Err(FetchError::NotFound) => return Ok(DownloadedCandidate::MissingAsset),
        Err(error) => {
            return Err(format!(
                "could not download {asset}: {}",
                fetch_error_message(error)
            ));
        }
        Ok(()) => {}
    }
    let checksum = fetch_bytes(&checksum_url).await.map_err(|error| {
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

fn cargo_candidate(package: Package, version: &Version, staging: &Path) -> Result<PathBuf, String> {
    let root = staging.join("cargo-root");
    let version_arg = format!("={version}");
    let root_arg = root
        .to_str()
        .ok_or_else(|| "Cargo staging path is not valid UTF-8".to_string())?;
    println!(
        "Running: cargo install {} --version {version_arg} --locked --root {root_arg}",
        package.crate_name
    );
    let status = Command::new("cargo")
        .args([
            "install",
            package.crate_name,
            "--version",
            &version_arg,
            "--locked",
            "--root",
            root_arg,
        ])
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "Cargo is not installed; update manually with `cargo install {} --version '={version}' --locked`",
                    package.crate_name
                )
            } else {
                format!("could not run cargo: {e}")
            }
        })?;
    if !status.success() {
        return Err(format!("cargo exited with status {status}"));
    }
    let binary = if cfg!(windows) {
        root.join("bin")
            .join(format!("{}.exe", package.binary_name))
    } else {
        root.join("bin").join(package.binary_name)
    };
    if !binary.is_file() {
        return Err(format!(
            "Cargo completed but did not produce {}",
            binary.display()
        ));
    }
    Ok(binary)
}

fn validate_candidate(path: &Path, binary_name: &str, version: &Version) -> Result<(), String> {
    let expected = format!("{binary_name} {version}");
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
    let parent = destination.parent().ok_or_else(|| {
        format!(
            "installed executable has no parent directory: {}",
            destination.display()
        )
    })?;
    let name = format!(
        ".{}.update-check-{}-{}",
        destination
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("snp"),
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
) -> Result<(), String> {
    #[cfg(unix)]
    {
        let parent = destination
            .parent()
            .ok_or_else(|| "installed executable has no parent directory".to_string())?;
        let name = format!(
            ".{}.update-{}",
            destination
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            std::process::id()
        );
        let staged = parent.join(name);
        let _ = fs::remove_file(&staged);
        if let Err(error) = fs::copy(candidate, &staged) {
            let _ = fs::remove_file(&staged);
            return Err(format!(
                "could not stage candidate beside installed executable: {error}"
            ));
        }
        let permissions = fs::metadata(destination).map(|m| m.permissions());
        if let Ok(permissions) = permissions {
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
        schedule_windows_self_replace(candidate, destination, _workdir)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (candidate, destination, _workdir);
        Err("executable replacement is not supported on this platform".into())
    }
}

#[cfg(windows)]
fn schedule_windows_self_replace(
    candidate: &Path,
    destination: &Path,
    workdir: &Path,
) -> Result<(), String> {
    let helper = workdir.join("snp-self-replace-helper.exe");
    let current = std::env::current_exe()
        .map_err(|e| format!("could not locate updater helper source: {e}"))?;
    fs::copy(current, &helper)
        .map_err(|e| format!("could not stage Windows replacement helper: {e}"))?;
    Command::new(&helper)
        .args([
            "__self-replace",
            "--candidate",
            candidate
                .to_str()
                .ok_or_else(|| "candidate path is not UTF-8".to_string())?,
            "--destination",
            destination
                .to_str()
                .ok_or_else(|| "destination path is not UTF-8".to_string())?,
        ])
        .spawn()
        .map_err(|e| format!("could not start Windows replacement helper: {e}"))?;
    println!(
        "Verified candidate staged; Windows replacement will complete after this process exits."
    );
    Ok(())
}

pub fn run_self_replace_helper(candidate: &Path, destination: &Path) -> Result<(), String> {
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
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (candidate, destination);
        Err("internal Windows replacement command is only supported on Windows".into())
    }
}

fn update_with_homebrew(formula: &str) -> Result<(), String> {
    println!("Running: brew upgrade {formula}");
    let status = Command::new("brew")
        .args(["upgrade", formula])
        .status()
        .map_err(|e| format!("could not run brew: {e}"))?;
    if status.success() {
        println!("Update complete.");
        Ok(())
    } else {
        Err(format!("brew exited with status {status}"))
    }
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
            host_target("linux", "aarch64"),
            Some(HostTarget::Prebuilt("aarch64-unknown-linux-gnu"))
        );
        assert_eq!(
            host_target("linux", "arm"),
            Some(HostTarget::SourceOnly("armv7-unknown-linux-gnueabihf"))
        );
        assert_eq!(
            host_target("macos", "aarch64"),
            Some(HostTarget::Prebuilt("aarch64-apple-darwin"))
        );
        assert_eq!(
            host_target("windows", "x86_64"),
            Some(HostTarget::Prebuilt("x86_64-pc-windows-msvc"))
        );
        assert_eq!(
            host_target("windows", "aarch64"),
            Some(HostTarget::SourceOnly("aarch64-pc-windows-msvc"))
        );
        assert_eq!(host_target("freebsd", "x86_64"), None);
    }

    #[test]
    fn component_tags_and_assets_are_exact() {
        let version = Version::new(1, 2, 3);
        assert_eq!(component_tag(CLIENT, &version), "v1.2.3");
        let expected = if cfg!(windows) {
            "snp-x86_64-unknown-linux-gnu.exe"
        } else {
            "snp-x86_64-unknown-linux-gnu"
        };
        assert_eq!(asset_name(CLIENT, "x86_64-unknown-linux-gnu"), expected);
    }

    #[test]
    fn checksum_parser_rejects_ambiguity_and_accepts_workflow_format() {
        let path = tempfile::NamedTempFile::new().unwrap();
        fs::write(path.path(), b"hello").unwrap();
        let mut hasher = Sha256::new();
        hasher.update(b"hello");
        let digest = digest_hex(hasher.finalize().as_ref());
        let sidecar = format!("{digest}  candidate\n");
        verify_checksum(path.path(), sidecar.as_bytes(), "candidate").unwrap();
        assert!(
            verify_checksum(
                path.path(),
                format!("{digest}  candidate\nextra\n").as_bytes(),
                "candidate"
            )
            .is_err()
        );
        assert!(
            verify_checksum(
                path.path(),
                format!("{digest}  other\n").as_bytes(),
                "candidate"
            )
            .is_err()
        );
    }

    #[test]
    fn unmanaged_executables_are_binary_first() {
        let executable = Path::new("/usr/local/bin/snp");
        assert_eq!(
            detect_install_method_with_prefixes(executable, None, None),
            InstallMethod::Direct
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
        run_self_replace_helper(&candidate, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"new executable");
        assert!(!candidate.exists());
    }

    fn detect_install_method_with_prefixes(
        executable: &Path,
        brew: Option<&Path>,
        cargo: Option<&Path>,
    ) -> InstallMethod {
        if brew.is_some_and(|prefix| executable.starts_with(prefix)) {
            return InstallMethod::Homebrew;
        }
        if cargo.is_some_and(|prefix| executable.starts_with(prefix)) {
            return InstallMethod::Cargo;
        }
        InstallMethod::Direct
    }

    /// Plan 014 transport tests. These use the `test-support` endpoint
    /// injection seam to point the updater at a tiny standard-library HTTP
    /// fixture (no mock framework, no second HTTP client). Each fixture
    /// serves plain HTTP on loopback; production-policy cases pass
    /// `allow_http = false` explicitly so the HTTPS-only rule is proven
    /// against live fixtures without needing TLS test certificates.
    #[cfg(feature = "test-support")]
    mod transport_tests {
        use super::*;
        use std::collections::HashMap;
        use std::net::{TcpListener, TcpStream};
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };
        use std::thread;

        const TEST_OVERALL: Duration = Duration::from_secs(10);

        struct Reply {
            status: u16,
            headers: Vec<(String, String)>,
            body: Vec<u8>,
            delay: Duration,
        }

        fn ok(body: &[u8]) -> Reply {
            Reply {
                status: 200,
                headers: Vec::new(),
                body: body.to_vec(),
                delay: Duration::ZERO,
            }
        }

        fn status_only(status: u16) -> Reply {
            Reply {
                status,
                headers: Vec::new(),
                body: Vec::new(),
                delay: Duration::ZERO,
            }
        }

        fn reason_phrase(status: u16) -> &'static str {
            match status {
                200 => "OK",
                301 => "Moved Permanently",
                302 => "Found",
                303 => "See Other",
                307 => "Temporary Redirect",
                308 => "Permanent Redirect",
                401 => "Unauthorized",
                403 => "Forbidden",
                404 => "Not Found",
                500 => "Internal Server Error",
                _ => "Unknown",
            }
        }

        /// Minimal single-threaded HTTP/1.1 fixture. Routes are matched on
        /// the request path (query stripped) and every hit is counted so
        /// tests can prove a target was never contacted.
        struct Fixture {
            base_url: String,
            counts: Arc<Mutex<HashMap<String, usize>>>,
            shutdown: Arc<AtomicBool>,
            thread: Option<thread::JoinHandle<()>>,
        }

        impl Fixture {
            fn start<H>(make_handler: impl FnOnce(String) -> H) -> Self
            where
                H: Fn(&str) -> Reply + Send + Sync + 'static,
            {
                let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
                let port = listener.local_addr().expect("fixture addr").port();
                listener.set_nonblocking(true).expect("nonblocking");
                let base_url = format!("http://127.0.0.1:{port}");
                let handler: Arc<dyn Fn(&str) -> Reply + Send + Sync> =
                    Arc::new(make_handler(base_url.clone()));
                let counts = Arc::new(Mutex::new(HashMap::new()));
                let shutdown = Arc::new(AtomicBool::new(false));
                let counts_loop = Arc::clone(&counts);
                let shutdown_loop = Arc::clone(&shutdown);
                let thread = thread::spawn(move || {
                    while !shutdown_loop.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                handle_connection(stream, &handler, &counts_loop);
                            }
                            Err(_) => thread::sleep(Duration::from_millis(5)),
                        }
                    }
                });
                Self {
                    base_url,
                    counts,
                    shutdown,
                    thread: Some(thread),
                }
            }

            fn url(&self, path: &str) -> String {
                format!("{}{}", self.base_url, path)
            }

            fn count(&self, path: &str) -> usize {
                self.counts
                    .lock()
                    .expect("fixture counts")
                    .get(path)
                    .copied()
                    .unwrap_or(0)
            }
        }

        impl Drop for Fixture {
            fn drop(&mut self) {
                self.shutdown.store(true, Ordering::Relaxed);
                if let Some(thread) = self.thread.take() {
                    let _ = thread.join();
                }
            }
        }

        fn handle_connection(
            mut stream: TcpStream,
            handler: &Arc<dyn Fn(&str) -> Reply + Send + Sync>,
            counts: &Arc<Mutex<HashMap<String, usize>>>,
        ) {
            stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        request.extend_from_slice(&chunk[..n]);
                        if request.windows(4).any(|w| w == b"\r\n\r\n") || request.len() > 65_536 {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let text = String::from_utf8_lossy(&request);
            let target = text
                .lines()
                .next()
                .unwrap_or_default()
                .split_whitespace()
                .nth(1)
                .unwrap_or("/");
            let route = target.split('?').next().unwrap_or("/").to_owned();
            counts
                .lock()
                .expect("fixture counts")
                .entry(route.clone())
                .and_modify(|hits| *hits += 1)
                .or_insert(1);
            let reply = handler(&route);
            if !reply.delay.is_zero() {
                thread::sleep(reply.delay);
            }
            let mut head = format!(
                "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                reply.status,
                reason_phrase(reply.status),
                reply.body.len()
            );
            for (name, value) in &reply.headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str("\r\n");
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&reply.body);
            let _ = stream.flush();
        }

        #[tokio::test]
        async fn initial_production_http_url_rejected_before_network_io() {
            let fixture = Fixture::start(|_| |_: &str| ok(b"must never be served"));
            let client = update_http_client();
            let error = fetch_bytes_with(
                &client,
                &fixture.url("/counted"),
                MAX_METADATA_BYTES as usize,
                false,
                TEST_OVERALL,
            )
            .await
            .expect_err("production policy must reject plaintext HTTP");
            assert!(
                matches!(error, FetchError::Failed(_)),
                "unexpected error: {error:?}"
            );
            assert_eq!(
                fixture.count("/counted"),
                0,
                "rejected URL must never be requested"
            );
        }

        #[tokio::test]
        async fn test_feature_http_fixture_url_accepted() {
            let fixture = Fixture::start(|_| |_: &str| ok(b"hello-bytes"));
            let client = update_http_client();
            let body = fetch_bytes_with(
                &client,
                &fixture.url("/hello"),
                MAX_METADATA_BYTES as usize,
                true,
                TEST_OVERALL,
            )
            .await
            .expect("test policy must accept HTTP fixtures");
            assert_eq!(body, b"hello-bytes");
            assert_eq!(fixture.count("/hello"), 1);
        }

        #[tokio::test]
        async fn metadata_body_over_limit_fails() {
            assert_eq!(MAX_METADATA_BYTES, 1024 * 1024);
            let big = vec![0x41u8; MAX_METADATA_BYTES as usize + 16];
            let fixture = Fixture::start(|_| move |_: &str| ok(&big));
            let client = update_http_client();
            let error = fetch_bytes_with(
                &client,
                &fixture.url("/big"),
                MAX_METADATA_BYTES as usize,
                true,
                TEST_OVERALL,
            )
            .await
            .expect_err("metadata over 1 MiB must fail");
            match error {
                FetchError::Failed(message) => {
                    assert!(
                        message.contains("exceeds the size limit"),
                        "unexpected message: {message}"
                    );
                }
                FetchError::NotFound => panic!("oversize body must not map to NotFound"),
            }
        }

        #[tokio::test]
        async fn binary_streams_to_disk() {
            let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
            let fixture = Fixture::start(|_| move |_: &str| ok(&body));
            let directory = tempfile::tempdir().unwrap();
            let staging = directory.path().join("snp-test-binary");
            assert!(!staging.exists());
            let client = update_http_client();
            fetch_file_with(
                &client,
                &fixture.url("/bin"),
                &staging,
                MAX_BINARY_BYTES as usize,
                true,
                TEST_OVERALL,
            )
            .await
            .expect("binary download must stream to disk");
            let expected: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
            assert_eq!(fs::read(&staging).unwrap(), expected);
        }

        #[tokio::test]
        async fn binary_over_limit_fails_without_partial() {
            assert_eq!(MAX_BINARY_BYTES, 256 * 1024 * 1024);
            // The streaming limit mechanism is exercised with a small bound;
            // the production 256 MiB bound itself is asserted above.
            let big = vec![0x42u8; 128 * 1024];
            let fixture = Fixture::start(|_| move |_: &str| ok(&big));
            let directory = tempfile::tempdir().unwrap();
            let staging = directory.path().join("snp-test-binary");
            let client = update_http_client();
            let error = fetch_file_with(
                &client,
                &fixture.url("/bigbin"),
                &staging,
                64 * 1024,
                true,
                TEST_OVERALL,
            )
            .await
            .expect_err("binary over the limit must fail");
            match error {
                FetchError::Failed(message) => {
                    assert!(
                        message.contains("exceeds the size limit"),
                        "unexpected message: {message}"
                    );
                }
                FetchError::NotFound => panic!("oversize body must not map to NotFound"),
            }
            assert!(
                !staging.exists(),
                "failed download must not leave a usable partial candidate"
            );
        }

        #[tokio::test]
        async fn not_found_maps_to_not_found_without_creating_staging_file() {
            let fixture = Fixture::start(|_| |_: &str| status_only(404));
            let client = update_http_client();
            let error = fetch_bytes_with(
                &client,
                &fixture.url("/metadata"),
                MAX_METADATA_BYTES as usize,
                true,
                TEST_OVERALL,
            )
            .await
            .expect_err("metadata 404 must fail");
            assert!(
                matches!(error, FetchError::NotFound),
                "unexpected error: {error:?}"
            );
            let directory = tempfile::tempdir().unwrap();
            let staging = directory.path().join("snp-test-binary");
            let error = fetch_file_with(
                &client,
                &fixture.url("/binary"),
                &staging,
                MAX_BINARY_BYTES as usize,
                true,
                TEST_OVERALL,
            )
            .await
            .expect_err("binary 404 must fail");
            assert!(
                matches!(error, FetchError::NotFound),
                "unexpected error: {error:?}"
            );
            assert!(
                !staging.exists(),
                "a 404 must not create or truncate the staging file"
            );
        }

        #[tokio::test]
        async fn missing_asset_falls_back_while_checksum_404_hard_fails() {
            let fixture = Fixture::start(|_| {
                |path: &str| {
                    if path.ends_with(".sha256") {
                        return status_only(404);
                    }
                    if path.contains("v9.9.8") {
                        return status_only(404);
                    }
                    ok(b"fake-binary")
                }
            });
            unsafe {
                std::env::set_var("SNIP_UPDATE_RELEASE_BASE_URL", &fixture.base_url);
            }
            let staging = tempfile::tempdir().unwrap();
            let target = "x86_64-unknown-linux-gnu";
            let missing =
                download_candidate(CLIENT, &Version::new(9, 9, 8), target, staging.path()).await;
            assert!(
                matches!(missing, Ok(DownloadedCandidate::MissingAsset)),
                "binary asset 404 must stay on the Cargo fallback path, got {missing:?}"
            );
            let checksum_missing =
                download_candidate(CLIENT, &Version::new(9, 9, 9), target, staging.path()).await;
            let message = checksum_missing.expect_err("checksum 404 must be a hard failure");
            assert!(
                message.contains("could not download checksum"),
                "unexpected message: {message}"
            );
            unsafe {
                std::env::remove_var("SNIP_UPDATE_RELEASE_BASE_URL");
            }
        }

        #[tokio::test]
        async fn http_error_statuses_are_hard_failures() {
            let fixture = Fixture::start(|_| {
                |path: &str| match path {
                    "/denied" => status_only(401),
                    "/forbidden" => status_only(403),
                    "/boom" => status_only(500),
                    _ => status_only(404),
                }
            });
            let client = update_http_client();
            for (path, expected) in [("/denied", 401), ("/forbidden", 403), ("/boom", 500)] {
                let error = fetch_bytes_with(
                    &client,
                    &fixture.url(path),
                    MAX_METADATA_BYTES as usize,
                    true,
                    TEST_OVERALL,
                )
                .await
                .unwrap_err();
                match error {
                    FetchError::Failed(message) => {
                        assert!(
                            message.contains(&format!("HTTP {expected}")),
                            "unexpected message: {message}"
                        );
                    }
                    FetchError::NotFound => {
                        panic!("HTTP {expected} must not map to NotFound")
                    }
                }
            }
        }

        #[tokio::test]
        async fn timeout_is_hard_failure() {
            let fixture = Fixture::start(|_| {
                |_: &str| Reply {
                    status: 200,
                    headers: Vec::new(),
                    body: b"late".to_vec(),
                    delay: Duration::from_secs(5),
                }
            });
            let client = update_http_client();
            let error = fetch_bytes_with(
                &client,
                &fixture.url("/slow"),
                1024,
                true,
                Duration::from_millis(300),
            )
            .await
            .expect_err("overall timeout must fail");
            match error {
                FetchError::Failed(message) => {
                    assert!(
                        message.contains("timed out"),
                        "unexpected message: {message}"
                    );
                }
                FetchError::NotFound => panic!("timeout must not map to NotFound"),
            }
            let impatient = http_client_with(Duration::from_secs(10), Duration::from_millis(100));
            let error = fetch_bytes_with(
                &impatient,
                &fixture.url("/slow"),
                1024,
                true,
                Duration::from_secs(3),
            )
            .await
            .expect_err("read timeout must fail");
            assert!(
                matches!(error, FetchError::Failed(_)),
                "timeout must be a hard failure, got {error:?}"
            );
        }

        #[tokio::test]
        async fn relative_redirect_resolves() {
            let fixture = Fixture::start(|_| {
                |path: &str| match path {
                    "/start" => Reply {
                        status: 302,
                        headers: vec![("Location".to_owned(), "/target".to_owned())],
                        body: Vec::new(),
                        delay: Duration::ZERO,
                    },
                    "/target" => ok(b"redirected-body"),
                    _ => status_only(404),
                }
            });
            let client = update_http_client();
            let body = fetch_bytes_with(
                &client,
                &fixture.url("/start"),
                MAX_METADATA_BYTES as usize,
                true,
                TEST_OVERALL,
            )
            .await
            .expect("relative redirect must resolve");
            assert_eq!(body, b"redirected-body");
            assert_eq!(fixture.count("/target"), 1);
        }

        #[tokio::test]
        async fn https_redirect_target_accepted_under_production_policy() {
            assert!(scheme_allowed("https://example.test/x", false));
            assert!(!scheme_allowed("http://example.test/x", false));
            assert!(!scheme_allowed("ftp://example.test/x", false));
            assert!(scheme_allowed("http://127.0.0.1:9/x", true));
            let fixture = Fixture::start(|_| {
                |_: &str| Reply {
                    status: 302,
                    headers: vec![(
                        "Location".to_owned(),
                        "https://example.test/other".to_owned(),
                    )],
                    body: Vec::new(),
                    delay: Duration::ZERO,
                }
            });
            let client = update_http_client();
            let response = client
                .get(&fixture.url("/hop"))
                .expect("fixture URL must parse")
                .send()
                .await
                .expect("fixture hop must be served");
            assert_eq!(response.status().as_u16(), 302);
            let location = redirect_location(&response).expect("Location must parse");
            assert_eq!(location, "https://example.test/other");
            let next = redirect_target(&response, &location, false)
                .expect("HTTPS target must pass the production gate");
            assert_eq!(next, "https://example.test/other");
        }

        #[tokio::test]
        async fn https_to_http_redirect_rejected_before_target_request() {
            let fixture = Fixture::start(|base| {
                move |path: &str| match path {
                    "/downgrade" => Reply {
                        status: 302,
                        headers: vec![("Location".to_owned(), format!("{base}/counted"))],
                        body: Vec::new(),
                        delay: Duration::ZERO,
                    },
                    "/counted" => ok(b"plaintext"),
                    _ => status_only(404),
                }
            });
            let client = update_http_client();
            // Obtain a live redirect response under the test policy, then run
            // the loop's exact redirect gate with the production policy.
            let response = client
                .get(&fixture.url("/downgrade"))
                .expect("fixture URL must parse")
                .send()
                .await
                .expect("fixture hop must be served");
            assert_eq!(response.status().as_u16(), 302);
            let location = redirect_location(&response).expect("Location must parse");
            let rejected = redirect_target(&response, &location, false);
            assert!(
                matches!(rejected, Err(FetchError::Failed(_))),
                "downgrade target must be rejected, got {rejected:?}"
            );
            assert_eq!(
                fixture.count("/counted"),
                0,
                "plaintext redirect target must never be requested"
            );
            // The full production-policy fetch rejects before any I/O at all.
            let downgrade_before = fixture.count("/downgrade");
            let error = fetch_bytes_with(
                &client,
                &fixture.url("/downgrade"),
                1024,
                false,
                TEST_OVERALL,
            )
            .await
            .expect_err("production policy must reject the HTTP fetch");
            assert!(
                matches!(error, FetchError::Failed(_)),
                "unexpected error: {error:?}"
            );
            assert_eq!(
                fixture.count("/downgrade"),
                downgrade_before,
                "rejected fetch must not issue further requests"
            );
            assert_eq!(fixture.count("/counted"), 0);
        }

        #[tokio::test]
        async fn redirect_loop_rejected() {
            let fixture = Fixture::start(|_| {
                |_: &str| Reply {
                    status: 302,
                    headers: vec![("Location".to_owned(), "/loop".to_owned())],
                    body: Vec::new(),
                    delay: Duration::ZERO,
                }
            });
            let client = update_http_client();
            let error = fetch_bytes_with(
                &client,
                &fixture.url("/loop"),
                MAX_METADATA_BYTES as usize,
                true,
                TEST_OVERALL,
            )
            .await
            .expect_err("redirect loop must fail");
            match error {
                FetchError::Failed(message) => {
                    assert!(
                        message.contains("too many redirects"),
                        "unexpected message: {message}"
                    );
                }
                FetchError::NotFound => panic!("redirect loop must not map to NotFound"),
            }
        }
    }
}
