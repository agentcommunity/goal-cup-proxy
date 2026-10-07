//! `goalcup-mcp` — three read-only tools over the signed status cache (D8, D25).
//!
//! # What this server is, said negatively
//!
//! It opens no socket, holds no token, and never talks to the daemon. It reads two files —
//! `<config_dir>/cache/status-<id>.json` and `<config_dir>/cache/status-latest.json`, the
//! pair `goalcup_daemon::status::mirror` writes — and one record per run under
//! `<config_dir>/entries/`, and it answers three questions off them. An agent that finds
//! this binary cannot use it to reach the control API, and nothing it reads can tell it to:
//! a status document is data, and this crate treats it as data.
//!
//! It is shipped and never auto-registered. The participant decides whether their agent
//! gets these tools at all, which is why `SKILL.md` says when NOT to call them as plainly
//! as it says when to.
//!
//! # Every answer comes from a document that verified against a PINNED key
//!
//! [`SignedStatus::verify_with`] takes the key the caller pinned, and there is deliberately
//! no `verify(&self)` in the protocol crate. The status route is unauthenticated by ruling,
//! so a local process that bound the control port first could serve a document signed with
//! its own key; checking a document against the key it carries would report a forged
//! countdown as trusted, and a forged countdown that vouches for itself is worse than no
//! countdown. The key this server checks with is the one `goalcup start` wrote into
//! `<config_dir>/entries/<id>.json` from `WorkspaceInfo.run_public_key` at the moment the
//! daemon handed it over — **never the key field inside the document**. A cached status
//! whose entry has no such record is [`McpError::Unverified`] and never data: this server
//! has nothing to check it with.
//!
//! The signature is not the whole check. One run key signs every document of that run, so a
//! document about entry B verifies perfectly in entry A's file; the id in the payload is
//! compared with the id that was asked for, and a mismatch is refused for the same reason
//! `goalcup status` refuses it.
//!
//! # The duplication with `goalcup-cli` is deliberate
//!
//! The pinned-key rule and the staleness threshold are implemented twice on purpose — once
//! here and once in `goalcup-cli` — because this crate must not link the CLI: a shipped MCP
//! binary that did would carry the CLI's whole surface into a process an agent is invited
//! to call. What is NOT duplicated is the threshold's value:
//! [`goalcup_protocol::control::STALE_AFTER_SECONDS`] is read from the protocol crate, so
//! the page, `goalcup status` and this server cannot drift into telling a participant three
//! different stories about one daemon. The entry record's shape is duplicated as
//! [`EntryKeyRecord`], and `tests/protocol.rs` reads a record `goalcup-cli`'s own writer
//! produced so the two shapes cannot drift unobserved either.
//!
//! # What the tools deliberately do not carry
//!
//! No workspace path, no session id, no arming recipe, no per-cadence counter. The
//! documents carry them and a human surface prints them (`goalcup status`), but an agent
//! asking how long it has does not need the participant's disk layout, and the arming
//! recipe is a step for a human — D8 keeps the tracker from speaking to the agent, and a
//! tool that handed over an instruction would be it speaking.
#![forbid(unsafe_code)]

use goalcup_protocol::control::{STALE_AFTER_SECONDS, SignedStatus, parse_z};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The MCP protocol versions this server supports, oldest first. There is one.
///
/// `2025-03-26` is not advertised. That revision's specification REQUIRES a server to
/// accept a JSON-RPC batch; this server has never implemented one and refuses every batch
/// with [`INVALID_REQUEST`]. Advertising a version whose batching a client is entitled to
/// use would be a promise this process does not keep, and the client that took it up would
/// be told its well-formed request was invalid. Batching was removed from the protocol in
/// `2025-06-18`, so the one version in here is the one whose rules this server actually
/// follows.
///
/// Pinned rather than tracked: a server that claimed whatever version a client asked for
/// would be claiming behaviour it has not been written against. To support another
/// revision, check the current MCP specification and **add** to this array rather than
/// replacing what is in it, recording the delta as a doc comment. The one reason to take an
/// entry out is the one above: a version this server does not in fact implement. A client
/// that asks for something not in here is answered with [`NEWEST_PROTOCOL_VERSION`] and not
/// with an error, which is what the specification's negotiation says to do.
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 1] = [NEWEST_PROTOCOL_VERSION];

