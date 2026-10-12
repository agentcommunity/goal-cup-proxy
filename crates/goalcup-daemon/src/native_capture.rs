//! Bounded, scoped native transcript byte capture.
//!
//! A harness writes its own session store — Codex's rollout JSONL under `CODEX_HOME/sessions`,
//! Claude Code's project JSONL under `CLAUDE_CONFIG_DIR/projects` — and this adapter retains
//! those bytes as [`CaptureFrame`]s submitted to a caller's [`CaptureSink`]. It is a byte
//! source and nothing else: no line is parsed, no record is produced, no partial line is held
//! back or thrown away, and no semantic session identity is extracted. The record-oriented
//! V1 tailers it once sat beside were removed by D48.
//!
//! # Scope
//!
//! Each [`NativeSourceSpec`] names an already-isolated absolute `root` and a relative
//! `pattern` below it. The pattern **is** the positive scope: the adapter never widens it,
//! never falls back to a broader directory, and has no recursive wildcard. A shared store
//! (both isolated harness homes are shared across runs) must be narrowed by the caller
//! to the run's own session file or directory. Every path component is reached through the
//! retained root capability ([`goalcup_platform::sourcecap`]): no symlink is followed at any
//! level, a FIFO or other special file is never opened, a mount crossing is refused, and a
//! changed or vanished path is a changed source, not a read. Labels are
//! `<source>:<relative path>#g<generation>` and never contain the root. Identical
//! root-and-pattern pairs are refused; keeping differently spelled patterns disjoint is the
//! caller's job, since two sources matching one file capture it twice under two labels.
//!
//! A harness-supplied session or thread ID never goes into a pattern as text. The caller
//! passes it as [`NativeSourceSpec::scope_id`] and writes `{id}` where it belongs; the adapter
//! refuses any ID that is not 1-128 ASCII letters, digits, `-` or `_` before substituting it,
//! so an ID can never add a wildcard, a separator or a relative component to the scope.
//!
//! # Producer, admission and ordering
//!
//! The adapter is one capture producer. It mints a fresh 128-bit producer ID, asks
//! [`CaptureSink::try_begin_epoch`] on each poll until the answer is `Ready`, and submits no
//! frame before that. `Pending` keeps the ID; `RetryFresh` retires it for a fresh one (bounded
//! by `MAX_PRODUCERS`); `Disabled` latches the loss. `seq` starts at zero and increases across
//! every frame of every file; payload streams start at one; stream zero is never used, because
//! a native producer emits no `Connection` or `Recovery` control. Each observation carries
//! `clock_id = producer` and the injected [`NativeClock`]'s wall and monotonic readings; no
//! timestamp is ever taken from file content.
//!
//! # Streams and generations
//!
//! Every file matched by a pattern gets one stream per *generation*. An append-only file
//! ([`NativeStoreForm::AppendOnly`]) keeps its stream open across polls: EOF of a live file is
//! not completion, and a quiet poll emits nothing. A snapshot file ([`NativeStoreForm::Snapshot`])
//! is read to the length observed when its stream opened and then ends `Complete`; a
//! completed snapshot whose facts later differ is read again as a new generation.
//! Replacement (a different identity), truncation (shorter than the reader position), a
//! vanished path, a path that became a non-regular file, or a snapshot that changed while it
//! was read ends the stream with `Gap { SourceChanged, lost_bytes: None }` then
//! `End { Interrupted }`, strictly before the next generation's `Open`. Offsets are absolute
//! positions in the observed file, so a new generation never reuses an old stream's offsets
//! and re-captured bytes are visible as re-captured. Truncate-then-regrow past the old reader
//! position between two polls keeps the inode and a larger length, and is not detectable here.
//!
//! # Bounds
//!
//! One reusable buffer of `MAX_SEGMENT_BYTES` (the source-chunk bound) is the only copy of
//! file bytes the adapter holds; every `Bytes` frame is at most that long. A poll reads at
//! most [`NativeCaptureLimits::poll_bytes`], shared across all files in round-robin order,
//! and reports [`NativePollReport::more_work`] when the byte budget deferred bytes it saw.
//! Discovery examines at most [`NativeCaptureLimits::max_dir_entries`] directory entries per
//! poll, walking directory levels in descending name order so a date-partitioned store
//! (`YYYY/MM/DD`) reaches its newest day before older history; it tracks at most
//! [`NativeCaptureLimits::max_files`] files and keeps at most
//! [`NativeCaptureLimits::max_open_streams`] streams open. Descriptors are held only inside a
//! poll, apart from one retained root per source: one directory chain of at most
//! [`MAX_NATIVE_PATTERN_DEPTH`] handles and one file, within the 32-handle
//! [`NativeHandleBudgetV1`]. Health counters are fixed-size.
//!
//! # Loss and durability
//!
//! `Queued` is volatile acceptance: it advances only the in-memory reader position. The
//! adapter makes no durability claim and persists nothing; durable coordinates belong to the
//! entry owner's appended `CaptureRecord`s and synced frontier. A `Disabled` answer latches
//! the loss for this producer: no further frame is submitted, the rejected frame is never
//! retried, polls stop doing I/O, and no `Close` is ever sent, so the receiver records unknown
//! tail loss. There is no wire-style recovery. A loss stays latched until an explicit resume
//! builds a fresh adapter ([`NativeCapture::resume`]) with a fresh producer, which opens each
//! resumed file as the next generation with an unknown-loss `Gap` before any byte.
//!
//! Losses that have no stream to carry a `Gap` frame — discovery stopped at its entry cap,
//! an entry refused or unavailable, a file over `max_files`, a file that never got a stream —
//! are kept for the adapter's lifetime, every poll and finish pass included, and reported in
//! [`NativeLossReport`] and [`NativeFinishReport`]. The entry owner records a stream-less
//! `CaptureRecord::Gap` for them. A finish report with none of them still does not prove the
//! expected file existed: the owner checks it saw the `Open` it expected.
//!
//! # Integration interface
//!
//! - `NativeCapture::new(config: NativeCaptureConfig, sink: Arc<dyn CaptureSink>,
//!   clock: Box<dyn NativeClock>) -> Result<NativeCapture, NativeCaptureError>`
//! - `NativeCapture::resume(config, sink, clock, points: Vec<NativeResumePoint>)
//!   -> Result<NativeCapture, NativeCaptureError>` — explicit resume only; never automatic.
//! - `NativeCapture::poll(&mut self) -> NativePollReport` — one bounded cadence step.
//! - `NativeCapture::finish(&mut self, budget: NativeFinishBudget) -> NativeFinishReport` —
//!   once-only drain, stream termination and ordered producer close.
//! - `NativeCapture::loss_report(&self) -> NativeLossReport` and `producer(&self) -> &str`.
//!
//! The caller owns the sink (and, through it, redaction and persistence), the cadence and
//! the thread: the adapter is `Send` and synchronous, and every sink call it makes is the
//! bounded, non-blocking sink call. Native hints are validated open-vocabulary strings copied
//! from schema-1 `protocol_hint`; SQLite stores are an explicit
//! [`NativeCaptureError::UnsupportedStore`].
//!
//! ```no_run
//! use goalcup_daemon::native_capture::{
//!     NativeCapture, NativeCaptureConfig, NativeCaptureLimits, NativeFinishBudget,
//!     NativeHint, NativeSourceSpec, NativeStoreForm, SystemNativeClock,
//! };
//! use goalcup_protocol::capture::CaptureSink;
//! use std::sync::Arc;
//!
//! fn wire(sink: Arc<dyn CaptureSink>, codex_home: std::path::PathBuf, thread_id: &str) {
//!     let config = NativeCaptureConfig {
//!         sources: vec![NativeSourceSpec {
//!             name: "codex-rollout".into(),
//!             root: codex_home.join("sessions"),
//!             pattern: "*/*/*/rollout-*-{id}.jsonl".into(),
//!             form: NativeStoreForm::AppendOnly,
//!             hint: Some(NativeHint::new("codex-jsonl").unwrap()),
//!             // Validated, then substituted for `{id}`: `*`, `?` or `/` is refused.
//!             scope_id: Some(thread_id.to_string()),
//!         }],
//!         limits: NativeCaptureLimits::default(),
//!     };
//!     let mut native = NativeCapture::new(config, sink, Box::new(SystemNativeClock::new()))
//!         .expect("scoped native source");
//!     // On the entry's cadence thread, until the capture-drain boundary:
//!     let report = native.poll();
//!     # let _ = report;
//!     let finished = native.finish(NativeFinishBudget {
//!         max_bytes: 64 << 20,
//!         max_duration_ns: 10_000_000_000,
//!     });
//!     # let _ = finished;
//! }
//! ```

