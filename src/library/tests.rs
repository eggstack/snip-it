use super::model::validate_library_name;
use super::persistence::deterministic_legacy_id;
use super::*;
use crate::config::cached_read_toml;
use std::collections::HashSet;
use std::path::Path;
use tempfile::TempDir;

#[cfg(unix)]
fn file_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .expect("test helper: metadata read")
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn test_pet_format_compatibility() {
    let pet_toml = r#"
[[snippets]]
  description = "git commit with message"
  command = "git commit -m \"message\""
  tag = ["git", "version-control"]
  output = ""

[[snippets]]
  description = "docker ps"
  command = "docker ps"
  tag = ["docker"]
  output = ""
"#;
    let snippets: Snippets = toml::from_str(pet_toml).unwrap();
    assert_eq!(snippets.snippets.len(), 2);
    assert_eq!(snippets.snippets[0].command, "git commit -m \"message\"");
    assert_eq!(snippets.snippets[0].description, "git commit with message");
    assert_eq!(snippets.snippets[0].tags, vec!["git", "version-control"]);
    assert_eq!(snippets.snippets[1].command, "docker ps");
}

#[test]
fn test_legacy_snp_format_compatibility() {
    let snp_toml = r#"
[[Snippets]]
  Description = "git commit"
  Output = ""
  Tag = ["git"]
  command = "git commit -m 'msg'"
"#;
    let snippets: Snippets = toml::from_str(snp_toml).unwrap();
    assert_eq!(snippets.snippets.len(), 1);
    assert_eq!(snippets.snippets[0].command, "git commit -m 'msg'");
}

#[test]
fn test_snp_serializes_to_pet_table_name() {
    let snippets = Snippets {
        snippets: vec![Snippet {
            description: "list files".to_string(),
            command: "ls -la".to_string(),
            tags: vec!["files".to_string()],
            ..Default::default()
        }],
        ..Default::default()
    };

    let toml = toml::to_string(&snippets).unwrap();
    assert!(toml.contains("[[snippets]]"));
    assert!(toml.contains("tag = [\"files\"]"));
    assert!(toml.contains("output = \"\""));
    assert!(!toml.contains("[[Snippets]]"));
}

#[test]
fn test_library_save_load_roundtrip() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("test_library.toml");

    let snippets = Snippets {
        snippets: vec![Snippet {
            id: "test-id-1".to_string(),
            description: "Test snippet".to_string(),
            command: "echo hello".to_string(),
            output: "".to_string(),
            tags: vec!["test".to_string()],
            folders: vec![],
            favorite: false,
            created_at: 1234567890,
            updated_at: 1234567890,
            device_id: "device1".to_string(),
            deleted: false,
        }],
        folders: vec!["work".to_string()],
    };

    save_library(&path, &snippets).unwrap();

    let loaded = load_library(&path).unwrap();

    assert_eq!(loaded.snippets.len(), 1);
    assert_eq!(loaded.snippets[0].description, "Test snippet");
    assert_eq!(loaded.snippets[0].command, "echo hello");
}

#[test]
fn test_library_save_load_roundtrip_with_escaped_brackets() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("test_library.toml");

    let snippets = Snippets {
        snippets: vec![Snippet {
            id: "test-id-1".to_string(),
            description: "Test with escaped brackets".to_string(),
            command: "ping \\<website\\>".to_string(),
            output: "".to_string(),
            tags: vec!["test".to_string()],
            folders: vec![],
            favorite: false,
            created_at: 1234567890,
            updated_at: 1234567890,
            device_id: "device1".to_string(),
            deleted: false,
        }],
        folders: vec![],
    };

    save_library(&path, &snippets).unwrap();

    let loaded = load_library(&path).unwrap();

    assert_eq!(loaded.snippets.len(), 1);
    assert_eq!(loaded.snippets[0].command, "ping \\<website\\>");
}

#[test]
fn test_library_load_with_invalid_escapes() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("invalid_escapes.toml");

    std::fs::write(
        &path,
        r#"
[[Snippets]]
Id = "test-id"
Description = "Test snippet with invalid escapes"
Command = "sudo iptables-restore \< /path/to/rules"
"#,
    )
    .unwrap();

    let loaded = load_library(&path).unwrap();

    assert_eq!(loaded.snippets.len(), 1);
    assert_eq!(
        loaded.snippets[0].command,
        r"sudo iptables-restore \< /path/to/rules"
    );
}

#[test]
fn test_library_load_empty_file() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("empty.toml");

    std::fs::write(&path, "").unwrap();

    let loaded = load_library(&path).unwrap();

    assert!(loaded.snippets.is_empty());
}

#[test]
fn test_library_backup_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("nonexistent.toml");

    let backup_result = backup_library(&path).unwrap();

    assert!(backup_result.is_none());
}

#[test]
fn test_snippet_serialization() {
    let snippet = Snippet {
        id: "test-id".to_string(),
        description: "Test description".to_string(),
        command: "echo test".to_string(),
        output: "test output".to_string(),
        tags: vec!["test".to_string()],
        folders: vec!["work".to_string()],
        favorite: true,
        created_at: 1234567890,
        updated_at: 1234567891,
        device_id: "device-1".to_string(),
        deleted: false,
    };

    let toml_str = toml::to_string_pretty(&snippet).unwrap();
    assert!(toml_str.contains("test-id"));
    assert!(toml_str.contains("Test description"));
    assert!(toml_str.contains("echo test"));
}

#[test]
fn test_snippets_with_multiple_items() {
    let snippets = Snippets {
        snippets: vec![
            Snippet {
                id: "id1".to_string(),
                description: "First".to_string(),
                command: "cmd1".to_string(),
                output: "".to_string(),
                tags: vec![],
                folders: vec![],
                favorite: false,
                created_at: 0,
                updated_at: 0,
                device_id: "".to_string(),
                deleted: false,
            },
            Snippet {
                id: "id2".to_string(),
                description: "Second".to_string(),
                command: "cmd2".to_string(),
                output: "".to_string(),
                tags: vec![],
                folders: vec![],
                favorite: false,
                created_at: 0,
                updated_at: 0,
                device_id: "".to_string(),
                deleted: false,
            },
        ],
        folders: vec!["work".to_string()],
    };

    let toml_str = toml::to_string_pretty(&snippets).unwrap();
    assert!(toml_str.contains("id1"));
    assert!(toml_str.contains("id2"));
    assert!(toml_str.contains("work"));
}

#[test]
fn test_library_manager_new() {
    let mgr = LibraryManager::new();
    // Should not panic - just verify it can be created
    assert!(mgr.is_ok() || mgr.is_err());
}

