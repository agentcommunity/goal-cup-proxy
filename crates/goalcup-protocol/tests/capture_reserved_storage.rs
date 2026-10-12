//! Physical reservation of the actual protocol-v5 capture chain file.
//!
//! Every fixture is a real `TraceExport`. Physical allocation is measured independently of the
//! code under test, from `st_blocks × 512` of `chain.jsonl` as the filesystem reports it, and the
//! logical end of file from `st_size`; `capture_storage()` is only ever compared against those
//! measurements, never trusted on its own.
//!
//! The small-reservation tests run on the normal temporary directory, which is APFS on the
//! supported macOS target. The disposable-volume tests — remount, a competing writer that
//! exhausts the volume, and the unsupported HFS+/FAT32 filesystems — attach `hdiutil` images and
//! carry two locks: `#[ignore]`, and `GOALCUP_T03R_VOLUME=1`, which a person types on purpose,
//! so neither `cargo test` nor `cargo test -- --include-ignored` attaches an image. They are run
//! by hand with
//!
//! ```sh
//! GOALCUP_T03R_VOLUME=1 cargo test -p goalcup-protocol --offline --test capture_reserved_storage -- \
//!     --ignored --exact --nocapture --test-threads=1 \
//!     reserved_chains_survive_remount_and_exhaustion unsupported_allocator_never_claims_reserved_bytes
//! ```
//!
//! The export-level refusals that need an unqualified filesystem or a short allocation run in
//! the default gate through the platform's debug-only, fail-closed `ReservationFault` seam.
//!
//! Off macOS only a Linux ext4 or XFS volume has an allocator, measured in Docker on Linux 7.0.12
//! (aarch64) loop-mounted images; every other filesystem refuses and reports nothing. The one
//! off-macOS test classifies its temporary directory itself and expects whichever of the two
//! follows.

// The pilot fixtures below are macOS-only; elsewhere only the refusal test uses these helpers.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use goalcup_platform::tracecap::{RefusalKind, TraceCapError};
use goalcup_protocol::capture::{
    CaptureRecord, CaptureTime, GapReason, M2_PROTOCOL_VERSION, StreamKey, TERMINAL_RESERVE_BYTES,
};
use goalcup_protocol::chain::Link;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::*;
use goalcup_protocol::provenance::{Rfc3339Utc, TerminationOutcomeV1};
use goalcup_protocol::redaction::RedactionAudit;
use goalcup_protocol::seal::SealManifest;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

const ENTRY: &str = "entry-m2-0001";
const PRODUCER: &str = "0123456789abcdef0123456789abcdef";
/// A small, deliberately unaligned pilot-shaped quota: 256 KiB + 123 bytes of payload in front
/// of the protected terminal allowance, which the quota includes.
const QUOTA: u64 = TERMINAL_RESERVE_BYTES + 256 * 1024 + 123;
/// Filesystem rounding a single top-up may add (APFS rounds to its 4 KiB block). Far below a
/// second quota, so a doubled allocation cannot hide inside it.
const ROUNDING: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Physical {
    /// `st_size`: the logical JSONL length.
    size: u64,
    /// `st_blocks × 512`: what the filesystem has actually allocated to the chain file.
    allocated: u64,
}

fn physical(root: &Path) -> Physical {
    let meta = std::fs::symlink_metadata(root.join(CHAIN_FILE)).unwrap();
    Physical {
        size: meta.len(),
        allocated: meta.blocks() * 512,
    }
}

fn at(mono_ns: u64) -> CaptureTime {
    CaptureTime {
        wall: "2026-09-28T12:00:00.250Z".to_string(),
        mono_ns,
        clock_id: "daemon-5b0c3a1e".to_string(),
    }
}

fn manifest() -> SealManifest {
    SealManifest {
        entry_id: ENTRY.to_string(),
        sealed_at: "2026-09-28T12:00:00Z".to_string(),
        workspace: "/ws".to_string(),
        files: Vec::new(),
        excluded: Vec::new(),
    }
}

fn v5(record: CaptureRecord) -> Envelope {
    Envelope {
        protocol_version: M2_PROTOCOL_VERSION,
        entry_id: ENTRY.to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-28T12:00:01Z".to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body: EventBody::Capture { record },
        redaction: None,
    }
}

/// Payload: a sanitized segment whose base64 text is `data_len` bytes long.
fn segment(part: u64, data_len: usize) -> Envelope {
    v5(CaptureRecord::Segment {
        stream: StreamKey {
            producer: PRODUCER.to_string(),
            stream: 1,
        },
        part,
        source_offset: part * 1024,
        source_len: 1024,
        first: at(10),
        last: at(20),
        data_b64: "A".repeat(data_len),
        audit: RedactionAudit { spans: Vec::new() },
    })
}

/// A bounded closure: spends the terminal allowance, never payload.
fn closure_gap(mono_ns: u64) -> Envelope {
    v5(CaptureRecord::Gap {
        stream: None,
        observed: at(mono_ns),
        reason: GapReason::StorageFull,
        lost_bytes: None,
    })
}

fn stopped() -> TerminationOutcomeV1 {
    TerminationOutcomeV1::AlreadyStopped {
        checked_at: Rfc3339Utc::try_from("2026-09-28T12:00:02Z").unwrap(),
    }
}