use goalcup_platform::handle_budget::NativeHandleBudgetV1;
use goalcup_platform::sourcecap::{
    RefusalKind, SourceCapError, SourceDir, SourceFactsV1, SourceFile, SourceKind, SourceLookup,
    SourceName, SourceObservation,
};
use goalcup_protocol::capture::{
    CaptureFrame, CaptureMeta, CaptureSink, CaptureSource, CaptureTime, EndReason, EpochAdmission,
    FrameEvent, GapReason, MAX_CAPTURE_QUEUE_BYTES, MAX_METADATA_STRING_BYTES, MAX_OPEN_STREAMS,
    MAX_PRODUCERS, MAX_SEGMENT_BYTES, StreamKey, SubmitResult,
};
use goalcup_protocol::hints::valid_hint;
use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::io::{Read, Seek, SeekFrom};
use std::ops::Bound;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;

/// Most native sources one adapter reads.
pub const MAX_NATIVE_SOURCES: usize = 8;
/// Most components in one source pattern.
pub const MAX_NATIVE_PATTERN_DEPTH: usize = 8;
/// Hard ceiling on [`NativeCaptureLimits::max_files`].
pub const MAX_NATIVE_FILES: usize = 20_000;
/// Hard ceiling on [`NativeCaptureLimits::max_dir_entries`].
pub const MAX_NATIVE_DIR_ENTRIES: usize = 65_536;

const MAX_SOURCE_NAME_BYTES: usize = 64;
const MAX_PATTERN_BYTES: usize = 1024;
/// Observe-then-open attempts before a racing writer defers a file to the next poll.
const OPEN_ATTEMPTS: usize = 3;
/// Room a label keeps for its `#g<generation>` suffix: `#g` and `u64::MAX`'s 20 digits.
const GENERATION_SUFFIX_BYTES: usize = 22;
/// The one placeholder a pattern may carry, replaced by the validated scope ID.
const SCOPE_ID_PLACEHOLDER: &str = "{id}";
const MAX_SCOPE_ID_BYTES: usize = 128;

/// A validated native protocol hint. It is open vocabulary and never inferred from connector
/// identity, form, path, or filename.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeHint(String);

impl NativeHint {
    pub fn new(hint: impl Into<String>) -> Result<Self, NativeCaptureError> {
        let hint = hint.into();
        if !valid_hint(&hint) {
            return Err(config(
                "a native protocol hint must match the protocol hint grammar",
            ));
        }
        Ok(Self(hint))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How a store's files grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeStoreForm {
    /// Appended in place (a JSONL transcript): one open stream per generation until finish.
    AppendOnly,
    /// Written whole (a spilled tool result): each generation ends `Complete` once read.
    Snapshot,
}

/// One positively scoped native store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeSourceSpec {
    /// Display name, 1-64 bytes of ASCII letters, digits, `-`, `_` or `.` (for example the
    /// manifest store name). The first part of every label.
    pub name: String,
    /// The already-isolated, absolute root directory. Opened once at construction and
    /// retained; it never appears in a label or an error.
    pub root: PathBuf,
    /// `/`-separated relative pattern under `root`. Each component is a literal name or a
    /// single-component glob using `*` and `?`; `**`, `.`, `..`, empty components and
    /// absolute patterns are refused. `{id}` marks where [`Self::scope_id`] goes; no other
    /// brace is allowed.
    pub pattern: String,
    pub form: NativeStoreForm,
    /// Protocol hint, append-only JSONL stores only.
    pub hint: Option<NativeHint>,
    /// The run's own session or thread ID (for example Codex's `thread.started` ID or the
    /// Claude session UUID), substituted for every `{id}` in `pattern` only after it is
    /// validated as 1-128 ASCII letters, digits, `-` or `_`. Required exactly when the
    /// pattern contains `{id}`.
    pub scope_id: Option<String>,
}

/// Per-adapter bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeCaptureLimits {
    /// Bytes read per poll, shared across files; 1..=`MAX_CAPTURE_QUEUE_BYTES`.
    pub poll_bytes: u64,
    /// Simultaneously open native streams; 1..=`MAX_OPEN_STREAMS`.
    pub max_open_streams: usize,
    /// Files tracked (open, completed or waiting); 1..=[`MAX_NATIVE_FILES`].
    pub max_files: usize,
    /// Directory entries examined by discovery per poll; 1..=[`MAX_NATIVE_DIR_ENTRIES`].
    pub max_dir_entries: usize,
}

impl Default for NativeCaptureLimits {
    /// 16 source chunks per poll (1 MiB, an eighth of the capture queue) and a quarter of the
    /// entry's open-stream allowance, leaving the rest to wire capture.
    fn default() -> Self {
        NativeCaptureLimits {
            poll_bytes: 16 * MAX_SEGMENT_BYTES as u64,
            max_open_streams: MAX_OPEN_STREAMS / 4,
            max_files: 1024,
            max_dir_entries: 4096,
        }
    }
}

impl NativeCaptureLimits {
    fn validate(&self) -> Result<(), NativeCaptureError> {
        if self.poll_bytes == 0 || self.poll_bytes > MAX_CAPTURE_QUEUE_BYTES as u64 {
            return Err(config(
                "poll_bytes must be between 1 and MAX_CAPTURE_QUEUE_BYTES",
            ));
        }
        if self.max_open_streams == 0 || self.max_open_streams > MAX_OPEN_STREAMS {
            return Err(config(
                "max_open_streams must be between 1 and MAX_OPEN_STREAMS",
            ));
        }
        if self.max_files == 0 || self.max_files > MAX_NATIVE_FILES {
            return Err(config("max_files must be between 1 and MAX_NATIVE_FILES"));
        }
        if self.max_dir_entries == 0 || self.max_dir_entries > MAX_NATIVE_DIR_ENTRIES {
            return Err(config(
                "max_dir_entries must be between 1 and MAX_NATIVE_DIR_ENTRIES",
            ));
        }
        Ok(())
    }
}