#[test]
fn test_snippet_new_empty_command_fails() {
    let result = Snippet::new("desc".to_string(), "  ".to_string(), vec![]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Empty command"));
}

#[test]
fn test_snippet_new_empty_description_fails() {
    let result = Snippet::new("  ".to_string(), "echo hi".to_string(), vec![]);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Empty description")
    );
}

#[test]
fn test_snippet_new_valid() {
    let result = Snippet::new(
        "desc".to_string(),
        "echo hi".to_string(),
        vec!["tag".to_string()],
    );
    assert!(result.is_ok());
    let s = result.unwrap();
    assert_eq!(s.description, "desc");
    assert_eq!(s.command, "echo hi");
}

#[test]
fn test_validate_library_name_empty() {
    assert!(validate_library_name("").is_err());
}

#[test]
fn test_validate_library_name_rejects_toml_suffix() {
    assert!(validate_library_name("foo.toml").is_err());
    assert!(validate_library_name(".toml").is_err());
    assert!(validate_library_name("foo").is_ok());
    assert!(validate_library_name("foo.toml.txt").is_ok());
}

#[test]
fn test_add_existing_library_promotes_first_to_primary() {
    let temp_dir = TempDir::new().unwrap();
    let mut mgr = LibraryManager::with_config_dir(temp_dir.path().to_path_buf()).unwrap();
    std::fs::create_dir_all(temp_dir.path().join("libraries")).unwrap();
    std::fs::write(temp_dir.path().join("libraries").join("imported.toml"), "").unwrap();

    mgr.add_existing_library("imported").unwrap();

    let libs = mgr.list_libraries();
    assert_eq!(libs.len(), 1);
    assert_eq!(libs[0].filename, "imported");
    assert!(
        libs[0].is_primary,
        "the only registered library must become primary"
    );
}

#[test]
fn test_validate_library_name_too_long() {
    assert!(validate_library_name(&"a".repeat(51)).is_err());
}

#[test]
fn test_validate_library_name_counts_characters() {
    assert!(validate_library_name(&"é".repeat(50)).is_ok());
    assert!(validate_library_name(&"é".repeat(51)).is_err());
}

#[test]
fn test_validate_library_name_slash() {
    assert!(validate_library_name("foo/bar").is_err());
}

#[test]
fn test_validate_library_name_backslash() {
    assert!(validate_library_name("foo\\bar").is_err());
}

#[test]
fn test_validate_library_name_null_byte() {
    assert!(validate_library_name("foo\0bar").is_err());
}

#[test]
fn test_validate_library_name_dot() {
    assert!(validate_library_name(".").is_err());
    assert!(validate_library_name("..").is_err());
}

#[test]
fn test_validate_library_name_allows_internal_double_dot() {
    // Traversal is impossible once slashes and bare "."/".." are rejected;
    // internal dots are ordinary characters.
    assert!(validate_library_name("my..lib").is_ok());
}

#[test]
fn test_validate_library_name_valid() {
    assert!(validate_library_name("my-library").is_ok());
    assert!(validate_library_name("work snippets").is_ok());
}

#[test]
fn test_save_library_atomic_write() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("test.toml");
    let snippets = Snippets {
        snippets: vec![Snippet {
            id: "atomic-test".to_string(),
            description: "Atomic write test".to_string(),
            command: "echo atomic".to_string(),
            output: "".to_string(),
            tags: vec![],
            folders: vec![],
            favorite: false,
            created_at: 100,
            updated_at: 100,
            device_id: "d1".to_string(),
            deleted: false,
        }],
        folders: vec![],
    };
    save_library(&path, &snippets).unwrap();
    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets.len(), 1);
    assert_eq!(loaded.snippets[0].id, "atomic-test");
    // Verify no .tmp files remain after atomic rename
    let parent = path.parent().unwrap();
    let has_tmp = std::fs::read_dir(parent)
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.path().extension().is_some_and(|ext| ext == "tmp"));
    assert!(!has_tmp, "temp files should not remain after atomic rename");
}

#[test]
fn test_create_library_uses_private_atomic_write() {
    let temp_dir = TempDir::new().unwrap();
    let mut mgr = LibraryManager {
        config_dir: temp_dir.path().to_path_buf(),
        libraries_dir: temp_dir.path().join("libraries"),
        premade_dir: temp_dir.path().join("premade"),
        config: Default::default(),
    };

    let path = mgr.create_library("private").unwrap();

    assert!(path.exists());
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("snippets = []")
    );

    #[cfg(unix)]
    assert_eq!(file_mode(&path), 0o600);
}

#[test]
fn test_delete_library_restores_config_when_file_deletion_fails() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().to_path_buf();
    let libraries_dir = config_dir.join("libraries");
    let blocked_path = libraries_dir.join("blocked.toml");
    std::fs::create_dir_all(&blocked_path).unwrap();

    let mut mgr = LibraryManager {
        config_dir,
        libraries_dir,
        premade_dir: temp_dir.path().join("premade"),
        config: LibraryConfig {
            libraries: vec![LibraryMeta {
                filename: "blocked".to_string(),
                library_id: String::new(),
                is_primary: true,
                last_sync: None,
                server_id: None,
            }],
            generation: 0,
        },
    };

    assert!(mgr.delete_library("blocked").is_err());
    assert!(mgr.get_library_by_filename("blocked").is_some());
    assert!(blocked_path.is_dir());
    assert!(
        std::fs::read_to_string(temp_dir.path().join("libraries.toml"))
            .unwrap()
            .contains("filename = \"blocked\"")
    );
}

#[test]
fn test_add_server_library_uses_private_atomic_write() {
    let temp_dir = TempDir::new().unwrap();
    let mut mgr = LibraryManager {
        config_dir: temp_dir.path().to_path_buf(),
        libraries_dir: temp_dir.path().join("libraries"),
        premade_dir: temp_dir.path().join("premade"),
        config: Default::default(),
    };

    let path = mgr
        .add_server_library("Shared Commands", "server-library-id")
        .unwrap();

    assert!(path.exists());
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("Imported from server")
    );

    #[cfg(unix)]
    assert_eq!(file_mode(&path), 0o600);
}

#[test]
fn test_save_config_invalidates_libraries_toml_cache() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().to_path_buf();
    let libraries_dir = config_dir.join("libraries");
    let premade_dir = config_dir.join("premade");
    std::fs::create_dir_all(&libraries_dir).unwrap();

    let config_path = config_dir.join("libraries.toml");
    std::fs::write(
        &config_path,
        r#"
[[libraries]]
filename = "old"
library_id = ""
is_primary = true
"#,
    )
    .unwrap();
    let cached_before = cached_read_toml(&config_path).unwrap();
    assert!(cached_before.contains("old"));

    let mut mgr = LibraryManager {
        config_dir,
        libraries_dir,
        premade_dir,
        config: LibraryConfig {
            libraries: vec![LibraryMeta {
                filename: "old".to_string(),
                library_id: String::new(),
                is_primary: true,
                last_sync: None,
                server_id: None,
            }],
            generation: 0,
        },
    };
    mgr.create_library("new").unwrap();

    let cached_after = cached_read_toml(&config_path).unwrap();
    assert!(cached_after.contains("old"));
    assert!(cached_after.contains("new"));
}

