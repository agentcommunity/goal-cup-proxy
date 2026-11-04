//! Where each harness keeps its thread hints (PRD-09 section 5): which reader reads which native
//! row, the wire thread hint and the reply id. Each comes from the `[usage]` table of the built-in
//! connector of that id (docs/CONNECTOR-SCHEMA.md#the-usage-table); a connector without one is
//! wire only.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::Value;

/// A native session format this build reads: Codex `sessions:**/rollout-*.jsonl`, Claude Code
/// `transcripts:**/*.jsonl`, the `meta:**/*.meta.json` snapshots, and Hermes's `state:state.db*`
/// database copies (snapshot streams), Gemini CLI's `chats:**/*.jsonl` session files (calls,
/// threads and parent links), Qwen Code's `chats:**/*.jsonl` session files (threads, links and
/// calls), Hermes's delegation files: `task-<n>.log` plain-text logs (read by path only, never
/// parsed) and the `manifest.json` snapshots (delegation id and task indexes), Kimi Code's
/// `wire:**/agents/<name>/wire.jsonl` agent files (one per agent), and the session database copies
/// of Crush (`db:crush.db*`) and Goose (`sessions:sessions.db*`), read for ids and parent ids
/// only, as [`thread_db`] says. `ThreadDb` comes from a `thread_db` row, never from `readers`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
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
    #[serde(skip_deserializing)]
    ThreadDb,
}

/// A connector's `[usage]` table. A `thread` or `parent` key is a request header name (any case),
/// or a `/`-led JSON pointer into a WebSocket `response.create`; the first key present wins.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Usage {
    readers: Vec<ReaderRow>,
    thread: Vec<String>,
    parent: Vec<String>,
    join: Option<JoinBy>,
    thread_rpc: bool,
    thread_db: Option<DbRow>,
}

/// A native row's files (`file`: a name with at most one `*`) and the reader that reads them.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReaderRow {
    row: String,
    file: String,
    reader: Reader,
}

/// A native row of session database copies, as [`ThreadDb`].
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DbRow {
    row: String,
    file: String,
    table: String,
    id: String,
    parent: String,
    xxh3: bool,
}

/// How native calls join the wire, as [`join_of`] says.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum JoinBy {
    Thread,
    ReplyId,
    Time,
}

/// A connector file: its id and `[usage]`; every other key is the recorder's.
#[derive(Deserialize)]
struct File {
    id: String,
    #[serde(default)]
    usage: Usage,
}

/// A connector file's id and `[usage]` table, refused when a `file` holds a `/` or more than one
/// `*`, when `join` is missing although another key is set (or is `"thread"` with no `thread`
/// key), or when `thread_db` sits beside `readers` (the native reader keeps one or the other).
fn parse(text: &str) -> Result<(String, Usage), String> {
    let File { id, usage: u } = toml::from_str(text).map_err(|e| e.to_string())?;
    let files = u.readers.iter().map(|r| &r.file);
    if let Some(f) = files
        .chain(u.thread_db.iter().map(|d| &d.file))
        .find(|f| f.contains('/') || f.matches('*').count() > 1)
    {
        return Err(format!(
            "{id}: usage file {f:?} is not a file name with at most one `*`"
        ));
    }
    let set = !u.readers.is_empty()
        || !u.thread.is_empty()
        || !u.parent.is_empty()
        || u.thread_rpc
        || u.thread_db.is_some();
    let bad = match u.join {
        None => set.then_some("needs a `join`"),
        Some(JoinBy::Thread) if u.thread.is_empty() => {
            Some("join = \"thread\" needs a `thread` key")
        }
        _ => None,
    };
    let bad = bad.or((u.thread_db.is_some() && !u.readers.is_empty())
        .then_some("has both `readers` and `thread_db`"));
    match bad {
        Some(b) => Err(format!("{id}: usage {b}")),
        None => Ok((id, u)),
    }
}

/// The `[usage]` table of a built-in connector. One that does not parse reads as none (the
/// `every_builtin_usage_table_parses` test keeps the built-in ones parsing).
pub(super) fn usage(connector: Option<&str>) -> Option<&'static Usage> {
    static USAGE: LazyLock<BTreeMap<String, Usage>> = LazyLock::new(|| {
        let all = goalcup_format::connectors::CONNECTORS.iter();
        all.filter_map(|(_, text)| parse(text).ok()).collect()
    });
    USAGE.get(connector?)
}

/// True when `name` fits `pat`: equal, or with one `*` standing for any run of characters.
fn fits(pat: &str, name: &str) -> bool {
    match pat.split_once('*') {
        None => pat == name,
        Some((pre, suf)) => {
            name.len() >= pre.len() + suf.len() && name.starts_with(pre) && name.ends_with(suf)
        }
    }
}

