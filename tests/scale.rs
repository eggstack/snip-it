//! Performance and scale tests for Release 4D.
//!
//! Creates deterministic fixtures representative of large migrated collections
//! and measures bounded algorithmic behavior. Avoids fragile wall-clock assertions.

mod support;

use support::helpers::*;

/// Create a library with `count` snippets for scale testing.
fn create_scale_library(config_dir: &std::path::Path, lib_name: &str, count: usize) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", lib_name]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libraries_dir).unwrap();

    let mut toml = String::new();
    for i in 0..count {
        let fav = if i % 100 == 0 { "true" } else { "false" };
        let updated = 100 + (i as i64);
        let created = 50 + (i as i64);
        let desc = format!("snippet-{i:05}");
        let cmd_str = format!("echo item-{i:05}");
        let tag_group = i % 10;
        toml.push_str(&format!(
            r#"[[Snippets]]
id = "scale-{i:05}"
description = "{desc}"
command = "{cmd_str}"
tags = ["group-{tag_group}"]
output = ""
folders = []
favorite = {fav}
created_at = {created}
updated_at = {updated}

"#
        ));
    }

    std::fs::write(libraries_dir.join(format!("{lib_name}.toml")), &toml).unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", lib_name]);
    cmd.output().unwrap();
}

/// Verify that the fixture was created correctly.
#[test]
fn test_scale_fixture_creation_1000() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-1k", 1000);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1000);
}

// ── Scale: list operations on large library ──

#[test]
fn test_scale_list_1000_default_order() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-list-1k", 1000);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1000);
    // First item should be the first inserted
    assert_eq!(items[0]["description"], "snippet-00000");
}

#[test]
fn test_scale_list_1000_sort_by_description() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-sort-1k", 1000);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "description", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1000);
    // Sorted alphabetically: snippet-00000 < snippet-00001 < ...
    assert_eq!(items[0]["description"], "snippet-00000");
    assert_eq!(items[999]["description"], "snippet-00999");
}

#[test]
fn test_scale_list_1000_sort_by_command() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-cmd-1k", 1000);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "command", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1000);
    assert_eq!(items[0]["command"], "echo item-00000");
    assert_eq!(items[999]["command"], "echo item-00999");
}

#[test]
fn test_scale_list_1000_sort_by_recent() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-recent-1k", 1000);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "recent", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1000);
    // Most recently updated first (updated_at 1099 > 1098 > ...)
    assert_eq!(items[0]["description"], "snippet-00999");
    assert_eq!(items[999]["description"], "snippet-00000");
}

#[test]
fn test_scale_list_1000_favorites_first() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-fav-1k", 1000);

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "list",
        "--sort",
        "description",
        "--favorites-first",
        "--json",
    ]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1000);
    // Favorites first (IDs ending in 00, 100, 200, ..., 900), then non-favorites
    // Favorites: scale-00000, scale-00100, scale-00200, ..., scale-00900 (10 items)
    // All favorites should come before non-favorites
    let last_fav_idx = items
        .iter()
        .rposition(|i| i["favorite"].as_bool().unwrap_or(false))
        .unwrap();
    let first_non_fav_idx = items
        .iter()
        .position(|i| !i["favorite"].as_bool().unwrap_or(false))
        .unwrap();
    assert!(
        last_fav_idx < first_non_fav_idx,
        "All favorites should come before non-favorites"
    );
}

// ── Scale: filter on large library ──

#[test]
fn test_scale_list_1000_filter() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-filter-1k", 1000);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--filter", "00500", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    // Should match snippet-00500 (description and command both contain "00500")
    assert!(
        items.iter().any(|i| i["description"] == "snippet-00500"),
        "Filter should match snippet-00500"
    );
}

// ── Scale: CSV output on large library ──

#[test]
fn test_scale_list_1000_csv() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-csv-1k", 1000);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--csv"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    // Header + 1000 data rows
    assert_eq!(lines.len(), 1001);
    // First data row should contain scale-00000
    assert!(
        lines[1].contains("snippet-00000"),
        "First CSV data row should contain snippet-00000"
    );
}

// ── Scale: JSON output on large library ──

#[test]
fn test_scale_list_1000_json_valid() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-json-1k", 1000);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Must parse as valid JSON array
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1000);

    // Each item should have all required fields
    for item in &items {
        assert!(
            item["description"].is_string(),
            "Each item should have a description"
        );
        assert!(
            item["command"].is_string(),
            "Each item should have a command"
        );
        assert!(item["tags"].is_array(), "Each item should have tags");
        assert!(item["output"].is_string(), "Each item should have output");
    }
}

// ── Scale: duplicate prefixes ──

