//! The encrypted write-ahead buffer, and the uploader that drains
//! it.
//!
//! The buffer cases below are about six properties and nothing else: the stream is
//! numbered from zero, the acknowledged record survives its own acknowledgement,
//! the acknowledgement watermark survives a restart, a read is contiguous and stops
//! at the first hole rather than stepping over it, the bytes at rest are ciphertext, and
//! a read is bounded by the chunk maximum rather than by how much the run has produced.
//!
//! The middle two exist because of the devserver's chunk contract: a chunk must start at
//! `last_acked + 1` and carry an unbroken range, and a chunk that starts anywhere else is
//! refused *forever* rather than retried into acceptance. A buffer that hands the uploader a
//! range with a hole in it, or that forgets across a restart what the server already
//! acknowledged, does not produce a slow upload — it produces a dead one.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_daemon::DaemonError;
use goalcup_daemon::buffer::Buffer;
use goalcup_daemon::upload::UploadClient;

type R = Result<(), Box<dyn std::error::Error>>;

#[test]
fn the_buffer_is_encrypted_streams_by_seq_from_zero_and_keeps_the_acked_record() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("buffer.redb");
    let b = Buffer::open(&path, &"11".repeat(32)).unwrap();
    for seq in 0..10u64 {
        b.append(&support::envelope(seq, "main")).unwrap();
    }

    // `after_seq: None` means "wherever this buffer is up to", which on a buffer that has
    // acknowledged nothing is the beginning. A `0` would skip the genesis envelope, because
    // TraceExport numbers from zero.
    assert_eq!(b.read_from(None, 4).unwrap().envelopes.len(), 4);
    assert_eq!(b.read_from(None, 4).unwrap().envelopes[0].link.seq, 0);
    assert_eq!(b.read_from(Some(3), 100).unwrap().envelopes[0].link.seq, 4);
    assert_eq!(b.pending(None).unwrap(), 10);
    assert_eq!(b.floor().unwrap(), None, "nothing acknowledged yet");

    assert_eq!(b.head().unwrap().unwrap().0, 9);
    b.ack(6).unwrap();
    assert_eq!(
        b.read_from(None, 100).unwrap().envelopes[0].link.seq,
        7,
        "the stream resumes AFTER the acknowledged seq: the devserver refuses a chunk that \
         starts at one it has already acknowledged, and refuses it permanently"
    );
    assert_eq!(
        b.pending(Some(5)).unwrap(),
        4,
        "R12: the acknowledged record is still on disk, for head() to resume the chain from"
    );
    assert_eq!(b.pending(Some(6)).unwrap(), 3);
    assert_eq!(b.pending(None).unwrap(), 3, "and it is not pending upload");
    assert_eq!(b.floor().unwrap(), Some(6));
    assert_eq!(
        b.head().unwrap().unwrap().0,
        9,
        "the head is still the head"
    );

    // Confidentiality, and only that: the plaintext is not on disk, and the key is.
    let raw = std::fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("\"kind\":\"leg1_record\""));
    assert!(
        Buffer::open(&path, &"22".repeat(32)).is_err(),
        "another key does not open it"
    );
}

#[cfg(unix)]
#[test]
fn opening_a_recovered_buffer_never_follows_a_symlink() {
    let t = tempfile::tempdir().unwrap();
    let victim = t.path().join("outside.redb");
    std::fs::write(&victim, b"do not open this target").unwrap();
    let path = t.path().join("buffer.redb");
    std::os::unix::fs::symlink(&victim, &path).unwrap();

    let error = Buffer::open(&path, &"11".repeat(32)).expect_err("a link is not a buffer");
    assert!(error.to_string().contains("symbolic link"));
    assert_eq!(std::fs::read(&victim).unwrap(), b"do not open this target");
}

#[cfg(unix)]
#[test]
fn opening_a_recovered_buffer_never_follows_a_symlinked_artifact_root() {
    let t = tempfile::tempdir().unwrap();
    let outside = t.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let victim = outside.join("buffer.redb");
    std::fs::write(&victim, b"do not open this target").unwrap();
    let artifact = t.path().join("artifact");
    std::os::unix::fs::symlink(&outside, &artifact).unwrap();

    let _error = Buffer::open(&artifact.join("buffer.redb"), &"11".repeat(32))
        .expect_err("a symbolic artifact root is not buffer authority");
    assert_eq!(std::fs::read(&victim).unwrap(), b"do not open this target");
}

