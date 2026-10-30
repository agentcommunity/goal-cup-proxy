//! Seam U1 (PRD-09): the usage call table, one row per model call, and the `usage` subcommand.
//!
//! [`Calls`] takes every record of an entry in order ([`Calls::push`]) and [`Calls::finish`] gives
//! the [`CallTable`]. The sink reads PART, LINK and the `start` EVENT from the pushed records; it
//! takes no argument of its own. The wire leg is read (`wire.rs`); the native leg is not yet, so
//! the table keeps [`Reason::NoReader`], no threads and children not read.
//!
//! Build a [`CallRow`], [`Thread`] or [`CallTable`] with [`CallRow::new`], [`Thread::new`] or
//! [`CallTable::default`], then set fields (or use struct update from one of them); never with a
//! full struct literal, so a field added later breaks no caller.
//!
//! Wall times are nanoseconds since the Unix epoch, as the LINK `wall_ns` of `goalcup_format`. A
//! token count of `None` means the source did not say; it is never 0 for that (PRD-09 section 4).

mod shapes;
mod sse;
mod wire;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::process::ExitCode;

use goalcup_format::Record;
use serde_json::Value;

/// Which log a row comes from; also which leg a call's tokens came from (`usage_from`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Leg {
    /// The recorded wire: a request and its reply.
    #[default]
    Wire,
    /// The harness's own session log, read back out of the record.
    Native,
}

/// What a row is (PRD-09 section 3). Only `Call` is a model call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Call,
    /// `response.create` with `generate: false`.
    Warmup,
    /// `response.interrupt`.
    Interrupt,
    /// Any other POST to a listed host.
    Other,
}

/// How a call ended, one per call (PRD-09 section 3). `status` is the HTTP status where there is
/// one; `code` is the safe error code token, never text from the run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    RateLimited {
        status: Option<u16>,
        code: Option<String>,
    },
    Error {
        status: Option<u16>,
        code: Option<String>,
    },
    /// The client interrupted it.
    Cancelled,
    /// No terminal event in the record and no named cause.
    #[default]
    Unknown,
}

impl Outcome {
    /// PRD-08's "error": true for `RateLimited` and `Error` only.
    pub fn is_error(&self) -> bool {
        matches!(self, Outcome::RateLimited { .. } | Outcome::Error { .. })
    }
}

/// The join result of one call (PRD-09 sections 5 and 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    /// A wire call and a native call of the same call.
    Matched,
    /// The harness made it; the wire has no `ok` copy.
    NativeOnly,
    /// On the wire with no harness entry (refused retries, side calls, auxiliary threads).
    WireOnly,
    /// An `ok` wire call of a harness with a wire thread hint whose request carries none.
    Unattributed,
}

/// The host's kind: the `conn` meta `kind`, else the `start` event's `hosts` (PRD-09 section 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKind {
    Provider,
    Gateway,
    Vendor,
    /// Participant and config hosts.
    Other,
}

/// The wire shape the request path picks (PRD-09 sections 3 and 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Responses,
    Anthropic,
    Chat,
    Gemini,
}

/// How a wire call travelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Http,
    Ws,
}

/// What a connector's native leg can give (PRD-08 section 3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeForm {
    /// Per-call rows with times.
    PerCall,
    /// Per-session totals only.
    Totals,
    /// No native leg.
    None,
}

/// Why a native leg is not readable (PRD-09 section 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// No reader for this connector's session format.
    NoReader,
    /// No database copy opened, even after the WAL repair.
    SnapshotUnreadable,
    /// A reader found zero native calls while the wire had calls.
    NoNative,
}

/// The five token numbers of one call. `None` means not said, never 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    /// Input tokens not read from a cache.
    pub input: Option<u64>,
    /// Input tokens read from a cache.
    pub cached: Option<u64>,
    /// Input tokens written to a cache.
    pub cache_write: Option<u64>,
    /// Output tokens, reasoning included.
    pub output: Option<u64>,
    /// The reasoning part of `output`.
    pub reasoning: Option<u64>,
}

