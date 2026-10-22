//! Scoped native byte capture (`goalcup_daemon::native_capture`).
//!
//! Every test owns its isolated temporary home, its sink and its clock: nothing here points
//! at a real Codex or Claude home, and nothing is shared between tests. Files are created only
//! at relative paths under that temporary home.
//!
//! The oracle is independent of the adapter: [`replay`] rebuilds each stream from the frames
//! the sink accepted and checks the producer-wide sequence, stream and offset rules; the byte
//! expectations are the files the test itself wrote.

use goalcup_daemon::native_capture::{
    ClockReading, NativeAdmission, NativeCapture, NativeCaptureConfig, NativeCaptureError,
    NativeCaptureLimits, NativeClock, NativeFinishBudget, NativeHint, NativeSourceSpec,
    NativeStoreForm,
};
use goalcup_protocol::capture::{
    CaptureFrame, CaptureSink, CaptureSource, EndReason, EpochAdmission, FrameEvent, GapReason,
    MAX_OPEN_STREAMS, MAX_SEGMENT_BYTES, StreamKey, SubmitResult, is_capture_id,
};
use goalcup_protocol::control::format_z;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

// ---------------------------------------------------------------------------------------
// Local bounded sink
// ---------------------------------------------------------------------------------------

/// A bounded in-memory [`CaptureSink`] that checks the sink lifecycle under one lock.
///
/// Admission follows an optional script, then admits exactly one producer: a second epoch
/// is native recovery, which this adapter does not support, so it is refused and recorded as
/// a violation rather than answered with a default success.
struct CollectSink {
    ceiling: usize,
    state: Mutex<SinkState>,
}

#[derive(Default)]
struct SinkState {
    frames: Vec<CaptureFrame>,
    used: usize,
    refused: Option<GapReason>,
    script: VecDeque<EpochAdmission>,
    asked: Vec<String>,
    admitted: Option<String>,
    open: BTreeSet<u64>,
    opened: BTreeSet<u64>,
    closed: Option<Option<u64>>,
    submit_calls: u64,
    close_calls: u64,
    violations: Vec<String>,
}

/// Per-frame accounting: data bytes plus metadata bytes plus a fixed envelope allowance.
fn frame_cost(frame: &CaptureFrame) -> usize {
    64 + match &frame.event {
        FrameEvent::Bytes { data, .. } => data.len(),
        FrameEvent::Open { metadata, .. } => {
            metadata.label.len() + metadata.protocol_hint.as_ref().map_or(0, String::len)
        }
        _ => 0,
    }
}

impl CollectSink {
    fn new() -> Arc<CollectSink> {
        Self::build(usize::MAX, &[])
    }

    fn with_ceiling(ceiling: usize) -> Arc<CollectSink> {
        Self::build(ceiling, &[])
    }

    fn scripted(answers: &[EpochAdmission]) -> Arc<CollectSink> {
        Self::build(usize::MAX, answers)
    }

    fn build(ceiling: usize, answers: &[EpochAdmission]) -> Arc<CollectSink> {
        Arc::new(CollectSink {
            ceiling,
            state: Mutex::new(SinkState {
                script: answers.iter().copied().collect(),
                ..SinkState::default()
            }),
        })
    }

    fn frames(&self) -> Vec<CaptureFrame> {
        self.state.lock().unwrap().frames.clone()
    }

    fn submit_calls(&self) -> u64 {
        self.state.lock().unwrap().submit_calls
    }

    fn close_calls(&self) -> u64 {
        self.state.lock().unwrap().close_calls
    }

    fn closed(&self) -> Option<Option<u64>> {
        self.state.lock().unwrap().closed
    }

    fn asked(&self) -> Vec<String> {
        self.state.lock().unwrap().asked.clone()
    }

    fn assert_clean(&self) {
        let state = self.state.lock().unwrap();
        assert!(
            state.violations.is_empty(),
            "sink lifecycle violations: {:?}",
            state.violations
        );
    }
}

impl CaptureSink for CollectSink {
    fn try_submit(&self, frame: CaptureFrame) -> SubmitResult {
        let mut state = self.state.lock().unwrap();
        state.submit_calls += 1;
        if let Some(reason) = state.refused {
            return SubmitResult::Disabled(reason);
        }
        if state.admitted.as_deref() != Some(frame.stream.producer.as_str()) {
            state
                .violations
                .push("frame from a producer the sink never admitted Ready".into());
        }
        if state.closed.is_some() {
            state
                .violations
                .push("frame after the producer closed".into());
        }
        let expected = state.frames.len() as u64;
        if frame.seq != expected {
            let message = format!("seq {} where {} was next", frame.seq, expected);
            state.violations.push(message);
        }
        if frame.validate().is_err() {
            state.violations.push("frame breaks a C1 bound".into());
        }
        let stream = frame.stream.stream;
        match &frame.event {
            FrameEvent::Open { .. } => {
                if !state.opened.insert(stream) {
                    state.violations.push("stream id reused".into());
                }
            }
            FrameEvent::Bytes { .. } | FrameEvent::Gap { .. } | FrameEvent::End { .. } => {
                if !state.open.contains(&stream) {
                    state
                        .violations
                        .push("payload on a stream that is not open".into());
                }
            }
            FrameEvent::Connection { .. }
            | FrameEvent::Recovery { .. }
            | FrameEvent::Loss { .. } => {
                state
                    .violations
                    .push("native producer emitted a wire control".into());
            }
        }
        let cost = frame_cost(&frame);
        if state.used + cost > self.ceiling {
            state.refused = Some(GapReason::QueueFull);
            return SubmitResult::Disabled(GapReason::QueueFull);
        }
        state.used += cost;
        match &frame.event {
            FrameEvent::Open { .. } => {
                state.open.insert(stream);
            }
            FrameEvent::End { .. } => {
                state.open.remove(&stream);
            }
            _ => {}
        }
        state.frames.push(frame);
        SubmitResult::Queued
    }

    fn try_begin_epoch(&self, producer: &str) -> EpochAdmission {
        let mut state = self.state.lock().unwrap();
        state.asked.push(producer.to_string());
        if !is_capture_id(producer) {
            state
                .violations
                .push("producer is not a fresh capture id".into());
        }
        let answer = match state.script.pop_front() {
            Some(answer) => answer,
            None if state.admitted.is_none() => EpochAdmission::Ready,
            None => {
                state
                    .violations
                    .push("second epoch requested: native recovery is unsupported".into());
                return EpochAdmission::Disabled(GapReason::ProducerLost);
            }
        };
        if answer == EpochAdmission::Ready {
            if state.admitted.is_some() {
                state
                    .violations
                    .push("second Ready: native recovery is unsupported".into());
                return EpochAdmission::Disabled(GapReason::ProducerLost);
            }
            state.admitted = Some(producer.to_string());
        }
        answer
    }

    fn try_close_epoch(&self, producer: &str, final_seq: Option<u64>) -> SubmitResult {
        let mut state = self.state.lock().unwrap();
        state.close_calls += 1;
        let last = state.frames.last().map(|frame| frame.seq);
        let problem = if state.admitted.as_deref() != Some(producer) {
            Some("close from a producer the sink never admitted")
        } else if !state.open.is_empty() {
            Some("close while payload streams are still open")
        } else if final_seq != last {
            Some("close final_seq differs from the last accepted frame")
        } else if state.closed.is_some() {
            Some("second close")
        } else {
            None
        };
        if let Some(problem) = problem {
            state.violations.push(problem.into());
            return SubmitResult::Disabled(GapReason::ProducerLost);
        }
        state.closed = Some(final_seq);
        SubmitResult::Queued
    }
}

// ---------------------------------------------------------------------------------------
// Test clock, fixture roots and configuration helpers
// ---------------------------------------------------------------------------------------

/// 2031-01-01T00:00:00Z: a wall time no fixture file content ever carries.
const CLOCK_BASE_SECS: u64 = 1_924_992_000;
const CLOCK_DAY: &str = "2031-01-01T";

/// A per-test observation clock: every reading advances `step` nanoseconds.
struct TestClock {
    next: Arc<AtomicU64>,
    step: u64,
}

impl NativeClock for TestClock {
    fn now(&mut self) -> ClockReading {
        let mono_ns = self.next.fetch_add(self.step, Ordering::Relaxed);
        ClockReading {
            wall: format_z(CLOCK_BASE_SECS + mono_ns / 1_000_000_000),
            mono_ns,
        }
    }
}

fn clock_with_step(step: u64) -> Box<TestClock> {
    Box::new(TestClock {
        next: Arc::new(AtomicU64::new(0)),
        step,
    })
}

fn clock() -> Box<TestClock> {
    clock_with_step(1_000)
}

/// An isolated temporary tree: `home` is the scoped store root and `outside` is a sibling
/// the scope must never reach.
struct Root {
    tmp: tempfile::TempDir,
    home: PathBuf,
    outside: PathBuf,
}

fn root() -> Root {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let outside = tmp.path().join("outside");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&outside).unwrap();
    Root { tmp, home, outside }
}

impl Root {
    fn path(&self, relative: &str) -> PathBuf {
        assert!(!relative.starts_with('/'));
        self.home.join(relative)
    }

    fn write(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    fn append(&self, relative: &str, bytes: &[u8]) {
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(self.path(relative))
            .unwrap();
        file.write_all(bytes).unwrap();
    }
}

fn source(
    name: &str,
    root: &Path,
    pattern: &str,
    form: NativeStoreForm,
    hint: Option<NativeHint>,
) -> NativeSourceSpec {
    NativeSourceSpec {
        name: name.to_string(),
        root: root.to_path_buf(),
        pattern: pattern.to_string(),
        form,
        hint,
        modified_since: None,
    }
}

/// A Codex-shaped thread ID (UUID v7) and the `sessions/YYYY/MM/DD` source built from
/// it. `root` stands for `CODEX_HOME/sessions`.
const THREAD: &str = "019a3c1e-7f52-7b21-9d4e-0c1a2b3c4d5e";

fn codex_thread(root: &Path, thread: &str) -> NativeSourceSpec {
    NativeSourceSpec {
        name: "codex-rollout".into(),
        root: root.to_path_buf(),
        pattern: format!("*/*/*/rollout-*-{thread}.jsonl"),
        form: NativeStoreForm::AppendOnly,
        hint: Some(NativeHint::new("codex-jsonl").unwrap()),
        modified_since: None,
    }
}

/// Restores a directory's mode when the test ends, even on failure, so the tempdir can go.
struct ModeGuard(PathBuf);

impl ModeGuard {
    fn lock(path: PathBuf) -> ModeGuard {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        ModeGuard(path)
    }
}

impl Drop for ModeGuard {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
    }
}

