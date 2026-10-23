//! Plan 011 parity tests: CLI-core and MCP read paths share one
//! selector/search contract.
//!
//! The same fixture library must produce equivalent identities and ordering
//! for equivalent CLI (`snp get` / `snp list --json`) and MCP
//! (`snippet_get` / `snippets_search`) queries, covering: exact ID, exact
//! description ambiguity, exact command, fuzzy description/command, tag
//! matching/filtering, output/notes opt-in behavior, deleted exclusion,
//! cross-library scope, and stable limit/ranking.
//!
//! M006 (search parity verification corrective) widens the net: the three
//! search surfaces named by the parity contract — `snp get --query`,
//! `snp list --filter`, and MCP `snippets_search` — are exercised against
//! the *same* fixture, so a divergence in the shared match primitive
//! (`selector::score_fuzzy_matches`) fails here.
//!
//! Two decided semantics are encoded below:
//!
//! 1. **An empty query is "no filter"** on all three surfaces: it matches
//!    every non-deleted snippet, *including* snippets whose searchable text
//!    is empty.
//! 2. **Cross-library ordering differs by surface family, by design.** The
//!    selector family (`snp get --query`, MCP `snippet_get`) resolves per
//!    library and aggregates, so its order is library → description → snippet
//!    id. The merged-set family (`snp list --filter`, MCP `snippets_search`)
//!    merges first and scores the merged set, so its order is global
//!    relevance. Within a single library the two orders coincide.

mod support;

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use support::helpers::*;

/// Absolute path of the canary command stored in the fixture.
///
/// The command text is deliberately built so it cannot fuzzy-match any query
/// used by this suite (it contains no `d`, so it can never match "deploy"),
/// and the file must never be created by a read-only surface.
const CANARY_PATH: &str = "/tmp/snp-parity-canary-must-not-run";

struct McpProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpProcess {
    fn start(config_dir: &std::path::Path) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_snp"));
        command
            .args(["mcp", "serve"])
            .env("XDG_CONFIG_HOME", config_dir.parent().unwrap())
            .env("SNP_ALLOW_PLAINTEXT_API_KEY", "true")
            .current_dir(config_dir.parent().unwrap().parent().unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("spawn MCP server");
        let stdin = child.stdin.take().expect("MCP stdin");
        let stdout = BufReader::new(child.stdout.take().expect("MCP stdout"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, message: Value) {
        serde_json::to_writer(&mut self.stdin, &message).expect("write MCP request");
        self.stdin.write_all(b"\n").expect("terminate MCP request");
        self.stdin.flush().expect("flush MCP request");
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("read MCP response");
        assert!(!line.is_empty(), "MCP server closed before responding");
        serde_json::from_str(&line).expect("MCP response must be JSON")
    }

    fn initialize(&mut self) {
        self.request(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "snip-it-parity-test", "version": "1" },
            }),
        );
        self.send(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        }));
    }

    fn call(&mut self, id: u64, name: &str, arguments: Value) -> Value {
        self.request(
            id,
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
    }

    /// PID of the running MCP server process.
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn finish(self) {
        drop(self.stdin);
        let output = self.child.wait_with_output().expect("wait for MCP server");
        assert!(
            output.status.success(),
            "MCP server failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn write_parity_fixture(config_dir: &std::path::Path) {
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        config_dir.join("libraries.toml"),
        r#"[[libraries]]
filename = "work"
library_id = "server-work"
is_primary = true

[[libraries]]
filename = "personal"
is_primary = false
"#,
    )
    .unwrap();
    fs::write(
        libraries_dir.join("work.toml"),
        r#"[[snippets]]
id = "work-deploy-1"
description = "Deploy service"
command = "kubectl apply -f deploy.yaml"
tag = ["deploy", "k8s"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[snippets]]
id = "work-deploy-2"
description = "Deploy service"
command = "helm upgrade prod ./chart"
tag = ["deploy"]
output = ""
folders = []
favorite = false
created_at = 200
updated_at = 200

[[snippets]]
id = "work-git-status"
description = "Git status"
command = "git status"
tag = ["git"]
output = ""
folders = []
favorite = true
created_at = 300
updated_at = 300

[[snippets]]
id = "work-tagonly"
description = "Unrelated description"
command = "echo unrelated"
tag = ["uniquetag123"]
output = ""
folders = []
favorite = false
created_at = 400
updated_at = 400

[[snippets]]
id = "work-output-note"
description = "Database backup"
command = "pg_dump mydb"
tag = ["db"]
output = "sample output note with uniqoutput456"
folders = []
favorite = false
created_at = 500
updated_at = 500

[[snippets]]
id = "work-deleted"
description = "Deleted snippet"
command = "echo deleted"
tag = ["deleted"]
output = ""
folders = []
favorite = false
created_at = 600
updated_at = 600
deleted = true

[[snippets]]
id = "work-nonsearchable"
description = ""
command = ""
tag = []
output = ""
folders = ["needlefolder"]
favorite = true
created_at = 700
updated_at = 700

[[snippets]]
id = "work-fav-true"
description = "Kappa marker"
command = "echo kappa one"
tag = []
output = ""
folders = []
favorite = true
created_at = 710
updated_at = 710

[[snippets]]
id = "work-fav-false"
description = "Kappa marker"
command = "echo kappa two"
tag = []
output = ""
folders = []
favorite = false
created_at = 720
updated_at = 720

[[snippets]]
id = "work-canary"
description = "Canary probe"
command = "touch /tmp/snp-parity-canary-must-not-run"
tag = []
output = ""
folders = []
favorite = false
created_at = 730
updated_at = 730
"#,
    )
    .unwrap();
    fs::write(
        libraries_dir.join("personal.toml"),
        r#"[[snippets]]
id = "personal-notes"
description = "Personal notes"
command = "echo notes"
tag = []
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[snippets]]
id = "personal-deploy"
description = "Deploy service"
command = "echo personal deploy"
tag = ["deploy"]
output = ""
folders = []
favorite = false
created_at = 200
updated_at = 200
"#,
    )
    .unwrap();
}

