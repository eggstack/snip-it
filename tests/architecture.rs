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

/// Every Rust source file under `dir`, recursively, in a stable order.
fn rust_files_under(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let mut entries: Vec<_> = fs::read_dir(&current)
            .unwrap_or_else(|e| panic!("Failed to list {}: {e}", current.display()))
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        entries.sort();
        for entry in entries {
            if entry.is_dir() {
                pending.push(entry);
            } else if entry.extension().is_some_and(|ext| ext == "rs") {
                files.push(entry);
            }
        }
    }
    files.sort();
    files
}

/// Detects a module-level `#![allow(dead_code)]` suppression in `source`.
///
/// `#![...]` is an *inner* attribute: it applies to the whole module, so it
/// hides dead code from the compiler for every item in the file. Only
/// item-level `#[allow(dead_code)]` attributes are permitted, and each of
/// those carries an adjacent comment stating why the item is retained.
///
/// Returns the 1-based line number and the offending line.
fn find_module_level_dead_code_suppression(source: &str) -> Option<(usize, String)> {
    source.lines().enumerate().find_map(|(index, line)| {
        let trimmed = line.trim();
        // `#[...]` is item-level and allowed; only `#![...]` is module-level.
        // A doc comment (`//!` / `///`) never starts with `#![`, so prose that
        // merely quotes an attribute is ignored.
        if !trimmed.starts_with("#![") {
            return None;
        }
        let normalized: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
        normalized
            .contains("allow(dead_code")
            .then(|| (index + 1, trimmed.to_string()))
    })
}

/// Plan 001 (maintenance consolidation): no module may carry a module-level
/// `#![allow(dead_code)]` suppression. A file-wide suppression would hide
/// dead code from the compiler for the whole module, which is exactly how the
/// dead subsystems this plan removes stayed invisible.
#[test]
fn no_module_declares_a_module_level_dead_code_suppression() {
    let mut violations = Vec::new();
    for path in rust_files_under(&src_dir()) {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
        // Generated files are never edited by hand (`scripts/build_themes.py`
        // emits its own module-level suppression); the guard governs
        // hand-written modules only.
        if source
            .lines()
            .take(5)
            .any(|line| line.contains("@generated"))
        {
            continue;
        }
        if let Some((line, text)) = find_module_level_dead_code_suppression(&source) {
            violations.push(format!("{}:{}: {text}", path.display(), line));
        }
    }
    assert!(
        violations.is_empty(),
        "Modules must not declare a module-level `#![allow(dead_code)]`:\n{}",
        violations.join("\n")
    );
}

/// A guard that cannot fail proves nothing, so the detector is exercised
/// against a synthetic module: it must report a module-level suppression and
/// must stay silent for the permitted item-level attribute.
#[test]
fn module_level_dead_code_suppression_detector_fires_on_a_synthetic_module() {
    let module_suppressed = "\
//! A synthetic module.
#![allow(dead_code)]

fn never_called() {}
";
    let detected = find_module_level_dead_code_suppression(module_suppressed);
    assert!(
        detected.is_some(),
        "the detector must report a module-level `#![allow(dead_code)]`"
    );
    let (line, text) = detected.expect("detector reported a suppression");
    assert_eq!(line, 2, "the reported line must point at the suppression");
    assert!(
        text.contains("allow(dead_code)"),
        "the reported line must be the offending line: {text}"
    );

    let item_level = "\
#[allow(dead_code)]
fn never_called() {}
";
    assert!(
        find_module_level_dead_code_suppression(item_level).is_none(),
        "an item-level `#[allow(dead_code)]` is permitted and must not be reported"
    );
}

