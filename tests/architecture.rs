//! Architecture boundary tests.
//!
//! Verifies that the layered dependency rules documented in `docs/LOGICAL_LAYERS.md`
//! are enforced at the source level. These are source-scanning tests that catch
//! regressions where a lower layer gains a dependency on a higher layer.

use std::fs;
use std::path::Path;

/// Modules that belong to the **Domain/Core** layer.
/// These must not depend on application, CLI, sync-client, or platform modules.
///
/// Entries may be files (`sort.rs`) or module directories (`library/` for
/// `src/library/mod.rs` + siblings after Plan 010).
const CORE_MODULES: &[&str] = &[
    "library/",
    "sort.rs",
    "output.rs",
    "usage.rs",
    "diagnostics.rs",
];

/// Modules that belong to the **Sync-Client** layer.
/// These must not depend on application modules (commands, ui, logging, etc.).
///
/// Note: `config/` is intentionally not listed. It carries a documented
/// cross-layer call (`save_sync_settings` → `crate::clipboard::...`) that
/// predates Plan 010; adding it here would fail on that known exception.
const SYNC_CLIENT_MODULES: &[&str] = &["sync.rs", "sync_commands.rs", "encryption.rs"];

/// Modules that are forbidden imports from the Core layer.
/// Note: `crate::config` is allowed because core modules use its TOML caching
/// helpers (`cached_read_toml`, `invalidate_toml_cache`) which are pure
/// persistence functions. The sync/keychain parts of config are not imported.
const FORBIDDEN_FROM_CORE: &[&str] = &[
    "crate::commands",
    "crate::ui",
    "crate::logging",
    "crate::auto_sync",
    "crate::clipboard",
    "crate::sync_commands",
    "crate::sync",
];

/// Modules that are forbidden imports from the Sync-Client layer.
const FORBIDDEN_FROM_SYNC_CLIENT: &[&str] = &[
    "crate::commands",
    "crate::ui",
    "crate::logging",
    "crate::auto_sync",
    "crate::clipboard",
];

fn src_dir() -> std::path::PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    Path::new(&manifest_dir).join("src")
}

fn read_source(module_name: &str) -> String {
    let path = src_dir().join(module_name);
    if path.is_dir() {
        let mut combined = String::new();
        let mut entries: Vec<_> = fs::read_dir(&path)
            .unwrap_or_else(|e| panic!("Failed to list {}: {e}", path.display()))
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "rs"))
            .collect();
        entries.sort();
        for entry in entries {
            let content = fs::read_to_string(&entry)
                .unwrap_or_else(|e| panic!("Failed to read {}: {e}", entry.display()));
            combined.push_str(&format!("\n// ── {} ──\n", entry.display()));
            combined.push_str(&content);
        }
        return combined;
    }
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("Failed to read {}: {}", path.display(), e))
}

fn check_forbidden_imports(module_name: &str, forbidden: &[&str], layer_name: &str) -> Vec<String> {
    let source = read_source(module_name);
    let mut violations = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        // Skip comments and doc comments
        if trimmed.starts_with("//") || trimmed.starts_with("//!") {
            continue;
        }
        for forbidden_path in forbidden {
            if trimmed.contains(forbidden_path) {
                violations.push(format!(
                    "{}:{} imports {} (forbidden from {} layer)",
                    module_name, trimmed, forbidden_path, layer_name,
                ));
            }
        }
    }
    violations
}

#[test]
fn core_modules_do_not_depend_on_application() {
    let mut all_violations = Vec::new();
    for module in CORE_MODULES {
        let violations = check_forbidden_imports(module, FORBIDDEN_FROM_CORE, "core");
        all_violations.extend(violations);
    }
    assert!(
        all_violations.is_empty(),
        "Core layer modules must not depend on application/sync-client modules:\n{}",
        all_violations.join("\n")
    );
}

