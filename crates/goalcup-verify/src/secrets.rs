//! The secret scan (PRD-08 section 3, seam S12): known values in every form and the shapes of
//! `shapes.tsv` without the left guard (ledger M1), in records and in raw bytes. A record's DATA is
//! joined per stream through a 4 KiB tail, cut at a GAP or an offset jump; meta, END and GAP
//! tails, events and the joined SSE and WebSocket deltas are scanned too. A hit is shown only as
//! 12 hex digits of its SHA-256.

use crate::input::{self, Entry};
use crate::integrity::{Check, Res};
use base64::{Engine, engine::general_purpose::STANDARD};
use goalcup_format::StreamKind::{HttpResp, WsDown};
use goalcup_format::{FLAG_FIN, Record, hex};
use regex::bytes::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::LazyLock;

/// Seam S12: kind, guard and regex per data line.
const SHAPES: &str = include_str!("../../goalcup/src/mask/shapes.tsv");
/// What ends just before a glued match when the masker's left guard still masks it (PRD-05
/// section 4: a backslash, `\u`, `\x`, percent or ANSI escape): such a hit is no `glued_shape`.
static ESCAPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:\\+(?:[A-Za-z0-9]|u[0-9A-Fa-f]{4}|x[0-9A-Fa-f]{2})|%[0-9A-Fa-f]{2}|(?:\x1b|\\+(?:u001b|x1b|033|e))\[[0-9;]{0,16}[A-Za-z])$").expect("escape")
});
/// The bytes a tail gathers before it is scanned again with the 4 KiB it keeps.
const BATCH: usize = 64 << 10;

#[derive(clap::Args)]
pub struct Args {
    /// Part files are records; a folder holding an entry has its record decoded and its files
    /// other than `rec/` read raw; other folders are walked; any other file, and `-` (stdin), is
    /// raw bytes.
    #[arg(required = true)]
    paths: Vec<PathBuf>,
    /// Known secret values, one per line.
    #[arg(long, value_name = "FILE")]
    known: Option<PathBuf>,
}

/// The matchers: kind, guard and regex, the known values first (kind `""`, one alternation of
/// every form, longest first), then `shapes.tsv` in order; the form of each known needle.
#[derive(Default)]
pub struct Rules {
    rules: Vec<(&'static str, bool, Regex)>,
    forms: HashMap<Vec<u8>, &'static str>,
    values: usize,
}

/// One scan's findings: hits by (list, form or kind, where, hash prefix), MASK spans by kind,
/// `fail_closed` bytes, and the other spans over 16 KiB.
#[derive(Default)]
struct Scan {
    hits: BTreeMap<(usize, &'static str, String, String), u64>,
    masks: BTreeMap<String, u64>,
    fail_closed: u64,
    big: Vec<Value>,
}

/// Where a run of bytes is named; the last 4 KiB scanned, then the bytes not scanned yet; where
/// they end; how many are scanned; per matcher the end of the last hit counted.
#[derive(Default)]
struct Tail {
    at: String,
    buf: Vec<u8>,
    end: u64,
    seen: usize,
    last: Vec<u64>,
}

/// An open stream: DATA and delta tails, the pending SSE line or WebSocket message, and MASK
/// spans by rule as (end, length) until one does not touch the next.
#[derive(Default)]
struct Stream {
    sse: bool,
    ws: bool,
    data: Tail,
    deltas: Tail,
    line: Vec<u8>,
    masks: BTreeMap<u64, (u64, u64)>,
}

impl Rules {
    /// The shapes alone. A line that is not kind, guard `0` or `1`, and a regex panics, so every
    /// scan test proves the file.
    pub fn shapes() -> Rules {
        let mut r = Rules::default();
        for l in SHAPES
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        {
            let [kind, g @ ("0" | "1"), re] = l.splitn(3, '\t').collect::<Vec<_>>()[..] else {
                panic!("shapes.tsv: {l}");
            };
            let re = Regex::new(re).expect("shapes.tsv");
            r.rules.push((kind, g == "1", re));
        }
        r
    }

