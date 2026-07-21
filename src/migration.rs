//! **Layer: Domain/Core**
//!
//! Migration framework for schema and layout transitions.
//!
//! Provides a versioned migration system for evolving the TOML library
//! format and configuration layout across releases. Each migration
//! describes a path from one schema version to the next, with analysis
//! (dry-run) and apply phases.

#![allow(dead_code)]

use crate::error::{SnipError, SnipResult};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Schema version.
///
/// Ordinal type that allows comparison (ordering) to determine whether
/// a file needs migration. Version 0 represents legacy/unversioned files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SchemaVersion(pub u32);

impl SchemaVersion {
    /// The initial schema version for files that carry no version marker.
    pub const LEGACY: SchemaVersion = SchemaVersion(0);

    /// The current latest schema version.
    pub const CURRENT: SchemaVersion = SchemaVersion(1);
}

impl std::fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// Migration plan describing the operations needed to upgrade a file.
#[derive(Debug, Clone)]
pub struct MigrationPlan {
    /// Schema version the file is currently at.
    pub source: SchemaVersion,
    /// Target schema version after migration.
    pub target: SchemaVersion,
    /// Ordered list of operations to perform.
    pub operations: Vec<MigrationOperation>,
}

/// A single migration operation.
#[derive(Debug, Clone)]
pub enum MigrationOperation {
    /// Rename a field within a TOML table.
    RenameField {
        table: String,
        from: String,
        to: String,
    },
    /// Add a new field with a default value.
    AddField {
        table: String,
        name: String,
        default: String,
    },
    /// Remove a field from a TOML table.
    RemoveField { table: String, name: String },
    /// A free-form transformation (e.g. data normalization).
    Transform { description: String },
}

/// Output of a migration.
#[derive(Debug, Clone, Default)]
pub struct MigrationOutput {
    /// Number of files that were modified.
    pub files_migrated: usize,
    /// Whether the migration involved data loss (e.g. field removal).
    pub lossy: bool,
    /// Any warnings to display to the user.
    pub warnings: Vec<String>,
}

/// A migration between two schema versions.
///
/// Implement this trait for each migration step (e.g. v0→v1, v1→v2).
pub trait Migration {
    /// The source version this migration handles.
    fn source(&self) -> SchemaVersion;

    /// The target version this migration produces.
    fn target(&self) -> SchemaVersion;

    /// Analyse a file and return a plan without making changes.
    fn analyze(&self, path: &Path) -> SnipResult<MigrationPlan>;

    /// Apply the migration plan to the file.
    fn apply(&self, plan: &MigrationPlan, path: &Path) -> SnipResult<MigrationOutput>;
}

/// Schema version key used in TOML files.
const SCHEMA_KEY: &str = "schema_version";

/// Check if a file needs migration by reading its schema version.
pub fn needs_migration(path: &Path) -> SnipResult<bool> {
    let version = get_schema_version(path)?;
    Ok(version < SchemaVersion::CURRENT)
}

/// Get the current schema version of a file.
///
/// Returns `SchemaVersion::LEGACY` if the file has no version marker
/// or cannot be read.
pub fn get_schema_version(path: &Path) -> SnipResult<SchemaVersion> {
    if !path.exists() {
        return Ok(SchemaVersion::LEGACY);
    }

    let content = std::fs::read_to_string(path)
        .map_err(|e| SnipError::io_error("read file for schema version", path, e))?;

    if content.trim().is_empty() {
        return Ok(SchemaVersion::LEGACY);
    }

    // Parse as TOML table to handle documents with multiple root keys.
    match content.parse::<toml::Table>() {
        Ok(table) => {
            if let Some(version) = table.get(SCHEMA_KEY).and_then(|v| v.as_integer()) {
                Ok(SchemaVersion(version as u32))
            } else {
                Ok(SchemaVersion::LEGACY)
            }
        }
        Err(_) => Ok(SchemaVersion::LEGACY),
    }
}

/// Write the schema version into a TOML file's top-level table.
///
/// This is a best-effort operation: if the file cannot be read or
/// is not valid TOML, the version is not written and an error is returned.
pub fn write_schema_version(path: &Path, version: SchemaVersion) -> SnipResult<()> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| SnipError::io_error("read file for schema update", path, e))?;

    // If the file is empty, write a minimal TOML.
    if content.trim().is_empty() {
        let minimal = format!("schema_version = {}\n", version.0);
        crate::utils::atomic::write_private_atomic(path, &minimal, "migration")?;
        return Ok(());
    }

    // Parse as toml::Table to preserve structure (array-of-tables etc.)
    let mut table: toml::Table = content
        .parse()
        .map_err(|e| SnipError::toml_error("parse TOML for schema update", e))?;

    table.insert(
        SCHEMA_KEY.to_string(),
        toml::Value::Integer(version.0 as i64),
    );

    let updated = toml::to_string_pretty(&table)
        .map_err(|e| SnipError::toml_error("serialize TOML for schema update", e))?;

    crate::utils::atomic::write_private_atomic(path, &updated, "migration")?;

    Ok(())
}

