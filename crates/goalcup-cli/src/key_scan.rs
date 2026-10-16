//! The key-fragment scan (D41).
//!
//! A provider key that was handed to a harness can come back out of it in pieces. Hermes, for
//! example, writes `sessions/request_dump_*.json` with `key[:8]…key[-4:]` in an `Authorization`
//! header on a 4xx, a 401 body may echo a masked key, and its credential pool records the
//! first sixteen hex digits of the key's SHA-256. Before any keyed run artifact is retained,
//! this module looks for those fragments, with the key held in memory only, in a directory
//! tree and in the captured trace. A hit fails the row, and the run directory stays local.
//!
//! # What it looks for
//!
//! Derived from the key, in memory, and never stored or printed:
//!
//! * the full key, its first eight bytes and its last eight bytes;
//! * every mask `key[..p]` + separator + `key[n-4..]`, for `p` in 4, 6, 8 and the
//!   separators `...`, `…`, `***` and the JSON escape `…`;
//! * a window rule for every other separator and width: `key[..4]`, then at most sixteen
//!   bytes of anything, then `key[n-4..]`;
//! * the first sixteen hex digits of the SHA-256 of the key, in either case;
//! * each of the byte forms above inside base64 (standard and URL-safe alphabets), at all
//!   three alignments, so a mask inside a `data_b64` segment of a trace is found.
//!
//! A bare four-character suffix is **not** a form: it collides with ordinary data. It counts
//! only inside the window rule or a mask.
//!
//! # What it never does
//!
//! It never prints, logs or returns a byte of the key or of a form: [`FragmentMatcher`]'s
//! `Debug` prints a count, and a hit names a path relative to its root and nothing else. The
//! key crosses this interface as bytes, not as the launch's credential type, so a caller that
//! holds the key some other way can use it. Every derived form is overwritten when the
//! matcher is dropped. Zeroing is hygiene, not a boundary: the key stays in the launching
//! process's environment for its lifetime, which D37 accepts.
//!
//! # Completeness
//!
//! A scan that could not read everything is not a clean scan. A symlink is not followed, and
//! a special file, an unreadable file or directory, and a tree larger than `max_bytes` are
//! all [`ScanOutcome::Incomplete`], which fails the row exactly as a hit does.

use crate::CliError;
use base64::Engine as _;
use goalcup_protocol::capture::{CaptureRecord, StreamKey};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The shortest key this scan accepts. A shorter secret yields forms common enough to hit on
/// ordinary data, and a scan that cries wolf is one that gets switched off.
pub(crate) const MIN_KEY_BYTES: usize = 20;

/// The shortest derived form. Every form is at least this long (a base64 form of an
/// eight-byte fragment, after the bits that depend on its neighbours are dropped, is not
/// shorter), so a chance hit on ordinary data is negligible.
const MIN_FORM_BYTES: usize = 8;

/// How far past the first four key bytes the window rule looks for the last four.
const WINDOW_GAP: usize = 16;

/// The separators a mask is enumerated with.
const SEPARATORS: [&str; 4] = ["...", "\u{2026}", "***", "\\u2026"];

/// The prefix lengths a mask is enumerated with.
const MASK_PREFIXES: [usize; 3] = [4, 6, 8];

/// The bytes of one read of a file.
const READ_CHUNK: usize = 64 * 1024;

/// The key's derived forms, held in memory only.
pub struct FragmentMatcher {
    /// Every byte form, grouped by its first four bytes for the scan.
    forms: HashMap<[u8; 4], Vec<Vec<u8>>>,
    /// The number of byte forms, for `Debug` and nothing else.
    count: usize,
    /// The window rule's two ends.
    window_head: [u8; 4],
    window_tail: [u8; 4],
    /// The longest span a match can cover.
    span: usize,
}

impl fmt::Debug for FragmentMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FragmentMatcher({} forms)", self.count)
    }
}

impl Drop for FragmentMatcher {
    fn drop(&mut self) {
        for list in self.forms.values_mut() {
            for form in list.iter_mut() {
                wipe(form);
            }
        }
        wipe(&mut self.window_head);
        wipe(&mut self.window_tail);
    }
}

/// Overwrite `bytes` with zeros, and tell the optimizer the zeros are observed so the stores
/// are not removed as dead. Hygiene for a value the matcher derived, not a boundary.
pub(crate) fn wipe(bytes: &mut [u8]) {
    bytes.fill(0);
    std::hint::black_box(&*bytes);
}