    /// The shapes and the values of `file`, one per line, trimmed; blank lines are skipped.
    pub fn new(file: &Path) -> Result<Rules, String> {
        let text = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut r = Rules::shapes();
        for v in text.split(|&b| b == b'\n').map(<[u8]>::trim_ascii) {
            r.values += usize::from(!v.is_empty());
            for (form, n) in forms(v).into_iter().filter(|f| !f.1.is_empty()) {
                r.forms.entry(n).or_insert(form);
            }
        }
        if r.forms.is_empty() {
            return Ok(r);
        }
        let mut all: Vec<&Vec<u8>> = r.forms.keys().collect();
        all.sort_by_key(|n| std::cmp::Reverse(n.len()));
        let esc = |n: &&Vec<u8>| n.iter().map(|b| format!(r"\x{b:02x}")).collect::<String>();
        let all: Vec<String> = all.iter().map(esc).collect();
        // The regex error could quote the values, so it is not shown.
        let re = Regex::new(&format!("(?-u:{})", all.join("|")));
        let re = re.map_err(|_| "the known values do not compile")?;
        r.rules.insert(0, ("", false, re));
        Ok(r)
    }
}

/// A value's forms: raw; base64 at each alignment, only the characters its neighbours cannot
/// change; percent-encoded (every byte but the unreserved), upper and lower hex; JSON-escaped.
fn forms(v: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
    let mut out = vec![("raw", v.to_vec())];
    for (pad, form) in ["base64_0", "base64_1", "base64_2"].into_iter().enumerate() {
        let b = STANDARD.encode([&[0; 2][..pad], v].concat());
        let (from, to) = ((8 * pad).div_ceil(6), 8 * (pad + v.len()) / 6);
        let n = b.as_bytes().get(from..to).unwrap_or_default();
        out.push((form, n.to_vec()));
    }
    for d in [b"0123456789ABCDEF", b"0123456789abcdef"] {
        let pct = |&b: &u8| match b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            true => vec![b],
            false => vec![b'%', d[usize::from(b >> 4)], d[usize::from(b & 15)]],
        };
        out.push(("percent", v.iter().flat_map(pct).collect()));
    }
    if let Ok(s) = std::str::from_utf8(v) {
        let j = json!(s).to_string();
        out.push(("json", j.as_bytes()[1..j.len() - 1].to_vec()));
    }
    out
}

/// Appends, in order, the streamed text of one JSON event: the string values at keys `delta`,
/// `text` and `partial_json` anywhere (Responses, Anthropic `delta.text`), and at `content`,
/// `reasoning_content`, `thinking` and `arguments` inside a `delta` object (Chat Completions
/// `choices[].delta`, its `tool_calls[].function.arguments`, Anthropic thinking).
pub fn delta_text(v: &Value, out: &mut String) {
    join(v, false, false, out);
}

/// Like [`delta_text`] but only the answer: no reasoning, thinking or tool-argument pieces, so
/// the joined text equals the text a later request resends as the assistant turn.
pub fn answer_text(v: &Value, out: &mut String) {
    join(v, false, true, out);
}

fn join(v: &Value, in_delta: bool, answer: bool, out: &mut String) {
    let [any, inner]: [&[&str]; 2] = match answer {
        true => [&["delta", "text"], &["content"]],
        false => [
            &["delta", "text", "partial_json"],
            &["content", "reasoning_content", "thinking", "arguments"],
        ],
    };
    match v {
        Value::Object(m) => {
            let kind = m.get("type").and_then(Value::as_str).unwrap_or_default();
            // Reasoning items and Gemini `"thought": true` parts are not the answer.
            let skip = kind.contains("reasoning") || kind.contains("thinking");
            if answer && (skip || m.get("thought") == Some(&Value::Bool(true))) {
                return;
            }
            m.iter().for_each(|(k, v)| match v.as_str() {
                Some(s) if any.contains(&&**k) || in_delta && inner.contains(&&**k) => {
                    out.push_str(s);
                }
                _ => join(v, in_delta || k == "delta", answer, out),
            });
        }
        Value::Array(a) => a.iter().for_each(|v| join(v, in_delta, answer, out)),
        _ => {}
    }
}

fn parse(b: &[u8]) -> Value {
    serde_json::from_slice(b.trim_ascii()).unwrap_or_default()
}

/// The open streams by boot and sid.
type Open = BTreeMap<(u64, u64), Stream>;

/// The open stream `sid` of boot `b`, named `<entry>/<boot>.<sid>`.
fn stream<'a>(open: &'a mut Open, name: &str, b: u64, sid: u64) -> &'a mut Stream {
    open.entry((b, sid)).or_insert_with(|| {
        let at = format!("{name}/{b}.{sid}");
        Stream {
            data: Tail::new(at.clone(), 0),
            deltas: Tail::new(at + "/deltas", 0),
            ..Stream::default()
        }
    })
}