#[test]
fn sync_client_modules_do_not_depend_on_application() {
    let mut all_violations = Vec::new();
    for module in SYNC_CLIENT_MODULES {
        let violations = check_forbidden_imports(module, FORBIDDEN_FROM_SYNC_CLIENT, "sync-client");
        all_violations.extend(violations);
    }
    assert!(
        all_violations.is_empty(),
        "Sync-client layer modules must not depend on application modules:\n{}",
        all_violations.join("\n")
    );
}

#[test]
fn internal_modules_are_not_pub_in_lib_rs() {
    let lib_rs = read_source("lib.rs");
    // Modules that should be pub(crate) — not needed by integration tests
    let pub_crate_modules = [
        "clipboard",
        "diagnostics",
        "encryption",
        "library",
        "output",
        "status_snapshot",
        "sync_commands",
        "utils",
    ];
    let mut violations = Vec::new();
    for module in &pub_crate_modules {
        let pub_pattern = format!("pub mod {};", module);
        let pub_crate_pattern = format!("pub(crate) mod {};", module);
        if lib_rs.contains(&pub_pattern) && !lib_rs.contains(&pub_crate_pattern) {
            violations.push(format!(
                "Module '{}' should be pub(crate) but is still pub",
                module
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "Visibility violations found:\n{}",
        violations.join("\n")
    );
}

/// Plan 014: `snp` self-update HTTP is handled in-process through
/// eggfetch-core, so `src/update.rs` must not shell out to an external
/// `curl` executable (the updater works with no `curl` on PATH). The
/// remaining subprocess uses there are Cargo, Homebrew, candidate
/// verification, and Windows replacement.
///
/// `snip-sync/src/update.rs` is intentionally excluded: it retains the
/// `curl` adapter as a measured footprint tradeoff (see its module docs),
/// so a source scan must not flag it.
#[test]
fn snp_updater_does_not_shell_out_to_curl() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let path = Path::new(&manifest_dir).join("src/update.rs");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
    let mut violations = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("//!") {
            continue;
        }
        if trimmed.contains("\"curl\"") || trimmed.contains("curl_protocol") {
            violations.push(format!("src/update.rs:{}: {trimmed}", index + 1));
        }
    }
    assert!(
        violations.is_empty(),
        "snp updater must not shell out to curl:\n{}",
        violations.join("\n")
    );
}

/// Plan 016: the `snp` updater pins `eggfetch-core =0.1.7` on the lean
/// `standard-http1 + redirects` profile, delegates redirect traversal to
/// `RedirectPolicy::strict` and the logical request/body deadline to native
/// `Timeout.total`. The superseded Plan 014/015 machinery — the manual
/// redirect loop and the duplicate outer Tokio timeout — must not return.
#[test]
fn snp_updater_pins_lean_eggfetch_profile() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let path = Path::new(&manifest_dir).join("Cargo.toml");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
    assert!(
        source.contains("eggfetch-core = { version = \"=0.1.7\""),
        "snp updater must pin eggfetch-core =0.1.7"
    );
    assert!(
        source.contains("\"standard-http1\""),
        "snp updater must use the lean standard-http1 profile, not the broad http1 alias"
    );
    assert!(
        source.contains("\"redirects\""),
        "snp updater must enable the redirects feature for strict redirect handling"
    );
}

#[test]
fn snp_updater_delegates_redirects_and_total_timeout() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let path = Path::new(&manifest_dir).join("src/update.rs");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
    assert!(
        source.contains("RedirectPolicy::strict"),
        "snp updater must delegate redirect traversal to RedirectPolicy::strict"
    );
    let mut violations = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("//!") {
            continue;
        }
        if trimmed.contains("tokio::time::timeout") {
            violations.push(format!("src/update.rs:{}: {trimmed}", index + 1));
        }
        for helper in [
            "fn safe_get",
            "fn is_redirect_status",
            "fn redirect_location",
            "fn redirect_target",
            "follow_redirects(false)",
        ] {
            if trimmed.contains(helper) {
                violations.push(format!("src/update.rs:{}: {trimmed}", index + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "snp updater must not retain the manual redirect loop or outer Tokio timeout:\n{}",
        violations.join("\n")
    );
}