impl FragmentMatcher {
    /// The forms of `secret`. Refused, without echoing it, when it is shorter than
    /// [`MIN_KEY_BYTES`] or holds anything but printable ASCII (a provider key is one token;
    /// a value with whitespace or control bytes is not one this scan can reason about).
    pub fn new(secret: &[u8]) -> Result<FragmentMatcher, CliError> {
        if secret.len() < MIN_KEY_BYTES || !secret.iter().all(|b| (0x21..=0x7e).contains(b)) {
            return Err(CliError::Refused(format!(
                "the key to scan for is not usable: it must be at least {MIN_KEY_BYTES} \
                 printable ASCII bytes with no whitespace"
            )));
        }
        let n = secret.len();
        let mut raw: Vec<Vec<u8>> = vec![
            secret.to_vec(),
            secret[..8].to_vec(),
            secret[n - 8..].to_vec(),
        ];
        for prefix in MASK_PREFIXES {
            for sep in SEPARATORS {
                let mut mask = secret[..prefix].to_vec();
                mask.extend_from_slice(sep.as_bytes());
                mask.extend_from_slice(&secret[n - 4..]);
                raw.push(mask);
            }
        }
        // The fingerprint, spelled byte by byte from a table: no formatting temporaries.
        let mut digest = Sha256::digest(secret);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut lower = Vec::with_capacity(16);
        for byte in &digest[..8] {
            lower.push(HEX[usize::from(byte >> 4)]);
            lower.push(HEX[usize::from(byte & 0x0f)]);
        }
        wipe(digest.as_mut_slice());
        raw.push(lower.to_ascii_uppercase());
        raw.push(lower);

        let mut forms: Vec<Vec<u8>> = Vec::new();
        for form in &raw {
            forms.push(form.clone());
            for alphabet in [Alphabet::Standard, Alphabet::Url] {
                for alignment in 0..3 {
                    forms.push(base64_inside(form, alignment, alphabet));
                }
            }
        }
        for form in &mut raw {
            wipe(form);
        }
        forms.sort();
        // `dedup`, but the copy it throws away is wiped first.
        let mut unique: Vec<Vec<u8>> = Vec::with_capacity(forms.len());
        for mut form in forms {
            if unique.last() == Some(&form) {
                wipe(&mut form);
            } else {
                unique.push(form);
            }
        }
        let forms = unique;
        let mut grouped: HashMap<[u8; 4], Vec<Vec<u8>>> = HashMap::new();
        let mut count = 0;
        let mut longest = 0;
        for form in forms {
            if form.len() < MIN_FORM_BYTES {
                continue;
            }
            longest = longest.max(form.len());
            let mut head = [0u8; 4];
            head.copy_from_slice(&form[..4]);
            grouped.entry(head).or_default().push(form);
            count += 1;
        }
        let mut window_head = [0u8; 4];
        window_head.copy_from_slice(&secret[..4]);
        let mut window_tail = [0u8; 4];
        window_tail.copy_from_slice(&secret[n - 4..]);
        Ok(FragmentMatcher {
            forms: grouped,
            count,
            window_head,
            window_tail,
            span: longest.max(4 + WINDOW_GAP + 4),
        })
    }

    /// The number of bytes of context a scan carries between two chunks.
    pub(crate) fn overlap(&self) -> usize {
        self.span - 1
    }

    /// Whether `haystack` holds any form or the window rule. Stateless.
    pub fn contains(&self, haystack: &[u8]) -> bool {
        if haystack.len() < 4 {
            return false;
        }
        for i in 0..=haystack.len() - 4 {
            let mut head = [0u8; 4];
            head.copy_from_slice(&haystack[i..i + 4]);
            if let Some(candidates) = self.forms.get(&head)
                && candidates.iter().any(|f| haystack[i..].starts_with(f))
            {
                return true;
            }
            if head == self.window_head {
                let from = i + 4;
                let to = (from + WINDOW_GAP + 4).min(haystack.len());
                if haystack[from..to]
                    .windows(4)
                    .position(|w| w == self.window_tail)
                    .is_some_and(|p| p <= WINDOW_GAP)
                {
                    return true;
                }
            }
        }
        false
    }
}

#[derive(Clone, Copy)]
enum Alphabet {
    Standard,
    Url,
}

impl Alphabet {
    fn table(self) -> &'static [u8; 64] {
        match self {
            Alphabet::Standard => {
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
            }
            Alphabet::Url => b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_",
        }
    }
}

/// The base64 text that `form` always produces when it sits inside a longer encoded stream
/// at byte offset `alignment` modulo three: the characters that depend on no byte outside
/// the form. Unpadded, since the end of the form is rarely the end of the stream.
///
/// With `alignment` bytes of unknown data before it, the first `ceil(alignment * 8 / 6)`
/// characters mix those bytes in, and the characters after the last complete group
/// are determined only as far as whole six-bit values of the form's own bits go.
fn base64_inside(form: &[u8], alignment: usize, alphabet: Alphabet) -> Vec<u8> {
    let table = alphabet.table();
    let mut padded = vec![0u8; alignment];
    padded.extend_from_slice(form);
    let mut out = Vec::with_capacity(padded.len() * 4 / 3 + 4);
    for group in padded.chunks(3) {
        let b = [
            group[0],
            group.get(1).copied().unwrap_or(0),
            group.get(2).copied().unwrap_or(0),
        ];
        let sextets = [
            b[0] >> 2,
            ((b[0] & 0x03) << 4) | (b[1] >> 4),
            ((b[1] & 0x0f) << 2) | (b[2] >> 6),
            b[2] & 0x3f,
        ];
        // A partial group of `len` bytes carries `len * 8 / 6` whole sextets.
        let whole = match group.len() {
            3 => 4,
            len => len * 8 / 6,
        };
        out.extend(sextets.iter().take(whole).map(|s| table[*s as usize]));
    }
    wipe(&mut padded);
    // Drop the characters that mix in the unknown bytes before the form, and wipe what is
    // dropped: `split_off` would leave the lead in the buffer it keeps.
    let lead = (alignment * 8).div_ceil(6).min(out.len());
    let kept = out[lead..].to_vec();
    wipe(&mut out);
    kept
}