impl Tail {
    /// An empty tail named `at` that ends at `end`.
    fn new(at: String, end: u64) -> Tail {
        Tail {
            at,
            end,
            ..Tail::default()
        }
    }
}

impl Scan {
    /// Scans a tail's new bytes after the 4 KiB it keeps. A hit counts when it ends in the new
    /// bytes and starts at or after the end of the last hit its matcher counted; a span of `*`
    /// only is masked.
    fn scan(&mut self, r: &Rules, t: &mut Tail) {
        if t.seen == t.buf.len() {
            return;
        }
        let word = |c: &u8| *c == b'_' || c.is_ascii_alphanumeric();
        let base = t.end - t.buf.len() as u64;
        t.last.resize(r.rules.len(), 0);
        let buf = &t.buf;
        for (i, (kind, guard, re)) in r.rules.iter().enumerate() {
            for m in re.captures_iter(buf).filter_map(|c| c.get(1).or(c.get(0))) {
                let (s, b) = (m.start(), m.as_bytes());
                let masked = b.iter().all(|&c| c == b'*');
                if m.end() <= t.seen || base + (s as u64) < t.last[i] || masked {
                    continue;
                }
                t.last[i] = base + m.end() as u64;
                let before = &buf[s.saturating_sub(32)..s];
                let glued = *guard && before.last().is_some_and(word) && !ESCAPE.is_match(before);
                let (list, name) = match (kind.is_empty(), glued) {
                    (true, _) => (0, r.forms[b]),
                    (false, false) => (1, *kind),
                    (false, true) => (2, *kind),
                };
                let key = (list, name, t.at.clone(), hex(&Sha256::digest(b)[..6]));
                *self.hits.entry(key).or_default() += 1;
            }
        }
        t.buf.drain(..t.buf.len().saturating_sub(4096));
        t.seen = t.buf.len();
    }

    /// Adds `piece` to a tail; scans once `BATCH` new bytes wait.
    fn feed(&mut self, r: &Rules, t: &mut Tail, piece: &[u8]) {
        t.buf.extend_from_slice(piece);
        t.end += piece.len() as u64;
        if t.buf.len() - t.seen >= BATCH {
            self.scan(r, t);
        }
    }

    /// Scans what a stream holds and joins nothing of it to what follows: after a GAP or an
    /// offset jump nothing is before a match (PRD-05 section 4).
    fn cut(&mut self, r: &Rules, s: &mut Stream) {
        for t in [&mut s.data, &mut s.deltas] {
            self.scan(r, t);
            *t = Tail::new(std::mem::take(&mut t.at), t.end);
        }
        s.line.clear();
    }

    /// Ends a stream: scans what it holds and counts its open MASK spans.
    fn close(&mut self, r: &Rules, kinds: &[String], mut s: Stream) {
        self.cut(r, &mut s);
        for (rule, (_, len)) in s.masks {
            self.span(kinds, rule, len, &s.data.at);
        }
    }

    /// One merged MASK span, its rule named by the start event's `mask_rules`.
    fn span(&mut self, kinds: &[String], rule: u64, len: u64, at: &str) {
        let kind = kinds.get(rule as usize).cloned();
        let kind = kind.unwrap_or_else(|| format!("other({rule})"));
        if kind == "fail_closed" {
            self.fail_closed += len;
        } else if len > 16 << 10 {
            self.big.push(json!({"kind": kind, "in": at, "len": len}));
        }
        *self.masks.entry(kind).or_default() += 1;
    }