// `free_bytes` is one `statvfs` call and returns `None` off unix, so the assertions below
// are unix-only. The behaviour off unix — `None`, never a guess — is stated in Acceptance.
#[cfg(unix)]
#[test]
fn disk_pressure_is_reachable_before_append_fails() {
    let t = tempfile::tempdir().unwrap();
    let b = Buffer::open(&t.path().join("buffer.redb"), &"11".repeat(32)).unwrap();
    let free = b.free_bytes().expect("statvfs answers on unix");
    assert!(!b.under_pressure(free / 2));
    assert!(
        b.under_pressure(free + (1 << 30)),
        "the daemon degrades visibly; it does not stop"
    );
}

/// The restart the ack policy exists for: a second `Buffer` over the same file and the
/// same run key reads back exactly what the first one wrote, in order, starting *after* the
/// last acknowledged seq — which is the only range the server will accept next.
#[test]
fn a_restart_under_the_same_run_key_reads_back_the_same_envelopes_in_order() -> R {
    let t = tempfile::tempdir()?;
    let path = t.path().join("buffer.redb");
    let key = "3c".repeat(32);
    let written: Vec<_> = (0..8u64).map(|s| support::envelope(s, "main")).collect();
    {
        let b = Buffer::open(&path, &key)?;
        for env in &written {
            b.append(env)?;
        }
        b.ack(5)?;
    }

    // The other half of "no plaintext on disk": there is ciphertext on disk. A file that
    // simply held nothing would pass the negative assertion for the wrong reason.
    let raw = String::from_utf8_lossy(&std::fs::read(&path)?).into_owned();
    assert!(raw.contains("age-encryption.org/v1"), "real age files");
    assert!(raw.contains("-> scrypt "), "under a passphrase recipient");
    assert!(!raw.contains("record 7"), "and none of the payloads");

    let b = Buffer::open(&path, &key)?;
    assert_eq!(
        b.floor()?,
        Some(5),
        "the acknowledgement watermark is on disk, not in the process that died"
    );
    let chunk = b.read_from(None, 100)?;
    assert_eq!(chunk.envelopes, written[6..], "byte for byte");
    assert_eq!(chunk.gap_after(), None, "and with no hole in it");
    assert_eq!(
        b.head()?,
        Some((7, written[7].link.hash.clone())),
        "the head survives the restart, hash and all"
    );
    Ok(())
}

/// Without the watermark a restarted daemon re-offers
/// a range the server has already acknowledged, and the server refuses that range for the
/// rest of the run rather than acknowledging it a second time.
#[test]
fn the_ack_watermark_survives_a_restart() -> R {
    let t = tempfile::tempdir()?;
    let path = t.path().join("buffer.redb");
    let key = "7f".repeat(32);
    {
        let b = Buffer::open(&path, &key)?;
        for seq in 0..=5u64 {
            b.append(&support::envelope(seq, "main"))?;
        }
        b.ack(3)?;
    }

    let b = Buffer::open(&path, &key)?;
    assert_eq!(b.floor()?, Some(3));
    assert_eq!(
        b.read_from(None, 10)?
            .envelopes
            .iter()
            .map(|e| e.link.seq)
            .collect::<Vec<_>>(),
        vec![4, 5],
        "the next chunk starts at last_acked + 1, which is the only start the server takes"
    );
    Ok(())
}

/// A hole stops the read at the hole. A read that stepped over a missing seq would produce
/// a chunk the devserver refuses permanently — `first_seq != last_acked + 1` on the very
/// next attempt, and every attempt after it. Stopping short turns an unrecoverable upload
/// into a short one plus a health fact the daemon can report.
#[test]
fn a_hole_stops_the_read_at_the_gap_rather_than_stepping_over_it() -> R {
    let t = tempfile::tempdir()?;
    let b = Buffer::open(&t.path().join("buffer.redb"), &"9d".repeat(32))?;
    for seq in [0u64, 1, 2, 4, 5] {
        b.append(&support::envelope(seq, "main"))?;
    }

    let chunk = b.read_from(None, 100)?;
    assert_eq!(
        chunk
            .envelopes
            .iter()
            .map(|e| e.link.seq)
            .collect::<Vec<_>>(),
        vec![0, 1, 2],
        "contiguous, and it stops rather than jumping the hole"
    );
    assert_eq!(chunk.gap_at, Some(3), "the seq that is missing");
    assert_eq!(chunk.gap_after(), Some(2), "the last seq before it");

    // A read that filled its bound, or that simply ran out of rows, reports no gap: it
    // stepped over nothing.
    assert_eq!(b.read_from(None, 2)?.gap_at, None, "bounded, not broken");
    assert_eq!(b.read_from(Some(3), 100)?.gap_at, None, "past the hole");
    assert_eq!(
        b.read_from(Some(3), 100)?
            .envelopes
            .iter()
            .map(|e| e.link.seq)
            .collect::<Vec<_>>(),
        vec![4, 5]
    );
    Ok(())
}