/// One row: a wire call, warm-up, interrupt or other request, or a native call. The wire fields
/// (`sid`, `msg`, `host`, `host_kind`, `shape`, `via`) are `None` on native rows, `path` on wire
/// rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallRow {
    pub leg: Leg,
    /// The entry id, `e` and 16 hex.
    pub entry: String,
    pub boot: u64,
    /// The request stream id.
    pub sid: Option<u64>,
    /// The session file path.
    pub path: Option<String>,
    /// The WebSocket message number on its stream.
    pub msg: Option<u64>,
    pub thread: Option<String>,
    pub parent: Option<String>,
    pub wall_start: Option<u64>,
    /// For a wire call, the reply's end.
    pub wall_end: Option<u64>,
    pub host: Option<String>,
    pub host_kind: Option<HostKind>,
    pub shape: Option<Shape>,
    pub via: Option<Via>,
    pub kind: Kind,
    pub outcome: Outcome,
    /// A safe reason token beside `outcome`, such as a `response.incomplete` reason.
    pub detail: Option<String>,
    /// The request, the reply and, for WebSocket, the message ended `complete` with no GAP.
    pub whole: bool,
    /// The model the reply named, else the requested one.
    pub model: Option<String>,
    /// The request body's `model`.
    pub model_requested: Option<String>,
    pub effort: Option<String>,
    pub tokens: Tokens,
    /// Other numeric usage members whose name ends in `tokens`, `Tokens` or `TokenCount`.
    pub extra: BTreeMap<String, u64>,
    /// Which leg `tokens` came from; never both added.
    pub usage_from: Option<Leg>,
    pub join: Option<Join>,
}

impl CallRow {
    /// A row of boot `boot` with every optional field empty, `outcome` `Unknown` and `whole`
    /// false. Boots count from 0, so `boot` is always given, never assumed.
    pub fn new(leg: Leg, entry: impl Into<String>, boot: u64, kind: Kind) -> Self {
        Self {
            leg,
            entry: entry.into(),
            boot,
            kind,
            ..Self::default()
        }
    }
}

/// One agent or subagent thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub id: String,
    pub parent: Option<String>,
    /// The harness's own agent label; bounded, printable.
    pub label: Option<String>,
    pub depth: Option<u32>,
    /// Native calls read for this thread.
    pub native_calls: u64,
    /// The harness's own call total where it keeps one.
    pub native_total: Option<u64>,
    /// Native calls with no `ok` wire copy.
    pub native_only: u64,
    /// Native calls after this time (the database copy's time) are not known.
    pub native_until: Option<u64>,
    /// Why the native leg is not readable; `None` means readable.
    pub reason: Option<Reason>,
}

impl Thread {
    /// True when the native leg was read.
    pub fn readable(&self) -> bool {
        self.reason.is_none()
    }

    /// A thread with nothing read: not readable, [`Reason::NoReader`].
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            parent: None,
            label: None,
            depth: None,
            native_calls: 0,
            native_total: None,
            native_only: 0,
            native_until: None,
            reason: Some(Reason::NoReader),
        }
    }
}

/// The call table of one entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallTable {
    pub rows: Vec<CallRow>,
    /// The threads read.
    pub threads: Vec<Thread>,
    /// Parent thread id to the child threads it names; `None` when no source of parentage was
    /// read. Whether it was read does not follow from `readable` or `reason`: a path-only harness
    /// has `no_reader` yet gives who spawned whom, and an unreadable database gives nothing. A
    /// check that needs it (`subagents_present`) skips on `None`, never passes, and runs on
    /// whatever was read otherwise.
    pub children: Option<BTreeMap<String, Vec<String>>>,
    /// Why the native leg is not readable; `None` means readable.
    pub reason: Option<Reason>,
    pub native_form: NativeForm,
    /// Calls that fell back to the time join.
    pub joined_by_time: u64,
}