fn tool_content(response: &Value) -> &Value {
    &response["result"]["structuredContent"]
}

fn cli_list_ids(config_dir: &std::path::Path, extra: &[&str]) -> Vec<String> {
    let mut args = vec!["list", "--json"];
    args.extend_from_slice(extra);
    let output = snp_in(config_dir).args(&args).output().unwrap();
    assert!(
        output.status.success(),
        "snp list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<Vec<Value>>(&output.stdout)
        .unwrap()
        .iter()
        .map(|item| {
            // `list --json` omits IDs; re-derive identity via description+command.
            format!(
                "{}||{}",
                item["description"].as_str().unwrap(),
                item["command"].as_str().unwrap()
            )
        })
        .collect()
}

/// Runs `snp get --query <query> --resolution all --json` and returns the
/// raw per-snippet envelopes in the order the CLI returned them.
///
/// `snp get` exits 3 (NOT_FOUND) and prints nothing when the query matched
/// nothing; that outcome maps to an empty vector. Every other non-zero exit
/// is a test failure.
fn cli_get_envelopes(config_dir: &std::path::Path, query: &str, extra: &[&str]) -> Vec<Value> {
    let mut args = vec!["get", "--query", query, "--resolution", "all", "--json"];
    args.extend_from_slice(extra);
    let output = snp_in(config_dir).args(&args).output().unwrap();
    if output.status.code() == Some(3) {
        // NOT_FOUND: no match, nothing printed.
        return Vec::new();
    }
    assert!(
        output.status.success(),
        "snp get --query {:?} failed with {:?}: {}",
        query,
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "snp get --query {:?} did not emit JSON: {e}; stderr: {}",
            query,
            String::from_utf8_lossy(&output.stderr)
        )
    });
    match parsed {
        // `--resolution all` emits one array of envelopes; a single match
        // through the cross-library aggregate path emits one bare object.
        Value::Array(items) => items,
        other => vec![other],
    }
}

