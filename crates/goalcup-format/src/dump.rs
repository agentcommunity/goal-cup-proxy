//! One JSON line per record, for people (`goalcup-verify dump`, PRD-08). Numbers stay numbers,
//! reasons and kinds are names, wall times are UTC text, payloads are text when they are UTF-8
//! (else `hex`). MASK rules are named from the last `start` event's `mask_rules`.

use crate::codec::Record;
use crate::read::Item;
use crate::{FLAG_FIN, hex, ws_opcode_name};
use serde_json::{Map, Value, json};
use std::io::{self, Read, Write};

/// Turns records into dump lines. Holds the `mask_rules` of the last `start` event.
#[derive(Default)]
pub struct Dumper {
    mask_rules: Vec<String>,
}

impl Dumper {
    /// The dump line of one record.
    pub fn line(&mut self, it: &Item) -> Value {
        let mut m = Map::new();
        m.insert("o".into(), it.off.into());
        let mut put = |k: &str, v: Value| {
            m.insert(k.into(), v);
        };
        match &it.rec {
            Record::Part { entry, boot, part, raw_offset, wall_ns, mono_ns, next_link, .. } => {
                put("k", "part".into());
                put("entry", entry.as_str().into());
                put("boot", (*boot).into());
                put("part", (*part).into());
                put("raw_offset", (*raw_offset).into());
                put("wall", wall(*wall_ns).into());
                put("mono", (*mono_ns).into());
                put("next_link", (*next_link).into());
            }
            Record::Open { sid, t, skind, conn, req, meta } => {
                put("k", "open".into());
                put("sid", (*sid).into());
                put("t", (*t).into());
                put("skind", skind.name().into());
                if *conn != 0 {
                    put("conn", (*conn).into());
                }
                if *req != 0 {
                    put("req", (*req).into());
                }
                put("meta", json_or_text(meta));
            }
            Record::Data { sid, t, off, flags, bytes } => {
                put("k", "data".into());
                put("sid", (*sid).into());
                put("t", (*t).into());
                put("off", (*off).into());
                if *flags != 0 {
                    let op = flags >> 4;
                    let op = ws_opcode_name(op).map_or_else(|| op.into(), Value::from);
                    put("flags", json!({"fin": flags & FLAG_FIN != 0, "op": op}));
                }
                put("len", bytes.len().into());
                match std::str::from_utf8(bytes) {
                    Ok(s) => put("text", s.into()),
                    Err(_) => put("hex", hex(bytes).into()),
                }
            }
            Record::End { sid, t, reason, seen, tail } => {
                put("k", "end".into());
                put("sid", (*sid).into());
                put("t", (*t).into());
                put("reason", reason.name().into());
                put("seen", (*seen).into());
                if !tail.is_empty() {
                    put("tail", json_or_text(tail));
                }
            }
            Record::Gap { sid, reason, start_wall, end_wall, start_mono, end_mono, off, lost, tail } => {
                put("k", "gap".into());
                put("sid", (*sid).into());
                put("reason", reason.name().into());
                put("start", wall(*start_wall).into());
                put("end", wall(*end_wall).into());
                put("start_mono", (*start_mono).into());
                put("end_mono", (*end_mono).into());
                put("off", (*off).into());
                put("lost", (*lost).into());
                if !tail.is_empty() {
                    put("tail", json_or_text(tail));
                }
            }
            Record::Mask { sid, off, len, rule } => {
                put("k", "mask".into());
                put("sid", (*sid).into());
                put("off", (*off).into());
                put("len", (*len).into());
                let name = usize::try_from(*rule).ok().and_then(|i| self.mask_rules.get(i));
                put("rule", name.map_or_else(|| (*rule).into(), |n| n.as_str().into()));
            }
            Record::Event { t, json } => {
                put("k", "event".into());
                put("t", (*t).into());
                match json_or_text(json) {
                    Value::Object(ev) => {
                        if ev.get("ev").and_then(Value::as_str) == Some("start")
                            && let Some(Value::Array(r)) = ev.get("mask_rules")
                        {
                            self.mask_rules = r.iter().map(|v| v.as_str().unwrap_or_default().to_string()).collect();
                        }
                        for (k, v) in ev {
                            if !matches!(k.as_str(), "o" | "k" | "t") {
                                put(&k, v);
                            }
                        }
                    }
                    other => put("json", other),
                }
            }
            Record::Link { seq, wall_ns, mono_ns, hash } => {
                put("k", "link".into());
                put("seq", (*seq).into());
                put("wall", wall(*wall_ns).into());
                put("mono", (*mono_ns).into());
                put("hash", hex(hash).into());
                if it.link_ok == Some(false) {
                    put("hash_ok", false.into());
                }
            }
            Record::Unknown { kind, body } => {
                put("k", "unknown".into());
                put("kind", (*kind).into());
                put("len", body.len().into());
            }
        }
        Value::Object(m)
    }
}

/// Dumps a compressed record stream, one line per record, then a last line when the stream
/// ended inside an open frame or in a cut record.
pub fn dump<R: Read, W: Write>(r: R, out: &mut W) -> io::Result<()> {
    let mut rd = crate::read::decode(r)?;
    let mut d = Dumper::default();
    for it in rd.by_ref() {
        writeln!(out, "{}", d.line(&it?))?;
    }
    let tail = rd.tail();
    if tail.open_frame || tail.truncated.is_some() {
        writeln!(out, "{}", json!({"k": "tail", "open_frame": tail.open_frame, "truncated_at": tail.truncated}))?;
    }
    Ok(())
}

/// A JSON body as a value, or as text when it does not parse.
fn json_or_text(b: &[u8]) -> Value {
    serde_json::from_slice(b).unwrap_or_else(|_| String::from_utf8_lossy(b).into_owned().into())
}

/// Unix nanoseconds as UTC text: `2026-11-07T00:00:00Z`, with 3, 6 or 9 fraction digits when
/// they are not zero.
pub fn wall(ns: u64) -> String {
    let (secs, frac) = (ns / 1_000_000_000, ns % 1_000_000_000);
    let (days, s) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let frac = match frac {
        0 => String::new(),
        f if f % 1_000_000 == 0 => format!(".{:03}", f / 1_000_000),
        f if f % 1_000 == 0 => format!(".{:06}", f / 1_000),
        f => format!(".{f:09}"),
    };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{frac}Z", s / 3600, s / 60 % 60, s % 60)
}