fn rollout(root: &Root, pattern: &str) -> NativeSourceSpec {
    source(
        "rollout",
        &root.home,
        pattern,
        NativeStoreForm::AppendOnly,
        Some(NativeHint::new("codex-jsonl").unwrap()),
    )
}

fn limits_with_poll_bytes(poll_bytes: u64) -> NativeCaptureLimits {
    NativeCaptureLimits {
        poll_bytes,
        ..NativeCaptureLimits::default()
    }
}

fn config(sources: Vec<NativeSourceSpec>, limits: NativeCaptureLimits) -> NativeCaptureConfig {
    NativeCaptureConfig { sources, limits }
}

fn start(
    sources: Vec<NativeSourceSpec>,
    limits: NativeCaptureLimits,
    sink: &Arc<CollectSink>,
) -> NativeCapture {
    NativeCapture::new(config(sources, limits), sink.clone(), clock())
        .expect("a valid native capture configuration")
}

fn generous() -> NativeFinishBudget {
    NativeFinishBudget {
        max_bytes: 64 << 20,
        max_duration_ns: u64::MAX,
    }
}

/// Deterministic, non-repeating-looking bytes for size-driven tests.
fn pattern_bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u32).wrapping_mul(31).wrapping_add(u32::from(seed)) as u8)
        .collect()
}

// ---------------------------------------------------------------------------------------
// Independent replay oracle
// ---------------------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct View {
    label: String,
    hint: Option<String>,
    open_seq: u64,
    first_offset: Option<u64>,
    next_offset: u64,
    data: Vec<u8>,
    bytes_frames: usize,
    gaps: Vec<(GapReason, Option<u64>)>,
    gaps_at_last_bytes: usize,
    first_bytes_seq: Option<u64>,
    first_gap_seq: Option<u64>,
    end: Option<(EndReason, u64)>,
}

/// Rebuild every stream from accepted frames, asserting as it goes: one producer-wide
/// sequence from zero across all files, no control stream, streams opened in increasing ID
/// order, native metadata only, bounded chunks, no hole or overlap within a loss-free
/// generation, nothing after End, and observation times from the test clock.
fn replay(frames: &[CaptureFrame]) -> BTreeMap<StreamKey, View> {
    let mut views: BTreeMap<StreamKey, View> = BTreeMap::new();
    let mut next_seq: BTreeMap<String, u64> = BTreeMap::new();
    let mut last_mono: BTreeMap<String, u64> = BTreeMap::new();
    for frame in frames {
        frame.validate().expect("every frame is within C1 bounds");
        let producer = frame.stream.producer.clone();
        assert!(is_capture_id(&producer));
        let expected = next_seq.entry(producer.clone()).or_insert(0);
        assert_eq!(
            frame.seq, *expected,
            "the producer sequence increases by one across all files"
        );
        *expected += 1;
        assert_ne!(
            frame.stream.stream, 0,
            "native capture never uses stream zero"
        );
        assert_eq!(
            frame.observed.clock_id, producer,
            "the producer names its clock"
        );
        assert!(
            frame.observed.wall.starts_with(CLOCK_DAY),
            "observation time comes from the injected clock, not from file content"
        );
        let mono = last_mono.entry(producer.clone()).or_insert(0);
        assert!(
            frame.observed.mono_ns >= *mono,
            "monotonic time never regresses"
        );
        *mono = frame.observed.mono_ns;

        match &frame.event {
            FrameEvent::Open { source, metadata } => {
                assert!(!views.contains_key(&frame.stream), "stream opened twice");
                assert!(
                    views
                        .keys()
                        .filter(|key| key.producer == producer)
                        .all(|key| key.stream < frame.stream.stream),
                    "stream IDs increase in Open order"
                );
                assert_eq!(*source, CaptureSource::Native);
                assert_eq!(
                    metadata.wire, None,
                    "native streams carry no transport identity"
                );
                assert_eq!(metadata.method, None);
                assert_eq!(metadata.host, None);
                assert_eq!(metadata.path, None);
                assert_eq!(metadata.status, None);
                assert_eq!(metadata.content_type, None);
                assert!(!metadata.label.starts_with('/'));
                views.insert(
                    frame.stream.clone(),
                    View {
                        label: metadata.label.clone(),
                        hint: metadata.protocol_hint.clone(),
                        open_seq: frame.seq,
                        ..View::default()
                    },
                );
            }
            FrameEvent::Bytes { offset, data } => {
                let view = views.get_mut(&frame.stream).expect("bytes after Open");
                assert!(view.end.is_none(), "bytes after End");
                assert!(!data.is_empty() && data.len() <= MAX_SEGMENT_BYTES);
                let after_gap = view.gaps.len() > view.gaps_at_last_bytes;
                if after_gap {
                    assert!(
                        *offset >= view.next_offset,
                        "a range after a gap never overlaps"
                    );
                } else {
                    assert_eq!(
                        *offset, view.next_offset,
                        "a loss-free generation has no hole or overlap"
                    );
                }
                view.first_offset.get_or_insert(*offset);
                view.first_bytes_seq.get_or_insert(frame.seq);
                view.next_offset = offset + data.len() as u64;
                view.data.extend_from_slice(data);
                view.bytes_frames += 1;
                view.gaps_at_last_bytes = view.gaps.len();
            }
            FrameEvent::Gap { reason, lost_bytes } => {
                let view = views.get_mut(&frame.stream).expect("gap after Open");
                assert!(view.end.is_none(), "gap after End");
                view.gaps.push((*reason, *lost_bytes));
                view.first_gap_seq.get_or_insert(frame.seq);
            }
            FrameEvent::End { reason } => {
                let view = views.get_mut(&frame.stream).expect("End after Open");
                assert!(view.end.is_none(), "End twice");
                view.end = Some((*reason, frame.seq));
            }
            FrameEvent::Connection { .. }
            | FrameEvent::Recovery { .. }
            | FrameEvent::Loss { .. } => {
                panic!("a native producer never emits wire controls")
            }
        }
    }
    views
}

fn by_label<'a>(views: &'a BTreeMap<StreamKey, View>, label: &str) -> (&'a StreamKey, &'a View) {
    views
        .iter()
        .find(|(_, view)| view.label == label)
        .unwrap_or_else(|| panic!("no stream labelled {label}; have {:?}", labels(views)))
}

fn labels(views: &BTreeMap<StreamKey, View>) -> Vec<String> {
    views.values().map(|view| view.label.clone()).collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

// ---------------------------------------------------------------------------------------
// Step 1: partial lines are bytes, segmented at the chunk bound
// ---------------------------------------------------------------------------------------

#[test]
fn large_partial_line_is_segmented_before_newline() {
    let root = root();
    let mut content = b"{\"type\":\"response_item\",\"payload\":\"".to_vec();
    content.extend(std::iter::repeat_n(b'a', 3 * MAX_SEGMENT_BYTES + 1234));
    root.write("sessions/rollout-a.jsonl", &content);
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/rollout-*.jsonl")],
        NativeCaptureLimits::default(),
        &sink,
    );

    let report = capture.poll();
    assert_eq!(report.admission, NativeAdmission::Ready);
    assert_eq!(report.bytes, content.len() as u64);
    let frames = sink.frames();
    let views = replay(&frames);
    let (_, view) = by_label(&views, "rollout:sessions/rollout-a.jsonl#g1");
    assert_eq!(
        view.data, content,
        "the unterminated line is retained byte for byte"
    );
    assert_eq!(view.first_offset, Some(0));
    assert!(
        view.bytes_frames >= 4,
        "more than three chunks' worth is segmented"
    );
    let mut expected_offset = 0u64;
    for frame in &frames {
        if let FrameEvent::Bytes { offset, data } = &frame.event {
            assert_eq!(
                *offset, expected_offset,
                "offsets cover the exact observed ranges"
            );
            assert!(data.len() <= MAX_SEGMENT_BYTES);
            assert_eq!(
                data[..],
                content[*offset as usize..*offset as usize + data.len()]
            );
            expected_offset += data.len() as u64;
        }
    }
    assert!(
        view.end.is_none(),
        "an unterminated live line is not a completed file"
    );

    // The newline arrives later: the same stream continues exactly where it stopped.
    root.append("sessions/rollout-a.jsonl", b"\"}\n{\"partial");
    capture.poll();
    let views = replay(&sink.frames());
    assert_eq!(views.len(), 1);
    let (_, view) = by_label(&views, "rollout:sessions/rollout-a.jsonl#g1");
    assert_eq!(
        view.data,
        fs::read(root.path("sessions/rollout-a.jsonl")).unwrap()
    );
    sink.assert_clean();
}

// ---------------------------------------------------------------------------------------
// Step 2: scoped discovery and bounded reads
// ---------------------------------------------------------------------------------------

#[test]
fn symlink_escape_is_not_read() {
    let root = root();
    let secret = b"OUTSIDE-SCOPE-SECRET";
    fs::write(root.outside.join("secret.jsonl"), secret).unwrap();
    fs::create_dir_all(root.outside.join("dir")).unwrap();
    fs::write(root.outside.join("dir/linked.jsonl"), secret).unwrap();
    root.write("projects/p/real.jsonl", b"{\"in\":\"scope\"}\n");
    std::os::unix::fs::symlink(
        root.outside.join("secret.jsonl"),
        root.path("projects/p/escape.jsonl"),
    )
    .unwrap();
    std::os::unix::fs::symlink(root.outside.join("dir"), root.path("projects/linkdir")).unwrap();

    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "projects/*/*.jsonl")],
        NativeCaptureLimits::default(),
        &sink,
    );
    let report = capture.poll();
    assert_eq!(
        report.refused_entries, 2,
        "a symlinked file and a symlinked directory are refused, not followed"
    );
    let views = replay(&sink.frames());
    assert_eq!(labels(&views), ["rollout:projects/p/real.jsonl#g1"]);

    // A tracked file swapped for a symlink out of scope is a changed source, never a read.
    fs::remove_file(root.path("projects/p/real.jsonl")).unwrap();
    std::os::unix::fs::symlink(
        root.outside.join("secret.jsonl"),
        root.path("projects/p/real.jsonl"),
    )
    .unwrap();
    capture.poll();
    capture.poll();
    let frames = sink.frames();
    for frame in &frames {
        if let FrameEvent::Bytes { data, .. } = &frame.event {
            assert!(!contains(data, secret), "out-of-scope bytes were read");
        }
    }
    let views = replay(&frames);
    let (_, view) = by_label(&views, "rollout:projects/p/real.jsonl#g1");
    assert_eq!(view.gaps, [(GapReason::SourceChanged, None)]);
    assert_eq!(
        view.end.map(|(reason, _)| reason),
        Some(EndReason::Interrupted)
    );
    assert_eq!(views.len(), 1, "the symlink never opens a new generation");
    sink.assert_clean();
}