/// The newest of [`SUPPORTED_PROTOCOL_VERSIONS`], and the answer to a version this server
/// does not know. Named, so nothing here has to index the array to find it.
pub const NEWEST_PROTOCOL_VERSION: &str = "2025-06-18";

/// The name this server answers `initialize` with.
pub const SERVER_NAME: &str = "goalcup";

/// The three tools, in the order `tools/list` lists them and `SKILL.md` describes them.
pub const TOOL_EVENT_STATUS: &str = "goalcup_event_status";
pub const TOOL_RUN_STATUS: &str = "goalcup_run_status";
pub const TOOL_ENTRIES: &str = "goalcup_entries";

/// The most of one cached document this server will read, and the most of one request line
/// it will hold.
///
/// The cache is under a directory the participant owns. A run that pointed
/// `$GOALCUP_CONFIG_DIR` at a directory holding a large file, by accident or otherwise,
/// would otherwise have had this process read the whole of it into memory before serde
/// looked at the first byte. The daemon's own documents are a few kilobytes.
///
/// `main.rs` bounds a line of stdin with the same number rather than a second one of its
/// own: a request and a document are both "one JSON value this process holds in memory",
/// and two caps would be two numbers to keep in step for no gain.
pub const MAX_DOCUMENT_BYTES: u64 = 4 * 1024 * 1024;

/// `<config_dir>/cache`, the only directory this server reads a document from.
const CACHE_DIR_NAME: &str = "cache";

/// `<config_dir>/entries`, where `goalcup start` pinned the run keys.
const ENTRIES_DIR_NAME: &str = "entries";

/// The pointer file the daemon keeps at the most recently started run.
const LATEST_FILE_NAME: &str = "status-latest.json";

/// What can go wrong before an answer exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpError {
    /// This machine has no document for the run that was asked about.
    NoEntry,
    /// A document was found and this server will not repeat anything out of it: it did not
    /// verify against the pinned run key, there is no pinned key to check it with, or it is
    /// about a different run than the one asked for.
    Unverified(String),
    Io(String),
    DocumentTooLarge {
        limit: u64,
    },
    Json(String),
    /// This server could not put a verified document into words. Not the document's fault
    /// and not the participant's: a shape this file assumes did not hold. It is an error
    /// rather than a placeholder because a placeholder would read like a state.
    Internal(String),
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpError::NoEntry => write!(
                f,
                "no entry: this machine holds no status document for that run. A run started \
                 on this machine publishes one, and `goalcup entries` lists what there is."
            ),
            McpError::Unverified(said) => write!(f, "{said}"),
            McpError::Io(said) => write!(f, "the status cache could not be read: {said}"),
            McpError::DocumentTooLarge { limit } => write!(
                f,
                "the status cache is larger than a status document ({limit} byte limit), \
                 and this server reads no further"
            ),
            McpError::Json(said) => {
                write!(f, "the status cache did not read as a document: {said}")
            }
            McpError::Internal(said) => {
                write!(f, "this server could not report the status: {said}.")
            }
        }
    }
}

impl std::error::Error for McpError {}

/// The run key this server checks a cached status against, pinned from `WorkspaceInfo` when
/// the entry was created and stored beside the cache. NEVER the key field inside the
/// document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedKey(pub String);

/// The two fields of `<config_dir>/entries/<id>.json` this server needs.
///
/// `goalcup-cli`'s `entries::EntryRecord` is the writer and carries more; this is a reader
/// of the same file with no shared type between them, because linking that crate would pull
/// its whole surface into this process. Unknown fields are ignored on purpose: the CLI may
/// add one, and a record this server refused to read would be a run it could not verify.
#[derive(Debug, Deserialize)]
struct EntryKeyRecord {
    entry_id: String,
    run_public_key: String,
}

/// The signed cache, and the keys to check it with.
pub struct StatusSource {
    cache: PathBuf,
    keys: BTreeMap<String, PinnedKey>,
}