/// Everything a constructor needs besides the sink and the clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCaptureConfig {
    pub sources: Vec<NativeSourceSpec>,
    pub limits: NativeCaptureLimits,
}

/// The explicit drain budget of [`NativeCapture::finish`]. At least one bounded pass runs;
/// further passes stop at whichever bound is reached first. Time is measured on the
/// adapter's [`NativeClock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeFinishBudget {
    pub max_bytes: u64,
    pub max_duration_ns: u64,
}

/// One observation instant from a [`NativeClock`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockReading {
    /// RFC 3339 UTC, ending in `Z`.
    pub wall: String,
    /// Nanoseconds since this clock's own fixed origin; never decreasing.
    pub mono_ns: u64,
}

/// The adapter's observation clock. The adapter stamps `clock_id` itself (its producer ID),
/// so a clock supplies only wall and monotonic readings.
pub trait NativeClock: Send {
    fn now(&mut self) -> ClockReading;
}

/// The process clock: `Instant` since construction and the daemon's RFC 3339 wall time.
#[derive(Debug, Clone, Copy)]
pub struct SystemNativeClock {
    origin: std::time::Instant,
}

impl SystemNativeClock {
    pub fn new() -> Self {
        SystemNativeClock {
            origin: std::time::Instant::now(),
        }
    }
}

impl Default for SystemNativeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeClock for SystemNativeClock {
    fn now(&mut self) -> ClockReading {
        ClockReading {
            wall: crate::now_rfc3339(),
            mono_ns: u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX),
        }
    }
}

/// A durable coordinate from the entry owner, for an explicit resume only.
///
/// The owner derives it from records it appended and synced — the native `Open` label
/// ([`NativeLabel::parse`]) and the end of that stream's last retained segment — never from
/// anything this adapter reported as queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeResumePoint {
    /// The [`NativeSourceSpec::name`] the label names.
    pub source: String,
    /// The label's relative path, exactly as rendered.
    pub relative_path: String,
    /// The highest generation any earlier producer opened for this path (at least one).
    pub generation: u64,
    /// End offset of the last durably retained byte range of that generation, or `None`
    /// when no range is known durable. The resumed generation starts here when the file is
    /// at least this long, and at zero otherwise; either way after an unknown-loss `Gap`.
    pub durable_end: Option<u64>,
}

/// A native stream label: `<source>:<relative path>#g<generation>`.
///
/// Display text for people and for the explicit-resume coordinate above; it is not a join
/// key between streams. Path bytes that are not printable UTF-8 are escaped (`\\`, `\u{..}`,
/// `\xNN`), so distinct names render distinctly and no control byte reaches metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLabel {
    pub source: String,
    pub relative_path: String,
    pub generation: u64,
}

impl NativeLabel {
    pub fn parse(label: &str) -> Option<NativeLabel> {
        let (source, rest) = label.split_once(':')?;
        let (relative_path, generation) = rest.rsplit_once("#g")?;
        if !valid_source_name(source)
            || relative_path.is_empty()
            || generation.is_empty()
            || generation.starts_with('0')
            || !generation.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        Some(NativeLabel {
            source: source.to_string(),
            relative_path: relative_path.to_string(),
            generation: generation.parse().ok()?,
        })
    }
}

impl fmt::Display for NativeLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}#g{}",
            self.source, self.relative_path, self.generation
        )
    }
}

/// The producer's admission state as a caller sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAdmission {
    /// Not yet admitted `Ready`; no frame has been submitted.
    Pending,
    Ready,
    /// Loss latched: no further frame, no close.
    Lost(GapReason),
    /// Finished and closed cleanly.
    Closed,
}

/// What one poll did. Every counter is this poll's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePollReport {
    pub admission: NativeAdmission,
    /// Frames the sink queued.
    pub frames: u64,
    /// Source bytes the sink queued.
    pub bytes: u64,
    /// The byte budget deferred bytes this poll observed; poll again soon.
    pub more_work: bool,
    /// Discovery stopped at `max_dir_entries`; some scoped entries were not examined.
    pub discovery_capped: bool,
    /// Matched files not tracked because `max_files` was reached (not captured).
    pub untracked_files: u64,
    /// Tracked files waiting for an open-stream slot.
    pub waiting_files: u64,
    /// Matched entries refused: symlinks, special files, directories where a file was
    /// expected, or a path too long for a label. Never opened, never followed.
    pub refused_entries: u64,
    /// Entries that could not be observed, opened or read this poll; retried next poll.
    pub unavailable: u64,
}

impl NativePollReport {
    fn new(admission: NativeAdmission) -> Self {
        NativePollReport {
            admission,
            frames: 0,
            bytes: 0,
            more_work: false,
            discovery_capped: false,
            untracked_files: 0,
            waiting_files: 0,
            refused_entries: 0,
            unavailable: 0,
        }
    }
}

/// Sticky, fixed-size facts about this producer's losses and bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLossReport {
    pub producer: String,
    pub admission: NativeAdmission,
    /// The producer-wide loss, if latched.
    pub latched: Option<GapReason>,
    /// The last sequence number the sink queued: a volatile position, never a durable one.
    pub last_queued_seq: Option<u64>,
    /// Replacements, truncations, vanished paths and changed snapshots observed.
    pub source_changes: u64,
    /// Streams ended `Interrupted` (source changes and the finish deadline).
    pub interrupted_streams: u64,
    pub open_streams: usize,
    pub tracked_files: usize,
    /// Some matched file was never tracked because `max_files` was reached.
    pub file_limit_reached: bool,
    /// Lifetime discovery facts; see [`NativeFinishReport::discovery_capped`].
    pub discovery_capped: bool,
    pub refused_entries: u64,
    pub unavailable: u64,
    pub untracked_files: u64,
}

