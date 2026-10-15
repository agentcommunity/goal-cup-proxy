//! The Claude Code project-JSONL format (`claude-code-jsonl`).
//!
//! Facts measured on `spikes/claude-code-cli/fixtures/main-session.jsonl` (Claude Code CLI
//! 2.1.259) and on real 2.1.283 chains. 2.1.283 carries no `origin` field at all and a
//! different `system` error shape (`subtype: "api_error"`, no `turn_duration` or
//! text-prefixed hook-block form), both read from real bytes. The version names the checked
//! range, not a single measured version.
//!
//! **Authorship.** `origin.kind == "human"` (2.1.259) or `promptSource == "typed"` on a
//! structurally eligible `type: "user"` record is the human marker. `isMeta`, `isSidechain` or
//! `isCompactSummary` true, `promptSource == "system"`, or any other `origin.kind` makes a
//! record ineligible. The sdk-delivered goal of 2.1.283 carries `promptSource: "sdk"`
//! (measured), so a headless run claims nothing. A tool-result continuation carries neither
//! marker.

use super::super::api::kinds;
use super::{NativeFormat, NativeOut, dotted};
use serde_json::{Value, json};

/// The Claude Code project-JSONL format.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeCode;

impl NativeFormat for ClaudeCode {
    const HINT: &'static str = "claude-code-jsonl";
    const ID: &'static str = "claude-code-native";
    const VERSION: &'static str = "2.1.259-2.1.283";
    type Stream = ();

    fn interpret(v: &Value, _stream: &mut (), out: &mut NativeOut<'_>) {
        match v.get("type").and_then(Value::as_str) {
            Some("user") => user(v, out),
            Some("assistant") => assistant(v, out),
            Some("system") => system(v, out),
            Some("attachment") => attachment(v, out),
            other => out.unrecognized(other),
        }
    }
}

fn user(v: &Value, out: &mut NativeOut<'_>) {
    // The positive markers: a supplied `origin.kind == "human"` (2.1.259) or
    // `promptSource == "typed"`. The sdk-delivered goal carries `promptSource: "sdk"` and
    // a tool-result continuation carries neither, so neither is claimed.
    let origin_kind = dotted(v, "origin.kind").and_then(Value::as_str);
    let prompt_source = v.get("promptSource").and_then(Value::as_str);
    let has_marker = origin_kind == Some("human") || prompt_source == Some("typed");
    let prompt_id = v.get("promptId").and_then(Value::as_str).map(String::from);
    let content = dotted(v, "message.content");

    // Harness-generated records are never human, whatever marker they carry: a
    // background sub-agent task notification carries `promptSource: "system"` and
    // `origin.kind: "task-notification"` (measured), a replayed continuation
    // `isMeta: true` (measured), a sub-agent turn `isSidechain: true`, and a compaction
    // summary `isCompactSummary: true`. A present `origin.kind` must be exactly `"human"`.
    let flag = |key: &str| v.get(key).and_then(Value::as_bool) == Some(true);
    let structurally_eligible = !flag("isMeta")
        && !flag("isSidechain")
        && !flag("isCompactSummary")
        && prompt_source != Some("system")
        && origin_kind.is_none_or(|kind| kind == "human");
    let is_human = structurally_eligible && has_marker;
    let mut any = false;
    match content {
        Some(Value::String(text)) if is_human && !text.is_empty() => {
            out.kind(kinds::HUMAN_MESSAGE, prompt_id, json!({"text": text}));
            any = true;
        }
        Some(Value::Array(blocks)) => {
            if is_human {
                let text: String = blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect();
                if !text.is_empty() {
                    out.kind(
                        kinds::HUMAN_MESSAGE,
                        prompt_id.clone(),
                        json!({"text": text}),
                    );
                    any = true;
                }
            }
            for block in blocks {
                if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                    let id = block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .map(String::from);
                    let mut fields = json!({});
                    if let Some(is_error) = block.get("is_error") {
                        fields["is_error"] = is_error.clone();
                    }
                    out.kind(kinds::TOOL_RESULT, id, fields);
                    any = true;
                }
            }
        }
        _ => {}
    }
    if !any {
        out.unrecognized(Some("user"));
    }
}