/// What a tree scan found. Only [`ScanOutcome::Clean`] is clean: ask [`ScanOutcome::is_clean`],
/// and never read `Incomplete` as a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "an outcome that is not Clean fails the row; check it"]
pub enum ScanOutcome {
    /// Every file under every root was read, and none held a fragment.
    Clean,
    /// These files held a fragment, as paths relative to their root, each led by its root's
    /// own directory name.
    Hit { relative_paths: Vec<PathBuf> },
    /// The scan could not read everything, and says why in a fixed word.
    Incomplete { reason: &'static str },
}

impl ScanOutcome {
    /// Whether the scan read everything it was asked to and found nothing. The only pass.
    pub fn is_clean(&self) -> bool {
        matches!(self, ScanOutcome::Clean)
    }

    /// The one word a status file records.
    pub fn verdict(&self) -> &'static str {
        match self {
            ScanOutcome::Clean => "clean",
            ScanOutcome::Hit { .. } => "hit",
            ScanOutcome::Incomplete { .. } => "incomplete",
        }
    }

    /// Both outcomes as one: every hit of either (a hit names its files, and outranks an
    /// incomplete scan); otherwise incomplete if either was, with the first reason; clean only
    /// when both were.
    pub(crate) fn merge(self, other: ScanOutcome) -> ScanOutcome {
        match (self, other) {
            (
                ScanOutcome::Hit {
                    relative_paths: mut a,
                },
                ScanOutcome::Hit { relative_paths: b },
            ) => {
                a.extend(b);
                a.sort();
                a.dedup();
                ScanOutcome::Hit { relative_paths: a }
            }
            (hit @ ScanOutcome::Hit { .. }, _) | (_, hit @ ScanOutcome::Hit { .. }) => hit,
            (incomplete @ ScanOutcome::Incomplete { .. }, _)
            | (_, incomplete @ ScanOutcome::Incomplete { .. }) => incomplete,
            (ScanOutcome::Clean, ScanOutcome::Clean) => ScanOutcome::Clean,
        }
    }
}

impl fmt::Display for ScanOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanOutcome::Clean => write!(f, "clean"),
            ScanOutcome::Hit { relative_paths } => {
                write!(f, "hit in {} file(s)", relative_paths.len())
            }
            ScanOutcome::Incomplete { reason } => write!(f, "incomplete: {reason}"),
        }
    }
}

/// Scan every regular file under each of `roots`, reading at most `max_bytes` in all.
///
/// Symlinks are never followed and never read: one is [`ScanOutcome::Incomplete`], as is a
/// special file, an entry that cannot be read, and a tree over the byte bound. Hits take
/// precedence over an incomplete scan (either fails the row, and a hit names its files).
/// The matcher is not mutated, so one matcher serves every root.
pub fn scan_tree(matcher: &FragmentMatcher, roots: &[&Path], max_bytes: u64) -> ScanOutcome {
    let mut state = Walk {
        matcher,
        budget: max_bytes,
        hits: Vec::new(),
        incomplete: None,
        files_read: 0,
    };
    for root in roots {
        let lead = root
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("root"));
        state.directory(root, &lead);
    }
    if !state.hits.is_empty() {
        state.hits.sort();
        state.hits.dedup();
        return ScanOutcome::Hit {
            relative_paths: state.hits,
        };
    }
    match state.incomplete {
        Some(reason) => ScanOutcome::Incomplete { reason },
        // A scan that read nothing has not shown the absence of anything: an empty list of
        // roots, an empty directory, or a tree of directories holding no file.
        None if state.files_read == 0 => ScanOutcome::Incomplete {
            reason: "there was nothing to scan",
        },
        None => ScanOutcome::Clean,
    }
}

/// The decoded segments of a trace's streams, scanned in order: the bytes of each stream are
/// followed across its segment records, however other streams' records are interleaved with
/// them, and two streams are never joined.
///
/// A segment record holds at most [`goalcup_protocol::capture::MAX_SEGMENT_BYTES`] of a stream
/// and segments split at flush ticks, so a mask can straddle two of them.
/// The base64 text of each record then holds only part of it, which no scan of the trace's text
/// can see; scanning the decoded stream can. Each stream keeps only the last bytes a match could
/// still span, wiped when the scan is dropped.
pub struct StreamScan<'a> {
    matcher: &'a FragmentMatcher,
    carries: HashMap<StreamKey, Vec<u8>>,
    hit: bool,
}