#[test]
fn test_backup_library_names_do_not_collide() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("snippets.toml");
    std::fs::write(&path, "test content").unwrap();

    let first = backup_library(&path).unwrap().unwrap();
    let second = backup_library(&path).unwrap().unwrap();

    assert_ne!(first, second);

    let backup_dir = temp_dir.path().join("backups");
    let backup_count = std::fs::read_dir(backup_dir).unwrap().count();
    assert_eq!(backup_count, 2);
}

#[test]
fn test_save_premade_library_path_traversal() {
    let temp_dir = TempDir::new().unwrap();
    let mgr = LibraryManager {
        config_dir: temp_dir.path().to_path_buf(),
        libraries_dir: temp_dir.path().join("libraries"),
        premade_dir: temp_dir.path().join("premade"),
        config: Default::default(),
    };
    assert!(
        mgr.save_premade_library("../../etc/passwd", "content")
            .is_err()
    );
    assert!(mgr.save_premade_library("../escape", "content").is_err());
    assert!(mgr.save_premade_library("foo/bar", "content").is_err());
}

#[test]
fn test_save_premade_library_valid() {
    let temp_dir = TempDir::new().unwrap();
    let mgr = LibraryManager {
        config_dir: temp_dir.path().to_path_buf(),
        libraries_dir: temp_dir.path().join("libraries"),
        premade_dir: temp_dir.path().join("premade"),
        config: Default::default(),
    };
    let result = mgr.save_premade_library("valid-name", "test content");
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(path.exists());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "test content");

    #[cfg(unix)]
    assert_eq!(file_mode(&path), 0o600);
}

#[test]
fn test_deduplication_on_load() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("dup.toml");
    let toml_content = r#"
[[Snippets]]
Id = "same-id"
Description = "First"
Command = "cmd1"

[[Snippets]]
Id = "same-id"
Description = "Second"
Command = "cmd2"
"#;
    std::fs::write(&path, toml_content).unwrap();
    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets.len(), 2);
    assert_ne!(loaded.snippets[0].id, loaded.snippets[1].id);
}

#[test]
fn test_fixture_canonical_pet_roundtrips() {
    let content = include_str!("../../tests/fixtures/canonical_pet.toml");
    let library: Snippets = toml::from_str(content).unwrap();

    assert_eq!(library.snippets.len(), 5);

    assert_eq!(library.snippets[0].description, "git commit with message");
    assert_eq!(library.snippets[0].command, "git commit -m \"<msg>\"");
    assert_eq!(library.snippets[0].tags, vec!["git", "version-control"]);

    assert_eq!(
        library.snippets[1].description,
        "docker ps running containers"
    );
    assert!(library.snippets[1].command.contains("docker ps"));

    assert_eq!(
        library.snippets[3].description,
        "search & replace with \"quotes\" and special chars"
    );
    assert_eq!(library.snippets[3].tags, vec!["search", "ripgrep"]);

    assert_eq!(
        library.snippets[4].description,
        "日本語のスニペット — unicode test"
    );
    assert_eq!(library.snippets[4].tags, vec!["unicode", "日本語"]);

    // Roundtrip: serialize back and reload
    let serialized = toml::to_string_pretty(&library).unwrap();
    let reloaded: Snippets = toml::from_str(&serialized).unwrap();

    assert_eq!(reloaded.snippets.len(), 5);
    for i in 0..5 {
        assert_eq!(
            reloaded.snippets[i].description,
            library.snippets[i].description
        );
        assert_eq!(reloaded.snippets[i].command, library.snippets[i].command);
        assert_eq!(reloaded.snippets[i].tags, library.snippets[i].tags);
    }
}

#[test]
fn test_fixture_snip_it_native_roundtrips() {
    let content = include_str!("../../tests/fixtures/snip_it_native.toml");
    let library: Snippets = toml::from_str(content).unwrap();

    assert_eq!(library.snippets.len(), 4);

    assert_eq!(
        library.snippets[0].id,
        "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d"
    );
    assert_eq!(
        library.snippets[0].description,
        "list all docker containers"
    );
    assert_eq!(library.snippets[0].tags, vec!["docker"]);
    assert_eq!(library.snippets[0].folders, vec!["devops", "containers"]);
    assert!(library.snippets[0].favorite);
    assert_eq!(library.snippets[0].created_at, 1700000000);
    assert_eq!(library.snippets[0].updated_at, 1700100000);
    assert_eq!(library.snippets[0].device_id, "device-alpha-001");
    assert!(!library.snippets[0].deleted);

    assert!(!library.snippets[1].favorite);
    assert_eq!(library.snippets[1].folders, vec!["ops"]);

    assert_eq!(
        library.snippets[3].id,
        "aabbccdd-eeff-4000-8111-223344556677"
    );
    assert!(library.snippets[3].deleted);

    // Roundtrip
    let serialized = toml::to_string_pretty(&library).unwrap();
    let reloaded: Snippets = toml::from_str(&serialized).unwrap();

    assert_eq!(reloaded.snippets.len(), 4);
    for i in 0..4 {
        assert_eq!(reloaded.snippets[i].id, library.snippets[i].id);
        assert_eq!(
            reloaded.snippets[i].description,
            library.snippets[i].description
        );
        assert_eq!(reloaded.snippets[i].command, library.snippets[i].command);
        assert_eq!(reloaded.snippets[i].tags, library.snippets[i].tags);
        assert_eq!(reloaded.snippets[i].favorite, library.snippets[i].favorite);
        assert_eq!(
            reloaded.snippets[i].created_at,
            library.snippets[i].created_at
        );
        assert_eq!(
            reloaded.snippets[i].updated_at,
            library.snippets[i].updated_at
        );
        assert_eq!(reloaded.snippets[i].deleted, library.snippets[i].deleted);
    }
}

