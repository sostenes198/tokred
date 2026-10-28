//! The `tokred mcp` server over in-memory stdio.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{self, BufRead, Cursor, Read, Write};

use serde_json::{Value, json};
use tokred_compress::mcp::{
    DEFAULT_LIMIT, LEGACY_PROTOCOL_VERSIONS, MAX_LIMIT, MAX_OUTPUT_BYTES, PROTOCOL_VERSION,
    tool_definition,
};
use tokred_compress::serve;
use tokred_core::Timestamp;
use tokred_core::compression::StoredResult;

const NOW: i64 = 1_000;

fn numbered(lines: usize) -> String {
    (1..=lines)
        .map(|n| format!("line {n}\n"))
        .collect::<Vec<_>>()
        .concat()
}

fn lookup(id: &str) -> Option<StoredResult> {
    let stored = |content: String, expires_at: i64| StoredResult {
        id: id.to_owned(),
        session_id: "s1".to_owned(),
        tool: "Bash".to_owned(),
        content,
        expires_at: Timestamp(expires_at),
        ..StoredResult::default()
    };
    match id {
        "abc" => Some(stored(numbered(1200), NOW + 1)),
        "wide" => Some(stored(
            format!("{}\n", "x".repeat(30_000)).repeat(3),
            NOW + 1,
        )),
        "old" => Some(stored(numbered(5), NOW)),
        "tiny" => Some(stored("x\n".repeat(MAX_LIMIT + 100), NOW + 1)),
        _ => None,
    }
}

/// Runs the server over `lines` and returns every answer it wrote.
fn session(lines: &[Value]) -> Vec<Value> {
    let input = lines
        .iter()
        .map(|l| format!("{l}\n"))
        .collect::<Vec<_>>()
        .concat();
    raw_session(&input)
}

fn raw_session(input: &str) -> Vec<Value> {
    let mut output = Vec::new();
    serve(
        &mut Cursor::new(input.as_bytes().to_vec()),
        &mut output,
        &lookup,
        &|| Timestamp(NOW),
        "9.9.9",
    )
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.is_empty() || text.ends_with('\n'));
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[allow(clippy::needless_pass_by_value)]
fn call(id: i64, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
           "params": {"name": "tokred_retrieve", "arguments": arguments}})
}

fn text_of(answer: &Value) -> (&str, bool) {
    let result = &answer["result"];
    assert_eq!(result["resultType"], "complete");
    (
        result["content"][0]["text"].as_str().unwrap(),
        result["isError"].as_bool().unwrap(),
    )
}

#[test]
fn a_legacy_client_initializes_lists_and_retrieves() {
    let answers = session(&[
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "claude-code", "version": "2.1.312"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        call(3, json!({"id": "abc", "offset": 40, "limit": 3})),
        json!({"jsonrpc": "2.0", "id": "p", "method": "ping"}),
    ]);
    assert_eq!(answers.len(), 4, "the notification is not answered");

    let init = &answers[0];
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(
        init["result"]["capabilities"],
        json!({"tools": {"listChanged": false}})
    );
    assert_eq!(
        init["result"]["serverInfo"],
        json!({"name": "tokred", "version": "9.9.9"})
    );
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("tokred_retrieve")
    );

    let tools = &answers[1]["result"]["tools"];
    assert_eq!(tools, &json!([tool_definition()]));
    assert_eq!(tools[0]["name"], "tokred_retrieve");
    assert_eq!(tools[0]["inputSchema"]["required"], json!(["id"]));

    let (text, is_error) = text_of(&answers[2]);
    assert!(!is_error);
    assert_eq!(
        text,
        "[tokred_retrieve id=abc (from Bash): lines 41-43 of 1200; 1157 more lines: call \
         tokred_retrieve again with offset=43 to read them.]\nline 41\nline 42\nline 43\n"
    );
    assert_eq!(
        answers[3],
        json!({"jsonrpc": "2.0", "id": "p", "result": {"resultType": "complete"}})
    );
}

#[test]
fn initialize_answers_a_supported_legacy_version() {
    for version in LEGACY_PROTOCOL_VERSIONS {
        let answers = session(&[json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                                       "params": {"protocolVersion": version}})]);
        assert_eq!(answers[0]["result"]["protocolVersion"], version);
    }
    for unknown in [json!({"protocolVersion": "1999-01-01"}), json!(null)] {
        let answers = session(&[json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                                       "params": unknown})]);
        assert_eq!(answers[0]["result"]["protocolVersion"], "2025-11-25");
    }
}