    /// Scans one entry's record in one pass. False when a decode error stopped it early.
    fn record(&mut self, r: &Rules, e: &Entry) -> io::Result<bool> {
        let (mut name, mut boot, mut whole) = (String::new(), 0, true);
        let mut kinds: Vec<String> = vec![];
        let mut open = Open::new();
        for it in input::open(e)?.0 {
            let Ok(it) = it else {
                whole = false;
                break;
            };
            let one = match it.rec {
                Record::Part { entry, boot: b, .. } => {
                    (name, boot) = (entry, b);
                    None
                }
                Record::Open {
                    sid, skind, meta, ..
                } => {
                    let h = parse(&meta)["headers"].to_string().to_ascii_lowercase();
                    let sse = h.contains(r#"["content-type","text/event-stream"#);
                    let s = stream(&mut open, &name, boot, sid);
                    (s.sse, s.ws) = (sse && skind == HttpResp, skind == WsDown);
                    Some((format!(".{sid}/meta"), meta))
                }
                Record::Data {
                    sid,
                    off,
                    flags,
                    bytes,
                    ..
                } => {
                    let s = stream(&mut open, &name, boot, sid);
                    if s.data.end != off {
                        self.cut(r, s);
                        s.data.end = off;
                    }
                    self.feed(r, &mut s.data, &bytes);
                    let mut text = String::new();
                    if s.ws || s.sse {
                        s.line.extend_from_slice(&bytes);
                    }
                    if s.ws && flags & FLAG_FIN != 0 {
                        delta_text(&parse(&std::mem::take(&mut s.line)), &mut text);
                    }
                    if s.sse {
                        while let Some(i) = s.line.iter().position(|&b| b == b'\n') {
                            let l: Vec<u8> = s.line.drain(..=i).collect();
                            if let Some(d) = l.strip_prefix(b"data:") {
                                delta_text(&parse(d), &mut text);
                            }
                        }
                    }
                    self.feed(r, &mut s.deltas, text.as_bytes());
                    None
                }
                Record::End { sid, tail, .. } => {
                    if let Some(s) = open.remove(&(boot, sid)) {
                        self.close(r, &kinds, s);
                    }
                    Some((format!(".{sid}"), tail))
                }
                Record::Gap { sid, tail, .. } => {
                    for ((b, n), s) in &mut open {
                        if *b == boot && (sid == 0 || *n == sid) {
                            self.cut(r, s);
                        }
                    }
                    Some((format!(".{sid}"), tail))
                }
                Record::Mask {
                    sid,
                    off,
                    len,
                    rule,
                } => {
                    let s = stream(&mut open, &name, boot, sid);
                    let m = s.masks.entry(rule).or_insert((off, 0));
                    if m.0 != off {
                        let done = std::mem::replace(m, (off, 0)).1;
                        self.span(&kinds, rule, done, &s.data.at);
                    }
                    *m = (off + len, m.1 + len);
                    None
                }
                Record::Event { json, .. } => {
                    let v = parse(&json);
                    if v["ev"] == "start" {
                        let k = v["mask_rules"].as_array().cloned().unwrap_or_default();
                        let k = k.iter().map(|k| k.as_str().unwrap_or("").to_string());
                        kinds = k.collect();
                    }
                    Some((".event".into(), json))
                }
                _ => None,
            };
            if let Some((sid, buf)) = one {
                let mut t = Tail::new(format!("{name}/{boot}{sid}"), buf.len() as u64);
                t.buf = buf;
                self.scan(r, &mut t);
            }
        }
        for (_, s) in open {
            self.close(r, &kinds, s);
        }
        Ok(whole)
    }

    /// Scans one path: whether a record was decoded in it, and whether it was read whole. `-` is
    /// stdin; a part file is a record; a folder holding an entry has its record decoded, then is
    /// walked without its part files and `rec/`, the recorder's own state that is never
    /// uploaded (index files, the run CA key); a `rec` folder given is its record only; another
    /// folder holding part files has its record decoded and its other files read raw; any
    /// other folder is walked; anything else is raw. In a walk, a path that cannot be read, and
    /// a symlink to a folder, leave the input partial; what is neither file nor folder is
    /// skipped. Only a top-level path is an error.
    fn path(&mut self, r: &Rules, p: &Path, top: bool) -> io::Result<(bool, bool)> {
        let n = p.file_name().unwrap_or_default().to_string_lossy();
        let part = n == input::FETCHED || n.starts_with("rec-") && n.ends_with(".gcr.zst");
        let mut raw: Box<dyn Read> = if p.as_os_str() == "-" {
            Box::new(io::stdin().lock())
        } else if p.is_dir() {
            let e = input::entry_at(p);
            let mut whole = e
                .as_ref()
                .is_none_or(|e| self.record(r, e).unwrap_or(false));
            let mut rec = e.is_some();
            let skip = |q: &Path| {
                let in_rec = |e: &Entry| q.starts_with(e.dir.join("rec"));
                e.as_ref()
                    .is_some_and(|e| in_rec(e) || e.files.iter().any(|f| f.0 == q))
            };
            let mut all = std::fs::read_dir(p)?.collect::<io::Result<Vec<_>>>()?;
            all.retain(|d| !skip(&d.path()));
            all.sort_by_key(std::fs::DirEntry::file_name);
            for d in all {
                let link = d.file_type().is_ok_and(|t| t.is_symlink()) && d.path().is_dir();
                let one = match link {
                    true => Ok((false, false)),
                    false => self.path(r, &d.path(), false),
                };
                let (got, w) = one.unwrap_or((false, false));
                (rec, whole) = (rec || got, whole && w);
            }
            return Ok((rec, whole));
        } else if !top && !p.metadata()?.is_file() {
            return Ok((false, true));
        } else if part {
            let dir = p.parent().unwrap_or(p).to_path_buf();
            let e = Entry {
                dir,
                files: vec![(p.to_path_buf(), None)],
            };
            return Ok((true, self.record(r, &e)?));
        } else {
            Box::new(std::fs::File::open(p)?)
        };
        let (mut t, mut b) = (Tail::new(p.display().to_string(), 0), vec![0; BATCH]);
        loop {
            let n = raw.read(&mut b);
            if let Ok(n @ 1..) = n {
                self.feed(r, &mut t, &b[..n]);
                continue;
            }
            self.scan(r, &mut t);
            return n.map(|_| (false, true));
        }
    }

    /// The result, the report keys, and one plain line with counts only.
    fn result(&self) -> (Res, Map<String, Value>, String) {
        let mut lists = [vec![], vec![], vec![]];
        for ((list, name, at, sha12), n) in &self.hits {
            let key = if *list == 0 { "form" } else { "kind" };
            lists[*list].push(json!({key: name, "in": at, "sha12": sha12, "n": n}));
        }
        let ([k, s, g], b) = (lists.each_ref().map(Vec::len), self.big.len());
        let res = match (k + s + b, g) {
            (1.., _) => Res::Fail,
            (0, 1..) => Res::Warn,
            _ => Res::Pass,
        };
        let why =
            format!("hits: {k} known, {s} shape, {g} glued_shape; {b} MASK spans over 16 KiB");
        let [k, s, g] = lists;
        let v = json!({"known_hits": k, "shape_hits": s, "glued_shape": g, "masks": self.masks,
            "mask_bytes": {"fail_closed": self.fail_closed}, "big_masks": self.big});
        (res, v.as_object().cloned().unwrap_or_default(), why)
    }
}

/// The `secrets` check of one entry (`check --known`). A record that cannot be read is the
/// integrity checks' to report.
pub fn check(r: &Rules, e: &Entry) -> Check {
    let mut s = Scan::default();
    let whole = s.record(r, e).unwrap_or(false);
    let (res, facts, why) = s.result();
    let mut c = Check::new("secrets", None, res, &why);
    if !whole {
        c.why += "; the record was read only in part";
    }
    c.facts = facts;
    c
}

/// `secrets`: the JSON report on stdout and one line on stderr. Exit 2 on a fail, else 1 when an
/// input was read only in part, else 0. The error (exit 3, no report): a path given, or the
/// known file, cannot be read.
pub fn run(a: &Args) -> Result<ExitCode, String> {
    let r = a
        .known
        .as_deref()
        .map_or_else(|| Ok(Rules::shapes()), Rules::new)?;
    let (mut s, mut inputs) = (Scan::default(), vec![]);
    for p in &a.paths {
        let (rec, whole) = s
            .path(&r, p, true)
            .map_err(|e| format!("{}: {e}", p.display()))?;
        let how = if rec { "record" } else { "raw" };
        let read = if whole { "whole" } else { "partial" };
        inputs.push(json!({"path": p.display().to_string(), "as": how, "read": read}));
    }
    let (res, mut v, why) = s.result();
    let partial = inputs.iter().any(|i| i["read"] == "partial");
    let exit = u8::from(partial).max(2 * u8::from(res == Res::Fail));
    let head = json!({"verifier": env!("CARGO_PKG_VERSION"), "result": res.name(), "exit": exit,
        "known_values": r.values, "inputs": inputs});
    v.extend(head.as_object().cloned().unwrap_or_default());
    let _ = writeln!(io::stdout(), "{}", Value::Object(v));
    eprintln!("goalcup-verify: secrets {}: {why}", res.name());
    Ok(ExitCode::from(exit))
}