fn snapshot(root: &Path) -> Vec<(std::ffi::OsString, Vec<u8>)> {
    let mut entries = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), std::fs::read(entry.path()).unwrap())
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

/// Every byte of the chain is a complete JSONL envelope: reservation never leaves zero fill,
/// padding or a torn line in the logical data.
fn assert_chain_is_whole_jsonl(root: &Path) -> usize {
    let bytes = std::fs::read(root.join(CHAIN_FILE)).unwrap();
    assert_eq!(
        bytes.last(),
        Some(&b'\n'),
        "the chain ends on a complete line"
    );
    assert!(
        !bytes.contains(&0),
        "no allocator fill reached the logical data"
    );
    let mut lines = 0;
    for line in bytes.split(|byte| *byte == b'\n').filter(|l| !l.is_empty()) {
        let env: Envelope = serde_json::from_slice(line).unwrap();
        assert_eq!(env.link.seq, lines as u64);
        lines += 1;
    }
    lines
}

/// Refused as unsupported: the unqualified allocator's typed refusal.
fn is_unsupported(result: &Result<(), ExportError>) -> bool {
    matches!(
        result,
        Err(ExportError::Capability(TraceCapError::Refused {
            reason: RefusalKind::UnsupportedFileSystem,
            ..
        }))
    )
}

#[cfg(target_os = "macos")]
#[test]
fn reserve_does_not_extend_chain_eof() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    assert_eq!(
        physical(tmp.path()),
        Physical {
            size: 0,
            allocated: 0
        }
    );

    export.reserve_capture(QUOTA).unwrap();

    let reserved = physical(tmp.path());
    assert_eq!(reserved.size, 0, "reservation never moves logical EOF");
    assert!(
        reserved.allocated >= QUOTA,
        "the whole quota, terminal allowance included, is physically allocated: {reserved:?}"
    );
    assert!(
        reserved.allocated < QUOTA + ROUNDING,
        "one allocation of the quota, not more: {reserved:?}"
    );
    let storage = export.capture_storage();
    assert_eq!(storage.quota_bytes, QUOTA);
    assert_eq!(storage.reserved_bytes, QUOTA, "measured, then reported");
    assert_eq!(storage.used_bytes, 0);
    assert!(storage.accounting_available);

    // The reservation selected capture exactly as a quota does: the v5 baseline and payload are
    // admitted, and they spend already-allocated blocks rather than growing the file.
    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    for part in 0..16 {
        export.append_chain(&mut segment(part, 4096)).unwrap();
    }
    export.sync_capture().unwrap();
    let written = physical(tmp.path());
    assert!(written.size > 64 * 1024);
    assert_eq!(
        written.allocated, reserved.allocated,
        "appends consumed the reservation, not new blocks"
    );
    assert_eq!(export.capture_storage().used_bytes, written.size);
    assert_eq!(export.capture_storage().reserved_bytes, QUOTA);
    assert_chain_is_whole_jsonl(tmp.path());
}

#[cfg(target_os = "macos")]
#[test]
fn repeated_reservation_tops_up_rather_than_adding() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export.reserve_capture(QUOTA).unwrap();
    let first = physical(tmp.path());

    // F_PREALLOCATE is additive from the physical end of file; the reservation must not be.
    export.reserve_capture(QUOTA).unwrap();
    assert_eq!(physical(tmp.path()), first, "a repeat is an empty top-up");
    let smaller = QUOTA - 64 * 1024;
    export.reserve_capture(smaller).unwrap();
    assert_eq!(
        physical(tmp.path()),
        first,
        "a smaller quota neither adds nor shrinks"
    );
    assert_eq!(export.capture_storage().quota_bytes, smaller);
    assert_eq!(export.capture_storage().reserved_bytes, smaller);

    let larger = QUOTA + 200_000 + 7;
    export.reserve_capture(larger).unwrap();
    let grown = physical(tmp.path());
    assert_eq!(grown.size, 0);
    assert!(grown.allocated >= larger, "{grown:?}");
    assert!(
        grown.allocated < larger + ROUNDING,
        "topped up by the difference, not by the whole new quota: {grown:?}"
    );
    assert_eq!(export.capture_storage().reserved_bytes, larger);

    // The quota-only path stays an explicitly unreserved experiment, even over real blocks.
    export.configure_capture_quota(larger).unwrap();
    assert_eq!(export.capture_storage().quota_bytes, larger);
    assert_eq!(
        export.capture_storage().reserved_bytes,
        0,
        "configure_capture_quota never reports a reservation"
    );
    assert_eq!(physical(tmp.path()), grown);
}