impl StatusSource {
    /// Reads `<config_dir>/entries/*.json` into `keys` (entry id → pinned run key) and
    /// points `cache` at `<config_dir>/cache/`.
    ///
    /// A configuration directory with no `entries/` in it is an empty map and not an error:
    /// a machine that has never started a run is the ordinary case, and the answer to every
    /// tool on it is "no entry". A record that does not parse is skipped rather than fatal,
    /// for the reason `goalcup-cli` skips one: an unreadable record for one run is not a
    /// reason the others cannot be answered for.
    pub fn open(config_dir: &Path) -> Result<StatusSource, McpError> {
        let mut keys = BTreeMap::new();
        let dir = config_dir.join(ENTRIES_DIR_NAME);
        match std::fs::read_dir(&dir) {
            Ok(records) => {
                for record in records.flatten() {
                    let path = record.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("json") {
                        continue;
                    }
                    let Ok(text) = read_bounded(&path) else {
                        continue;
                    };
                    let Ok(rec) = serde_json::from_str::<EntryKeyRecord>(&text) else {
                        continue;
                    };
                    // The record has to be in the file `goalcup start` would have written
                    // it to. Without this, two records claiming one entry id would both
                    // pin a key for it and whichever the directory listed last would win —
                    // a key this server checks with, chosen by the order a directory came
                    // back in. At most one file can be `<id>.json`.
                    if path.file_stem().and_then(|s| s.to_str()) != Some(rec.entry_id.as_str()) {
                        continue;
                    }
                    keys.insert(rec.entry_id, PinnedKey(rec.run_public_key));
                }
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(McpError::Io(source.to_string())),
        }
        Ok(StatusSource {
            cache: config_dir.join(CACHE_DIR_NAME),
            keys,
        })
    }

    /// The document at `status-latest.json` — the most recently started run — verified.
    pub fn latest(&self) -> Result<SignedStatus, McpError> {
        let signed = read_document(&self.cache.join(LATEST_FILE_NAME))?;
        self.checked(signed, None)
    }

    /// One run's document, verified, and refused unless the payload is about `id`.
    pub fn for_entry(&self, id: &str) -> Result<SignedStatus, McpError> {
        if !one_path_component(id) {
            return Err(McpError::NoEntry);
        }
        let signed = read_document(&self.cache.join(format!("status-{id}.json")))?;
        self.checked(signed, Some(id))
    }

    /// Every run this machine holds a document for, in entry-id order.
    ///
    /// Only the documents that verified. What was left out is counted rather than dropped
    /// silently — see [`StatusSource::scan`], which is what `goalcup_entries` answers from.
    pub fn all(&self) -> Result<Vec<SignedStatus>, McpError> {
        Ok(self.scan()?.verified)
    }

    /// Every document in the cache, split into the ones that checked out and the file names
    /// of the ones that did not.
    ///
    /// A list that quietly omitted a forged document would be this server hiding the one
    /// thing worth reporting about it, so the count of refusals is part of the answer.
    /// `status-latest.json` is skipped: it is a copy of one of the others, and listing it
    /// would report one run twice.
    fn scan(&self) -> Result<Scan, McpError> {
        let mut out = Scan::default();
        let files = match std::fs::read_dir(&self.cache) {
            Ok(files) => files,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(source) => return Err(McpError::Io(source.to_string())),
        };
        let mut named: Vec<(String, PathBuf)> = Vec::new();
        for file in files.flatten() {
            let path = file.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name == LATEST_FILE_NAME {
                continue;
            }
            let Some(id) = name
                .strip_prefix("status-")
                .and_then(|n| n.strip_suffix(".json"))
            else {
                continue;
            };
            named.push((id.to_string(), path.clone()));
        }
        named.sort();
        for (id, path) in named {
            match read_document(&path).and_then(|signed| self.checked(signed, Some(&id))) {
                Ok(signed) => out.verified.push(signed),
                Err(_) => out.refused.push(id),
            }
        }
        Ok(out)
    }

    /// The one gate every answer goes through.
    ///
    /// Three questions, in this order: is this document about the run that was asked for,
    /// is there a key this machine pinned for that run, and does the signature check out
    /// against it. A `no` to any of them is a refusal carrying nothing out of the payload.
    fn checked(&self, signed: SignedStatus, asked: Option<&str>) -> Result<SignedStatus, McpError> {
        let id = signed.payload.entry_id.clone();
        // Every id that reaches a message goes through `as_an_id` first: until the signature
        // has been checked the id in the payload is text a rogue writer chose, and a refusal
        // that quoted it whole would be this server carrying that writer's words into an
        // agent's context.
        if let Some(asked) = asked
            && asked != id
        {
            return Err(McpError::Unverified(format!(
                "the cached document for entry {} is about entry {}, so nothing in it has \
                 been reported. One run key signs every document of that run, so a good \
                 signature does not make a document be about the run that was asked for.",
                as_an_id(asked),
                as_an_id(&id)
            )));
        }
        let Some(key) = self.keys.get(&id) else {
            return Err(McpError::Unverified(format!(
                "this machine pinned no run key for entry {}, so there is nothing to check \
                 its status document against and none of it has been reported. Only a run \
                 started from this machine leaves a key behind, and the key inside a document \
                 vouches for itself.",
                as_an_id(&id)
            )));
        };
        match signed.verify_with(&key.0) {
            Ok(()) => Ok(signed),
            Err(_) => Err(McpError::Unverified(format!(
                "the status document for entry {} did not verify against the run key this \
                 machine pinned when the entry was created, so nothing in it has been \
                 reported. The status route is unauthenticated by ruling: something other \
                 than the daemon that opened this run may have written it.",
                as_an_id(&id)
            ))),
        }
    }
}

/// What one pass over the cache found.
#[derive(Default)]
struct Scan {
    verified: Vec<SignedStatus>,
    refused: Vec<String>,
}

/// The longest entry id a refusal will repeat. A daemon-minted one is a dozen characters.
const ID_MAX_CHARS: usize = 64;

/// What a refusal is allowed to say the entry was called.
const NOT_AN_ID: &str = "<a name that is not an entry id>";

/// An entry id as a refusal may repeat it, or [`NOT_AN_ID`] if it does not look like one.
///
/// A refusal is written because a document could NOT be vouched for, which means the id
/// inside it is text whoever wrote that document chose. Repeating it would carry their
/// words — a paragraph of instructions, a control character, a megabyte of anything — into
/// the context of the agent that asked a question about the time. Masking the characters
/// one at a time is not enough: the words survive the masking and it is the words that do
/// the damage. So it is quoted whole or not at all.
fn as_an_id(id: &str) -> String {
    let looks_like_one = !id.is_empty()
        && id.chars().count() <= ID_MAX_CHARS
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    match looks_like_one {
        true => id.to_string(),
        false => NOT_AN_ID.to_string(),
    }
}

/// One entry id is one path component, or it does not name a file in the cache.
///
/// The id arrives from an agent's tool call and is interpolated into a path, which is the
/// shape that has to be checked rather than trusted. Refused rather than sanitised.
fn one_path_component(id: &str) -> bool {
    !(id.is_empty()
        || id == "."
        || id == ".."
        || id.contains('/')
        || id.contains('\\')
        || id.contains('\0'))
}

/// At most [`MAX_DOCUMENT_BYTES`] of `path`, as text.
fn read_bounded(path: &Path) -> Result<String, McpError> {
    use std::io::Read as _;

    let file = goalcup_platform::readcap::open_regular(path).map_err(|error| {
        if error.io_kind() == Some(std::io::ErrorKind::NotFound) {
            McpError::NoEntry
        } else {
            McpError::Io(error.to_string())
        }
    })?;
    // The metadata and bytes come from the same opened object. A path replacement after
    // open cannot swap in a different document, and the `take` still catches growth after
    // this fast sparse-file check.
    if file
        .metadata()
        .map_err(|source| McpError::Io(source.to_string()))?
        .len()
        > MAX_DOCUMENT_BYTES
    {
        return Err(McpError::DocumentTooLarge {
            limit: MAX_DOCUMENT_BYTES,
        });
    }
    let mut text = String::new();
    let read = file
        .take(MAX_DOCUMENT_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|source| McpError::Io(source.to_string()))?;
    if read as u64 > MAX_DOCUMENT_BYTES {
        return Err(McpError::DocumentTooLarge {
            limit: MAX_DOCUMENT_BYTES,
        });
    }
    Ok(text)
}

/// One cached document, parsed and not yet checked.
fn read_document(path: &Path) -> Result<SignedStatus, McpError> {
    let text = read_bounded(path)?;
    serde_json::from_str(&text).map_err(|e| McpError::Json(e.to_string()))
}

// ------------------------------------------------------------------ the tools

/// The three descriptors, in the order `tools/list` answers with.
///
/// `SKILL.md` carries this document byte for byte, and `tests/protocol.rs` compares the
/// two: a tool renamed here and not there would be a skill describing a server that is not
/// running.
pub fn tools_manifest() -> Value {
    json!([
        {
            "name": TOOL_EVENT_STATUS,
            "title": "Round status",
            "description": "The round's timeline as the last signed status document carries \
                            it: t0, when the start window closes, when publication closes, \
                            and the rules version. `source` says where those dates came \
                            from — `config` means an organiser configured them on this \
                            machine, `fallback` means no round was configured and the fields \
                            are this entry's own timeline standing in for one. A `fallback` \
                            answer is not a deadline anybody set. Read-only.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": TOOL_RUN_STATUS,
            "title": "This run's status",
            "description": "How the current run is doing, from the signed status cache: \
                            arming state, health, seconds remaining, when the tracker last \
                            synced, and whether that document is stale (`offline`). \
                            `arming: \"refused\"` means the gate will never arm and waiting \
                            for it is waiting for nothing. Read-only.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entry_id": {
                        "type": "string",
                        "description": "Which run. Defaults to the most recently started one."
                    }
                },
                "additionalProperties": false
            }
        },
        {
            "name": TOOL_ENTRIES,
            "title": "Runs on this machine",
            "description": "Every run this machine holds a verified status document for, \
                            with its arming state, health and time remaining. `unverified` \
                            counts the cached documents that did NOT check out against a \
                            pinned run key and are therefore not in the list. Read-only.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        }
    ])
}

