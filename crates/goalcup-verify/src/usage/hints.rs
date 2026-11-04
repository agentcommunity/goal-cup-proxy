//! Where each harness keeps its thread hints (PRD-09 section 5): which reader reads which native
//! row, the wire thread hint and the reply id. The one file that names connectors.

use serde_json::Value;

/// A native session format this build reads: Codex `sessions:**/rollout-*.jsonl`, Claude Code
/// `transcripts:**/*.jsonl`, the `meta:**/*.meta.json` snapshots, and Hermes's `state:state.db*`
/// database copies (snapshot streams), Gemini CLI's `chats:**/*.jsonl` session files (calls,
/// threads and parent links), Qwen Code's `chats:**/*.jsonl` session files (threads, links and
/// calls), Hermes's delegation files: `task-<n>.log` plain-text logs (read by path only, never
/// parsed) and the `manifest.json` snapshots (delegation id and task indexes), Kimi Code's
/// `wire:**/agents/<name>/wire.jsonl` agent files (one per agent), and the session database copies
/// of Crush (`db:crush.db*`) and Goose (`sessions:sessions.db*`), read for ids and parent ids
/// only, as [`thread_db`] says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reader {
    CodexRollout,
    ClaudeTranscript,
    ClaudeMeta,
    HermesDb,
    GeminiChat,
    QwenChat,
    HermesTask,
    HermesManifest,
    KimiWire,
    ThreadDb,
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
        ("kimi-code-cli", "wire", true) if file == "wire.jsonl" => Reader::KimiWire,
        ("crush-cli", "db", _) if file.starts_with("crush.db") => Reader::ThreadDb,
        ("goose-cli", "sessions", _) if file.starts_with("sessions.db") => Reader::ThreadDb,
        _ => return None,
    })
}