fn assistant(v: &Value, out: &mut NativeOut<'_>) {
    let message_id = dotted(v, "message.id")
        .and_then(Value::as_str)
        .map(String::from);
    let mut any = false;
    if let Some(model) = dotted(v, "message.model").and_then(Value::as_str) {
        out.kind(
            kinds::MODEL,
            message_id.clone(),
            json!({"via": "message", "model": model}),
        );
        any = true;
    }
    if let Some(usage) = dotted(v, "message.usage") {
        let input = usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let output = usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        // A `<synthetic>` resume record carries an all-zero usage block that made no API
        // call (measured): recording it would assert a spend that never happened, so only a
        // real non-zero snapshot becomes an observation.
        if input > 0 || output > 0 {
            let mut fields =
                json!({"scope": "per_call", "input_tokens": input, "output_tokens": output});
            if let Some(c) = usage.get("cache_read_input_tokens") {
                fields["cache_read_input_tokens"] = c.clone();
            }
            if let Some(c) = usage.get("cache_creation_input_tokens") {
                fields["cache_creation_input_tokens"] = c.clone();
            }
            out.kind(kinds::USAGE, message_id.clone(), fields);
            any = true;
        }
    }
    if let Some(Value::Array(blocks)) = dotted(v, "message.content") {
        for block in blocks {
            if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                let id = block.get("id").and_then(Value::as_str).map(String::from);
                let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                out.kind(kinds::TOOL_CALL, id, json!({"name": name}));
                any = true;
            }
        }
    }
    if !any {
        out.unrecognized(Some("assistant"));
    }
}

fn system(v: &Value, out: &mut NativeOut<'_>) {
    match v.get("subtype").and_then(Value::as_str) {
        Some("informational") => {
            let content = v.get("content").and_then(Value::as_str).unwrap_or("");
            // A real, measured hook denial (2.1.259): the harness's own account that a
            // submission was blocked. Only a short, safe code is kept, never the raw content,
            // which carries the (possibly long, possibly sensitive) prompt text.
            if content.starts_with("UserPromptSubmit operation blocked by hook:") {
                out.kind(
                    kinds::ERROR,
                    None,
                    json!({"subtype": "informational", "code": "prompt_blocked_by_hook"}),
                );
            } else {
                out.unrecognized(Some("system.informational"));
            }
        }
        Some("turn_duration") => {
            // A turn's own observed duration, never a claim that the wider task succeeded.
            let mut fields = json!({"subtype": "turn_duration"});
            if let Some(d) = v.get("durationMs") {
                fields["duration_ms"] = d.clone();
            }
            if let Some(m) = v.get("messageCount") {
                fields["message_count"] = m.clone();
            }
            out.kind(kinds::OUTCOME, None, fields);
        }
        Some("api_error") => {
            // 2.1.283's real error form. Only the bounded, safe supplied facts are kept: never
            // the raw `error.message`/`error.formatted` text, which can carry arbitrary
            // provider or network detail.
            let mut fields = json!({"subtype": "api_error"});
            if let Some(level) = v.get("level").and_then(Value::as_str) {
                fields["level"] = Value::String(level.to_string());
            }
            if let Some(code) = dotted(v, "error.connection.code").and_then(Value::as_str) {
                fields["code"] = Value::String(code.to_string());
            }
            if let Some(attempt) = v.get("retryAttempt") {
                fields["retry_attempt"] = attempt.clone();
            }
            if let Some(max) = v.get("maxRetries") {
                fields["max_retries"] = max.clone();
            }
            out.kind(kinds::ERROR, None, fields);
        }
        other => out.unrecognized(other.map(|s| format!("system.{s}")).as_deref()),
    }
}

fn attachment(v: &Value, out: &mut NativeOut<'_>) {
    if dotted(v, "attachment.type").and_then(Value::as_str) == Some("model")
        && let Some(id) = dotted(v, "attachment.identity.modelId").and_then(Value::as_str)
    {
        out.kind(kinds::MODEL, None, json!({"via": "switch", "model": id}));
        return;
    }
    out.unrecognized(Some("attachment"));
}
