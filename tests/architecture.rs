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

/// The source of every Rust file under `dir`, with its path.
fn sources_under(dir: &Path) -> Vec<(String, String)> {
    rust_files_under(dir)
        .into_iter()
        .map(|path| {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
            (path.display().to_string(), source)
        })
        .collect()
}

/// The source of every Rust file under `src/`, with its module name.
fn client_crate_sources() -> Vec<(String, String)> {
    sources_under(&src_dir())
}

/// The source of every Rust file under `snip-sync/src/`, with its path.
fn server_crate_sources() -> Vec<(String, String)> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    sources_under(&Path::new(&manifest_dir).join("snip-sync").join("src"))
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

/// The source with every run of whitespace collapsed to a single space and
/// every non-ASCII character replaced by one.
///
/// The cast syntax is ASCII and the scanner works on bytes, so dropping the
/// rest keeps every byte offset a character boundary.
fn flatten_whitespace(source: &str) -> Vec<u8> {
    let mut flattened: Vec<u8> = Vec::with_capacity(source.len());
    let mut pending_space = false;
    for character in source.chars() {
        if character.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space {
            flattened.push(b' ');
            pending_space = false;
        }
        if character.is_ascii() {
            flattened.push(character as u8);
        } else {
            flattened.push(b' ');
        }
    }
    flattened
}

/// The first index of `needle` in `haystack`.
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The last index of `byte` in `haystack`.
fn rfind_byte(haystack: &[u8], byte: u8) -> Option<usize> {
    haystack.iter().rposition(|candidate| *candidate == byte)
}

/// `text` without its leading ASCII whitespace.
fn trim_ascii_start(text: &[u8]) -> &[u8] {
    let start = text
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(text.len());
    &text[start..]
}

/// `text` without its trailing ASCII whitespace.
fn trim_ascii_end(text: &[u8]) -> &[u8] {
    let end = text
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map(|index| index + 1)
        .unwrap_or(0);
    &text[..end]
}

/// `text` without a leading `mut` keyword, when it is one.
fn strip_mut_keyword(text: &[u8]) -> &[u8] {
    let Some(rest) = text.strip_prefix(b"mut".as_slice()) else {
        return text;
    };
    // `mut` is a keyword only when the next byte cannot continue an identifier.
    match rest.first() {
        Some(byte) if byte.is_ascii_alphanumeric() || *byte == b'_' => text,
        _ => trim_ascii_start(rest),
    }
}

/// Whether `segment` is one path segment of an identifier.
fn is_path_segment(segment: &[u8]) -> bool {
    !segment.is_empty()
        && segment
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        && !segment[0].is_ascii_digit()
}