impl<'a> StreamScan<'a> {
    pub fn new(matcher: &'a FragmentMatcher) -> StreamScan<'a> {
        StreamScan {
            matcher,
            carries: HashMap::new(),
            hit: false,
        }
    }

    /// The next decoded segment of `stream`. `true` when it, with the bytes of the same stream
    /// before it, completes a match.
    pub fn segment(&mut self, stream: &StreamKey, bytes: &[u8]) -> bool {
        let carry = self.carries.entry(stream.clone()).or_default();
        let mut window = std::mem::take(carry);
        window.extend_from_slice(bytes);
        let found = self.matcher.contains(&window);
        let start = window.len().saturating_sub(self.matcher.overlap());
        wipe(&mut window[..start]);
        window.drain(..start);
        *carry = window;
        self.hit |= found;
        found
    }

    /// Whether any segment so far completed a match.
    pub fn hit(&self) -> bool {
        self.hit
    }
}

impl Drop for StreamScan<'_> {
    fn drop(&mut self) {
        for carry in self.carries.values_mut() {
            wipe(carry);
        }
    }
}

/// The streams of the capture trace in `trace_dir`, decoded and scanned in order.
///
/// Read through the verifier's own trace reader, so what is scanned is what the verifier
/// would read. A trace it cannot read, a segment that does not decode, or a trace holding no
/// segment at all is [`ScanOutcome::Incomplete`]: a scan that could not see the evidence has not
/// shown there is nothing in it, and neither has a chain the reader stopped partway through.
/// A hit names the chain as `<trace dir>/chain.jsonl`, whichever part held it. A chain whose
/// first part was deleted here (uploaded and acknowledged) is [`ScanOutcome::Incomplete`]: only
/// the server's copy is the whole run.
pub fn scan_trace(matcher: &FragmentMatcher, trace_dir: &Path) -> ScanOutcome {
    let mut scan = StreamScan::new(matcher);
    let mut segments = 0u64;
    let mut undecodable = false;
    let read = goalcup_verifier::capture_analysis::reader::scan_capture_trace_default(
        trace_dir,
        &mut |record| {
            if let CaptureRecord::Segment {
                stream, data_b64, ..
            } = record
            {
                segments += 1;
                match base64::engine::general_purpose::STANDARD.decode(data_b64) {
                    Ok(mut bytes) => {
                        scan.segment(stream, &bytes);
                        wipe(&mut bytes);
                    }
                    Err(_) => undecodable = true,
                }
            }
            Ok(())
        },
    );
    let Ok(report) = read else {
        return ScanOutcome::Incomplete {
            reason: "the trace could not be read",
        };
    };
    // The reader answers `Ok` for invalid evidence and simply stops at the first bad record, so
    // an `Ok` is not "read to the end". Only a chain whose validated prefix is every byte of its
    // parts, with no torn tail, was seen whole; anything else is a partial read, and a partial
    // read is never clean. A hit in what was read still counts.
    let chain = if report.torn_tail {
        ChainRead::Partial
    } else {
        chain_read(trace_dir, report.validated_bytes)
    };
    if scan.hit() {
        let lead = trace_dir
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("trace"));
        return ScanOutcome::Hit {
            relative_paths: vec![lead.join("chain.jsonl")],
        };
    }
    match chain {
        ChainRead::Whole => {}
        ChainRead::Partial => {
            return ScanOutcome::Incomplete {
                reason: "the trace was not read to its end",
            };
        }
        // The parts before the first one left were uploaded and deleted here: what remains is
        // not the run, and only the server's copy is.
        ChainRead::PrunedLocally => {
            return ScanOutcome::Incomplete {
                reason: "the start of the trace was deleted from this machine after it was uploaded",
            };
        }
    }
    if undecodable {
        return ScanOutcome::Incomplete {
            reason: "a trace segment did not decode",
        };
    }
    if segments == 0 {
        return ScanOutcome::Incomplete {
            reason: "the trace holds no segment",
        };
    }
    ScanOutcome::Clean
}

/// What the chain parts on disk are, against what the reader validated.
enum ChainRead {
    /// Contiguous from byte 0, regular files, and ending where the reader's validated prefix ends.
    Whole,
    /// Part 0 is gone: acknowledged parts were deleted here ([`crate::upload::prune`]).
    PrunedLocally,
    /// Anything else: unreadable, a link, a gap, bytes the reader did not validate.
    Partial,
}

fn chain_read(trace_dir: &Path, validated_bytes: u64) -> ChainRead {
    let Ok(parts) = crate::upload::prune::parts_now(trace_dir) else {
        return ChainRead::Partial;
    };
    let regular = parts
        .iter()
        .all(|part| fs::symlink_metadata(&part.path).is_ok_and(|meta| meta.is_file()));
    let (Some(first), Some(last)) = (parts.first(), parts.last()) else {
        return ChainRead::Partial;
    };
    if !regular || last.start.saturating_add(last.len) != validated_bytes {
        ChainRead::Partial
    } else if first.start > 0 {
        ChainRead::PrunedLocally
    } else {
        ChainRead::Whole
    }
}

