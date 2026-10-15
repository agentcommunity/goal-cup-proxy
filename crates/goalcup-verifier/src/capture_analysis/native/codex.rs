//! The Codex rollout-JSONL format (`codex-jsonl`).
//!
//! Facts measured on `spikes/codex-cli/fixtures/rollout-main-01a06614.jsonl` (Codex CLI
//! 0.153.0, TUI launch) and on real `exec`-launch chains. 0.153.0 exactly matches the
//! connector's `tested_version_range`.
//!
//! **Authorship.** `internal_chat_message_metadata_passthrough.content_item_kinds` containing
//! `"user.text"` on a `response_item` message of `role: "user"` is the human marker (an
//! injected `<environment_context>` user-role message carries
//! `["environments.environment_context"]` instead). It counts only on a stream whose
//! `session_meta.payload.source` is `"cli"` or `"vscode"`. An `exec` launch has no input
//! channel: its first marked message is the tracker's own delivered goal, which carries the
//! identical marker (measured on real bytes), and no later one is claimed either. A stream
//! with no `session_meta` before the message has an unknown launch, so nothing on it is claimed.

use super::super::api::kinds;
use super::{NativeFormat, NativeOut, dotted};
use serde_json::{Value, json};

/// The Codex rollout-JSONL format.
#[derive(Debug, Clone, Copy, Default)]
pub struct Codex;

/// What a Codex stream remembers across its records: two small fields.
#[derive(Default)]
pub struct CodexStream {
    /// The session's own declared `session_meta.payload.source`, once that record has been
    /// seen on this stream. `None` until then (or if it never arrives).
    launch: Option<String>,
    /// True once this `exec` stream's one delivered-goal record has been labelled as such.
    goal_excluded: bool,
}

impl NativeFormat for Codex {
    const HINT: &'static str = "codex-jsonl";
    const ID: &'static str = "codex-native";
    const VERSION: &'static str = "0.153.0";
    type Stream = CodexStream;

    fn interpret(v: &Value, stream: &mut CodexStream, out: &mut NativeOut<'_>) {
        let record_type = v.get("type").and_then(Value::as_str);
        // Bookkeeping only (no observation shape changes): remembers whether this stream's
        // session was launched as `exec` (its argv goal is not human input) or something
        // else. `session_meta` is still counted as `unknown{unrecognized}` below, unchanged.
        if record_type == Some("session_meta") {
            stream.launch = dotted(v, "payload.source")
                .and_then(Value::as_str)
                .map(String::from);
        }
        match record_type {
            Some("turn_context") => {
                if let Some(model) = dotted(v, "payload.model").and_then(Value::as_str) {
                    let turn_id = dotted(v, "payload.turn_id")
                        .and_then(Value::as_str)
                        .map(String::from);
                    out.kind(
                        kinds::MODEL,
                        turn_id,
                        json!({"via": "turn_context", "model": model}),
                    );
                } else {
                    out.unrecognized(Some("turn_context"));
                }
            }
            Some("response_item") => response_item(v, stream, out),
            Some("event_msg") => event_msg(v, out),
            other => out.unrecognized(other),
        }
    }
}

