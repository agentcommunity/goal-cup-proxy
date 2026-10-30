//! Framing and records (sections 2 and 3): `kind` u8, `len` varint, body.
//!
//! A [`Record`] carries absolute times on the record clock. [`Codec`] holds the running time, so
//! OPEN, DATA, END and EVENT are written with `dt` (signed, from the previous timed record in file
//! order) and read back as absolute times. PART, LINK and GAP carry their time in full and set the
//! running time (a GAP's time is its `end_mono`); MASK carries none and leaves it alone.

use crate::{EndReason, GapReason, MAGIC, MAX_BODY, StreamKind, VERSION, invalid, kind};
use std::io;

/// One record. JSON bodies (`meta`, `json`, `tail`) are the compact UTF-8 bytes as written; an
/// empty `tail` means no tail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    /// First record of every part file. Sets the time base.
    Part {
        /// `e` and 16 hex.
        entry: String,
        boot: u64,
        part: u64,
        raw_offset: u64,
        wall_ns: u64,
        mono_ns: u64,
        boot_id: [u8; 16],
        next_link: u64,
        /// The hash of the last LINK before this part file (32 zero bytes for an entry's first).
        prev: [u8; 32],
    },
    Open {
        sid: u64,
        t: u64,
        skind: StreamKind,
        conn: u64,
        req: u64,
        meta: Vec<u8>,
    },
    Data {
        sid: u64,
        t: u64,
        off: u64,
        flags: u8,
        bytes: Vec<u8>,
    },
    End {
        sid: u64,
        t: u64,
        reason: EndReason,
        seen: u64,
        tail: Vec<u8>,
    },
    /// `lost` is bytes lost plus one: 0 unknown, 1 none.
    Gap {
        sid: u64,
        reason: GapReason,
        start_wall: u64,
        end_wall: u64,
        start_mono: u64,
        end_mono: u64,
        off: u64,
        lost: u64,
        tail: Vec<u8>,
    },
    Mask {
        sid: u64,
        off: u64,
        len: u64,
        rule: u64,
    },
    Event {
        t: u64,
        json: Vec<u8>,
    },
    Link {
        seq: u64,
        wall_ns: u64,
        mono_ns: u64,
        hash: [u8; 32],
    },
    /// A kind this build does not know, kept so a reader can skip it by its length.
    Unknown {
        kind: u8,
        body: Vec<u8>,
    },
}

impl Record {
    pub fn kind(&self) -> u8 {
        match self {
            Record::Part { .. } => kind::PART,
            Record::Open { .. } => kind::OPEN,
            Record::Data { .. } => kind::DATA,
            Record::End { .. } => kind::END,
            Record::Gap { .. } => kind::GAP,
            Record::Mask { .. } => kind::MASK,
            Record::Event { .. } => kind::EVENT,
            Record::Link { .. } => kind::LINK,
            Record::Unknown { kind, .. } => *kind,
        }
    }

    /// The running time after this record: PART and LINK `mono_ns`, GAP `end_mono`, the `t` of
    /// OPEN, DATA, END and EVENT. MASK and unknown kinds leave it alone.
    fn time(&self) -> Option<u64> {
        match self {
            Record::Part { mono_ns, .. } | Record::Link { mono_ns, .. } => Some(*mono_ns),
            Record::Gap { end_mono, .. } => Some(*end_mono),
            Record::Open { t, .. }
            | Record::Data { t, .. }
            | Record::End { t, .. }
            | Record::Event { t, .. } => Some(*t),
            Record::Mask { .. } | Record::Unknown { .. } => None,
        }
    }
}

/// The running time of one record stream, for `dt`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Codec {
    pub t: u64,
}

