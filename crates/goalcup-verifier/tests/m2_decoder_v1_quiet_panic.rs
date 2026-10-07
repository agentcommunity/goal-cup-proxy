//! A panic inside a decoder prints nothing (contract S7, review N4 and S7): the default hook
//! would write the payload, which could carry record text, to stderr. This test owns its
//! process, because it replaces the global panic hook, and walks the documented order of
//! events: a host hook installed before the first session is wrapped and stays quiet; a host
//! hook installed after it replaces the wrapper, so decoder panics reach that hook; and
//! `quiet_panic_hook` wraps the host's hook again.

use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, StreamKey,
};
use goalcup_verifier::capture_analysis::api::{
    DecoderFailure, DecoderSummary, DecoderV1, Disposition, Observation, StreamOpen,
};
use goalcup_verifier::capture_analysis::registry::{DecoderRegistry, Registered};
use goalcup_verifier::capture_analysis::session::{decode_records, quiet_panic_hook};
use std::sync::Mutex;

static PRINTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Boom;

impl DecoderV1 for Boom {
    fn id(&self) -> &'static str {
        "boom"
    }
    fn version(&self) -> &'static str {
        "1"
    }
    fn accepts(&self, _: &StreamOpen<'_>) -> bool {
        true
    }
    fn push(
        &mut self,
        _: &CaptureRecord,
        _: &mut dyn FnMut(Observation),
    ) -> Result<(), DecoderFailure> {
        panic!("SECRET-RECORD-TEXT");
    }
    fn finish(
        self: Box<Self>,
        _: &mut dyn FnMut(Observation),
    ) -> Result<DecoderSummary, DecoderFailure> {
        unreachable!("a failed decoder is not finished")
    }
}

fn host_hook() {
    std::panic::set_hook(Box::new(|info| {
        PRINTED.lock().unwrap().push(info.to_string());
    }));
}

fn printed() -> Vec<String> {
    std::mem::take(&mut *PRINTED.lock().unwrap())
}

/// One session whose only decoder panics on its only record.
fn decode_a_panic() {
    let mut registry = DecoderRegistry::empty();
    registry
        .register(Registered::new(|| Box::new(Boom), "unit", &[]))
        .unwrap();
    let open = CaptureRecord::Open {
        stream: StreamKey {
            producer: "0123456789abcdef0123456789abcdef".to_string(),
            stream: 1,
        },
        observed: CaptureTime {
            wall: "2026-10-05T12:00:00.000000001Z".to_string(),
            mono_ns: 1,
            clock_id: "clock".to_string(),
        },
        source: CaptureSource::Native,
        metadata: CaptureMeta {
            wire: None,
            label: "unit".to_string(),
            protocol_hint: None,
            method: None,
            host: None,
            path: None,
            status: None,
            content_type: None,
        },
    };
    let (outcome, _) = decode_records([Ok(open)], &registry, |_: &'static str, _: Observation| {
        Ok(Disposition::Kept)
    })
    .unwrap();
    let failed = outcome.decoders[0].result.as_ref().unwrap_err();
    assert_eq!(failed.failure.reason, "panicked");
}

fn host_panic() {
    let _ = std::panic::catch_unwind(|| panic!("host panic"));
}

#[test]
fn decoder_panics_are_quiet_until_a_host_hook_replaces_the_wrapper() {
    // A: the host's hook comes first; the session wraps it.
    host_hook();
    decode_a_panic();
    assert!(printed().is_empty(), "a decoder panic reached the hook");
    host_panic();
    let seen = printed();
    assert_eq!(
        seen.len(),
        1,
        "a panic outside a decoder still reaches the host hook"
    );
    assert!(seen[0].contains("host panic"));

    // B: a hook installed after the first session replaces the wrapper. Decoder panics now
    // reach it, payload included: the documented limit of the automatic install.
    host_hook();
    decode_a_panic();
    let seen = printed();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].contains("SECRET-RECORD-TEXT"));

    // C: the host wraps its hook with `quiet_panic_hook`; decoder panics are silent again and
    // every other panic still reaches the host.
    std::panic::set_hook(quiet_panic_hook(std::panic::take_hook()));
    decode_a_panic();
    assert!(printed().is_empty());
    host_panic();
    assert_eq!(printed().len(), 1);
}