#[cfg(target_os = "macos")]
#[test]
fn allocation_failure_refuses_pilot() {
    // Far beyond any volume's free space: refused before any allocation is attempted, so the
    // host filesystem is never asked to fill.
    for impossible in [1u64 << 60, u64::MAX] {
        let tmp = tempfile::tempdir().unwrap();
        let mut export = TraceExport::create(tmp.path()).unwrap();
        let before = snapshot(tmp.path());

        let refused = export.reserve_capture(impossible);
        assert!(
            matches!(
                refused,
                Err(ExportError::Capability(TraceCapError::Refused {
                    reason: RefusalKind::InsufficientSpace,
                    ..
                }))
            ),
            "{impossible}: {refused:?}"
        );
        let storage = export.capture_storage();
        assert_eq!(storage.reserved_bytes, 0, "a refusal claims nothing");
        assert_eq!(storage.quota_bytes, 0, "and enables no smaller budget");
        assert_eq!(
            physical(tmp.path()),
            Physical {
                size: 0,
                allocated: 0
            }
        );
        // Capture was never selected: the pilot's v5 genesis cannot be written.
        assert!(
            export
                .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
                .is_err()
        );
        assert_eq!(snapshot(tmp.path()), before, "retained evidence unchanged");
    }

    // A reserved chain asked for more keeps exactly its measured budget: neither inflated to
    // the refused request nor silently reduced.
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export.reserve_capture(QUOTA).unwrap();
    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    export.append_chain(&mut segment(0, 512)).unwrap();
    export.sync_capture().unwrap();
    let held = physical(tmp.path());
    let storage = export.capture_storage();
    let before = snapshot(tmp.path());

    assert!(export.reserve_capture(1 << 60).is_err());
    assert_eq!(export.capture_storage(), storage);
    assert_eq!(physical(tmp.path()), held);
    assert_eq!(snapshot(tmp.path()), before);
    export.append_chain(&mut segment(1, 512)).unwrap();
}

/// Bytes this writer did not append would push the chain past the allocation its accounting
/// believes in, so the reservation is refused rather than reported.
#[cfg(target_os = "macos")]
#[test]
fn a_chain_length_this_writer_did_not_write_is_refused() {
    use std::io::Write as _;
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    std::fs::OpenOptions::new()
        .append(true)
        .open(tmp.path().join(CHAIN_FILE))
        .unwrap()
        .write_all(b"{\"foreign\":true}\n")
        .unwrap();

    let refused = export.reserve_capture(QUOTA);
    assert!(
        matches!(
            refused,
            Err(ExportError::ChainLengthChanged {
                validated_bytes: 0,
                found_bytes: 17
            })
        ),
        "{refused:?}"
    );
    let storage = export.capture_storage();
    assert_eq!((storage.quota_bytes, storage.reserved_bytes), (0, 0));
}