/// The round's dates, and where they came from.
///
/// No entry fact is in here — no id, no workspace, no session, no countdown. The round is
/// not the entry, and a body that mixed the two would invite an agent to read one as the
/// other.
fn event_status_body(src: &StatusSource, now: &str) -> Result<Value, McpError> {
    let signed = src.latest()?;
    let (_, offline, freshness) = freshness(&signed, now);
    let e = &signed.payload.event;
    Ok(json!({
        "t0": e.t0,
        "start_window_closes_at": e.start_window_closes_at,
        "publication_closes_at": e.publication_closes_at,
        "rules_version": e.rules_version,
        "source": wire_word(&signed.payload.event_status_source)?,
        "published_at": signed.cached_at,
        "offline": offline,
        "freshness": freshness,
    }))
}

/// One run, as much of it as an agent can act on.
fn run_status_body(
    src: &StatusSource,
    entry_id: Option<&str>,
    now: &str,
) -> Result<Value, McpError> {
    let signed = match entry_id {
        Some(id) => src.for_entry(id)?,
        None => src.latest()?,
    };
    let (stale, offline, freshness) = freshness(&signed, now);
    let p = &signed.payload;
    if p.capture.is_some() {
        return Ok(m2_status_unavailable(&p.entry_id, &p.connector_id));
    }
    Ok(json!({
        "entry_id": p.entry_id,
        "connector_id": p.connector_id,
        // The refused state reaches an agent as itself: `refused` is not one more pending
        // state, and an agent told a dead gate is pending waits for a thing that will not
        // happen.
        "arming": wire_word(&p.arming)?,
        "health": wire_word(&p.health)?,
        "detail": p.detail,
        "started_at": p.started_at,
        "deadline_at": p.deadline_at,
        "seconds_remaining": p.seconds_remaining,
        "events_uploaded": p.events_uploaded,
        "events_pending": p.events_pending,
        "last_sync_at": p.last_sync_at,
        "published_at": signed.cached_at,
        "stale_seconds": stale,
        "offline": offline,
        "freshness": freshness,
    }))
}