#[test]
fn fifo_is_not_opened() {
    let root = root();
    root.write("sessions/ok.jsonl", b"{\"ok\":true}\n");
    let status = std::process::Command::new("mkfifo")
        .arg(root.path("sessions/pipe.jsonl"))
        .status()
        .unwrap();
    assert!(status.success());
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/*.jsonl")],
        NativeCaptureLimits::default(),
        &sink,
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let report = capture.poll();
        let _ = sender.send((report, capture));
    });
    let (report, _capture) = receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("a FIFO in scope must never block a poll");
    assert_eq!(report.refused_entries, 1);
    assert_eq!(
        labels(&replay(&sink.frames())),
        ["rollout:sessions/ok.jsonl#g1"]
    );
    sink.assert_clean();
}

#[test]
fn native_label_never_contains_absolute_home() {
    let root = root();
    root.write("projects/-ws-slug/sess-1.jsonl", b"{\"a\":1}\n");
    // A control byte and a backslash in a name are escaped, never passed into metadata.
    // (APFS refuses non-UTF-8 names outright, so the `\xNN` escape has no fixture here.)
    fs::write(
        root.path("projects/-ws-slug/we\u{1}ird\\.jsonl"),
        b"{\"b\":2}\n",
    )
    .unwrap();
    let sink = CollectSink::new();
    let mut capture = start(
        vec![source(
            "main",
            &root.home,
            "projects/-ws-slug/*.jsonl",
            NativeStoreForm::AppendOnly,
            Some(NativeHint::new("claude-code-jsonl").unwrap()),
        )],
        NativeCaptureLimits::default(),
        &sink,
    );
    capture.poll();
    let views = replay(&sink.frames());
    let home = root.home.to_string_lossy().to_string();
    let canonical = fs::canonicalize(&root.home)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let tmp = root.tmp.path().to_string_lossy().to_string();
    let mut seen = labels(&views);
    seen.sort();
    assert_eq!(
        seen,
        [
            "main:projects/-ws-slug/sess-1.jsonl#g1",
            "main:projects/-ws-slug/we\\u{1}ird\\\\.jsonl#g1",
        ]
    );
    for view in views.values() {
        assert!(!view.label.contains(&home));
        assert!(!view.label.contains(&canonical));
        assert!(!view.label.contains(&tmp));
        assert_eq!(view.hint.as_deref(), Some("claude-code-jsonl"));
    }
    sink.assert_clean();
}

#[test]
fn poll_budget_is_shared_across_files() {
    let root = root();
    let size = 30 * 1024;
    let names = ["a", "b", "c"];
    for (seed, name) in names.iter().enumerate() {
        root.write(
            &format!("sessions/{name}.jsonl"),
            &pattern_bytes(size, seed as u8),
        );
    }
    let budget = 12 * 1024u64;
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/*.jsonl")],
        limits_with_poll_bytes(budget),
        &sink,
    );

    let mut polls = 0;
    loop {
        let report = capture.poll();
        polls += 1;
        assert!(
            report.bytes <= budget,
            "one poll never exceeds its shared byte budget"
        );
        if polls == 3 {
            let views = replay(&sink.frames());
            for name in names {
                let (_, view) = by_label(&views, &format!("rollout:sessions/{name}.jsonl#g1"));
                assert!(!view.data.is_empty(), "{name} starved behind its siblings");
            }
        }
        if !report.more_work {
            break;
        }
        assert!(polls < 20, "the backlog drains");
    }
    assert!(polls >= 8, "90 KiB at 12 KiB per poll needs several polls");
    let views = replay(&sink.frames());
    for (seed, name) in names.iter().enumerate() {
        let (_, view) = by_label(&views, &format!("rollout:sessions/{name}.jsonl#g1"));
        assert_eq!(view.data, pattern_bytes(size, seed as u8));
    }
    sink.assert_clean();
}

#[test]
fn scope_is_only_the_positive_pattern() {
    let root = root();
    let other = b"OTHER-RUN-TRANSCRIPT";
    root.write("projects/-other-run/sess-9.jsonl", other);
    root.write("projects/-this-run/sess-9.jsonl", other);
    root.write("projects/-this-run/sess-1.jsonl", b"mine\n");
    root.write(
        "projects/-this-run/sess-1/subagents/agent-a.jsonl",
        b"sub\n",
    );
    let sink = CollectSink::new();
    let mut capture = start(
        vec![
            source(
                "main",
                &root.home,
                "projects/-this-run/sess-1.jsonl",
                NativeStoreForm::AppendOnly,
                Some(NativeHint::new("claude-code-jsonl").unwrap()),
            ),
            source(
                "main_subagent",
                &root.home,
                "projects/-this-run/sess-1/subagents/agent-*.jsonl",
                NativeStoreForm::AppendOnly,
                Some(NativeHint::new("claude-code-jsonl").unwrap()),
            ),
        ],
        NativeCaptureLimits::default(),
        &sink,
    );
    capture.poll();
    let frames = sink.frames();
    let mut seen = labels(&replay(&frames));
    seen.sort();
    assert_eq!(
        seen,
        [
            "main:projects/-this-run/sess-1.jsonl#g1",
            "main_subagent:projects/-this-run/sess-1/subagents/agent-a.jsonl#g1",
        ]
    );
    for frame in &frames {
        if let FrameEvent::Bytes { data, .. } = &frame.event {
            assert!(
                !contains(data, other),
                "a sibling session outside scope was read"
            );
        }
    }
    sink.assert_clean();
}

#[test]
fn shared_day_directory_opens_only_the_target() {
    let root = root();
    let other = b"OTHER-RUN-ROLLOUT";
    let target = format!("2026/09/27/rollout-2026-09-27T10-00-00-{THREAD}.jsonl");
    root.write(
        "2026/09/27/rollout-2026-09-27T09-00-00-019a3c1e-0000-7000-8000-00000000aaaa.jsonl",
        other,
    );
    root.write(
        &format!("2026/09/27/rollout-2026-09-27T11-00-00-{THREAD}-x.jsonl"),
        other,
    );
    root.write(
        &format!("2026/09/27/rollout-2026-09-27T12-00-00-{THREAD}.jsonl.bak"),
        other,
    );
    root.write(
        &format!("2026/09/27/xrollout-2026-09-27T13-00-00-{THREAD}.jsonl"),
        other,
    );
    root.write(
        "2026/09/26/rollout-2026-09-26T08-00-00-019a3c1e-0000-7000-8000-00000000bbbb.jsonl",
        other,
    );
    root.write(&target, b"{\"type\":\"session_meta\"}\n");
    let sink = CollectSink::new();
    let mut capture = start(
        vec![codex_thread(&root.home, THREAD)],
        NativeCaptureLimits::default(),
        &sink,
    );
    let report = capture.poll();
    assert_eq!((report.refused_entries, report.unavailable), (0, 0));
    let frames = sink.frames();
    let opens = frames
        .iter()
        .filter(|frame| matches!(frame.event, FrameEvent::Open { .. }))
        .count();
    assert_eq!(
        opens, 1,
        "only the run's own rollout opens in a shared day directory"
    );
    assert_eq!(
        labels(&replay(&frames)),
        [format!("codex-rollout:{target}#g1")]
    );
    for frame in &frames {
        if let FrameEvent::Bytes { data, .. } = &frame.event {
            assert!(!contains(data, other), "another run's rollout was read");
        }
    }
    sink.assert_clean();
}

#[test]
fn newest_day_is_reached_before_older_history_exhausts_the_cap() {
    let root = root();
    for day in 1..=12 {
        for n in 0..5 {
            root.write(
                &format!("2025/12/{day:02}/rollout-2025-12-{day:02}T00-00-00-old{n}.jsonl"),
                b"old\n",
            );
        }
    }
    let target = format!("2026/09/27/rollout-2026-09-27T10-00-00-{THREAD}.jsonl");
    root.write(&target, b"today\n");
    let sink = CollectSink::new();
    let mut capture = start(
        vec![codex_thread(&root.home, THREAD)],
        NativeCaptureLimits {
            max_dir_entries: 16,
            ..NativeCaptureLimits::default()
        },
        &sink,
    );
    let report = capture.poll();
    let views = replay(&sink.frames());
    let (_, view) = by_label(&views, &format!("codex-rollout:{target}#g1"));
    assert_eq!(
        view.data, b"today\n",
        "the newest day is walked before older history"
    );
    assert!(
        report.discovery_capped,
        "the unexamined older history is still reported"
    );
    assert!(capture.loss_report().discovery_capped);
    sink.assert_clean();
}

#[test]
fn finish_reports_discovery_it_could_not_complete() {
    // The run's rollout sits past the discovery cap and finish is the only discovery. A finish
    // with time walks a whole discovery cycle and finds it; one without time must not read
    // like "no file existed".
    let capped = root();
    for day in ["26", "27"] {
        for n in 0..6 {
            capped.write(
                &format!("2026/09/{day}/rollout-2026-09-{day}T00-00-00-other{n}.jsonl"),
                b"other\n",
            );
        }
    }
    capped.write(
        &format!("2026/09/25/rollout-2026-09-25T23-59-59-{THREAD}.jsonl"),
        b"x\n",
    );
    let sink = CollectSink::new();
    let mut capture = start(
        vec![codex_thread(&capped.home, THREAD)],
        NativeCaptureLimits {
            max_dir_entries: 8,
            ..NativeCaptureLimits::default()
        },
        &sink,
    );
    let finished = capture.finish(generous());
    assert!(finished.admitted && finished.closed);
    assert_eq!(
        finished.completed_streams, 1,
        "the finish passes on until its discovery cycle is whole"
    );
    assert!(!finished.discovery_capped);
    assert!(capture.loss_report().discovery_capped);
    sink.assert_clean();

    let sink = CollectSink::new();
    let mut capture = start(
        vec![codex_thread(&capped.home, THREAD)],
        NativeCaptureLimits {
            max_dir_entries: 8,
            ..NativeCaptureLimits::default()
        },
        &sink,
    );
    let finished = capture.finish(NativeFinishBudget {
        max_bytes: 64 << 20,
        max_duration_ns: 0,
    });
    assert!(finished.admitted && finished.closed);
    assert_eq!(finished.completed_streams + finished.interrupted_streams, 0);
    assert!(
        finished.discovery_capped,
        "a finish that ran out of time mid-cycle is reported"
    );
    assert!(capture.loss_report().discovery_capped);
    sink.assert_clean();

    // An unreadable directory in scope at finish is reported, not silently empty.
    let locked = root();
    locked.write(
        &format!("2026/09/27/rollout-2026-09-27T10-00-00-{THREAD}.jsonl"),
        b"x\n",
    );
    let _locked = ModeGuard::lock(locked.path("2026/09/27"));
    let sink = CollectSink::new();
    let mut capture = start(
        vec![codex_thread(&locked.home, THREAD)],
        NativeCaptureLimits::default(),
        &sink,
    );
    let finished = capture.finish(generous());
    assert!(finished.admitted && finished.closed);
    assert!(
        finished.unavailable >= 1,
        "an unreadable scope directory is reported"
    );
    assert!(capture.loss_report().unavailable >= 1);
    sink.assert_clean();

    // A refusal seen in one poll stays in the lifetime facts after it disappears, and a
    // refusal first seen by a finish pass reaches the finish report.
    let linked = root();
    let link = linked.path(&format!(
        "2026/09/27/rollout-2026-09-27T10-00-00-{THREAD}.jsonl"
    ));
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(linked.outside.join("elsewhere.jsonl"), &link).unwrap();
    let sink = CollectSink::new();
    let mut capture = start(
        vec![codex_thread(&linked.home, THREAD)],
        NativeCaptureLimits::default(),
        &sink,
    );
    assert_eq!(capture.poll().refused_entries, 1);
    fs::remove_file(&link).unwrap();
    assert_eq!(capture.poll().refused_entries, 0);
    assert_eq!(
        capture.loss_report().refused_entries,
        1,
        "lifetime facts are sticky"
    );
    std::os::unix::fs::symlink(linked.outside.join("elsewhere.jsonl"), &link).unwrap();
    let finished = capture.finish(generous());
    assert_eq!(
        finished.refused_entries, 2,
        "the finish pass is folded into the facts"
    );
    assert_eq!(finished.completed_streams + finished.interrupted_streams, 0);
    sink.assert_clean();
}

#[test]
fn a_store_that_does_not_exist_yet_is_captured_once_it_appears() {
    let root = root();
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "projects/-ws/sess-1.jsonl")],
        NativeCaptureLimits::default(),
        &sink,
    );
    let report = capture.poll();
    assert_eq!(report.admission, NativeAdmission::Ready);
    assert_eq!(report.frames, 0);
    root.write("projects/-ws/sess-1.jsonl", b"late\n");
    capture.poll();
    let views = replay(&sink.frames());
    let (_, view) = by_label(&views, "rollout:projects/-ws/sess-1.jsonl#g1");
    assert_eq!(view.data, b"late\n");
    sink.assert_clean();
}

#[test]
fn invalid_configuration_is_refused() {
    let root = root();
    let sink = CollectSink::new();
    let attempt = |sources: Vec<NativeSourceSpec>, limits: NativeCaptureLimits| {
        NativeCapture::new(config(sources, limits), sink.clone(), clock()).map(|_| ())
    };
    let defaults = NativeCaptureLimits::default;
    for pattern in [
        "",
        "/abs/x.jsonl",
        "../x.jsonl",
        "a/../b",
        "a/**/b",
        "a//b",
        "a/",
        "./a",
        "rollout-{id}.jsonl",
        "a/{x}/b",
        "a}b",
    ] {
        assert!(
            matches!(
                attempt(vec![rollout(&root, pattern)], defaults()),
                Err(NativeCaptureError::Config(_))
            ),
            "pattern {pattern:?} must be refused"
        );
    }
    let mut relative = rollout(&root, "x.jsonl");
    relative.root = PathBuf::from("relative/home");
    assert!(matches!(
        attempt(vec![relative], defaults()),
        Err(NativeCaptureError::Config(_))
    ));
    let mut missing = rollout(&root, "x.jsonl");
    missing.root = root.home.join("absent");
    assert!(matches!(
        attempt(vec![missing], defaults()),
        Err(NativeCaptureError::Root(_))
    ));
    assert!(matches!(
        attempt(
            vec![rollout(&root, "a.jsonl"), rollout(&root, "b.jsonl")],
            defaults()
        ),
        Err(NativeCaptureError::Config(_))
    ));
    assert!(matches!(
        attempt(Vec::new(), defaults()),
        Err(NativeCaptureError::Config(_))
    ));
    let many: Vec<_> = (0..9)
        .map(|i| {
            source(
                &format!("s{i}"),
                &root.home,
                &format!("f{i}"),
                NativeStoreForm::AppendOnly,
                None,
            )
        })
        .collect();
    assert!(matches!(
        attempt(many, defaults()),
        Err(NativeCaptureError::Config(_))
    ));
    let bad_name = source("a:b", &root.home, "x", NativeStoreForm::AppendOnly, None);
    assert!(matches!(
        attempt(vec![bad_name], defaults()),
        Err(NativeCaptureError::Config(_))
    ));
    let hinted_snapshot = source(
        "spill",
        &root.home,
        "tool-results/*",
        NativeStoreForm::Snapshot,
        Some(NativeHint::new("claude-code-jsonl").unwrap()),
    );
    assert!(matches!(
        attempt(vec![hinted_snapshot], defaults()),
        Err(NativeCaptureError::Config(_))
    ));
    for limits in [
        limits_with_poll_bytes(0),
        NativeCaptureLimits {
            max_open_streams: MAX_OPEN_STREAMS + 1,
            ..defaults()
        },
        NativeCaptureLimits {
            max_open_streams: 0,
            ..defaults()
        },
        NativeCaptureLimits {
            max_files: 0,
            ..defaults()
        },
        NativeCaptureLimits {
            max_dir_entries: 0,
            ..defaults()
        },
    ] {
        assert!(matches!(
            attempt(vec![rollout(&root, "x.jsonl")], limits),
            Err(NativeCaptureError::Config(_))
        ));
    }
}

