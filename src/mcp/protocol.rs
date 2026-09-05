//! Minimal newline-delimited JSON-RPC transport for MCP stdio.

use super::tools;
use crate::error::{SnipError, SnipResult};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    MCP_PROTOCOL_VERSION,
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];
const MAX_MESSAGE_BYTES: usize = 1_048_576;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const SERVER_NOT_INITIALIZED: i64 = -32002;

#[derive(Debug, Default)]
struct ServerState {
    initialized: bool,
    client_initialized: bool,
}

pub fn serve() -> SnipResult<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    let mut state = ServerState::default();

    loop {
        let Some(line) = read_bounded_line(&mut input)? else {
            return Ok(());
        };
        if line.is_empty() || line == b"\r" {
            write_error(&mut output, None, PARSE_ERROR, "Empty MCP message", None)?;
            continue;
        }

        let request: Value = match serde_json::from_slice(trim_line(&line)) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("MCP parse error: {error}");
                write_error(
                    &mut output,
                    None,
                    PARSE_ERROR,
                    "Parse error",
                    Some(json!({ "message": error.to_string() })),
                )?;
                continue;
            }
        };

        handle_message(&request, &mut state, &mut output)?;
    }
}

fn handle_message<W: Write>(
    request: &Value,
    state: &mut ServerState,
    output: &mut W,
) -> SnipResult<()> {
    let Some(object) = request.as_object() else {
        return write_error(output, None, INVALID_REQUEST, "Invalid Request", None);
    };
    if object.get("jsonrpc") != Some(&Value::String("2.0".to_string())) {
        return write_error(
            output,
            request_id(object),
            INVALID_REQUEST,
            "Invalid Request",
            None,
        );
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return write_error(
            output,
            request_id(object),
            INVALID_REQUEST,
            "Invalid Request",
            None,
        );
    };
    let id = request_id(object);
    let is_notification = id.is_none();
    let params = object.get("params");

    if method == "notifications/initialized" {
        if state.initialized {
            state.client_initialized = true;
        }
        return Ok(());
    }

    if method == "ping" {
        if let Some(id) = id {
            return write_result(output, id, json!({}));
        }
        return Ok(());
    }

    if method == "initialize" {
        if is_notification {
            return Ok(());
        }
        if state.initialized {
            return write_error(output, id, INVALID_REQUEST, "Already initialized", None);
        }
        return initialize(params, id, state, output);
    }

    if !state.initialized || !state.client_initialized {
        if is_notification {
            return Ok(());
        }
        return write_error(
            output,
            id,
            SERVER_NOT_INITIALIZED,
            "Server is not initialized",
            None,
        );
    }

    match method {
        "tools/list" => {
            if is_notification {
                return Ok(());
            }
            if params.is_some_and(|params| !params.is_object()) {
                return write_error(
                    output,
                    id,
                    INVALID_PARAMS,
                    "tools/list params must be an object",
                    None,
                );
            }
            if params
                .and_then(Value::as_object)
                .and_then(|params| params.get("cursor"))
                .is_some()
            {
                return write_error(
                    output,
                    id,
                    INVALID_PARAMS,
                    "Pagination is not supported",
                    None,
                );
            }
            write_result(output, id.expect("request ID is present"), tool_list())
        }
        "tools/call" => {
            if is_notification {
                return Ok(());
            }
            call_tool(params, id.expect("request ID is present"), output)
        }
        _ => {
            if is_notification {
                Ok(())
            } else {
                write_error(output, id, METHOD_NOT_FOUND, "Method not found", None)
            }
        }
    }
}

fn initialize<W: Write>(
    params: Option<&Value>,
    id: Option<Value>,
    state: &mut ServerState,
    output: &mut W,
) -> SnipResult<()> {
    let Some(params) = params.and_then(Value::as_object) else {
        return write_error(
            output,
            id,
            INVALID_PARAMS,
            "initialize params must be an object",
            None,
        );
    };
    let Some(version) = params.get("protocolVersion").and_then(Value::as_str) else {
        return write_error(
            output,
            id,
            INVALID_PARAMS,
            "initialize requires protocolVersion",
            None,
        );
    };
    if !params.get("capabilities").is_some_and(Value::is_object)
        || !params.get("clientInfo").is_some_and(Value::is_object)
    {
        return write_error(
            output,
            id,
            INVALID_PARAMS,
            "initialize requires capabilities and clientInfo objects",
            None,
        );
    }
    if !SUPPORTED_PROTOCOL_VERSIONS.contains(&version) {
        return write_error(
            output,
            id,
            INVALID_PARAMS,
            "Unsupported protocol version",
            Some(json!({
                "supported": SUPPORTED_PROTOCOL_VERSIONS,
                "requested": version,
            })),
        );
    }

    state.initialized = true;
    write_result(
        output,
        id.expect("initialize request ID is present"),
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "snip-it",
                "version": env!("CARGO_PKG_VERSION"),
                "description": "Read-only local snippet library",
            },
            "instructions": "Use snippets_list, snippets_search, and snippet_get to read snippets. Commands are returned as text and are never executed by this server.",
        }),
    )
}