/// The end of the type expression that starts at `start` in `code`.
///
/// A cast target is a type, so it ends at the first token that cannot
/// continue one. Angle brackets are tracked, so a generic argument list
/// (`::<A, B>`) does not end the target early, and a space is only part of
/// the target when it is not the separator of the next link in the chain.
fn type_expression_end(code: &[u8], start: usize) -> usize {
    let mut index = start;
    let mut angle_depth = 0usize;
    while index < code.len() {
        match code[index] {
            b'<' => {
                angle_depth += 1;
                index += 1;
            }
            b'>' => {
                if angle_depth == 0 {
                    break;
                }
                angle_depth -= 1;
                index += 1;
            }
            b',' if angle_depth == 0 => break,
            b')' | b']' | b'}' | b';' | b'=' => break,
            b' ' | b'\t' if angle_depth == 0 => {
                let lookahead = trim_ascii_start(&code[index..]);
                if lookahead.starts_with(b"as ") {
                    break;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    index
}

/// Whether `text` ends with a reference expression: `&value`, `&mut value`,
/// or a field path rooted at one (`&mut self.value`).
///
/// The reference must be the last expression of `text`, so an `&` inside a
/// call's argument list does not count: `f(&mut value)` ends with `)`, not
/// with the reference.
fn ends_with_reference(text: &[u8]) -> bool {
    let text = trim_ascii_end(text);
    let Some(index) = rfind_byte(text, b'&') else {
        return false;
    };
    let after = strip_mut_keyword(trim_ascii_start(&text[index + 1..]));
    let mut segments = after.split(|byte| *byte == b'.');
    match segments.next() {
        Some(first) if is_path_segment(first) => segments.all(is_path_segment),
        _ => false,
    }
}

/// The casts in `source` whose final link takes a reference as its operand.
///
/// A reference cannot be cast straight to `*mut c_void`: the compiler rejects
/// it with `E0606`, because the pointee sizes differ. The macOS-only start
/// token reader in `src/record_lock.rs` shipped exactly that cast, and only
/// the macOS CI job could see it — the Linux gate never compiles the code
/// that carries it, so the error reached `main` unnoticed.
///
/// The safe form casts the reference to an explicitly typed pointer first
/// (`&mut value as *mut T as *mut c_void`), so the final link of a chain
/// takes a pointer, never a reference.
fn reference_to_raw_pointer_casts(source: &str) -> Vec<String> {
    let code = flatten_whitespace(&code_without_comments(source));
    let mut violations = Vec::new();
    let mut search_from = 0usize;
    let mut previous_target: Option<&[u8]> = None;
    while let Some(relative) = find_bytes(&code[search_from..], b" as ") {
        let cast = search_from + relative;
        let target_start = cast + " as ".len();
        let target_end = type_expression_end(&code, target_start);
        let target = &code[target_start..target_end];
        // A chain continues when another `as` follows the target.
        let is_final_link = !trim_ascii_start(&code[target_end..]).starts_with(b"as ");
        if is_final_link {
            // The operand of the final link is the previous link's target, or
            // — for a single-link chain — the expression before the cast.
            let operand = match previous_target {
                Some(previous) => previous,
                None => {
                    let start = rfind_byte(&code[..cast], b';').map_or(0, |index| index + 1);
                    &code[start..cast]
                }
            };
            if ends_with_reference(operand) {
                violations.push(format!(
                    "`{} as {}`",
                    String::from_utf8_lossy(trim_ascii_end(operand)),
                    String::from_utf8_lossy(trim_ascii_end(target))
                ));
            }
        }
        previous_target = if is_final_link { None } else { Some(target) };
        search_from = target_end.max(cast + " as ".len());
    }
    violations
}

/// A reference must never be the operand of the final link of a raw-pointer
/// cast: the compiler rejects that cast when the target is `c_void`, and the
/// error only surfaces on the platform that compiles the code.
#[test]
fn no_reference_is_cast_straight_to_a_raw_pointer() {
    let mut violations = Vec::new();
    for (name, source) in client_crate_sources()
        .into_iter()
        .chain(server_crate_sources())
    {
        for violation in reference_to_raw_pointer_casts(&source) {
            violations.push(format!("{name}: {violation}"));
        }
    }
    assert!(
        violations.is_empty(),
        "a reference must not be cast straight to a raw pointer:\n{}",
        violations.join("\n")
    );
}

/// A guard that cannot fail proves nothing, so the detector is exercised
/// against the cast that broke the macOS build: it must be reported, and the
/// safe two-step form must not be.
#[test]
fn reference_to_raw_pointer_cast_detector_reports_the_broken_cast() {
    let broken = r#"
fn broken(info: &mut proc_bsdinfo) {
    let ret = unsafe {
        proc_pidinfo(
            pid as c_int,
            PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _,
            std::mem::size_of::<proc_bsdinfo>() as c_int,
        )
    };
}
"#;
    let violations = reference_to_raw_pointer_casts(broken);
    assert_eq!(
        violations.len(),
        1,
        "the detector must report the broken cast: {violations:?}"
    );

    let safe = r#"
fn safe(info: &mut proc_bsdinfo) {
    let ret = unsafe {
        proc_pidinfo(
            pid as c_int,
            PROC_PIDTBSDINFO,
            0,
            &mut info as *mut proc_bsdinfo as *mut c_void,
            std::mem::size_of::<proc_bsdinfo>() as c_int,
        )
    };
}
"#;
    assert!(
        reference_to_raw_pointer_casts(safe).is_empty(),
        "the detector must not report the safe two-step cast"
    );

    // A reference that is not the last expression of the operand is not a
    // reference operand: the call's result is what gets cast.
    let call_result = r#"
fn call_result(info: &mut proc_bsdinfo) {
    let ret = unsafe { borrow(info) as *mut _ };
}
"#;
    assert!(
        reference_to_raw_pointer_casts(call_result).is_empty(),
        "the detector must not report a cast of a call's result"
    );
}
