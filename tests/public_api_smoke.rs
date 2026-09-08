//! Plan 010: supported public-API smoke test.
//!
//! Proves the documented `snip_it` library surface still resolves after the
//! module-boundary decomposition (`src/library/`, `src/config/`). Every
//! import below comes from the "Supported types and functions" table in
//! `src/lib.rs`. Assertions are intentionally light; behavior is covered by
//! the owning modules' tests. Exact counts prove deterministic shape.

use snip_it::config::{
    AUTO_SYNC_DEBOUNCE_MAX, AutoSyncFailureMode, DEFAULT_SERVER_URL, SyncDirection, SyncSettings,
};
use snip_it::error::{SnipError, SyncFailureKind};
use snip_it::outcome::{CliOutcome, OutputContext, exit_code};
use snip_it::sort::{SnippetSort, SortOptions, rank_snippets};
use snip_it::{
    AtomicWriteOptions, Durability, LibraryConfig, LibraryMeta, Snippet, Snippets, load_library,
};
use tempfile::TempDir;

#[test]
fn supported_root_types_roundtrip_through_toml() {
    let snippet = Snippet {
        id: "smoke-1".to_string(),
        description: "smoke fixture".to_string(),
        command: "echo smoke".to_string(),
        ..Default::default()
    };
    let snippets = Snippets {
        snippets: vec![snippet],
        folders: vec![],
    };
    // In-memory serialization proves field renames/aliases still hold.
    let toml_str = toml::to_string_pretty(&snippets).unwrap();
    assert!(toml_str.contains("[[snippets]]"));
    let parsed: Snippets = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.snippets.len(), 1);
    assert_eq!(parsed.snippets[0].id, "smoke-1");

    let meta = LibraryMeta::new("smoke");
    assert_eq!(meta.filename, "smoke");
    let config = LibraryConfig {
        libraries: vec![meta],
        generation: 7,
    };
    assert_eq!(config.libraries.len(), 1);
    assert_eq!(config.generation, 7);
}

#[test]
fn supported_load_library_reads_isolated_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("smoke.toml");
    std::fs::write(
        &path,
        "[[snippets]]\nid = \"smoke-1\"\ndescription = \"smoke\"\ncommand = \"echo hi\"\n",
    )
    .unwrap();
    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets.len(), 1);
    assert_eq!(loaded.snippets[0].command, "echo hi");
}

#[test]
fn supported_config_sort_outcome_error_types_resolve() {
    let settings = SyncSettings::default();
    assert_eq!(settings.server_url, DEFAULT_SERVER_URL);
    assert_eq!(settings.auto_sync_failure, AutoSyncFailureMode::Warn);
    assert_eq!(settings.sync_direction, SyncDirection::Push);
    assert!(settings.auto_sync_debounce_seconds <= AUTO_SYNC_DEBOUNCE_MAX);

    let opts = SortOptions {
        mode: SnippetSort::Relevance,
        favorites_first: false,
    };
    let snippets = vec![Snippet {
        id: "a".to_string(),
        description: "alpha".to_string(),
        command: "echo a".to_string(),
        ..Default::default()
    }];
    let ranked = rank_snippets(&[0], &snippets, None, None, &opts);
    assert_eq!(ranked, vec![0]);

    assert_eq!(CliOutcome::Success.exit_code(), exit_code::SUCCESS);
    let _ctx = OutputContext::human();

    let err = SnipError::runtime_error("smoke", None);
    assert!(!err.to_string().is_empty());
    let _kind = SyncFailureKind::Timeout;

    let _opts = AtomicWriteOptions::for_durability(Durability::DurableUserData);
}