/// The whole of a run: every file under each of `roots` and under `trace_dir` (the trace's own
/// text, base64 forms included), and the decoded streams of the trace ([`scan_trace`]). `roots`
/// must not be empty: a scan of nothing is [`ScanOutcome::Incomplete`].
pub fn scan_run(
    matcher: &FragmentMatcher,
    roots: &[&Path],
    trace_dir: &Path,
    max_bytes: u64,
) -> ScanOutcome {
    if roots.is_empty() {
        return ScanOutcome::Incomplete {
            reason: "there was nothing to scan",
        };
    }
    let mut all: Vec<&Path> = roots.to_vec();
    all.push(trace_dir);
    scan_tree(matcher, &all, max_bytes).merge(scan_trace(matcher, trace_dir))
}

/// Write `<run_dir>/key-scan.txt`: the verdict word, then, for a hit, the relative paths one
/// per line, or, for an incomplete scan, its fixed reason. Never a byte of the key or of a form.
/// Mode 0600, replacing an earlier verdict. Whatever retains a run's artifacts reads this file
/// and refuses unless its first line is `clean`.
///
/// This writes no [`Binding`]: [`check_verdict`] reads such a file as [`VerdictCheck::Unbound`],
/// never as current. A scan that read the run writes [`write_verdict_bound`].
pub fn write_verdict(run_dir: &Path, outcome: &ScanOutcome) -> std::io::Result<()> {
    write_verdict_text(run_dir, outcome, None)
}

/// [`write_verdict`], then the [`Binding`] of what was scanned as the last three lines
/// (`trace`, `trace-chain`, `scanned-files`). The first line is still exactly the verdict word,
/// and the detail lines (hit paths, an incomplete reason) still follow it, so a script that reads
/// the first line sees what it always saw. H-18a.
pub fn write_verdict_bound(
    run_dir: &Path,
    outcome: &ScanOutcome,
    binding: &Binding,
) -> std::io::Result<()> {
    write_verdict_text(run_dir, outcome, Some(binding))
}