/// `"description||command"` identity of every snippet `snp get --query`
/// returned, in CLI order.
fn cli_get_pairs(config_dir: &std::path::Path, query: &str, extra: &[&str]) -> Vec<String> {
    cli_get_envelopes(config_dir, query, extra)
        .iter()
        .map(|item| {
            format!(
                "{}||{}",
                item["description"].as_str().unwrap(),
                item["command"].as_str().unwrap()
            )
        })
        .collect()
}

/// Snippet IDs of every snippet `snp get --query` returned, in CLI order.
fn cli_get_ids(config_dir: &std::path::Path, query: &str, extra: &[&str]) -> Vec<String> {
    cli_get_envelopes(config_dir, query, extra)
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect()
}

fn mcp_search_ids(server: &mut McpProcess, id: u64, arguments: Value) -> Vec<String> {
    let response = server.call(id, "snippets_search", arguments);
    assert_eq!(response["result"]["isError"], false);
    tool_content(&response)["snippets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect()
}

/// `"description||command"` identity of every MCP `snippets_search` result,
/// in MCP order — directly comparable with [`cli_get_pairs`] and
/// [`cli_list_ids`].
fn mcp_search_pairs(server: &mut McpProcess, id: u64, arguments: Value) -> Vec<String> {
    let response = server.call(id, "snippets_search", arguments);
    assert_eq!(response["result"]["isError"], false);
    tool_content(&response)["snippets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            format!(
                "{}||{}",
                item["description"].as_str().unwrap(),
                item["command"].as_str().unwrap()
            )
        })
        .collect()
}

/// Snapshot of the library material: every file under `libraries/` plus the
/// `libraries.toml` index, keyed by path and valued by raw bytes.
///
/// Deliberately excludes the rest of the config dir — the app writes rotating
/// log files under `<config>/logs/`, which is expected and unrelated to
/// library metadata.
fn snapshot_library_material(config_dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut snapshot = BTreeMap::new();
    collect_tree(&config_dir.join("libraries"), &mut snapshot);
    let index = config_dir.join("libraries.toml");
    snapshot.insert(index.clone(), fs::read(&index).unwrap());
    snapshot
}

fn collect_tree(dir: &Path, snapshot: &mut BTreeMap<PathBuf, Vec<u8>>) {
    if !dir.exists() {
        return;
    }
    for entry in fs::read_dir(dir).expect("read library directory") {
        let entry = entry.expect("library directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_tree(&path, snapshot);
        } else {
            snapshot.insert(path.clone(), fs::read(&path).expect("read library file"));
        }
    }
}

#[test]
fn exact_id_cli_and_mcp_agree() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    let cli = snp_in(&config_dir)
        .args(["get", "--id", "work-git-status"])
        .output()
        .unwrap();
    assert!(cli.status.success());
    assert_eq!(String::from_utf8_lossy(&cli.stdout).trim(), "git status");

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let got = server.call(10, "snippet_get", json!({ "id": "work-git-status" }));
    assert_eq!(tool_content(&got)["command"], "git status");
    assert_eq!(tool_content(&got)["id"], "work-git-status");
    server.finish();
}

#[test]
fn exact_description_ambiguity_parity() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    // Primary scope (`work`) holds two "Deploy service" snippets.
    let cli = snp_in(&config_dir)
        .args(["get", "--description-exact", "Deploy service"])
        .output()
        .unwrap();
    assert_eq!(cli.status.code(), Some(5));

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let ambiguous = server.call(
        11,
        "snippet_get",
        json!({ "description": "Deploy service", "library": "work" }),
    );
    assert_eq!(tool_content(&ambiguous)["error"], "ambiguous");
    assert_eq!(
        tool_content(&ambiguous)["matches"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    server.finish();
}

#[test]
fn exact_command_cli_and_mcp_agree() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    let cli = snp_in(&config_dir)
        .args(["get", "--command-exact", "git status"])
        .output()
        .unwrap();
    assert!(cli.status.success());
    assert_eq!(String::from_utf8_lossy(&cli.stdout).trim(), "git status");

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let got = server.call(12, "snippet_get", json!({ "command": "git status" }));
    assert_eq!(tool_content(&got)["id"], "work-git-status");
    assert_eq!(tool_content(&got)["command"], "git status");

    // Exactly-one-of enforcement: id + command together is a tool error.
    let both = server.call(
        13,
        "snippet_get",
        json!({ "id": "work-git-status", "command": "git status" }),
    );
    assert_eq!(both["result"]["isError"], true);
    server.finish();
}