#[test]
fn test_scale_duplicate_prefixes_sort_deterministically() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "scale-dup"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libraries_dir).unwrap();

    // Create 100 snippets all starting with "deploy-" prefix
    let mut toml = String::new();
    for i in 0..100 {
        toml.push_str(&format!(
            r#"[[Snippets]]
id = "dup-{i:03}"
description = "deploy-service-{i:03}"
command = "deploy.sh --service=service-{i:03}"
tags = ["deploy"]
output = ""
folders = []
favorite = false
created_at = {i}
updated_at = {i}

"#
        ));
    }

    std::fs::write(libraries_dir.join("scale-dup.toml"), &toml).unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "scale-dup"]);
    cmd.output().unwrap();

    // Sort by description - should be deterministic
    let mut cmd1 = snp_in(&config_dir);
    cmd1.args(["list", "--sort", "description", "--json"]);
    let output1 = cmd1.output().unwrap();
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    let items1: Vec<serde_json::Value> = serde_json::from_str(&stdout1).unwrap();

    let mut cmd2 = snp_in(&config_dir);
    cmd2.args(["list", "--sort", "description", "--json"]);
    let output2 = cmd2.output().unwrap();
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    let items2: Vec<serde_json::Value> = serde_json::from_str(&stdout2).unwrap();

    assert_eq!(items1.len(), items2.len());
    for (a, b) in items1.iter().zip(items2.iter()) {
        assert_eq!(
            a["description"], b["description"],
            "Sort should be deterministic across runs"
        );
    }
}

// ── Scale: multiline output in large library ──

#[test]
fn test_scale_multiline_output_preserved() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "scale-ml"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libraries_dir).unwrap();

    // Create 100 snippets with multiline output
    let mut toml = String::new();
    for i in 0..100 {
        toml.push_str(&format!(
            r#"[[Snippets]]
id = "ml-{i:03}"
description = "multiline-{i:03}"
command = "echo ml-{i:03}"
tags = ["multiline"]
output = "line1 for {i}\nline2 for {i}\nline3 for {i}"
folders = []
favorite = false
created_at = {i}
updated_at = {i}

"#
        ));
    }

    std::fs::write(libraries_dir.join("scale-ml.toml"), &toml).unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "scale-ml"]);
    cmd.output().unwrap();

    // Verify JSON roundtrip preserves multiline output
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 100);

    // Check a specific multiline output
    let item50 = items
        .iter()
        .find(|i| i["description"] == "multiline-050")
        .unwrap();
    assert_eq!(
        item50["output"].as_str().unwrap(),
        "line1 for 50\nline2 for 50\nline3 for 50"
    );
}

// ── Scale: usage sidecar entries ──

#[test]
fn test_scale_usage_sidecar_with_large_library() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-usage", 1000);

    // Create a usage.toml with entries for half the snippets
    let mut usage_toml = String::new();
    for i in 0..500 {
        usage_toml.push_str(&format!(
            r#"[[entries]]
id = "scale-{i:05}"
use_count = {count}
last_used_at = {ts}

"#,
            count = i as u64,
            ts = 1700000000 + i as i64,
        ));
    }

    std::fs::write(config_dir.join("usage.toml"), &usage_toml).unwrap();

    // List with sort by last_used should use the sidecar data
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "last-used", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1000);
    // Items with usage data should sort by last_used_at descending
    // scale-00499 has last_used_at = 1700000499 (highest)
    assert_eq!(items[0]["description"], "snippet-00499");
}

// ── Scale: multiple libraries ──

#[test]
fn test_scale_multiple_libraries() {
    let (_tmp, config_dir) = setup_test_env();

    // Create two libraries with 500 snippets each
    create_scale_library(&config_dir, "scale-lib-a", 500);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "scale-lib-b"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    let mut toml_b = String::new();
    for i in 0..500 {
        toml_b.push_str(&format!(
            r#"[[Snippets]]
id = "b-{i:05}"
description = "lib-b-{i:05}"
command = "echo b-{i:05}"
tags = ["lib-b"]
output = ""
folders = []
favorite = false
created_at = {i}
updated_at = {i}

"#
        ));
    }
    std::fs::write(libraries_dir.join("scale-lib-b.toml"), &toml_b).unwrap();

    // List library A
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--library", "scale-lib-a", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let items_a: Vec<serde_json::Value> =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(items_a.len(), 500);

    // List library B
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--library", "scale-lib-b", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let items_b: Vec<serde_json::Value> =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(items_b.len(), 500);
}

// ── Scale: sort ordering with populated usage data ──────────────────────