/// The committed result of [`NativeCapture::finish`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NativeFinishReport {
    /// The producer was admitted `Ready`.
    pub admitted: bool,
    /// The ordered producer close was queued.
    pub closed: bool,
    /// The last queued sequence number (the close's `final_seq`).
    pub final_seq: Option<u64>,
    /// Streams drained to their observed length and ended `Complete`, a snapshot that a pass
    /// had already completed included.
    pub completed_streams: u64,
    /// Streams ended `Interrupted` after `Gap { ShutdownDeadline, lost_bytes: None }`.
    pub interrupted_streams: u64,
    /// Tracked files that never got a stream (waiting for a slot).
    pub uncaptured_files: u64,
    pub bytes_drained: u64,
    /// The producer-wide loss, if latched.
    pub latched: Option<GapReason>,
    /// Lifetime discovery facts, every poll and finish pass included. Discovery stopped at
    /// `max_dir_entries` at least once, so some scoped entries went unexamined.
    pub discovery_capped: bool,
    /// Matched entries refused (symlink, special file, directory, over-long label), summed
    /// over passes: an entry seen in several passes counts in each.
    pub refused_entries: u64,
    /// Entries that could not be observed, opened or read, summed over passes.
    pub unavailable: u64,
    /// Matched files never tracked because `max_files` was reached, summed over passes.
    pub untracked_files: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum NativeCaptureError {
    #[error("native byte capture does not support SQLite stores")]
    UnsupportedStore,
    #[error("invalid native capture configuration: {0}")]
    Config(&'static str),
    #[error("a native source root could not be opened: {0}")]
    Root(#[source] SourceCapError),
}

fn config(reason: &'static str) -> NativeCaptureError {
    NativeCaptureError::Config(reason)
}

/// The adapter. See the module documentation for its contract.
pub struct NativeCapture {
    sources: Vec<Source>,
    limits: NativeCaptureLimits,
    emitter: Emitter,
    files: BTreeMap<FileKey, Tracked>,
    resume: BTreeMap<FileKey, ResumePlan>,
    rotate_after: Option<FileKey>,
    buffer: Vec<u8>,
    source_changes: u64,
    interrupted_streams: u64,
    file_limit_reached: bool,
    facts: DiscoveryFacts,
    finished: Option<NativeFinishReport>,
}

impl fmt::Debug for NativeCapture {
    /// Counters only: no path, label or byte.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeCapture")
            .field("producer", &self.emitter.producer)
            .field("admission", &self.emitter.admission)
            .field("next_seq", &self.emitter.next_seq)
            .field("open_streams", &self.emitter.open_streams)
            .field("tracked_files", &self.files.len())
            .field("finished", &self.finished.is_some())
            .finish_non_exhaustive()
    }
}

impl NativeCapture {
    /// A fresh producer over freshly scoped sources. Opens each root; performs no read and
    /// no sink call.
    pub fn new(
        config: NativeCaptureConfig,
        sink: Arc<dyn CaptureSink>,
        clock: Box<dyn NativeClock>,
    ) -> Result<NativeCapture, NativeCaptureError> {
        Self::build(config, sink, clock, Vec::new())
    }

    /// An explicit resume after a daemon restart: a fresh producer whose files named by
    /// `points` open as `generation + 1` with an unknown-loss `Gap` before any byte, starting
    /// at the durable end. Unnamed files start at generation one, offset zero. Nothing is
    /// continued automatically and no earlier stream is reused.
    pub fn resume(
        config: NativeCaptureConfig,
        sink: Arc<dyn CaptureSink>,
        clock: Box<dyn NativeClock>,
        points: Vec<NativeResumePoint>,
    ) -> Result<NativeCapture, NativeCaptureError> {
        Self::build(config, sink, clock, points)
    }

    fn build(
        config: NativeCaptureConfig,
        sink: Arc<dyn CaptureSink>,
        clock: Box<dyn NativeClock>,
        points: Vec<NativeResumePoint>,
    ) -> Result<NativeCapture, NativeCaptureError> {
        let NativeCaptureConfig { sources, limits } = config;
        limits.validate()?;
        if sources.is_empty() || sources.len() > MAX_NATIVE_SOURCES {
            return Err(self::config(
                "between one and eight native sources are required",
            ));
        }
        let budget = NativeHandleBudgetV1::new();
        let mut built: Vec<Source> = Vec::with_capacity(sources.len());
        let mut scopes: Vec<(PathBuf, String)> = Vec::with_capacity(sources.len());
        for spec in sources {
            if !valid_source_name(&spec.name) {
                return Err(self::config(
                    "a source name is 1-64 ASCII letters, digits, '-', '_' or '.'",
                ));
            }
            if built.iter().any(|source| source.name == spec.name) {
                return Err(self::config("source names must be unique"));
            }
            if spec.hint.is_some() && spec.form != NativeStoreForm::AppendOnly {
                return Err(self::config(
                    "a native hint applies only to an append-only JSONL store",
                ));
            }
            if !spec.root.is_absolute() {
                return Err(self::config("a source root must be an absolute path"));
            }
            let expanded = expand_pattern(&spec.pattern, spec.scope_id.as_deref())?;
            let pattern = parse_pattern(&expanded)?;
            let scope = (spec.root.clone(), expanded);
            if scopes.contains(&scope) {
                return Err(self::config(
                    "two sources declare the same root and pattern",
                ));
            }
            let root =
                SourceDir::open_selected(&spec.root, &budget).map_err(NativeCaptureError::Root)?;
            scopes.push(scope);
            built.push(Source {
                name: spec.name,
                root,
                pattern,
                form: spec.form,
                hint: spec.hint,
            });
        }

        if points.len() > limits.max_files {
            return Err(self::config("more resume points than max_files"));
        }
        let mut resume = BTreeMap::new();
        for point in points {
            let Some(index) = built.iter().position(|source| source.name == point.source) else {
                return Err(self::config("a resume point names an unknown source"));
            };
            if point.relative_path.is_empty()
                || point.relative_path.len() > MAX_METADATA_STRING_BYTES
            {
                return Err(self::config("a resume point needs a bounded relative path"));
            }
            if point.generation == 0 {
                return Err(self::config(
                    "a resume point names a generation of at least one",
                ));
            }
            let plan = ResumePlan {
                generation: point.generation,
                start: ResumeStart {
                    durable_end: point.durable_end,
                },
            };
            if resume.insert((index, point.relative_path), plan).is_some() {
                return Err(self::config("two resume points name the same file"));
            }
        }

        Ok(NativeCapture {
            sources: built,
            limits,
            emitter: Emitter {
                sink,
                clock,
                producer: fresh_producer(),
                consumed: 1,
                admission: Admission::Unadmitted,
                next_seq: 0,
                last_seq: None,
                next_stream: 1,
                open_streams: 0,
            },
            files: BTreeMap::new(),
            resume,
            rotate_after: None,
            buffer: vec![0; MAX_SEGMENT_BYTES],
            source_changes: 0,
            interrupted_streams: 0,
            file_limit_reached: false,
            facts: DiscoveryFacts::default(),
            finished: None,
        })
    }

    /// The current producer ID (it changes only on `RetryFresh`, before any frame).
    pub fn producer(&self) -> &str {
        &self.emitter.producer
    }

    /// One bounded step: admission, discovery, then reading within `poll_bytes`. After a
    /// latched loss or finish it does no I/O and calls no sink method.
    pub fn poll(&mut self) -> NativePollReport {
        let mut report = NativePollReport::new(self.admission());
        if self.finished.is_none() && self.emitter.admit() {
            self.pass(self.limits.poll_bytes, &mut report);
        }
        report.admission = self.admission();
        report
    }

    /// Drain within `budget`, end every open stream, then close the producer in order.
    ///
    /// A stream read to the length its last observation reported ends `Complete`: every byte
    /// through that finish-boundary observation was emitted, including a partial terminal
    /// line. It does not claim the harness stopped writing. Any other open stream receives
    /// `Gap { ShutdownDeadline, lost_bytes: None }` and `End { Interrupted }`. The close is
    /// sent only after every stream ended; a latched loss sends none. Once-only: later calls
    /// return the committed result.
    pub fn finish(&mut self, budget: NativeFinishBudget) -> NativeFinishReport {
        if let Some(done) = &self.finished {
            return done.clone();
        }
        let mut report = NativeFinishReport::default();
        if self.emitter.admit() {
            report.admitted = true;
            let started = self.emitter.clock.now().mono_ns;
            let mut left = budget.max_bytes;
            loop {
                let mut pass = NativePollReport::new(NativeAdmission::Ready);
                self.pass(left.min(self.limits.poll_bytes), &mut pass);
                left = left.saturating_sub(pass.bytes);
                report.bytes_drained += pass.bytes;
                if !self.emitter.live() || pass.bytes == 0 || left == 0 {
                    break;
                }
                let elapsed = self.emitter.clock.now().mono_ns.saturating_sub(started);
                if elapsed >= budget.max_duration_ns {
                    break;
                }
            }
            if self.emitter.live() {
                self.end_all(&mut report);
            }
            if self.emitter.live() {
                report.closed = self.emitter.close();
            }
        } else if self.emitter.admission == Admission::Unadmitted {
            // Never admitted before the boundary: whatever the store held was not captured.
            self.emitter.admission = Admission::Lost(GapReason::ShutdownDeadline);
        }
        report.final_seq = self.emitter.last_seq;
        report.latched = self.emitter.latched();
        report.discovery_capped = self.facts.capped;
        report.refused_entries = self.facts.refused;
        report.unavailable = self.facts.unavailable;
        report.untracked_files = self.facts.untracked;
        self.finished = Some(report.clone());
        report
    }

    pub fn loss_report(&self) -> NativeLossReport {
        NativeLossReport {
            producer: self.emitter.producer.clone(),
            admission: self.admission(),
            latched: self.emitter.latched(),
            last_queued_seq: self.emitter.last_seq,
            source_changes: self.source_changes,
            interrupted_streams: self.interrupted_streams,
            open_streams: self.emitter.open_streams,
            tracked_files: self.files.len(),
            file_limit_reached: self.file_limit_reached,
            discovery_capped: self.facts.capped,
            refused_entries: self.facts.refused,
            unavailable: self.facts.unavailable,
            untracked_files: self.facts.untracked,
        }
    }

    fn admission(&self) -> NativeAdmission {
        match self.emitter.admission {
            Admission::Unadmitted => NativeAdmission::Pending,
            Admission::Ready => NativeAdmission::Ready,
            Admission::Lost(reason) => NativeAdmission::Lost(reason),
            Admission::Closed => NativeAdmission::Closed,
        }
    }

    /// One discovery-and-read pass. `report` is fresh for each pass, and its stream-less loss
    /// facts are folded into the adapter's lifetime facts here, for polls and finish alike.
    fn pass(&mut self, byte_budget: u64, report: &mut NativePollReport) {
        let seq_before = self.emitter.next_seq;
        self.discover(report);
        self.read(byte_budget, report);
        report.frames += self.emitter.next_seq - seq_before;
        self.facts.fold(report);
    }

    fn discover(&mut self, report: &mut NativePollReport) {
        let mut entries_left = self.limits.max_dir_entries;
        for (index, source) in self.sources.iter().enumerate() {
            let mut walker = Walker {
                index,
                source,
                files: &mut self.files,
                resume: &self.resume,
                max_files: self.limits.max_files,
                entries_left: &mut entries_left,
                report: &mut *report,
                file_limit_reached: &mut self.file_limit_reached,
            };
            walker.walk(&source.root, &mut Vec::new());
        }
    }

    fn read(&mut self, byte_budget: u64, report: &mut NativePollReport) {
        let order = rotation(&self.files, self.rotate_after.as_ref());
        let mut pass = Pass {
            emitter: &mut self.emitter,
            buffer: &mut self.buffer,
            max_open_streams: self.limits.max_open_streams,
            budget: byte_budget,
            report,
            source_changes: &mut self.source_changes,
            interrupted: &mut self.interrupted_streams,
        };
        let mut stopped_at = None;
        for key in order {
            if !pass.emitter.live() {
                break;
            }
            let Some(tracked) = self.files.get_mut(&key) else {
                continue;
            };
            if process(&mut pass, &self.sources[key.0], &key.1, tracked) {
                pass.report.more_work = true;
            }
            if pass.budget == 0 && stopped_at.is_none() {
                stopped_at = Some(key);
            }
        }
        self.rotate_after = stopped_at;
    }

    fn end_all(&mut self, report: &mut NativeFinishReport) {
        for tracked in self.files.values_mut() {
            match &tracked.state {
                FileState::Open(stream) => {
                    let id = stream.stream;
                    let caught_up = tracked.caught_up;
                    tracked.state = FileState::Gone;
                    if caught_up {
                        if !self.emitter.end(id, EndReason::Complete) {
                            return;
                        }
                        report.completed_streams += 1;
                    } else {
                        if !self.emitter.interrupt(id, GapReason::ShutdownDeadline) {
                            return;
                        }
                        report.interrupted_streams += 1;
                        self.interrupted_streams += 1;
                    }
                }
                FileState::Pending { .. } => report.uncaptured_files += 1,
                // A snapshot a pass already read to its length and ended `Complete`: complete
                // at this boundary, and counted as such so a source captured whole is never
                // read as one that completed nothing.
                FileState::Done { .. } => report.completed_streams += 1,
                FileState::Gone => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Producer-side sequence, stream and admission state
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    Unadmitted,
    Ready,
    Lost(GapReason),
    Closed,
}

struct Emitter {
    sink: Arc<dyn CaptureSink>,
    clock: Box<dyn NativeClock>,
    producer: String,
    /// Producer IDs that may have reached the receiver (the initial one and each
    /// `RetryFresh` replacement), bounded by `MAX_PRODUCERS`.
    consumed: usize,
    admission: Admission,
    next_seq: u64,
    last_seq: Option<u64>,
    next_stream: u64,
    open_streams: usize,
}

impl Emitter {
    fn live(&self) -> bool {
        self.admission == Admission::Ready
    }

    fn latched(&self) -> Option<GapReason> {
        match self.admission {
            Admission::Lost(reason) => Some(reason),
            _ => None,
        }
    }

    /// Drive one admission step; `true` only once `Ready`.
    fn admit(&mut self) -> bool {
        if self.admission != Admission::Unadmitted {
            return self.live();
        }
        match self.sink.try_begin_epoch(&self.producer) {
            EpochAdmission::Ready => self.admission = Admission::Ready,
            EpochAdmission::Pending => {}
            EpochAdmission::RetryFresh => {
                if self.consumed >= MAX_PRODUCERS {
                    self.admission = Admission::Lost(GapReason::LimitExceeded);
                } else {
                    self.consumed += 1;
                    self.producer = fresh_producer();
                }
            }
            EpochAdmission::Disabled(reason) => self.admission = Admission::Lost(reason),
        }
        self.live()
    }

    /// Submit one frame at the next sequence number; any refusal latches the loss.
    fn submit(&mut self, stream: u64, event: FrameEvent) -> bool {
        if !self.live() {
            return false;
        }
        let reading = self.clock.now();
        let frame = CaptureFrame {
            stream: StreamKey {
                producer: self.producer.clone(),
                stream,
            },
            seq: self.next_seq,
            observed: CaptureTime {
                wall: reading.wall,
                mono_ns: reading.mono_ns,
                clock_id: self.producer.clone(),
            },
            event,
        };
        // Every frame this adapter builds is within bounds by construction; a clock that breaks
        // the wall format is the one way to fail here, and nothing is half-submitted.
        if frame.validate().is_err() {
            self.admission = Admission::Lost(GapReason::LimitExceeded);
            return false;
        }
        match self.sink.try_submit(frame) {
            SubmitResult::Queued => {
                self.last_seq = Some(self.next_seq);
                self.next_seq += 1;
                true
            }
            SubmitResult::Disabled(reason) => {
                self.admission = Admission::Lost(reason);
                false
            }
        }
    }

    fn open(&mut self, metadata: CaptureMeta) -> Option<u64> {
        let stream = self.next_stream;
        let event = FrameEvent::Open {
            source: CaptureSource::Native,
            metadata,
        };
        if !self.submit(stream, event) {
            return None;
        }
        self.next_stream += 1;
        self.open_streams += 1;
        Some(stream)
    }

    fn end(&mut self, stream: u64, reason: EndReason) -> bool {
        if !self.submit(stream, FrameEvent::End { reason }) {
            return false;
        }
        self.open_streams -= 1;
        true
    }

    fn interrupt(&mut self, stream: u64, reason: GapReason) -> bool {
        let gap = FrameEvent::Gap {
            reason,
            lost_bytes: None,
        };
        self.submit(stream, gap) && self.end(stream, EndReason::Interrupted)
    }

    fn close(&mut self) -> bool {
        if !self.live() || self.open_streams != 0 {
            return false;
        }
        match self.sink.try_close_epoch(&self.producer, self.last_seq) {
            SubmitResult::Queued => {
                self.admission = Admission::Closed;
                true
            }
            SubmitResult::Disabled(reason) => {
                self.admission = Admission::Lost(reason);
                false
            }
        }
    }
}

fn fresh_producer() -> String {
    format!("{:032x}", rand::random::<u128>())
}

// ---------------------------------------------------------------------------------------
// Sources, patterns and tracked files
// ---------------------------------------------------------------------------------------

struct Source {
    name: String,
    root: SourceDir,
    pattern: Vec<Component>,
    form: NativeStoreForm,
    hint: Option<NativeHint>,
}

enum Component {
    Literal(SourceName),
    Glob(String),
}

/// Source index and rendered relative path.
type FileKey = (usize, String);

#[derive(Debug, Clone, Copy)]
struct ResumePlan {
    generation: u64,
    start: ResumeStart,
}

#[derive(Debug, Clone, Copy)]
struct ResumeStart {
    durable_end: Option<u64>,
}

struct Tracked {
    components: Vec<SourceName>,
    /// The last generation opened for this path (zero before the first).
    generation: u64,
    state: FileState,
    /// The stream had emitted every byte its latest observation reported.
    caught_up: bool,
}

enum FileState {
    /// Known, without a stream: newly discovered, changed, or waiting for a slot.
    Pending {
        resume: Option<ResumeStart>,
    },
    Open(OpenStream),
    /// A snapshot read completely at these facts.
    Done {
        facts: SourceFactsV1,
    },
    /// Vanished or no longer a regular file.
    Gone,
}

struct OpenStream {
    stream: u64,
    identity: String,
    /// The next source offset: every byte before it was queued.
    offset: u64,
    /// For a snapshot, the facts it is read at.
    snapshot: Option<SourceFactsV1>,
}

/// Stream-less loss facts kept for the adapter's lifetime.
#[derive(Debug, Clone, Copy, Default)]
struct DiscoveryFacts {
    capped: bool,
    refused: u64,
    unavailable: u64,
    untracked: u64,
}

impl DiscoveryFacts {
    fn fold(&mut self, pass: &NativePollReport) {
        self.capped |= pass.discovery_capped;
        self.refused = self.refused.saturating_add(pass.refused_entries);
        self.unavailable = self.unavailable.saturating_add(pass.unavailable);
        self.untracked = self.untracked.saturating_add(pass.untracked_files);
    }
}

fn valid_scope_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_SCOPE_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Substitute a validated scope ID for `{id}`. An ID is required exactly when the pattern
/// names the placeholder, and no other brace may remain.
fn expand_pattern(pattern: &str, scope_id: Option<&str>) -> Result<String, NativeCaptureError> {
    let expanded = match (scope_id, pattern.contains(SCOPE_ID_PLACEHOLDER)) {
        (Some(id), true) => {
            if !valid_scope_id(id) {
                return Err(config(
                    "a scope id is 1-128 ASCII letters, digits, '-' or '_'",
                ));
            }
            pattern.replace(SCOPE_ID_PLACEHOLDER, id)
        }
        (None, false) => pattern.to_string(),
        (Some(_), false) => {
            return Err(config(
                "a scope id needs an `{id}` placeholder in its pattern",
            ));
        }
        (None, true) => return Err(config("an `{id}` placeholder needs a scope id")),
    };
    if expanded.contains(['{', '}']) {
        return Err(config("`{id}` is the only placeholder a pattern may use"));
    }
    Ok(expanded)
}

fn valid_source_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SOURCE_NAME_BYTES
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn parse_pattern(pattern: &str) -> Result<Vec<Component>, NativeCaptureError> {
    if pattern.is_empty() || pattern.len() > MAX_PATTERN_BYTES {
        return Err(config("a pattern is 1-1024 bytes"));
    }
    if pattern.starts_with('/') {
        return Err(config("a pattern is relative to its root"));
    }
    let mut components = Vec::new();
    for part in pattern.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains('\0') {
            return Err(config(
                "pattern components are non-empty and neither `.` nor `..`",
            ));
        }
        if part.contains("**") {
            return Err(config("recursive `**` is not supported; name each level"));
        }
        if part.contains(['*', '?']) {
            components.push(Component::Glob(part.to_string()));
        } else {
            let name = SourceName::from_component(part)
                .map_err(|_| config("a literal pattern component is not a valid name"))?;
            components.push(Component::Literal(name));
        }
    }
    if components.len() > MAX_NATIVE_PATTERN_DEPTH {
        return Err(config("a pattern has at most eight components"));
    }
    Ok(components)
}

/// One component against one glob: `*` matches any run (including none), `?` exactly one
/// character, anything else itself. These are the semantics of the private matcher in
/// `paths.rs` the V1 tailers used, applied here to capability-enumerated names.
fn glob_matches(name: &str, pattern: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let (mut i, mut j) = (0usize, 0usize);
    let (mut star, mut matched) = (None, 0usize);
    while i < name.len() {
        if j < pattern.len() && (pattern[j] == '?' || pattern[j] == name[i]) {
            i += 1;
            j += 1;
        } else if j < pattern.len() && pattern[j] == '*' {
            star = Some(j);
            matched = i;
            j += 1;
        } else if let Some(position) = star {
            j = position + 1;
            matched += 1;
            i = matched;
        } else {
            return false;
        }
    }
    while j < pattern.len() && pattern[j] == '*' {
        j += 1;
    }
    j == pattern.len()
}

fn render_relative(components: &[SourceName]) -> String {
    let mut out = String::new();
    for (index, component) in components.iter().enumerate() {
        if index > 0 {
            out.push('/');
        }
        for chunk in component.as_os_str().as_bytes().utf8_chunks() {
            for character in chunk.valid().chars() {
                match character {
                    '\\' => out.push_str("\\\\"),
                    c if c.is_control() => {
                        let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
                    }
                    c => out.push(c),
                }
            }
            for byte in chunk.invalid() {
                let _ = write!(out, "\\x{byte:02x}");
            }
        }
    }
    out
}

fn rotation(files: &BTreeMap<FileKey, Tracked>, after: Option<&FileKey>) -> Vec<FileKey> {
    let Some(after) = after else {
        return files.keys().cloned().collect();
    };
    let tail = files.range::<FileKey, _>((Bound::Excluded(after), Bound::Unbounded));
    let head = files.range::<FileKey, _>((Bound::Unbounded, Bound::Included(after)));
    tail.chain(head).map(|(key, _)| key.clone()).collect()
}

// ---------------------------------------------------------------------------------------
// Capability-rooted discovery
// ---------------------------------------------------------------------------------------

struct Walker<'a> {
    index: usize,
    source: &'a Source,
    files: &'a mut BTreeMap<FileKey, Tracked>,
    resume: &'a BTreeMap<FileKey, ResumePlan>,
    max_files: usize,
    entries_left: &'a mut usize,
    report: &'a mut NativePollReport,
    file_limit_reached: &'a mut bool,
}

impl Walker<'_> {
    fn walk(&mut self, dir: &SourceDir, prefix: &mut Vec<SourceName>) {
        let level = prefix.len();
        let last = level + 1 == self.source.pattern.len();
        let names = match &self.source.pattern[level] {
            Component::Literal(name) => vec![name.clone()],
            // Directory levels newest first: a `YYYY/MM/DD` store reaches today before history.
            Component::Glob(glob) => match self.enumerate(dir, glob, !last) {
                Ok(names) => names,
                Err(_) => {
                    self.report.unavailable += 1;
                    return;
                }
            },
        };
        for name in names {
            let kind = match dir.observe(&name) {
                Ok(SourceLookup::Present(observed)) => observed.kind(),
                Ok(SourceLookup::Missing) => continue,
                Err(_) => {
                    self.report.unavailable += 1;
                    continue;
                }
            };
            match (kind, last) {
                (SourceKind::Regular, true) => {
                    prefix.push(name);
                    self.register(prefix);
                    prefix.pop();
                }
                (SourceKind::Directory, false) => match open_child_dir(dir, &name) {
                    Ok(Some(child)) => {
                        prefix.push(name);
                        self.walk(&child, prefix);
                        prefix.pop();
                    }
                    Ok(None) => {}
                    Err(_) => self.report.unavailable += 1,
                },
                (SourceKind::Regular, false) => {}
                (SourceKind::Directory | SourceKind::SymbolicLink | SourceKind::Special, _) => {
                    self.report.refused_entries += 1;
                }
            }
        }
    }

    fn enumerate(
        &mut self,
        dir: &SourceDir,
        glob: &str,
        descending: bool,
    ) -> Result<Vec<SourceName>, SourceCapError> {
        let mut names = Vec::new();
        {
            let mut entries = dir.entries()?;
            loop {
                if *self.entries_left == 0 {
                    self.report.discovery_capped = true;
                    break;
                }
                let Some(name) = entries.next_entry()? else {
                    break;
                };
                *self.entries_left -= 1;
                if glob_matches(&name.as_os_str().to_string_lossy(), glob) {
                    names.push(name);
                }
            }
        }
        names.sort_by(|a, b| a.as_os_str().as_bytes().cmp(b.as_os_str().as_bytes()));
        if descending {
            names.reverse();
        }
        Ok(names)
    }

    fn register(&mut self, components: &[SourceName]) {
        let key = (self.index, render_relative(components));
        if self.files.contains_key(&key) {
            return;
        }
        let label_bytes = self.source.name.len() + 1 + key.1.len() + GENERATION_SUFFIX_BYTES;
        if label_bytes > MAX_METADATA_STRING_BYTES {
            self.report.refused_entries += 1;
            return;
        }
        if self.files.len() >= self.max_files {
            self.report.untracked_files += 1;
            *self.file_limit_reached = true;
            return;
        }
        let plan = self.resume.get(&key).copied();
        self.files.insert(
            key,
            Tracked {
                components: components.to_vec(),
                generation: plan.map_or(0, |plan| plan.generation),
                state: FileState::Pending {
                    resume: plan.map(|plan| plan.start),
                },
                caught_up: false,
            },
        );
    }
}

/// A directory reached through the retained root: the root itself or an owned child.
enum DirRef<'a> {
    Root(&'a SourceDir),
    Child(SourceDir),
}

impl DirRef<'_> {
    fn get(&self) -> &SourceDir {
        match self {
            DirRef::Root(dir) => dir,
            DirRef::Child(dir) => dir,
        }
    }
}