#[test]
fn fuzzy_description_command_order_parity() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    let cli_ids = cli_list_ids(&config_dir, &["--library", "work", "--filter", "git stat"]);
    assert_eq!(cli_ids.len(), 1);
    assert!(cli_ids[0].contains("Git status"));

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let ids = mcp_search_ids(
        &mut server,
        14,
        json!({ "query": "git stat", "library": "work" }),
    );
    assert_eq!(ids, vec!["work-git-status".to_string()]);
    server.finish();
}

#[test]
fn tag_matching_and_explicit_tag_filter_parity() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    // Implicit tag-text matching: the tag-only snippet has no query text in
    // its description or command, so a match proves tags participate.
    let cli_ids = cli_list_ids(
        &config_dir,
        &["--library", "work", "--filter", "uniquetag123"],
    );
    assert_eq!(cli_ids.len(), 1);
    assert!(cli_ids[0].contains("Unrelated description"));

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let ids = mcp_search_ids(
        &mut server,
        15,
        json!({ "query": "uniquetag123", "library": "work" }),
    );
    assert_eq!(ids, vec!["work-tagonly".to_string()]);

    // Explicit `tags` filter with an empty query behaves as a tag listing.
    let tagged = server.call(
        16,
        "snippets_search",
        json!({ "query": "", "library": "work", "tags": ["uniquetag123"] }),
    );
    assert_eq!(tagged["result"]["isError"], false);
    let snippets = tool_content(&tagged)["snippets"].as_array().unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0]["id"], "work-tagonly");

    // Case-insensitive tag filter.
    let upper = server.call(
        17,
        "snippets_search",
        json!({ "query": "", "library": "work", "tags": ["UNIQUETAG123"] }),
    );
    assert_eq!(
        tool_content(&upper)["snippets"].as_array().unwrap().len(),
        1
    );
    server.finish();
}

#[test]
fn output_notes_opt_in_parity() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    // Default: output text is not searchable.
    let cli_default = cli_list_ids(
        &config_dir,
        &["--library", "work", "--filter", "uniqoutput456"],
    );
    assert_eq!(cli_default.len(), 0);
    let cli_opt_in = cli_list_ids(
        &config_dir,
        &[
            "--library",
            "work",
            "--filter",
            "uniqoutput456",
            "--search-output",
        ],
    );
    assert_eq!(cli_opt_in.len(), 1);
    assert!(cli_opt_in[0].contains("Database backup"));

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let default_ids = mcp_search_ids(
        &mut server,
        18,
        json!({ "query": "uniqoutput456", "library": "work" }),
    );
    assert_eq!(default_ids.len(), 0);
    let opt_in_ids = mcp_search_ids(
        &mut server,
        19,
        json!({ "query": "uniqoutput456", "library": "work", "search_output": true }),
    );
    assert_eq!(opt_in_ids, vec!["work-output-note".to_string()]);
    server.finish();
}

#[test]
fn deleted_snippets_excluded_everywhere() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    let cli_get = snp_in(&config_dir)
        .args(["get", "--id", "work-deleted"])
        .output()
        .unwrap();
    assert_eq!(cli_get.status.code(), Some(3));

    let cli_list = cli_list_ids(
        &config_dir,
        &["--library", "work", "--filter", "Deleted snippet"],
    );
    assert_eq!(cli_list.len(), 0);

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let missing = server.call(20, "snippet_get", json!({ "id": "work-deleted" }));
    assert_eq!(tool_content(&missing)["error"], "not_found");
    let ids = mcp_search_ids(
        &mut server,
        21,
        json!({ "query": "Deleted snippet", "library": "work" }),
    );
    assert_eq!(ids.len(), 0);
    server.finish();
}