/// Create a usage.toml with entries for all snippets in the library.
/// Usage count and last_used_at are deliberately divergent from updated_at.
fn write_scale_usage(config_dir: &std::path::Path, count: usize) {
    let mut usage_toml = String::new();
    for i in 0..count {
        // Use count: reverse of index (so snippet-00999 has use_count=999)
        let use_count = (count - 1 - i) as u64;
        // last_used_at: divergent from updated_at (which is 100+i)
        // Make it so snippet-00000 has the highest last_used_at
        let last_used_at = 1700000000 + (count as i64 - 1 - i as i64);
        usage_toml.push_str(&format!(
            r#"[[entries]]
id = "scale-{i:05}"
use_count = {use_count}
last_used_at = {last_used_at}

"#,
        ));
    }
    std::fs::write(config_dir.join("usage.toml"), &usage_toml).unwrap();
}

#[test]
fn test_scale_sort_most_used_with_usage_data() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-mu", 1000);
    write_scale_usage(&config_dir, 1000);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "most-used", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1000);

    // snippet-00000 has use_count=999 (highest)
    assert_eq!(
        items[0]["description"], "snippet-00000",
        "most-used should rank snippet-00000 first (use_count=999)"
    );
    // snippet-00001 has use_count=998
    assert_eq!(
        items[1]["description"], "snippet-00001",
        "most-used should rank snippet-00001 second (use_count=998)"
    );
    // snippet-00999 has use_count=0 (lowest)
    assert_eq!(
        items[999]["description"], "snippet-00999",
        "most-used should rank snippet-00999 last (use_count=0)"
    );
}

#[test]
fn test_scale_sort_last_used_with_usage_data() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-lu", 1000);
    write_scale_usage(&config_dir, 1000);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "last-used", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1000);

    // snippet-00000 has last_used_at=1700000999 (highest)
    assert_eq!(
        items[0]["description"], "snippet-00000",
        "last-used should rank snippet-00000 first (last_used_at highest)"
    );
    // snippet-00001 has last_used_at=1700000998
    assert_eq!(
        items[1]["description"], "snippet-00001",
        "last-used should rank snippet-00001 second"
    );
    // snippet-00999 has last_used_at=1700000000 (lowest)
    assert_eq!(
        items[999]["description"], "snippet-00999",
        "last-used should rank snippet-00999 last (last_used_at lowest)"
    );
}

#[test]
fn test_scale_sort_most_used_vs_recent_divergence() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-div", 1000);
    write_scale_usage(&config_dir, 1000);

    // most-used order: snippet-00000 (use_count=999), snippet-00001 (998), ...
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "most-used", "--json"]);
    let output = cmd.output().unwrap();
    let most_used: Vec<serde_json::Value> =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();

    // recent order: snippet-00999 (updated_at=1099), snippet-00998 (1098), ...
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "recent", "--json"]);
    let output = cmd.output().unwrap();
    let recent: Vec<serde_json::Value> =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();

    // The two orderings must differ when metadata diverges
    assert_ne!(
        most_used[0]["description"], recent[0]["description"],
        "most-used and recent should produce different first items with divergent metadata"
    );
    assert_eq!(most_used[0]["description"], "snippet-00000");
    assert_eq!(recent[0]["description"], "snippet-00999");
}

#[test]
fn test_scale_sort_favorites_first_with_usage_data() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-fav", 1000);
    write_scale_usage(&config_dir, 1000);

    // Favorites-first + most-used: favorites are indices 0, 100, 200, ..., 900
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "most-used", "--favorites-first", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let items: Vec<serde_json::Value> =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(items.len(), 1000);

    // First 10 items should be favorites (indices 0,100,...,900)
    // Within favorites, most-used desc: scale-00000 (use_count=999),
    // scale-00100 (899), scale-00200 (799), ...
    assert_eq!(
        items[0]["description"], "snippet-00000",
        "first favorite by most-used should be snippet-00000 (use_count=999)"
    );
    assert_eq!(
        items[1]["description"], "snippet-00100",
        "second favorite by most-used should be snippet-00100 (use_count=899)"
    );
}

#[test]
fn test_scale_sort_deterministic_with_usage_data() {
    let (_tmp, config_dir) = setup_test_env();
    create_scale_library(&config_dir, "scale-det", 1000);
    write_scale_usage(&config_dir, 1000);

    // Run the same sort twice and verify identical output
    let mut cmd1 = snp_in(&config_dir);
    cmd1.args(["list", "--sort", "most-used", "--json"]);
    let out1 = cmd1.output().unwrap();
    let stdout1 = String::from_utf8_lossy(&out1.stdout);

    let mut cmd2 = snp_in(&config_dir);
    cmd2.args(["list", "--sort", "most-used", "--json"]);
    let out2 = cmd2.output().unwrap();
    let stdout2 = String::from_utf8_lossy(&out2.stdout);

    assert_eq!(
        stdout1, stdout2,
        "most-used sort with usage data must be deterministic across runs"
    );
}
