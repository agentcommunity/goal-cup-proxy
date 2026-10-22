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
//! A source with [`NativeSourceSpec::modified_since`] (D57) admits a matched file into its
//! tracked set only once the later of the file's modification and status-change times, in
//! whole seconds, is at or after it: a harness or agent can set the modification time back,
//! but not the change time.
//! The check comes before the file counts against [`NativeCaptureLimits::max_files`], so a
//! shared store's old sessions are neither tracked nor reported; once admitted, a file stays
//! and is read from offset 0. Every admitted file is recorded, and none is preferred.
//!
//! # Producer, admission and ordering
//!
//! The adapter is one capture producer. It mints a fresh 128-bit producer ID, asks
//! [`CaptureSink::try_begin_epoch`] on each poll until the answer is `Ready`, and submits no
//! frame before that. `Pending` keeps the ID; `RetryFresh` retires it for a fresh one (at most
//! `MAX_FRESH_IDS` IDs in all); `Disabled` latches the loss. `seq` starts at zero and increases
//! across every frame of every file; payload streams start at one; stream zero is never used,
//! because a native producer emits no `Connection` or `Recovery` control. Each observation carries
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
//! (`YYYY/MM/DD`) reaches its newest day before older history. A poll that stops at the cap
//! leaves a resume point: the next poll reads past the entries already examined (readdir
//! only: no match, no `fstatat`, except to descend a directory level) and examines the next
//! window, and a poll that reaches the end starts the next cycle at the front. Every entry of
//! a store of `n` entries is examined within `ceil(n / max_dir_entries)` polls, whatever order
//! readdir returns. It tracks at most
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
//! tail loss. There is no wire-style recovery. A loss stays latched for the life of the
//! adapter; nothing in this tree resumes a lost producer.
//!
//! Losses that have no stream to carry a `Gap` frame are reported in [`NativeLossReport`] and
//! [`NativeFinishReport`]. A matched entry kept out for good — a file over `max_files`, or an
//! entry refused (symlink, special file, over-long label), each since the source's start —
//! counts in [`NativeLossReport::lost_entries`], the most found in one discovery cycle, so a
//! lasting loss counts once and a cycle with more losses than any before raises it. The entry
//! owner records a stream-less `CaptureRecord::Gap { LimitExceeded }` when it first rises and
//! again when it grows. The
//! finish walks a whole discovery cycle when its budget allows; one that could not is
//! [`NativeFinishReport::discovery_capped`], and the owner records that too. An entry
//! unavailable in one poll is retried in the next and is not a loss. A finish report with none
//! of them still does not prove the expected file existed: the owner checks it saw the `Open`
//! it expected.
//!
//! # Integration interface
//!
//! - `NativeCapture::new(config: NativeCaptureConfig, sink: Arc<dyn CaptureSink>,
//!   clock: Box<dyn NativeClock>) -> Result<NativeCapture, NativeCaptureError>`
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
//! fn wire(sink: Arc<dyn CaptureSink>, codex_home: std::path::PathBuf) {
//!     let config = NativeCaptureConfig {
//!         sources: vec![NativeSourceSpec {
//!             name: "codex-rollout".into(),
//!             root: codex_home.join("sessions"),
//!             pattern: "*/*/*/rollout-*.jsonl".into(),
//!             form: NativeStoreForm::AppendOnly,
//!             hint: Some(NativeHint::new("codex-jsonl").unwrap()),
//!             modified_since: None,
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
    MAX_SEGMENT_BYTES, StreamKey, SubmitResult,
};
use goalcup_protocol::harness::glob_matches;
use goalcup_protocol::hints::valid_hint;
use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::io::{Read, Seek, SeekFrom};
use std::ops::Bound;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

/// Most native sources one adapter reads.
pub(crate) const MAX_NATIVE_SOURCES: usize = 8;
/// Most components in one source pattern.
pub(crate) const MAX_NATIVE_PATTERN_DEPTH: usize = 8;
/// Hard ceiling on [`NativeCaptureLimits::max_files`].
pub(crate) const MAX_NATIVE_FILES: usize = 20_000;
/// Hard ceiling on [`NativeCaptureLimits::max_dir_entries`].
pub(crate) const MAX_NATIVE_DIR_ENTRIES: usize = 65_536;

