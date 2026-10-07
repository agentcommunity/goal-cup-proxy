//! The daemon writes one chain, and it writes it through the protocol crate's
//! own exporter. Every assertion here is about a property `ChainWriter` inherits from
//! `TraceExport` rather than one it implements: the seq is contiguous because the export
//! stamps it, the seal is last because `write_seal` is the only thing that builds one,
//! and a restart finds its position by reading `chain.jsonl` and nothing else.
#![cfg(feature = "v1-compat")]

use goalcup_daemon::chain_writer::ChainWriter;
use goalcup_daemon::redact::Redactor;
use goalcup_protocol::chain::{GENESIS, Link};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::*;

fn resume_writer(entry_id: &str, trace_dir: &std::path::Path) -> ChainWriter {
    let root = TraceRoot::open(trace_dir).expect("the trace root opens");
    let prepared =
        TraceExport::prepare_resume_from_root(root).expect("the complete chain prepares");
    ChainWriter::resume_prepared(entry_id, prepared, Redactor::new())
        .expect("the prepared writer resumes")
}

#[test]
fn one_chain_carries_the_record_and_the_daemon_events_and_every_line_recomputes() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("trace");
    let mut w = ChainWriter::create("e1", &dir, Redactor::new()).unwrap();
    assert_eq!(w.head(), None, "no links yet — not a head at seq 0");
    assert_eq!(w.seq_written(), None);

    let rec = goalcup_daemon::tailer::CapturedRecord {
        store: "main".to_string(),
        source_path: "/h/main.jsonl".to_string(),
        payload: serde_json::Value::String(r#"{"type":"user","sessionId":"s1"}"#.to_string()),
        value: serde_json::json!({"type":"user","sessionId":"s1"}),
        bytes: br#"{"type":"user","sessionId":"s1"}"#.to_vec(),
    };
    assert_eq!(
        w.leg1(&rec).unwrap().len(),
        1,
        "nothing redacted, so one link"
    );
    w.event(EventBody::Heartbeat {
        ts: "2026-09-04T00:00:02Z".to_string(),
        stores_seen: Vec::new(),
        receiver_ok: true,
    })
    .unwrap();

    // One file. No leg1/, no leg2/, no daemon.jsonl.
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(names.contains(&CHAIN_FILE.to_string()));
    assert!(!names.iter().any(|n| n == "leg1" || n == "leg2"));

    let text = std::fs::read_to_string(dir.join(CHAIN_FILE)).unwrap();
    let mut prev = GENESIS.to_string();
    let mut sources = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let e: Envelope = serde_json::from_str(line).unwrap();
        assert_eq!(e.entry_id, "e1");
        assert_eq!(e.protocol_version, goalcup_protocol::PROTOCOL_VERSION);
        assert_eq!(
            e.link.seq, i as u64,
            "contiguous from 0, written by the export"
        );
        assert_eq!(e.link.prev, prev);
        assert_eq!(
            e.link.hash,
            e.expected_hash(&prev).unwrap(),
            "the verifier recomputes this"
        );
        assert!(e.recorded_at.ends_with('Z'), "RFC-3339, not a float");
        prev = e.link.hash;
        sources.push(e.source);
    }
    assert_eq!(sources, vec!["main", "daemon"]);
    assert_eq!(w.head(), Some((1, prev.clone())));

    // A restart continues this chain from what is on disk, with no position handed in.
    drop(w);
    let w2 = resume_writer("e1", &dir);
    assert_eq!(
        w2.head(),
        Some((1, prev)),
        "the writer reads its own position"
    );
    drop(w2);
    // And a second `create` over the same directory is refused rather than writing a
    // second run's link 0 onto the end of the first run's links.
    assert!(ChainWriter::create("e1", &dir, Redactor::new()).is_err());
}

