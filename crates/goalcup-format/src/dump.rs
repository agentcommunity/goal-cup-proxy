//! One JSON line per record, for people (`goalcup-verify dump`, PRD-08). Numbers stay numbers,
//! reasons and kinds are names, wall times are UTC text, payloads are text when they are UTF-8
//! (else `hex`). MASK rules are named from the last `start` event's `mask_rules`.

use crate::codec::Record;
use crate::read::Item;
use crate::{FLAG_FIN, hex, ws_opcode_name};
use serde_json::{Value, json};
use std::io::{self, Read, Write};

/// Turns records into dump lines. Holds the `mask_rules` of the last `start` event.
#[derive(Default)]
pub struct Dumper {
    mask_rules: Vec<String>,
}

impl Dumper {
    /// The dump line of one record. Keys that do not apply (a zero `conn`, an empty tail) are
    /// left out.
    pub fn line(&mut self, it: &Item) -> Value {
        let mut v = match &it.rec {
            Record::Part {
                entry,
                boot,
                part,
                raw_offset,
                wall_ns,
                mono_ns,
                next_link,
                ..
            } => json!({"k": "part", "entry": entry, "boot": boot, "part": part,
                "raw_offset": raw_offset, "wall": wall(*wall_ns), "mono": mono_ns,
                "next_link": next_link}),
            Record::Open {
                sid,
                t,
                skind,
                conn,
                req,
                meta,
            } => json!({"k": "open", "sid": sid, "t": t, "skind": skind.name(),
                "conn": (*conn != 0).then_some(conn), "req": (*req != 0).then_some(req),
                "meta": json_or_text(meta)}),
            Record::Data {
                sid,
                t,
                off,
                flags,
                bytes,
            } => {
                let op = flags >> 4;
                let op = ws_opcode_name(op).map_or_else(|| op.into(), Value::from);
                let fl = json!({"fin": flags & FLAG_FIN != 0, "op": op});
                let s = std::str::from_utf8(bytes);
                json!({"k": "data", "sid": sid, "t": t, "off": off,
                    "flags": (*flags != 0).then_some(fl), "len": bytes.len(),
                    "text": s.ok(), "hex": s.is_err().then(|| hex(bytes))})
            }
            Record::End {
                sid,
                t,
                reason,
                seen,
                tail,
            } => json!({"k": "end", "sid": sid, "t": t, "reason": reason.name(), "seen": seen,
                "tail": opt_json(tail)}),
            Record::Gap {
                sid,
                reason,
                start_wall,
                end_wall,
                start_mono,
                end_mono,
                off,
                lost,
                tail,
            } => json!({"k": "gap", "sid": sid, "reason": reason.name(),
                "start": wall(*start_wall), "end": wall(*end_wall), "start_mono": start_mono,
                "end_mono": end_mono, "off": off, "lost": lost, "tail": opt_json(tail)}),
            Record::Mask {
                sid,
                off,
                len,
                rule,
            } => {
                let name = usize::try_from(*rule)
                    .ok()
                    .and_then(|i| self.mask_rules.get(i));
                json!({"k": "mask", "sid": sid, "off": off, "len": len,
                    "rule": name.map_or_else(|| (*rule).into(), |n| Value::from(n.as_str()))})
            }
            Record::Event { t, .. } => json!({"k": "event", "t": t}),
            Record::Link {
                seq,
                wall_ns,
                mono_ns,
                hash,
            } => json!({"k": "link", "seq": seq, "wall": wall(*wall_ns), "mono": mono_ns,
                "hash": hex(hash), "hash_ok": (it.link_ok == Some(false)).then_some(false)}),
            Record::Unknown { kind, body } => {
                json!({"k": "unknown", "kind": kind, "len": body.len()})
            }
        };
        let m = v.as_object_mut().expect("an object");
        m.retain(|_, x| !x.is_null());
        m.insert("o".into(), it.off.into());
        if let Record::Event { json, .. } = &it.rec {
            match json_or_text(json) {
                Value::Object(ev) => {
                    if ev.get("ev").and_then(Value::as_str) == Some("start")
                        && let Some(Value::Array(r)) = ev.get("mask_rules")
                    {
                        let name = |v: &Value| v.as_str().unwrap_or_default().to_string();
                        self.mask_rules = r.iter().map(name).collect();
                    }
                    for (k, x) in ev {
                        if !matches!(k.as_str(), "o" | "k" | "t") {
                            m.insert(k, x);
                        }
                    }
                }
                other => {
                    m.insert("json".into(), other);
                }
            }
        }
        v
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
        writeln!(
            out,
            "{}",
            json!({"k": "tail", "open_frame": tail.open_frame, "truncated_at": tail.truncated})
        )?;
    }
    Ok(())
}

/// A tail as a value, or null when there is none.
fn opt_json(b: &[u8]) -> Option<Value> {
    (!b.is_empty()).then(|| json_or_text(b))
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
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{frac}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}
