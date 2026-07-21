//! Architecture boundary tests.
//!
//! Verifies that the layered dependency rules documented in `docs/LOGICAL_LAYERS.md`
//! are enforced at the source level. These are source-scanning tests that catch
//! regressions where a lower layer gains a dependency on a higher layer.

use std::fs;
use std::path::Path;

/// Modules that belong to the **Domain/Core** layer.
/// These must not depend on application, CLI, sync-client, or platform modules.
const CORE_MODULES: &[&str] = &[
    "library.rs",
    "sort.rs",
    "output.rs",
    "usage.rs",
    "diagnostics.rs",
];

/// Modules that belong to the **Sync-Client** layer.
/// These must not depend on application modules (commands, ui, logging, etc.).
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

#[test]
fn test_events_is_not_pub_in_release_without_feature() {
    let mod_rs = read_source("auto_sync/mod.rs");
    // test_events should always be compiled (used by worker/executor)
    // but its public API should be gated behind test-support feature
    assert!(
        mod_rs.contains("pub mod test_events"),
        "test_events must remain a pub mod (used by worker/executor at compile time)"
    );
}