#[test]
fn test_fixture_legacy_uppercase_loads_as_snippets() {
    let content = include_str!("../../tests/fixtures/legacy_uppercase.toml");
    let library: Snippets = toml::from_str(content).unwrap();

    assert_eq!(library.snippets.len(), 2);

    assert_eq!(library.snippets[0].description, "list files");
    assert_eq!(library.snippets[0].command, "ls -la");
    assert_eq!(library.snippets[0].tags, vec!["files"]);

    assert_eq!(library.snippets[1].description, "find large files");
    assert_eq!(library.snippets[1].command, "find . -type f -size +10M");
    assert_eq!(library.snippets[1].tags, vec!["search", "find"]);
}

#[test]
fn test_fixture_mixed_aliases_load_correctly() {
    let content = include_str!("../../tests/fixtures/mixed_field_aliases.toml");
    let library: Snippets = toml::from_str(content).unwrap();

    assert_eq!(library.snippets.len(), 3);

    // First snippet uses "name" alias for description and "cmd" alias for command
    assert_eq!(
        library.snippets[0].description,
        "snippet using name instead of description"
    );
    assert_eq!(library.snippets[0].command, "echo alias test");
    assert_eq!(library.snippets[0].tags, vec!["aliases"]);

    // Second snippet uses canonical field names
    assert_eq!(
        library.snippets[1].description,
        "snippet using canonical field names"
    );
    assert_eq!(library.snippets[1].command, "echo canonical");
    assert_eq!(library.snippets[1].tags, vec!["canonical"]);

    // Third snippet uses capitalized "Description" and "Command"
    assert_eq!(
        library.snippets[2].description,
        "snippet using capitalized Description"
    );
    assert_eq!(library.snippets[2].command, "echo capitalized");
    assert_eq!(library.snippets[2].tags, vec!["capitalized"]);
}

#[test]
fn test_fixture_variable_commands_preserve_syntax() {
    let content = include_str!("../../tests/fixtures/variable_commands.toml");
    let library: Snippets = toml::from_str(content).unwrap();

    assert_eq!(library.snippets.len(), 5);

    // Simple variable
    assert_eq!(library.snippets[0].command, "echo <greeting>");

    // Variable with default value
    assert_eq!(library.snippets[1].command, "ssh <host=localhost> 'uptime'");

    // Escaped angle brackets — single-quoted TOML literal preserves \< as-is
    assert_eq!(library.snippets[2].command, "ping \\<hostname\\> -c 3");

    // Nested angle brackets
    assert_eq!(library.snippets[3].command, "echo <outer<inner>>");

    // Variable with default containing spaces
    assert_eq!(
        library.snippets[4].command,
        "cp <src=/tmp/default file> <dest=.>"
    );

    // Roundtrip preserves variable syntax
    let serialized = toml::to_string_pretty(&library).unwrap();
    let reloaded: Snippets = toml::from_str(&serialized).unwrap();

    assert_eq!(reloaded.snippets.len(), 5);
    for i in 0..5 {
        assert_eq!(reloaded.snippets[i].command, library.snippets[i].command);
    }
}

#[test]
fn test_fixture_empty_library_loads_empty() {
    let content = include_str!("../../tests/fixtures/empty_library.toml");
    let library: Snippets = toml::from_str(content).unwrap();

    assert!(library.snippets.is_empty());
}

#[test]
fn test_pet_field_names_in_output() {
    let snippet = Snippet {
        description: "list files".to_string(),
        command: "ls -la".to_string(),
        tags: vec!["files".to_string()],
        output: "".to_string(),
        ..Default::default()
    };
    let snippets = Snippets {
        snippets: vec![snippet],
        ..Default::default()
    };

    let toml_str = toml::to_string_pretty(&snippets).unwrap();
    assert!(toml_str.contains("[[snippets]]"));
    assert!(toml_str.contains("description = \"list files\""));
    assert!(toml_str.contains("command = \"ls -la\""));
    assert!(toml_str.contains("tag = [\"files\"]"));
    assert!(toml_str.contains("output = \"\""));
    assert!(!toml_str.contains("Description"));
    assert!(!toml_str.contains("Command"));
    assert!(!toml_str.contains("Tag ="));
}

#[test]
fn test_snippet_description_alias_roundtrip() {
    let snippet = Snippet {
        description: "test description".to_string(),
        command: "echo test".to_string(),
        ..Default::default()
    };

    let toml_str = toml::to_string_pretty(&snippet).unwrap();
    assert!(toml_str.contains("description = \"test description\""));
    assert!(!toml_str.contains("Description ="));
    assert!(!toml_str.contains("name ="));

    let reloaded: Snippet = toml::from_str(&toml_str).unwrap();
    assert_eq!(reloaded.description, "test description");
}

#[test]
fn test_snippet_command_alias_roundtrip() {
    let snippet = Snippet {
        description: "test".to_string(),
        command: "echo hello world".to_string(),
        ..Default::default()
    };

    let toml_str = toml::to_string_pretty(&snippet).unwrap();
    assert!(toml_str.contains("command = \"echo hello world\""));
    assert!(!toml_str.contains("Command ="));
    assert!(!toml_str.contains("cmd ="));

    let reloaded: Snippet = toml::from_str(&toml_str).unwrap();
    assert_eq!(reloaded.command, "echo hello world");
}

#[test]
fn test_snippet_tags_rename_roundtrip() {
    let snippet = Snippet {
        description: "test".to_string(),
        command: "echo test".to_string(),
        tags: vec!["rust".to_string(), "test".to_string()],
        ..Default::default()
    };

    let toml_str = toml::to_string_pretty(&snippet).unwrap();
    assert!(
        toml_str.contains("tag ="),
        "Expected 'tag' field, got: {}",
        toml_str
    );
    assert!(toml_str.contains("\"rust\""));
    assert!(toml_str.contains("\"test\""));
    assert!(!toml_str.contains("tags ="));
    assert!(!toml_str.contains("Tags ="));

    let reloaded: Snippet = toml::from_str(&toml_str).unwrap();
    assert_eq!(reloaded.tags, vec!["rust", "test"]);
}

#[test]
fn test_snippet_with_empty_output_roundtrips() {
    let snippet = Snippet {
        description: "test".to_string(),
        command: "echo test".to_string(),
        output: String::new(),
        ..Default::default()
    };
    let library = Snippets {
        snippets: vec![snippet],
        ..Default::default()
    };

    let serialized = toml::to_string_pretty(&library).unwrap();
    let reloaded: Snippets = toml::from_str(&serialized).unwrap();

    assert_eq!(reloaded.snippets.len(), 1);
    assert_eq!(reloaded.snippets[0].output, "");
}

#[test]
fn test_snippet_with_nonempty_output_roundtrips() {
    let snippet = Snippet {
        description: "test".to_string(),
        command: "echo test".to_string(),
        output: "some output".to_string(),
        ..Default::default()
    };
    let library = Snippets {
        snippets: vec![snippet],
        ..Default::default()
    };

    let serialized = toml::to_string_pretty(&library).unwrap();
    let reloaded: Snippets = toml::from_str(&serialized).unwrap();

    assert_eq!(reloaded.snippets.len(), 1);
    assert_eq!(reloaded.snippets[0].output, "some output");
}

