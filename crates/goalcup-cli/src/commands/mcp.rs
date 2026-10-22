//! `goalcup mcp`: the run's time left as a tool the agent calls (fix plan v9, D66).
//!
//! A minimal MCP server on stdin and stdout: newline-delimited JSON-RPC 2.0, one message per
//! line. It answers `initialize`, `ping`, `tools/list` and `tools/call`, takes every notification
//! (`notifications/initialized` among them) without an answer, and stops at the end of its input
//! with exit 0. Its one tool, [`TOOL`], takes no arguments and returns the line
//! `goalcup time-left` prints ([`super::time_left::answer`]), so the two cannot differ.
//!
//! The harness starts it by absolute path, so the agent's `PATH` does not matter. It only reads:
//! it writes no file, opens no port, asks no server, and cannot seal, stop or continue anything
//! (`AGENTS.md`: no finish tool). Goal Cup never adds it to a harness's config (D67): the
//! participant does, from `docs/HARNESS-SETUP.md` or the snippet `goalcup prep` prints.

use crate::CliError;
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};

/// The tool's name.
pub const TOOL: &str = "time_left";

/// The server's name in `initialize`, and the name the setup snippets give it.
pub const SERVER_NAME: &str = "goalcup";

/// The MCP versions this server answers as. Its whole surface (tools without arguments, text
/// results) is the same in each. A client asking for another is answered with the newest.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The longest line read. A longer one is answered with a parse error and skipped.
const MAX_LINE_BYTES: u64 = 1 << 20;

/// JSON-RPC 2.0's error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// What the tool says it does, for the agent.
const TOOL_DESCRIPTION: &str = "How much time this /goal Cup run has left, and the run's \
                                deadline in UTC. Read-only; takes no arguments.";

/// Serve `input` until it ends, answering each request on `output`. `time_left` is the tool's
/// answer: the line, or the refusal whose text the tool returns as an error result. Malformed
/// input is answered with a JSON-RPC error, never a crash; only a failed write stops early.
pub fn serve(
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    time_left: &mut dyn FnMut() -> Result<String, CliError>,
) -> Result<(), CliError> {
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = Read::take(&mut *input, MAX_LINE_BYTES + 1)
            .read_until(b'\n', &mut line)
            .map_err(|source| io("standard input", source))?;
        if read == 0 {
            return Ok(());
        }
        let reply = if line.len() as u64 > MAX_LINE_BYTES && line.last() != Some(&b'\n') {
            skip_rest_of_line(input)?;
            Some(error(Value::Null, PARSE_ERROR, "the line is too long"))
        } else if line.iter().all(u8::is_ascii_whitespace) {
            None
        } else {
            match serde_json::from_slice::<Value>(&line) {
                Ok(message) => answer(&message, time_left),
                Err(_) => Some(error(Value::Null, PARSE_ERROR, "Parse error")),
            }
        };
        if let Some(reply) = reply {
            let mut text = serde_json::to_vec(&reply)?;
            text.push(b'\n');
            output
                .write_all(&text)
                .and_then(|()| output.flush())
                .map_err(|source| io("standard output", source))?;
        }
    }
}

/// The reply to one message, or `None` for a notification (a message with no `id`).
fn answer(
    message: &Value,
    time_left: &mut dyn FnMut() -> Result<String, CliError>,
) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(error(Value::Null, INVALID_REQUEST, "Invalid Request"));
    };
    let id = object.get("id").cloned();
    let method = object.get("method").and_then(Value::as_str);
    let (Some(id), Some(method)) = (id.clone(), method) else {
        // A notification, or a response this server never asked for: nothing to say, unless it
        // carries an id and no method, which is a request it cannot read.
        return match (id, method) {
            (Some(id), None) if object.get("result").is_none() && object.get("error").is_none() => {
                Some(error(id, INVALID_REQUEST, "Invalid Request"))
            }
            _ => None,
        };
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error(id, INVALID_REQUEST, "Invalid Request"));
    }
    let params = object.get("params");
    Some(match method {
        "initialize" => result(id, initialize(params)),
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({ "tools": [tool()] })),
        "tools/call" => match params.and_then(|p| p.get("name")).and_then(Value::as_str) {
            Some(TOOL) => result(id, call(time_left)),
            Some(other) => error(id, INVALID_PARAMS, &format!("Unknown tool: {other}")),
            None => error(id, INVALID_PARAMS, "tools/call needs a tool name"),
        },
        other => error(id, METHOD_NOT_FOUND, &format!("Method not found: {other}")),
    })
}

/// `initialize`: the client's protocol version when this server answers as it, else the newest.
fn initialize(params: Option<&Value>) -> Value {
    let asked = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_str);
    let version = PROTOCOL_VERSIONS
        .iter()
        .find(|v| Some(**v) == asked)
        .unwrap_or(&PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": SERVER_NAME, "version": crate::CLI_VERSION },
        "instructions": format!(
            "Call `{TOOL}` to see how much time this /goal Cup run has left."
        ),
    })
}

/// The one tool, as `tools/list` lists it.
fn tool() -> Value {
    json!({
        "name": TOOL,
        "title": "Time left",
        "description": TOOL_DESCRIPTION,
        "inputSchema": { "type": "object", "properties": {} },
        "annotations": {
            "title": "Time left",
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }
    })
}