impl Codec {
    /// Appends the encoded record to `out`. A refused record writes nothing and leaves the
    /// running time as it was, so the next record's `dt` is from the last record written.
    pub fn encode(&mut self, r: &Record, out: &mut Vec<u8>) -> io::Result<()> {
        let mut b = Vec::new();
        let v = |b: &mut Vec<u8>, xs: &[u64]| xs.iter().for_each(|x| put_varint(b, *x));
        let dt = |b: &mut Vec<u8>, t: u64| -> io::Result<()> {
            let dt = i64::try_from(i128::from(t) - i128::from(self.t))
                .map_err(|_| invalid("time step out of range"))?;
            put_varint(b, zigzag(dt));
            Ok(())
        };
        match r {
            Record::Part {
                entry,
                boot,
                part,
                raw_offset,
                wall_ns,
                mono_ns,
                boot_id,
                next_link,
                prev,
            } => {
                if entry.len() != 17 {
                    return Err(invalid("entry id is not 17 bytes"));
                }
                b.extend_from_slice(MAGIC);
                b.push(VERSION);
                b.extend_from_slice(entry.as_bytes());
                v(&mut b, &[*boot, *part, *raw_offset, *wall_ns, *mono_ns]);
                b.extend_from_slice(boot_id);
                v(&mut b, &[*next_link]);
                b.extend_from_slice(prev);
            }
            Record::Open {
                sid,
                t,
                skind,
                conn,
                req,
                meta,
            } => {
                v(&mut b, &[*sid]);
                dt(&mut b, *t)?;
                b.push(skind.to_u8());
                v(&mut b, &[*conn, *req]);
                b.extend_from_slice(meta);
            }
            Record::Data {
                sid,
                t,
                off,
                flags,
                bytes,
            } => {
                v(&mut b, &[*sid]);
                dt(&mut b, *t)?;
                v(&mut b, &[*off]);
                b.push(*flags);
                b.extend_from_slice(bytes);
            }
            Record::End {
                sid,
                t,
                reason,
                seen,
                tail,
            } => {
                v(&mut b, &[*sid]);
                dt(&mut b, *t)?;
                b.push(reason.to_u8());
                v(&mut b, &[*seen]);
                b.extend_from_slice(tail);
            }
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
            } => {
                v(&mut b, &[*sid]);
                b.push(reason.to_u8());
                v(
                    &mut b,
                    &[*start_wall, *end_wall, *start_mono, *end_mono, *off, *lost],
                );
                b.extend_from_slice(tail);
            }
            Record::Mask {
                sid,
                off,
                len,
                rule,
            } => v(&mut b, &[*sid, *off, *len, *rule]),
            Record::Event { t, json } => {
                dt(&mut b, *t)?;
                b.extend_from_slice(json);
            }
            Record::Link {
                seq,
                wall_ns,
                mono_ns,
                hash,
            } => {
                v(&mut b, &[*seq, *wall_ns, *mono_ns]);
                b.extend_from_slice(hash);
            }
            Record::Unknown { body, .. } => b.extend_from_slice(body),
        }
        if r.kind() == 0 {
            return Err(invalid("record kind 0 is never valid"));
        }
        if b.len() as u64 > MAX_BODY {
            let n = b.len();
            return Err(invalid(format!("record body of {n} bytes is over 16 MiB")));
        }
        out.push(r.kind());
        put_varint(out, b.len() as u64);
        out.extend_from_slice(&b);
        self.t = r.time().unwrap_or(self.t);
        Ok(())
    }

    /// Decodes one record body of kind `k`. Fields are read in the order written.
    pub fn decode(&mut self, k: u8, body: &[u8]) -> io::Result<Record> {
        let mut c = Cur(body);
        let base = self.t;
        let dt = |c: &mut Cur<'_>| -> io::Result<u64> {
            let t = i128::from(base) + i128::from(unzigzag(c.var()?));
            u64::try_from(t).map_err(|_| invalid("record time out of range"))
        };
        let r = match k {
            kind::PART => {
                if c.take(4)? != MAGIC {
                    return Err(invalid("PART without GCR1 magic"));
                }
                if c.u8()? != VERSION {
                    return Err(invalid("PART of an unknown format version"));
                }
                Record::Part {
                    entry: String::from_utf8(c.take(17)?.to_vec())
                        .map_err(|_| invalid("entry id is not ASCII"))?,
                    boot: c.var()?,
                    part: c.var()?,
                    raw_offset: c.var()?,
                    wall_ns: c.var()?,
                    mono_ns: c.var()?,
                    boot_id: c.take(16)?.try_into().expect("16 bytes"),
                    next_link: c.var()?,
                    prev: c.take(32)?.try_into().expect("32 bytes"),
                }
            }
            kind::OPEN => Record::Open {
                sid: c.var()?,
                t: dt(&mut c)?,
                skind: StreamKind::from_u8(c.u8()?),
                conn: c.var()?,
                req: c.var()?,
                meta: c.rest(),
            },
            kind::DATA => Record::Data {
                sid: c.var()?,
                t: dt(&mut c)?,
                off: c.var()?,
                flags: c.u8()?,
                bytes: c.rest(),
            },
            kind::END => Record::End {
                sid: c.var()?,
                t: dt(&mut c)?,
                reason: EndReason::from_u8(c.u8()?),
                seen: c.var()?,
                tail: c.rest(),
            },
            kind::GAP => Record::Gap {
                sid: c.var()?,
                reason: GapReason::from_u8(c.u8()?),
                start_wall: c.var()?,
                end_wall: c.var()?,
                start_mono: c.var()?,
                end_mono: c.var()?,
                off: c.var()?,
                lost: c.var()?,
                tail: c.rest(),
            },
            kind::MASK => Record::Mask {
                sid: c.var()?,
                off: c.var()?,
                len: c.var()?,
                rule: c.var()?,
            },
            kind::EVENT => Record::Event {
                t: dt(&mut c)?,
                json: c.rest(),
            },
            kind::LINK => Record::Link {
                seq: c.var()?,
                wall_ns: c.var()?,
                mono_ns: c.var()?,
                hash: c.take(32)?.try_into().expect("32 bytes"),
            },
            0 => return Err(invalid("record kind 0 is never valid")),
            k => Record::Unknown {
                kind: k,
                body: body.to_vec(),
            },
        };
        // PART, MASK and LINK have a fixed length; the others end in a free tail.
        if matches!(k, kind::PART | kind::MASK | kind::LINK) && !c.0.is_empty() {
            return Err(invalid("record body too long for its kind"));
        }
        self.t = r.time().unwrap_or(self.t);
        Ok(r)
    }
}

/// Unsigned LEB128.
pub fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Reads one varint from the front of `b`: the value and the bytes it took. `None` when `b` ends
/// first; an error past 10 bytes or 64 bits.
pub fn get_varint(b: &[u8]) -> io::Result<Option<(u64, usize)>> {
    let mut v: u64 = 0;
    for (i, &x) in b.iter().enumerate() {
        if i == 9 && x > 1 {
            return Err(invalid("varint over 64 bits"));
        }
        v |= u64::from(x & 0x7f) << (7 * i);
        if x & 0x80 == 0 {
            return Ok(Some((v, i + 1)));
        }
    }
    Ok(None)
}

pub fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

pub fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

struct Cur<'a>(&'a [u8]);

impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < n {
            return Err(invalid("record body too short"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn var(&mut self) -> io::Result<u64> {
        let (v, n) = get_varint(self.0)?.ok_or_else(|| invalid("record body too short"))?;
        self.0 = &self.0[n..];
        Ok(v)
    }
    fn rest(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.0).to_vec()
    }
}