/// Open `name` below `dir` as a directory, no-follow and on the root's mount. `None` when
/// it is absent or not a directory (a symlink included).
fn open_child_dir(dir: &SourceDir, name: &SourceName) -> Result<Option<SourceDir>, SourceCapError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let observed = match dir.observe(name)? {
            SourceLookup::Present(observed) if observed.kind() == SourceKind::Directory => observed,
            _ => return Ok(None),
        };
        match dir.open_dir_matching(name, &observed) {
            Ok(child) => return Ok(Some(child)),
            Err(SourceCapError::Refused {
                reason: RefusalKind::MetadataChanged,
                ..
            }) if attempt < OPEN_ATTEMPTS => continue,
            Err(error) => return Err(error),
        }
    }
}

fn open_chain<'a>(
    root: &'a SourceDir,
    components: &[SourceName],
) -> Result<Option<DirRef<'a>>, SourceCapError> {
    let mut current = DirRef::Root(root);
    for name in components {
        match open_child_dir(current.get(), name)? {
            Some(child) => current = DirRef::Child(child),
            None => return Ok(None),
        }
    }
    Ok(Some(current))
}

// ---------------------------------------------------------------------------------------
// Bounded reading
// ---------------------------------------------------------------------------------------

struct Pass<'a> {
    emitter: &'a mut Emitter,
    buffer: &'a mut [u8],
    max_open_streams: usize,
    budget: u64,
    report: &'a mut NativePollReport,
    source_changes: &'a mut u64,
    interrupted: &'a mut u64,
}