/// Plan 017 adopted the lean `standard-http1 + redirects` profile on
/// `eggfetch-core =0.2.0`; the Eggstack dependency-refresh M001 later moved
/// the pin to `=0.2.2`. Either way the `snp` updater must stay on the lean
/// profile, delegate redirect traversal to `RedirectPolicy::strict`, and
/// delegate the logical request/body deadline to native `Timeout.total`.
/// The superseded Plan 014/015 machinery — the manual redirect loop and the
/// duplicate outer Tokio timeout — must not return.
#[test]
fn snp_updater_pins_lean_eggfetch_profile() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let path = Path::new(&manifest_dir).join("Cargo.toml");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
    assert!(
        source.contains("eggfetch-core = { version = \"=0.2.2\""),
        "snp updater must pin eggfetch-core =0.2.2"
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

/// The operations that identify the owned-record lock acquisition protocol.
///
/// The protocol is the `create_new` acquisition followed by the record write
/// and the record sync, plus the owner-liveness observation that classifies
/// an existing lock record. The atomic-write primitive and the transaction
/// staging writer use `create_new` + `write_all` + `sync_all` as well, but
/// they never observe a lock owner, so the four markers together are
/// specific to the owned-record lock family.
const OWNED_RECORD_PROTOCOL_MARKERS: &[&str] =
    &["create_new(true)", "write_all(", "sync_all(", "observe("];

/// The source of every Rust file under `src/`, with its module name.
fn client_crate_sources() -> Vec<(String, String)> {
    rust_files_under(&src_dir())
        .into_iter()
        .map(|path| {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
            (path.display().to_string(), source)
        })
        .collect()
}

/// The source with comment-only lines removed, so a marker quoted in a doc
/// comment cannot be mistaken for an implementation.
fn code_without_comments(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether `source` contains the owned-record lock acquisition protocol.
fn contains_owned_record_protocol(source: &str) -> bool {
    let code = code_without_comments(source);
    OWNED_RECORD_PROTOCOL_MARKERS
        .iter()
        .all(|marker| code.contains(marker))
}

/// The modules that contain the owned-record lock acquisition protocol.
fn owned_record_protocol_modules(sources: &[(String, String)]) -> Vec<String> {
    sources
        .iter()
        .filter(|(_, source)| contains_owned_record_protocol(source))
        .map(|(name, _)| name.clone())
        .collect()
}

/// The owned-record lock acquisition protocol must exist in exactly one
/// place in the client crate. A second copy would let the protocol drift
/// away from the shared implementation again.
#[test]
fn owned_record_lock_protocol_exists_in_exactly_one_module() {
    let modules = owned_record_protocol_modules(&client_crate_sources());
    assert_eq!(
        modules.len(),
        1,
        "the owned-record lock acquisition protocol must exist in exactly one module, found: {modules:?}"
    );
}

/// A guard that cannot fail proves nothing, so the detector is exercised
/// against a synthetic second copy of the protocol: it must be reported,
/// and the module count must grow to two.
#[test]
fn owned_record_lock_protocol_detector_reports_a_second_copy() {
    let synthetic = r#"
fn second_copy(path: &Path, content: &str, pid: u32) {
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file.sync_all().unwrap();
    let observed = ProcessIdentity::observe(pid);
}
"#;
    assert!(
        contains_owned_record_protocol(synthetic),
        "the detector must report a synthetic copy of the protocol"
    );

    let mut sources = vec![(
        "synthetic-second-copy.rs".to_string(),
        synthetic.to_string(),
    )];
    sources.extend(client_crate_sources());
    let modules = owned_record_protocol_modules(&sources);
    assert_eq!(
        modules.len(),
        2,
        "the detector must report the synthetic second copy: {modules:?}"
    );
    assert!(
        modules
            .iter()
            .any(|name| name == "synthetic-second-copy.rs"),
        "the detector must report the synthetic second copy: {modules:?}"
    );
}

/// The transient-contention helper must exist in exactly one place: two
/// copies would let the Windows pending-delete handling diverge.
#[test]
fn transient_lock_contention_helper_is_defined_in_exactly_one_module() {
    let definitions: Vec<String> = client_crate_sources()
        .into_iter()
        .filter(|(_, source)| {
            code_without_comments(source).contains("fn is_transient_lock_contention")
        })
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        definitions.len(),
        1,
        "the transient-contention helper must be defined in exactly one module, found: {definitions:?}"
    );
}