// ---------------------------------------------------------------------------------------
// Step 3: append-only streaming without a logical-line cutoff
// ---------------------------------------------------------------------------------------

#[test]
fn binary_and_non_json_native_bytes_survive() {
    let root = root();
    let mut content =
        b"\x00\xff\xfe not json at all\r\n{\"timestamp\":\"2001-01-01T00:00:00Z\"}\n".to_vec();
    content.extend_from_slice(b"\xc3\x28 invalid utf-8 \x80\x80\n");
    content.extend((0..=255u8).cycle().take(3000));
    root.write("sessions/rollout-bin.jsonl", &content);
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/rollout-*.jsonl")],
        NativeCaptureLimits::default(),
        &sink,
    );
    capture.poll();
    let finished = capture.finish(generous());
    assert!(finished.closed);
    let views = replay(&sink.frames());
    let (_, view) = by_label(&views, "rollout:sessions/rollout-bin.jsonl#g1");
    assert_eq!(
        view.data, content,
        "invalid UTF-8, NUL and non-JSON bytes are unmodified"
    );
    assert_eq!(view.hint.as_deref(), Some("codex-jsonl"));
    assert_eq!(
        view.end.map(|(reason, _)| reason),
        Some(EndReason::Complete)
    );
    sink.assert_clean();
}

#[test]
fn live_eof_is_not_complete() {
    let root = root();
    root.write("sessions/rollout-live.jsonl", b"line-1\n");
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/rollout-*.jsonl")],
        NativeCaptureLimits::default(),
        &sink,
    );
    capture.poll();
    let after_first = sink.frames().len();
    for _ in 0..5 {
        let report = capture.poll();
        assert_eq!(report.frames, 0, "a sparse poll emits nothing");
        assert!(!report.more_work);
    }
    assert_eq!(sink.frames().len(), after_first);
    assert!(
        sink.frames()
            .iter()
            .all(|frame| !matches!(frame.event, FrameEvent::End { .. })),
        "EOF of a live file is not completion"
    );
    assert_eq!(capture.loss_report().open_streams, 1);

    root.append("sessions/rollout-live.jsonl", b"line-2\n");
    capture.poll();
    let views = replay(&sink.frames());
    assert_eq!(views.len(), 1, "the same stream continues");
    let (_, view) = by_label(&views, "rollout:sessions/rollout-live.jsonl#g1");
    assert_eq!(view.data, b"line-1\nline-2\n");
    assert!(view.end.is_none());
    sink.assert_clean();
}

// ---------------------------------------------------------------------------------------
// Step 4: replacement, truncation and snapshots
// ---------------------------------------------------------------------------------------

#[test]
fn rotation_preserves_old_and_new_generations() {
    let root = root();
    root.write("sessions/current.jsonl", b"old-1\nold-2\n");
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/current.jsonl")],
        NativeCaptureLimits::default(),
        &sink,
    );
    capture.poll();
    fs::create_dir_all(root.path("archive")).unwrap();
    fs::rename(
        root.path("sessions/current.jsonl"),
        root.path("archive/old.jsonl"),
    )
    .unwrap();
    // Longer than the old reader position, so only the changed identity reveals the swap.
    let replacement = b"new-1\nnew-2\nnew-3\nnew-4\n";
    root.write("sessions/current.jsonl", replacement);
    capture.poll();

    let views = replay(&sink.frames());
    let (old_key, old) = by_label(&views, "rollout:sessions/current.jsonl#g1");
    let (new_key, new) = by_label(&views, "rollout:sessions/current.jsonl#g2");
    assert_ne!(old_key, new_key);
    assert_eq!(
        old.data, b"old-1\nold-2\n",
        "the old generation keeps its bytes"
    );
    assert_eq!(old.gaps, [(GapReason::SourceChanged, None)]);
    let (old_end, old_end_seq) = old.end.unwrap();
    assert_eq!(old_end, EndReason::Interrupted);
    assert!(
        old_end_seq < new.open_seq,
        "the old stream ends before the new generation opens"
    );
    assert_eq!(new.first_offset, Some(0));
    assert_eq!(new.data, replacement);
    assert_eq!(capture.loss_report().source_changes, 1);
    sink.assert_clean();
}