fn tool_list() -> Value {
    json!({
        "tools": [
            {
                "name": "snippets_list",
                "description": "List read-only snippet metadata from the primary or selected library. The returned command text is never executed.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "library": { "type": "string", "description": "Optional library name; use 'all' for every registered library." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100 },
                    },
                    "additionalProperties": false,
                },
            },
            {
                "name": "snippets_search",
                "description": "Search snippet descriptions and command text using snip-it's deterministic fuzzy ranking. Read-only; commands are never executed.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Required fuzzy search query." },
                        "library": { "type": "string", "description": "Optional library name; use 'all' for every registered library." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100 },
                    },
                    "required": ["query"],
                    "additionalProperties": false,
                },
            },
            {
                "name": "snippet_get",
                "description": "Get one snippet by exact ID or a unique exact description. Read-only; the command is returned as text and never executed.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Exact snippet ID." },
                        "description": { "type": "string", "description": "Exact description, case-insensitive; must be unique in the selected scope." },
                        "library": { "type": "string", "description": "Optional library name; use 'all' for every registered library." },
                    },
                    "oneOf": [
                        { "required": ["id"] },
                        { "required": ["description"] },
                    ],
                    "additionalProperties": false,
                },
            },
        ]
    })
}

fn call_tool<W: Write>(params: Option<&Value>, id: Value, output: &mut W) -> SnipResult<()> {
    let Some(params) = params.and_then(Value::as_object) else {
        return write_error(
            output,
            Some(id),
            INVALID_PARAMS,
            "tools/call params must be an object",
            None,
        );
    };
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return write_error(
            output,
            Some(id),
            INVALID_PARAMS,
            "tools/call requires a tool name",
            None,
        );
    };
    let result = match name {
        "snippets_list" => tools::list(params.get("arguments")),
        "snippets_search" => tools::search(params.get("arguments")),
        "snippet_get" => tools::get(params.get("arguments")),
        _ => {
            return write_error(
                output,
                Some(id),
                INVALID_PARAMS,
                "Unknown tool",
                Some(json!({ "name": name })),
            );
        }
    };

    match result {
        Ok(value) => {
            let error = tools::is_error(&value);
            write_result(
                output,
                id,
                json!({
                    "content": [{ "type": "text", "text": serde_json::to_string(&value).map_err(|e| SnipError::runtime_error("MCP result serialization failed", Some(&e.to_string())))? }],
                    "structuredContent": value,
                    "isError": error,
                }),
            )
        }
        Err(error) => {
            let structured = json!({
                "error": "data_error",
                "message": error.to_string(),
            });
            write_result(
                output,
                id,
                json!({
                    "content": [{ "type": "text", "text": serde_json::to_string(&structured).map_err(|e| SnipError::runtime_error("MCP result serialization failed", Some(&e.to_string())))? }],
                    "structuredContent": structured,
                    "isError": true,
                }),
            )
        }
    }
}

fn request_id(object: &serde_json::Map<String, Value>) -> Option<Value> {
    object.get("id").cloned()
}

fn write_result<W: Write>(output: &mut W, id: Value, result: Value) -> SnipResult<()> {
    write_json(
        output,
        json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    )
}

fn write_error<W: Write>(
    output: &mut W,
    id: Option<Value>,
    code: i64,
    message: &str,
    data: Option<Value>,
) -> SnipResult<()> {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    write_json(
        output,
        json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    )
}

fn write_json<W: Write>(output: &mut W, value: Value) -> SnipResult<()> {
    serde_json::to_writer(&mut *output, &value).map_err(|error| {
        SnipError::runtime_error(
            "MCP response serialization failed",
            Some(&error.to_string()),
        )
    })?;
    output
        .write_all(b"\n")
        .and_then(|_| output.flush())
        .map_err(|error| SnipError::io_error("write MCP response", "stdout", error))?;
    Ok(())
}

fn trim_line(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\n")
        .unwrap_or(line)
        .strip_suffix(b"\r")
        .unwrap_or_else(|| line.strip_suffix(b"\n").unwrap_or(line))
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(line))
            };
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(buffer.len(), |position| position + 1);
        if line.len() + take > MAX_MESSAGE_BYTES {
            reader.consume(take);
            if newline.is_none() {
                discard_until_newline(reader)?;
            }
            return Ok(Some(Vec::from(
                b"{\"jsonrpc\":\"2.0\",\"__oversized\":true}" as &[u8],
            )));
        }
        line.extend_from_slice(&buffer[..take]);
        reader.consume(take);
        if newline.is_some() {
            return Ok(Some(line));
        }
    }
}

fn discard_until_newline<R: BufRead>(reader: &mut R) -> io::Result<()> {
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(());
        }
        let take = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |position| position + 1);
        let done = take < buffer.len() || buffer.last() == Some(&b'\n');
        reader.consume(take);
        if done {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn bounded_reader_discards_oversized_lines() {
        let mut input = Cursor::new(format!("{}\n{{}}\n", "x".repeat(MAX_MESSAGE_BYTES + 10)));
        let first = read_bounded_line(&mut input).unwrap().unwrap();
        assert_eq!(first, br#"{"jsonrpc":"2.0","__oversized":true}"#);
        let second = read_bounded_line(&mut input).unwrap().unwrap();
        assert_eq!(trim_line(&second), b"{}");
    }
}