#[test]
fn cross_library_scope_parity() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    // `all` scope sees work(2) + personal(1) "Deploy service" snippets.
    let cli = snp_in(&config_dir)
        .args([
            "get",
            "--description-exact",
            "Deploy service",
            "--library",
            "all",
        ])
        .output()
        .unwrap();
    assert_eq!(cli.status.code(), Some(5));

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let ambiguous = server.call(
        22,
        "snippet_get",
        json!({ "description": "Deploy service", "library": "all" }),
    );
    assert_eq!(tool_content(&ambiguous)["error"], "ambiguous");
    let matches = tool_content(&ambiguous)["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 3);

    // Deterministic order: library → description → ID.
    let libraries: Vec<&str> = matches
        .iter()
        .map(|m| m["library"].as_str().unwrap())
        .collect();
    assert_eq!(libraries, vec!["personal", "work", "work"]);
    server.finish();
}

#[test]
fn stable_limit_and_ranking_parity() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let full = mcp_search_ids(
        &mut server,
        23,
        json!({ "query": "deploy", "library": "work" }),
    );
    assert_eq!(full.len(), 2);
    let limited = mcp_search_ids(
        &mut server,
        24,
        json!({ "query": "deploy", "library": "work", "limit": 1 }),
    );
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0], full[0]);

    // Repeating the query yields identical order (deterministic ranking).
    let again = mcp_search_ids(
        &mut server,
        25,
        json!({ "query": "deploy", "library": "work" }),
    );
    assert_eq!(again, full);

    // CLI list order for the same filter matches MCP order by identity:
    // both surfaces are the merged-set family, so both rank the merged set
    // by global relevance.
    let cli = snp_in(&config_dir)
        .args(["list", "--library", "work", "--filter", "deploy", "--json"])
        .output()
        .unwrap();
    assert!(cli.status.success());
    let cli_items: Vec<Value> = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(cli_items.len(), 2);
    let cli_descriptions: Vec<String> = cli_items
        .iter()
        .map(|item| {
            format!(
                "{}||{}",
                item["description"].as_str().unwrap(),
                item["command"].as_str().unwrap()
            )
        })
        .collect();
    // Both deploy snippets share the description; order follows relevance
    // then stable tie-breaks. The comparison below is order-sensitive: the
    // two ordered vectors must be equal, not merely the same set.
    assert_eq!(cli_descriptions.len(), full.len());
    let mcp_search = server.call(
        26,
        "snippets_search",
        json!({ "query": "deploy", "library": "work" }),
    );
    let mcp_keys: Vec<String> = tool_content(&mcp_search)["snippets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            format!(
                "{}||{}",
                item["description"].as_str().unwrap(),
                item["command"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(cli_descriptions, mcp_keys);
    server.finish();
}

// ---------------------------------------------------------------------------
// M006 — three-surface parity against one fixture.
// ---------------------------------------------------------------------------

#[test]
fn get_query_parity_matches_list_and_mcp() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    let list_pairs = cli_list_ids(&config_dir, &["--library", "work", "--filter", "git stat"]);
    let get_pairs = cli_get_pairs(&config_dir, "git stat", &["--library", "work"]);
    let get_ids = cli_get_ids(&config_dir, "git stat", &["--library", "work"]);

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let mcp_pairs = mcp_search_pairs(
        &mut server,
        30,
        json!({ "query": "git stat", "library": "work" }),
    );
    let mcp_ids = mcp_search_ids(
        &mut server,
        31,
        json!({ "query": "git stat", "library": "work" }),
    );

    // Same identifiers, in the same order, on all three surfaces.
    assert_eq!(get_pairs, list_pairs);
    assert_eq!(get_pairs, mcp_pairs);
    // `get` and MCP both expose the snippet id, so they are comparable
    // directly.
    assert_eq!(get_ids, mcp_ids);
    assert_eq!(get_ids, vec!["work-git-status".to_string()]);
    server.finish();
}

