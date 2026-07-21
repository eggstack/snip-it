mod support;

use std::process::Command;

#[test]
fn test_cargo_package_dry_run() {
    let output = Command::new("cargo")
        .args(["package", "--list", "--allow-dirty"])
        .output()
        .expect("failed to run cargo package --list");
    assert!(
        output.status.success(),
        "cargo package --list must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("src/main.rs"),
        "package must include src/main.rs"
    );
    assert!(
        stdout.contains("Cargo.toml"),
        "package must include Cargo.toml"
    );
}

#[test]
fn test_binary_name_is_snp() {
    let output = Command::new(env!("CARGO_BIN_EXE_snp"))
        .arg("--version")
        .output()
        .expect("failed to run snp --version");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("snp"),
        "binary version must mention 'snp', got: {stdout}"
    );
}

#[test]
fn test_binary_help_mentions_all_subcommands() {
    let output = Command::new(env!("CARGO_BIN_EXE_snp"))
        .arg("--help")
        .output()
        .expect("failed to run snp --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for subcmd in &[
        "new", "list", "run", "clip", "select", "search", "edit", "sync",
    ] {
        assert!(
            stdout.contains(subcmd),
            "help output must mention '{subcmd}'"
        );
    }
}

#[test]
fn test_release_binary_has_no_debug_assertions() {
    let output = Command::new(env!("CARGO_BIN_EXE_snp"))
        .arg("--version")
        .output()
        .expect("failed to run snp --version");
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("thread 'main' panicked"),
        "binary must not panic on --version"
    );
}

#[test]
fn test_binary_links_successfully() {
    let output = Command::new(env!("CARGO_BIN_EXE_snp"))
        .arg("--help")
        .output()
        .expect("failed to run snp --help");
    assert!(
        output.status.success(),
        "binary must link and run successfully"
    );
}
