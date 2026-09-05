//! Local, read-only Model Context Protocol support.
//!
//! The MCP adapter deliberately lives outside the normal command modules. It
//! is a synchronous stdio server: an MCP client owns the child process and
//! communicates with it using one JSON-RPC message per line.

pub mod client_install;
mod protocol;
mod tools;

pub use client_install::McpClient;

/// Run the local MCP server until stdin reaches EOF.
pub fn serve() -> crate::SnipResult<()> {
    protocol::serve()
}

/// Print setup instructions for one supported MCP client.
pub fn instructions(client: McpClient) -> crate::SnipResult<()> {
    client_install::print_instructions(client)
}

/// Install this binary into one supported MCP client when that client's
/// official noninteractive registration command is available. Otherwise,
/// print the exact manual instructions.
pub fn install(client: McpClient) -> crate::SnipResult<()> {
    client_install::install(client)
}