#[test]
fn within_one_library_all_three_surfaces_agree_on_order() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    // "deploy" matches two snippets inside `work`.
    let list_pairs = cli_list_ids(&config_dir, &["--library", "work", "--filter", "deploy"]);
    let get_pairs = cli_get_pairs(&config_dir, "deploy", &["--library", "work"]);

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let mcp_pairs = mcp_search_pairs(
        &mut server,
        32,
        json!({ "query": "deploy", "library": "work" }),
    );

    assert_eq!(list_pairs.len(), 2);
    // Ordered equality, not just set equality: within one library both
    // families collapse to the same relevance order.
    assert_eq!(get_pairs, list_pairs);
    assert_eq!(mcp_pairs, list_pairs);
    server.finish();
}

#[test]
fn empty_query_is_no_filter_on_all_three_surfaces() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    let list_pairs = cli_list_ids(&config_dir, &["--library", "work", "--filter", ""]);
    let get_pairs = cli_get_pairs(&config_dir, "", &["--library", "work"]);
    let get_ids = cli_get_ids(&config_dir, "", &["--library", "work"]);

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let mcp_pairs = mcp_search_pairs(&mut server, 33, json!({ "query": "", "library": "work" }));
    let mcp_ids = mcp_search_ids(&mut server, 34, json!({ "query": "", "library": "work" }));

    // `work` holds 9 live snippets; the 10th (`work-deleted`) is deleted.
    assert_eq!(list_pairs.len(), 9);
    assert_eq!(get_pairs.len(), 9);
    assert_eq!(mcp_pairs.len(), 9);

    // The empty-searchable-text snippet is part of the empty-query result:
    // an empty query filters nothing, so it cannot drop it.
    assert!(get_ids.contains(&"work-nonsearchable".to_string()));
    assert!(mcp_ids.contains(&"work-nonsearchable".to_string()));
    // `list --json` omits ids, so the empty-text snippet shows up there as
    // the only entry with an empty description *and* an empty command.
    assert!(list_pairs.contains(&"||".to_string()));

    // Identity sets are equal across the three surfaces.
    let mut list_sorted = list_pairs.clone();
    list_sorted.sort();
    let mut get_sorted = get_pairs.clone();
    get_sorted.sort();
    let mut mcp_sorted = mcp_pairs.clone();
    mcp_sorted.sort();
    assert_eq!(list_sorted, get_sorted);
    assert_eq!(list_sorted, mcp_sorted);
    server.finish();
}

#[test]
fn empty_query_matches_plain_list() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    let with_filter = cli_list_ids(&config_dir, &["--library", "work", "--filter", ""]);
    let without_filter = cli_list_ids(&config_dir, &["--library", "work"]);

    assert_eq!(with_filter.len(), 9);
    assert_eq!(with_filter, without_filter);
}