fn write_verdict_text(
    run_dir: &Path,
    outcome: &ScanOutcome,
    binding: Option<&Binding>,
) -> std::io::Result<()> {
    use std::io::Write;
    let mut text = String::from(outcome.verdict());
    text.push('\n');
    match outcome {
        ScanOutcome::Clean => {}
        ScanOutcome::Hit { relative_paths } => {
            for path in relative_paths {
                text.push_str(&path.to_string_lossy());
                text.push('\n');
            }
        }
        ScanOutcome::Incomplete { reason } => {
            text.push_str(reason);
            text.push('\n');
        }
    }
    if let Some(binding) = binding {
        text.push_str(&binding.lines());
    }
    let path = run_dir.join(VERDICT_FILE);
    match fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(&path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// The verdict file's name in a run directory.
pub(crate) const VERDICT_FILE: &str = "key-scan.txt";

/// What a verdict was a verdict about: the trace's chain (its length and SHA-256) and a digest of
/// the file list of everything scanned (the run directory without the verdict file, and the trace
/// directory), each entry as a relative path, a kind and a size. A run-directory file also
/// contributes its modification time (to the nanosecond) and its inode, so a rewrite of the same
/// size reads as stale. The content of those files is not hashed: the trace, which is the
/// evidence, is bound by content. A copy or a restore from backup changes both numbers and so
/// also reads as stale; the binding describes this directory as the scan left it. Captured
/// *before* the scan reads anything, so a change during the scan reads as stale, never as
/// covered.
///
/// A value of `unreadable` (or a trace path that cannot be written on one line) is never equal to
/// anything: a binding that could not be computed binds nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    trace: Option<PathBuf>,
    chain: String,
    files: String,
}

const UNREADABLE: &str = "unreadable";

impl Binding {
    /// The binding of `run_dir` and, when the finish returned one, `trace_dir`, as they are now.
    pub fn capture(run_dir: &Path, trace_dir: Option<&Path>) -> Binding {
        let chain = match trace_dir {
            None => "none".to_string(),
            Some(trace) => chain_digest(trace),
        };
        let mut hasher = Sha256::new();
        let mut count = 0u64;
        let mut ok = list_tree(run_dir, "0", true, true, &mut hasher, &mut count);
        if let Some(trace) = trace_dir {
            ok &= list_tree(trace, "1", false, false, &mut hasher, &mut count);
        }
        let files = if ok {
            format!(
                "count={count} sha256={}",
                hex_of(hasher.finalize().as_slice())
            )
        } else {
            UNREADABLE.to_string()
        };
        let trace = trace_dir.map(Path::to_path_buf);
        Binding {
            trace,
            chain,
            files,
        }
    }

    fn trace_text(&self) -> Option<String> {
        match &self.trace {
            None => Some("none".to_string()),
            Some(path) => {
                let text = path.to_string_lossy();
                (!text.contains('\n') && text != "none").then(|| text.into_owned())
            }
        }
    }

    fn lines(&self) -> String {
        let trace = self.trace_text().unwrap_or_else(|| UNREADABLE.to_string());
        format!(
            "trace {trace}\ntrace-chain {}\nscanned-files {}\n",
            self.chain, self.files
        )
    }

    fn computable(&self) -> bool {
        self.trace_text().is_some() && self.chain != UNREADABLE && self.files != UNREADABLE
    }
}

fn hex_of(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// `bytes=<n> sha256=<hex>` of the chain: its parts, in order, as one stream (a chain of one
/// part gives the digest the single file always gave). `absent` when there is no part,
/// `unreadable` when a part is a link or a special file, the parts do not follow one another,
/// or one cannot be read.
fn chain_digest(trace_dir: &Path) -> String {
    let parts = match crate::upload::prune::parts_now(trace_dir) {
        Ok(parts) if parts.is_empty() => return "absent".to_string(),
        Ok(parts) => parts,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return "absent".to_string(),
        Err(_) => return UNREADABLE.to_string(),
    };
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut chunk = vec![0u8; READ_CHUNK];
    for part in &parts {
        match fs::symlink_metadata(&part.path) {
            Ok(meta) if meta.is_file() => {}
            _ => return UNREADABLE.to_string(),
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let Ok(mut file) = options.open(&part.path) else {
            return UNREADABLE.to_string();
        };
        loop {
            match file.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    hasher.update(&chunk[..read]);
                    total += read as u64;
                }
                Err(_) => return UNREADABLE.to_string(),
            }
        }
    }
    format!(
        "bytes={total} sha256={}",
        hex_of(hasher.finalize().as_slice())
    )
}

/// Feed the sorted listing of `dir` (every entry, links not followed) into `hasher` as
/// `<tag>/<relative path>\t<kind><size>\n`; `skip_verdict` leaves out the top-level verdict file,
/// and `identify` adds a file's modification time and inode after its size. False when something
/// could not be listed.
fn list_tree(
    dir: &Path,
    tag: &str,
    skip_verdict: bool,
    identify: bool,
    hasher: &mut Sha256,
    count: &mut u64,
) -> bool {
    struct Listing<'a> {
        tag: &'a str,
        skip_verdict: bool,
        identify: bool,
        hasher: &'a mut Sha256,
        count: &'a mut u64,
    }

    impl Listing<'_> {
        fn walk(&mut self, dir: &Path, rel: &Path, top: bool) -> bool {
            let Ok(entries) = fs::read_dir(dir) else {
                return false;
            };
            let mut names = Vec::new();
            for entry in entries {
                match entry {
                    Ok(entry) => names.push(entry.file_name()),
                    Err(_) => return false,
                }
            }
            names.sort();
            let mut ok = true;
            for name in names {
                if top && self.skip_verdict && name == VERDICT_FILE {
                    continue;
                }
                let path = dir.join(&name);
                let shown = rel.join(&name);
                let Ok(meta) = fs::symlink_metadata(&path) else {
                    ok = false;
                    continue;
                };
                let kind = meta.file_type();
                let (letter, size) = if kind.is_symlink() {
                    ('l', 0)
                } else if kind.is_dir() {
                    ('d', 0)
                } else if kind.is_file() {
                    ('f', meta.len())
                } else {
                    ('o', 0)
                };
                let who = if self.identify && letter == 'f' {
                    file_identity(&meta)
                } else {
                    String::new()
                };
                self.hasher.update(format!(
                    "{}/{}\t{letter}{size}{who}\n",
                    self.tag,
                    shown.to_string_lossy()
                ));
                *self.count += 1;
                if kind.is_dir() {
                    ok &= self.walk(&path, &shown, false);
                }
            }
            ok
        }
    }

    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() => {}
        _ => return false,
    }
    Listing {
        tag,
        skip_verdict,
        identify,
        hasher,
        count,
    }
    .walk(dir, Path::new(""), true)
}

/// ` m<seconds>.<nanoseconds> i<inode>`: what tells a file rewritten at the same size from the
/// file that was scanned.
fn file_identity(meta: &fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!(
        " m{}.{:09} i{}",
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ino()
    )
}

/// What [`check_verdict`] found. Only [`VerdictCheck::Current`] with the word `clean` is clean.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "a verdict that is not Current and clean must not retain a run"]
pub enum VerdictCheck {
    /// The recomputed binding equals the recorded one: the file still describes what is there.
    Current { verdict: String },
    /// What the verdict was about changed after the scan (or cannot be recomputed): it says
    /// nothing about what is on disk now, whatever its first line reads.
    Stale { reason: &'static str },
    /// The file carries no binding (an earlier build, a run that was never scanned, or a file
    /// that does not parse), so it cannot be shown to describe the current state.
    Unbound,
    /// There is no verdict file, or it cannot be read.
    Missing,
}

impl VerdictCheck {
    /// Whether a retention step may treat the run as scanned and clean: the verdict is current
    /// and its word is `clean`. Every other case, stale included, is not.
    pub fn is_clean(&self) -> bool {
        matches!(self, VerdictCheck::Current { verdict } if verdict == "clean")
    }
}

impl fmt::Display for VerdictCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerdictCheck::Current { verdict } => write!(f, "current: {verdict}"),
            VerdictCheck::Stale { reason } => write!(f, "stale: {reason}"),
            VerdictCheck::Unbound => write!(f, "unbound: the verdict file records no binding"),
            VerdictCheck::Missing => write!(f, "missing: there is no readable verdict file"),
        }
    }
}

