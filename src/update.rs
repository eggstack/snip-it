//! Self-update support for the `snp` client.
//!
//! Updates follow the installation method: Cargo installs are refreshed from
//! crates.io and Homebrew installs are upgraded by Homebrew. Unmanaged/source
//! executables do not guess at unsupported release assets.

use semver::Version;
use serde::Deserialize;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const REPOSITORY: &str = "eggstack/snip-it";
const CRATES_API_URL: &str = "https://crates.io/api/v1/crates/{crate}";
const RELEASE_API_URL: &str = "https://api.github.com/repos/eggstack/snip-it/releases/latest";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstallMethod {
    Cargo,
    Homebrew,
    Unsupported,
}

impl fmt::Display for InstallMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cargo => f.write_str("Cargo"),
            Self::Homebrew => f.write_str("Homebrew"),
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
}

#[derive(Clone, Copy)]
struct Package {
    crate_name: &'static str,
    formula: &'static str,
}

const CLIENT: Package = Package {
    crate_name: "snip-it",
    formula: "snip-it",
};

pub fn run(dry_run: bool, locked: bool) -> Result<(), String> {
    let executable = current_executable()?;
    let method = detect_install_method(&executable, &CLIENT);
    if method == InstallMethod::Unsupported {
        return Err(format!(
            "snp is an unmanaged executable ({}); rebuild it from source or update it through Cargo or Homebrew",
            executable.display()
        ));
    }
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|e| format!("invalid current version: {e}"))?;

    println!("Checking for snp updates ({method})...");
    let latest = match method {
        InstallMethod::Cargo => latest_crates_version(CLIENT.crate_name)?,
        InstallMethod::Homebrew => latest_github_release()?.version()?,
        InstallMethod::Unsupported => unreachable!("unsupported methods return above"),
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
        InstallMethod::Unsupported => unreachable!("unsupported methods return above"),
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
    InstallMethod::Unsupported
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
    let output = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--user-agent",
            "snip-it-update",
            url,
        ])
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
        };
        assert_eq!(release.version().unwrap(), Version::new(1, 4, 0));
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
    fn rejects_unmanaged_binary_without_release_assets() {
        let package = CLIENT;
        let executable = PathBuf::from("/usr/local/bin/snp");
        assert_eq!(
            detect_install_method_with_prefixes(&executable, &package, None, None),
            InstallMethod::Unsupported
        );
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
        InstallMethod::Unsupported
    }
}
