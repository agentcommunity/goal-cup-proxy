//! The secret scan (PRD-08 section 3, seam S12): known values in every form and the shapes of
//! `shapes.tsv` without the left guard (ledger M1), in records and in raw bytes. A record's DATA is
//! joined per stream through a 4 KiB tail; meta, END and GAP tails, events and the joined SSE and
//! WebSocket deltas are scanned too. A hit is shown only as 12 hex digits of its SHA-256.

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
pub const SHAPES: &str = include_str!("../../goalcup/src/mask/shapes.tsv");
/// What ends just before a glued match when the masker's left guard still masks it (PRD-05
/// section 4: a backslash, `\u`, `\x`, percent or ANSI escape): such a hit is no `glued_shape`.
static ESCAPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:\\+(?:[A-Za-z0-9]|u[0-9A-Fa-f]{4}|x[0-9A-Fa-f]{2})|%[0-9A-Fa-f]{2}|(?:\x1b|\\+(?:u001b|x1b|033|e))\[[0-9;]{0,16}[A-Za-z])$").expect("escape")
});

#[derive(clap::Args)]
pub struct Args {
    /// Entry, run or fetched folders and part files are records; any other file or folder, and
    /// `-` (stdin), is raw bytes.
    #[arg(required = true)]
    paths: Vec<PathBuf>,
    /// Known secret values, one per line.
    #[arg(long, value_name = "FILE")]
    known: Option<PathBuf>,
    /// Accepted: the report is always JSON on stdout.
    #[arg(long)]
    json: bool,
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
pub struct Scan {
    hits: BTreeMap<(usize, &'static str, String, String), u64>,
    masks: BTreeMap<String, u64>,
    fail_closed: u64,
    big: Vec<Value>,
}

/// The last 4 KiB of a stream, where it ends, and per matcher the end of the last hit counted.
#[derive(Default)]
struct Tail(Vec<u8>, u64, HashMap<usize, u64>);

/// An open stream: DATA and delta tails, the pending SSE line or WebSocket message, and MASK
/// spans by kind as (end, length) until one does not touch the next.
#[derive(Default)]
struct Stream {
    sse: bool,
    ws: bool,
    data: Tail,
    deltas: Tail,
    line: Vec<u8>,
    masks: HashMap<String, (u64, u64)>,
}

impl Rules {
    /// The shapes alone. A bad line panics; a test compiles every line.
    pub fn shapes() -> Rules {
        let mut r = Rules::default();
        for l in SHAPES.lines().filter(|l| !l.starts_with('#')) {
            if let [kind, guard, re] = l.splitn(3, '\t').collect::<Vec<_>>()[..] {
                r.rules
                    .push((kind, guard == "1", Regex::new(re).expect("shapes.tsv")));
            }
        }
        r
    }

