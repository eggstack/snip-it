//! Tests for stable snippet and library identity contract.
//!
//! Verifies identity behavior across the full lifecycle.

use snip_it::{LibraryConfig, LibraryMeta, Snippet, Snippets, load_library, save_library};
use tempfile::TempDir;

fn temp_library_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("test_library.toml")
}

fn make_snippet(id: &str, desc: &str, cmd: &str) -> Snippet {
    Snippet {
        id: id.to_string(),
        description: desc.to_string(),
        command: cmd.to_string(),
        output: String::new(),
        tags: vec![],
        folders: vec![],
        favorite: false,
        created_at: 1000,
        updated_at: 1000,
        device_id: String::new(),
        deleted: false,
    }
}

#[test]
fn test_edit_retains_id() {
    let dir = TempDir::new().unwrap();
    let path = temp_library_path(&dir);
    let snippets = Snippets {
        snippets: vec![make_snippet("stable-id", "original", "echo old")],
        folders: vec![],
    };
    save_library(&path, &snippets).unwrap();

    let mut loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets[0].id, "stable-id");

    loaded.snippets[0].command = "echo updated".to_string();
    loaded.snippets[0].description = "updated desc".to_string();
    loaded.snippets[0].updated_at = 2000;
    save_library(&path, &loaded).unwrap();

    let reloaded = load_library(&path).unwrap();
    assert_eq!(reloaded.snippets[0].id, "stable-id");
    assert_eq!(reloaded.snippets[0].command, "echo updated");
    assert_eq!(reloaded.snippets[0].description, "updated desc");
}

#[test]
fn test_move_retains_id() {
    let dir = TempDir::new().unwrap();
    let lib_a = dir.path().join("lib_a.toml");
    let lib_b = dir.path().join("lib_b.toml");

    let snippets_a = Snippets {
        snippets: vec![make_snippet("movable-id", "portable", "echo move")],
        folders: vec![],
    };
    save_library(&lib_a, &snippets_a).unwrap();

    let mut from = load_library(&lib_a).unwrap();
    let snippet = from.snippets.remove(0);
    let moved_id = snippet.id.clone();
    assert_eq!(moved_id, "movable-id");

    let mut to = load_library(&lib_b).unwrap();
    to.snippets.push(snippet);
    save_library(&lib_b, &to).unwrap();

    from.snippets.clear();
    save_library(&lib_a, &from).unwrap();

    let loaded_b = load_library(&lib_b).unwrap();
    assert_eq!(loaded_b.snippets.len(), 1);
    assert_eq!(loaded_b.snippets[0].id, "movable-id");
    assert_eq!(loaded_b.snippets[0].command, "echo move");
}

#[test]
fn test_native_export_includes_id() {
    let dir = TempDir::new().unwrap();
    let path = temp_library_path(&dir);
    let snippets = Snippets {
        snippets: vec![make_snippet("export-id", "exportable", "echo export")],
        folders: vec![],
    };
    save_library(&path, &snippets).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("export-id"),
        "Exported TOML must include the snippet ID"
    );
    assert!(
        content.contains("exportable"),
        "Exported TOML must include description"
    );
}

#[test]
fn test_import_without_id_gets_new() {
    let dir = TempDir::new().unwrap();
    let path = temp_library_path(&dir);

    let toml_content = r#"
[[snippets]]
description = "imported snippet"
command = "echo imported"
"#;
    std::fs::write(&path, toml_content).unwrap();

    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets.len(), 1);
    assert!(
        !loaded.snippets[0].id.is_empty(),
        "Imported snippet without ID should get a new ID"
    );
    assert_eq!(loaded.snippets[0].description, "imported snippet");
}

#[test]
fn test_dedup_same_id_same_content() {
    let dir = TempDir::new().unwrap();
    let path = temp_library_path(&dir);

    let toml_content = r#"
[[snippets]]
id = "dup-id"
description = "first"
command = "echo one"

[[snippets]]
id = "dup-id"
description = "second"
command = "echo two"
"#;
    std::fs::write(&path, toml_content).unwrap();

    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets.len(), 2);
    assert_ne!(
        loaded.snippets[0].id, loaded.snippets[1].id,
        "Duplicate IDs must be resolved on load"
    );
}

