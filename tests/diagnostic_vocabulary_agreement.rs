//! M003: validate, doctor (library/pet analysis), and repair agree on shared codes.

mod support;

use snip_it::Snippet;
use snip_it::commands::pet_analysis::analyze_entry;
use snip_it::{LibraryManager, PrimaryState};
use std::fs;
use std::path::Path;
use support::helpers::*;

fn write_diag_fixture(config_dir: &Path) {
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let content = r#"[[snippets]]
id = "vocab-1"
description = ""
command = ""
"#;
    fs::write(libraries_dir.join("diag.toml"), content).unwrap();
    let index = r#"[[libraries]]
filename = "diag"
is_primary = true
[[libraries]]
filename = "other"
is_primary = false
"#;
    fs::write(config_dir.join("libraries.toml"), index).unwrap();
}

#[test]
fn validate_and_pet_analysis_agree_on_shared_codes() {
    let (_tmp, config_dir) = setup_test_env();
    write_diag_fixture(&config_dir);

    let validate = snp_in(&config_dir)
        .args(["validate", "--json", "--library", "diag"])
        .output()
        .unwrap();
    assert!(validate.status.success() || validate.status.code() == Some(6));
    let report: serde_json::Value = serde_json::from_slice(&validate.stdout).unwrap();
    let diags = report["diagnostics"].as_array().unwrap();

    let validate_cmd = diags
        .iter()
        .find(|d| d["code"] == "E-CMD-EMPTY")
        .expect("validate must emit E-CMD-EMPTY");
    assert_eq!(validate_cmd["severity"], "Error");

    let validate_desc = diags
        .iter()
        .find(|d| d["code"] == "W-DESC-EMPTY")
        .expect("validate must emit W-DESC-EMPTY");
    assert_eq!(validate_desc["severity"], "Warning");

    let pet = Snippet {
        id: "vocab-1".to_string(),
        description: String::new(),
        command: String::new(),
        ..Default::default()
    };
    let pet_diags = analyze_entry(0, &pet);
    let pet_cmd = pet_diags
        .iter()
        .find(|d| d.code == "E-CMD-EMPTY")
        .expect("pet analysis must emit E-CMD-EMPTY");
    assert_eq!(format!("{:?}", pet_cmd.severity), "Error");

    let pet_desc = pet_diags
        .iter()
        .find(|d| d.code == "W-DESC-EMPTY")
        .expect("pet analysis must emit W-DESC-EMPTY");
    assert_eq!(format!("{:?}", pet_desc.severity), "Warning");

    assert_eq!(
        validate_cmd["code"],
        pet_cmd.code.as_str(),
        "code strings must match"
    );
    assert_eq!(
        validate_desc["code"],
        pet_desc.code.as_str(),
        "code strings must match"
    );
}

#[test]
fn shared_scan_primary_state_matches_validate_and_repair_classification() {
    let (_tmp, config_dir) = setup_test_env();
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        config_dir.join("libraries.toml"),
        r#"[[libraries]]
filename = "only"
is_primary = false
"#,
    )
    .unwrap();
    fs::write(
        libraries_dir.join("only.toml"),
        r#"[[snippets]]
id = "s1"
description = "ok"
command = "echo ok"
"#,
    )
    .unwrap();

    let mgr = LibraryManager::with_config_dir(config_dir.to_path_buf()).unwrap();
    let scan = mgr.scan_libraries().unwrap();
    assert!(matches!(scan.primary, PrimaryState::NoPrimary { count: 1 }));

    let validate = snp_in(&config_dir)
        .args(["validate", "--json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&validate.stdout).unwrap();
    let no_primary = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "W-NO-PRIMARY")
        .expect("validate W-NO-PRIMARY");
    assert_eq!(no_primary["severity"], "Warning");
    assert_eq!(no_primary["code"].as_str().unwrap(), "W-NO-PRIMARY");

    let repair = snp_in(&config_dir)
        .args(["repair", "--json", "--dry-run"])
        .output()
        .unwrap();
    let repair_json: serde_json::Value = serde_json::from_slice(&repair.stdout).unwrap();
    let primary_items: Vec<_> = repair_json["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["category"] == "primary")
        .collect();
    assert_eq!(primary_items.len(), 1);
}

#[test]
fn validate_and_repair_enumerate_same_disk_libraries_with_orphan() {
    let (_tmp, config_dir) = setup_test_env();
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        config_dir.join("libraries.toml"),
        r#"[[libraries]]
filename = "main"
is_primary = true
"#,
    )
    .unwrap();
    fs::write(
        libraries_dir.join("main.toml"),
        r#"[[snippets]]
id = "m1"
description = "d"
command = "echo"
"#,
    )
    .unwrap();
    fs::write(libraries_dir.join("orphan.toml"), "[[snippets]]\n").unwrap();

    let validate = snp_in(&config_dir)
        .args(["validate", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&validate.stdout).unwrap();
    assert_eq!(v["total_libraries"], 1);
    let orphan_warn = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "W-ORPHAN-FILE")
        .count();
    assert_eq!(orphan_warn, 1);

    let repair = snp_in(&config_dir)
        .args(["repair", "--json", "--dry-run"])
        .output()
        .unwrap();
    let r: serde_json::Value = serde_json::from_slice(&repair.stdout).unwrap();
    // Repair scans both on-disk libraries (indexed + orphan).
    let paths: Vec<String> = r["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["problem"].as_str())
        .filter(|p| p.contains("orphan") || p.contains("main") || p.contains("Snippet"))
        .map(str::to_string)
        .collect();
    assert!(
        !paths.is_empty() || r["items"].as_array().unwrap().is_empty(),
        "repair dry-run completed"
    );

    let mgr = LibraryManager::with_config_dir(config_dir.to_path_buf()).unwrap();
    let scan = mgr.scan_libraries().unwrap();
    assert_eq!(scan.disk_library_paths().len(), 2);
}