    /// The shapes and the values of `file`, one per line, kept as they are.
    pub fn new(file: &Path) -> Result<Rules, String> {
        let text = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut r = Rules::shapes();
        for v in text.split(|&b| b == b'\n') {
            let v = v.strip_suffix(b"\r").unwrap_or(v);
            r.values += usize::from(!v.is_empty());
            for (form, n) in forms(v).into_iter().filter(|f| !f.1.is_empty()) {
                r.forms.entry(n).or_insert(form);
            }
        }
        let mut all: Vec<&Vec<u8>> = r.forms.keys().collect();
        all.sort_by_key(|n| std::cmp::Reverse(n.len()));
        let esc = |n: &&Vec<u8>| n.iter().map(|b| format!(r"\x{b:02x}")).collect::<String>();
        let all: Vec<String> = all.iter().map(esc).collect();
        let re = Regex::new(&format!("(?-u:{})", all.join("|"))).map_err(|e| e.to_string())?;
        if !all.is_empty() {
            r.rules.insert(0, ("", false, re));
        }
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

/// Appends, in order, the string values at keys `delta`, `text` and `partial_json` of one JSON
/// event, looking inside objects and arrays (so Anthropic's `delta.text` counts).
pub fn delta_text(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => m.iter().for_each(|(k, v)| match v.as_str() {
            Some(s) if ["delta", "text", "partial_json"].contains(&k.as_str()) => out.push_str(s),
            _ => delta_text(v, out),
        }),
        Value::Array(a) => a.iter().for_each(|v| delta_text(v, out)),
        _ => {}
    }
}

fn parse(b: &[u8]) -> Value {
    serde_json::from_slice(b.trim_ascii()).unwrap_or_default()
}

impl Scan {
    /// Scans `piece` after the tail `t`. A hit counts when it ends in `piece` and starts at or
    /// after the end of the last hit its matcher counted there; a span of `*` only is masked.
    fn feed(&mut self, r: &Rules, t: &mut Tail, piece: &[u8], at: &str) {
        let word = |c: &u8| *c == b'_' || c.is_ascii_alphanumeric();
        let (base, new) = (t.1 - t.0.len() as u64, t.0.len());
        t.0.extend_from_slice(piece);
        t.1 += piece.len() as u64;
        for (i, (kind, guard, re)) in r.rules.iter().enumerate() {
            for m in re.captures_iter(&t.0).filter_map(|c| c.get(1).or(c.get(0))) {
                let (s, b, last) = (m.start(), m.as_bytes(), t.2.entry(i).or_default());
                if m.end() <= new || base + (s as u64) < *last || b.iter().all(|&c| c == b'*') {
                    continue;
                }
                *last = base + m.end() as u64;
                let before = &t.0[s.saturating_sub(32)..s];
                let glued = *guard && before.last().is_some_and(word) && !ESCAPE.is_match(before);
                let list = usize::from(!kind.is_empty()) + usize::from(glued);
                let name = if kind.is_empty() { r.forms[b] } else { kind };
                let key = (list, name, at.to_string(), hex(&Sha256::digest(b)[..6]));
                *self.hits.entry(key).or_default() += 1;
            }
        }
        t.0.drain(..t.0.len().saturating_sub(4096));
    }

    /// One merged MASK span.
    fn span(&mut self, kind: &str, len: u64, at: &str) {
        *self.masks.entry(kind.into()).or_default() += 1;
        if kind == "fail_closed" {
            self.fail_closed += len;
        } else if len > 16 << 10 {
            self.big.push(json!({"kind": kind, "in": at, "len": len}));
        }
    }

    /// Scans one entry's record in one pass. False when a decode error stopped it early.
    pub fn record(&mut self, r: &Rules, e: &Entry) -> io::Result<bool> {
        let (mut name, mut boot, mut whole) = (String::new(), 0, true);
        let mut kinds: Vec<String> = vec![];
        let mut open: HashMap<(u64, u64), Stream> = HashMap::new();
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
                    let s = open.entry((boot, sid)).or_default();
                    (s.sse, s.ws) = (sse && skind == HttpResp, skind == WsDown);
                    Some((format!(".{sid}/meta"), meta))
                }
                Record::Data {
                    sid, flags, bytes, ..
                } => {
                    let at = format!("{name}/{boot}.{sid}");
                    let s = open.entry((boot, sid)).or_default();
                    self.feed(r, &mut s.data, &bytes, &at);
                    let mut text = String::new();
                    if s.ws || s.sse {
                        s.line.extend_from_slice(&bytes);
                    }
                    if s.ws && flags & FLAG_FIN != 0 {
                        delta_text(&parse(&std::mem::take(&mut s.line)), &mut text);
                    }
                    while let Some(i) = s.line.iter().position(|&b| b == b'\n').filter(|_| s.sse) {
                        let l: Vec<u8> = s.line.drain(..=i).collect();
                        if let Some(d) = l.strip_prefix(b"data:") {
                            delta_text(&parse(d), &mut text);
                        }
                    }
                    if !text.is_empty() {
                        self.feed(r, &mut s.deltas, text.as_bytes(), &format!("{at}/deltas"));
                    }
                    None
                }
                Record::End { sid, tail, .. } => {
                    for (k, (_, len)) in open.remove(&(boot, sid)).unwrap_or_default().masks {
                        self.span(&k, len, &format!("{name}/{boot}.{sid}"));
                    }
                    Some((format!(".{sid}"), tail))
                }
                Record::Gap { sid, tail, .. } => Some((format!(".{sid}"), tail)),
                Record::Mask {
                    sid,
                    off,
                    len,
                    rule,
                } => {
                    let kind = kinds.get(rule as usize).cloned();
                    let kind = kind.unwrap_or_else(|| format!("other({rule})"));
                    let s = open.entry((boot, sid)).or_default();
                    let m = s.masks.entry(kind.clone()).or_insert((off, 0));
                    if m.0 != off {
                        let done = std::mem::replace(m, (off, 0)).1;
                        self.span(&kind, done, &format!("{name}/{boot}.{sid}"));
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
            if let Some((sid, b)) = one {
                self.feed(r, &mut Tail::default(), &b, &format!("{name}/{boot}{sid}"));
            }
        }
        for ((b, sid), s) in open {
            for (k, (_, len)) in s.masks {
                self.span(&k, len, &format!("{name}/{b}.{sid}"));
            }
        }
        Ok(whole)
    }