#[test]
fn a_redacted_record_carries_one_inline_audit_without_an_oracle() {
    let t = tempfile::tempdir().unwrap();
    let mut w = ChainWriter::create("e1", &t.path().join("trace"), Redactor::new()).unwrap();
    let body =
        serde_json::json!({"headers":{"authorization":"Bearer sk-ant-api03-AAAABBBBCCCCDDDD"}});
    let line = serde_json::to_string(&body).unwrap();
    let leg1 = goalcup_daemon::tailer::CapturedRecord {
        store: "main".to_string(),
        source_path: "/h/main.jsonl".to_string(),
        payload: serde_json::Value::String(line.clone()),
        value: body.clone(),
        bytes: line.into_bytes(),
    };
    let captured = w.leg1(&leg1).unwrap();
    assert_eq!(captured.len(), 1, "one altered subject is one append");
    match &captured[0].body {
        EventBody::Leg1Record { payload, .. } => {
            assert!(
                !payload.to_string().contains("sk-ant-api03"),
                "leg 1 is not shipped raw"
            );
        }
        other => panic!("{other:?}"),
    }
    let audit = captured[0]
        .redaction
        .as_ref()
        .expect("the altered body has evidence");
    assert_eq!(audit.spans.len(), 1);
    assert_eq!(audit.spans[0].kind, "anthropic_key");

    let again = w.leg1(&leg1).unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].redaction, captured[0].redaction);

    let wire = std::fs::read_to_string(t.path().join("trace").join(CHAIN_FILE)).unwrap();
    assert_eq!(wire.lines().count(), 2);
    assert!(!wire.contains("salt"));
    assert!(!wire.contains("digest"));
    assert!(!wire.contains("AAAABBBB"));
}

#[test]
fn the_six_documents_land_where_the_verifiers_descriptor_looks_for_them() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("trace");
    let connector_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors/claude-code-cli");
    let mut w = ChainWriter::create("e1", &dir, Redactor::new()).unwrap();
    let manifest_rel = w.copy_manifest(&connector_dir).unwrap();
    assert_eq!(
        std::fs::read(dir.join(&manifest_rel)).unwrap(),
        std::fs::read(connector_dir.join("manifest.toml")).unwrap(),
        "byte-for-byte: the digest pin compares these bytes (R-D)"
    );

    let seal = goalcup_protocol::seal::SealManifest {
        entry_id: "e1".to_string(),
        sealed_at: "2026-09-08T00:00:00Z".to_string(),
        workspace: "/ws/e1".to_string(),
        files: Vec::new(),
        excluded: vec![".claude/settings.json".to_string()],
    };
    // The baseline is the first link and the seal is the last. Nothing between them here,
    // but the order is the contract: `write_baseline` on a chain that already has links is
    // `ExportError::BaselineNotFirst`, because a manifest taken after the run started
    // describes a workspace the agent has been writing to.
    let (_, base) = w.write_baseline(&seal).unwrap();
    assert_eq!(
        base.seq, 0,
        "the receipt's chain_head witnesses the baseline"
    );
    let (_, sealed) = w.write_seal(&seal).unwrap();
    assert_eq!(w.head(), Some((sealed.seq, sealed.hash.clone())));
    assert!(
        w.event(EventBody::Heartbeat {
            ts: "2026-09-08T00:00:01Z".to_string(),
            stores_seen: Vec::new(),
            receiver_ok: true,
        })
        .is_err(),
        "the seal closes the chain: no record may follow it (threat N9)"
    );
    w.write_descriptor(&TraceDescriptorDoc {
        connector: "claude-code-cli".to_string(),
        entry_id: "e1".to_string(),
        workspace: "/ws/e1".to_string(),
        tracked_sessions: vec!["s1".to_string()],
        harness_version: "2.1.259".to_string(),
        connector_manifest: manifest_rel,
        chain: Some(CHAIN_FILE.to_string()),
        receipt: Some(RECEIPT_FILE.to_string()),
        baseline: Some(BASELINE_FILE.to_string()),
        acks: Some(ACKS_FILE.to_string()),
        seal: Some(SEAL_FILE.to_string()),
        server_public_key: Some("ab".repeat(32)),
        // Non-`Option`, written unconditionally. A daemon-produced trace
        // never asks for the relaxed reading: `false` is the claim, not a default.
        expected_reread: false,
    })
    .unwrap();

    for f in [
        DESCRIPTOR_FILE,
        BASELINE_FILE,
        SEAL_FILE,
        CHAIN_FILE,
        ACKS_FILE,
    ] {
        assert!(dir.join(f).exists(), "{f} was not written");
    }
    let raw = std::fs::read_to_string(dir.join(DESCRIPTOR_FILE)).unwrap();
    assert!(
        !raw.contains(&t.path().display().to_string()),
        "no absolute path: the directory must survive being copied"
    );
}

