//! The `tokred mcp` server: MCP over stdio (one JSON-RPC 2.0 message per line) with a single
//! tool, `tokred_retrieve`, that reads back the originals that reversible compression replaced.
//!
//! The server is *dual-era* in the sense of MCP `2026-07-28` ("Backward Compatibility with
//! Initialization-Based Versions"): it answers the legacy `initialize` handshake (versions up to
//! `2025-11-25`) and the modern per-request form (`server/discover`, `_meta` carrying
//! `io.modelcontextprotocol/protocolVersion`). It keeps no state between requests, so it serves
//! `tools/list` and `tools/call` with or without a prior `initialize`.
//!
//! Every answer stays under Claude Code's 50,000-character limit for MCP text results: long
//! originals come back in pages whose first line says which lines they hold and the `offset` of
//! the next page, so nothing is ever cut silently.

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};
use tokred_core::Timestamp;
use tokred_core::compression::StoredResult;

use crate::preview::{lines, page};
use crate::scan::RETRIEVE_TOOL;

/// Name the server reports for itself.
pub const SERVER_NAME: &str = "tokred";

/// The modern protocol version (per-request metadata).
pub const PROTOCOL_VERSION: &str = "2026-07-28";

/// Legacy protocol versions accepted in `initialize`, newest first.
pub const LEGACY_PROTOCOL_VERSIONS: [&str; 4] =
    ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Lines returned when the call does not give a `limit`.
pub const DEFAULT_LIMIT: usize = 500;

/// Largest `limit` honoured.
pub const MAX_LIMIT: usize = 5000;

/// Bytes of original text per answer (Claude Code moves MCP text results above 50,000
/// characters to a file).
pub const MAX_OUTPUT_BYTES: usize = 40_000;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

const VERSION_META: &str = "io.modelcontextprotocol/protocolVersion";
const SERVER_INFO_META: &str = "io.modelcontextprotocol/serverInfo";

const INSTRUCTIONS: &str = "tokred shortens very large tool results to their first and last \
lines plus a marker `[tokred: N lines omitted (id=...)...]`. Call tokred_retrieve with that id to \
read the omitted lines.";

const DESCRIPTION: &str = "Reads back the full text of a large tool result that tokred \
shortened. A shortened result shows its first and last lines and a marker such as \
`[tokred: 4960 lines omitted (id=0123456789abcdef). Call tokred_retrieve ...]`; call this tool \
with that id when you need the omitted lines. `offset` is the 0-based line to start from (the \
preview already showed the first lines, so offset 40 starts at line 41) and `limit` is how many \
lines to return (default 500). Long results come back in pages: the first line of each answer \
says which lines it holds and the offset of the next page.";

fn supported_versions() -> Vec<&'static str> {
    let mut versions = vec![PROTOCOL_VERSION];
    versions.extend(LEGACY_PROTOCOL_VERSIONS);
    versions
}

fn server_info(version: &str) -> Value {
    json!({ "name": SERVER_NAME, "version": version })
}

fn capabilities() -> Value {
    json!({ "tools": { "listChanged": false } })
}

/// The `tokred_retrieve` tool definition, as listed by `tools/list`.
#[must_use]
pub fn tool_definition() -> Value {
    json!({
        "name": RETRIEVE_TOOL,
        "title": "Read back a shortened tool result",
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "The id shown in the marker `[tokred: N lines omitted (id=...)...]`."
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "0-based line to start from (default 0)."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Number of lines to return (default 500)."
                }
            },
            "required": ["id"],
            "additionalProperties": false
        },
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }
    })
}

fn error(id: &Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

fn tool_result(text: &str, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// A non-negative integer argument, also accepted as a numeric string.
fn integer(value: &Value) -> Option<usize> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
        .and_then(|n| usize::try_from(n).ok())
}

struct Server<'a> {
    lookup: &'a dyn Fn(&str) -> Option<StoredResult>,
    now: &'a dyn Fn() -> Timestamp,
    version: &'a str,
}