/// Most producer IDs one adapter proposes. A sink retires only an ID it saw before, so a freshly
/// minted one is never answered `RetryFresh`; this stops a sink that answers it forever. It is
/// not the entry's producer bound (D58).
const MAX_FRESH_IDS: usize = 128;

const MAX_SOURCE_NAME_BYTES: usize = 64;
const MAX_PATTERN_BYTES: usize = 1024;
/// Observe-then-open attempts before a racing writer defers a file to the next poll.
const OPEN_ATTEMPTS: usize = 3;
/// Room a label keeps for its `#g<generation>` suffix: `#g` and `u64::MAX`'s 20 digits.
const GENERATION_SUFFIX_BYTES: usize = 22;

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
    /// single-component glob using `*` and `?`; `**`, `.`, `..`, a brace, empty components and
    /// absolute patterns are refused.
    pub pattern: String,
    pub form: NativeStoreForm,
    /// Protocol hint, append-only JSONL stores only.
    pub hint: Option<NativeHint>,
    /// The run's start (D57): a matched file is tracked only once the later of its
    /// modification and status-change times, truncated to whole seconds, is at or after it.
    /// `None` tracks every matched file.
    pub modified_since: Option<SystemTime>,
}

/// Per-adapter bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeCaptureLimits {
    /// Bytes read per poll, shared across files; 1..=`MAX_CAPTURE_QUEUE_BYTES`.
    pub poll_bytes: u64,
    /// Simultaneously open native streams; 1..=`MAX_OPEN_STREAMS`.
    pub max_open_streams: usize,
    /// Files tracked (open, completed or waiting); 1..=`MAX_NATIVE_FILES`.
    pub max_files: usize,
    /// Directory entries examined by discovery per poll; 1..=`MAX_NATIVE_DIR_ENTRIES`.
    pub max_dir_entries: usize,
}