impl Default for CallTable {
    /// The empty table of an entry with no reader: not readable, [`Reason::NoReader`], children
    /// not read.
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            threads: Vec::new(),
            children: None,
            reason: Some(Reason::NoReader),
            native_form: NativeForm::None,
            joined_by_time: 0,
        }
    }
}

impl CallTable {
    /// True when the native leg was read.
    pub fn readable(&self) -> bool {
        self.reason.is_none()
    }
}

/// A host kind by its record name.
fn host_kind(s: Option<&str>) -> Option<HostKind> {
    match s? {
        "provider" => Some(HostKind::Provider),
        "gateway" => Some(HostKind::Gateway),
        "vendor" => Some(HostKind::Vendor),
        "other" => Some(HostKind::Other),
        _ => None,
    }
}

/// What the sink knows of the boot being read: entry, boot, wall base and the `start` hosts.
#[derive(Debug, Default)]
struct Ctx {
    entry: String,
    boot: u64,
    /// (wall_ns, mono_ns) of the last PART or LINK of this boot.
    base: Option<(u64, u64)>,
    /// The `start` event's hosts and kinds, wildcards as given.
    hosts: Vec<(String, Option<HostKind>)>,
}

impl Ctx {
    /// A record time of this boot as wall ns, by the nearest PART or LINK before it.
    fn wall(&self, t: u64) -> Option<u64> {
        let (w, m) = self.base?;
        u64::try_from(i128::from(w) + i128::from(t) - i128::from(m)).ok()
    }

    /// The kind the `start` hosts give `host` (`*.x` matches by suffix); `None` when not listed.
    fn host_kind(&self, host: &str) -> Option<Option<HostKind>> {
        let h = host.to_ascii_lowercase();
        let hit = |p: &str| match p.strip_prefix('*') {
            Some(end) => end.starts_with('.') && h.ends_with(end),
            None => p == h,
        };
        self.hosts.iter().find(|(p, _)| hit(p)).map(|(_, k)| *k)
    }

    fn start(&mut self, ev: &Value) {
        let lists = ev.get("hosts").and_then(Value::as_object).into_iter().flatten();
        self.hosts = lists
            .filter_map(|(_, l)| l.as_array())
            .flatten()
            .filter_map(|e| {
                let host = e.get("host")?.as_str()?.to_ascii_lowercase();
                Some((host, host_kind(e.get("kind").and_then(Value::as_str))))
            })
            .collect();
    }
}

/// The U1 sink: push every record of an entry in order, then finish.
#[derive(Debug, Default)]
pub struct Calls {
    ctx: Ctx,
    wire: wire::Wire,
}

impl Calls {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes one record.
    pub fn push(&mut self, rec: &Record) {
        match rec {
            Record::Part {
                entry,
                boot,
                wall_ns,
                mono_ns,
                ..
            } => {
                (self.ctx.entry, self.ctx.boot) = (entry.clone(), *boot);
                self.ctx.base = Some((*wall_ns, *mono_ns));
            }
            Record::Link {
                wall_ns, mono_ns, ..
            } => self.ctx.base = Some((*wall_ns, *mono_ns)),
            Record::Event { json, .. } => {
                let ev = serde_json::from_slice::<Value>(json).unwrap_or(Value::Null);
                if ev.get("ev").and_then(Value::as_str) == Some("start") {
                    self.ctx.start(&ev);
                }
            }
            _ => self.wire.push(&self.ctx, rec),
        }
    }

    /// The table: wire rows in (boot, sid, msg) order; the native leg is not read yet.
    pub fn finish(self) -> CallTable {
        CallTable {
            rows: self.wire.finish(),
            ..CallTable::default()
        }
    }
}

/// Runs `goalcup-verify usage <args>` (PRD-09 section 7). Not built yet: says so and exits 3 (no
/// usable report, PRD-08 exit codes).
pub fn run(args: Vec<OsString>) -> ExitCode {
    let _ = args;
    eprintln!("goalcup-verify: the usage reader is not built yet");
    ExitCode::from(3)
}