/// A refusal is never a quota-only fallback: an unqualified filesystem and an allocator that
/// grants nothing both leave no quota, no reported reservation and no v5 selection, on a fresh
/// chain and on one already reserved.
#[cfg(all(target_os = "macos", debug_assertions))]
#[test]
fn unqualified_or_short_allocation_never_falls_back_to_quota_only() {
    use goalcup_platform::tracecap::{ReservationFault, inject_reservation_fault};
    for (fault, reason) in [
        (
            ReservationFault::UnqualifiedFileSystem,
            RefusalKind::UnsupportedFileSystem,
        ),
        (
            ReservationFault::AllocatorGrantsNothing,
            RefusalKind::AllocationNotRetained,
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let mut export = TraceExport::create(tmp.path()).unwrap();
        let before = snapshot(tmp.path());

        inject_reservation_fault(fault);
        let refused = export.reserve_capture(QUOTA);
        assert!(
            matches!(
                &refused,
                Err(ExportError::Capability(TraceCapError::Refused { reason: found, .. }))
                    if *found == reason
            ),
            "{fault:?}: {refused:?}"
        );
        let storage = export.capture_storage();
        assert_eq!(
            (storage.quota_bytes, storage.reserved_bytes),
            (0, 0),
            "{fault:?}: no quota-only fallback"
        );
        assert!(
            export
                .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
                .is_err(),
            "{fault:?}: capture was not selected"
        );
        assert_eq!(physical(tmp.path()).allocated, 0);
        assert_eq!(snapshot(tmp.path()), before);

        // An already reserved chain asked to grow keeps exactly its measured budget.
        export.reserve_capture(QUOTA).unwrap();
        let held = physical(tmp.path());
        let storage = export.capture_storage();
        inject_reservation_fault(fault);
        let refused = export.reserve_capture(QUOTA + 256 * 1024);
        assert!(
            matches!(
                &refused,
                Err(ExportError::Capability(TraceCapError::Refused { reason: found, .. }))
                    if *found == reason
            ),
            "{fault:?}: {refused:?}"
        );
        assert_eq!(export.capture_storage(), storage, "{fault:?}");
        assert_eq!(physical(tmp.path()), held, "{fault:?}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn reservation_survives_close_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export.reserve_capture(QUOTA).unwrap();
    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    export.append_chain(&mut segment(0, 2048)).unwrap();
    export.sync_capture().unwrap();
    let held = physical(tmp.path());
    drop(export);

    let closed = physical(tmp.path());
    assert_eq!(closed, held, "closing the last descriptor releases nothing");
    assert!(closed.allocated >= QUOTA);

    let mut export = TraceExport::resume(tmp.path()).unwrap();
    assert_eq!(
        export.capture_storage().reserved_bytes,
        0,
        "a reopened writer claims nothing before it remeasures"
    );
    export.reserve_capture(QUOTA).unwrap();
    assert_eq!(
        physical(tmp.path()),
        held,
        "the retained allocation already satisfies the quota: nothing is added"
    );
    assert_eq!(export.capture_storage().reserved_bytes, QUOTA);
    export.append_chain(&mut segment(1, 2048)).unwrap();
    export
        .write_local_finish(&manifest(), at(99), "participant_stop", stopped())
        .unwrap();
    assert_eq!(physical(tmp.path()).allocated, held.allocated);
    assert_chain_is_whole_jsonl(tmp.path());
}

#[cfg(target_os = "macos")]
const CHILD: &str = "child_reserves_and_waits_to_be_killed";
#[cfg(target_os = "macos")]
const CHILD_ROOT: &str = "GOALCUP_T03R_CHILD_ROOT";
#[cfg(target_os = "macos")]
const CHILD_READY: &str = "GOALCUP_T03R_CHILD_READY";

/// Runs only as the child of [`reservation_survives_abnormal_child_exit`]: reserve through the
/// production path, then hold the descriptor open until killed.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "helper: runs only as a child process with its environment set"]
fn child_reserves_and_waits_to_be_killed() {
    let (Some(root), Some(ready)) = (std::env::var_os(CHILD_ROOT), std::env::var_os(CHILD_READY))
    else {
        return;
    };
    let mut export = TraceExport::create(Path::new(&root)).unwrap();
    export.reserve_capture(QUOTA).unwrap();
    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    export.append_chain(&mut segment(0, 1024)).unwrap();
    export.sync_capture().unwrap();
    std::fs::write(&ready, b"reserved").unwrap();
    for _ in 0..1200 {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("the parent never killed this child");
}

/// Reserve in a child process, `SIGKILL` it with the chain descriptor open, and return the
/// chain's physical measurement after the kernel has closed every descriptor.
#[cfg(target_os = "macos")]
fn reserve_in_killed_child(root: &Path) -> Physical {
    use std::os::unix::process::ExitStatusExt;
    let ready = root.with_extension("ready");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            CHILD,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ROOT, root)
        .env(CHILD_READY, &ready)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !ready.exists() {
        if std::time::Instant::now() > deadline || child.try_wait().unwrap().is_some() {
            let _ = child.kill();
            panic!("the child never reported its reservation");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let held = physical(root);
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(9), "the child died by SIGKILL");
    std::fs::remove_file(&ready).unwrap();
    assert!(held.allocated >= QUOTA, "{held:?}");
    held
}

#[cfg(target_os = "macos")]
#[test]
fn reservation_survives_abnormal_child_exit() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("trace");
    let held = reserve_in_killed_child(&root);

    let after = physical(&root);
    assert_eq!(
        after, held,
        "SIGKILL with the descriptor open released nothing"
    );

    let mut export = TraceExport::resume(&root).unwrap();
    assert_eq!(export.capture_storage().reserved_bytes, 0);
    export.reserve_capture(QUOTA).unwrap();
    assert_eq!(physical(&root), held, "remeasured, not reallocated");
    assert_eq!(export.capture_storage().reserved_bytes, QUOTA);
    export.append_chain(&mut segment(1, 1024)).unwrap();
    export
        .write_local_finish(&manifest(), at(99), "participant_stop", stopped())
        .unwrap();
    assert_eq!(physical(&root).allocated, held.allocated);
    assert_chain_is_whole_jsonl(&root);
}

#[cfg(target_os = "macos")]
#[test]
fn reopened_reservation_is_remeasured() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export.reserve_capture(QUOTA).unwrap();
    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    export.append_chain(&mut segment(0, 4096)).unwrap();
    export.sync_capture().unwrap();
    drop(export);
    let held = physical(tmp.path());

    // Restart: status claims nothing, and nothing is admitted, until the chain is remeasured.
    let mut export = TraceExport::resume(tmp.path()).unwrap();
    let storage = export.capture_storage();
    assert_eq!(
        (storage.quota_bytes, storage.reserved_bytes),
        (0, 0),
        "a restarted status snapshot does not carry the old reservation forward"
    );
    assert!(matches!(
        export.append_chain(&mut segment(1, 16)),
        Err(ExportError::CaptureQuotaNotConfigured)
    ));
    export.reserve_capture(QUOTA).unwrap();
    assert_eq!(physical(tmp.path()), held, "remeasured, not reallocated");
    assert_eq!(export.capture_storage().reserved_bytes, QUOTA);

    // The allocation is capacity, never admission: payload still stops at the quota minus the
    // terminal allowance, and closures stop at the quota.
    let mut part = 1;
    let payload_refusal = loop {
        match export.append_chain(&mut segment(part, 4096)) {
            Ok(_) => part += 1,
            Err(error) => break error,
        }
    };
    assert!(
        matches!(
            payload_refusal,
            ExportError::CaptureStorageFull {
                terminal_allowance: false,
                ..
            }
        ),
        "{payload_refusal:?}"
    );
    let payload_end = physical(tmp.path()).size;
    assert!(payload_end <= QUOTA - TERMINAL_RESERVE_BYTES);
    let mut mono = 1;
    let closure_refusal = loop {
        match export.append_chain(&mut closure_gap(mono)) {
            Ok(_) => mono += 1,
            Err(error) => break error,
        }
    };
    assert!(
        matches!(
            closure_refusal,
            ExportError::CaptureStorageFull {
                terminal_allowance: true,
                ..
            }
        ),
        "{closure_refusal:?}"
    );
    export.sync_capture().unwrap();

    let end = physical(tmp.path());
    assert!(end.size <= QUOTA, "no append beyond the quota: {end:?}");
    assert!(
        end.size > QUOTA - 4096,
        "the terminal allowance was really spent"
    );
    assert_eq!(
        end.allocated, held.allocated,
        "every admitted byte landed in reserved blocks"
    );
    assert_eq!(export.capture_storage().used_bytes, end.size);
    assert_eq!(export.capture_storage().reserved_bytes, QUOTA);
    assert!(!export.capture_storage().accounting_available);
    assert!(assert_chain_is_whole_jsonl(tmp.path()) > 100);
}

/// An assistance record, the fact that a person intervened, is bounded and rare: with the
/// payload part of the quota full, it spends the terminal allowance rather than being refused
/// as payload.
#[test]
fn assistance_spends_the_terminal_allowance() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    export.configure_capture_quota(QUOTA).unwrap();
    export
        .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
        .unwrap();
    // A quota whose payload part the baseline fills exactly.
    let payload_end = export.capture_storage().used_bytes;
    export
        .configure_capture_quota(payload_end + TERMINAL_RESERVE_BYTES)
        .unwrap();
    let payload = export.append_chain(&mut segment(0, 0));
    assert!(
        matches!(
            payload,
            Err(ExportError::CaptureStorageFull {
                available: 0,
                terminal_allowance: false,
                ..
            })
        ),
        "{payload:?}"
    );

    export
        .append_chain(&mut v5(CaptureRecord::Assistance {
            observed: at(30),
            action: "manual_resume".to_string(),
        }))
        .unwrap();
    assert!(export.capture_storage().used_bytes > payload_end);
    assert!(export.capture_storage().accounting_available);
    assert_eq!(assert_chain_is_whole_jsonl(tmp.path()), 2);
}

/// Off macOS the refusal is real, typed, and claims nothing, unless the temporary directory is
/// on Linux ext4 or XFS (classified here by `statfs(2)` `f_type`, not by the code under
/// test): then the quota is reserved behind an unmoved logical end and reported as measured.
#[cfg(not(target_os = "macos"))]
#[test]
fn unsupported_allocator_never_claims_reserved_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut export = TraceExport::create(tmp.path()).unwrap();
    let before = snapshot(tmp.path());
    let refused = export.reserve_capture(QUOTA);
    let qualified = linux_ext4_or_xfs(tmp.path());
    println!("{}: ext4 or XFS {qualified}", tmp.path().display());
    if qualified {
        refused.unwrap();
        let reserved = physical(tmp.path());
        assert_eq!(reserved.size, 0, "reservation never moves logical EOF");
        assert!(
            reserved.allocated >= QUOTA && reserved.allocated < QUOTA + ROUNDING,
            "one measured allocation of the quota: {reserved:?}"
        );
        let storage = export.capture_storage();
        assert_eq!(
            (storage.quota_bytes, storage.reserved_bytes),
            (QUOTA, QUOTA)
        );
        assert_eq!(snapshot(tmp.path()), before, "no logical byte was written");
        return;
    }
    assert!(is_unsupported(&refused), "{refused:?}");
    let storage = export.capture_storage();
    assert_eq!((storage.quota_bytes, storage.reserved_bytes), (0, 0));
    assert_eq!(physical(tmp.path()).allocated, 0);
    assert_eq!(snapshot(tmp.path()), before);
}