#[test]
fn test_same_id_diff_content_new_id() {
    let dir = TempDir::new().unwrap();
    let path = temp_library_path(&dir);

    let toml_content = r#"
[[snippets]]
id = "shared-id"
description = "version A"
command = "echo alpha"

[[snippets]]
id = "shared-id"
description = "version B"
command = "echo beta"
"#;
    std::fs::write(&path, toml_content).unwrap();

    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets.len(), 2);
    assert_ne!(loaded.snippets[0].id, loaded.snippets[1].id);
}

#[test]
fn test_diff_id_same_content_both_kept() {
    let dir = TempDir::new().unwrap();
    let path = temp_library_path(&dir);

    let toml_content = r#"
[[snippets]]
id = "id-alpha"
description = "same"
command = "echo same"

[[snippets]]
id = "id-beta"
description = "same"
command = "echo same"
"#;
    std::fs::write(&path, toml_content).unwrap();

    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets.len(), 2);
    assert_eq!(loaded.snippets[0].id, "id-alpha");
    assert_eq!(loaded.snippets[1].id, "id-beta");
}

#[test]
fn test_delete_preserves_tombstone() {
    let dir = TempDir::new().unwrap();
    let path = temp_library_path(&dir);
    let snippets = Snippets {
        snippets: vec![
            make_snippet("keep-id", "kept", "echo keep"),
            make_snippet("deleted-id", "deleted", "echo deleted"),
        ],
        folders: vec![],
    };
    save_library(&path, &snippets).unwrap();

    let mut loaded = load_library(&path).unwrap();
    loaded.snippets[1].deleted = true;
    save_library(&path, &loaded).unwrap();

    let reloaded = load_library(&path).unwrap();
    assert_eq!(reloaded.snippets.len(), 2);

    let deleted = reloaded
        .snippets
        .iter()
        .find(|s| s.id == "deleted-id")
        .unwrap();
    assert!(
        deleted.deleted,
        "Tombstone flag must persist through save/load"
    );

    let kept = reloaded
        .snippets
        .iter()
        .find(|s| s.id == "keep-id")
        .unwrap();
    assert!(!kept.deleted);
}

#[test]
fn test_recreate_gets_new_id() {
    let dir = TempDir::new().unwrap();
    let path = temp_library_path(&dir);
    let snippets = Snippets {
        snippets: vec![make_snippet("original-id", "original", "echo original")],
        folders: vec![],
    };
    save_library(&path, &snippets).unwrap();

    let mut loaded = load_library(&path).unwrap();
    let old_id = loaded.snippets[0].id.clone();
    loaded.snippets[0].deleted = true;
    save_library(&path, &loaded).unwrap();

    let mut reloaded = load_library(&path).unwrap();
    let new_snippet = make_snippet("", "recreated", "echo recreated");
    reloaded.snippets.push(new_snippet);
    save_library(&path, &reloaded).unwrap();

    let final_lib = load_library(&path).unwrap();
    assert_eq!(final_lib.snippets.len(), 2);

    let new = final_lib
        .snippets
        .iter()
        .find(|s| s.description == "recreated")
        .unwrap();
    assert_ne!(new.id, old_id, "Recreated snippet must get a new ID");
    assert!(!new.id.is_empty());
}

#[test]
fn test_sync_preserves_id() {
    let dir = TempDir::new().unwrap();
    let path = temp_library_path(&dir);
    let snippets = Snippets {
        snippets: vec![make_snippet("sync-stable-id", "synced", "echo sync")],
        folders: vec![],
    };
    save_library(&path, &snippets).unwrap();

    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets[0].id, "sync-stable-id");

    let mut reloaded = load_library(&path).unwrap();
    reloaded.snippets[0].updated_at = 3000;
    save_library(&path, &reloaded).unwrap();

    let final_lib = load_library(&path).unwrap();
    assert_eq!(
        final_lib.snippets[0].id, "sync-stable-id",
        "ID must survive sync round-trip"
    );
}