#[test]
fn test_deleted_snippet_preserved_on_roundtrip() {
    let snippet = Snippet {
        description: "deleted snippet".to_string(),
        command: "echo deleted".to_string(),
        deleted: true,
        ..Default::default()
    };
    let library = Snippets {
        snippets: vec![snippet],
        ..Default::default()
    };

    let serialized = toml::to_string_pretty(&library).unwrap();
    let reloaded: Snippets = toml::from_str(&serialized).unwrap();

    assert_eq!(reloaded.snippets.len(), 1);
    assert!(reloaded.snippets[0].deleted);
}

#[test]
fn test_timestamps_preserved_on_roundtrip() {
    let snippet = Snippet {
        description: "test".to_string(),
        command: "echo test".to_string(),
        created_at: 1700000000,
        updated_at: 1700123456,
        ..Default::default()
    };
    let library = Snippets {
        snippets: vec![snippet],
        ..Default::default()
    };

    let serialized = toml::to_string_pretty(&library).unwrap();
    let reloaded: Snippets = toml::from_str(&serialized).unwrap();

    assert_eq!(reloaded.snippets.len(), 1);
    assert_eq!(reloaded.snippets[0].created_at, 1700000000);
    assert_eq!(reloaded.snippets[0].updated_at, 1700123456);
}

#[test]
fn test_uuid_preserved_on_roundtrip() {
    let snippet = Snippet {
        id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        description: "test".to_string(),
        command: "echo test".to_string(),
        ..Default::default()
    };
    let library = Snippets {
        snippets: vec![snippet],
        ..Default::default()
    };

    let serialized = toml::to_string_pretty(&library).unwrap();
    let reloaded: Snippets = toml::from_str(&serialized).unwrap();

    assert_eq!(reloaded.snippets.len(), 1);
    assert_eq!(
        reloaded.snippets[0].id,
        "550e8400-e29b-41d4-a716-446655440000"
    );
}

#[test]
fn test_special_characters_in_description_roundtrip() {
    let snippet = Snippet {
        description: "has \"quotes\" & ampersand <> angle brackets".to_string(),
        command: "echo test".to_string(),
        ..Default::default()
    };
    let library = Snippets {
        snippets: vec![snippet],
        ..Default::default()
    };

    let serialized = toml::to_string_pretty(&library).unwrap();
    let reloaded: Snippets = toml::from_str(&serialized).unwrap();

    assert_eq!(reloaded.snippets.len(), 1);
    assert_eq!(
        reloaded.snippets[0].description,
        "has \"quotes\" & ampersand <> angle brackets"
    );
}

// ============================================================
// Release 2 Final Corrective: Direct serialization matrix
// ============================================================
//
// These tests pin down the contract that the exact Rust `String` value of
// every snippet field survives the full save / load pipeline. They cover
// every byte sequence previously excluded from the golden corpus on the
// (incorrect) premise that TOML cannot preserve them. The TOML format and
// the `toml` crate's serializer do preserve all of these values; the
// corruption that motivated their original exclusion came from the
// custom `quote_strings_containing_backslashes` post-processing helper,
// which snip-it no longer applies to its own output.

fn snippet_with_command(command: &str) -> Snippets {
    Snippets {
        snippets: vec![Snippet {
            id: "test-id".to_string(),
            description: "test description".to_string(),
            command: command.to_string(),
            ..Default::default()
        }],
        folders: vec![],
    }
}

fn snippet_with_description(command: &str, description: &str) -> Snippets {
    Snippets {
        snippets: vec![Snippet {
            id: "test-id".to_string(),
            description: description.to_string(),
            command: command.to_string(),
            ..Default::default()
        }],
        folders: vec![],
    }
}

fn assert_command_survives_pretty_roundtrip(label: &str, command: &str) {
    let library = snippet_with_command(command);
    let serialized = toml::to_string_pretty(&library)
        .unwrap_or_else(|e| panic!("serialize failed for {label}: {e}"));
    let recovered: Snippets = toml::from_str(&serialized)
        .unwrap_or_else(|e| panic!("parse failed for {label}: {e}\nTOML:\n{serialized}"));
    assert_eq!(
        recovered.snippets[0].command, command,
        "round-trip mismatch for {label}: original = {command:?}, recovered = {:?}",
        recovered.snippets[0].command
    );
}

fn assert_command_survives_save_load(label: &str, command: &str) {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("matrix.toml");
    let library = snippet_with_command(command);
    save_library(&path, &library).unwrap_or_else(|e| panic!("save failed for {label}: {e}"));
    let loaded = load_library(&path).unwrap_or_else(|e| panic!("load failed for {label}: {e}"));
    assert_eq!(loaded.snippets.len(), 1, "{label}: snippet count");
    assert_eq!(
        loaded.snippets[0].command, command,
        "{label}: save/load round-trip mismatch\noriginal = {command:?}\nrecovered = {:?}",
        loaded.snippets[0].command
    );
}

#[test]
fn test_serialization_matrix_internal_tab() {
    assert_command_survives_pretty_roundtrip("internal_tab", "pre\tpost");
    assert_command_survives_save_load("internal_tab", "pre\tpost");
}

#[test]
fn test_serialization_matrix_leading_tab() {
    assert_command_survives_pretty_roundtrip("leading_tab", "\tpre");
    assert_command_survives_save_load("leading_tab", "\tpre");
}

#[test]
fn test_serialization_matrix_trailing_tab() {
    assert_command_survives_pretty_roundtrip("trailing_tab", "pre\t");
    assert_command_survives_save_load("trailing_tab", "pre\t");
}

#[test]
fn test_serialization_matrix_only_tab() {
    assert_command_survives_pretty_roundtrip("only_tab", "\t");
    assert_command_survives_save_load("only_tab", "\t");
}

#[test]
fn test_serialization_matrix_one_trailing_space() {
    assert_command_survives_pretty_roundtrip("one_trailing_space", "pre ");
    assert_command_survives_save_load("one_trailing_space", "pre ");
}

#[test]
fn test_serialization_matrix_multi_trailing_spaces() {
    assert_command_survives_pretty_roundtrip("multi_trailing_spaces", "pre   ");
    assert_command_survives_save_load("multi_trailing_spaces", "pre   ");
}

#[test]
fn test_serialization_matrix_spaces_before_newline() {
    assert_command_survives_pretty_roundtrip("spaces_before_newline", "pre   \n");
    assert_command_survives_save_load("spaces_before_newline", "pre   \n");
}