/// The reader of a native or snapshot stream by its connector and `meta.path` (`<row>:<rel>`):
/// the first `readers` row whose row and file fit, else the `thread_db` row. Rows are not checked
/// against the connector's `native` rows, so a row of an earlier build still reads.
pub(super) fn reader(connector: Option<&str>, path: &str) -> Option<Reader> {
    let (row, rel) = path.split_once(':')?;
    let file = rel.rsplit_once('/').map_or(rel, |(_, f)| f);
    let u = usage(connector)?;
    let hit = |r: &str, pat: &str| r == row && fits(pat, file);
    let db = u.thread_db.as_ref().filter(|d| hit(&d.row, &d.file));
    let read = u.readers.iter().find(|r| hit(&r.row, &r.file));
    read.map(|r| r.reader).or(db.map(|_| Reader::ThreadDb))
}

/// For a connector this build reads threads for: (its wire requests carry their thread, `join =
/// "thread"`; native calls join the wire by the reply's request id, `join = "reply-id"`, as
/// Claude Code's do). With `join = "time"` they have neither: their native calls join the entry's
/// wire calls by time, a wire call whose usage equals the native call's first. A wire call no
/// file holds stays `WireOnly`.
pub(super) fn join_of(connector: Option<&str>) -> Option<(bool, bool)> {
    Some(match usage(connector)?.join? {
        JoinBy::Thread => (true, false),
        JoinBy::ReplyId => (false, true),
        JoinBy::Time => (false, false),
    })
}

/// True when the connector's WebSocket is a thread socket of JSON-RPC frames whose server
/// messages report each model turn ([`super::rpc`]).
pub(super) fn thread_rpc(connector: Option<&str>) -> bool {
    usage(connector).is_some_and(|u| u.thread_rpc)
}

/// True when this build has a native reader for the connector (a `readers` row); a connector
/// with only wire thread keys keeps [`super::Reason::NoReader`].
pub(super) fn reads_native(connector: Option<&str>) -> bool {
    usage(connector).is_some_and(|u| !u.readers.is_empty())
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

/// The [`ThreadDb`] of a connector, from its `thread_db` row.
pub(super) fn thread_db(connector: Option<&str>) -> Option<ThreadDb> {
    let d = usage(connector)?.thread_db.as_ref()?;
    Some(ThreadDb {
        table: &d.table,
        id: &d.id,
        parent: &d.parent,
        hash: d.xxh3,
    })
}

/// The thread a wire request names and its parent, by the connector's `thread` and `parent`
/// keys: from the request headers (HTTP), else from the WebSocket `response.create` body. A
/// parent equal to the thread is dropped.
pub(super) fn wire_ids(
    connector: Option<&str>,
    headers: Option<&Value>,
    body: Option<&Value>,
) -> (Option<String>, Option<String>) {
    let Some(u) = usage(connector) else {
        return (None, None);
    };
    let thread = first(&u.thread, headers, body);
    let parent = first(&u.parent, headers, body).filter(|p| Some(p) != thread.as_ref());
    (thread, parent)
}

/// The first of these keys present, as a header or a `/`-led body pointer.
fn first(keys: &[String], headers: Option<&Value>, body: Option<&Value>) -> Option<String> {
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

    #[test]
    fn every_builtin_usage_table_parses() {
        for (id, text) in goalcup_format::connectors::CONNECTORS {
            assert_eq!(parse(text).map(|u| u.0).as_deref(), Ok(*id));
        }
    }

    #[test]
    fn usage_refusals() {
        let bad = |usage: &str| parse(&format!("id = \"t-cli\"\n[usage]\n{usage}\n")).is_err();
        assert!(bad(
            r#"readers = [{ row = "db", file = "a.db", reader = "thread-db" }]"#
        ));
        assert!(bad(
            r#"readers = [{ row = "db", file = "a*b*", reader = "kimi-wire" }]
join = "time""#
        ));
        assert!(bad(r#"join = "thread""#));
        assert!(bad(r#"threads = ["x"]"#));
        assert!(bad(r#"thread = ["x"]"#), "no join");
        assert!(bad(r#"parent = ["x"]"#), "no join");
        let wire =
            r#"readers = [{ row = "w", file = "agents/*/wire.jsonl", reader = "kimi-wire" }]"#;
        assert!(bad(&format!("{wire}\njoin = \"time\"")), "a / in file");
        let db = r#"thread_db = { row = "s", file = "s.db*", table = "t", id = "i", parent = "p", xxh3 = false }"#;
        let read = r#"readers = [{ row = "s", file = "s.db*", reader = "hermes-db" }]"#;
        assert!(
            bad(&format!("{read}\n{db}\njoin = \"time\"")),
            "readers and thread_db"
        );
        assert!(!bad("thread = [\"x\"]\njoin = \"thread\""));
    }
}
