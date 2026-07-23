//! **Layer: Application**
//!
//! Self-update support for the `snp` client.
//!
//! The update source follows the way the executable was installed: Cargo
//! installs are refreshed from crates.io, Homebrew installs are upgraded by
//! Homebrew, and standalone release binaries are replaced from GitHub.

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use uuid::Uuid;

const REPOSITORY: &str = "eggstack/snip-it";
const CRATES_API_URL: &str = "https://crates.io/api/v1/crates/{crate}";
const RELEASE_API_URL: &str = "https://api.github.com/repos/eggstack/snip-it/releases/latest";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstallMethod {
    Cargo,
    Homebrew,
    GitHubRelease,
    Unsupported,
}

impl fmt::Display for InstallMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cargo => f.write_str("Cargo"),
            Self::Homebrew => f.write_str("Homebrew"),
            Self::GitHubRelease => f.write_str("GitHub release binary"),
            Self::Unsupported => f.write_str("unmanaged executable"),
        }
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

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Clone, Copy)]
struct Package {
    crate_name: &'static str,
    formula: &'static str,
    binary_name: &'static str,
    release_prefix: &'static str,
}

const CLIENT: Package = Package {
    crate_name: "snip-it",
    formula: "snip-it",
    binary_name: "snp",
    release_prefix: "snip-it",
};

pub fn run(dry_run: bool, locked: bool) -> Result<(), String> {
    let executable = current_executable()?;
    let method = detect_install_method(&executable, &CLIENT);
    if method == InstallMethod::Unsupported {
        return Err(format!(
            "snp is running from a source build ({}); rebuild it from source or install it with Cargo, Homebrew, or a GitHub release archive",
            executable.display()
        ));
    }
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|e| format!("invalid current version: {e}"))?;

    println!("Checking for snp updates ({method})...");
    let release = match method {
        InstallMethod::Homebrew | InstallMethod::GitHubRelease => Some(latest_github_release()?),
        InstallMethod::Cargo | InstallMethod::Unsupported => None,
    };
    let latest = match method {
        InstallMethod::Cargo => latest_crates_version(CLIENT.crate_name)?,
        InstallMethod::Homebrew | InstallMethod::GitHubRelease => release
            .as_ref()
            .expect("release metadata was fetched for this method")
            .version()?,
        InstallMethod::Unsupported => {
            unreachable!("unsupported methods return before update checks")
        }
    };

    if latest <= current {
        println!("snp {current} is already up to date.");
        return Ok(());
    }

    println!("Update available: snp {current} -> {latest}");
    if dry_run {
        println!("Dry run: no changes were made.");
        return Ok(());
    }

    match method {
        InstallMethod::Cargo => update_with_cargo(CLIENT.crate_name, locked),
        InstallMethod::Homebrew => update_with_homebrew(CLIENT.formula),
        InstallMethod::GitHubRelease => update_from_github(
            &executable,
            &CLIENT,
            release
                .as_ref()
                .expect("release metadata was fetched for this method"),
        ),
        InstallMethod::Unsupported => unreachable!("unsupported methods return before updating"),
    }
}

fn current_executable() -> Result<PathBuf, String> {
    let path = std::env::current_exe()
        .map_err(|e| format!("could not locate the running executable: {e}"))?;
    Ok(fs::canonicalize(&path).unwrap_or(path))
}

fn detect_install_method(executable: &Path, package: &Package) -> InstallMethod {
    if let Some(prefix) = homebrew_formula_prefix(package.formula)
        && executable.starts_with(&prefix)
    {
        return InstallMethod::Homebrew;
    }

    if is_cargo_install_path(executable) {
        return InstallMethod::Cargo;
    }

    if is_source_build_path(executable) {
        InstallMethod::Unsupported
    } else {
        InstallMethod::GitHubRelease
    }
}

