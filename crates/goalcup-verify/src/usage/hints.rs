//! Where each harness keeps its thread hints (PRD-09 section 5): which reader reads which native
//! row, the wire thread hint and the reply id. The one file that names connectors.

use serde_json::Value;

/// A native session format this build reads: Codex `sessions:**/rollout-*.jsonl`, Claude Code
/// `transcripts:**/*.jsonl`, the `meta:**/*.meta.json` snapshots, and Hermes's `state:state.db*`
/// database copies (snapshot streams), Gemini CLI's `chats:**/*.jsonl` session files (calls,
/// threads and parent links), Qwen Code's `chats:**/*.jsonl` session files (threads, links and
/// calls), and Hermes's delegation files: `task-<n>.log` plain-text logs (read by path only, never
/// parsed) and the `manifest.json` snapshots (delegation id and task indexes).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Reader {
    CodexRollout,
    ClaudeTranscript,
    ClaudeMeta,
    HermesDb,
    GeminiChat,
    QwenChat,
    HermesTask,
    HermesManifest,
}

/// The reader of a native or snapshot stream by its connector and `meta.path` (`<row>:<rel>`).
pub(super) fn reader(connector: Option<&str>, path: &str) -> Option<Reader> {
    let (row, rel) = path.split_once(':')?;
    let file = rel.rsplit_once('/').map_or(rel, |(_, f)| f);
    Some(match (connector?, row, file.ends_with(".jsonl")) {
        ("codex-cli", "sessions", true) if file.starts_with("rollout-") => Reader::CodexRollout,
        ("claude-code-cli", "transcripts", true) => Reader::ClaudeTranscript,
        ("claude-code-cli", "meta", _) if file.ends_with(".meta.json") => Reader::ClaudeMeta,
        ("hermes-cli", "state", _) if file.starts_with("state.db") => Reader::HermesDb,
        ("hermes-cli", "delegation", _) if file.starts_with("task-") && file.ends_with(".log") => {
            Reader::HermesTask
        }
        ("hermes-cli", "delegation-manifest", _) if file == "manifest.json" => {
            Reader::HermesManifest
        }
        ("gemini-cli", "chats", true) => Reader::GeminiChat,
        ("qwen-cli", "chats", true) => Reader::QwenChat,
        _ => return None,
    })
}

/// Where a connector's wire requests name their calling thread and its parent: (connector,
/// thread keys, parent keys), the first key present wins. A key is a request header name (any
/// case), or a `/`-led JSON pointer into a WebSocket `response.create`. Only Codex also has a
/// native reader; the others' threads and parent links come from the wire alone.
const WIRE_IDS: &[(&str, &[&str], &[&str])] = &[
    (
        "codex-cli",
        &["thread-id", "/client_metadata/thread_id"],
        &["x-codex-parent-thread-id", "/client_metadata/x-codex-parent-thread-id"],
    ),
    (
        "opencode-cli",
        &["x-opencode-session-id", "x-session-id"],
        &["x-opencode-parent-session-id", "x-parent-session-id"],
    ),
    (
        "opencode2-cli",
        &["x-opencode-session-id", "x-session-id"],
        &["x-opencode-parent-session-id", "x-parent-session-id"],
    ),
    ("kilo-cli", &["x-session-id"], &["x-parent-session-id"]),
    ("goose-cli", &["agent-session-id"], &[]),
    ("crush-cli", &["x-session-id"], &[]),
];

/// For a connector this build reads threads for: (its wire requests carry their thread, as
/// [`WIRE_IDS`] says; native calls join the wire by the reply's request id, as Claude Code's do).
/// Hermes, Gemini and Qwen Code have neither: their calls join the entry's wire calls by time, a
/// wire call whose usage equals the native call's first (Gemini's chat `tokens` equal the reply's
/// `usageMetadata`, and Qwen's chat counts equal the wire's). A wire call no file holds (a Gemini
/// utility call, a Qwen memory extractor call) stays `WireOnly`.
pub(super) fn join_of(connector: Option<&str>) -> Option<(bool, bool)> {
    match connector? {
        "claude-code-cli" => Some((false, true)),
        "hermes-cli" | "gemini-cli" | "qwen-cli" => Some((false, false)),
        c => WIRE_IDS.iter().any(|w| w.0 == c).then_some((true, false)),
    }
}

/// True when this build has a native reader for the connector (an arm in [`reader`]); a
/// connector with only [`WIRE_IDS`] keeps [`super::Reason::NoReader`].
pub(super) fn reads_native(connector: Option<&str>) -> bool {
    matches!(
        connector,
        Some("codex-cli" | "claude-code-cli" | "hermes-cli" | "gemini-cli" | "qwen-cli")
    )
}

/// The thread a wire request names and its parent, by the connector's [`WIRE_IDS`]: from the
/// request headers (HTTP), else from the WebSocket `response.create` body. A parent equal to the
/// thread is dropped.
pub(super) fn wire_ids(
    connector: Option<&str>,
    headers: Option<&Value>,
    body: Option<&Value>,
) -> (Option<String>, Option<String>) {
    let Some(&(_, thread, parent)) = WIRE_IDS.iter().find(|w| Some(w.0) == connector) else {
        return (None, None);
    };
    let thread = first(thread, headers, body);
    let parent = first(parent, headers, body).filter(|p| Some(p) != thread.as_ref());
    (thread, parent)
}

/// The first of these keys present, as a header or a `/`-led body pointer.
fn first(keys: &[&str], headers: Option<&Value>, body: Option<&Value>) -> Option<String> {
    keys.iter().find_map(|k| {
        if k.starts_with('/') {
            id(body?.pointer(k)?.as_str()?)
        } else {
            header(headers, k)
        }
    })
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