#[test]
fn cross_library_order_is_selector_family_order() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    // Selector family: `get` resolves per library and aggregates, so its
    // cross-library order is library → description → snippet id.
    let get_ids = cli_get_ids(&config_dir, "deploy", &["--library", "all"]);
    assert_eq!(
        get_ids,
        vec![
            "personal-deploy".to_string(),
            "work-deploy-1".to_string(),
            "work-deploy-2".to_string(),
        ]
    );

    // Merged-set family: `list --filter` and MCP `snippets_search` merge the
    // libraries first and score the merged set, so their order is global
    // relevance. The two agree with each other order-sensitively.
    let list_pairs = cli_list_ids(&config_dir, &["--library", "all", "--filter", "deploy"]);
    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let mcp_pairs = mcp_search_pairs(
        &mut server,
        35,
        json!({ "query": "deploy", "library": "all" }),
    );

    assert_eq!(list_pairs.len(), 3);
    assert_eq!(mcp_pairs.len(), 3);
    assert_eq!(list_pairs, mcp_pairs);

    // The merged-set family's order is global relevance over the merged set
    // (the libraries are merged in `libraries.toml` order: work, then
    // personal). It is asserted explicitly — it is a different contract from
    // the selector family's library → description → id order above, and the
    // two are NOT equal.
    assert_eq!(
        list_pairs,
        vec![
            "Deploy service||kubectl apply -f deploy.yaml".to_string(),
            "Deploy service||helm upgrade prod ./chart".to_string(),
            "Deploy service||echo personal deploy".to_string(),
        ]
    );

    // Both families select the same three snippets; the orders are asserted
    // separately because they are different contracts.
    let mut expected_pairs = vec![
        "Deploy service||echo personal deploy".to_string(),
        "Deploy service||helm upgrade prod ./chart".to_string(),
        "Deploy service||kubectl apply -f deploy.yaml".to_string(),
    ];
    expected_pairs.sort();
    let mut list_set = list_pairs.clone();
    list_set.sort();
    let mut mcp_set = mcp_pairs.clone();
    mcp_set.sort();
    assert_eq!(list_set, expected_pairs);
    assert_eq!(mcp_set, expected_pairs);
    server.finish();
}

#[test]
fn non_searchable_fields_are_never_returned() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    // `folders` is organizational metadata and is never searchable.
    let list = cli_list_ids(
        &config_dir,
        &["--library", "work", "--filter", "needlefolder"],
    );
    assert_eq!(list.len(), 0);
    let get = cli_get_pairs(&config_dir, "needlefolder", &["--library", "work"]);
    assert_eq!(get.len(), 0);

    // The snippet id is sync metadata, not searchable text.
    let list = cli_list_ids(
        &config_dir,
        &["--library", "work", "--filter", "work-nonsearchable"],
    );
    assert_eq!(list.len(), 0);
    let get = cli_get_pairs(&config_dir, "work-nonsearchable", &["--library", "work"]);
    assert_eq!(get.len(), 0);

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let folders = mcp_search_ids(
        &mut server,
        36,
        json!({ "query": "needlefolder", "library": "work" }),
    );
    assert_eq!(folders.len(), 0);
    let id_query = mcp_search_ids(
        &mut server,
        37,
        json!({ "query": "work-nonsearchable", "library": "work" }),
    );
    assert_eq!(id_query.len(), 0);

    // `favorite` is never a filter: the favourite and the non-favourite
    // snippet both match "kappa".
    let list = cli_list_ids(&config_dir, &["--library", "work", "--filter", "kappa"]);
    let get = cli_get_pairs(&config_dir, "kappa", &["--library", "work"]);
    let get_ids = cli_get_ids(&config_dir, "kappa", &["--library", "work"]);
    let mcp = mcp_search_pairs(
        &mut server,
        38,
        json!({ "query": "kappa", "library": "work" }),
    );
    let mcp_ids = mcp_search_ids(
        &mut server,
        41,
        json!({ "query": "kappa", "library": "work" }),
    );

    assert!(list.contains(&"Kappa marker||echo kappa one".to_string()));
    assert!(list.contains(&"Kappa marker||echo kappa two".to_string()));
    assert!(get.contains(&"Kappa marker||echo kappa one".to_string()));
    assert!(get.contains(&"Kappa marker||echo kappa two".to_string()));
    assert!(mcp.contains(&"Kappa marker||echo kappa one".to_string()));
    assert!(mcp.contains(&"Kappa marker||echo kappa two".to_string()));

    // By id: `work-fav-true` (favorite = true) and `work-fav-false`
    // (favorite = false) are both present, so the favorite flag never
    // excludes a snippet from matching.
    assert!(get_ids.contains(&"work-fav-true".to_string()));
    assert!(get_ids.contains(&"work-fav-false".to_string()));
    assert!(mcp_ids.contains(&"work-fav-true".to_string()));
    assert!(mcp_ids.contains(&"work-fav-false".to_string()));

    // All three surfaces still agree on the whole result set.
    assert_eq!(get, list);
    assert_eq!(mcp, list);
    server.finish();
}