#[test]
fn a_hand_built_seal_or_baseline_is_not_an_event() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("trace");
    let mut w = ChainWriter::create("e1", &dir, Redactor::new()).unwrap();

    // Refused here before they reach the export, in this crate's vocabulary. The
    // export reserves both bodies as well — asserted at the end of this test — so the
    // rule holds for a caller who reaches around this writer too.
    let seal = w.event(EventBody::Seal {
        manifest_sha256: "00".repeat(32),
        sealed_at: "2026-09-08T00:00:00Z".to_string(),
        file_count: 0,
    });
    assert!(
        matches!(seal, Err(goalcup_daemon::DaemonError::Refused(_))),
        "{seal:?}"
    );
    let baseline = w.event(EventBody::Baseline {
        manifest_sha256: "00".repeat(32),
        file_count: 0,
    });
    assert!(
        matches!(baseline, Err(goalcup_daemon::DaemonError::Refused(_))),
        "{baseline:?}"
    );

    // Neither refusal wrote a line, so the baseline that follows is still the first link.
    assert_eq!(w.head(), None);
    assert_eq!(
        std::fs::read_to_string(dir.join(CHAIN_FILE)).unwrap(),
        "",
        "a refused event leaves no link behind"
    );

    // The guard does not depend on going through this writer: the export refuses
    // both bodies itself (`ExportError::ReservedBody`), so reaching around `ChainWriter`
    // to a `TraceExport` does not buy a hand-rolled seal. The daemon's own check stays
    // because it fails earlier and says so in this crate's vocabulary.
    let mut bypass = TraceExport::create(&t.path().join("bypass")).unwrap();
    let hand_rolled = |body| Envelope {
        protocol_version: goalcup_protocol::PROTOCOL_VERSION,
        entry_id: "e1".to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-08T00:00:00Z".to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body,
        redaction: None,
    };
    let mut seal = hand_rolled(EventBody::Seal {
        manifest_sha256: "00".repeat(32),
        sealed_at: "2026-09-08T00:00:00Z".to_string(),
        file_count: 0,
    });
    assert!(
        matches!(
            bypass.append_chain(&mut seal),
            Err(ExportError::ReservedBody { kind: "seal" })
        ),
        "the export reserves the body write_seal builds"
    );
    let mut baseline = hand_rolled(EventBody::Baseline {
        manifest_sha256: "00".repeat(32),
        file_count: 0,
    });
    assert!(
        matches!(
            bypass.append_chain(&mut baseline),
            Err(ExportError::ReservedBody { kind: "baseline" })
        ),
        "the export reserves the body write_baseline builds"
    );
}