fn response_item(v: &Value, stream: &mut CodexStream, out: &mut NativeOut<'_>) {
    let Some(payload) = v.get("payload") else {
        out.unrecognized(Some("response_item"));
        return;
    };
    let Some(item_type) = payload.get("type").and_then(Value::as_str) else {
        out.unrecognized(Some("response_item"));
        return;
    };
    match item_type {
        "message" => {
            let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
            // The demonstrated positive marker (module doc): not role alone, not a text
            // prefix. `content_item_kinds` is the record's own account of what kind of
            // content this is; only "user.text" is real human-typed input.
            let has_marker = dotted(
                payload,
                "internal_chat_message_metadata_passthrough.content_item_kinds",
            )
            .and_then(Value::as_array)
            .is_some_and(|kinds| kinds.iter().any(|k| k.as_str() == Some("user.text")));
            if role != "user" || !has_marker {
                out.unrecognized(Some("response_item.message"));
                return;
            }
            // The marker counts only in a session launched with an input channel. An
            // `exec` stream's first marked message is the tracker's delivered goal; no
            // marked message on an `exec` or unknown-launch stream is claimed human.
            match stream.launch.as_deref() {
                Some("cli" | "vscode") => {}
                Some("exec") if !stream.goal_excluded => {
                    stream.goal_excluded = true;
                    out.unrecognized(Some("response_item.message.goal"));
                    return;
                }
                _ => {
                    out.unrecognized(Some("response_item.message"));
                    return;
                }
            }
            let text = payload
                .get("content")
                .and_then(Value::as_array)
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter_map(|b| b.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();
            let id = payload.get("id").and_then(Value::as_str).map(String::from);
            out.kind(
                kinds::HUMAN_MESSAGE,
                id,
                json!({"role": "user", "text": text}),
            );
        }
        "custom_tool_call" | "function_call" => {
            let call_id = payload
                .get("call_id")
                .and_then(Value::as_str)
                .map(String::from);
            let mut fields = json!({"item_type": item_type});
            if let Some(name) = payload.get("name").and_then(Value::as_str) {
                fields["name"] = Value::String(name.to_string());
            }
            if let Some(status) = payload.get("status").and_then(Value::as_str) {
                fields["status"] = Value::String(status.to_string());
            }
            out.kind(kinds::TOOL_CALL, call_id, fields);
        }
        "custom_tool_call_output" | "function_call_output" => {
            let call_id = payload
                .get("call_id")
                .and_then(Value::as_str)
                .map(String::from);
            out.kind(kinds::TOOL_RESULT, call_id, json!({"item_type": item_type}));
        }
        other => out.unrecognized(Some(&format!("response_item.{other}"))),
    }
}

fn event_msg(v: &Value, out: &mut NativeOut<'_>) {
    let Some(payload) = v.get("payload") else {
        out.unrecognized(Some("event_msg"));
        return;
    };
    match payload.get("type").and_then(Value::as_str) {
        Some("token_count") => {
            let info = payload.get("info");
            let mut any = false;
            if let Some(last) = info.and_then(|i| i.get("last_token_usage")) {
                let mut fields = json!({"scope": "per_call", "last_token_usage": last});
                if let Some(window) = info.and_then(|i| i.get("model_context_window")) {
                    fields["model_context_window"] = window.clone();
                }
                out.kind(kinds::USAGE, None, fields);
                any = true;
            }
            // The running total is preserved as its own, separately scoped observation —
            // never summed with the per-call deltas above into a total this decoder computed.
            if let Some(total) = info.and_then(|i| i.get("total_token_usage")) {
                out.kind(
                    kinds::USAGE,
                    None,
                    json!({"scope": "cumulative", "total_token_usage": total}),
                );
                any = true;
            }
            if !any {
                out.unrecognized(Some("event_msg.token_count"));
            }
        }
        Some("thread_settings_applied") => {
            if let Some(model) = dotted(payload, "thread_settings.model").and_then(Value::as_str) {
                out.kind(kinds::MODEL, None, json!({"via": "switch", "model": model}));
            } else {
                out.unrecognized(Some("event_msg.thread_settings_applied"));
            }
        }
        Some("task_complete") => {
            // A turn's own observed completion, never a claim that the wider task succeeded.
            let turn_id = payload
                .get("turn_id")
                .and_then(Value::as_str)
                .map(String::from);
            let mut fields = json!({"scope": "turn"});
            for key in [
                "duration_ms",
                "time_to_first_token_ms",
                "started_at",
                "completed_at",
            ] {
                if let Some(value) = payload.get(key) {
                    fields[key] = value.clone();
                }
            }
            out.kind(kinds::OUTCOME, turn_id, fields);
        }
        other => out.unrecognized(other.map(|t| format!("event_msg.{t}")).as_deref()),
    }
}
