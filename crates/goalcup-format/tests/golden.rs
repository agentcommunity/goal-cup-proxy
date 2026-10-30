//! The worked example of PRD-01 section 16: the records encode to the committed 998 bytes, decode
//! back, the link hash and file hash are the published ones, the part writer produces the same
//! raw stream at window 2^27, and `dump` gives the committed lines (compared as JSON values).

use goalcup_format::codec::{Codec, Record};
use goalcup_format::dump::Dumper;
use goalcup_format::link::{Chain, ZERO};
use goalcup_format::read::{Reader, open_dir};
use goalcup_format::{EndReason, Now, PartOptions, PartWriter, StreamKind, hex};
use sha2::{Digest, Sha256};

const GCR: &[u8] = include_bytes!("data/worked-example.gcr");
const DUMP: &str = include_str!("data/worked-example.dump.jsonl");
const ENTRY: &str = "e0123456789abcdef";
const WALL: u64 = 1_794_009_600_000_000_000;
const MONO: u64 = 5_000_000_000;
const LINK_HASH: &str = "87e927badc48e694b3f55d0e8f129502bb8b27c605c25e05bd8bc8616dde8dbd";
const FILE_HASH: &str = "013c82339391fbb8af8b6c8d296fa7ad841fdbcf62266a10bd35bc16d3ee4898";

const START: &str = r#"{"build":"0123456789abcdef0123456789abcdef01234567","connector":"codex-cli","connector_source":"builtin","entry":"e0123456789abcdef","ev":"start","goalcup":"1.0.0","mask_rules":["openai_key","bearer"],"resume":false,"run":"r0123456789abcdef"}"#;
const CONN: &str = r#"{"alpn":"http/1.1","fingerprint":"0123456789abcdef","host":"api.openai.com","listed":"shared","mode":"decrypt","port":443,"reason":null,"tls":"1.3"}"#;
const REQ: &str = r#"{"coding":null,"headers":[["authorization","********************"],["content-type","application/json"]],"masked":["authorization"],"method":"POST","path":"/v1/responses","ver":"1.1"}"#;
const RESP: &str = r#"{"coding":["br"],"decoded":true,"headers":[["content-type","text/event-stream"],["content-encoding","br"]],"masked":[],"status":200,"ver":"1.1"}"#;

const MS: u64 = 1_000_000;

/// The records after PART and before the LINK.
fn body() -> Vec<Record> {
    let b = |s: &str| s.as_bytes().to_vec();
    vec![
        Record::Event {
            t: MONO,
            json: b(START),
        },
        Record::Open {
            sid: 1,
            t: MONO + MS,
            skind: StreamKind::Conn,
            conn: 0,
            req: 0,
            meta: b(CONN),
        },
        Record::Open {
            sid: 2,
            t: MONO + 3 * MS,
            skind: StreamKind::HttpReq,
            conn: 1,
            req: 0,
            meta: b(REQ),
        },
        Record::Data {
            sid: 2,
            t: MONO + 4 * MS,
            off: 0,
            flags: 0,
            bytes: b(r#"{"model":"m1","input":"hi"}"#),
        },
        Record::End {
            sid: 2,
            t: MONO + 4 * MS,
            reason: EndReason::Complete,
            seen: 27,
            tail: vec![],
        },
        Record::Open {
            sid: 3,
            t: MONO + 304 * MS,
            skind: StreamKind::HttpResp,
            conn: 1,
            req: 2,
            meta: b(RESP),
        },
        Record::Data {
            sid: 3,
            t: MONO + 309 * MS,
            off: 0,
            flags: 0,
            bytes: b("data: {\"type\":\"response.completed\"}\n\n"),
        },
        Record::End {
            sid: 3,
            t: MONO + 310 * MS,
            reason: EndReason::Complete,
            seen: 37,
            tail: b(r#"{"wire":41}"#),
        },
    ]
}

fn link_now() -> Now {
    Now {
        wall_ns: WALL + 310 * MS,
        mono_ns: MONO + 310 * MS,
    }
}

fn records() -> Vec<Record> {
    let part = Record::Part {
        entry: ENTRY.into(),
        boot: 0,
        part: 0,
        raw_offset: 0,
        wall_ns: WALL,
        mono_ns: MONO,
        boot_id: [0x11; 16],
        next_link: 0,
        prev: ZERO,
    };
    let mut v = vec![part];
    v.extend(body());
    v.push(Record::Link {
        seq: 0,
        wall_ns: link_now().wall_ns,
        mono_ns: link_now().mono_ns,
        hash: GCR[966..998].try_into().unwrap(),
    });
    v
}

#[test]
fn records_encode_to_the_committed_bytes_and_back() {
    assert_eq!(GCR.len(), 998);
    assert_eq!(hex(&Sha256::digest(GCR)), FILE_HASH);
    assert_eq!(hex(&Chain::new(ZERO).link(&GCR[..966])), LINK_HASH);

    let mut codec = Codec::default();
    let mut out = Vec::new();
    for r in records() {
        codec.encode(&r, &mut out).unwrap();
    }
    assert_eq!(out, GCR);

    let items: Vec<_> = Reader::new(GCR).collect::<Result<_, _>>().unwrap();
    let back: Vec<Record> = items.iter().map(|i| i.rec.clone()).collect();
    assert_eq!(back, records());
    let offs: Vec<u64> = items.iter().map(|i| i.off).collect();
    assert_eq!(offs, [0, 90, 336, 494, 687, 722, 728, 884, 930, 949]);
    assert_eq!(items.last().unwrap().link_ok, Some(true));
}

#[test]
fn part_writer_writes_the_worked_example_at_window_27() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden-writer");
    let _ = std::fs::remove_dir_all(&dir);
    let opts = PartOptions::default();
    assert_eq!(opts.window_log, 27);
    let mut w = PartWriter::open(
        &dir,
        opts,
        ENTRY,
        [0x11; 16],
        Now {
            wall_ns: WALL,
            mono_ns: MONO,
        },
    )
    .unwrap();
    for r in body() {
        w.write(&r).unwrap();
    }
    let line = w.link(link_now()).unwrap();
    assert_eq!(
        (line.seq, line.raw_end, hex(&line.hash)),
        (0, 998, LINK_HASH.to_string())
    );

    // The part file at the flush point is an open frame that decodes to the worked example.
    let mut rd = open_dir(&dir).unwrap();
    let items: Vec<_> = rd.by_ref().collect::<Result<_, _>>().unwrap();
    assert!(rd.tail().open_frame);
    assert_eq!(rd.tail().truncated, None);
    let back: Vec<Record> = items.into_iter().map(|i| i.rec).collect();
    assert_eq!(back, records());

    let d = w
        .seal(Now {
            wall_ns: WALL + 311 * MS,
            mono_ns: MONO + 311 * MS,
        })
        .unwrap();
    assert_eq!(d.line.seq, 1);
    let mut z = Vec::new();
    for p in goalcup_format::read::part_files(&dir).unwrap() {
        z.extend(std::fs::read(p).unwrap());
    }
    let raw = zstd::stream::decode_all(&z[..]).unwrap();
    assert_eq!(&raw[..998], GCR);
}

#[test]
fn dump_matches_the_committed_lines() {
    let mut d = Dumper::default();
    let got: Vec<serde_json::Value> = Reader::new(GCR).map(|i| d.line(&i.unwrap())).collect();
    let want: Vec<serde_json::Value> = DUMP
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(got, want);
}