/// Every run this machine can vouch for, and a count of the ones it cannot.
fn entries_body(src: &StatusSource, now: &str) -> Result<Value, McpError> {
    let scan = src.scan()?;
    let rows: Vec<Value> = scan
        .verified
        .iter()
        .map(|signed| -> Result<Value, McpError> {
            let (stale, offline, freshness) = freshness(signed, now);
            let p = &signed.payload;
            if p.capture.is_some() {
                return Ok(m2_status_unavailable(&p.entry_id, &p.connector_id));
            }
            Ok(json!({
                "entry_id": p.entry_id,
                "connector_id": p.connector_id,
                "arming": wire_word(&p.arming)?,
                "health": wire_word(&p.health)?,
                "started_at": p.started_at,
                "deadline_at": p.deadline_at,
                "seconds_remaining": p.seconds_remaining,
                "last_sync_at": p.last_sync_at,
                "published_at": signed.cached_at,
                "stale_seconds": stale,
                "offline": offline,
                "freshness": freshness,
            }))
        })
        .collect::<Result<Vec<Value>, McpError>>()?;
    Ok(json!({
        "entries": rows,
        "count": scan.verified.len(),
        "unverified": scan.refused.len(),
    }))
}

/// The legacy status fields do not describe M2 capture health. Keep capture rows visible
/// while directing the reader to the CLI surface that reports their capture details.
fn m2_status_unavailable(entry_id: &str, connector_id: &str) -> Value {
    json!({
        "entry_id": entry_id,
        "connector_id": connector_id,
        "m2_health": "unavailable",
        "detail": "M2 capture details are unavailable through this MCP tool. Run `goalcup status` for capture health.",
    })
}

