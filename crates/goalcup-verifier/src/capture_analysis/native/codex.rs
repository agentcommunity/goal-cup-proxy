//! The Codex rollout-JSONL format (`codex-jsonl`).
//!
//! Decoder version 2: measured on Codex CLI 0.153.0 CLI/exec rollouts and a 0.156.1
//! native child containing inherited context. Decoder version is independent of harness version.
//!
//! **Authorship.** The first `session_meta` fixes source authority for this stream, even
//! when malformed or ambiguous. A `cli`/`vscode` source with no child/fork indicators allows
//! the positive `user.text` marker on a user-role response item; an id is not required.
//! Exec input, child assignments and copied parent prompts never become new human input.
//! Later copied headers cannot promote the stream. Missing headers and loss before the first
//! header leave authority unavailable; established identity survives explicit capture loss.
//!
//! **Source-session provenance.** Parsed observations may carry `fields.native_context`:
//! `role` (interactive/headless/subagent/fork/unknown), supplied `launch_source`, bounded own
//! `session_id` from `payload.id` only, and agreeing supplied `parent_session_id`. `header_source`
//! references the first header's retained parts; `identity_field`/`parent_field` name the supplied
//! fields used. IDs over 256 bytes or inconsistent parent IDs are omitted. A parent-valued
//! `payload.session_id` never replaces the child's own id. `history_scope: may_include_inherited`
//! qualifies child/fork containers: it labels neither each record's author nor its freshness.
//! All copied tool/model/usage occurrences remain; no cross-stream deduplication is performed.
//! Unsupported records remain source-referenced unknowns, including session metadata itself.

use super::super::api::kinds;
use super::{NativeFormat, NativeOut, dotted};
use serde_json::{Value, json};

/// The Codex rollout-JSONL format.
#[derive(Debug, Clone, Copy, Default)]
pub struct Codex;

/// Bounded source identity and authorship authority belonging to one native stream.
#[derive(Default)]
pub struct CodexStream {
    /// The session's own declared `session_meta.payload.source`, once that record has been
    /// seen on this stream. `None` until then (or if it never arrives).
    launch: Option<&'static str>,
    /// The first header is authoritative even when unusable. Loss before it also closes
    /// this gate: a later inherited parent header cannot impersonate the missing owner.
    header_seen: bool,
    /// A small, bounded projection of the first header, never replaced by copied headers.
    native_context: Option<Value>,
    /// True once this `exec` stream's one delivered-goal record has been labelled as such.
    goal_excluded: bool,
}

impl NativeFormat for Codex {
    const HINT: &'static str = "codex-jsonl";
    const ID: &'static str = "codex-native";
    const VERSION: &'static str = "2";
    type Stream = CodexStream;

    fn lost_context(stream: &mut CodexStream) {
        // Established identity survives loss; the framework still reports that loss.
        stream.header_seen = true;
    }

    fn interpret(v: &Value, stream: &mut CodexStream, out: &mut NativeOut<'_>) {
        let record_type = v.get("type").and_then(Value::as_str);
        if record_type == Some("session_meta") && !stream.header_seen {
            stream.header_seen = true;
            stream.bind_header(v, out);
        }
        out.set_native_context(stream.native_context.as_ref());
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

/// Context strings are display/provenance identifiers, not arbitrary header content. The
/// three possible parent fields and one own id are each capped; no history-sized state.
const MAX_CONTEXT_ID_BYTES: usize = 256;

fn context_id(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= MAX_CONTEXT_ID_BYTES)
}

impl CodexStream {
    fn bind_header(&mut self, record: &Value, out: &NativeOut<'_>) {
        let payload = record.get("payload").unwrap_or(&Value::Null);
        let source = payload.get("source").and_then(Value::as_str);
        let thread_source = payload.get("thread_source");
        let subagent = dotted(payload, "source.subagent").is_some_and(|v| !v.is_null())
            || thread_source.and_then(Value::as_str) == Some("subagent");
        let parents: Vec<_> = [
            (
                "payload.source.subagent.thread_spawn.parent_thread_id",
                dotted(payload, "source.subagent.thread_spawn.parent_thread_id"),
            ),
            ("payload.parent_thread_id", payload.get("parent_thread_id")),
            ("payload.forked_from_id", payload.get("forked_from_id")),
        ]
        .into_iter()
        .filter_map(|(path, v)| v.filter(|v| !v.is_null()).map(|v| (path, v)))
        .collect();
        let ordinary_thread =
            thread_source.is_none_or(|v| v.is_null() || v.as_str() == Some("user"));
        let role = if subagent {
            "subagent"
        } else if !parents.is_empty() {
            "fork"
        } else if !ordinary_thread {
            "unknown"
        } else {
            match source {
                Some("cli" | "vscode") => "interactive",
                Some("exec") => "headless",
                _ => "unknown",
            }
        };
        self.launch = match (role, source) {
            ("interactive", Some("cli")) => Some("cli"),
            ("interactive", Some("vscode")) => Some("vscode"),
            ("headless", Some("exec")) => Some("exec"),
            _ => None,
        };
        let mut context = json!({"role": role, "header_source": out.source_ref()});
        if let Some(source @ ("cli" | "vscode" | "exec")) = source {
            context["launch_source"] = json!(source);
        } else if dotted(payload, "source.subagent.thread_spawn").is_some() {
            context["launch_source"] = json!("subagent.thread_spawn");
        }
        // In measured forked sources session_id can still name the parent. Only id is
        // the own identity; missing/oversize id does not remove ordinary CLI eligibility.
        if let Some(id) = payload.get("id").and_then(context_id) {
            context["session_id"] = json!(id);
            context["identity_field"] = json!("payload.id");
        }
        if let Some((path, first)) = parents.first()
            && let Some(parent) = context_id(first)
            && parents
                .iter()
                .all(|(_, value)| context_id(value) == Some(parent))
        {
            context["parent_session_id"] = json!(parent);
            context["parent_field"] = json!(path);
        }
        if matches!(role, "subagent" | "fork") {
            context["history_scope"] = json!("may_include_inherited");
        }
        self.native_context = Some(context);
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
            match stream.launch {
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