/// A wrong key fails closed and says so in this crate's vocabulary, rather than opening an
/// empty-looking buffer and letting the run start a second chain over the top of the first.
#[test]
fn a_wrong_run_key_fails_closed_with_a_named_error() -> R {
    let t = tempfile::tempdir()?;
    let path = t.path().join("buffer.redb");
    let (mine, theirs) = ("1a".repeat(32), "2b".repeat(32));
    Buffer::open(&path, &mine)?.append(&support::envelope(0, "main"))?;

    let err = Buffer::open(&path, &theirs).expect_err("another key does not open it");
    assert!(matches!(err, DaemonError::Crypto(_)), "got {err:?}");
    let text = format!("{err}");
    assert!(
        text.contains("buffer.redb"),
        "the message names the file: {text}"
    );
    assert!(
        !text.contains(&mine) && !text.contains(&theirs),
        "and never the key material: {text}"
    );
    assert!(
        text.contains("corrupted"),
        "and it hedges: a failed unseal cannot tell a wrong key from a damaged file, and \
         reading it as a key mismatch sends the operator after the wrong thing: {text}"
    );

    // A key that is not 32 bytes of hex is refused before any database is touched.
    let fresh = t.path().join("fresh.redb");
    assert!(matches!(
        Buffer::open(&fresh, "not-hex"),
        Err(DaemonError::Crypto(_))
    ));
    assert!(!fresh.exists(), "a refused key leaves no file behind");
    Ok(())
}

/// Bounded memory: a read is capped by the chunk maximum it was
/// handed, whatever the run has buffered. Nothing here ever holds a run.
#[test]
fn a_read_is_bounded_by_the_chunk_maximum_and_never_by_the_run() -> R {
    let t = tempfile::tempdir()?;
    let b = Buffer::open(&t.path().join("buffer.redb"), &"11".repeat(32))?;
    for seq in 0..300u64 {
        b.append(&support::envelope(seq, "main"))?;
    }
    // The real `upload_chunk_max`, read off the config rather than restated: a second
    // copy of 256 here is how the bound the uploader honours and the bound this test
    // asserts drift apart.
    let max = chunk_max()?;
    assert_eq!(b.read_from(None, max)?.envelopes.len(), max);
    assert_eq!(b.pending(None)?, 300, "counting is not reading");
    assert_eq!(
        b.read_from(Some(299), max)?.envelopes.len(),
        0,
        "past the head is empty"
    );
    Ok(())
}

/// `upload_chunk_max` as a machine that configured nothing gets it.
fn chunk_max() -> Result<usize, DaemonError> {
    Ok(
        goalcup_daemon::config::DaemonConfig::from_toml_str(&format!(
            "artifact_root = '/a'\nconnectors_dir = '/c'\nconfig_dir = '/d'\n\
         workspace_parent = '/w'\ncontrol_addr = '127.0.0.1:1'\n\
         upload_url = 'http://127.0.0.1:1/u'\nserver_public_key = '{}'\n",
            "ab".repeat(32)
        ))?
        .upload_chunk_max,
    )
}

/// Truncation is truncation: what was acknowledged is off the disk, and stays off it
/// across a restart.
#[test]
fn acknowledged_envelopes_are_deleted_and_stay_deleted() -> R {
    let t = tempfile::tempdir()?;
    let path = t.path().join("buffer.redb");
    let key = "5e".repeat(32);
    let b = Buffer::open(&path, &key)?;
    for seq in 0..10u64 {
        b.append(&support::envelope(seq, "main"))?;
    }
    assert_eq!(b.ack(6)?, 6, "seq 0 through 5 are gone; seq 6 is kept");
    assert_eq!(
        b.ack(6)?,
        0,
        "acknowledging the same seq twice deletes nothing"
    );
    assert_eq!(b.pending(None)?, 3, "6 is kept, and 6 is not pending");
    assert_eq!(
        b.pending(Some(5))?,
        4,
        "kept, though: 6 through 9 are there"
    );
    drop(b);

    let b = Buffer::open(&path, &key)?;
    assert_eq!(b.floor()?, Some(6));
    assert_eq!(b.pending(None)?, 3);
    assert_eq!(
        b.read_from(None, 100)?
            .envelopes
            .iter()
            .map(|e| e.link.seq)
            .collect::<Vec<_>>(),
        vec![7, 8, 9]
    );
    Ok(())
}