/// How old this document is, whether that is past the one threshold every reader of this
/// cache shares, and the sentence that says which of those two it is.
///
/// A `cached_at` that does not parse is `null` and OFFLINE, never fresh: "unknown age" is
/// the one answer that must not read as "published just now".
///
/// A `cached_at` in the FUTURE is that same answer for that same reason.
/// [`SignedStatus::stale_seconds`] subtracts saturatingly, so a document dated after this
/// machine's clock came back as age `0` — the freshest reading there is — when what it
/// actually means is that the daemon's clock and this one disagree and the document's age is
/// therefore not known. The skew is said in words instead of being rounded away into a zero,
/// because an agent that reads `0` plans against a countdown nobody can place in time.
fn freshness(signed: &SignedStatus, now: &str) -> (Value, bool, String) {
    let (published, asked) = match (parse_z(&signed.cached_at), parse_z(now)) {
        (Some(published), Some(asked)) => (published, asked),
        _ => {
            return (
                Value::Null,
                true,
                "published at a time this server could not read".to_string(),
            );
        }
    };
    if published > asked {
        let skew = published - asked;
        return (
            Value::Null,
            true,
            format!("published in the future by {skew}s (clock skew)"),
        );
    }
    let age = asked - published;
    match age > STALE_AFTER_SECONDS {
        true => (json!(age), true, format!("published {age}s ago (stale)")),
        false => (json!(age), false, format!("published {age}s ago")),
    }
}

/// A wire enum in the one word the wire spells it with.
///
/// `serde` and not a match, so this file cannot grow a second name for a state the JSON
/// already has one for, and a variant added to the protocol needs no edit here.
///
/// A value that does not serialise as one string is [`McpError::Internal`] and never the
/// word `"unknown"`. A placeholder would arrive in the same field, in the same shape, as
/// `armed`, `refused` and `online`, and an agent has no way to read it as anything but a
/// state the tracker is in — so a state this server cannot name is reported as a failure to
/// report, which is what it is. Nothing out of the document goes into the message.
fn wire_word<T: serde::Serialize>(value: &T) -> Result<String, McpError> {
    match serde_json::to_value(value) {
        Ok(Value::String(word)) => Ok(word),
        _ => Err(McpError::Internal(
            "a status field this server reports as a single word did not serialise as one"
                .to_string(),
        )),
    }
}

// ------------------------------------------------------------------ the protocol

/// JSON-RPC's four codes, and no fifth.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// One request line in, one response line out.
///
/// `None` for a notification: JSON-RPC says a request with no id gets no response, and that
/// includes the errors it would otherwise have produced. Everything else about this
/// function is pure — the clock and the cache are both arguments — so `main.rs` is a
/// read-a-line, write-a-line loop and the protocol is testable without a process.
pub fn handle_request(line: &str, src: &StatusSource, now: &str) -> Option<String> {
    let request: Value = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(e) => {
            return Some(error_response(
                Value::Null,
                PARSE_ERROR,
                &format!("that line did not read as JSON: {e}"),
            ));
        }
    };
    // A batch — a top-level array — is answered with one error rather than with silence. A
    // client that sent one and got nothing back would wait for a response that is not
    // coming. The refusal names the version so the answer is a statement about the protocol
    // in force rather than about this server's mood: batching left MCP in 2025-06-18, which
    // is the only version `initialize` advertises, so a client that batched was speaking an
    // older protocol than the one it was told it had.
    if !request.is_object() {
        return Some(error_response(
            Value::Null,
            INVALID_REQUEST,
            &format!(
                "this server speaks MCP {NEWEST_PROTOCOL_VERSION}, which has no JSON-RPC \
                 batching: it answers one request object per line, and an array is not a \
                 request object."
            ),
        ));
    }
    // A notification has no id, or a null one.
    let id = match request.get("id") {
        None | Some(Value::Null) => return None,
        Some(id) => id.clone(),
    };
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = request.get("params");
    Some(match method {
        "initialize" => result_response(id, initialize_result(params)),
        "tools/list" => result_response(id, json!({"tools": tools_manifest()})),
        "ping" => result_response(id, json!({})),
        "tools/call" => match call_tool(params, src, now) {
            Ok(result) => result_response(id, result),
            Err((code, said)) => error_response(id, code, &said),
        },
        other => error_response(
            id,
            METHOD_NOT_FOUND,
            &format!(
                "this server has no method `{other}`; it has initialize, tools/list, tools/call and ping."
            ),
        ),
    })
}