impl Pass<'_> {
    fn unavailable(&mut self, tracked: &mut Tracked) -> bool {
        self.report.unavailable += 1;
        tracked.caught_up = false;
        false
    }
}

/// Observe one tracked file, settle its stream against what is there now, open its next
/// generation if needed and read within the pass budget. Returns `true` when the byte budget
/// deferred bytes this observation reported.
fn process(pass: &mut Pass<'_>, source: &Source, relative: &str, tracked: &mut Tracked) -> bool {
    let Some((name, parents)) = tracked.components.split_last() else {
        return false;
    };
    let name = name.clone();
    let parent = match open_chain(&source.root, parents) {
        Ok(parent) => parent,
        Err(_) => return pass.unavailable(tracked),
    };
    let mut attempt = 0;
    loop {
        attempt += 1;
        let observed = match parent.as_ref().map(|dir| dir.get().observe(&name)) {
            None => None,
            Some(Ok(SourceLookup::Present(observed))) if observed.kind() == SourceKind::Regular => {
                Some(observed)
            }
            Some(Ok(_)) => None,
            Some(Err(_)) => return pass.unavailable(tracked),
        };
        if !settle(pass, tracked, observed.as_ref()) {
            return false;
        }
        let Some(observed) = observed else {
            tracked.caught_up = false;
            return false;
        };
        if matches!(tracked.state, FileState::Pending { .. })
            && !open_generation(pass, source, relative, tracked, &observed)
        {
            tracked.caught_up = false;
            return false;
        }
        let FileState::Open(stream) = &mut tracked.state else {
            tracked.caught_up = true; // an unchanged completed snapshot
            return false;
        };
        let length = observed.length();
        if stream.offset < length && pass.budget > 0 {
            let Some(dir) = parent.as_ref() else {
                return false;
            };
            let file = match dir.get().open_regular_matching(&name, &observed, u64::MAX) {
                Ok(file) => file,
                Err(SourceCapError::Refused {
                    reason: RefusalKind::MetadataChanged,
                    ..
                }) if attempt < OPEN_ATTEMPTS => continue,
                Err(_) => return pass.unavailable(tracked),
            };
            match read_stream(pass, stream, file) {
                ReadEnd::Read => {}
                ReadEnd::Lost => return false,
                ReadEnd::Failed => return pass.unavailable(tracked),
                ReadEnd::SnapshotChanged => {
                    let id = stream.stream;
                    tracked.state = FileState::Pending { resume: None };
                    tracked.caught_up = false;
                    *pass.source_changes += 1;
                    if pass.emitter.interrupt(id, GapReason::SourceChanged) {
                        *pass.interrupted += 1;
                    }
                    return false;
                }
            }
        }
        if let Some(facts) = &stream.snapshot
            && stream.offset == facts.length
        {
            let (id, facts) = (stream.stream, facts.clone());
            tracked.state = FileState::Done { facts };
            tracked.caught_up = true;
            pass.emitter.end(id, EndReason::Complete);
            return false;
        }
        tracked.caught_up = stream.offset == length;
        return stream.offset < length && pass.budget == 0;
    }
}