/// The runtime hands one `Buffer` to the polling side and the uploading side at once, so the
/// type has to survive an `Arc` across threads. A compile-time assertion rather than a
/// runtime one: if `age`'s recipient ever stops being `Sync`, the failure belongs here and
/// not in a runtime nobody is watching.
#[test]
fn a_buffer_can_be_shared_across_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Buffer>();
}

// ----------------------------------------------------------------------------------------
// The uploader that drains the buffer — signed chunks, verified acknowledgements,
// and a catch-up that says how far behind it is.
//
// Every case below runs against a real `goalcup_devserver::DevServer` on a loopback port in
// this process (behind the recording proxy in `support/mock_server.rs`), because the whole
// of what makes an acknowledgement worth anything is that somebody other than the uploader
// decided to sign it. A mock that agreed with whatever was sent would test the uploader
// against its own opinion.
// ----------------------------------------------------------------------------------------

#[tokio::test]
async fn a_chunk_is_signed_the_ack_is_verified_and_only_a_verified_ack_reaches_the_trace() {
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_devserver(t.path()).await; // buffer + writer + uploader
    for seq in 0..5u64 {
        h.buffer.append(&support::envelope(seq, "main")).unwrap();
    }

    let out = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!((out.uploaded, out.acked_seq), (5, Some(4)));
    assert!(!out.ack_unverified);
    let sent = h.devserver.last_chunk().unwrap();
    let run_key = h.run_keys.public_key_hex().unwrap();
    assert_eq!(sent.run_public_key, run_key);
    assert_eq!((sent.claims.first_seq, sent.claims.last_seq), (0, 4));
    assert!(
        sent.verify_with(&run_key).is_ok(),
        "R11: every chunk carries a per-run signature"
    );
    assert_eq!(
        sent.claims.chunk_id,
        sent.claims.derived_id(),
        "the id is derived from the range, never carried in from the uploader"
    );
    assert_eq!(
        sent.claims.chunk_id,
        goalcup_protocol::chunk::ChunkClaims::id("e1", 0, 4, &sent.claims.head)
    );

    // acks.jsonl carries the verified acknowledgement, with the time the server saw it.
    let line =
        std::fs::read_to_string(h.trace_dir.join(goalcup_protocol::export::ACKS_FILE)).unwrap();
    let ack: goalcup_protocol::ack::AckCheckpoint = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(ack.seq, 4);
    assert!(ack.acked_at_secs().is_some());
    assert!(
        ack.verify(&goalcup_devserver::dev_server_public_key_hex())
            .is_ok()
    );

    // An acknowledgement the pinned key does not vouch for anchors nothing.
    h.devserver
        .sign_next_with(&ed25519_dalek::SigningKey::from_bytes(&[4u8; 32]));
    h.buffer.append(&support::envelope(5, "main")).unwrap();
    let out = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert!(out.ack_unverified);
    assert_eq!(
        out.acked_seq,
        Some(4),
        "the last GOOD anchor, not the forged one"
    );
    assert_eq!(
        std::fs::read_to_string(h.trace_dir.join(goalcup_protocol::export::ACKS_FILE))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert_eq!(
        h.buffer.floor().unwrap(),
        Some(4),
        "and the buffer was not advanced over it"
    );
}

#[tokio::test]
async fn an_outage_backs_off_and_the_catch_up_says_the_sentence_r13_asks_for() {
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_devserver(t.path()).await;
    h.devserver.set_offline(true);
    for seq in 0..40u64 {
        h.buffer.append(&support::envelope(seq, "main")).unwrap();
    }

    let first = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert!(
        first.offline && !first.backing_off,
        "a closed laptop is the normal path (D9)"
    );
    assert!(
        h.uploader
            .pump(&h.buffer, &mut h.writer)
            .await
            .unwrap()
            .backing_off
    );
    assert!(h.uploader.backoff_seconds() > 0.0);

    h.devserver.set_offline(false);
    h.uploader.resume(None);
    let back = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert!(back.back_online);
    assert!(back.message.unwrap().contains("back online"));
    assert_eq!(h.uploader.backoff_seconds(), 0.0);

    // Idempotency, at the level `pump`'s ordering actually depends on: the exact bytes that
    // were acknowledged, offered again, answer the *same* acknowledgement rather than a
    // second one at a later time. That is what makes "append_ack, then buffer.ack" safe — a
    // failed append leaves the range in the buffer to be re-offered, and re-offering it does
    // not move the server. Driving this through the uploader would prove nothing: the range
    // is acknowledged, so a second pump has nothing left to send and `last_chunk()` would
    // still be the first chunk however the clamps behaved.
    let sent = h.devserver.last_chunk().unwrap();
    let posted = h.devserver.chunk_count();
    let first_ack = h.uploader.last_ack().cloned().unwrap();
    let again = UploadClient::new(h.url.clone()).send(&sent).await.unwrap();
    assert_eq!(
        h.devserver.chunk_count(),
        posted + 1,
        "the same bytes really did go back over the wire"
    );
    assert_eq!(
        again, first_ack,
        "the same document, the same acknowledgement"
    );

    // 7b: the server's clock is the only external time reference; report, never correct.
    h.devserver.set_clock("2030-01-01T00:00:00Z");
    h.buffer.append(&support::envelope(40, "main")).unwrap();
    let out = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert!(out.clock_skew_seconds.unwrap() > 120.0);
}

/// Recovery at the size it actually happens: an outage longer than one chunk drains
/// in chunk-sized steps, and the first pump back says how far behind the run is. Bounded
/// memory is the point — a catch-up that read the whole outage into one chunk would be a
/// daemon whose peak allocation is the length of the laptop's lid being shut.
#[tokio::test]
async fn a_catch_up_drains_in_chunk_sized_steps_and_names_how_far_behind_it_was() {
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_chunk_max(t.path(), 16).await;
    h.devserver.set_offline(true);
    for seq in 0..40u64 {
        h.buffer.append(&support::envelope(seq, "main")).unwrap();
    }
    assert!(
        h.uploader
            .pump(&h.buffer, &mut h.writer)
            .await
            .unwrap()
            .offline
    );

    h.devserver.set_offline(false);
    h.uploader.resume(None);
    let back = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert!(back.back_online);
    assert_eq!(
        back.message.as_deref(),
        Some("back online, 40 behind"),
        "R13 asks for the count, not just the fact"
    );
    assert_eq!((back.uploaded, back.acked_seq), (16, Some(15)));

    let second = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert!(!second.back_online, "it only comes back online once");
    assert_eq!((second.uploaded, second.acked_seq), (16, Some(31)));
    let third = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!((third.uploaded, third.acked_seq), (8, Some(39)));
    let idle = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!((idle.uploaded, idle.acked_seq), (0, Some(39)));

    // Three chunks on the wire, none of them bigger than the bound, and one acknowledgement
    // line per chunk.
    let sizes: Vec<usize> = h
        .devserver
        .chunks()
        .iter()
        .map(|c| c.envelopes.len())
        .collect();
    assert_eq!(
        sizes,
        vec![16, 16, 16, 8],
        "the offline attempt sent one too"
    );
    assert_eq!(support::ack_lines(&h.trace_dir), 3);
    assert_eq!(h.buffer.floor().unwrap(), Some(39));
}

/// A refusal is not an outage and is not retried into. The devserver refuses a chunk whose
/// links do not make the chain it acknowledged, and it refuses it *permanently* — so an
/// uploader that answered by sending the same bytes again would spin forever. It backs off
/// instead, doubling to the five-minute cap, and it advances nothing.
#[tokio::test]
async fn a_refusal_backs_off_to_the_cap_rather_than_retrying_forever() {
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_devserver(t.path()).await;
    // A chain the receipt does not witness: seq 0 hashes to something other than the
    // `chain_head` the entry was opened with.
    for seq in 0..4u64 {
        h.buffer.append(&support::envelope(seq, "other")).unwrap();
    }

    let out = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!(
        out.rejected,
        Some(400),
        "the server's own status, not a guess"
    );
    assert!(!out.offline, "a refusal is not an outage");
    assert_eq!((out.uploaded, out.acked_seq), (0, None));
    assert!(out.message.is_some());
    assert!(h.uploader.backoff_seconds() > 0.0);

    // The next pump does not even try: "we did not try" and "we tried and could not reach
    // it" are different facts.
    let gated = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert!(gated.backing_off && !gated.offline && gated.rejected.is_none());
    assert_eq!(h.devserver.chunk_count(), 1, "one refusal, one attempt");

    // Driven past the gate, the delay doubles and stops at five minutes.
    let mut seen = Vec::new();
    for _ in 0..14 {
        h.uploader.resume(None);
        h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
        seen.push(h.uploader.backoff_seconds());
    }
    assert!(seen[1] > seen[0], "it doubles: {seen:?}");
    let last = seen[seen.len() - 1];
    assert!(
        (150.0..=300.0).contains(&last),
        "capped at five minutes, jitter and all: {last}"
    );
    assert!(seen.iter().all(|s| *s <= 300.0), "{seen:?}");

    // Nothing was believed and nothing moved.
    assert_eq!(h.buffer.floor().unwrap(), None);
    assert_eq!(support::ack_lines(&h.trace_dir), 0);
    assert!(h.uploader.last_ack().is_none());
}

/// A hole in the buffer is a health fact, not something to wait for. Nothing refills a gap
/// in a write-ahead log, so the uploader reports it and keeps sending what it has rather
/// than retrying the missing seq for the rest of the run.
#[tokio::test]
async fn a_gap_is_reported_and_never_retried_into() {
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_devserver(t.path()).await;
    for seq in [0u64, 1, 2, 4, 5] {
        h.buffer.append(&support::envelope(seq, "main")).unwrap();
    }

    let out = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!((out.uploaded, out.acked_seq), (3, Some(2)));
    assert_eq!(out.gap_at, Some(3), "the seq that is missing");

    let again = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!(again.gap_at, Some(3));
    assert_eq!(again.uploaded, 0);
    assert!(!again.offline && again.rejected.is_none());
    assert_eq!(
        h.devserver.chunk_count(),
        1,
        "a gap is not an attempt: nothing was posted the second time"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn upload_refill_refuses_a_chain_leaf_exchange_and_restore_does_not_unpoison() {
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_devserver(t.path()).await;
    h.buffer.append(&support::envelope(0, "main")).unwrap();
    h.buffer.append(&support::envelope(2, "main")).unwrap();

    let named = h.trace_dir.join(goalcup_protocol::export::CHAIN_FILE);
    let original = h.trace_dir.join("held-chain-original");
    std::fs::rename(&named, &original).expect("the writer's chain is detached");
    std::fs::write(&named, b"replacement chain").expect("the replacement is planted");

    assert!(h.uploader.pump(&h.buffer, &mut h.writer).await.is_err());
    assert_eq!(h.devserver.chunk_count(), 0, "no request was attempted");
    assert_eq!(
        h.uploader.chain_bytes_scanned(),
        0,
        "unsafe acquisition never reached the blocking scanner"
    );

    std::fs::remove_file(&named).expect("the replacement is removed");
    std::fs::rename(&original, &named).expect("the original name is restored");
    assert!(
        h.writer.independent_chain_reader().is_err(),
        "the restored directory entry cannot unpoison the writer"
    );
}

/// D14: the run key stays in the process. What leaves it is a chunk carrying the *public*
/// key and a signature — never the signing seed, never the buffer key, never the receiver
/// token.
#[tokio::test]
async fn the_run_key_leaves_the_process_only_as_a_signature() {
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_devserver(t.path()).await;
    for seq in 0..3u64 {
        h.buffer.append(&support::envelope(seq, "main")).unwrap();
    }
    h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();

    let wire = serde_json::to_string(&h.devserver.last_chunk().unwrap()).unwrap();
    assert!(wire.contains(&h.run_keys.public_key_hex().unwrap()));
    for secret in [
        &h.run_keys.signing_seed_hex,
        &h.run_keys.buffer_key_hex,
        &h.run_keys.receiver_token,
    ] {
        assert!(!wire.contains(secret.as_str()), "a secret reached the wire");
    }
}

/// The failure the whole "append_ack, then buffer.ack" ordering is for, and the one thing
/// that ordering does not by itself buy: a trace directory that will not take the anchor is
/// a *persisting* condition — a full disk, a read-only artifact root — so an uploader that
/// treated it as a one-off would post the same chunk again on the very next tick, forever,
/// for as long as the disk stayed full. The persist failure has to arm the backoff exactly
/// as an outage does.
#[tokio::test]
async fn a_trace_that_will_not_take_the_acknowledgement_arms_the_backoff() {
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_devserver(t.path()).await;
    for seq in 0..5u64 {
        h.buffer.append(&support::envelope(seq, "main")).unwrap();
    }
    h.writer.fail_next_ack();

    let failed = h.uploader.pump(&h.buffer, &mut h.writer).await;
    assert!(
        failed.is_err(),
        "a trace that cannot hold the anchor is one of the two things that really are broken"
    );
    assert_eq!(h.devserver.chunk_count(), 1);
    assert_eq!(
        h.buffer.floor().unwrap(),
        None,
        "nothing is truncated behind an anchor that never reached acks.jsonl"
    );
    assert!(
        h.uploader.backoff_seconds() > 0.0,
        "the persist failure armed the backoff"
    );

    let gated = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert!(gated.backing_off && !gated.offline && gated.rejected.is_none());
    assert_eq!(
        h.devserver.chunk_count(),
        1,
        "and the same chunk was not re-posted inside the backoff window"
    );
}

/// A caller's resume position moves the start forward and never over anything. A
/// position above the daemon's own watermark names envelopes it holds no acknowledgement
/// for, and the `Buffer::ack` that followed a success would delete them unread — a hole
/// punched in the run by the resync path that exists to repair one.
#[tokio::test]
async fn a_resume_above_the_watermark_does_not_step_over_unacknowledged_envelopes() {
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_devserver(t.path()).await;
    for seq in 0..5u64 {
        h.buffer.append(&support::envelope(seq, "main")).unwrap();
    }
    h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!(h.buffer.floor().unwrap(), Some(4));
    for seq in 5..10u64 {
        h.buffer.append(&support::envelope(seq, "main")).unwrap();
    }

    // A caller that believes the server is four envelopes further along than this daemon's
    // watermark says. The belief is not obeyed at the cost of seqs 5 through 8.
    h.uploader.resume(Some(8));
    let out = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!(
        h.devserver.last_chunk().unwrap().claims.first_seq,
        5,
        "one above the durable watermark, not one above the caller's guess"
    );
    assert_eq!((out.uploaded, out.acked_seq), (5, Some(9)));
    assert_eq!(h.buffer.floor().unwrap(), Some(9));
    assert_eq!(support::ack_lines(&h.trace_dir), 2);
}

/// A hole does not stop being a hole because the uploader is waiting. The backoff gate is
/// about *requests*, and `gap_at` is a fact about the buffer on this machine, so a health
/// surface polling through an outage must not watch the hole blink out and back.
#[tokio::test]
async fn a_hole_is_still_reported_while_the_uploader_is_backing_off() {
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_devserver(t.path()).await;
    h.devserver.set_offline(true);
    for seq in [0u64, 1, 2, 4, 5] {
        h.buffer.append(&support::envelope(seq, "main")).unwrap();
    }

    let out = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert!(out.offline);
    assert_eq!(out.gap_at, Some(3));

    let gated = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert!(gated.backing_off && !gated.offline);
    assert_eq!(
        gated.gap_at,
        Some(3),
        "the hole is reported by the pump that did not try, too"
    );
}

/// The other half of "the daemon verifies, and never believes". An acknowledgement can be
/// genuinely the server's — right key, right signature — and still not be an acknowledgement
/// of what this daemon sent: a replay of an earlier range, or a server that lost its place.
/// The pinned key says yes and the range says no, and the range wins. It is reported as its
/// own fact rather than folded into `offline`, because a status surface that could not tell
/// the two apart would read "the server is answering the wrong questions" as "the café wifi
/// is bad".
#[tokio::test]
async fn a_genuinely_signed_acknowledgement_for_another_range_anchors_nothing() {
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_devserver(t.path()).await;
    for seq in 0..5u64 {
        h.buffer.append(&support::envelope(seq, "main")).unwrap();
    }
    h.devserver.ack_next_at(9);

    let out = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert!(out.ack_mismatched, "the range is not the one that was sent");
    assert!(
        !out.ack_unverified,
        "the signature was good; it is the document that is wrong"
    );
    assert!(!out.offline && out.rejected.is_none());
    assert_eq!((out.uploaded, out.acked_seq), (0, None));
    assert!(out.message.unwrap().contains("seq 9"));
    assert_eq!(
        h.buffer.floor().unwrap(),
        None,
        "the buffer was not advanced over an acknowledgement of a range it never sent"
    );
    assert_eq!(support::ack_lines(&h.trace_dir), 0);
    assert!(h.uploader.last_ack().is_none());
    assert!(h.uploader.backoff_seconds() > 0.0);

    // And the honest answer, next time, is believed: nothing about the mismatch is sticky.
    h.uploader.resume(None);
    let good = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert!(!good.ack_mismatched);
    assert_eq!((good.uploaded, good.acked_seq), (5, Some(4)));
}

/// Every chaining call appends to `chain.jsonl` and then to the buffer, and the
/// export does not fsync — so an ordinary `kill -9` of the daemon leaves a seq the chain
/// holds and the buffer does not. `read_from` then stops there forever and the run is
/// capped at `gap_at - 1`: every record for the rest of the 168 hours is unanchored, while
/// the participant reads `UploadLagging`, whose words are "nothing is lost".
///
/// The chain is the authority on what was recorded, so the hole is asked of it — once per
/// hole, streamed — and the upload catches up. A second hole further up is a second seq and
/// gets its own read.
#[tokio::test]
async fn a_record_the_chain_holds_and_the_buffer_lost_is_refilled_and_anchored() {
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_devserver(t.path()).await;

    // The chain has every link; the buffer is missing the one a `kill -9` tore out.
    write_chain(&h.trace_dir, 0..=5);
    for seq in [0u64, 1, 2, 4, 5] {
        h.buffer.append(&support::envelope(seq, "main")).unwrap();
    }

    let out = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!(out.refilled, 1, "the chain's own line went back in");
    assert_eq!(out.gap_at, None, "and the hole is closed");
    // The scan stopped at the line it wanted, and not at the end of the file.
    assert_eq!(
        h.uploader.chain_bytes_scanned(),
        chain_bytes(0..=3),
        "the refill read past the link it was looking for"
    );
    assert_eq!(
        (out.uploaded, out.acked_seq),
        (6, Some(5)),
        "the whole run is anchored, not the three below the hole"
    );
    assert_eq!(h.devserver.acked_seq("e1"), Some(5));

    // Once per hole: a second pump over the same buffer reads no chain and refills nothing.
    let again = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!((again.refilled, again.gap_at), (0, None));

    // A second hole, further up, is its own seq and its own single read.
    write_chain(&h.trace_dir, 0..=8);
    for seq in [6u64, 8] {
        h.buffer.append(&support::envelope(seq, "main")).unwrap();
    }
    let second = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!(second.refilled, 1, "a second gap refills again");
    assert_eq!(second.gap_at, None);
    assert_eq!(h.devserver.acked_seq("e1"), Some(8));

    // And it resumed. The chain is append-only and its seqs are monotonic,
    // so the second hole is further into the file than the first scan stopped — a refill
    // that started over would have read lines 0 through 7 to find seq 7, on a file that
    // reaches tens of gigabytes over a 168-hour run.
    assert_eq!(
        h.uploader.chain_bytes_scanned(),
        chain_bytes(0..=3) + chain_bytes(4..=7),
        "the second refill re-read the chain from the beginning"
    );
}

/// The bytes `write_chain` puts on disk for `seqs`, so a scan's reach can be named exactly
/// rather than bounded.
fn chain_bytes(seqs: impl IntoIterator<Item = u64>) -> u64 {
    seqs.into_iter()
        .map(|seq| {
            serde_json::to_string(&support::envelope(seq, "main"))
                .expect("an envelope writes")
                .len() as u64
                + 1
        })
        .sum()
}

/// A hole the chain cannot answer for is the other shape of the same fact: the record was
/// never recorded anywhere, so nothing refills it and `gap_at` survives — which is the
/// health fact the run is capped by, and the one `UploadLagging` must not be mistaken for.
#[tokio::test]
async fn a_hole_the_chain_cannot_answer_for_stays_a_reported_gap() {
    let t = tempfile::tempdir().unwrap();
    let mut h = support::harness_with_devserver(t.path()).await;

    // The chain is missing seq 3 too: it was never recorded at all.
    write_chain(&h.trace_dir, [0u64, 1, 2, 4, 5]);
    for seq in [0u64, 1, 2, 4, 5] {
        h.buffer.append(&support::envelope(seq, "main")).unwrap();
    }

    let out = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!(out.refilled, 0);
    assert_eq!(out.gap_at, Some(3), "the fact survives the attempt");
    assert_eq!((out.uploaded, out.acked_seq), (3, Some(2)));

    // And it is not re-read every pump: one hole, one read of a chain that reaches tens of
    // gigabytes over a 168-hour run.
    let again = h.uploader.pump(&h.buffer, &mut h.writer).await.unwrap();
    assert_eq!((again.refilled, again.gap_at), (0, Some(3)));
}

/// `chain.jsonl` as the export would have left it, for a run whose buffer then lost a line.
fn write_chain(trace_dir: &std::path::Path, seqs: impl IntoIterator<Item = u64>) {
    let text: String = seqs
        .into_iter()
        .map(|seq| {
            serde_json::to_string(&support::envelope(seq, "main")).expect("an envelope writes")
                + "\n"
        })
        .collect();
    std::fs::write(
        trace_dir.join(goalcup_protocol::export::CHAIN_FILE),
        text.as_bytes(),
    )
    .expect("the chain is written");
}