fn is_source_build_path(path: &Path) -> bool {
    let components: Vec<_> = path.components().collect();
    components.windows(2).any(|window| {
        window[0].as_os_str() == "target"
            && matches!(window[1].as_os_str().to_str(), Some("debug" | "release"))
    })
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

fn latest_crates_version(crate_name: &str) -> Result<Version, String> {
    let template =
        std::env::var("SNIP_UPDATE_CRATES_API_URL").unwrap_or_else(|_| CRATES_API_URL.to_owned());
    let url = template.replace("{crate}", crate_name);
    let body = fetch_url(&url)?;
    let response: CratesResponse = serde_json::from_slice(&body)
        .map_err(|e| format!("could not parse crates.io response: {e}"))?;
    Version::parse(&response.crate_info.max_version).map_err(|e| {
        format!(
            "crates.io returned invalid version {:?}: {e}",
            response.crate_info.max_version
        )
    })
}

fn latest_github_release() -> Result<GitHubRelease, String> {
    let url =
        std::env::var("SNIP_UPDATE_RELEASE_API_URL").unwrap_or_else(|_| RELEASE_API_URL.to_owned());
    let body = fetch_url(&url)?;
    serde_json::from_slice(&body)
        .map_err(|e| format!("could not parse GitHub release response: {e}"))
}

impl GitHubRelease {
    fn version(&self) -> Result<Version, String> {
        let tag = self.tag_name.strip_prefix('v').unwrap_or(&self.tag_name);
        Version::parse(tag).map_err(|e| {
            format!(
                "GitHub returned invalid release tag {:?}: {e}",
                self.tag_name
            )
        })
    }
}

fn fetch_url(url: &str) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err(format!(
            "insecure or unsupported URL scheme rejected (production update requires HTTPS): {url}"
        ));
    }
    let mut args = vec![
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--tlsv1.2",
        "--user-agent",
        "snip-it-update",
    ];
    args.push(url);
    let output = Command::new("curl")
        .args(&args)
        .output()
        .map_err(|e| format!("could not run curl: {e}. Install curl or update manually from https://github.com/{REPOSITORY}/releases"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if detail.is_empty() {
            format!("download failed with status {}", output.status)
        } else {
            format!("download failed: {detail}")
        });
    }
    Ok(output.stdout)
}

fn update_with_cargo(crate_name: &str, locked: bool) -> Result<(), String> {
    let mut args = vec!["install", crate_name];
    if locked {
        args.push("--locked");
    }
    println!("Running: cargo {}", args.join(" "));
    run_status("cargo", &args)?;
    println!("Update complete.");
    Ok(())
}

fn update_with_homebrew(formula: &str) -> Result<(), String> {
    println!("Running: brew upgrade {formula}");
    run_status("brew", &["upgrade", formula])?;
    println!("Update complete.");
    Ok(())
}

fn update_from_github(
    executable: &Path,
    package: &Package,
    release: &GitHubRelease,
) -> Result<(), String> {
    let target = platform_target()?;
    let archive_extension = if cfg!(windows) { "zip" } else { "tar.gz" };
    let tag = &release.tag_name;
    let archive_name = format!(
        "{}-{tag}-{target}.{archive_extension}",
        package.release_prefix
    );
    let archive = release
        .assets
        .iter()
        .find(|asset| asset.name == archive_name)
        .ok_or_else(|| {
            format!(
                "GitHub release {} has no asset for this platform ({archive_name}); install {} with Cargo instead",
                release.tag_name, package.crate_name
            )
        })?;
    let checksum = release
        .assets
        .iter()
        .find(|asset| asset.name == "SHA256SUMS")
        .ok_or_else(|| "GitHub release is missing its SHA256SUMS manifest".to_owned())?;

    let work_dir = temporary_directory(executable.parent())?;
    let archive_path = work_dir.join(&archive.name);
    write_download(&archive.browser_download_url, &archive_path)?;
    let checksums = fetch_url(&checksum.browser_download_url)?;
    verify_checksum(&checksums, &archive.name, &archive_path)?;

    extract_archive(&archive_path, &work_dir)?;
    let extracted = work_dir.join(if cfg!(windows) {
        format!("{}.exe", package.binary_name)
    } else {
        package.binary_name.to_owned()
    });
    if !extracted.is_file() {
        let _ = fs::remove_dir_all(&work_dir);
        return Err(format!(
            "release archive did not contain {}",
            extracted.display()
        ));
    }

    install_binary(&extracted, executable, &work_dir)?;
    println!(
        "Update complete. Restart snp to use version {}.",
        release.tag_name
    );
    Ok(())
}