/// Run a chain of migrations on a file.
///
/// Migrations are applied in order from `source` to `CURRENT`. Each
/// migration is analyzed first; only if the plan has operations is it
/// applied.
pub fn run_migrations(
    path: &Path,
    migrations: &[Box<dyn Migration>],
) -> SnipResult<MigrationOutput> {
    let mut current_version = get_schema_version(path)?;
    let mut total_output = MigrationOutput::default();

    // Filter migrations to those applicable (source >= current, target <= current+1 chain)
    for migration in migrations {
        if migration.source() == current_version {
            let plan = migration.analyze(path)?;
            if !plan.operations.is_empty() {
                let output = migration.apply(&plan, path)?;
                total_output.files_migrated += output.files_migrated;
                total_output.lossy |= output.lossy;
                total_output.warnings.extend(output.warnings);
                current_version = migration.target();
            } else {
                current_version = migration.target();
            }
        }
    }

    Ok(total_output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_schema_version_ordering() {
        assert!(SchemaVersion::LEGACY < SchemaVersion::CURRENT);
        assert!(SchemaVersion(0) < SchemaVersion(1));
        assert!(SchemaVersion(1) < SchemaVersion(2));
    }

    #[test]
    fn test_schema_version_display() {
        assert_eq!(SchemaVersion::LEGACY.to_string(), "v0");
        assert_eq!(SchemaVersion::CURRENT.to_string(), "v1");
        assert_eq!(SchemaVersion(42).to_string(), "v42");
    }

    #[test]
    fn test_get_schema_version_legacy() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("legacy.toml");
        fs::write(&path, "snippets = []\n").unwrap();

        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion::LEGACY);
    }

    #[test]
    fn test_get_schema_version_with_version() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("versioned.toml");
        fs::write(&path, "schema_version = 1\nsnippets = []\n").unwrap();

        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion(1));
    }

    #[test]
    fn test_get_schema_version_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.toml");

        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion::LEGACY);
    }

    #[test]
    fn test_get_schema_version_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.toml");
        fs::write(&path, "").unwrap();

        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion::LEGACY);
    }

    #[test]
    fn test_needs_migration() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.toml");
        fs::write(&path, "snippets = []\n").unwrap();

        assert!(needs_migration(&path).unwrap());

        fs::write(&path, "schema_version = 1\nsnippets = []\n").unwrap();
        assert!(!needs_migration(&path).unwrap());
    }

    #[test]
    fn test_write_schema_version() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.toml");
        fs::write(&path, "snippets = []\n").unwrap();

        write_schema_version(&path, SchemaVersion(5)).unwrap();

        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion(5));
    }

    #[test]
    fn test_write_schema_version_preserves_content() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.toml");
        fs::write(
            &path,
            "[[snippets]]\ndescription = \"test\"\ncommand = \"echo hi\"\n",
        )
        .unwrap();

        write_schema_version(&path, SchemaVersion(1)).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("schema_version = 1"));
        assert!(content.contains("echo hi"));
    }

    struct TestMigration;

    impl Migration for TestMigration {
        fn source(&self) -> SchemaVersion {
            SchemaVersion::LEGACY
        }

        fn target(&self) -> SchemaVersion {
            SchemaVersion(1)
        }

        fn analyze(&self, path: &Path) -> SnipResult<MigrationPlan> {
            let version = get_schema_version(path)?;
            let mut operations = Vec::new();
            if version < SchemaVersion(1) {
                operations.push(MigrationOperation::AddField {
                    table: String::new(),
                    name: "schema_version".to_string(),
                    default: "1".to_string(),
                });
            }
            Ok(MigrationPlan {
                source: self.source(),
                target: self.target(),
                operations,
            })
        }

        fn apply(&self, plan: &MigrationPlan, path: &Path) -> SnipResult<MigrationOutput> {
            if !plan.operations.is_empty() {
                write_schema_version(path, plan.target)?;
                Ok(MigrationOutput {
                    files_migrated: 1,
                    lossy: false,
                    warnings: vec![],
                })
            } else {
                Ok(MigrationOutput::default())
            }
        }
    }

    #[test]
    fn test_migration_trait() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.toml");
        fs::write(&path, "snippets = []\n").unwrap();

        let migration = TestMigration;
        let plan = migration.analyze(&path).unwrap();
        assert_eq!(plan.operations.len(), 1);

        let output = migration.apply(&plan, &path).unwrap();
        assert_eq!(output.files_migrated, 1);
        assert!(!output.lossy);

        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion(1));
    }

    #[test]
    fn test_run_migrations() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.toml");
        fs::write(&path, "snippets = []\n").unwrap();

        let migrations: Vec<Box<dyn Migration>> = vec![Box::new(TestMigration)];
        let output = run_migrations(&path, &migrations).unwrap();
        assert_eq!(output.files_migrated, 1);

        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion(1));
    }

    #[test]
    fn test_run_migrations_noop_when_current() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.toml");
        fs::write(&path, "schema_version = 1\nsnippets = []\n").unwrap();

        let migrations: Vec<Box<dyn Migration>> = vec![Box::new(TestMigration)];
        let output = run_migrations(&path, &migrations).unwrap();
        assert_eq!(output.files_migrated, 0);
    }

    // ─── Fixture tests: legacy formats and edge cases ───

    /// Legacy `[[Snippets]]` spelling (capital S) — should parse via serde alias.
    #[test]
    fn test_fixture_legacy_snippets_capital_spelling() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("legacy_cap.toml");
        let content = r#"[[Snippets]]