/// The answer to a line that never became a request: a `-32700` with a null id, the same
/// response [`handle_request`] gives a line that is not JSON.
///
/// `main.rs` needs it for the two failures that happen BEFORE there is a line to hand over —
/// input longer than [`MAX_DOCUMENT_BYTES`], and input that is not UTF-8. Neither is a
/// reason to end the process: a session that died on one byte would take the run's status
/// with it for the rest of the run.
pub fn unreadable_line_response(said: &str) -> String {
    error_response(Value::Null, PARSE_ERROR, said)
}

/// The handshake. An unsupported version is answered with the newest one this server
/// speaks, which is what the specification's negotiation says to do — an error would leave
/// a client with no server rather than with an older protocol.
fn initialize_result(params: Option<&Value>) -> Value {
    let asked = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let version = match SUPPORTED_PROTOCOL_VERSIONS.contains(&asked) {
        true => asked,
        false => NEWEST_PROTOCOL_VERSION,
    };
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
        "instructions": "Read-only status for a /goal Cup run, from the tracker's signed \
                         cache. Nothing here changes anything, and a document that does not \
                         verify against this machine's pinned run key comes back as an error \
                         rather than as data.",
    })
}

/// `tools/call`: the tool's own refusal is a result with `isError`, and only a malformed
/// call is a JSON-RPC error.
fn call_tool(
    params: Option<&Value>,
    src: &StatusSource,
    now: &str,
) -> Result<Value, (i64, String)> {
    let Some(name) = params.and_then(|p| p.get("name")).and_then(|n| n.as_str()) else {
        return Err((
            INVALID_PARAMS,
            "a tools/call needs a `name`, and this one carried none.".to_string(),
        ));
    };
    let empty = serde_json::Map::new();
    let arguments = match params.and_then(|p| p.get("arguments")) {
        None => &empty,
        Some(Value::Object(arguments)) => arguments,
        Some(_) => {
            return Err((
                INVALID_PARAMS,
                "`arguments` must be an object when it is present.".to_string(),
            ));
        }
    };
    let entry_id = match arguments.get("entry_id") {
        None => None,
        Some(Value::String(id)) => Some(id.clone()),
        Some(_) => {
            return Err((
                INVALID_PARAMS,
                "`entry_id` must be a string when it is present.".to_string(),
            ));
        }
    };
    let body = match name {
        TOOL_EVENT_STATUS | TOOL_ENTRIES if !arguments.is_empty() => {
            return Err((INVALID_PARAMS, format!("`{name}` accepts no arguments.")));
        }
        TOOL_RUN_STATUS if arguments.keys().any(|key| key != "entry_id") => {
            return Err((
                INVALID_PARAMS,
                "`goalcup_run_status` accepts only the optional `entry_id` argument.".to_string(),
            ));
        }
        TOOL_EVENT_STATUS => event_status_body(src, now),
        TOOL_RUN_STATUS => run_status_body(src, entry_id.as_deref(), now),
        TOOL_ENTRIES => entries_body(src, now),
        other => {
            return Err((
                INVALID_PARAMS,
                format!(
                    "this server has no tool `{other}`; it has {TOOL_EVENT_STATUS}, \
                     {TOOL_RUN_STATUS} and {TOOL_ENTRIES}."
                ),
            ));
        }
    };
    Ok(match body {
        Ok(body) => tool_result(body),
        // Everything that could not be vouched for arrives here, and nothing out of the
        // document comes with it.
        Err(refusal) => tool_error(&refusal.to_string()),
    })
}

/// A tool result, in both renderings a client may read: the text a model sees, and the
/// same document as `structuredContent`.
fn tool_result(body: Value) -> Value {
    let text = serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string());
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": body,
        "isError": false,
    })
}

/// A refusal the model is meant to read, rather than a protocol error the client swallows.
fn tool_error(said: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": said}],
        "isError": true,
    })
}

fn result_response(id: Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}