/// Where a connector's wire requests name their calling thread and its parent: (connector,
/// thread keys, parent keys), the first key present wins. A key is a request header name (any
/// case), or a `/`-led JSON pointer into a WebSocket `response.create`. Only Codex also has a
/// native reader; the others' threads and parent links come from the wire alone, except Crush's
/// and Goose's parent links, which come from their [`thread_db`].
const WIRE_IDS: &[(&str, &[&str], &[&str])] = &[
    (
        "codex-cli",
        &["thread-id", "/client_metadata/thread_id"],
        &[
            "x-codex-parent-thread-id",
            "/client_metadata/x-codex-parent-thread-id",
        ],
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
/// Amp's threads come from its thread socket ([`thread_rpc`]), with no native leg to join.
/// Hermes, Gemini, Qwen Code and Kimi Code have neither: their calls join the entry's wire calls
/// by time, a wire call whose usage equals the native call's first (Gemini's chat `tokens` equal
/// the reply's `usageMetadata`, and Qwen's chat counts equal the wire's). Kimi's
/// `prompt_cache_key` names only the session, which main and subagents share. A wire call no file
/// holds (a Gemini utility call, a Qwen memory extractor call) stays `WireOnly`.
pub(super) fn join_of(connector: Option<&str>) -> Option<(bool, bool)> {
    match connector? {
        "claude-code-cli" => Some((false, true)),
        "hermes-cli" | "gemini-cli" | "qwen-cli" | "kimi-code-cli" | "amp-cli" => {
            Some((false, false))
        }
        c => WIRE_IDS.iter().any(|w| w.0 == c).then_some((true, false)),
    }
}

/// True when the connector's WebSocket is a thread socket of JSON-RPC frames whose server
/// messages report each model turn ([`super::rpc`]): Amp's `/actors/gateway/threadActor/`.
pub(super) fn thread_rpc(connector: Option<&str>) -> bool {
    connector == Some("amp-cli")
}

/// True when this build has a native reader for the connector (an arm in [`reader`]); a
/// connector with only [`WIRE_IDS`] keeps [`super::Reason::NoReader`].
pub(super) fn reads_native(connector: Option<&str>) -> bool {
    const READ: [&str; 6] = [
        "codex-cli",
        "claude-code-cli",
        "hermes-cli",
        "gemini-cli",
        "qwen-cli",
        "kimi-code-cli",
    ];
    connector.is_some_and(|c| READ.contains(&c))
}

/// A session database that gives parent links only, never calls: its table, its id and parent
/// columns, and whether the wire names a session by the xxh3-64 of its id (16 lowercase hex
/// digits) rather than by the id. No other column is read.
pub(super) struct ThreadDb {
    pub(super) table: &'static str,
    pub(super) id: &'static str,
    pub(super) parent: &'static str,
    pub(super) hash: bool,
}

/// The [`ThreadDb`] of a connector. Crush 0.99.0 (`internal/db/migrations`, `session.HashID`):
/// `sessions(id, parent_session_id)`, its `X-Session-Id` header the xxh3-64 of the id. Goose
/// 1.57.0 (`session_manager.rs`): `sessions(id, parent_session_id)`, its `agent-session-id`
/// header the id itself.
pub(super) fn thread_db(connector: Option<&str>) -> Option<ThreadDb> {
    let hash = match connector? {
        "crush-cli" => true,
        "goose-cli" => false,
        _ => return None,
    };
    Some(ThreadDb {
        table: "sessions",
        id: "id",
        parent: "parent_session_id",
        hash,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN: [&str; 16] = [
        "amp-cli",
        "claude-code-cli",
        "cline-cli",
        "codex-cli",
        "copilot-cli",
        "crush-cli",
        "droid-cli",
        "gemini-cli",
        "goose-cli",
        "hermes-cli",
        "kilo-cli",
        "kimi-code-cli",
        "openclaw-cli",
        "opencode-cli",
        "opencode2-cli",
        "qwen-cli",
    ];

    #[test]
    fn the_connector_arms_as_of_1_0() {
        use Reader::*;
        let r = |c: &str, p: &str| reader(Some(c), p);
        assert_eq!(
            r("codex-cli", "sessions:2026/rollout-1.jsonl"),
            Some(CodexRollout)
        );
        assert_eq!(r("codex-cli", "sessions:2026/x.jsonl"), None);
        assert_eq!(
            r("claude-code-cli", "transcripts:p/s.jsonl"),
            Some(ClaudeTranscript)
        );
        assert_eq!(r("claude-code-cli", "meta:p/a.meta.json"), Some(ClaudeMeta));
        assert_eq!(r("hermes-cli", "state:state.db-wal"), Some(HermesDb));
        assert_eq!(r("hermes-cli", "delegation:d/task-3.log"), Some(HermesTask));
        assert_eq!(
            r("hermes-cli", "delegation-manifest:d/manifest.json"),
            Some(HermesManifest)
        );
        assert_eq!(r("gemini-cli", "chats:h/chats/s.jsonl"), Some(GeminiChat));
        assert_eq!(r("qwen-cli", "chats:p/c.jsonl"), Some(QwenChat));
        assert_eq!(r("qwen-cli", "meta:p/a.meta.json"), None);
        assert_eq!(
            r("kimi-code-cli", "wire:s/agents/main/wire.jsonl"),
            Some(KimiWire)
        );
        assert_eq!(r("crush-cli", "db:crush.db"), Some(ThreadDb));
        assert_eq!(r("goose-cli", "sessions:sessions.db-shm"), Some(ThreadDb));
        assert_eq!(reader(None, "chats:a.jsonl"), None);
        let j = |c: &str| join_of(Some(c));
        assert_eq!(j("claude-code-cli"), Some((false, true)));
        for c in [
            "hermes-cli",
            "gemini-cli",
            "qwen-cli",
            "kimi-code-cli",
            "amp-cli",
        ] {
            assert_eq!(j(c), Some((false, false)), "{c}");
        }
        for c in [
            "codex-cli",
            "opencode-cli",
            "opencode2-cli",
            "kilo-cli",
            "goose-cli",
            "crush-cli",
        ] {
            assert_eq!(j(c), Some((true, false)), "{c}");
        }
        for c in [
            "cline-cli",
            "copilot-cli",
            "droid-cli",
            "openclaw-cli",
            "nope-cli",
        ] {
            assert_eq!(j(c), None, "{c}");
        }
        let native: Vec<_> = KNOWN
            .into_iter()
            .filter(|c| reads_native(Some(c)))
            .collect();
        assert_eq!(
            native,
            [
                "claude-code-cli",
                "codex-cli",
                "gemini-cli",
                "hermes-cli",
                "kimi-code-cli",
                "qwen-cli"
            ]
        );
        assert!(thread_rpc(Some("amp-cli")) && !thread_rpc(Some("codex-cli")));
        let db = |c| thread_db(Some(c)).map(|d| (d.table, d.id, d.parent, d.hash));
        assert_eq!(
            db("crush-cli"),
            Some(("sessions", "id", "parent_session_id", true))
        );
        assert_eq!(
            db("goose-cli"),
            Some(("sessions", "id", "parent_session_id", false))
        );
        assert_eq!(db("hermes-cli").map(|_| ()), None);
        let h = serde_json::json!([["X-Opencode-Session-Id", "k"], ["x-parent-session-id", "m"]]);
        assert_eq!(
            wire_ids(Some("opencode2-cli"), Some(&h), None),
            (Some("k".into()), Some("m".into()))
        );
        let b = serde_json::json!({"client_metadata": {"thread_id": "t", "x-codex-parent-thread-id": "p"}});
        assert_eq!(
            wire_ids(Some("codex-cli"), None, Some(&b)),
            (Some("t".into()), Some("p".into()))
        );
        assert_eq!(
            wire_ids(
                Some("goose-cli"),
                Some(&serde_json::json!([["agent-session-id", "g"]])),
                None
            ),
            (Some("g".into()), None)
        );
    }
}
