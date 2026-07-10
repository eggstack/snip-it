//! Self-update support for the `snip-sync` Cargo installation.

use semver::Version;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const CRATE_NAME: &str = "snip-sync";
const CRATES_API_URL: &str = "https://crates.io/api/v1/crates/{crate}";

#[derive(Debug, Deserialize)]
struct CratesResponse {
    #[serde(rename = "crate")]
    crate_info: CrateInfo,
}

#[derive(Debug, Deserialize)]
struct CrateInfo {
    max_version: String,
}

pub fn run(dry_run: bool, locked: bool) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|e| format!("could not locate the running executable: {e}"))?;
    let executable = std::fs::canonicalize(&executable).unwrap_or(executable);
    if !is_cargo_install(&executable) {
        return Err(format!(
            "this snip-sync executable is not installed by Cargo ({}); update the package that manages it, or install it with `cargo install {CRATE_NAME}`",
            executable.display()
        ));
    }

    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|e| format!("invalid current version: {e}"))?;
    let latest = latest_crates_version()?;

    println!("Checking for snip-sync updates (Cargo)...");
    if latest <= current {
        println!("snip-sync {current} is already up to date.");
        return Ok(());
    }

    println!("Update available: snip-sync {current} -> {latest}");
    if dry_run {
        println!("Dry run: no changes were made.");
        return Ok(());
    }

    let mut args = vec!["install", CRATE_NAME];
    if locked {
        args.push("--locked");
    }
    println!("Running: cargo {}", args.join(" "));
    let status = Command::new("cargo")
        .args(&args)
        .status()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if !status.success() {
        return Err(format!("cargo exited with status {status}"));
    }
    println!("Update complete.");
    Ok(())
}

fn is_cargo_install(executable: &Path) -> bool {
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

fn cargo_bin_dir() -> Option<PathBuf> {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".cargo"))
        })?;
    let cargo_bin = cargo_home.join("bin");
    Some(std::fs::canonicalize(&cargo_bin).unwrap_or(cargo_bin))
}

fn latest_crates_version() -> Result<Version, String> {
    let template =
        std::env::var("SNIP_UPDATE_CRATES_API_URL").unwrap_or_else(|_| CRATES_API_URL.to_owned());
    let url = template.replace("{crate}", CRATE_NAME);
    let output = Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location", "--user-agent", "snip-it-update", &url])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run curl: {e}. Install curl or update manually with `cargo install {CRATE_NAME}`"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if detail.is_empty() {
            format!("crates.io request failed with status {}", output.status)
        } else {
            format!("crates.io request failed: {detail}")
        });
    }

    let response: CratesResponse = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("could not parse crates.io response: {e}"))?;
    Version::parse(&response.crate_info.max_version).map_err(|e| {
        format!(
            "crates.io returned invalid version {:?}: {e}",
            response.crate_info.max_version
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_cargo_bin_path() {
        let cargo_bin = cargo_bin_dir().expect("test environment should have a home directory");
        assert!(is_cargo_install(&cargo_bin.join("snip-sync")));
    }

    #[test]
    fn rejects_unmanaged_path() {
        assert!(!is_cargo_install(Path::new("/usr/local/bin/snip-sync")));
    }
}