#[test]
fn truncate_then_append_never_reuses_old_offsets() {
    let root = root();
    root.write("sessions/rollout-t.jsonl", &[b'A'; 100]);
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/rollout-*.jsonl")],
        NativeCaptureLimits::default(),
        &sink,
    );
    capture.poll();
    fs::OpenOptions::new()
        .write(true)
        .open(root.path("sessions/rollout-t.jsonl"))
        .unwrap()
        .set_len(0)
        .unwrap();
    root.append("sessions/rollout-t.jsonl", &[b'B'; 20]);
    capture.poll();

    let views = replay(&sink.frames());
    let (old_key, old) = by_label(&views, "rollout:sessions/rollout-t.jsonl#g1");
    let (new_key, new) = by_label(&views, "rollout:sessions/rollout-t.jsonl#g2");
    assert_ne!(
        old_key, new_key,
        "a truncated file never continues its old stream"
    );
    assert_eq!(old.data, [b'A'; 100]);
    assert_eq!(old.gaps, [(GapReason::SourceChanged, None)]);
    assert_eq!(
        old.end.map(|(reason, _)| reason),
        Some(EndReason::Interrupted)
    );
    assert!(old.end.unwrap().1 < new.open_seq);
    assert_eq!(new.first_offset, Some(0));
    assert_eq!(new.data, [b'B'; 20]);
    sink.assert_clean();
}

#[test]
fn snapshot_larger_than_chunk_completes() {
    let root = root();
    let content = pattern_bytes(3 * MAX_SEGMENT_BYTES + 7, 9);
    root.write("sess/tool-results/big.txt", &content);
    let sink = CollectSink::new();
    let mut capture = start(
        vec![source(
            "spill",
            &root.home,
            "sess/tool-results/*",
            NativeStoreForm::Snapshot,
            None,
        )],
        limits_with_poll_bytes(MAX_SEGMENT_BYTES as u64),
        &sink,
    );
    for _ in 0..3 {
        let report = capture.poll();
        assert!(report.more_work);
        let views = replay(&sink.frames());
        assert!(
            by_label(&views, "spill:sess/tool-results/big.txt#g1")
                .1
                .end
                .is_none()
        );
    }
    capture.poll();
    let views = replay(&sink.frames());
    let (_, first) = by_label(&views, "spill:sess/tool-results/big.txt#g1");
    assert_eq!(first.data, content);
    assert_eq!(first.hint, None, "no parser is inferred from a filename");
    assert_eq!(
        first.end.map(|(reason, _)| reason),
        Some(EndReason::Complete)
    );
    assert_eq!(capture.loss_report().open_streams, 0);

    // A completed snapshot that changes is a new generation, not silently immutable.
    capture.poll();
    assert_eq!(
        replay(&sink.frames()).len(),
        1,
        "an unchanged snapshot is not re-read"
    );
    root.append("sess/tool-results/big.txt", b"-changed");
    for _ in 0..5 {
        capture.poll();
    }
    let views = replay(&sink.frames());
    let (_, second) = by_label(&views, "spill:sess/tool-results/big.txt#g2");
    assert_eq!(
        second.data,
        fs::read(root.path("sess/tool-results/big.txt")).unwrap()
    );
    assert_eq!(
        second.end.map(|(reason, _)| reason),
        Some(EndReason::Complete)
    );

    // A snapshot that changes while it is being read is interrupted and read again.
    let torn = pattern_bytes(2 * MAX_SEGMENT_BYTES, 3);
    root.write("sess/tool-results/torn.txt", &torn);
    capture.poll();
    root.append("sess/tool-results/torn.txt", b"+tail");
    for _ in 0..5 {
        capture.poll();
    }
    let views = replay(&sink.frames());
    let (_, torn_first) = by_label(&views, "spill:sess/tool-results/torn.txt#g1");
    assert_eq!(torn_first.gaps, [(GapReason::SourceChanged, None)]);
    assert_eq!(
        torn_first.end.map(|(reason, _)| reason),
        Some(EndReason::Interrupted)
    );
    let (_, torn_second) = by_label(&views, "spill:sess/tool-results/torn.txt#g2");
    assert_eq!(
        torn_second.data,
        fs::read(root.path("sess/tool-results/torn.txt")).unwrap()
    );
    assert_eq!(
        torn_second.end.map(|(reason, _)| reason),
        Some(EndReason::Complete)
    );
    sink.assert_clean();
}

#[test]
fn native_hints_are_validated_open_values_and_stores_have_explicit_forms() {
    assert!(NativeHint::new("").is_err());
    assert!(NativeHint::new("UPPER").is_err());
    assert!(NativeHint::new(format!("a{}", "b".repeat(64))).is_err());
    assert_eq!(
        NativeHint::new("codex-jsonl").unwrap().as_str(),
        "codex-jsonl"
    );
    assert_eq!(
        NativeHint::new("future-format.v7").unwrap().as_str(),
        "future-format.v7"
    );
    assert_eq!(
        NativeHint::new("claude-code-jsonl").unwrap().as_str(),
        "claude-code-jsonl"
    );
    assert_ne!(NativeStoreForm::AppendOnly, NativeStoreForm::Snapshot);

    // A Codex-looking filename under an unknown schema profile carries no hint.
    let root = root();
    root.write(
        "sessions/rollout-2026-x.jsonl",
        b"{\"type\":\"session_meta\"}\n",
    );
    let sink = CollectSink::new();
    let mut capture = start(
        vec![source(
            "rollout",
            &root.home,
            "sessions/*.jsonl",
            NativeStoreForm::AppendOnly,
            None,
        )],
        NativeCaptureLimits::default(),
        &sink,
    );
    capture.poll();
    let views = replay(&sink.frames());
    assert_eq!(
        by_label(&views, "rollout:sessions/rollout-2026-x.jsonl#g1")
            .1
            .hint,
        None
    );
    sink.assert_clean();
}

// ---------------------------------------------------------------------------------------
// Step 5: admission, loss latching and restart
// ---------------------------------------------------------------------------------------

#[test]
fn admission_precedes_the_first_frame_and_ids_are_fresh() {
    let root = root();
    root.write("sessions/rollout-1.jsonl", b"x\n");
    root.write("sessions/rollout-2.jsonl", b"y\n");
    let sink = CollectSink::scripted(&[
        EpochAdmission::Pending,
        EpochAdmission::Pending,
        EpochAdmission::RetryFresh,
        EpochAdmission::Ready,
    ]);
    let mut capture = start(
        vec![rollout(&root, "sessions/rollout-*.jsonl")],
        NativeCaptureLimits::default(),
        &sink,
    );
    for _ in 0..3 {
        let report = capture.poll();
        assert_eq!(report.admission, NativeAdmission::Pending);
        assert_eq!(report.frames, 0);
    }
    assert_eq!(sink.submit_calls(), 0, "no frame before Ready");
    assert_eq!(capture.poll().admission, NativeAdmission::Ready);

    let asked = sink.asked();
    assert_eq!(asked.len(), 4);
    assert!(asked.iter().all(|id| is_capture_id(id)));
    assert_eq!(
        asked[0], asked[1],
        "a pending ID is kept across polls of one attempt"
    );
    assert_eq!(asked[1], asked[2]);
    assert_ne!(asked[2], asked[3], "RetryFresh retires the ID");
    assert_eq!(capture.producer(), asked[3]);
    let frames = sink.frames();
    assert_eq!(frames[0].seq, 0);
    assert!(frames.iter().all(|frame| frame.stream.producer == asked[3]));
    let streams: BTreeSet<u64> = frames.iter().map(|frame| frame.stream.stream).collect();
    assert_eq!(
        streams,
        BTreeSet::from([1, 2]),
        "payload streams start at one"
    );
    replay(&frames);
    sink.assert_clean();

    // A terminal admission answer latches the loss and is never asked again.
    let refused = CollectSink::scripted(&[EpochAdmission::Disabled(GapReason::StorageFull)]);
    let mut capture = start(
        vec![rollout(&root, "sessions/rollout-*.jsonl")],
        NativeCaptureLimits::default(),
        &refused,
    );
    for _ in 0..3 {
        assert_eq!(
            capture.poll().admission,
            NativeAdmission::Lost(GapReason::StorageFull)
        );
    }
    assert_eq!(refused.asked().len(), 1);
    assert_eq!(refused.submit_calls(), 0);
    let finished = capture.finish(generous());
    assert!(!finished.closed);
    assert_eq!(finished.latched, Some(GapReason::StorageFull));
    assert_eq!(refused.close_calls(), 0);
}

#[test]
fn disabled_sink_does_not_spin_or_advance_durable_claims() {
    let root = root();
    let content = pattern_bytes(10 * MAX_SEGMENT_BYTES, 1);
    root.write("sessions/rollout-q.jsonl", &content);
    let sink = CollectSink::with_ceiling(3 * MAX_SEGMENT_BYTES);
    let mut capture = start(
        vec![rollout(&root, "sessions/rollout-*.jsonl")],
        NativeCaptureLimits::default(),
        &sink,
    );
    let report = capture.poll();
    assert_eq!(
        report.admission,
        NativeAdmission::Lost(GapReason::QueueFull)
    );
    let calls = sink.submit_calls();
    let accepted = sink.frames();
    let last_seq = accepted.last().map(|frame| frame.seq);
    let views = replay(&accepted);
    let (_, view) = by_label(&views, "rollout:sessions/rollout-q.jsonl#g1");
    assert_eq!(
        view.data,
        content[..2 * MAX_SEGMENT_BYTES],
        "an accepted prefix only"
    );

    root.append("sessions/rollout-q.jsonl", b"more after the loss\n");
    for _ in 0..5 {
        let report = capture.poll();
        assert_eq!(
            report.admission,
            NativeAdmission::Lost(GapReason::QueueFull)
        );
        assert_eq!((report.frames, report.bytes), (0, 0));
        assert!(!report.more_work, "a latched loss is not more work");
    }
    assert_eq!(
        sink.submit_calls(),
        calls,
        "the rejected frame is never retried"
    );
    assert_eq!(sink.asked().len(), 1, "no recovery epoch is requested");

    let loss = capture.loss_report();
    assert_eq!(loss.latched, Some(GapReason::QueueFull));
    assert_eq!(
        loss.last_queued_seq, last_seq,
        "claims stop at the last queued frame"
    );

    let finished = capture.finish(generous());
    assert!(!finished.closed, "a lost producer never closes cleanly");
    assert_eq!(finished.latched, Some(GapReason::QueueFull));
    assert_eq!(finished.final_seq, last_seq);
    assert_eq!(sink.close_calls(), 0);
    assert_eq!(sink.submit_calls(), calls);
    sink.assert_clean();
}