/// Reconcile a tracked file with its current observation. A changed source ends its stream
/// (`Gap { SourceChanged }`, `End { Interrupted }`) before anything else can open. Returns
/// `false` when that submission latched a loss.
fn settle(
    pass: &mut Pass<'_>,
    tracked: &mut Tracked,
    observed: Option<&SourceObservation>,
) -> bool {
    let next = match &tracked.state {
        FileState::Open(stream) => {
            let changed = match observed {
                None => true,
                Some(observed) => {
                    let facts = observed.facts();
                    facts.identity != stream.identity
                        || facts.length < stream.offset
                        || stream
                            .snapshot
                            .as_ref()
                            .is_some_and(|snapshot| *snapshot != facts)
                }
            };
            if !changed {
                return true;
            }
            let id = stream.stream;
            tracked.state = FileState::Gone;
            *pass.source_changes += 1;
            if !pass.emitter.interrupt(id, GapReason::SourceChanged) {
                return false;
            }
            *pass.interrupted += 1;
            observed.map(|_| FileState::Pending { resume: None })
        }
        FileState::Done { facts } => match observed {
            None => Some(FileState::Gone),
            Some(observed) if observed.facts() != *facts => {
                Some(FileState::Pending { resume: None })
            }
            Some(_) => None,
        },
        FileState::Gone => observed.map(|_| FileState::Pending { resume: None }),
        FileState::Pending { .. } => None,
    };
    if let Some(next) = next {
        tracked.state = next;
    }
    true
}