#[test]
fn a_baseline_after_a_record_is_refused_by_the_export_and_surfaced_as_a_daemon_error() {
    let t = tempfile::tempdir().unwrap();
    let mut w = ChainWriter::create("e1", &t.path().join("trace"), Redactor::new()).unwrap();
    w.event(EventBody::Heartbeat {
        ts: "2026-09-08T00:00:00Z".to_string(),
        stores_seen: Vec::new(),
        receiver_ok: false,
    })
    .unwrap();
    let m = goalcup_protocol::seal::SealManifest {
        entry_id: "e1".to_string(),
        sealed_at: "2026-09-08T00:00:00Z".to_string(),
        workspace: "/ws/e1".to_string(),
        files: Vec::new(),
        excluded: Vec::new(),
    };
    // Not re-implemented here: the export decides, this crate reports.
    assert!(matches!(
        w.write_baseline(&m),
        Err(goalcup_daemon::DaemonError::Export(
            ExportError::BaselineNotFirst { head_seq: 0 }
        ))
    ));
    assert!(!t.path().join("trace").join(BASELINE_FILE).exists());
}

#[test]
fn a_restart_appends_at_the_next_seq_over_the_head_it_read() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("trace");
    let mut w = ChainWriter::create("e1", &dir, Redactor::new()).unwrap();
    w.event(EventBody::Heartbeat {
        ts: "2026-09-04T00:00:01Z".to_string(),
        stores_seen: Vec::new(),
        receiver_ok: true,
    })
    .unwrap();
    let (seq, hash) = w.head().unwrap();
    drop(w);

    let mut w = resume_writer("e1", &dir);
    let restart = w
        .event(EventBody::DaemonRestart {
            at: "2026-09-04T00:01:00Z".to_string(),
            previous_head_seq: seq,
            previous_head_hash: hash.clone(),
        })
        .unwrap();
    assert_eq!(restart.link.seq, seq + 1, "the next seq, read off the file");
    assert_eq!(restart.link.prev, hash, "over the head it read");

    // The whole file still recomputes across the restart boundary, and a second resume
    // reports the same head this one left.
    let text = std::fs::read_to_string(dir.join(CHAIN_FILE)).unwrap();
    let mut prev = GENESIS.to_string();
    for line in text.lines() {
        let e: Envelope = serde_json::from_str(line).unwrap();
        assert_eq!(e.link.prev, prev);
        assert_eq!(e.link.hash, e.expected_hash(&prev).unwrap());
        prev = e.link.hash;
    }
    assert_eq!(w.head(), Some((1, prev.clone())));
    drop(w);
    assert_eq!(resume_writer("e1", &dir).head(), Some((1, prev)));
}

#[test]
fn a_row_shaped_record_is_still_a_row_after_a_span_leaves_it() {
    let t = tempfile::tempdir().unwrap();
    let payload = serde_json::json!({
        "db": "/h/state.db",
        "table": "messages",
        "row": {"id": 4, "text": "export OPENAI_API_KEY=sk-proj-AAAABBBBCCCCDDDDEEEEFFFF"}
    });
    let rec = goalcup_daemon::tailer::CapturedRecord {
        store: "state".to_string(),
        source_path: "/h/state.db".to_string(),
        payload: payload.clone(),
        value: payload.clone(),
        bytes: serde_json::to_vec(&payload).unwrap(),
    };
    let mut w = ChainWriter::create("e1", &t.path().join("trace"), Redactor::new()).unwrap();
    let out = w.leg1(&rec).unwrap();
    assert_eq!(out.len(), 1, "the altered row and its audit are one link");
    match &out[0].body {
        EventBody::Leg1Record { store, payload } => {
            assert_eq!(store, "state");
            assert!(
                payload.is_object(),
                "a SQLite row stays an object: {payload}"
            );
            assert_eq!(payload["table"], serde_json::json!("messages"));
            assert!(!payload.to_string().contains("sk-proj-"));
        }
        other => panic!("{other:?}"),
    }
    assert!(
        out[0]
            .redaction
            .as_ref()
            .is_some_and(|audit| !audit.spans.is_empty()),
        "the altered row carries its audit inline"
    );
    assert_eq!(
        out[0].source, "state",
        "a leg-1 record's source is its store"
    );
}