#[test]
fn test_id_roundtrip_preserves_all_fields() {
    let dir = TempDir::new().unwrap();
    let path = temp_library_path(&dir);

    let original = Snippet {
        id: "full-roundtrip-id".to_string(),
        description: "full test".to_string(),
        command: "echo full".to_string(),
        output: "some output".to_string(),
        tags: vec!["tag1".to_string(), "tag2".to_string()],
        folders: vec!["folder1".to_string()],
        favorite: true,
        created_at: 100,
        updated_at: 200,
        device_id: "device-xyz".to_string(),
        deleted: false,
    };

    let snippets = Snippets {
        snippets: vec![original.clone()],
        folders: vec![],
    };
    save_library(&path, &snippets).unwrap();

    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets.len(), 1);
    let s = &loaded.snippets[0];
    assert_eq!(s.id, "full-roundtrip-id");
    assert_eq!(s.description, "full test");
    assert_eq!(s.command, "echo full");
    assert_eq!(s.output, "some output");
    assert_eq!(s.tags, vec!["tag1", "tag2"]);
    assert_eq!(s.folders, vec!["folder1"]);
    assert!(s.favorite);
    assert_eq!(s.created_at, 100);
    assert_eq!(s.updated_at, 200);
    assert_eq!(s.device_id, "device-xyz");
    assert!(!s.deleted);
}

#[test]
fn test_empty_id_gets_assigned_on_load() {
    let dir = TempDir::new().unwrap();
    let path = temp_library_path(&dir);

    let toml_content = r#"
[[snippets]]
description = "no id snippet"
command = "echo no-id"
"#;
    std::fs::write(&path, toml_content).unwrap();

    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets.len(), 1);
    assert!(
        !loaded.snippets[0].id.is_empty(),
        "Empty ID must be assigned on load"
    );
}

#[test]
fn test_multiple_edits_retain_id() {
    let dir = TempDir::new().unwrap();
    let path = temp_library_path(&dir);
    let snippets = Snippets {
        snippets: vec![make_snippet("multi-edit-id", "v1", "echo v1")],
        folders: vec![],
    };
    save_library(&path, &snippets).unwrap();

    for version in 2..=5 {
        let mut loaded = load_library(&path).unwrap();
        loaded.snippets[0].description = format!("v{version}");
        loaded.snippets[0].command = format!("echo v{version}");
        loaded.snippets[0].updated_at = 100 * version;
        save_library(&path, &loaded).unwrap();
    }

    let final_lib = load_library(&path).unwrap();
    assert_eq!(final_lib.snippets[0].id, "multi-edit-id");
    assert_eq!(final_lib.snippets[0].description, "v5");
}

#[test]
fn test_library_meta_roundtrip() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("libraries.toml");

    let config = LibraryConfig {
        libraries: vec![
            LibraryMeta {
                filename: "work".to_string(),
                library_id: "lib-id-work".to_string(),
                is_primary: true,
                last_sync: Some(1000),
                server_id: Some("server-work".to_string()),
            },
            LibraryMeta {
                filename: "personal".to_string(),
                library_id: "lib-id-personal".to_string(),
                is_primary: false,
                last_sync: None,
                server_id: None,
            },
        ],
        generation: 0,
    };

    let toml_str = toml::to_string_pretty(&config).unwrap();
    std::fs::write(&config_path, &toml_str).unwrap();

    let content = std::fs::read_to_string(&config_path).unwrap();
    let loaded: LibraryConfig = toml::from_str(&content).unwrap();
    assert_eq!(loaded.libraries.len(), 2);
    assert_eq!(loaded.libraries[0].filename, "work");
    assert_eq!(loaded.libraries[0].library_id, "lib-id-work");
    assert!(loaded.libraries[0].is_primary);
    assert_eq!(loaded.libraries[1].filename, "personal");
    assert!(!loaded.libraries[1].is_primary);
}