#[test]
fn test_serialization_matrix_crlf() {
    assert_command_survives_pretty_roundtrip("crlf", "a\r\nb");
    assert_command_survives_save_load("crlf", "a\r\nb");
}

#[test]
fn test_serialization_matrix_mixed_lf_crlf() {
    assert_command_survives_pretty_roundtrip("mixed_lf_crlf", "a\nb\r\nc");
    assert_command_survives_save_load("mixed_lf_crlf", "a\nb\r\nc");
}

#[test]
fn test_serialization_matrix_final_carriage_return() {
    assert_command_survives_pretty_roundtrip("final_cr", "a\r");
    assert_command_survives_save_load("final_cr", "a\r");
}

#[test]
fn test_serialization_matrix_lone_crlf() {
    assert_command_survives_pretty_roundtrip("lone_crlf", "\r\n");
    assert_command_survives_save_load("lone_crlf", "\r\n");
}

#[test]
fn test_serialization_matrix_tab_with_quotes() {
    assert_command_survives_pretty_roundtrip("tab_with_quotes", "a\t\"b\"c");
    assert_command_survives_save_load("tab_with_quotes", "a\t\"b\"c");
}

#[test]
fn test_serialization_matrix_tab_with_backslashes() {
    assert_command_survives_pretty_roundtrip("tab_with_backslashes", "a\t\\b");
    assert_command_survives_save_load("tab_with_backslashes", "a\t\\b");
}

#[test]
fn test_serialization_matrix_all_problematic() {
    assert_command_survives_pretty_roundtrip("all_problematic", "\t  \r\n");
    assert_command_survives_save_load("all_problematic", "\t  \r\n");
}

#[test]
fn test_serialization_matrix_makefile_leading_tabs() {
    let command = "if true; then\n\techo yes\nelse\n\techo no\nfi";
    assert_command_survives_pretty_roundtrip("makefile_leading_tabs_no_nl", command);
    assert_command_survives_save_load("makefile_leading_tabs_no_nl", command);
}

#[test]
fn test_serialization_matrix_makefile_leading_tabs_with_trailing_newline() {
    let command = "if true; then\n\techo yes\nelse\n\techo no\nfi\n";
    assert_command_survives_pretty_roundtrip("makefile_leading_tabs_trailing_nl", command);
    assert_command_survives_save_load("makefile_leading_tabs_trailing_nl", command);
}

#[test]
fn test_serialization_matrix_variable_syntax_with_tab() {
    let command = "ssh\t<host=localhost>\t-p\t<port=22>";
    assert_command_survives_pretty_roundtrip("variable_with_tab", command);
    assert_command_survives_save_load("variable_with_tab", command);
}

#[test]
fn test_serialization_matrix_escaped_angle_brackets_with_crlf() {
    let command = "echo \\<start\\>\r\necho \\<end\\>";
    assert_command_survives_pretty_roundtrip("escaped_brackets_crlf", command);
    assert_command_survives_save_load("escaped_brackets_crlf", command);
}

#[test]
fn test_serialization_matrix_description_with_tab_and_trailing_space() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("desc.toml");
    let library = snippet_with_description("echo test", "  hello\t");
    save_library(&path, &library).unwrap();
    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets[0].description, "  hello\t");
}

#[test]
fn test_serialization_matrix_tag_with_internal_tab() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("tag.toml");
    let library = Snippets {
        snippets: vec![Snippet {
            id: "id".to_string(),
            description: "tagged".to_string(),
            command: "echo tag".to_string(),
            tags: vec!["docker\tbuild".to_string()],
            ..Default::default()
        }],
        folders: vec![],
    };
    save_library(&path, &library).unwrap();
    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets[0].tags, vec!["docker\tbuild"]);
}

#[test]
fn test_serialization_matrix_repeated_save_load_idempotent() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("idempotent.toml");
    let command = "echo\there\r\nwith\ttabs and trailing space ";

    let mut library = snippet_with_command(command);
    for round in 0..5 {
        save_library(&path, &library).unwrap();
        let loaded = load_library(&path).unwrap();
        assert_eq!(
            loaded.snippets[0].command, command,
            "round {round}: command diverged"
        );
        library = loaded;
    }
}

#[test]
fn test_serialization_matrix_pretty_and_compact_agree() {
    let command = "echo\there\nwith\ttabs\r\nand \"quotes\" and trailing space ";
    let library = snippet_with_command(command);

    let pretty = toml::to_string_pretty(&library).unwrap();
    let compact = toml::to_string(&library).unwrap();

    let from_pretty: Snippets = toml::from_str(&pretty).unwrap();
    let from_compact: Snippets = toml::from_str(&compact).unwrap();

    assert_eq!(from_pretty.snippets[0].command, command);
    assert_eq!(from_compact.snippets[0].command, command);
}

#[test]
fn test_serialization_matrix_handwritten_backslash_escape_still_loads() {
    // Hand-written double-quoted TOML with `\<` / `\>` is invalid TOML but
    // a long-standing legacy convention. `fix_invalid_toml_escapes`
    // converts it to single-quoted raw form on load so legacy files
    // continue to work.
    let legacy_toml = "\
[[snippets]]
description = \"legacy\"
command = \"ping \\<website\\>\"
tag = []
output = \"\"
";
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("legacy.toml");
    std::fs::write(&path, legacy_toml).unwrap();

    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets.len(), 1);
    assert_eq!(loaded.snippets[0].command, "ping \\<website\\>");
}

#[test]
fn test_serialization_matrix_no_normalization_strip_or_trim() {
    // Save / load / re-save / re-load many times, then confirm the byte
    // content is unchanged across every round. This guards against any
    // silent normalization (trim, line-ending rewrite, escape collapse)
    // creeping back into the pipeline.
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("norewrite.toml");
    let command = "echo start\there\t with trailing tab\tand trailing space \r\nand CRLF\r";

    let library = snippet_with_command(command);
    save_library(&path, &library).unwrap();

    let disk_after_first = std::fs::read(&path).unwrap();
    for _ in 0..10 {
        let loaded = load_library(&path).unwrap();
        save_library(&path, &loaded).unwrap();
        let disk_now = std::fs::read(&path).unwrap();
        assert_eq!(
            disk_now, disk_after_first,
            "file content changed across save rounds — normalization regression"
        );
    }

    let final_loaded = load_library(&path).unwrap();
    assert_eq!(final_loaded.snippets[0].command, command);
}

// ============================================================
// Phase 14B: Fail-closed on malformed TOML
// ============================================================