fn temporary_directory(preferred_parent: Option<&Path>) -> Result<PathBuf, String> {
    let parent = preferred_parent
        .filter(|path| path.is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    let path = parent.join(format!(".snip-it-update-{}", Uuid::new_v4()));
    fs::create_dir(&path)
        .map_err(|e| format!("could not create update directory {}: {e}", path.display()))?;
    Ok(path)
}

fn write_download(url: &str, path: &Path) -> Result<(), String> {
    let body = fetch_url(url)?;
    let mut file =
        fs::File::create(path).map_err(|e| format!("could not create {}: {e}", path.display()))?;
    file.write_all(&body)
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

fn verify_checksum(manifest: &[u8], filename: &str, path: &Path) -> Result<(), String> {
    let manifest = String::from_utf8_lossy(manifest);
    let expected = manifest
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let hash = fields.next()?;
            let name = fields.next()?.trim_start_matches('*');
            (name == filename).then_some(hash)
        })
        .next()
        .ok_or_else(|| format!("SHA256SUMS did not contain {filename}"))?;

    let bytes = fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let actual = hex_digest(Sha256::digest(bytes).as_ref());
    if actual != expected {
        return Err(format!(
            "checksum mismatch for {filename}: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn extract_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    let file_name = archive.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz") {
        extract_tar_gz(archive, destination)
    } else if file_name.ends_with(".zip") {
        extract_zip(archive, destination)
    } else {
        Err(format!("unsupported archive format: {}", archive.display()))
    }
}

const MAX_TAR_ENTRIES: usize = 1000;
const MAX_ENTRY_UNCOMPRESSED_SIZE: u64 = 100 * 1024 * 1024; // 100 MiB
const MAX_TOTAL_UNCOMPRESSED_SIZE: u64 = 500 * 1024 * 1024; // 500 MiB
const MAX_ZIP_ENTRIES: usize = 1000;
const MAX_ZIP_ENTRY_UNCOMPRESSED_SIZE: u64 = 100 * 1024 * 1024; // 100 MiB
const MAX_ZIP_TOTAL_UNCOMPRESSED_SIZE: u64 = 500 * 1024 * 1024; // 500 MiB

fn extract_tar_gz(archive: &Path, destination: &Path) -> Result<(), String> {
    let file = fs::File::open(archive)
        .map_err(|e| format!("could not open {}: {e}", archive.display()))?;
    let dec = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    archive.set_overwrite(false);
    let entries = archive
        .entries()
        .map_err(|e| format!("could not read tar entries: {e}"))?;
    let mut entry_count: usize = 0;
    let mut total_uncompressed: u64 = 0;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("could not read tar entry: {e}"))?;
        let entry_path = entry
            .path()
            .map_err(|e| format!("invalid tar entry path: {e}"))?;
        validate_tar_entry(&entry_path, &entry)?;
        entry_count += 1;
        if entry_count > MAX_TAR_ENTRIES {
            return Err(format!(
                "tar archive exceeds maximum entry count of {MAX_TAR_ENTRIES}"
            ));
        }
        let size = entry.size();
        total_uncompressed += size;
        if size > MAX_ENTRY_UNCOMPRESSED_SIZE {
            return Err(format!(
                "tar entry {} exceeds maximum uncompressed size of {MAX_ENTRY_UNCOMPRESSED_SIZE} bytes",
                entry_path.display()
            ));
        }
        if total_uncompressed > MAX_TOTAL_UNCOMPRESSED_SIZE {
            return Err(format!(
                "tar archive exceeds maximum total uncompressed size of {MAX_TOTAL_UNCOMPRESSED_SIZE} bytes"
            ));
        }
        entry
            .unpack_in(destination)
            .map_err(|e| format!("could not extract tar entry: {e}"))?;
    }
    Ok(())
}