    /// One path: a record (folder or part file), else raw bytes; `-` is stdin. Any other folder is
    /// walked, each path in it taken the same way, so an entry's `rec/` is never read raw.
    /// Returns how it was read and whether it was read whole.
    fn path(&mut self, r: &Rules, p: &Path) -> io::Result<(&'static str, bool)> {
        let n = p.file_name().unwrap_or_default().to_string_lossy();
        let part = n == input::FETCHED || n.starts_with("rec-") && n.ends_with(".gcr.zst");
        let mut raw: Box<dyn Read> = match input::resolve(p, false, false) {
            _ if p.as_os_str() == "-" => Box::new(io::stdin().lock()),
            Ok(i) if p.is_dir() || part => {
                let mut whole = true;
                for e in &i.entries {
                    whole &= self.record(r, e)?;
                }
                return Ok(("record", whole));
            }
            _ if p.is_dir() => {
                let all = std::fs::read_dir(p)?.map(|e| e.map(|e| e.path()));
                let (mut all, mut whole) = (all.collect::<io::Result<Vec<_>>>()?, true);
                all.sort();
                for f in all {
                    whole &= self.path(r, &f)?.1;
                }
                return Ok(("raw", whole));
            }
            _ => Box::new(std::fs::File::open(p)?),
        };
        let (mut t, mut b, at) = (Tail::default(), vec![0; 1 << 16], p.display().to_string());
        loop {
            match raw.read(&mut b)? {
                0 => return Ok(("raw", true)),
                n => self.feed(r, &mut t, &b[..n], &at),
            }
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
        let warn = [Res::Pass, Res::Warn][usize::from(g > 0)];
        let res = if k + s + b > 0 { Res::Fail } else { warn };
        let why = format!("hits: {k} known, {s} shape, {g} glued; {b} MASK spans over 16 KiB");
        let [k, s, g] = lists;
        let v = json!({"known_hits": k, "shape_hits": s, "glued": g, "masks": self.masks,
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
/// input was read only in part, else 0; 3 when an input or the known file cannot be read.
pub fn run(a: &Args) -> ExitCode {
    let k = a.known.as_deref();
    let r = match k.map_or_else(|| Ok(Rules::shapes()), Rules::new) {
        Ok(r) => r,
        Err(e) => return no_report(&e),
    };
    let (mut s, mut inputs) = (Scan::default(), vec![]);
    for p in &a.paths {
        let (how, whole) = match s.path(&r, p) {
            Ok(x) => x,
            Err(e) => return no_report(&format!("{}: {e}", p.display())),
        };
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
    ExitCode::from(exit)
}

fn no_report(e: &str) -> ExitCode {
    eprintln!("goalcup-verify: {e}");
    ExitCode::from(3)
}
