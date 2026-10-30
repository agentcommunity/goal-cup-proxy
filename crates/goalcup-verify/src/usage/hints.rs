//! Where each harness keeps its thread hints (PRD-09 section 5): which reader reads which native
//! row, the wire thread hint and the reply id. The one file that names connectors.

use serde_json::Value;

/// A native session format this build reads: Codex `sessions:**/rollout-*.jsonl`, Claude Code
/// `transcripts:**/*.jsonl` and the `meta:**/*.meta.json` snapshots.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Reader {
    CodexRollout,
    ClaudeTranscript,
    ClaudeMeta,
}

/// The reader of a native or snapshot stream by its connector and `meta.path` (`<row>:<rel>`).
pub(super) fn reader(connector: Option<&str>, path: &str) -> Option<Reader> {
    let (row, rel) = path.split_once(':')?;
    let file = rel.rsplit_once('/').map_or(rel, |(_, f)| f);
    let r = match (connector?, row, file.ends_with(".jsonl")) {
        ("codex-cli", "sessions", true) if file.starts_with("rollout-") => Reader::CodexRollout,
        ("claude-code-cli", "transcripts", true) => Reader::ClaudeTranscript,
        ("claude-code-cli", "meta", _) if file.ends_with(".meta.json") => Reader::ClaudeMeta,
        _ => return None,
    };
    Some(r)
}

/// For a connector this build has a native reader for: (its wire requests carry their thread, as
/// Codex's do; native calls join the wire by the reply's request id, as Claude Code's do).
pub(super) fn join_of(connector: Option<&str>) -> Option<(bool, bool)> {
    match connector? {
        "codex-cli" => Some((true, false)),
        "claude-code-cli" => Some((false, true)),
        _ => None,
    }
}

/// The thread a wire request names: Codex's `thread-id` header (HTTP), else
/// `client_metadata.thread_id` of a WebSocket `response.create`.
pub(super) fn wire_thread(
    connector: Option<&str>,
    headers: Option<&Value>,
    body: Option<&Value>,
) -> Option<String> {
    let body = || id(body?.pointer("/client_metadata/thread_id")?.as_str()?);
    let hint = join_of(connector).is_some_and(|j| j.0);
    hint.then(|| header(headers, "thread-id").or_else(body))?
}

/// The reply's `request-id` header; Claude Code's native `requestId` equals it.
pub(super) fn reply_id(headers: Option<&Value>) -> Option<String> {
    header(headers, "request-id")
}

/// A header value by name (any case) from a meta `headers` list of `[name, value]` pairs.
fn header(headers: Option<&Value>, name: &str) -> Option<String> {
    let named = |p: &&Value| p[0].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name));
    id(headers?.as_array()?.iter().find(named)?[1].as_str()?)
}

/// An id or label, kept only as printable ASCII of at most 128 bytes and never all `*` (masked).
pub(super) fn id(s: &str) -> Option<String> {
    let ok = !s.is_empty()
        && s.len() <= 128
        && s.bytes().all(|b| b.is_ascii_graphic())
        && !s.bytes().all(|b| b == b'*');
    ok.then(|| s.to_string())
}