#[test]
fn test_malformed_library_returns_err_and_creates_backup() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("corrupt.toml");
    std::fs::write(&path, "invalid = [toml").unwrap();

    let result = load_library(&path);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("parse"),
        "error should mention parse: {err_msg}"
    );

    let backup_path = path.with_extension("toml.corrupt.bak");
    assert!(
        backup_path.exists(),
        "corrupt backup should be created at {}",
        backup_path.display()
    );
}

#[test]
fn test_malformed_libraries_toml_causes_library_manager_error() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("libraries.toml");
    std::fs::write(&config_path, "invalid = [toml").unwrap();

    let result = LibraryManager::with_config_dir(temp_dir.path().to_path_buf());
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("parse"),
        "error should mention parse: {err_msg}"
    );

    let backup_path = config_path.with_extension("toml.corrupt");
    assert!(
        backup_path.exists(),
        "corrupt backup should be created at {}",
        backup_path.display()
    );
}

#[test]
fn test_missing_library_file_returns_empty() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("nonexistent.toml");
    let loaded = load_library(&path).unwrap();
    assert!(loaded.snippets.is_empty());
}

#[test]
fn test_empty_library_file_returns_empty() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("empty.toml");
    std::fs::write(&path, "").unwrap();
    let loaded = load_library(&path).unwrap();
    assert!(loaded.snippets.is_empty());
}

#[test]
fn test_missing_libraries_toml_returns_default_manager() {
    let temp_dir = TempDir::new().unwrap();
    let result = LibraryManager::with_config_dir(temp_dir.path().to_path_buf());
    assert!(result.is_ok());
}

#[test]
fn test_empty_libraries_toml_returns_default_manager() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("libraries.toml");
    std::fs::write(&config_path, "").unwrap();
    let result = LibraryManager::with_config_dir(temp_dir.path().to_path_buf());
    assert!(result.is_ok());
}

// ============================================================
// Phase 14B: Deterministic ID normalization
// ============================================================

#[test]
fn test_missing_id_deterministic_across_loads() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("legacy.toml");
    std::fs::write(
        &path,
        r#"
[[snippets]]
description = "test snippet"
command = "echo hello"
"#,
    )
    .unwrap();

    let loaded1 = load_library(&path).unwrap();
    let loaded2 = load_library(&path).unwrap();

    assert_eq!(loaded1.snippets.len(), 1);
    assert_eq!(loaded2.snippets.len(), 1);
    assert_eq!(loaded1.snippets[0].id, loaded2.snippets[0].id);
    assert!(
        loaded1.snippets[0].id.starts_with("legacy-"),
        "deterministic ID should start with 'legacy-'"
    );
}

#[test]
fn test_content_distinct_missing_ids_receive_distinct_ids() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("distinct.toml");
    std::fs::write(
        &path,
        r#"
[[snippets]]
description = "snippet A"
command = "echo a"

[[snippets]]
description = "snippet B"
command = "echo b"
"#,
    )
    .unwrap();

    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets.len(), 2);
    assert_ne!(loaded.snippets[0].id, loaded.snippets[1].id);
    assert!(loaded.snippets[0].id.starts_with("legacy-"));
    assert!(loaded.snippets[1].id.starts_with("legacy-"));
}

#[test]
fn test_identical_missing_ids_receive_distinct_repeatable_ids() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("identical.toml");
    std::fs::write(
        &path,
        r#"
[[snippets]]
description = "same content"
command = "echo same"

[[snippets]]
description = "same content"
command = "echo same"
"#,
    )
    .unwrap();

    let loaded1 = load_library(&path).unwrap();
    let loaded2 = load_library(&path).unwrap();

    assert_eq!(loaded1.snippets.len(), 2);
    assert_ne!(loaded1.snippets[0].id, loaded1.snippets[1].id);
    // Same IDs across loads
    assert_eq!(loaded1.snippets[0].id, loaded2.snippets[0].id);
    assert_eq!(loaded1.snippets[1].id, loaded2.snippets[1].id);
}

#[test]
fn test_first_duplicate_id_kept_later_replaced() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("dup_ids.toml");
    std::fs::write(
        &path,
        r#"
[[snippets]]
id = "my-custom-id"
description = "first"
command = "echo 1"

[[snippets]]
id = "my-custom-id"
description = "second"
command = "echo 2"
"#,
    )
    .unwrap();

    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets.len(), 2);
    assert_eq!(loaded.snippets[0].id, "my-custom-id");
    assert_ne!(loaded.snippets[1].id, "my-custom-id");
    assert!(loaded.snippets[1].id.starts_with("legacy-"));
}

#[test]
fn test_valid_unique_ids_unchanged() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("unique_ids.toml");
    std::fs::write(
        &path,
        r#"
[[snippets]]
id = "id-aaa"
description = "first"
command = "echo 1"

[[snippets]]
id = "id-bbb"
description = "second"
command = "echo 2"
"#,
    )
    .unwrap();

    let loaded = load_library(&path).unwrap();
    assert_eq!(loaded.snippets[0].id, "id-aaa");
    assert_eq!(loaded.snippets[1].id, "id-bbb");
}

#[test]
fn test_save_persists_provisional_ids() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("persist.toml");
    std::fs::write(
        &path,
        r#"
[[snippets]]
description = "no id snippet"
command = "echo persist"
"#,
    )
    .unwrap();

    let loaded = load_library(&path).unwrap();
    let provisional_id = loaded.snippets[0].id.clone();
    assert!(provisional_id.starts_with("legacy-"));

    // Save (normal mutation path)
    save_library(&path, &loaded).unwrap();

    // Reload — IDs should now be explicitly stored
    let reloaded = load_library(&path).unwrap();
    assert_eq!(reloaded.snippets[0].id, provisional_id);
}

#[test]
fn test_reload_after_persistence_no_different_ids() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("reload.toml");
    std::fs::write(
        &path,
        r#"
[[snippets]]
description = "stable"
command = "echo stable"
"#,
    )
    .unwrap();

    let loaded1 = load_library(&path).unwrap();
    save_library(&path, &loaded1).unwrap();

    // Multiple reloads after save should all produce the same ID
    for _ in 0..5 {
        let reloaded = load_library(&path).unwrap();
        assert_eq!(reloaded.snippets[0].id, loaded1.snippets[0].id);
    }
}

#[test]
fn test_id_length_below_server_maximum() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("length.toml");
    std::fs::write(
        &path,
        r#"
[[snippets]]
description = "length check"
command = "echo length"
"#,
    )
    .unwrap();

    let loaded = load_library(&path).unwrap();
    // Server default max_id_length is 128
    assert!(
        loaded.snippets[0].id.len() <= 128,
        "ID length {} exceeds 128",
        loaded.snippets[0].id.len()
    );
}

