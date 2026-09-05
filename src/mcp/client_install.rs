//! Safe client registration and schema-specific MCP setup instructions.

use crate::{SnipError, SnipResult};
use clap::ValueEnum;
use serde_json::json;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Supported client registration targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum McpClient {
    #[value(name = "claude")]
    Claude,
    #[value(name = "codex")]
    Codex,
    #[value(name = "vscode")]
    VsCode,
    #[value(name = "cursor")]
    Cursor,
    #[value(name = "opencode")]
    OpenCode,
    #[value(name = "zed")]
    Zed,
}

impl McpClient {
    fn command(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::VsCode => "code",
            Self::Cursor => "cursor",
            Self::OpenCode => "opencode",
            Self::Zed => "zed",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::VsCode => "VS Code",
            Self::Cursor => "Cursor",
            Self::OpenCode => "OpenCode",
            Self::Zed => "Zed",
        }
    }
}

pub fn print_instructions(client: McpClient) -> SnipResult<()> {
    let executable = current_executable()?;
    print_instructions_for(client, &executable, false);
    Ok(())
}

pub fn install(client: McpClient) -> SnipResult<()> {
    let executable = current_executable()?;
    let Some(arguments) = official_invocation(client, &executable) else {
        eprintln!(
            "{} has no stable noninteractive MCP registration contract; no config was changed.",
            client.label()
        );
        print_instructions_for(client, &executable, true);
        return Ok(());
    };

    let mut command = Command::new(client.command());
    command
        .args(&arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    match command.status() {
        Ok(status) if status.success() => {
            println!("Registered snip-it with {}.", client.label());
            Ok(())
        }
        Ok(status) => Err(SnipError::runtime_error(
            "MCP client registration failed",
            Some(&format!(
                "{} exited with {}. Re-run the printed instructions if needed.",
                client.label(),
                status
            )),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "{} CLI '{}' was not found; no config was changed.",
                client.label(),
                client.command()
            );
            print_instructions_for(client, &executable, true);
            Ok(())
        }
        Err(error) => Err(SnipError::io_error(
            "launch MCP client registration",
            PathBuf::from(client.command()),
            error,
        )),
    }
}

fn current_executable() -> SnipResult<PathBuf> {
    std::env::current_exe()
        .map_err(|error| SnipError::io_error("locate snp executable", "current executable", error))
}

fn official_invocation(client: McpClient, executable: &PathBuf) -> Option<Vec<String>> {
    match client {
        McpClient::Claude => Some(vec![
            "mcp".into(),
            "add".into(),
            "snip-it".into(),
            "--scope".into(),
            "user".into(),
            "--".into(),
            executable.display().to_string(),
            "mcp".into(),
            "serve".into(),
        ]),
        McpClient::Codex => Some(vec![
            "mcp".into(),
            "add".into(),
            "snip-it".into(),
            "--".into(),
            executable.display().to_string(),
            "mcp".into(),
            "serve".into(),
        ]),
        McpClient::VsCode => Some(vec![
            "--add-mcp".into(),
            serde_json::to_string(&vscode_config(executable))
                .expect("static MCP config is serializable"),
        ]),
        McpClient::Cursor | McpClient::OpenCode | McpClient::Zed => None,
    }
}

fn vscode_config(executable: &PathBuf) -> serde_json::Value {
    json!({
        "name": "snip-it",
        "command": executable,
        "args": ["mcp", "serve"],
    })
}

fn print_instructions_for(client: McpClient, executable: &PathBuf, fallback: bool) {
    if fallback {
        println!("Manual setup for {}:", client.label());
    } else {
        println!("{} MCP setup:", client.label());
    }
    match client {
        McpClient::Claude => println!(
            "  claude mcp add snip-it --scope user -- {} mcp serve",
            shell_quote(&executable.display().to_string())
        ),
        McpClient::Codex => println!(
            "  codex mcp add snip-it -- {} mcp serve",
            shell_quote(&executable.display().to_string())
        ),
        McpClient::VsCode => {
            println!("  Official CLI (user profile):");
            println!(
                "  code --add-mcp {}",
                shell_quote(&vscode_config(executable).to_string())
            );
            println!("  Configuration object:");
            print_json(&vscode_config(executable));
        }
        McpClient::Cursor => {
            println!(
                "  Add this entry to ~/.cursor/mcp.json (global) or .cursor/mcp.json (project):"
            );
            print_json(&json!({
                "mcpServers": {
                    "snip-it": {
                        "command": executable,
                        "args": ["mcp", "serve"],
                    }
                }
            }));
            println!(
                "  Cursor also supports its official MCP install deeplink flow; review the prompt before accepting it."
            );
        }
        McpClient::OpenCode => {
            println!("  Current OpenCode v2 config (opencode.jsonc):");
            print_json(&json!({
                "$schema": "https://opencode.ai/config.json",
                "mcp": {
                    "servers": {
                        "snip-it": {
                            "type": "local",
                            "command": [executable, "mcp", "serve"],
                        }
                    }
                }
            }));
            println!(
                "  OpenCode's guided `opencode mcp add` command is interactive, so snp does not invoke it automatically."
            );
        }
        McpClient::Zed => {
            println!("  Merge this entry into Zed settings (use `zed: open settings file`):");
            print_json(&json!({
                "context_servers": {
                    "snip-it": {
                        "command": executable,
                        "args": ["mcp", "serve"],
                        "env": {},
                    }
                }
            }));
        }
    }
}

fn print_json(value: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("static MCP config is serializable")
    );
}

fn shell_quote(value: &str) -> String {
    let path = value.to_string();
    if path
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._/-\\:".contains(character))
    {
        path
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_invocations_use_exact_server_arguments() {
        let executable = PathBuf::from("/tmp/snp with space");
        assert_eq!(
            official_invocation(McpClient::Codex, &executable).unwrap(),
            vec![
                "mcp",
                "add",
                "snip-it",
                "--",
                "/tmp/snp with space",
                "mcp",
                "serve"
            ]
        );
        let vscode = official_invocation(McpClient::VsCode, &executable).unwrap();
        assert_eq!(vscode[0], "--add-mcp");
        assert!(vscode[1].contains("snip-it"));
    }

    #[test]
    fn unsafe_clients_have_no_automatic_invocation() {
        let executable = PathBuf::from("/tmp/snp");
        assert!(official_invocation(McpClient::Cursor, &executable).is_none());
        assert!(official_invocation(McpClient::OpenCode, &executable).is_none());
        assert!(official_invocation(McpClient::Zed, &executable).is_none());
    }
}