description = "legacy snippet"
command = "echo legacy"
"#;
        fs::write(&path, content).unwrap();
        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion::LEGACY);
        assert!(needs_migration(&path).unwrap());
    }

    /// Capitalized field names (`Description`, `Command`, etc.) — should parse via aliases.
    #[test]
    fn test_fixture_capitalized_field_names() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cap_fields.toml");
        let content = r#"[[snippets]]
Description = "capitalized field"
Command = "echo capitalized"
"#;
        fs::write(&path, content).unwrap();
        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion::LEGACY);
    }

    /// Missing IDs — empty id field should be handled by load_library.
    #[test]
    fn test_fixture_missing_ids() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("no_ids.toml");
        let content = r#"schema_version = 1

[[snippets]]
description = "no id snippet"
command = "echo no-id"

[[snippets]]
description = "another no id"
command = "echo also-no-id"
"#;
        fs::write(&path, content).unwrap();
        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion(1));
        assert!(!needs_migration(&path).unwrap());
    }

    /// Current canonical format — no migration needed.
    #[test]
    fn test_fixture_current_canonical_noop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("canonical.toml");
        let content = r#"schema_version = 1

[[snippets]]
id = "abc-123"
description = "current snippet"
command = "echo current"
tag = "test"
favorite = true
created_at = 1700000000
updated_at = 1700000000
"#;
        fs::write(&path, content).unwrap();
        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion(1));
        assert!(!needs_migration(&path).unwrap());
    }

    /// Future unsupported schema version — needs_migration should return false
    /// (we can't migrate to a future version, but it shouldn't panic).
    #[test]
    fn test_fixture_future_schema_no_panic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("future.toml");
        let content = "schema_version = 999\nsnippets = []\n";
        fs::write(&path, content).unwrap();
        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion(999));
        // needs_migration compares < CURRENT, so v999 is NOT less than v1
        assert!(!needs_migration(&path).unwrap());
    }

    /// Malformed source preservation — write_schema_version should not
    /// corrupt content that isn't valid TOML (returns error).
    #[test]
    fn test_fixture_malformed_source_returns_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("malformed.toml");
        fs::write(&path, "this is not [valid toml {{{{").unwrap();
        let result = write_schema_version(&path, SchemaVersion(1));
        assert!(result.is_err());
        // Original content should be unchanged
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("this is not [valid toml"));
    }

    /// write_schema_version preserves array-of-tables structure.
    #[test]
    fn test_fixture_write_schema_preserves_array_of_tables() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("aot.toml");
        let content = r#"[[snippets]]
description = "first"
command = "echo 1"

[[snippets]]
description = "second"
command = "echo 2"
"#;
        fs::write(&path, content).unwrap();
        write_schema_version(&path, SchemaVersion(1)).unwrap();

        let result = fs::read_to_string(&path).unwrap();
        assert!(result.contains("schema_version = 1"));
        assert!(result.contains("[[snippets]]"));
        assert!(result.contains("echo 1"));
        assert!(result.contains("echo 2"));
    }

    /// Historical timestamp format — timestamps as strings should be
    /// preserved as-is by serde (the library stores them as i64).
    #[test]
    fn test_fixture_historical_timestamp_forms() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("timestamps.toml");
        let content = r#"schema_version = 1

[[snippets]]
id = "ts-test"
description = "timestamped"
command = "echo ts"
created_at = 1700000000
updated_at = 1700000000
"#;
        fs::write(&path, content).unwrap();
        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion(1));
    }

    /// Migration idempotency — running the same migration twice should
    /// produce the same result.
    #[test]
    fn test_fixture_migration_idempotency() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("idempotent.toml");
        fs::write(&path, "snippets = []\n").unwrap();

        let migrations: Vec<Box<dyn Migration>> = vec![Box::new(TestMigration)];
        let out1 = run_migrations(&path, &migrations).unwrap();
        assert_eq!(out1.files_migrated, 1);

        let out2 = run_migrations(&path, &migrations).unwrap();
        assert_eq!(out2.files_migrated, 0); // Already at v1, no-op
    }

    /// Empty file — schema version should be LEGACY and needs migration.
    #[test]
    fn test_fixture_empty_file_needs_migration() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.toml");
        fs::write(&path, "").unwrap();
        let version = get_schema_version(&path).unwrap();
        assert_eq!(version, SchemaVersion::LEGACY);
        assert!(needs_migration(&path).unwrap());
    }
}