#[test]
fn get_query_never_searches_output() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    // `snp get` has no output/notes opt-in, so the shared projection it uses
    // excludes the output field entirely.
    let get = cli_get_pairs(&config_dir, "uniqoutput456", &["--library", "work"]);
    assert_eq!(get.len(), 0);

    // `snp list --search-output` opts the output field in and finds it.
    let list = cli_list_ids(
        &config_dir,
        &[
            "--library",
            "work",
            "--filter",
            "uniqoutput456",
            "--search-output",
        ],
    );
    assert_eq!(list.len(), 1);
    assert_eq!(list[0], "Database backup||pg_dump mydb");
}

#[test]
fn deleted_snippets_excluded_by_get_query() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    // The deleted snippet's own text must not be reachable through the
    // fuzzy query surface either.
    let output = snp_in(&config_dir)
        .args(["get", "--query", "Deleted snippet", "--library", "work"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());

    let get = cli_get_pairs(&config_dir, "Deleted snippet", &["--library", "work"]);
    assert_eq!(get.len(), 0);
}

#[test]
fn mcp_search_does_not_execute_and_binds_no_port() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    let canary = PathBuf::from(CANARY_PATH);
    let _ = fs::remove_file(&canary);
    assert!(
        !canary.exists(),
        "canary path must not exist before the test"
    );

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let ids = mcp_search_ids(&mut server, 39, json!({ "query": "", "library": "work" }));
    assert_eq!(ids.len(), 9);

    // A stored command is never executed by a read-only surface.
    assert!(
        !canary.exists(),
        "MCP snippets_search must not execute stored snippet commands"
    );

    // `snp mcp serve` is stdio-only: the process holds no socket fd.
    #[cfg(target_os = "linux")]
    assert_no_socket_fds(server.pid());

    server.finish();
}

/// Linux-only: assert the process holds no socket file descriptor.
///
/// The MCP server is stdio-only, so every fd it owns must point at a file,
/// pipe, or device — never at a socket. A socket fd would mean the server
/// bound a port it is not supposed to bind.
#[cfg(target_os = "linux")]
fn assert_no_socket_fds(pid: u32) {
    let fd_dir = format!("/proc/{pid}/fd");
    let entries = fs::read_dir(&fd_dir).unwrap_or_else(|e| {
        panic!("cannot read {fd_dir}: {e}");
    });
    for entry in entries {
        let entry = entry.expect("fd entry");
        let path = entry.path();
        // The fd may be closed between listing and reading the link.
        let Ok(target) = fs::read_link(&path) else {
            continue;
        };
        let target = target.to_string_lossy().to_string();
        assert!(
            !target.starts_with("socket:["),
            "MCP server must not hold a socket fd, found {target} at {}",
            path.display()
        );
    }
}

#[test]
fn read_only_search_does_not_rewrite_library_metadata() {
    let (_tmp, config_dir) = setup_test_env();
    write_parity_fixture(&config_dir);

    let before = snapshot_library_material(&config_dir);

    let list = cli_list_ids(&config_dir, &["--library", "work", "--filter", ""]);
    assert_eq!(list.len(), 9);
    let get = cli_get_pairs(&config_dir, "", &["--library", "work"]);
    assert_eq!(get.len(), 9);

    let mut server = McpProcess::start(&config_dir);
    server.initialize();
    let ids = mcp_search_ids(&mut server, 40, json!({ "query": "", "library": "work" }));
    assert_eq!(ids.len(), 9);
    server.finish();

    let after = snapshot_library_material(&config_dir);
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>(),
        "read-only search must not create or remove library files"
    );
    for (path, bytes) in &before {
        assert_eq!(
            after.get(path),
            Some(bytes),
            "read-only search must not rewrite {}",
            path.display()
        );
    }
}