/// `tools/call` of [`TOOL`]: the time-left line, or the refusal's text as an error result.
fn call(time_left: &mut dyn FnMut() -> Result<String, CliError>) -> Value {
    match time_left() {
        Ok(line) => json!({ "content": [{ "type": "text", "text": line }], "isError": false }),
        Err(refusal) => json!({
            "content": [{ "type": "text", "text": refusal.to_string() }],
            "isError": true
        }),
    }
}

fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Read and drop the rest of a line that was too long.
fn skip_rest_of_line(input: &mut dyn BufRead) -> Result<(), CliError> {
    loop {
        let buf = input.fill_buf().map_err(|e| io("standard input", e))?;
        if buf.is_empty() {
            return Ok(());
        }
        match buf.iter().position(|b| *b == b'\n') {
            Some(at) => {
                input.consume(at + 1);
                return Ok(());
            }
            None => {
                let n = buf.len();
                input.consume(n);
            }
        }
    }
}

fn io(path: &str, source: std::io::Error) -> CliError {
    CliError::Io {
        path: path.to_string(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `lines` through the server with a fixed tool answer; the replies, one per line.
    fn exchange(lines: &[&str], answer: Result<&str, &str>) -> Vec<Value> {
        let input = lines.join("\n") + "\n";
        let mut out = Vec::new();
        let mut time_left = || match answer {
            Ok(line) => Ok(line.to_string()),
            Err(why) => Err(CliError::Refused(why.to_string())),
        };
        serve(&mut input.as_bytes(), &mut out, &mut time_left).unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

    #[test]
    fn initialize_names_the_server_and_offers_tools() {
        let replies = exchange(&[INIT, INITIALIZED], Ok("x"));
        assert_eq!(
            replies.len(),
            1,
            "a notification is not answered: {replies:?}"
        );
        let r = &replies[0]["result"];
        assert_eq!(replies[0]["id"], 1);
        assert_eq!(r["protocolVersion"], "2025-06-18");
        assert_eq!(r["serverInfo"]["name"], SERVER_NAME);
        assert_eq!(r["serverInfo"]["version"], crate::CLI_VERSION);
        assert!(r["capabilities"]["tools"].is_object(), "{r}");
        // A version this server does not know is answered with its newest.
        let other = exchange(
            &[
                r#"{"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
            ],
            Ok("x"),
        );
        assert_eq!(other[0]["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
        assert_eq!(other[0]["id"], "a");
    }

    #[test]
    fn tools_list_lists_exactly_time_left_and_call_returns_its_line() {
        let replies = exchange(
            &[
                INIT,
                INITIALIZED,
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"time_left","arguments":{}}}"#,
                r#"{"jsonrpc":"2.0","id":4,"method":"ping"}"#,
            ],
            Ok("About 2 h 0 min left."),
        );
        let tools = replies[1]["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, [TOOL]);
        assert_eq!(tools[0]["inputSchema"]["type"], "object");
        let called = &replies[2]["result"];
        assert_eq!(called["isError"], false);
        assert_eq!(called["content"][0]["text"], "About 2 h 0 min left.");
        assert_eq!(replies[3]["result"], json!({}));
    }

    #[test]
    fn a_refusal_is_an_error_result_with_its_text() {
        let replies = exchange(
            &[r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"time_left"}}"#],
            Err("unknown right now"),
        );
        assert_eq!(replies[0]["result"]["isError"], true);
        assert_eq!(
            replies[0]["result"]["content"][0]["text"],
            "unknown right now"
        );
    }

    #[test]
    fn unknown_methods_tools_and_malformed_lines_are_json_rpc_errors() {
        let replies = exchange(
            &[
                r#"{"jsonrpc":"2.0","id":5,"method":"resources/list"}"#,
                r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"seal"}}"#,
                r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{}}"#,
                "this is not json",
                "[1,2]",
                r#"{"id":8,"method":"ping"}"#,
                r#"{"jsonrpc":"2.0","id":9}"#,
                "",
                r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{}}"#,
                r#"{"jsonrpc":"2.0","id":10,"method":"ping"}"#,
            ],
            Ok("x"),
        );
        let codes: Vec<(Value, i64)> = replies
            .iter()
            .map(|r| (r["id"].clone(), r["error"]["code"].as_i64().unwrap_or(0)))
            .collect();
        assert_eq!(
            codes,
            [
                (json!(5), METHOD_NOT_FOUND),
                (json!(6), INVALID_PARAMS),
                (json!(7), INVALID_PARAMS),
                (Value::Null, PARSE_ERROR),
                (Value::Null, INVALID_REQUEST),
                (json!(8), INVALID_REQUEST),
                (json!(9), INVALID_REQUEST),
                (json!(10), 0),
            ]
        );
    }

    #[test]
    fn a_line_too_long_is_skipped_and_the_next_answered() {
        let long = "x".repeat(MAX_LINE_BYTES as usize + 10);
        let replies = exchange(
            &[&long, r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#],
            Ok("x"),
        );
        assert_eq!(replies.len(), 2, "{replies:?}");
        assert_eq!(replies[0]["error"]["code"], PARSE_ERROR);
        assert_eq!(replies[1]["id"], 1);
    }

    #[test]
    fn the_end_of_input_ends_the_server() {
        let mut out = Vec::new();
        let mut never = || -> Result<String, CliError> { panic!("not called") };
        serve(&mut "".as_bytes(), &mut out, &mut never).unwrap();
        assert!(out.is_empty());
    }
}