#[cfg(target_os = "macos")]
mod disposable {
    //! Disposable, size-limited `hdiutil` images for the by-hand volume tests. Every
    //! image lives in a private (0700) directory under this crate's target tmp directory, is
    //! verified to be a distinct small device of the expected filesystem after *every* attach
    //! and before any write, and is force-detached and deleted when its guard drops.

    use std::ffi::CString;

    /// The second lock (the repository's `GOALCUP_REGEN` convention): `#[ignore]` alone still
    /// runs under `--include-ignored`, so an image is attached only when a person set this.
    pub fn permitted(test: &str) -> bool {
        if std::env::var("GOALCUP_T03R_VOLUME").as_deref() == Ok("1") {
            return true;
        }
        println!(
            "{test}: GOALCUP_T03R_VOLUME=1 is not set, so no disposable hdiutil image was \
             attached. Run it by hand as the header of capture_reserved_storage.rs shows (R16)."
        );
        false
    }
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    pub const MIB: u64 = 1024 * 1024;

    pub struct Scratch(pub PathBuf);

    impl Scratch {
        pub fn new(label: &str) -> Self {
            let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
                .join(format!("t03r-{label}-{}", std::process::id()));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub struct Volume {
        pub image: PathBuf,
        pub mount: PathBuf,
        parent_device: u64,
        size_mib: u64,
        fstype: &'static str,
        attached: bool,
    }

    fn hdiutil(args: &[&str]) {
        let status = Command::new("hdiutil").args(args).status().unwrap();
        assert!(status.success(), "hdiutil {args:?} failed: {status}");
    }

    pub fn statfs(path: &Path) -> libc::statfs {
        let path = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        let mut buffer = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: the NUL-terminated path and writable output buffer are live for the call.
        let result = unsafe { libc::statfs(path.as_ptr(), buffer.as_mut_ptr()) };
        assert_eq!(result, 0, "statfs {}", std::io::Error::last_os_error());
        // SAFETY: statfs succeeded and initialized the buffer.
        unsafe { buffer.assume_init() }
    }

    pub fn fstypename(volume: &libc::statfs) -> String {
        volume
            .f_fstypename
            .iter()
            .take_while(|c| **c != 0)
            .map(|c| *c as u8 as char)
            .collect()
    }

    impl Volume {
        /// `hdiutil create` a sparse image of `size_mib` MiB and attach it without Finder.
        pub fn create(
            scratch: &Scratch,
            name: &str,
            hdiutil_fs: &str,
            fstype: &'static str,
            size_mib: u64,
        ) -> Self {
            let base = scratch.0.join(format!("vol-{name}"));
            let image = scratch.0.join(format!("vol-{name}.sparseimage"));
            assert!(!image.exists(), "refusing to reuse a stale image");
            let mount = scratch.0.join(format!("mnt-{name}"));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&mount)
                .unwrap();
            let mut volume = Self {
                image,
                mount,
                parent_device: std::fs::metadata(&scratch.0).unwrap().dev(),
                size_mib,
                fstype,
                attached: false,
            };
            hdiutil(&[
                "create",
                "-size",
                &format!("{size_mib}m"),
                "-fs",
                hdiutil_fs,
                "-volname",
                name,
                "-type",
                "SPARSE",
                "-nospotlight",
                base.to_str().unwrap(),
            ]);
            volume.attach();
            volume
        }

        fn attach(&mut self) {
            self.attached = true;
            hdiutil(&[
                "attach",
                self.image.to_str().unwrap(),
                "-nobrowse",
                "-mountpoint",
                self.mount.to_str().unwrap(),
            ]);
            self.verify();
        }

        /// A real unmount/remount of the same backing image at the same mount point.
        pub fn remount(&mut self) {
            hdiutil(&["detach", self.mount.to_str().unwrap()]);
            self.attached = false;
            self.attach();
        }

        /// Before any write: the mount is a distinct device, no larger than twice the image,
        /// carrying the expected filesystem. Anything else aborts the test (and the guard
        /// detaches), so a failed attach can never resolve to a host directory.
        fn verify(&self) {
            let device = std::fs::metadata(&self.mount).unwrap().dev();
            assert_ne!(
                device,
                self.parent_device,
                "SAFETY: {} is not a distinct device",
                self.mount.display()
            );
            let volume = statfs(&self.mount);
            let total = volume.f_blocks * u64::from(volume.f_bsize);
            assert!(
                total <= 2 * self.size_mib * MIB,
                "SAFETY: {} reports {total} bytes, not a {} MiB image",
                self.mount.display(),
                self.size_mib
            );
            assert_eq!(fstypename(&volume), self.fstype);
        }

        pub fn available(&self) -> u64 {
            let volume = statfs(&self.mount);
            volume.f_bavail * u64::from(volume.f_bsize)
        }
    }