#[test]
fn a_dropped_adapter_never_manufactures_a_close() {
    let root = root();
    let first = pattern_bytes(100 * 1024, 4);
    root.write("sessions/rollout-1.jsonl", &first);
    let sources = vec![rollout(&root, "sessions/rollout-*.jsonl")];

    let before = CollectSink::new();
    let mut crashed = start(sources, NativeCaptureLimits::default(), &before);
    crashed.poll();
    let views = replay(&before.frames());
    let (_, view) = by_label(&views, "rollout:sessions/rollout-1.jsonl#g1");
    assert_eq!(
        view.data, first,
        "everything was queued: volatile, not durable"
    );
    drop(crashed);
    assert_eq!(
        before.close_calls(),
        0,
        "a crash never manufactures a clean close"
    );
}

#[test]
fn stream_file_and_discovery_limits_are_explicit() {
    let root = root();
    for name in ["a", "b", "c", "d", "e"] {
        root.write(&format!("sessions/{name}.jsonl"), b"x\n");
    }
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/*.jsonl")],
        NativeCaptureLimits {
            max_open_streams: 2,
            max_files: 3,
            ..NativeCaptureLimits::default()
        },
        &sink,
    );
    let report = capture.poll();
    assert_eq!(
        report.waiting_files, 1,
        "a tracked file waits for a stream slot"
    );
    assert_eq!(
        report.untracked_files, 2,
        "files beyond the file bound are reported"
    );
    let loss = capture.loss_report();
    assert!(loss.file_limit_reached);
    assert_eq!((loss.tracked_files, loss.open_streams), (3, 2));
    let mut seen = labels(&replay(&sink.frames()));
    seen.sort();
    assert_eq!(
        seen,
        ["rollout:sessions/a.jsonl#g1", "rollout:sessions/b.jsonl#g1"]
    );
    let finished = capture.finish(generous());
    assert_eq!(finished.completed_streams, 2);
    assert_eq!(finished.uncaptured_files, 1);
    assert!(finished.closed);
    sink.assert_clean();

    let sink = CollectSink::new();
    let mut capped = start(
        vec![rollout(&root, "sessions/*.jsonl")],
        NativeCaptureLimits {
            max_dir_entries: 2,
            ..NativeCaptureLimits::default()
        },
        &sink,
    );
    let report = capped.poll();
    assert!(report.discovery_capped);
    assert!(capped.loss_report().tracked_files <= 2);
    sink.assert_clean();
}

// ---------------------------------------------------------------------------------------
// Step 6: honest finish
// ---------------------------------------------------------------------------------------

#[test]
fn finish_preserves_partial_terminal_line() {
    let root = root();
    let content = b"{\"a\":1}\n{\"b\":".to_vec();
    root.write("sessions/rollout-p.jsonl", &content);
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/rollout-*.jsonl")],
        NativeCaptureLimits::default(),
        &sink,
    );
    let finished = capture.finish(generous());
    assert!(finished.admitted);
    assert!(finished.closed);
    assert_eq!(finished.completed_streams, 1);
    assert_eq!(finished.interrupted_streams, 0);
    let frames = sink.frames();
    assert_eq!(finished.final_seq, frames.last().map(|frame| frame.seq));
    assert_eq!(sink.closed(), Some(finished.final_seq));
    let views = replay(&frames);
    let (_, view) = by_label(&views, "rollout:sessions/rollout-p.jsonl#g1");
    assert_eq!(
        view.data, content,
        "the partial terminal line is retained as bytes"
    );
    assert_eq!(
        view.end.map(|(reason, _)| reason),
        Some(EndReason::Complete)
    );

    // Finish is once-only; later calls observe the committed result.
    root.append("sessions/rollout-p.jsonl", b"1}\n");
    assert_eq!(capture.finish(generous()), finished);
    assert_eq!(capture.poll().admission, NativeAdmission::Closed);
    assert_eq!(sink.frames().len(), frames.len());
    sink.assert_clean();
}

#[test]
fn finish_budget_reports_unknown_tail() {
    let root = root();
    let content = pattern_bytes(5 * MAX_SEGMENT_BYTES, 7);
    root.write("sessions/rollout-u.jsonl", &content);
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/rollout-*.jsonl")],
        NativeCaptureLimits::default(),
        &sink,
    );
    let budget = MAX_SEGMENT_BYTES as u64 + 10;
    let finished = capture.finish(NativeFinishBudget {
        max_bytes: budget,
        max_duration_ns: u64::MAX,
    });
    assert_eq!(finished.bytes_drained, budget);
    assert_eq!(
        (finished.completed_streams, finished.interrupted_streams),
        (0, 1)
    );
    assert!(
        finished.closed,
        "an interrupted stream still ends before the ordered close"
    );
    let views = replay(&sink.frames());
    let (_, view) = by_label(&views, "rollout:sessions/rollout-u.jsonl#g1");
    assert_eq!(view.data, content[..budget as usize]);
    assert_eq!(
        view.gaps,
        [(GapReason::ShutdownDeadline, None)],
        "the tail is unknown"
    );
    assert_eq!(
        view.end.map(|(reason, _)| reason),
        Some(EndReason::Interrupted)
    );
    sink.assert_clean();

    // The time budget is measured on the injected clock: one bounded pass, then stop.
    let sink = CollectSink::new();
    let mut capture = NativeCapture::new(
        config(
            vec![rollout(&root, "sessions/rollout-*.jsonl")],
            limits_with_poll_bytes(MAX_SEGMENT_BYTES as u64),
        ),
        sink.clone(),
        clock_with_step(1_000_000_000),
    )
    .unwrap();
    let finished = capture.finish(NativeFinishBudget {
        max_bytes: u64::MAX,
        max_duration_ns: 0,
    });
    assert_eq!(finished.bytes_drained, MAX_SEGMENT_BYTES as u64);
    assert_eq!(finished.interrupted_streams, 1);
    let views = replay(&sink.frames());
    let (_, view) = by_label(&views, "rollout:sessions/rollout-u.jsonl#g1");
    assert_eq!(view.gaps, [(GapReason::ShutdownDeadline, None)]);
    sink.assert_clean();
}

#[test]
fn default_limits_capture_a_realistic_rollout() {
    let limits = NativeCaptureLimits::default();
    assert_eq!(limits.poll_bytes, 16 * MAX_SEGMENT_BYTES as u64);
    assert!(limits.max_open_streams <= MAX_OPEN_STREAMS);

    let root = root();
    let mut content = Vec::new();
    let mut line = 0u64;
    while content.len() < 3 << 20 {
        let text = "x".repeat((line as usize * 37) % 4000);
        content.extend_from_slice(
            format!(
                "{{\"timestamp\":\"2001-01-01T00:00:00Z\",\"type\":\"response_item\",\"payload\":{{\"i\":{line},\"text\":\"{text}\"}}}}\n"
            )
            .as_bytes(),
        );
        if line == 100 {
            content.extend_from_slice(b"{\"type\":\"big\",\"text\":\"");
            content.extend(std::iter::repeat_n(b'y', 200 * 1024));
            content.extend_from_slice(b"\"}\n");
        }
        line += 1;
    }
    content.extend_from_slice(b"{\"type\":\"unterminated");
    root.write(
        "sessions/2026/09/27/rollout-2026-09-27T00-00-00-thread.jsonl",
        &content,
    );
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/*/*/*/rollout-*-thread.jsonl")],
        limits,
        &sink,
    );
    let mut polls = 0;
    loop {
        let report = capture.poll();
        polls += 1;
        assert!(report.bytes <= limits.poll_bytes);
        if !report.more_work {
            break;
        }
        assert!(polls < 64);
    }
    assert!(
        polls >= 3,
        "a 3 MiB backlog is caught up over several polls"
    );
    let finished = capture.finish(generous());
    assert!(finished.closed);
    assert_eq!(finished.completed_streams, 1);
    let views = replay(&sink.frames());
    let (_, view) = by_label(
        &views,
        "rollout:sessions/2026/09/27/rollout-2026-09-27T00-00-00-thread.jsonl#g1",
    );
    assert_eq!(view.data, content);
    assert_eq!(
        view.end.map(|(reason, _)| reason),
        Some(EndReason::Complete)
    );
    sink.assert_clean();
}

#[test]
fn the_adapter_moves_to_a_poll_thread() {
    fn assert_send<T: Send>() {}
    assert_send::<NativeCapture>();
}

// ---------------------------------------------------------------------------------------
// A Hermes-shaped `sessions` directory as a Snapshot source, one session file named in the
// pattern. The spec is built by hand; what a shipped connector resolves to is pinned in the
// unit tests of `runtime/capture/tests.rs`. Every byte is synthetic, and every directory here
// is a private temporary one: nothing resolves a real `HOME` or any `.hermes`.
// ---------------------------------------------------------------------------------------

const HERMES_SESSION: &str = "20260930_141500_a1b2c3";
const HERMES_OTHER: &str = "20260930_141400_ffeedd";

fn hermes_session(sessions: &Path, id: &str) -> NativeSourceSpec {
    NativeSourceSpec {
        name: "session".into(),
        root: sessions.to_path_buf(),
        pattern: format!("session_{id}.json"),
        form: NativeStoreForm::Snapshot,
        hint: None,
        modified_since: None,
    }
}

/// Only the run's own `session_<id>.json` is read: not another run's session, and never a
/// `request_dump_*` file, which can hold a masked provider key (D41).
#[test]
fn hermes_scope_reads_its_own_snapshot_and_never_a_foreign_or_request_dump_file() {
    let root = root();
    let own = pattern_bytes(1234, 1);
    root.write(&format!("sessions/session_{HERMES_SESSION}.json"), &own);
    root.write(
        &format!("sessions/session_{HERMES_OTHER}.json"),
        b"FOREIGN-SESSION",
    );
    root.write(
        &format!("sessions/request_dump_{HERMES_SESSION}_20260930_141501.json"),
        b"REQUEST-DUMP-WITH-KEY-FRAGMENT",
    );
    root.write("sessions/sessions.json", b"INDEX");
    let sink = CollectSink::new();
    let mut capture = start(
        vec![hermes_session(&root.path("sessions"), HERMES_SESSION)],
        NativeCaptureLimits::default(),
        &sink,
    );
    let finished = capture.finish(generous());
    assert!(finished.admitted && finished.closed);
    assert_eq!(finished.interrupted_streams, 0);
    let frames = sink.frames();
    for frame in &frames {
        if let FrameEvent::Bytes { data, .. } = &frame.event {
            for forbidden in [
                &b"FOREIGN-SESSION"[..],
                b"REQUEST-DUMP-WITH-KEY-FRAGMENT",
                b"INDEX",
            ] {
                assert!(!contains(data, forbidden), "an out-of-scope file was read");
            }
        }
    }
    let views = replay(&frames);
    assert_eq!(
        labels(&views),
        [format!("session:session_{HERMES_SESSION}.json#g1")]
    );
    let (_, view) = by_label(&views, &format!("session:session_{HERMES_SESSION}.json#g1"));
    assert_eq!(view.data, own);
    assert_eq!(
        view.hint, None,
        "no parser hint until the verifier registers one"
    );
    assert_eq!(
        view.end.map(|(reason, _)| reason),
        Some(EndReason::Complete)
    );
    sink.assert_clean();
}

/// Hermes rewrites the whole snapshot through temp-file-and-rename. A rewrite that lands
/// while the snapshot is being read interrupts that generation with `SourceChanged` and the
/// new bytes are read whole as the next generation.
#[test]
fn hermes_atomic_rewrite_during_a_read_is_a_source_changed_gap() {
    let root = root();
    let name = format!("sessions/session_{HERMES_SESSION}.json");
    root.write(&name, &pattern_bytes(2 * MAX_SEGMENT_BYTES, 5));
    let sink = CollectSink::new();
    let mut capture = start(
        vec![hermes_session(&root.path("sessions"), HERMES_SESSION)],
        limits_with_poll_bytes(MAX_SEGMENT_BYTES as u64),
        &sink,
    );
    capture.poll();
    let replacement = pattern_bytes(MAX_SEGMENT_BYTES + 9, 6);
    let temp = root.path("sessions/.session.tmp");
    fs::write(&temp, &replacement).unwrap();
    fs::rename(&temp, root.path(&name)).unwrap();
    for _ in 0..6 {
        capture.poll();
    }
    let views = replay(&sink.frames());
    let (_, first) = by_label(&views, &format!("session:session_{HERMES_SESSION}.json#g1"));
    assert_eq!(first.gaps, [(GapReason::SourceChanged, None)]);
    assert_eq!(
        first.end.map(|(reason, _)| reason),
        Some(EndReason::Interrupted)
    );
    let (_, second) = by_label(&views, &format!("session:session_{HERMES_SESSION}.json#g2"));
    assert_eq!(second.data, replacement);
    assert_eq!(
        second.end.map(|(reason, _)| reason),
        Some(EndReason::Complete)
    );
    sink.assert_clean();
}

/// Hermes lane B, S7: a snapshot read and ended `Complete` by a pass (a poll, or the first pass
/// of the finish itself) is a completed stream at the finish boundary. The owner's rule that a
/// required native source which completed nothing is a gap reads this count, so a snapshot that
/// was captured whole must not read as one that never was.
#[test]
fn hermes_snapshot_completed_by_a_pass_counts_as_completed_at_the_finish() {
    let root = root();
    let own = pattern_bytes(2048, 3);
    root.write(&format!("sessions/session_{HERMES_SESSION}.json"), &own);
    for polled_first in [false, true] {
        let sink = CollectSink::new();
        let mut capture = start(
            vec![hermes_session(&root.path("sessions"), HERMES_SESSION)],
            NativeCaptureLimits::default(),
            &sink,
        );
        if polled_first {
            capture.poll();
        }
        let finished = capture.finish(generous());
        assert_eq!(
            (finished.completed_streams, finished.interrupted_streams),
            (1, 0),
            "polled first: {polled_first}"
        );
        let views = replay(&sink.frames());
        let (_, view) = by_label(&views, &format!("session:session_{HERMES_SESSION}.json#g1"));
        assert_eq!(view.data, own);
        sink.assert_clean();
    }
}

/// A snapshot larger than the finish budget keeps what fit and says the tail is unknown.
#[test]
fn hermes_snapshot_over_the_finish_budget_is_a_shutdown_deadline_gap() {
    let root = root();
    let content = pattern_bytes(3 * MAX_SEGMENT_BYTES, 7);
    root.write(&format!("sessions/session_{HERMES_SESSION}.json"), &content);
    let sink = CollectSink::new();
    let mut capture = start(
        vec![hermes_session(&root.path("sessions"), HERMES_SESSION)],
        limits_with_poll_bytes(MAX_SEGMENT_BYTES as u64),
        &sink,
    );
    let finished = capture.finish(NativeFinishBudget {
        max_bytes: MAX_SEGMENT_BYTES as u64,
        max_duration_ns: u64::MAX,
    });
    assert_eq!(
        (finished.completed_streams, finished.interrupted_streams),
        (0, 1)
    );
    let views = replay(&sink.frames());
    let (_, view) = by_label(&views, &format!("session:session_{HERMES_SESSION}.json#g1"));
    assert_eq!(view.gaps, [(GapReason::ShutdownDeadline, None)]);
    assert_eq!(view.data, content[..MAX_SEGMENT_BYTES]);
    sink.assert_clean();
}

/// A symlink in the sessions directory is refused, never followed: neither a symlinked
/// session file nor one swapped in for the tracked file.
#[test]
fn hermes_symlinked_session_is_not_read() {
    let root = root();
    let secret = b"OUTSIDE-SCOPE-SECRET";
    fs::write(root.outside.join("secret.json"), secret).unwrap();
    fs::create_dir_all(root.path("sessions")).unwrap();
    std::os::unix::fs::symlink(
        root.outside.join("secret.json"),
        root.path(&format!("sessions/session_{HERMES_SESSION}.json")),
    )
    .unwrap();
    let sink = CollectSink::new();
    let mut capture = start(
        vec![hermes_session(&root.path("sessions"), HERMES_SESSION)],
        NativeCaptureLimits::default(),
        &sink,
    );
    assert_eq!(capture.poll().refused_entries, 1);
    let finished = capture.finish(generous());
    assert_eq!(finished.completed_streams + finished.interrupted_streams, 0);
    for frame in sink.frames() {
        if let FrameEvent::Bytes { data, .. } = &frame.event {
            assert!(!contains(data, secret), "out-of-scope bytes were read");
        }
    }
    sink.assert_clean();
}

// ---------------------------------------------------------------------------------------
// Since the run started (D57)
// ---------------------------------------------------------------------------------------

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// Set `path`'s modification time. Its change time becomes now: only the kernel sets that.
fn set_mtime(path: &Path, when: SystemTime) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

/// A shared, unscoped session store read from `started`: every file is a candidate.
fn since(root: &Root, started: SystemTime) -> NativeSourceSpec {
    NativeSourceSpec {
        modified_since: Some(started),
        ..rollout(root, "sessions/rollout-*.jsonl")
    }
}

fn unix_seconds(at: SystemTime) -> u64 {
    at.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs()
}

/// The run starts at the next whole second, and this returns a little after it. Every file
/// written (or changed) before the call is from before the run, its change time included,
/// since the filter compares whole seconds; every file written after it is the run's. The
/// margin covers a kernel that stamps files from a coarse clock.
fn run_start() -> SystemTime {
    let started = SystemTime::UNIX_EPOCH + Duration::from_secs(unix_seconds(SystemTime::now()) + 1);
    let ready = started + Duration::from_millis(50);
    if let Ok(wait) = ready.duration_since(SystemTime::now()) {
        std::thread::sleep(wait);
    }
    started
}

#[test]
fn old_files_are_skipped_and_the_new_one_found_among_5000() {
    let root = root();
    for n in 0..5000 {
        let old = root.write(
            &format!("sessions/rollout-yesterday-{n:04}.jsonl"),
            b"old\n",
        );
        set_mtime(&old, SystemTime::now() - DAY);
    }
    let started = run_start();
    root.write("sessions/rollout-today.jsonl", b"new\n");
    let sink = CollectSink::new();
    let mut capture = start(
        vec![since(&root, started)],
        NativeCaptureLimits::default(),
        &sink,
    );
    let report = capture.poll();
    assert!(
        !report.discovery_capped,
        "one directory of 5 000 sessions is enumerated within the walker's bound"
    );
    assert_eq!(
        report.untracked_files, 0,
        "an old file never counts against max_files"
    );
    assert_eq!(
        labels(&replay(&sink.frames())),
        ["rollout:sessions/rollout-today.jsonl#g1"]
    );
    assert_eq!(capture.loss_report().tracked_files, 1);
    let finished = capture.finish(generous());
    assert!(finished.closed && !finished.discovery_capped);
    assert_eq!(finished.completed_streams, 1);
    assert_eq!(finished.untracked_files, 0);
    let views = replay(&sink.frames());
    assert_eq!(views.len(), 1);
    assert_eq!(
        by_label(&views, "rollout:sessions/rollout-today.jsonl#g1")
            .1
            .data,
        b"new\n"
    );
    sink.assert_clean();
}

#[test]
fn two_new_session_files_are_both_recorded() {
    let root = root();
    let old = root.write("sessions/rollout-old.jsonl", b"old\n");
    set_mtime(&old, SystemTime::now() - DAY);
    let started = run_start();
    let sink = CollectSink::new();
    let mut capture = start(
        vec![since(&root, started)],
        NativeCaptureLimits::default(),
        &sink,
    );
    root.write("sessions/rollout-first.jsonl", b"first\n");
    capture.poll();
    root.write("sessions/rollout-second.jsonl", b"second\n");
    root.append("sessions/rollout-first.jsonl", b"more\n");
    capture.poll();
    let finished = capture.finish(generous());
    assert!(finished.closed);
    assert_eq!(finished.completed_streams, 2, "neither file is preferred");
    let views = replay(&sink.frames());
    let mut seen = labels(&views);
    seen.sort();
    assert_eq!(
        seen,
        [
            "rollout:sessions/rollout-first.jsonl#g1",
            "rollout:sessions/rollout-second.jsonl#g1",
        ]
    );
    assert_eq!(
        by_label(&views, "rollout:sessions/rollout-first.jsonl#g1")
            .1
            .data,
        b"first\nmore\n"
    );
    assert_eq!(
        by_label(&views, "rollout:sessions/rollout-second.jsonl#g1")
            .1
            .data,
        b"second\n"
    );
    sink.assert_clean();
}

#[test]
fn an_old_file_written_during_the_run_is_recorded() {
    let root = root();
    let resumed = root.write("sessions/rollout-resumed.jsonl", b"yesterday\n");
    set_mtime(&resumed, SystemTime::now() - DAY);
    let started = run_start();
    let sink = CollectSink::new();
    let mut capture = start(
        vec![since(&root, started)],
        NativeCaptureLimits::default(),
        &sink,
    );
    let report = capture.poll();
    assert_eq!(report.frames, 0, "an old file left alone is not read");
    assert_eq!(capture.loss_report().tracked_files, 0);
    // The harness resumes the old session: appending makes it this run's.
    root.append("sessions/rollout-resumed.jsonl", b"today\n");
    capture.poll();
    assert_eq!(capture.loss_report().tracked_files, 1);
    // Once admitted it stays, whatever its time says later.
    set_mtime(&resumed, started - DAY);
    capture.poll();
    assert_eq!(capture.loss_report().tracked_files, 1);
    assert_eq!(capture.loss_report().source_changes, 0);
    let finished = capture.finish(generous());
    assert_eq!(finished.completed_streams, 1);
    let views = replay(&sink.frames());
    assert_eq!(
        labels(&views),
        ["rollout:sessions/rollout-resumed.jsonl#g1"]
    );
    assert_eq!(
        by_label(&views, "rollout:sessions/rollout-resumed.jsonl#g1")
            .1
            .data,
        b"yesterday\ntoday\n",
        "the whole file is recorded, its lines from before the run included"
    );
    sink.assert_clean();
}

#[test]
fn a_backdated_file_written_during_the_run_is_recorded() {
    // `touch -d`, `cp -p` and `tar x` set the modification time back; nothing sets the change
    // time back, so a session file written during the run is still the run's.
    let root = root();
    let started = run_start();
    let backdated = root.write("sessions/rollout-copied.jsonl", b"copied\n");
    set_mtime(&backdated, started - DAY);
    let sink = CollectSink::new();
    let mut capture = start(
        vec![since(&root, started)],
        NativeCaptureLimits::default(),
        &sink,
    );
    capture.poll();
    let finished = capture.finish(generous());
    assert_eq!(finished.completed_streams, 1);
    let views = replay(&sink.frames());
    assert_eq!(
        by_label(&views, "rollout:sessions/rollout-copied.jsonl#g1")
            .1
            .data,
        b"copied\n"
    );
    sink.assert_clean();
}

/// A start a few seconds ahead, at `second` plus `nanos`: the files' change times (now) are
/// all before it, so their modification times alone decide.
fn ahead(nanos: u32) -> (u64, SystemTime) {
    let second = unix_seconds(SystemTime::now()) + 5;
    let since = SystemTime::UNIX_EPOCH + Duration::new(second, nanos);
    (second, since)
}

#[test]
fn a_file_earlier_in_the_start_second_is_admitted() {
    let root = root();
    let (second, start_at) = ahead(700_000_000);
    let same = root.write("sessions/rollout-same-second.jsonl", b"same\n");
    set_mtime(
        &same,
        SystemTime::UNIX_EPOCH + Duration::new(second, 200_000_000),
    );
    let sink = CollectSink::new();
    let mut capture = start(
        vec![since(&root, start_at)],
        NativeCaptureLimits::default(),
        &sink,
    );
    capture.poll();
    assert_eq!(
        labels(&replay(&sink.frames())),
        ["rollout:sessions/rollout-same-second.jsonl#g1"],
        "whole seconds compare equal, so the file is admitted"
    );
    sink.assert_clean();
}

#[test]
fn a_file_a_second_before_the_start_is_skipped() {
    let root = root();
    let (second, start_at) = ahead(0);
    let before = root.write("sessions/rollout-second-before.jsonl", b"before\n");
    set_mtime(
        &before,
        SystemTime::UNIX_EPOCH + Duration::new(second - 1, 999_000_000),
    );
    let sink = CollectSink::new();
    let mut capture = start(
        vec![since(&root, start_at)],
        NativeCaptureLimits::default(),
        &sink,
    );
    let report = capture.poll();
    assert_eq!(report.frames, 0);
    assert_eq!(capture.loss_report().tracked_files, 0);
    sink.assert_clean();
}

// ---------------------------------------------------------------------------------------
// Discovery losses and the entry cap (D57)
// ---------------------------------------------------------------------------------------

#[test]
fn a_directory_over_the_entry_cap_is_walked_whole_across_polls() {
    // Each poll examines at most `max_dir_entries` entries and the next resumes past them, so
    // a file that readdir returns after the first window is still reached, whatever the order.
    let root = root();
    for n in 0..30 {
        root.write(&format!("sessions/{n:02}.jsonl"), b"x\n");
    }
    let cap = 8;
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/*.jsonl")],
        NativeCaptureLimits {
            max_dir_entries: cap,
            ..NativeCaptureLimits::default()
        },
        &sink,
    );
    let first = capture.poll();
    assert!(first.discovery_capped);
    assert_eq!(capture.loss_report().tracked_files, cap);
    root.write("sessions/new.jsonl", b"new\n");
    let cycle = 31usize.div_ceil(cap);
    for _ in 0..2 * cycle {
        capture.poll();
    }
    assert_eq!(
        capture.loss_report().tracked_files,
        31,
        "every entry is reached within a cycle of ceil(n/cap) polls"
    );
    let views = replay(&sink.frames());
    assert_eq!(
        by_label(&views, "rollout:sessions/new.jsonl#g1").1.data,
        b"new\n"
    );
    assert_eq!(capture.loss_report().lost_entries, 0);
    sink.assert_clean();
}

#[test]
fn a_file_past_max_files_is_one_loss_until_another_arrives() {
    let root = root();
    for name in ["a", "b", "c"] {
        root.write(&format!("sessions/{name}.jsonl"), b"x\n");
    }
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/*.jsonl")],
        NativeCaptureLimits {
            max_files: 2,
            ..NativeCaptureLimits::default()
        },
        &sink,
    );
    for _ in 0..5 {
        capture.poll();
        assert_eq!(
            capture.loss_report().lost_entries,
            1,
            "the same untracked file is one loss, however often it is seen"
        );
    }
    root.write("sessions/d.jsonl", b"x\n");
    capture.poll();
    assert_eq!(capture.loss_report().lost_entries, 2);
    capture.poll();
    assert_eq!(capture.loss_report().lost_entries, 2);
    sink.assert_clean();
}

#[test]
fn a_refused_entry_is_one_loss_across_a_capped_cycle() {
    // The symlink sits in a directory walked over several polls; passing over entries an
    // earlier poll examined never counts them again.
    let root = root();
    for n in 0..10 {
        root.write(&format!("sessions/{n:02}.jsonl"), b"x\n");
    }
    std::os::unix::fs::symlink(
        root.outside.join("elsewhere.jsonl"),
        root.path("sessions/link.jsonl"),
    )
    .unwrap();
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/*.jsonl")],
        NativeCaptureLimits {
            max_dir_entries: 3,
            ..NativeCaptureLimits::default()
        },
        &sink,
    );
    for _ in 0..20 {
        capture.poll();
    }
    let loss = capture.loss_report();
    assert_eq!(loss.tracked_files, 10);
    assert_eq!(loss.lost_entries, 1);
    sink.assert_clean();
}

#[test]
fn a_refusal_counts_only_since_the_run_start() {
    // A shared store's old symlink is not this run's, like an old session file (D57); one
    // made during the run is a loss.
    let root = root();
    fs::create_dir_all(root.path("sessions")).unwrap();
    std::os::unix::fs::symlink(
        root.outside.join("elsewhere.jsonl"),
        root.path("sessions/rollout-old-link.jsonl"),
    )
    .unwrap();
    let started = run_start();
    let sink = CollectSink::new();
    let mut capture = start(
        vec![since(&root, started)],
        NativeCaptureLimits::default(),
        &sink,
    );
    assert_eq!(capture.poll().refused_entries, 0);
    assert_eq!(capture.loss_report().lost_entries, 0);
    std::os::unix::fs::symlink(
        root.outside.join("elsewhere.jsonl"),
        root.path("sessions/rollout-new-link.jsonl"),
    )
    .unwrap();
    assert_eq!(capture.poll().refused_entries, 1);
    assert_eq!(capture.loss_report().lost_entries, 1);
    let finished = capture.finish(generous());
    assert!(finished.closed && !finished.discovery_capped);
    assert_eq!(capture.loss_report().lost_entries, 1);
    sink.assert_clean();
}

#[test]
fn a_finish_that_walked_a_whole_cycle_is_not_capped() {
    // Four passes of three entries walk the ten files whole; the byte budget then ends the
    // finish in the fifth, a capped pass of the next cycle, which is no loss.
    let root = root();
    for n in 0..10 {
        root.write(
            &format!("sessions/{n:02}.jsonl"),
            &pattern_bytes(4 * MAX_SEGMENT_BYTES, n),
        );
    }
    let sink = CollectSink::new();
    let mut capture = start(
        vec![rollout(&root, "sessions/*.jsonl")],
        NativeCaptureLimits {
            max_dir_entries: 3,
            poll_bytes: MAX_SEGMENT_BYTES as u64,
            ..NativeCaptureLimits::default()
        },
        &sink,
    );
    let finished = capture.finish(NativeFinishBudget {
        max_bytes: 5 * MAX_SEGMENT_BYTES as u64,
        max_duration_ns: u64::MAX,
    });
    assert!(finished.closed);
    assert_eq!(finished.bytes_drained, 5 * MAX_SEGMENT_BYTES as u64);
    assert!(
        !finished.discovery_capped,
        "the finish walked a whole cycle before it stopped"
    );
    sink.assert_clean();
}