impl Default for NativeCaptureLimits {
    /// 16 source chunks per poll (1 MiB, an eighth of the capture queue) and a quarter of the
    /// entry's open-stream allowance, leaving the rest to wire capture. 8192 directory entries
    /// enumerate a shared home's directory of 5 000 old sessions whole on every poll (D57).
    fn default() -> Self {
        NativeCaptureLimits {
            poll_bytes: 16 * MAX_SEGMENT_BYTES as u64,
            max_open_streams: MAX_OPEN_STREAMS / 4,
            max_files: 1024,
            max_dir_entries: 8192,
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

/// The process clock: nanoseconds since construction, sleep included
/// ([`goalcup_protocol::clock`]), and the daemon's RFC 3339 wall time.
#[derive(Debug, Clone, Copy)]
pub struct SystemNativeClock {
    origin: goalcup_protocol::clock::ContinuousOrigin,
}

impl SystemNativeClock {
    pub fn new() -> Self {
        SystemNativeClock {
            origin: goalcup_protocol::clock::ContinuousOrigin::now(),
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
            mono_ns: self.origin.elapsed_ns(),
        }
    }
}

/// A native stream label: `<source>:<relative path>#g<generation>`.
///
/// Display text for people; it is not a join key between streams. Path bytes that are not printable UTF-8 are escaped (`\\`, `\u{..}`,
/// `\xNN`), so distinct names render distinctly and no control byte reaches metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLabel {
    pub source: String,
    pub relative_path: String,
    pub generation: u64,
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
    /// Discovery stopped at `max_dir_entries`; the next poll resumes past what this one
    /// examined.
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
    /// Discovery stopped at `max_dir_entries` at least once, every poll and finish pass
    /// included. With rotation this is a cost, not a loss; the finish's own loss is
    /// [`NativeFinishReport::discovery_capped`].
    pub discovery_capped: bool,
    pub refused_entries: u64,
    pub unavailable: u64,
    pub untracked_files: u64,
    /// Matched entries kept out for good (refused, or past `max_files`), written or changed
    /// since the source's start: the most found in one discovery cycle, which examines each
    /// entry once. It never falls and a lasting loss does not raise it. A high-water mark, it
    /// rises only when a cycle finds more than any cycle before: a new loss that replaces one
    /// gone since is not seen here.
    pub lost_entries: u64,
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
    /// The finish's budget ended before any of its discovery cycles was whole: an entry it
    /// never reached may hold a file no stream carries.
    pub discovery_capped: bool,
    /// Lifetime discovery facts, every poll and finish pass included: matched entries refused
    /// (symlink, special file, directory, over-long label), summed over passes, so an entry
    /// seen in several passes counts in each.
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
    rotate_after: Option<FileKey>,
    buffer: Vec<u8>,
    source_changes: u64,
    interrupted_streams: u64,
    file_limit_reached: bool,
    facts: DiscoveryFacts,
    /// Entries the current discovery cycle already examined: the next pass reads past them.
    resume_at: u64,
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
            let pattern = parse_pattern(&spec.pattern)?;
            let scope = (spec.root.clone(), spec.pattern);
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
                modified_since: spec.modified_since.map(whole_seconds),
            });
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
            rotate_after: None,
            buffer: vec![0; MAX_SEGMENT_BYTES],
            source_changes: 0,
            interrupted_streams: 0,
            file_limit_reached: false,
            facts: DiscoveryFacts::default(),
            resume_at: 0,
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
            // The finish walks its own cycle from the front: an entry examined by an earlier
            // poll may hold a file written since.
            self.resume_at = 0;
            self.facts.cycle = 0;
            let mut whole = false;
            loop {
                let mut pass = NativePollReport::new(NativeAdmission::Ready);
                self.pass(left.min(self.limits.poll_bytes), &mut pass);
                left = left.saturating_sub(pass.bytes);
                report.bytes_drained += pass.bytes;
                whole |= !pass.discovery_capped;
                report.discovery_capped = !whole;
                let settled = pass.bytes == 0 && !pass.discovery_capped;
                if !self.emitter.live() || settled || left == 0 {
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
            lost_entries: self.facts.lost,
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

    /// One window of the discovery cycle: read past what the cycle already examined, then
    /// examine up to `max_dir_entries` entries. A capped window moves the resume point on; one
    /// that reached the end starts the next cycle at the front.
    fn discover(&mut self, report: &mut NativePollReport) {
        let mut entries_left = self.limits.max_dir_entries;
        let mut pass_over = self.resume_at;
        for (index, source) in self.sources.iter().enumerate() {
            let mut walker = Walker {
                index,
                source,
                files: &mut self.files,
                max_files: self.limits.max_files,
                entries_left: &mut entries_left,
                pass_over: &mut pass_over,
                report: &mut *report,
                file_limit_reached: &mut self.file_limit_reached,
            };
            walker.walk(&source.root, &mut Vec::new());
        }
        self.resume_at = if report.discovery_capped {
            self.resume_at + self.limits.max_dir_entries as u64
        } else {
            0
        };
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
                FileState::Pending => report.uncaptured_files += 1,
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
    /// `RetryFresh` replacement), bounded by `MAX_FRESH_IDS`.
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
                if self.consumed >= MAX_FRESH_IDS {
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
    /// [`NativeSourceSpec::modified_since`] in whole seconds since the epoch.
    modified_since: Option<i64>,
}

/// `at` truncated to whole seconds since the epoch; a time before it admits every file.
fn whole_seconds(at: SystemTime) -> i64 {
    at.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(i64::MIN, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

enum Component {
    Literal(SourceName),
    Glob(String),
}

/// Source index and rendered relative path.
type FileKey = (usize, String);

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
    Pending,
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
    /// Entries kept out so far in the current discovery cycle.
    cycle: u64,
    /// The most `cycle` has reached: [`NativeLossReport::lost_entries`].
    lost: u64,
}

impl DiscoveryFacts {
    fn fold(&mut self, pass: &NativePollReport) {
        self.capped |= pass.discovery_capped;
        self.refused = self.refused.saturating_add(pass.refused_entries);
        self.unavailable = self.unavailable.saturating_add(pass.unavailable);
        self.untracked = self.untracked.saturating_add(pass.untracked_files);
        let kept_out = pass.refused_entries.saturating_add(pass.untracked_files);
        self.cycle = self.cycle.saturating_add(kept_out);
        self.lost = self.lost.max(self.cycle);
        if !pass.discovery_capped {
            self.cycle = 0;
        }
    }
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
    if pattern.contains(['{', '}']) {
        return Err(config("a pattern has no placeholder and no brace"));
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
    max_files: usize,
    entries_left: &'a mut usize,
    /// Entries still to read past: the current cycle examined them in an earlier pass.
    pass_over: &'a mut u64,
    report: &'a mut NativePollReport,
    file_limit_reached: &'a mut bool,
}

impl Walker<'_> {
    fn walk(&mut self, dir: &SourceDir, prefix: &mut Vec<SourceName>) {
        let level = prefix.len();
        let last = level + 1 == self.source.pattern.len();
        let names = match &self.source.pattern[level] {
            Component::Literal(name) => vec![(name.clone(), false)],
            // Directory levels newest first: a `YYYY/MM/DD` store reaches today before history.
            Component::Glob(glob) => match self.enumerate(dir, glob, !last) {
                Ok(names) => names,
                Err(_) => {
                    self.report.unavailable += 1;
                    return;
                }
            },
        };
        for (name, passed) in names {
            let observed = match dir.observe(&name) {
                Ok(SourceLookup::Present(observed)) => observed,
                Ok(SourceLookup::Missing) => continue,
                Err(_) => {
                    self.report.unavailable += 1;
                    continue;
                }
            };
            // A passed-over entry was judged earlier in this cycle; only its subtree is new.
            if passed && observed.kind() != SourceKind::Directory {
                continue;
            }
            match (observed.kind(), last) {
                (SourceKind::Regular, true) => {
                    prefix.push(name);
                    self.register(prefix, &observed);
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
                // An entry from before the run is not this run's, refused or not (D57).
                (SourceKind::Directory | SourceKind::SymbolicLink | SourceKind::Special, _) => {
                    if self.since_start(&observed) {
                        self.report.refused_entries += 1;
                    }
                }
            }
        }
    }

    /// The matching names of `dir`, each with whether it was passed over. At a directory
    /// level (`descending`) a passed-over match is kept, since the cycle's position lies in its
    /// subtree; at the file level it is dropped unread.
    fn enumerate(
        &mut self,
        dir: &SourceDir,
        glob: &str,
        descending: bool,
    ) -> Result<Vec<(SourceName, bool)>, SourceCapError> {
        let mut names = Vec::new();
        {
            let mut entries = dir.entries()?;
            loop {
                let passed = *self.pass_over > 0;
                if !passed && *self.entries_left == 0 {
                    self.report.discovery_capped = true;
                    break;
                }
                let Some(name) = entries.next_entry()? else {
                    break;
                };
                if passed {
                    *self.pass_over -= 1;
                    if !descending {
                        continue;
                    }
                } else {
                    *self.entries_left -= 1;
                }
                if glob_matches(&name.as_os_str().to_string_lossy(), glob) {
                    names.push((name, passed));
                }
            }
        }
        names.sort_by(|a, b| a.0.as_os_str().as_bytes().cmp(b.0.as_os_str().as_bytes()));
        if descending {
            names.reverse();
        }
        Ok(names)
    }

    /// Written or changed since the run started, or the source has no start. Otherwise the
    /// entry is not this run's yet and is not counted (D57). The no-follow observation's change
    /// time cannot be set back, so a backdated entry is still seen.
    fn since_start(&self, observed: &SourceObservation) -> bool {
        self.source.modified_since.is_none_or(|since| {
            let facts = observed.facts();
            facts.modified_seconds.max(facts.changed_seconds) >= since
        })
    }

    fn register(&mut self, components: &[SourceName], observed: &SourceObservation) {
        let key = (self.index, render_relative(components));
        if self.files.contains_key(&key) {
            return;
        }
        if !self.since_start(observed) {
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
        self.files.insert(
            key,
            Tracked {
                components: components.to_vec(),
                generation: 0,
                state: FileState::Pending,
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
        if matches!(tracked.state, FileState::Pending)
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
                    tracked.state = FileState::Pending;
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
            observed.map(|_| FileState::Pending)
        }
        FileState::Done { facts } => match observed {
            None => Some(FileState::Gone),
            Some(observed) if observed.facts() != *facts => Some(FileState::Pending),
            Some(_) => None,
        },
        FileState::Gone => observed.map(|_| FileState::Pending),
        FileState::Pending => None,
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
    if !matches!(tracked.state, FileState::Pending) {
        return true;
    }
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
    tracked.state = FileState::Open(OpenStream {
        stream,
        identity: facts.identity.clone(),
        offset: 0,
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