/// Recompute the binding of `<run_dir>/key-scan.txt` and compare it with the recorded one. A
/// retention step runs this and refuses unless [`VerdictCheck::is_clean`]: a `clean` first line
/// alone is a verdict about a past state. Read-only; never reads a byte of any key.
pub fn check_verdict(run_dir: &Path) -> VerdictCheck {
    let Ok(text) = fs::read_to_string(run_dir.join(VERDICT_FILE)) else {
        return VerdictCheck::Missing;
    };
    let mut lines: Vec<&str> = text.lines().collect();
    let Some(&verdict) = lines.first() else {
        return VerdictCheck::Missing;
    };
    if lines.len() < 4 {
        return VerdictCheck::Unbound;
    }
    let files = lines.pop().and_then(|l| l.strip_prefix("scanned-files "));
    let chain = lines.pop().and_then(|l| l.strip_prefix("trace-chain "));
    let trace = lines.pop().and_then(|l| l.strip_prefix("trace "));
    let (Some(files), Some(chain), Some(trace)) = (files, chain, trace) else {
        return VerdictCheck::Unbound;
    };
    let trace_dir = (trace != "none").then(|| PathBuf::from(trace));
    let now = Binding::capture(run_dir, trace_dir.as_deref());
    if !now.computable() || chain == UNREADABLE || files == UNREADABLE || trace == UNREADABLE {
        return VerdictCheck::Stale {
            reason: "the binding could not be computed, so the verdict cannot be shown current",
        };
    }
    if now.chain != chain {
        return VerdictCheck::Stale {
            reason: "the trace chain changed after the scan",
        };
    }
    if now.files != files {
        return VerdictCheck::Stale {
            reason: "the run directory or trace file list changed after the scan",
        };
    }
    VerdictCheck::Current {
        verdict: verdict.to_string(),
    }
}

struct Walk<'a> {
    matcher: &'a FragmentMatcher,
    budget: u64,
    hits: Vec<PathBuf>,
    incomplete: Option<&'static str>,
    /// Regular files opened and read to their end (or to a hit).
    files_read: u64,
}

