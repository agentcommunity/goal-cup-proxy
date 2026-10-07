//! The binary: one JSON-RPC document per line in, one per line out, over stdio.
//!
//! Spawned through `env!("CARGO_BIN_EXE_goalcup-mcp")` and `std::process::Command`, which
//! is the whole of what a harness runner does to it. There is no `assert_cmd` here:
//! a test that needs a spawned process needs a spawned process, and
//! the standard library spawns one.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

mod support;

/// The server, spawned against a fixture's configuration directory, with all three streams
/// on pipes.
fn spawn(config_dir: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_goalcup-mcp"))
        .arg("--config-dir")
        .arg(config_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn")
}

/// The default path is the shared platform resolver. This process-level check keeps the
/// command-line boundary honest without mutating this test binary's global environment.
fn spawn_with_default_config_dir(config_dir: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_goalcup-mcp"))
        .env("GOALCUP_CONFIG_DIR", config_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn")
}

/// The response lines a finished server wrote.
fn lines(stdout: &[u8]) -> Vec<String> {
    BufReader::new(stdout)
        .lines()
        .map(|l| l.expect("utf-8 line"))
        .collect()
}

/// The acceptance line, run for real:
/// `echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | goalcup-mcp`.
#[test]
fn the_binary_answers_one_line_per_request_and_says_nothing_to_a_notification() {
    let fixture = support::source_with_one_entry();
    let mut child = spawn(fixture.config_dir());

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        // A notification in the middle: the answers must still line up with the requests
        // that asked for one.
        for line in [
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#.to_string(),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string(),
            support::call("goalcup_run_status"),
        ] {
            writeln!(stdin, "{line}").expect("write");
        }
    }
    child.stdin.take();

    let out = support::finish_by_deadline(child, support::EXIT_DEADLINE);
    assert!(out.status.success(), "exit: {:?}", out.status);
    let lines = lines(&out.stdout);
    assert_eq!(
        lines.len(),
        2,
        "one line per request, none for the notification"
    );

    let listed: serde_json::Value = serde_json::from_str(&lines[0]).expect("json");
    assert_eq!(listed["id"], 1);
    assert_eq!(
        listed["result"]["tools"].as_array().expect("tools").len(),
        3
    );
    let status: serde_json::Value = serde_json::from_str(&lines[1]).expect("json");
    assert_ne!(status["result"]["isError"], true);
    assert!(support::tool_body(&status).get("offline").is_some());
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains(support::RECEIVER_TOKEN),
        "a secret reached the wire"
    );
}

#[test]
fn the_default_config_dir_reads_the_environment_override() {
    let fixture = support::source_with_one_entry();
    let mut child = spawn_with_default_config_dir(fixture.config_dir());
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"ping"}}"#).expect("write");
    }
    child.stdin.take();

    let out = support::finish_by_deadline(child, support::EXIT_DEADLINE);
    assert!(
        out.status.success(),
        "exit: {:?}; stderr: {:?}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let responses = lines(&out.stdout);
    assert_eq!(responses.len(), 1);
    let response: serde_json::Value = serde_json::from_str(&responses[0]).expect("json");
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"], serde_json::json!({}));
}

/// A line longer than the cap is answered `-32700`, and the request AFTER it is answered
/// normally: the loop drained the long line to its newline rather than parsing its tail as
/// the next request, and the session survived it.
#[test]
fn a_line_longer_than_the_cap_is_answered_and_the_session_keeps_serving() {
    let fixture = support::source_with_one_entry();
    let mut child = spawn(fixture.config_dir());
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        let mut long = vec![b'a'; support::oversize()];
        long.push(b'\n');
        stdin.write_all(&long).expect("write");
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"ping"}}"#).expect("write");
    }
    child.stdin.take();

    let out = support::finish_by_deadline(child, support::EXIT_DEADLINE);
    assert!(
        out.status.success(),
        "a long line is not a reason to stop serving: {:?}",
        out.status
    );
    let lines = lines(&out.stdout);
    assert_eq!(
        lines.len(),
        2,
        "one refusal, then the next request answered"
    );
    let refused: serde_json::Value = serde_json::from_str(&lines[0]).expect("json");
    assert_eq!(refused["error"]["code"], -32700);
    let pinged: serde_json::Value = serde_json::from_str(&lines[1]).expect("json");
    assert_eq!(
        pinged["id"], 1,
        "the tail of the long line was not a request"
    );
    assert_eq!(pinged["result"], serde_json::json!({}));
}

/// One byte that is not UTF-8 must not end the process: a `BufRead::lines` loop would
/// return an `Err` and exit `1`, taking the run's status with it for the rest of the run.
#[test]
fn a_line_that_is_not_utf8_is_answered_and_the_session_keeps_serving() {
    let fixture = support::source_with_one_entry();
    let mut child = spawn(fixture.config_dir());
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"\xff\xfe\"}\n")
            .expect("write");
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"ping"}}"#).expect("write");
    }
    child.stdin.take();

    let out = support::finish_by_deadline(child, support::EXIT_DEADLINE);
    assert!(
        out.status.success(),
        "invalid UTF-8 is not a reason to stop serving: {:?}",
        out.status
    );
    let lines = lines(&out.stdout);
    assert_eq!(
        lines.len(),
        2,
        "one refusal, then the next request answered"
    );
    let refused: serde_json::Value = serde_json::from_str(&lines[0]).expect("json");
    assert_eq!(refused["error"]["code"], -32700);
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("UTF-8")
    );
    let pinged: serde_json::Value = serde_json::from_str(&lines[1]).expect("json");
    assert_eq!(pinged["id"], 2);
}

/// A usage error is 1, not 2 — 2 is a `Fail` verdict in this tree's ladder.
#[test]
fn a_usage_error_exits_one_and_help_exits_zero() {
    let bad = Command::new(env!("CARGO_BIN_EXE_goalcup-mcp"))
        .arg("--no-such-flag")
        .output()
        .expect("spawn");
    assert_eq!(bad.status.code(), Some(1));
    assert!(!bad.stderr.is_empty(), "a usage error says what was wrong");

    let help = Command::new(env!("CARGO_BIN_EXE_goalcup-mcp"))
        .arg("--help")
        .output()
        .expect("spawn");
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).contains("--config-dir"));
}