#[test]
fn a_modern_client_discovers_and_calls_with_metadata() {
    let meta = json!({"io.modelcontextprotocol/protocolVersion": PROTOCOL_VERSION,
                      "io.modelcontextprotocol/clientCapabilities": {}});
    let answers = session(&[
        json!({"jsonrpc": "2.0", "id": "d", "method": "server/discover", "params": {"_meta": meta}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
            "name": "tokred_retrieve", "arguments": {"id": "abc", "offset": 1198}, "_meta": meta}}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list", "params": {"_meta": {
            "io.modelcontextprotocol/protocolVersion": "2030-01-01"}}}),
    ]);
    let discover = &answers[0]["result"];
    assert_eq!(
        discover["supportedVersions"],
        json!([
            "2026-07-28",
            "2025-11-25",
            "2025-06-18",
            "2025-03-26",
            "2024-11-05"
        ])
    );
    assert_eq!(
        discover["capabilities"]["tools"],
        json!({"listChanged": false})
    );
    assert_eq!(
        discover["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "tokred"
    );

    let (text, is_error) = text_of(&answers[1]);
    assert!(!is_error);
    assert_eq!(
        text,
        "[tokred_retrieve id=abc (from Bash): lines 1199-1200 of 1200; this is the end.]\n\
         line 1199\nline 1200\n"
    );
    assert_eq!(
        answers[1]["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
        "9.9.9"
    );

    let unsupported = &answers[2];
    assert_eq!(unsupported["id"], 3);
    assert_eq!(unsupported["error"]["code"], -32022);
    assert_eq!(unsupported["error"]["data"]["requested"], "2030-01-01");
    assert_eq!(unsupported["error"]["data"]["supported"][0], "2026-07-28");
}

#[test]
fn long_results_come_back_in_pages_within_the_limits() {
    let answers = session(&[
        call(1, json!({"id": "abc"})),
        call(
            2,
            json!({"id": " abc ", "offset": "1000", "limit": 1_000_000}),
        ),
        call(3, json!({"id": "wide", "offset": null, "limit": null})),
        call(4, json!({"id": "tiny", "limit": 1_000_000})),
    ]);
    let (text, _) = text_of(&answers[0]);
    assert!(text.starts_with(&format!(
        "[tokred_retrieve id=abc (from Bash): lines 1-{DEFAULT_LIMIT} of 1200; 700 more lines: \
         call tokred_retrieve again with offset={DEFAULT_LIMIT}"
    )));
    assert_eq!(text.lines().count(), DEFAULT_LIMIT + 1);

    let (text, _) = text_of(&answers[1]);
    assert!(text.starts_with("[tokred_retrieve id=abc (from Bash): lines 1001-1200 of 1200;"));

    // Two 30,000-byte lines do not fit in one answer: one line, and where to go next.
    let (text, _) = text_of(&answers[2]);
    assert!(text.starts_with(
        "[tokred_retrieve id=wide (from Bash): lines 1-1 of 3; 2 more lines: call \
         tokred_retrieve again with offset=1"
    ));
    assert!(text.len() < MAX_OUTPUT_BYTES + 200);

    // A huge `limit` is capped.
    let (text, _) = text_of(&answers[3]);
    assert!(text.starts_with(&format!(
        "[tokred_retrieve id=tiny (from Bash): lines 1-{MAX_LIMIT} of {}; 100 more lines",
        MAX_LIMIT + 100
    )));
}

#[test]
fn retrieval_problems_are_tool_errors_with_a_clear_message() {
    let answers = session(&[
        call(1, json!({"id": "nope"})),
        call(2, json!({"id": "old"})),
        call(3, json!({"id": "abc", "offset": 1200})),
        call(4, json!({"id": "abc", "offset": -1})),
        call(5, json!({"id": "abc", "limit": 0})),
        call(6, json!({})),
        call(7, json!({"id": "  "})),
        json!({"jsonrpc": "2.0", "id": 8, "method": "tools/call",
               "params": {"name": "tokred_retrieve"}}),
    ]);
    let messages: Vec<(&str, bool)> = answers.iter().map(text_of).collect();
    assert!(messages.iter().all(|(_, is_error)| *is_error));
    assert!(messages[0].0.contains("No stored result with id `nope`"));
    assert!(messages[1].0.contains("`old` has expired"));
    assert!(messages[2].0.contains("offset` 1200 is past the end"));
    assert!(messages[2].0.contains("has 1200 lines"));
    assert!(
        messages[3]
            .0
            .contains("`offset` must be a non-negative integer")
    );
    assert!(messages[4].0.contains("`limit` must be a positive integer"));
    for missing in &messages[5..] {
        assert!(missing.0.contains("Missing required argument `id`"));
    }
}

#[test]
fn malformed_messages_get_json_rpc_errors() {
    let answers = raw_session(concat!(
        "{not json\n",
        "\n",
        "   \n",
        "[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}]\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":4,\"error\":{\"code\":1,\"message\":\"x\"}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":null,\"method\":\"ping\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":true,\"method\":\"ping\"}\n",
        "{\"id\":5,\"method\":\"ping\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"resources/list\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{\"name\":\"Read\"}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\"}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}",
    ));
    let codes: Vec<(Value, i64)> = answers
        .iter()
        .map(|a| (a["id"].clone(), a["error"]["code"].as_i64().unwrap_or(0)))
        .collect();
    assert_eq!(
        codes,
        [
            (Value::Null, -32700),
            (Value::Null, -32600),
            (json!(2), -32600),
            (Value::Null, -32600),
            (Value::Null, -32600),
            (json!(5), -32600),
            (json!(6), -32601),
            (json!(7), -32602),
            (json!(8), -32602),
            (json!(9), 0),
        ]
    );
    assert_eq!(answers[7]["error"]["message"], "Unknown tool: Read");
    assert!(answers.iter().all(|a| a["jsonrpc"] == "2.0"));
}

#[test]
fn invalid_utf8_is_a_parse_error() {
    let mut output = Vec::new();
    serve(
        &mut Cursor::new(b"\xff\xfe\n".to_vec()),
        &mut output,
        &lookup,
        &|| Timestamp(NOW),
        "1",
    )
    .unwrap();
    let answer: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(answer["error"]["code"], -32700);
}

struct Broken;

impl Read for Broken {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("closed"))
    }
}

impl BufRead for Broken {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Err(io::Error::other("closed"))
    }
    fn consume(&mut self, _: usize) {}
}

impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("closed"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("closed"))
    }
}

#[test]
fn io_errors_end_the_server() {
    let mut output = Vec::new();
    assert!(serve(&mut Broken, &mut output, &lookup, &|| Timestamp(NOW), "1").is_err());
    let mut input = Cursor::new(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n".to_vec());
    assert!(serve(&mut input, &mut Broken, &lookup, &|| Timestamp(NOW), "1").is_err());
}