impl Server<'_> {
    /// The answer to one line, or `None` when nothing must be written (notifications and
    /// responses).
    fn handle(&self, line: &[u8]) -> Option<Value> {
        let Ok(message) = serde_json::from_slice::<Value>(line) else {
            return Some(error(
                &Value::Null,
                PARSE_ERROR,
                "Parse error: the line is not valid JSON",
                None,
            ));
        };
        let Some(request) = message.as_object() else {
            return Some(error(
                &Value::Null,
                INVALID_REQUEST,
                "Invalid Request: expected one JSON-RPC 2.0 object (batches are not supported)",
                None,
            ));
        };
        let id = request.get("id");
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            // A response to a request this server never sends: nothing to answer.
            if request.contains_key("result") || request.contains_key("error") {
                return None;
            }
            return Some(error(
                id.unwrap_or(&Value::Null),
                INVALID_REQUEST,
                "Invalid Request: `method` must be a string",
                None,
            ));
        };
        // Notifications (`notifications/initialized`, `notifications/cancelled`, ...) are never
        // answered.
        let id = id?;
        if !(id.is_string() || id.is_number()) {
            return Some(error(
                &Value::Null,
                INVALID_REQUEST,
                "Invalid Request: `id` must be a string or a number",
                None,
            ));
        }
        if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(error(
                id,
                INVALID_REQUEST,
                "Invalid Request: `jsonrpc` must be \"2.0\"",
                None,
            ));
        }
        let params = request.get("params");
        let requested = params
            .and_then(|p| p.get("_meta"))
            .and_then(|meta| meta.get(VERSION_META))
            .and_then(Value::as_str);
        if let Some(requested) = requested.filter(|v| *v != PROTOCOL_VERSION) {
            return Some(error(
                id,
                UNSUPPORTED_PROTOCOL_VERSION,
                "Unsupported protocol version",
                Some(json!({ "supported": supported_versions(), "requested": requested })),
            ));
        }
        let outcome = match method {
            "initialize" => Ok(self.initialize(params)),
            "ping" => Ok(json!({})),
            "server/discover" => Ok(json!({
                "supportedVersions": supported_versions(),
                "capabilities": capabilities(),
                "_meta": { SERVER_INFO_META: server_info(self.version) },
                "instructions": INSTRUCTIONS,
            })),
            "tools/list" => Ok(json!({ "tools": [tool_definition()] })),
            "tools/call" => self.call(params),
            other => Err((METHOD_NOT_FOUND, format!("Method not found: {other}"))),
        };
        Some(match outcome {
            Ok(mut result) => {
                result["resultType"] = json!("complete");
                if requested.is_some() {
                    result["_meta"] = json!({ SERVER_INFO_META: server_info(self.version) });
                }
                json!({ "jsonrpc": "2.0", "id": id, "result": result })
            }
            Err((code, message)) => error(id, code, &message, None),
        })
    }

    /// Legacy handshake: the client's version when supported, else the newest legacy one.
    fn initialize(&self, params: Option<&Value>) -> Value {
        let asked = params
            .and_then(|p| p.get("protocolVersion"))
            .and_then(Value::as_str);
        let version = LEGACY_PROTOCOL_VERSIONS
            .into_iter()
            .find(|v| Some(*v) == asked)
            .unwrap_or(LEGACY_PROTOCOL_VERSIONS[0]);
        json!({
            "protocolVersion": version,
            "capabilities": capabilities(),
            "serverInfo": server_info(self.version),
            "instructions": INSTRUCTIONS,
        })
    }

    fn call(&self, params: Option<&Value>) -> Result<Value, (i64, String)> {
        let name = params
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .ok_or((
                INVALID_PARAMS,
                "Invalid params: `name` (the tool to call) is required".to_owned(),
            ))?;
        if name != RETRIEVE_TOOL {
            return Err((INVALID_PARAMS, format!("Unknown tool: {name}")));
        }
        let arguments = params.and_then(|p| p.get("arguments"));
        Ok(match self.retrieve(arguments) {
            Ok(text) => tool_result(&text, false),
            Err(text) => tool_result(&text, true),
        })
    }

    fn retrieve(&self, arguments: Option<&Value>) -> Result<String, String> {
        let argument = |name: &str| {
            arguments
                .and_then(|a| a.get(name))
                .filter(|value| !value.is_null())
        };
        let id = argument("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or("Missing required argument `id`: pass the id shown in the tokred marker.")?;
        let offset = match argument("offset") {
            None => 0,
            Some(value) => integer(value).ok_or_else(|| {
                format!("`offset` must be a non-negative integer (0-based line), got {value}.")
            })?,
        };
        let limit = match argument("limit") {
            None => DEFAULT_LIMIT,
            Some(value) => integer(value)
                .filter(|n| *n > 0)
                .ok_or_else(|| format!("`limit` must be a positive integer, got {value}."))?
                .min(MAX_LIMIT),
        };
        let stored = (self.lookup)(id).ok_or_else(|| {
            format!(
                "No stored result with id `{id}`. Check the id in the marker; originals are \
                 kept for a limited time (`[compression] retention`) and then deleted."
            )
        })?;
        if stored.expires_at <= (self.now)() {
            return Err(format!(
                "The stored result `{id}` has expired and is no longer available (originals are \
                 kept for a limited time, `[compression] retention`)."
            ));
        }
        let page = page(&stored.content, offset, limit, MAX_OUTPUT_BYTES).ok_or_else(|| {
            format!(
                "`offset` {offset} is past the end: the result `{id}` has {} lines.",
                lines(&stored.content).len()
            )
        })?;
        let end = page.first + page.count;
        let next = match page.remaining() {
            0 => "this is the end.".to_owned(),
            remaining => format!(
                "{remaining} more lines: call tokred_retrieve again with offset={end} to read them."
            ),
        };
        Ok(format!(
            "[tokred_retrieve id={id} (from {}): lines {}-{end} of {}; {next}]\n{}",
            stored.tool,
            page.first + 1,
            page.total,
            page.text
        ))
    }
}

/// Runs the MCP server until `input` ends: reads one JSON-RPC message per line and writes each
/// answer as one line, flushed. `lookup` finds a stored original by id (the CLI wires it to the
/// store), `now` is the clock used to refuse expired originals, and `version` is reported as the
/// server version. Only I/O errors on `input`/`output` end it early.
pub fn serve(
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    lookup: &dyn Fn(&str) -> Option<StoredResult>,
    now: &dyn Fn() -> Timestamp,
    version: &str,
) -> io::Result<()> {
    let server = Server {
        lookup,
        now,
        version,
    };
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if let Some(answer) = server.handle(&line) {
            let mut text = answer.to_string();
            text.push('\n');
            output.write_all(text.as_bytes())?;
            output.flush()?;
        }
    }
}