fn validate_tar_entry(
    path: &std::path::Path,
    entry: &tar::Entry<'_, impl std::io::Read>,
) -> Result<(), String> {
    let components: Vec<_> = path.components().collect();
    for component in &components {
        match component {
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(format!(
                    "rejecting absolute path in archive: {}",
                    path.display()
                ));
            }
            std::path::Component::ParentDir => {
                return Err(format!(
                    "rejecting parent traversal in archive: {}",
                    path.display()
                ));
            }
            _ => {}
        }
    }
    let file_type = entry.header().entry_type();
    match file_type {
        tar::EntryType::Regular | tar::EntryType::Continuous => {}
        tar::EntryType::Symlink => {
            return Err(format!("rejecting symlink in archive: {}", path.display()));
        }
        tar::EntryType::Link => {
            return Err(format!(
                "rejecting hard link in archive: {}",
                path.display()
            ));
        }
        tar::EntryType::Directory => {}
        _ => {
            return Err(format!(
                "rejecting unexpected entry type {:?} in archive: {}",
                file_type,
                path.display()
            ));
        }
    }
    Ok(())
}

fn extract_zip(archive: &Path, destination: &Path) -> Result<(), String> {
    let file = fs::File::open(archive)
        .map_err(|e| format!("could not open {}: {e}", archive.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("could not read zip archive: {e}"))?;

    let mut entry_count: usize = 0;
    let mut total_uncompressed: u64 = 0;

    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("could not read zip entry: {e}"))?;
        let entry_path = entry
            .enclosed_name()
            .ok_or_else(|| format!("rejecting malicious zip entry path: {:?}", entry.name()))?;

        validate_zip_entry_path(&entry_path)?;

        if entry.is_symlink() {
            return Err(format!(
                "rejecting symlink in zip archive: {}",
                entry.name()
            ));
        }

        entry_count += 1;
        if entry_count > MAX_ZIP_ENTRIES {
            return Err(format!(
                "zip archive exceeds maximum entry count of {MAX_ZIP_ENTRIES}"
            ));
        }

        let size = entry.size();
        total_uncompressed += size;
        if size > MAX_ZIP_ENTRY_UNCOMPRESSED_SIZE {
            return Err(format!(
                "zip entry {} exceeds maximum uncompressed size of {MAX_ZIP_ENTRY_UNCOMPRESSED_SIZE} bytes",
                entry.name()
            ));
        }
        if total_uncompressed > MAX_ZIP_TOTAL_UNCOMPRESSED_SIZE {
            return Err(format!(
                "zip archive exceeds maximum total uncompressed size of {MAX_ZIP_TOTAL_UNCOMPRESSED_SIZE} bytes"
            ));
        }
    }

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("could not read zip entry: {e}"))?;
        let entry_path = entry
            .enclosed_name()
            .ok_or_else(|| format!("rejecting malicious zip entry path: {:?}", entry.name()))?;

        let out_path = destination.join(&entry_path);
        if entry.is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("could not create directory {}: {e}", out_path.display()))?;
        } else if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("could not create directory {}: {e}", parent.display()))?;
            let mut out_file = fs::File::create(&out_path)
                .map_err(|e| format!("could not create file {}: {e}", out_path.display()))?;
            std::io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("could not write file {}: {e}", out_path.display()))?;
        }
    }

    Ok(())
}

fn validate_zip_entry_path(path: &std::path::Path) -> Result<(), String> {
    let components: Vec<_> = path.components().collect();
    for component in &components {
        match component {
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(format!(
                    "rejecting absolute path in zip archive: {}",
                    path.display()
                ));
            }
            std::path::Component::ParentDir => {
                return Err(format!(
                    "rejecting parent traversal in zip archive: {}",
                    path.display()
                ));
            }
            _ => {}
        }
    }
    if components.is_empty() {
        return Err("empty path in zip archive".to_string());
    }
    Ok(())
}