impl Walk<'_> {
    fn stop(&mut self, reason: &'static str) {
        self.incomplete.get_or_insert(reason);
    }

    fn directory(&mut self, dir: &Path, shown: &Path) {
        let Ok(meta) = fs::symlink_metadata(dir) else {
            return self.stop("a root could not be read");
        };
        if meta.file_type().is_symlink() {
            return self.stop("a symlink was not followed");
        }
        if !meta.is_dir() {
            return self.stop("a root is not a directory");
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return self.stop("a directory could not be read");
        };
        let mut names = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => names.push(entry.file_name()),
                Err(_) => self.stop("a directory entry could not be read"),
            }
        }
        names.sort();
        for name in names {
            let path = dir.join(&name);
            let shown = shown.join(&name);
            let Ok(meta) = fs::symlink_metadata(&path) else {
                self.stop("an entry could not be read");
                continue;
            };
            let kind = meta.file_type();
            if kind.is_symlink() {
                self.stop("a symlink was not followed");
            } else if kind.is_dir() {
                self.directory(&path, &shown);
            } else if kind.is_file() {
                self.file(&path, &shown, meta.len());
            } else {
                self.stop("a special file was not read");
            }
        }
    }

    fn file(&mut self, path: &Path, shown: &Path, len: u64) {
        if len > self.budget {
            self.budget = 0;
            return self.stop("the tree is larger than the scan bound");
        }
        // Opened without following a symlink swapped in after the walk looked at the entry,
        // and without blocking on a FIFO swapped in; then checked on the open file itself.
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let Ok(mut file) = options.open(path) else {
            return self.stop("a file could not be opened");
        };
        if !file.metadata().is_ok_and(|meta| meta.is_file()) {
            return self.stop("a special file was not read");
        }
        self.files_read += 1;
        let overlap = self.matcher.overlap();
        let mut window: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; READ_CHUNK];
        loop {
            let read = match file.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => read,
                Err(_) => return self.stop("a file could not be read"),
            };
            if read as u64 > self.budget {
                self.budget = 0;
                return self.stop("the tree is larger than the scan bound");
            }
            self.budget -= read as u64;
            window.extend_from_slice(&chunk[..read]);
            if self.matcher.contains(&window) {
                self.hits.push(shown.to_path_buf());
                return;
            }
            let start = window.len().saturating_sub(overlap);
            window.drain(..start);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_part(dir: &Path, start: u64, bytes: &[u8]) {
        fs::write(
            dir.join(goalcup_protocol::export::part_file_name(start)),
            bytes,
        )
        .unwrap();
    }

    /// A chain is whole only when its parts run from byte 0 to the end the reader validated;
    /// with the first part deleted here, it is the server's copy that is the record.
    #[test]
    fn a_chain_is_whole_only_when_its_parts_run_from_byte_zero() {
        let dir = tempfile::tempdir().unwrap();
        put_part(dir.path(), 0, b"aaaa\n");
        put_part(dir.path(), 5, b"bb\n");
        assert!(matches!(chain_read(dir.path(), 8), ChainRead::Whole));
        assert!(matches!(chain_read(dir.path(), 5), ChainRead::Partial));
        assert!(matches!(chain_read(dir.path(), 9), ChainRead::Partial));
        // The first part acknowledged and deleted: the reader validates the rest up to the same
        // logical end, and that must not read as the whole chain.
        fs::remove_file(dir.path().join("chain.jsonl")).unwrap();
        assert!(matches!(
            chain_read(dir.path(), 8),
            ChainRead::PrunedLocally
        ));
        assert!(matches!(chain_read(dir.path(), 5), ChainRead::Partial));
        // No part at all, and a part missing between two others.
        let empty = tempfile::tempdir().unwrap();
        assert!(matches!(chain_read(empty.path(), 0), ChainRead::Partial));
        put_part(dir.path(), 8, b"cc\n");
        put_part(dir.path(), 20, b"dd\n");
        assert!(matches!(chain_read(dir.path(), 23), ChainRead::Partial));
    }

    /// The binding digest is of the parts as one stream: a single part gives what the single
    /// file always gave, and deleting a part changes it.
    #[test]
    fn the_chain_digest_covers_every_part_in_order() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(chain_digest(dir.path()), "absent");
        assert_eq!(chain_digest(&dir.path().join("none")), "absent");
        put_part(dir.path(), 0, b"aaaa\nbb\n");
        let single = chain_digest(dir.path());
        assert!(single.starts_with("bytes=8 "), "{single}");
        let split = tempfile::tempdir().unwrap();
        put_part(split.path(), 0, b"aaaa\n");
        put_part(split.path(), 5, b"bb\n");
        assert_eq!(chain_digest(split.path()), single);
        fs::remove_file(split.path().join("chain.jsonl")).unwrap();
        assert_ne!(chain_digest(split.path()), single);
        put_part(split.path(), 99, b"x\n");
        assert_eq!(
            chain_digest(split.path()),
            UNREADABLE,
            "a gap binds nothing"
        );
    }

    /// The base64 forms are exactly what an encoder produces for the form at each
    /// alignment: encode a longer stream, and find the form's text inside it.
    #[test]
    fn base64_inside_is_found_in_a_real_encoding_at_every_alignment() {
        let form = b"sk-abcdef...wxyz";
        for alignment in 0..3 {
            for tail in 0..3 {
                let mut stream = vec![0xA5u8; alignment];
                stream.extend_from_slice(form);
                stream.extend(std::iter::repeat_n(0x5Au8, tail));
                let mut text = Vec::new();
                for group in stream.chunks(3) {
                    let b = [
                        group[0],
                        group.get(1).copied().unwrap_or(0),
                        group.get(2).copied().unwrap_or(0),
                    ];
                    let s = [
                        b[0] >> 2,
                        ((b[0] & 3) << 4) | (b[1] >> 4),
                        ((b[1] & 15) << 2) | (b[2] >> 6),
                        b[2] & 63,
                    ];
                    let whole = if group.len() == 3 {
                        4
                    } else {
                        group.len() * 8 / 6 + 1
                    };
                    text.extend(
                        s.iter()
                            .take(whole)
                            .map(|v| Alphabet::Standard.table()[*v as usize]),
                    );
                }
                let needle = base64_inside(form, alignment, Alphabet::Standard);
                assert!(
                    text.windows(needle.len()).any(|w| w == needle),
                    "alignment {alignment} tail {tail}"
                );
            }
        }
    }

    /// The file is opened without following a symlink, so a link swapped
    /// in after the directory walk looked at the entry is never read, and a FIFO swapped in
    /// does not block the scan.
    #[cfg(unix)]
    #[test]
    fn a_file_swapped_for_a_symlink_or_a_fifo_after_the_walk_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let key = "sk-3f9a1c7e5b2d4086a1f3c5e7d9b0a2c4";
        let matcher = FragmentMatcher::new(key.as_bytes()).unwrap();
        let target = dir.path().join("target.txt");
        std::fs::write(&target, key).unwrap();
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let mut walk = Walk {
            matcher: &matcher,
            budget: 1 << 20,
            hits: Vec::new(),
            incomplete: None,
            files_read: 0,
        };
        walk.file(&link, Path::new("link.txt"), 10);
        assert!(walk.hits.is_empty(), "the link was followed and read");
        assert!(walk.incomplete.is_some());
        assert_eq!(walk.files_read, 0);

        let fifo = dir.path().join("pipe");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let (sender, receiver) = std::sync::mpsc::channel();
        let matcher = std::sync::Arc::new(matcher);
        let worker = std::sync::Arc::clone(&matcher);
        std::thread::spawn(move || {
            let mut walk = Walk {
                matcher: &worker,
                budget: 1 << 20,
                hits: Vec::new(),
                incomplete: None,
                files_read: 0,
            };
            walk.file(&fifo, Path::new("pipe"), 0);
            let _ = sender.send((walk.incomplete.is_some(), walk.files_read));
        });
        let (stopped, read) = receiver
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("a FIFO must never block the scan");
        assert!(stopped && read == 0);
    }
}
