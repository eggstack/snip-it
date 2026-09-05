//! End-to-end coverage for the local stdio MCP server.

use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use tempfile::TempDir;

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

    fn initialize(&mut self) -> Value {
        let response = self.request(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "snip-it-test", "version": "1" },
            }),
        );
        self.send(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        }));
        response
    }

    fn call(&mut self, id: u64, name: &str, arguments: Value) -> Value {
        self.request(
            id,
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
    }

    fn finish(self) -> (std::process::ExitStatus, String) {
        drop(self.stdin);
        let output = self.child.wait_with_output().expect("wait for MCP server");
        (
            output.status,
            String::from_utf8(output.stderr).expect("MCP stderr is UTF-8"),
        )
    }
}

fn fixture() -> TempDir {
    let tmp = TempDir::new().expect("temporary directory");
    let config_dir = tmp.path().join(".config").join("snp");
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).expect("library directory");
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
    .expect("library metadata");
    fs::write(
        libraries_dir.join("work.toml"),
        r#"[[snippets]]
id = "work-deploy"
description = "Deploy service"
command = "echo executed > mcp-command-must-not-run"
tag = ["deploy"]

[[snippets]]
id = "work-status"
description = "Git status"
command = "git status"
tag = ["git"]

[[snippets]]
id = "work-duplicate"
description = "Deploy service"
command = "kubectl apply"
tag = ["deploy"]
"#,
    )
    .expect("work library");
    fs::write(
        libraries_dir.join("personal.toml"),
        r#"[[snippets]]
id = "personal-notes"
description = "Personal notes"
command = "echo notes"
"#,
    )
    .expect("personal library");
    tmp
}

fn tool_result(response: &Value) -> &Value {
    response.get("result").expect("successful JSON-RPC result")
}

#[test]
fn initialize_and_tools_list_follow_mcp_handshake() {
    let tmp = fixture();
    let config_dir = tmp.path().join(".config").join("snp");
    let mut server = McpProcess::start(&config_dir);

    let initialize = server.initialize();
    assert_eq!(initialize["jsonrpc"], "2.0");
    assert_eq!(initialize["id"], 1);
    assert_eq!(initialize["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(initialize["result"]["capabilities"], json!({ "tools": {} }));

    let listed_response = server.request(2, "tools/list", json!({}));
    let listed = tool_result(&listed_response);
    let mut names: Vec<&str> = listed["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["snippet_get", "snippets_list", "snippets_search"]
    );
    assert_eq!(listed["tools"].as_array().unwrap().len(), 3);

    let (status, stderr) = server.finish();
    assert!(status.success(), "MCP server failed: {stderr}");
}

#[test]
fn list_search_get_are_read_only_and_structured() {
    let tmp = fixture();
    let config_dir = tmp.path().join(".config").join("snp");
    let mut server = McpProcess::start(&config_dir);
    let _ = server.initialize();

    let listed = tool_result(&server.call(3, "snippets_list", json!({}))).clone();
    assert_eq!(
        listed["structuredContent"]["snippets"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    let search = server.call(
        4,
        "snippets_search",
        json!({ "query": "deploy", "library": "work" }),
    );
    let search_result = tool_result(&search);
    assert_eq!(search_result["isError"], false);
    assert_eq!(
        search_result["structuredContent"]["snippets"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let mcp_descriptions: Vec<&str> = search_result["structuredContent"]["snippets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|snippet| snippet["description"].as_str().unwrap())
        .collect();
    let cli = Command::new(env!("CARGO_BIN_EXE_snp"))
        .args(["list", "--library", "work", "--filter", "deploy", "--json"])
        .env("XDG_CONFIG_HOME", config_dir.parent().unwrap())
        .env("SNP_ALLOW_PLAINTEXT_API_KEY", "true")
        .output()
        .expect("run CLI search fixture");
    assert!(cli.status.success());
    let cli_descriptions: Vec<String> = serde_json::from_slice::<Vec<Value>>(&cli.stdout)
        .unwrap()
        .into_iter()
        .map(|snippet| snippet["description"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(mcp_descriptions, cli_descriptions);

    let got = server.call(5, "snippet_get", json!({ "id": "work-status" }));
    assert_eq!(
        tool_result(&got)["structuredContent"]["command"],
        "git status"
    );

    let missing = server.call(6, "snippet_get", json!({ "id": "missing" }));
    assert_eq!(tool_result(&missing)["isError"], true);
    assert_eq!(
        tool_result(&missing)["structuredContent"]["error"],
        "not_found"
    );

    let ambiguous = server.call(
        7,
        "snippet_get",
        json!({ "description": "deploy service", "library": "work" }),
    );
    assert_eq!(tool_result(&ambiguous)["isError"], true);
    assert_eq!(
        tool_result(&ambiguous)["structuredContent"]["error"],
        "ambiguous"
    );
    assert_eq!(
        tool_result(&ambiguous)["structuredContent"]["matches"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    assert!(
        !tmp.path().join("mcp-command-must-not-run").exists(),
        "MCP must not execute returned command text"
    );
    let (status, stderr) = server.finish();
    assert!(status.success(), "MCP server failed: {stderr}");
}

#[test]
fn malformed_and_oversized_messages_return_errors_without_crashing() {
    let tmp = fixture();
    let config_dir = tmp.path().join(".config").join("snp");
    let mut server = McpProcess::start(&config_dir);

    server.stdin.write_all(b"not json\n").unwrap();
    server.stdin.flush().unwrap();
    let mut line = String::new();
    server.stdout.read_line(&mut line).unwrap();
    let parse_error: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(parse_error["error"]["code"], -32700);

    let malformed_initialize = server.request(9, "initialize", json!({}));
    assert_eq!(malformed_initialize["error"]["code"], -32602);

    let unsupported_initialize = server.request(
        10,
        "initialize",
        json!({
            "protocolVersion": "1.0",
            "capabilities": {},
            "clientInfo": { "name": "snip-it-test", "version": "1" },
        }),
    );
    assert_eq!(unsupported_initialize["error"]["code"], -32602);

    server
        .stdin
        .write_all(format!("{}\n", "x".repeat(1_048_577)).as_bytes())
        .unwrap();
    server.stdin.flush().unwrap();
    line.clear();
    server.stdout.read_line(&mut line).unwrap();
    let oversized_error: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(oversized_error["error"]["code"], -32600);

    let initialize = server.initialize();
    assert!(initialize.get("result").is_some());
    let (status, stderr) = server.finish();
    assert!(status.success(), "MCP server failed: {stderr}");
}

#[test]
fn malformed_library_fails_closed_as_tool_error() {
    let tmp = fixture();
    let config_dir = tmp.path().join(".config").join("snp");
    fs::write(
        config_dir.join("libraries").join("work.toml"),
        "[[snippets]\nthis is malformed",
    )
    .unwrap();
    let mut server = McpProcess::start(&config_dir);
    let _ = server.initialize();
    let response = server.call(8, "snippets_list", json!({}));
    let result = tool_result(&response);
    assert_eq!(result["isError"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("TOML")
    );
    let (status, stderr) = server.finish();
    assert!(status.success(), "MCP server failed: {stderr}");
}