/// Open the next generation of a pending file. Returns `false` when it must wait for a
/// stream slot or a submission latched a loss.
fn open_generation(
    pass: &mut Pass<'_>,
    source: &Source,
    relative: &str,
    tracked: &mut Tracked,
    observed: &SourceObservation,
) -> bool {
    let FileState::Pending { resume } = tracked.state else {
        return true;
    };
    if pass.emitter.open_streams >= pass.max_open_streams {
        pass.report.waiting_files += 1;
        return false;
    }
    let generation = tracked.generation + 1;
    let label = NativeLabel {
        source: source.name.clone(),
        relative_path: relative.to_string(),
        generation,
    };
    let metadata = CaptureMeta {
        wire: None,
        label: label.to_string(),
        protocol_hint: source.hint.as_ref().map(|hint| hint.as_str().to_string()),
        method: None,
        host: None,
        path: None,
        status: None,
        content_type: None,
    };
    let Some(stream) = pass.emitter.open(metadata) else {
        return false;
    };
    tracked.generation = generation;
    let facts = observed.facts();
    let mut offset = 0;
    if let Some(ResumeStart { durable_end }) = resume {
        let reason = match durable_end {
            Some(end) if end <= facts.length => {
                offset = end;
                GapReason::ProducerLost
            }
            Some(_) => GapReason::SourceChanged,
            None => GapReason::ProducerLost,
        };
        let gap = FrameEvent::Gap {
            reason,
            lost_bytes: None,
        };
        if !pass.emitter.submit(stream, gap) {
            return false;
        }
    }
    tracked.state = FileState::Open(OpenStream {
        stream,
        identity: facts.identity.clone(),
        offset,
        snapshot: (source.form == NativeStoreForm::Snapshot).then_some(facts),
    });
    true
}

enum ReadEnd {
    Read,
    Lost,
    Failed,
    SnapshotChanged,
}

/// Read from the stream's offset toward the opening length, one bounded chunk per frame.
fn read_stream(pass: &mut Pass<'_>, stream: &mut OpenStream, mut file: SourceFile) -> ReadEnd {
    if file.seek(SeekFrom::Start(stream.offset)).is_err() {
        return ReadEnd::Failed;
    }
    let limit = file.observation().length();
    while stream.offset < limit && pass.budget > 0 {
        let want = (limit - stream.offset)
            .min(pass.budget)
            .min(MAX_SEGMENT_BYTES as u64) as usize;
        let count = match file.read(&mut pass.buffer[..want]) {
            // Shorter than observed: truncated under the reader; the next poll settles it.
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => return ReadEnd::Failed,
        };
        let bytes = FrameEvent::Bytes {
            offset: stream.offset,
            data: pass.buffer[..count].to_vec(),
        };
        if !pass.emitter.submit(stream.stream, bytes) {
            return ReadEnd::Lost;
        }
        stream.offset += count as u64;
        pass.budget -= count as u64;
        pass.report.bytes += count as u64;
    }
    if stream
        .snapshot
        .as_ref()
        .is_some_and(|snapshot| stream.offset == snapshot.length)
        && file.finish().is_err()
    {
        return ReadEnd::SnapshotChanged;
    }
    ReadEnd::Read
}
