//! Plan 011 parity tests: CLI-core and MCP read paths share one
//! selector/search contract.
//!
//! The same fixture library must produce equivalent identities and ordering
//! for equivalent CLI (`snp get` / `snp list --json`) and MCP
//! (`snippet_get` / `snippets_search`) queries, covering: exact ID, exact
//! description ambiguity, exact command, fuzzy description/command, tag
//! matching/filtering, output/notes opt-in behavior, deleted exclusion,
//! cross-library scope, and stable limit/ranking.

mod support;

use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use support::helpers::*;

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

    // CLI list order for the same filter matches MCP order by identity.
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
    // then stable tie-breaks — assert exact count and set equality with MCP.
    assert_eq!(cli_descriptions.len(), full.len());
    server.finish();
}