#[cfg(not(windows))]
fn install_binary(source: &Path, destination: &Path, work_dir: &Path) -> Result<(), String> {
    fs::set_permissions(
        source,
        fs::metadata(destination)
            .map_err(|e| format!("could not inspect {}: {e}", destination.display()))?
            .permissions(),
    )
    .map_err(|e| format!("could not preserve executable permissions: {e}"))?;

    // Preserve the old binary as a rollback target before replacement.
    let backup_path = destination.with_extension("bak");
    if destination.exists() {
        fs::copy(destination, &backup_path).map_err(|e| {
            format!(
                "could not backup old binary to {}: {e}",
                backup_path.display()
            )
        })?;
    }

    fs::rename(source, destination).map_err(|e| {
        // Attempt to restore old binary if rename fails.
        if backup_path.exists() {
            let _ = fs::copy(&backup_path, destination);
        }
        format!(
            "could not replace {}: {e}. You may need permission to write the installation directory",
            destination.display()
        )
    })?;

    // Clean up the backup on successful replacement.
    let _ = fs::remove_file(&backup_path);
    fs::remove_dir_all(work_dir).ok();
    Ok(())
}

#[cfg(windows)]
fn install_binary(source: &Path, destination: &Path, _work_dir: &Path) -> Result<(), String> {
    let source = powershell_quote(source);
    let destination = powershell_quote(destination);
    let script = format!(
        "$pid_to_wait={}; while (Get-Process -Id $pid_to_wait -ErrorAction SilentlyContinue) {{ Start-Sleep -Milliseconds 100 }}; Move-Item -LiteralPath '{}' -Destination '{}' -Force",
        std::process::id(),
        source,
        destination
    );
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .spawn()
        .map_err(|e| format!("could not schedule the Windows executable replacement: {e}"))?;
    Ok(())
}

#[cfg(windows)]
fn powershell_quote(path: &Path) -> String {
    path.display().to_string().replace('\'', "''")
}

fn platform_target() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        (os, arch) => Err(format!(
            "GitHub release updates are not available for {arch}-{os}"
        )),
    }
}

fn run_status(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with status {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_release_version() {
        let release = GitHubRelease {
            tag_name: "v1.4.0".to_owned(),
            assets: Vec::new(),
        };
        assert_eq!(release.version().unwrap(), Version::new(1, 4, 0));
    }

    #[test]
    fn verifies_checksum_manifest() {
        let dir = std::env::temp_dir().join(format!("snp-update-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("archive.tar.gz");
        fs::write(&file, b"test archive").unwrap();
        let checksum = format!(
            "{}  archive.tar.gz\n",
            hex_digest(Sha256::digest(b"test archive").as_ref())
        );
        verify_checksum(checksum.as_bytes(), "archive.tar.gz", &file).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recognizes_cargo_install_path() {
        let package = CLIENT;
        let cargo_bin = PathBuf::from("/home/test/.cargo/bin");
        let executable = cargo_bin.join("snp");
        assert_eq!(
            detect_install_method_with_prefixes(&executable, &package, None, Some(&cargo_bin)),
            InstallMethod::Cargo
        );
    }

    #[test]
    fn recognizes_homebrew_install_path() {
        let package = CLIENT;
        let brew_prefix = PathBuf::from("/opt/homebrew/Cellar/snip-it/1.3.1");
        let executable = brew_prefix.join("bin/snp");
        assert_eq!(
            detect_install_method_with_prefixes(&executable, &package, Some(&brew_prefix), None),
            InstallMethod::Homebrew
        );
    }

    #[test]
    fn falls_back_to_github_for_standalone_binary() {
        let package = CLIENT;
        let executable = PathBuf::from("/usr/local/bin/snp");
        assert_eq!(
            detect_install_method_with_prefixes(&executable, &package, None, None),
            InstallMethod::GitHubRelease
        );
    }

    #[test]
    fn does_not_replace_a_source_build() {
        assert!(is_source_build_path(Path::new(
            "/work/snip-it/target/release/snp"
        )));
        assert!(!is_source_build_path(Path::new("/usr/local/bin/snp")));
    }

    fn detect_install_method_with_prefixes(
        executable: &Path,
        _package: &Package,
        brew_prefix: Option<&Path>,
        cargo_bin: Option<&Path>,
    ) -> InstallMethod {
        if brew_prefix.is_some_and(|prefix| executable.starts_with(prefix)) {
            return InstallMethod::Homebrew;
        }
        if cargo_bin.is_some_and(|prefix| executable.starts_with(prefix)) {
            return InstallMethod::Cargo;
        }
        InstallMethod::GitHubRelease
    }
}
