//! Framing and records: every kind round trips, unknown kinds are skipped by their length, a body
//! over 16 MiB is refused, a record cut at the end is reported and not read, and MASK carries no
//! time.

use goalcup_format::codec::{Codec, Record, put_varint};
use goalcup_format::read::Reader;
use goalcup_format::{EndReason, GapReason, StreamKind};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// Small and large values alike, so every varint length is used.
    fn num(&mut self) -> u64 {
        let bits = self.below(63);
        self.next() >> (63 - bits)
    }
    fn bytes(&mut self, max: u64) -> Vec<u8> {
        (0..self.below(max)).map(|_| self.next() as u8).collect()
    }
    fn arr<const N: usize>(&mut self) -> [u8; N] {
        std::array::from_fn(|_| self.next() as u8)
    }
}

fn random(r: &mut Rng) -> Record {
    let t = r.below(1 << 50);
    match r.below(9) {
        0 => Record::Part {
            entry: format!("e{:016x}", r.next()),
            boot: r.num(),
            part: r.num(),
            raw_offset: r.num(),
            wall_ns: r.num(),
            mono_ns: t,
            boot_id: r.arr(),
            next_link: r.num(),
            prev: r.arr(),
        },
        1 => Record::Open {
            sid: r.num(),
            t,
            skind: StreamKind::from_u8(r.below(9) as u8),
            conn: r.num(),
            req: r.num(),
            meta: r.bytes(300),
        },
        2 => Record::Data {
            sid: r.num(),
            t,
            off: r.num(),
            flags: r.next() as u8,
            bytes: r.bytes(3000),
        },
        3 => Record::End {
            sid: r.num(),
            t,
            reason: EndReason::from_u8(r.below(7) as u8),
            seen: r.num(),
            tail: r.bytes(50),
        },
        4 => Record::Gap {
            sid: r.num(),
            reason: GapReason::from_u8(r.below(17) as u8),
            start_wall: r.num(),
            end_wall: r.num(),
            start_mono: r.num(),
            end_mono: t,
            off: r.num(),
            lost: r.num(),
            tail: r.bytes(50),
        },
        5 => Record::Mask {
            sid: r.num(),
            off: r.num(),
            len: r.num(),
            rule: r.num(),
        },
        6 => Record::Event {
            t,
            json: r.bytes(300),
        },
        7 => Record::Link {
            seq: r.num(),
            wall_ns: r.num(),
            mono_ns: t,
            hash: r.arr(),
        },
        _ => Record::Unknown {
            kind: 9 + r.below(247) as u8,
            body: r.bytes(100),
        },
    }
}

fn encode(recs: &[Record]) -> Vec<u8> {
    let mut c = Codec::default();
    let mut out = Vec::new();
    for r in recs {
        c.encode(r, &mut out).unwrap();
    }
    out
}

fn decode(raw: &[u8]) -> Vec<Record> {
    Reader::new(raw).map(|i| i.unwrap().rec).collect()
}

#[test]
fn random_records_of_every_kind_round_trip() {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let recs: Vec<Record> = (0..3000).map(|_| random(&mut r)).collect();
    let raw = encode(&recs);
    let mut rd = Reader::new(&raw[..]);
    let back: Vec<Record> = rd.by_ref().map(|i| i.unwrap().rec).collect();
    assert_eq!(back, recs);
    assert_eq!(rd.tail().truncated, None);
    assert_eq!(rd.tail().raw, raw.len() as u64);
}

#[test]
fn a_body_over_16_mib_is_refused() {
    let mut raw = vec![0x03];
    put_varint(&mut raw, (16 << 20) + 1);
    let r: Vec<_> = Reader::new(&raw[..]).collect();
    assert_eq!(r.len(), 1);
    assert!(r[0].is_err());
    let mut c = Codec::default();
    let big = Record::Data {
        sid: 1,
        t: 5000,
        off: 0,
        flags: 0,
        bytes: vec![0; 16 << 20],
    };
    // A refused record writes nothing and leaves the running time as it was.
    let ev = |t| Record::Event {
        t,
        json: b"{}".to_vec(),
    };
    let mut raw = Vec::new();
    c.encode(&ev(100), &mut raw).unwrap();
    assert!(c.encode(&big, &mut raw).is_err());
    c.encode(&ev(200), &mut raw).unwrap();
    assert_eq!(decode(&raw), [ev(100), ev(200)]);
}

#[test]
fn a_record_cut_at_the_end_is_reported_not_read() {
    let recs = [
        Record::Event {
            t: 10,
            json: b"{\"ev\":\"a\"}".to_vec(),
        },
        Record::Data {
            sid: 1,
            t: 20,
            off: 0,
            flags: 0,
            bytes: vec![b'x'; 300],
        },
    ];
    let raw = encode(&recs);
    let first = encode(&recs[..1]).len();
    for cut in [first + 1, first + 2, first + 4, raw.len() - 1] {
        let mut rd = Reader::new(&raw[..cut]);
        let got: Vec<Record> = rd.by_ref().map(|i| i.unwrap().rec).collect();
        assert_eq!(got, recs[..1], "cut at {cut}");
        assert_eq!(rd.tail().truncated, Some(first as u64), "cut at {cut}");
        assert_eq!(rd.tail().raw, cut as u64, "cut at {cut}");
    }
}

#[test]
fn mask_carries_no_time() {
    let with = [
        Record::Event {
            t: 100,
            json: b"{}".to_vec(),
        },
        Record::Mask {
            sid: 1,
            off: 0,
            len: 4,
            rule: 0,
        },
        Record::Event {
            t: 150,
            json: b"{}".to_vec(),
        },
    ];
    assert_eq!(decode(&encode(&with)), with);
    // The second event's `dt` is 50, from the record before the MASK.
    let without = encode(&[with[0].clone(), with[2].clone()]);
    let masked = encode(&with);
    assert_eq!(masked[masked.len() - 4..], without[without.len() - 4..]);
}