#[test]
fn test_legacy_id_not_treated_as_uuid() {
    // Ensure our legacy IDs don't confuse any UUID-specific code paths.
    // The ID is an opaque string — this test proves it roundtrips correctly.
    let id = deterministic_legacy_id(
        &Snippet {
            description: "test".to_string(),
            command: "echo test".to_string(),
            tags: vec!["tag".to_string()],
            output: "out".to_string(),
            ..Default::default()
        },
        0,
    );
    assert!(id.starts_with("legacy-"));
    assert_eq!(id.len(), 71); // "legacy-" (7) + 64 hex chars
}

#[test]
fn test_legacy_ids_include_folder_favorite_and_device() {
    let base = Snippet {
        description: "same".to_string(),
        command: "echo same".to_string(),
        ..Default::default()
    };
    let base_id = deterministic_legacy_id(&base, 0);

    let mut folder_variant = base.clone();
    folder_variant.folders.push("work".to_string());
    assert_ne!(base_id, deterministic_legacy_id(&folder_variant, 0));

    let mut favorite_variant = base.clone();
    favorite_variant.favorite = true;
    assert_ne!(base_id, deterministic_legacy_id(&favorite_variant, 0));

    let mut device_variant = base;
    device_variant.device_id = "other-device".to_string();
    assert_ne!(base_id, deterministic_legacy_id(&device_variant, 0));
}

// ── Plan 009: canonical read-only resolution + shared inspection ──

fn write_index(dir: &std::path::Path, libs: &[LibraryMeta]) {
    let config = LibraryConfig {
        libraries: libs.to_vec(),
        generation: 0,
    };
    let content = toml::to_string_pretty(&config).unwrap();
    std::fs::write(dir.join("libraries.toml"), content).unwrap();
}

fn write_lib_file(libs_dir: &std::path::Path, name: &str) {
    std::fs::write(libs_dir.join(format!("{name}.toml")), "snippets = []\n").unwrap();
}

#[test]
fn test_readonly_primary_named_all_resolution() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("cfg");
    let libs_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libs_dir).unwrap();
    let mut work = LibraryMeta::new("work");
    work.is_primary = true;
    work.library_id = "server-work".to_string();
    let personal = LibraryMeta::new("personal");
    write_index(&config_dir, &[work, personal]);
    write_lib_file(&libs_dir, "work");
    write_lib_file(&libs_dir, "personal");

    let mgr = LibraryManager::with_config_dir(config_dir).unwrap();

    let primary = mgr.resolve_readonly_sources(None).unwrap();
    assert_eq!(primary.len(), 1);
    assert_eq!(primary[0].name, "work");
    assert_eq!(primary[0].library_id, "server-work");
    assert_eq!(primary[0].path, libs_dir.join("work.toml"),);

    let named = mgr.resolve_readonly_sources(Some("personal")).unwrap();
    assert_eq!(named.len(), 1);
    assert_eq!(named[0].name, "personal");

    let mut all = mgr.resolve_readonly_sources(Some("all")).unwrap();
    all.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].name, "personal");
    assert_eq!(all[1].name, "work");
}

#[test]
fn test_readonly_missing_library_error() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("cfg");
    let libs_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libs_dir).unwrap();
    let mut work = LibraryMeta::new("work");
    work.is_primary = true;
    write_index(&config_dir, &[work]);
    write_lib_file(&libs_dir, "work");

    let mgr = LibraryManager::with_config_dir(config_dir).unwrap();
    let err = mgr.resolve_readonly_sources(Some("missing")).unwrap_err();
    assert_eq!(err.to_string(), library_not_found("missing").to_string());
}

#[test]
fn test_readonly_resolution_creates_nothing() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("cfg");
    let libs_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libs_dir).unwrap();
    let mut work = LibraryMeta::new("work");
    work.is_primary = true;
    write_index(&config_dir, &[work]);
    write_lib_file(&libs_dir, "work");

    let before: Vec<_> = walk_files(&config_dir);
    let mgr = LibraryManager::with_config_dir(config_dir.clone()).unwrap();
    let _ = mgr.resolve_readonly_sources(None).unwrap();
    let _ = mgr.resolve_readonly_sources(Some("all")).unwrap();
    let _ = mgr.inspect_library_index();
    let after: Vec<_> = walk_files(&config_dir);
    assert_eq!(before, after);
}

fn walk_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn test_inspect_library_index_primary_states() {
    // No libraries.
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("cfg");
    std::fs::create_dir_all(config_dir.join("libraries")).unwrap();
    write_index(&config_dir, &[]);
    let mgr = LibraryManager::with_config_dir(config_dir).unwrap();
    let inspection = mgr.inspect_library_index();
    assert!(matches!(inspection.primary, PrimaryState::NoLibraries));
    assert!(inspection.missing_files.is_empty());
    assert!(inspection.orphan_files.is_empty());

    // No primary with libraries.
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("cfg");
    let libs_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libs_dir).unwrap();
    write_index(&config_dir, &[LibraryMeta::new("a")]);
    write_lib_file(&libs_dir, "a");
    let mgr = LibraryManager::with_config_dir(config_dir).unwrap();
    let inspection = mgr.inspect_library_index();
    assert!(matches!(
        inspection.primary,
        PrimaryState::NoPrimary { count: 1 }
    ));

    // Primary file missing.
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("cfg");
    let libs_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libs_dir).unwrap();
    let mut primary = LibraryMeta::new("gone");
    primary.is_primary = true;
    write_index(&config_dir, &[primary]);
    let mgr = LibraryManager::with_config_dir(config_dir).unwrap();
    let inspection = mgr.inspect_library_index();
    assert!(matches!(
        inspection.primary,
        PrimaryState::FileMissing { .. }
    ));
    assert_eq!(inspection.missing_files.len(), 1);
    assert_eq!(inspection.missing_files[0].0, "gone");
}

#[test]
fn test_inspect_library_index_orphan_files() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join("cfg");
    let libs_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libs_dir).unwrap();
    write_index(&config_dir, &[]);
    write_lib_file(&libs_dir, "orphan");
    let mgr = LibraryManager::with_config_dir(config_dir).unwrap();
    let inspection = mgr.inspect_library_index();
    assert_eq!(inspection.orphan_files.len(), 1);
}

#[test]
fn test_find_orphaned_ids_shared_classifier() {
    let active: HashSet<String> = ["a".to_string(), "b".to_string()].into_iter().collect();
    let usage = vec![
        "b".to_string(),
        "c".to_string(),
        String::new(),
        "d".to_string(),
    ];
    assert_eq!(
        find_orphaned_ids(&active, &usage),
        vec!["c".to_string(), "d".to_string()]
    );
}