    impl Drop for Volume {
        fn drop(&mut self) {
            if self.attached {
                let _ = Command::new("hdiutil")
                    .args(["detach", "-force", self.mount.to_str().unwrap()])
                    .status();
            }
            let _ = std::fs::remove_file(&self.image);
            let _ = std::fs::remove_dir(&self.mount);
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "attaches disposable hdiutil HFS+ and FAT32 images; set GOALCUP_T03R_VOLUME=1 and run by hand (R16)"]
fn unsupported_allocator_never_claims_reserved_bytes() {
    use disposable::{Scratch, Volume};
    if !disposable::permitted("unsupported_allocator_never_claims_reserved_bytes") {
        return;
    }
    let scratch = Scratch::new("unsupported");
    for (name, hdiutil_fs, fstype, size_mib) in [
        ("hfs", "HFS+", "hfs", 16),
        ("fat", "MS-DOS FAT32", "msdos", 64),
    ] {
        let mut volume = Volume::create(&scratch, name, hdiutil_fs, fstype, size_mib);
        let root = volume.mount.join("trace");
        let mut export = TraceExport::create(&root).unwrap();
        let before = physical(&root);
        let refused = export.reserve_capture(QUOTA);
        eprintln!("T03R {fstype}: reserve_capture({QUOTA}) -> {refused:?}; chain {before:?}");
        assert!(is_unsupported(&refused), "{fstype}: {refused:?}");
        let storage = export.capture_storage();
        assert_eq!((storage.quota_bytes, storage.reserved_bytes), (0, 0));
        assert_eq!(physical(&root), before, "{fstype}: nothing was allocated");
        drop(export);
        volume.remount();
        assert_eq!(physical(&root), before, "{fstype}: after a real remount");
    }
}

/// Exhaust the disposable volume with a competing writer, bounded by `cap` bytes, and confirm
/// exhaustion by a new metadata-style file failing with ENOSPC. Returns the bytes written.
#[cfg(target_os = "macos")]
fn exhaust(mount: &Path, cap: u64) -> u64 {
    use std::io::Write as _;
    let enospc = |error: &std::io::Error| error.raw_os_error() == Some(28);
    let mut written = 0u64;
    let mut index = 0;
    for round in 0..4 {
        for chunk in [1usize << 20, 64 << 10, 4 << 10] {
            let path = mount.join(format!("competing-{index}.bin"));
            index += 1;
            let mut file = match std::fs::File::create_new(&path) {
                Ok(file) => file,
                Err(error) if enospc(&error) => continue,
                Err(error) => panic!("competing writer: {error}"),
            };
            let buffer = vec![0x5a; chunk];
            loop {
                assert!(
                    written + chunk as u64 <= cap,
                    "the competing writer reached its bound before the volume filled"
                );
                match file.write_all(&buffer) {
                    Ok(()) => written += chunk as u64,
                    Err(error) if enospc(&error) => break,
                    Err(error) => panic!("competing writer: {error}"),
                }
            }
            let _ = file.sync_all();
        }
        let probe = mount.join("registry-probe.json");
        let attempt = std::fs::File::create_new(&probe).and_then(|mut file| {
            file.write_all(&[b'{'; 4096])?;
            file.sync_all()
        });
        match attempt {
            Err(error) if enospc(&error) => {
                let _ = std::fs::remove_file(&probe);
                eprintln!(
                    "T03R exhaustion: competing writer {written} bytes in {index} files, round {round}; \
                     a new metadata-style file fails with ENOSPC ({error})"
                );
                return written;
            }
            Err(error) => panic!("probe: {error}"),
            Ok(()) => std::fs::remove_file(&probe).unwrap(),
        }
    }
    panic!("the disposable volume could not be confirmed exhausted");
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "attaches a disposable 64 MiB hdiutil APFS image and exhausts it; set GOALCUP_T03R_VOLUME=1 and run by hand (R16)"]
fn reserved_chains_survive_remount_and_exhaustion() {
    use disposable::{MIB, Scratch, Volume};
    if !disposable::permitted("reserved_chains_survive_remount_and_exhaustion") {
        return;
    }
    let scratch = Scratch::new("exhaustion");
    let mut volume = Volume::create(&scratch, "apfs", "APFS", "apfs", 64);
    let quota_a = TERMINAL_RESERVE_BYTES + 4 * MIB + 123;
    let quota_b = TERMINAL_RESERVE_BYTES + MIB;
    let (a, b, c) = (
        volume.mount.join("entry-a"),
        volume.mount.join("entry-b"),
        volume.mount.join("entry-c"),
    );
    eprintln!("T03R volume: {} bytes available", volume.available());

    // Two independent entries reserved through the production path, and a third reserved by a
    // child that is SIGKILLed with its descriptor open.
    for (root, quota) in [(&a, quota_a), (&b, quota_b)] {
        let mut export = TraceExport::create(root).unwrap();
        export.reserve_capture(quota).unwrap();
        export
            .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
            .unwrap();
        export.sync_capture().unwrap();
        eprintln!(
            "T03R reserve {}: quota {quota} -> {:?}",
            root.display(),
            physical(root)
        );
    }
    let killed = reserve_in_killed_child(&c);
    eprintln!("T03R reserve entry-c (child SIGKILLed): {killed:?}");
    let reserved = [physical(&a), physical(&b), physical(&c)];

    // A real unmount/remount: only a persistent allocation survives it.
    volume.remount();
    let remounted = [physical(&a), physical(&b), physical(&c)];
    eprintln!("T03R after remount: {remounted:?}");
    assert_eq!(
        remounted, reserved,
        "every reservation survived the remount"
    );
    assert!(remounted[0].allocated >= quota_a && remounted[1].allocated >= quota_b);
    assert!(remounted[2].allocated >= QUOTA);

    let mut export_a = TraceExport::resume(&a).unwrap();
    let mut export_b = TraceExport::resume(&b).unwrap();
    let mut export_c = TraceExport::resume(&c).unwrap();
    export_a.reserve_capture(quota_a).unwrap();
    export_b.reserve_capture(quota_b).unwrap();
    export_c.reserve_capture(QUOTA).unwrap();
    assert_eq!(
        [physical(&a), physical(&b), physical(&c)],
        reserved,
        "remeasured after the remount without reallocating"
    );

    let competing = exhaust(&volume.mount, 64 * MIB);
    eprintln!(
        "T03R exhausted: {competing} competing bytes, {} bytes reported available",
        volume.available()
    );

    // Entry A: payload to its quota, bounded syncs, then closures and the terminal record, all
    // on the exhausted volume and all inside its own reserved blocks.
    let mut part = 0;
    let payload_refusal = loop {
        match export_a.append_chain(&mut segment(part, 16 * 1024)) {
            Ok(_) => {
                part += 1;
                if part % 32 == 0 {
                    export_a.sync_capture().unwrap();
                }
            }
            Err(error) => break error,
        }
    };
    assert!(
        matches!(
            payload_refusal,
            ExportError::CaptureStorageFull {
                terminal_allowance: false,
                ..
            }
        ),
        "{payload_refusal:?}"
    );
    export_a.sync_capture().unwrap();
    let mut mono = 1;
    while export_a.capture_storage().used_bytes + 8 * 1024 < quota_a {
        export_a.append_chain(&mut closure_gap(mono)).unwrap();
        mono += 1;
    }
    export_a.sync_capture().unwrap();
    let before_finish = physical(&a);
    let finish = export_a.write_local_finish(&manifest(), at(999), "participant_stop", stopped());
    let seal = std::fs::read(a.join(SEAL_FILE)).ok();
    eprintln!(
        "T03R entry-a on the exhausted volume: {part} payload segments, {} closures, \
         {before_finish:?} before finish; write_local_finish -> {:?}; seal.json bytes: {:?}",
        mono - 1,
        finish.as_ref().map(|_| ()),
        seal.as_ref().map(Vec::len),
    );
    if finish.is_ok() {
        assert_eq!(
            seal.as_deref(),
            Some(serde_json::to_vec(&manifest().sorted()).unwrap().as_slice()),
            "success means the whole seal was written"
        );
    }
    let finished = physical(&a);
    assert_eq!(
        finished.allocated, reserved[0].allocated,
        "no new block was needed"
    );
    assert!(finished.size <= quota_a && finished.size > quota_a - 8 * 1024);
    let lines = std::fs::read(a.join(CHAIN_FILE)).unwrap();
    let last = lines[..lines.len() - 1]
        .rsplit(|byte| *byte == b'\n')
        .next()
        .unwrap();
    let terminal: Envelope = serde_json::from_slice(last).unwrap();
    assert!(
        matches!(
            terminal.body,
            EventBody::Capture {
                record: CaptureRecord::LocalFinish { .. }
            }
        ),
        "the terminal record was chained inside the reservation"
    );
    drop(export_a);

    // Entry B, independently: A's spending and the competing writer left B's reservation whole.
    let mut part = 0;
    while export_b.capture_storage().used_bytes + 16 * 1024 < quota_b - TERMINAL_RESERVE_BYTES {
        export_b.append_chain(&mut segment(part, 8 * 1024)).unwrap();
        part += 1;
    }
    export_b.append_chain(&mut closure_gap(1)).unwrap();
    export_b.sync_capture().unwrap();
    assert_eq!(physical(&b).allocated, reserved[1].allocated);
    // Entry C (reserved by the killed child) still appends and syncs.
    export_c.append_chain(&mut segment(9, 64 * 1024)).unwrap();
    export_c.sync_capture().unwrap();
    assert_eq!(physical(&c).allocated, reserved[2].allocated);

    // Growing B on the exhausted volume: above the reported free space it is refused before any
    // allocation; within it, the kernel decides, and the claim follows the remeasurement.
    let storage_b = export_b.capture_storage();
    let above = reserved[1].allocated + volume.available() + MIB;
    let refused = export_b.reserve_capture(above);
    eprintln!("T03R entry-b reserve_capture({above}) on the exhausted volume -> {refused:?}");
    assert!(
        matches!(
            refused,
            Err(ExportError::Capability(TraceCapError::Refused {
                reason: RefusalKind::InsufficientSpace,
                ..
            }))
        ),
        "{refused:?}"
    );
    assert_eq!(export_b.capture_storage(), storage_b, "nothing claimed");
    assert_eq!(physical(&b).allocated, reserved[1].allocated);
    let within = reserved[1].allocated + volume.available().max(4096);
    let attempt = export_b.reserve_capture(within);
    let measured = physical(&b);
    eprintln!(
        "T03R entry-b reserve_capture({within}) (within reported free space) -> {attempt:?}; {measured:?}"
    );
    match attempt {
        Ok(()) => {
            assert!(measured.allocated >= within);
            assert_eq!(export_b.capture_storage().reserved_bytes, within);
        }
        Err(_) => {
            assert_eq!(export_b.capture_storage(), storage_b, "nothing claimed");
            assert_eq!(measured.allocated, reserved[1].allocated, "all or nothing");
        }
    }

    // Metadata needs headroom the chain reservation does not provide: with some returned, B's
    // local finish writes its seal.
    let freed = volume.mount.join("competing-0.bin");
    let freed_bytes = std::fs::metadata(&freed).unwrap().len();
    std::fs::remove_file(&freed).unwrap();
    eprintln!(
        "T03R headroom returned before entry-b's local finish: deleted competing-0.bin, \
         {freed_bytes} bytes; {} bytes reported available",
        volume.available()
    );
    export_b
        .write_local_finish(&manifest(), at(999), "participant_stop", stopped())
        .unwrap();
    assert!(b.join(SEAL_FILE).exists());
    drop(export_b);
    drop(export_c);

    volume.remount();
    for root in [&a, &b, &c] {
        assert_chain_is_whole_jsonl(root);
        eprintln!("T03R final {}: {:?}", root.display(), physical(root));
    }
    assert!(TraceExport::resume(&b).is_ok());
    assert!(TraceExport::resume(&c).is_ok());
}

/// Whether `path` is on ext4 (`0xEF53`) or XFS (`0x58465342`), by a path-based `statfs(2)`.
#[cfg(target_os = "linux")]
fn linux_ext4_or_xfs(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let encoded = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    let mut volume = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: the path is NUL terminated and the output buffer is writable.
    assert_eq!(
        unsafe { libc::statfs(encoded.as_ptr(), volume.as_mut_ptr()) },
        0
    );
    // SAFETY: statfs reported success, so it fully initialised the output buffer.
    #[allow(clippy::unnecessary_cast)]
    let magic = unsafe { volume.assume_init() }.f_type as u64;
    matches!(magic, 0xEF53 | 0x5846_5342)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn linux_ext4_or_xfs(_: &Path) -> bool {
    false
}
