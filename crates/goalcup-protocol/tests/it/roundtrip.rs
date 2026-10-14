use ed25519_dalek::SigningKey;
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::capture::{CaptureRecord, CaptureTime, GapReason, M2_PROTOCOL_VERSION};
use goalcup_protocol::chain::{GENESIS, Link, link_hash};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::events::HealthState;
use goalcup_protocol::provenance::{
    CaptureUnavailableV1, CutoffReachedV1, FinalizationV1, NoRepositoryReasonV1,
    PreCutoffDrainOutcomeV1, PublisherNotAttemptedReasonV1, PublisherNotAttemptedV1,
    RepositoryNonSelectionV1, Rfc3339Utc, Sha256Digest, SnapshotNotAttemptedV1,
    TerminationOutcomeV1,
};
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
use goalcup_protocol::seal::{SealManifest, SealedFile};

/// A v5 baseline body: the smallest body a v5 chain carries.
fn baseline_body() -> EventBody {
    EventBody::Baseline {
        manifest_sha256: "aa".repeat(32),
        file_count: 1,
    }
}

fn claims() -> StartReceiptClaims {
    StartReceiptClaims {
        entry_id: "entry-0001".into(),
        connector_id: "claude-code-cli".into(),
        workspace: "/Users/team/goalcup-spikes/ws-claude-code-cli-20260903T073054Z".into(),
        session_id: "95addb64-9dac-4046-8ef6-8aa53fa0c7f1".into(),
        started_at: "2026-09-03T07:42:00.446Z".into(),
        deadline_at: "2026-09-10T07:42:00.446Z".into(),
        manifest_sha256: "0f7aa0d17c0895d764b2a8a5f9beee2b84075c867639679d57555411043a1761".into(),
        chain_head: "4e8c478cc81cc43c4fcd273ffd454b5383ed3963c2a07bdf238a33d2afd6d6ca".into(),
        connector_dir_sha256: Some(
            "51ae55f6faee49f8267d6cd0c1f9c105913d1ee1d89c298337cf7fcd847f299e".into(),
        ),
        run_public_key: None,
        questionnaire_version: None,
    }
}

#[test]
fn envelope_round_trips_every_body() {
    let bodies = vec![
        baseline_body(),
        EventBody::Capture {
            record: CaptureRecord::Gap {
                stream: None,
                observed: CaptureTime {
                    wall: "2026-09-03T07:42:00.250Z".into(),
                    mono_ns: 9000,
                    clock_id: "daemon-5b0c3a1e".into(),
                },
                reason: GapReason::ProducerLost,
                lost_bytes: None,
            },
        },
        EventBody::Seal {
            manifest_sha256: "bb".repeat(32),
            sealed_at: "2026-09-10T07:42:00Z".into(),
            file_count: 1,
        },
        EventBody::CutoffReachedV1(CutoffReachedV1 {
            receipt_sha256: Sha256Digest::try_from("aa".repeat(32)).unwrap(),
            deadline_at: Rfc3339Utc::try_from("2026-09-10T07:42:00Z").unwrap(),
            pre_cutoff_drain: PreCutoffDrainOutcomeV1::Complete,
        }),
        EventBody::FinalizationV1(FinalizationV1::CaptureUnavailable {
            receipt_sha256: Sha256Digest::try_from("aa".repeat(32)).unwrap(),
            termination: TerminationOutcomeV1::AlreadyStopped {
                checked_at: Rfc3339Utc::try_from("2026-09-10T07:42:01Z").unwrap(),
            },
            capture: CaptureUnavailableV1::NotAttempted(SnapshotNotAttemptedV1 {
                selection: RepositoryNonSelectionV1::None {
                    reason: NoRepositoryReasonV1::NotFound,
                },
            }),
            publisher: PublisherNotAttemptedV1 {
                reason: PublisherNotAttemptedReasonV1::SelectionFailed,
            },
        }),
    ];
    assert_eq!(bodies.len(), 5, "EventBody is closed at the five v5 bodies");

    let mut prev = GENESIS.to_string();
    for (seq, body) in bodies.into_iter().enumerate() {
        let mut env = Envelope {
            protocol_version: M2_PROTOCOL_VERSION,
            entry_id: "entry-0001".into(),
            source: "daemon".into(),
            recorded_at: "2026-09-03T07:42:00.673Z".into(),
            link: Link {
                seq: seq as u64,
                prev: prev.clone(),
                hash: String::new(),
            },
            body,
            redaction: None,
        };
        env.link.hash = env.expected_hash(&prev).unwrap();
        prev = env.link.hash.clone();
        let text = serde_json::to_string(&env).unwrap();
        let back: Envelope = serde_json::from_str(&text).unwrap();
        assert_eq!(env, back);
        // The chain fields are flattened to the top level, as the stub tailer wrote them.
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(v.get("seq").is_some(), "seq must be top level, got {text}");
        assert!(v.get("prev").is_some());
        assert!(v.get("hash").is_some());
    }
}

#[test]
fn start_receipt_signs_and_verifies() {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let pinned = hex::encode(key.verifying_key().to_bytes());
    let receipt = StartReceipt::sign(claims(), &key).unwrap();
    assert_eq!(
        receipt.signature,
        "81b7f4c11e8bda180a7c756e96cbfb4f2199528d0578bbe589d5035b0efb293b87e77c464a8319d8f61391ab7e8f2f2336598c9f5bea6fd8ef957e9d0b19e80e",
        "the envelope-version migration does not move legacy receipt signing bytes"
    );
    receipt
        .verify_with(&pinned)
        .expect("freshly signed receipt verifies");

    let text = serde_json::to_string(&receipt).unwrap();
    let back: StartReceipt = serde_json::from_str(&text).unwrap();
    assert_eq!(receipt, back);
    back.verify_with(&pinned)
        .expect("round-tripped receipt verifies");
}

#[test]
fn start_receipt_rejects_a_tampered_claim() {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let pinned = hex::encode(key.verifying_key().to_bytes());
    let mut receipt = StartReceipt::sign(claims(), &key).unwrap();
    receipt.claims.started_at = "2026-09-02T00:00:00.000Z".into();
    assert!(
        receipt.verify_with(&pinned).is_err(),
        "a rewritten started_at must not verify"
    );
}

#[test]
fn claims_signing_bytes_are_stable() {
    let a = claims().signing_bytes().unwrap();
    let b = claims().signing_bytes().unwrap();
    assert_eq!(a, b);
    let json = &a[goalcup_protocol::receipt::RECEIPT_DOMAIN.len()..];
    assert!(
        String::from_utf8(json.to_vec())
            .unwrap()
            .starts_with(r#"{"entry_id":"entry-0001""#)
    );
}

#[test]
fn receipt_claims_preserve_the_additive_run_key_and_questionnaire_in_signing_bytes() {
    let raw = r#"{"entry_id":"e1","connector_id":"claude-code-cli","workspace":"/ws","session_id":"s1","started_at":"2026-09-04T00:00:00Z","deadline_at":"2026-09-11T00:00:00Z","manifest_sha256":"manifest","chain_head":"head","connector_dir_sha256":"connector","run_public_key":"run-key","questionnaire_version":"2026-09-10-v1"}"#;
    let claims: StartReceiptClaims = serde_json::from_str(raw).unwrap();
    let signed = claims.signing_bytes().unwrap();
    assert_eq!(
        &signed[goalcup_protocol::receipt::RECEIPT_DOMAIN.len()..],
        raw.as_bytes(),
        "both appended bindings must survive parsing and remain in their canonical field order"
    );
}

#[test]
fn start_receipt_claims_reject_unknown_fields_before_signing() {
    let mut value = serde_json::to_value(claims()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("unexpected".to_string(), serde_json::json!(true));
    assert!(serde_json::from_value::<StartReceiptClaims>(value).is_err());
}

#[test]
fn complete_start_receipts_reject_unknown_outer_fields_before_digesting() {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let receipt = StartReceipt::sign(claims(), &key).unwrap();
    let mut value = serde_json::to_value(receipt).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("unexpected".to_string(), serde_json::json!(true));
    assert!(serde_json::from_value::<StartReceipt>(value).is_err());
}

#[test]
fn receipt_signatures_pin_the_historical_current_and_new_field_layouts() {
    let server = SigningKey::from_bytes(&[7u8; 32]);

    let mut historical = claims();
    historical.connector_dir_sha256 = None;
    let historical_receipt = StartReceipt::sign(historical.clone(), &server).unwrap();
    assert_eq!(
        historical_receipt.signature,
        "ab682fe8e62d6a4d9eb34bd0ca541989dea3213fafd78b632aeca5f5c887a6f4f3ac781585344e610ef77bf9e8331632f2295654497aa1eec73d030e97f8150b"
    );
    let historical_json = String::from_utf8(
        historical.signing_bytes().unwrap()[goalcup_protocol::receipt::RECEIPT_DOMAIN.len()..]
            .to_vec(),
    )
    .unwrap();
    assert!(!historical_json.contains("connector_dir_sha256"));
    assert!(!historical_json.contains("run_public_key"));
    assert!(!historical_json.contains("questionnaire_version"));

    let current = StartReceipt::sign(claims(), &server).unwrap();
    assert_eq!(
        current.signature,
        "81b7f4c11e8bda180a7c756e96cbfb4f2199528d0578bbe589d5035b0efb293b87e77c464a8319d8f61391ab7e8f2f2336598c9f5bea6fd8ef957e9d0b19e80e"
    );

    let run_key = SigningKey::from_bytes(&[9u8; 32]);
    let mut bound = claims();
    bound.run_public_key = Some(hex::encode(run_key.verifying_key().to_bytes()));
    bound.questionnaire_version = Some("2026-09-10-v1".to_string());
    let bound_receipt = StartReceipt::sign(bound.clone(), &server).unwrap();
    assert_eq!(
        bound_receipt.signature,
        "c6311684d7f5fb9c0fe286f977d2a131ba601584a12efd0fbe66e5d704ee074b33ef1d698da1edc80614b6cbb0e71ac3b6008a948e1b7564a0970abad0cdd203"
    );
    let bound_json = String::from_utf8(
        bound.signing_bytes().unwrap()[goalcup_protocol::receipt::RECEIPT_DOMAIN.len()..].to_vec(),
    )
    .unwrap();
    assert!(
        bound_json.ends_with(r#""questionnaire_version":"2026-09-10-v1"}"#),
        "questionnaire remains the last canonical field: {bound_json}"
    );

    let digest = bound_receipt.canonical_digest().unwrap();
    let mut edited_complete_receipt = bound_receipt;
    edited_complete_receipt.signature.replace_range(0..2, "00");
    assert_ne!(
        edited_complete_receipt.canonical_digest().unwrap(),
        digest,
        "the digest covers the complete signed receipt, not only its claims"
    );
}

#[test]
fn every_signed_document_carries_its_own_domain_tag() {
    // Three signed documents share one keypair shape and were separated only by their
    // JSON structure. A distinct, version-tagged prefix per document is what stops a
    // receipt's bytes being presented as an acknowledgement's.
    use goalcup_protocol::ack::ACK_DOMAIN;
    use goalcup_protocol::chain::CHAIN_DOMAIN;
    use goalcup_protocol::control::STATUS_DOMAIN;
    use goalcup_protocol::receipt::{RECEIPT_DOMAIN, START_RECEIPT_DIGEST_DOMAIN};

    let domains = [
        CHAIN_DOMAIN,
        RECEIPT_DOMAIN,
        ACK_DOMAIN,
        STATUS_DOMAIN,
        START_RECEIPT_DIGEST_DOMAIN,
    ];
    for (i, a) in domains.iter().enumerate() {
        for b in &domains[i + 1..] {
            assert_ne!(a, b, "two documents share a domain tag");
            // Neither may be a prefix of the other, or the tag stops separating them.
            assert!(!a.starts_with(b) && !b.starts_with(a));
        }
        assert!(
            a.ends_with(b"\0"),
            "a domain tag ends with its NUL terminator"
        );
    }
}

#[test]
fn a_receipt_verifies_only_against_the_key_the_caller_pins() {
    use ed25519_dalek::SigningKey;
    use goalcup_protocol::receipt::{ReceiptError, StartReceipt, StartReceiptClaims};

    let server = SigningKey::from_bytes(&[0x42; 32]);
    let impostor = SigningKey::from_bytes(&[6u8; 32]);
    let claims = StartReceiptClaims {
        entry_id: "e1".to_string(),
        connector_id: "claude-code-cli".to_string(),
        workspace: "/ws".to_string(),
        session_id: "s1".to_string(),
        started_at: "2026-09-04T00:00:00Z".to_string(),
        deadline_at: "2026-09-11T00:00:00Z".to_string(),
        manifest_sha256: "ab".repeat(32),
        chain_head: "cd".repeat(32),
        // Absent, which is the layout this method produced before the claim existed: the
        // key-mismatch path below is about who signed, not about what was signed.
        connector_dir_sha256: None,
        run_public_key: None,
        questionnaire_version: None,
    };
    let pinned = hex::encode(server.verifying_key().to_bytes());

    let good = StartReceipt::sign(claims.clone(), &server).unwrap();
    assert_eq!(good.verify_with(&pinned), Ok(()));

    // The whole point: a receipt the participant signed themselves names its own key,
    // and is refused before any cryptography runs.
    let forged = StartReceipt::sign(claims.clone(), &impostor).unwrap();
    match forged.verify_with(&pinned) {
        Err(ReceiptError::KeyMismatch { found, pinned: p }) => {
            assert_eq!(found, hex::encode(impostor.verifying_key().to_bytes()));
            assert_eq!(p, pinned);
        }
        other => panic!("expected KeyMismatch, got {other:?}"),
    }

    // A genuine key with a tampered claim.
    let mut tampered = good.clone();
    tampered.claims.deadline_at = "2026-12-31T00:00:00Z".to_string();
    assert_eq!(
        tampered.verify_with(&pinned),
        Err(ReceiptError::SignatureInvalid)
    );

    // Domain separation: the receipt's bytes must not be the raw claims JSON, or an ack
    // signer and a receipt signer become interchangeable.
    let bytes = claims.signing_bytes().unwrap();
    assert!(bytes.starts_with(goalcup_protocol::receipt::RECEIPT_DOMAIN));
    assert_ne!(bytes, serde_json::to_vec(&claims).unwrap());
}

#[test]
fn acked_at_is_inside_the_ack_signature() {
    use ed25519_dalek::Signer;
    use ed25519_dalek::SigningKey;
    use goalcup_protocol::ack::{ACK_DOMAIN, AckCheckpoint};

    let server = SigningKey::from_bytes(&[0x42; 32]);
    let pinned = hex::encode(server.verifying_key().to_bytes());
    let mut ack = AckCheckpoint {
        entry_id: "e1".to_string(),
        seq: 42,
        hash: "ab".repeat(32),
        acked_at: "2026-09-04T00:00:00Z".to_string(),
        server_signature: String::new(),
    };
    ack.server_signature = hex::encode(server.sign(&ack.signing_bytes().unwrap()).to_bytes());
    assert!(ack.verify(&pinned).is_ok());
    assert!(ack.signing_bytes().unwrap().starts_with(ACK_DOMAIN));
    assert_eq!(ack.acked_at_secs(), Some(1_788_480_000));

    // Move the clock and the signature must fail: this is the offline-until-deadline
    // attack, which would otherwise leave no trace at all.
    let backdated = AckCheckpoint {
        acked_at: "2026-09-01T00:00:00Z".to_string(),
        ..ack.clone()
    };
    assert!(
        backdated.verify(&pinned).is_err(),
        "acked_at outside the signature is a free-form field the participant can rewrite"
    );
}

#[test]
fn ack_and_seal_round_trip() {
    let ack = AckCheckpoint {
        entry_id: "entry-0001".into(),
        seq: 206,
        hash: "cd69fd57bba70605571379c4bfb0f48b085373f0878cef8e44204e727ea0dabc".into(),
        acked_at: "2026-09-03T07:51:11Z".into(),
        server_signature: "00".repeat(64),
    };
    assert_eq!(
        ack,
        serde_json::from_str(&serde_json::to_string(&ack).unwrap()).unwrap()
    );

    let seal = SealManifest {
        entry_id: "entry-0001".into(),
        sealed_at: "2026-09-10T07:42:00Z".into(),
        workspace: "/tmp/ws".into(),
        files: vec![
            SealedFile {
                path: "hello.py".into(),
                bytes: 4,
                sha256: "aa".repeat(32),
            },
            SealedFile {
                path: "README.md".into(),
                bytes: 12,
                sha256: "bb".repeat(32),
            },
        ],
        excluded: vec![".claude/settings.json".into()],
        git: None,
        home: None,
    }
    .sorted();
    assert_eq!(
        seal.files[0].path, "README.md",
        "seal files must sort by path"
    );
    assert_eq!(
        seal,
        serde_json::from_str(&serde_json::to_string(&seal).unwrap()).unwrap()
    );
}

#[test]
fn health_state_carries_the_fifteen_r14_states_including_the_two_new_ones() {
    let all = [
        HealthState::Online,
        HealthState::UploadLagging,
        HealthState::ConnectionLost,
        HealthState::AgentExited,
        HealthState::TrackerInterrupted,
        HealthState::CaptureStalled,
        HealthState::HooksSilent,
        HealthState::ConfigRestored,
        HealthState::DeadlineReached,
        HealthState::ArmingPending,
        HealthState::ArmingLost,
        HealthState::VersionChanged,
        HealthState::ConfigMismatch,
        HealthState::StoreReplaced,
        HealthState::DiskPressure,
    ];
    assert_eq!(all.len(), 15);
    assert_eq!(
        serde_json::to_string(&HealthState::StoreReplaced).unwrap(),
        "\"store_replaced\""
    );
    assert_eq!(
        serde_json::to_string(&HealthState::DiskPressure).unwrap(),
        "\"disk_pressure\""
    );
}

#[test]
fn an_acknowledgement_verifies_and_rejects_a_rewritten_hash() {
    // The acknowledgement is the one integrity anchor the participant does not hold.
    let key = SigningKey::from_bytes(&[11u8; 32]);
    let mut ack = AckCheckpoint {
        entry_id: "entry-0001".into(),
        seq: 206,
        hash: "cd69fd57bba70605571379c4bfb0f48b085373f0878cef8e44204e727ea0dabc".into(),
        acked_at: "2026-09-03T07:51:11Z".into(),
        server_signature: String::new(),
    };
    // The server signs `signing_bytes`; the test plays the server.
    let bytes = ack.signing_bytes().unwrap();
    let json =
        String::from_utf8(bytes[goalcup_protocol::ack::ACK_DOMAIN.len()..].to_vec()).unwrap();
    assert!(
        json.starts_with(r#"{"entry_id":"entry-0001","seq":206,"hash":"#),
        "signing bytes are {{entry_id, seq, hash, acked_at}} in that order, got {json}"
    );
    assert!(
        json.ends_with(r#","acked_at":"2026-09-03T07:51:11Z"}"#),
        "acked_at is inside the signature, got {json}"
    );
    ack.server_signature = hex::encode(ed25519_dalek::Signer::sign(&key, &bytes).to_bytes());
    let pk = hex::encode(key.verifying_key().to_bytes());
    ack.verify(&pk)
        .expect("a freshly signed acknowledgement verifies");

    let mut tampered = ack.clone();
    tampered.hash = "00".repeat(32);
    assert!(
        tampered.verify(&pk).is_err(),
        "an acknowledgement whose hash was rewritten must not verify"
    );

    let other = SigningKey::from_bytes(&[12u8; 32]);
    assert!(
        ack.verify(&hex::encode(other.verifying_key().to_bytes()))
            .is_err(),
        "a different server key must not verify"
    );
}

#[test]
fn chain_bytes_are_domain_tagged_canonical_json_over_the_six_fields() {
    use goalcup_protocol::chain::CHAIN_DOMAIN;

    let env = Envelope {
        protocol_version: M2_PROTOCOL_VERSION,
        entry_id: "e1".to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-04T00:00:00Z".to_string(),
        link: Link {
            seq: 0,
            prev: GENESIS.to_string(),
            hash: String::new(),
        },
        body: baseline_body(),
        redaction: None,
    };

    let mut want: Vec<u8> = CHAIN_DOMAIN.to_vec();
    want.extend_from_slice(
        format!(
            r#"{{"protocol_version":5,"entry_id":"e1","source":"daemon","recorded_at":"2026-09-04T00:00:00Z","seq":0,"body":{{"kind":"baseline","content":{{"manifest_sha256":"{}","file_count":1}}}}}}"#,
            "aa".repeat(32)
        )
        .as_bytes(),
    );
    assert_eq!(env.chain_bytes().unwrap(), want);

    // `prev` and `hash` are the link, not the content: changing them must not change
    // the bytes, or a chain could never be re-linked after a documented scrub.
    let mut moved = env.clone();
    moved.link.prev = "ff".repeat(32);
    moved.link.hash = "ee".repeat(32);
    assert_eq!(moved.chain_bytes().unwrap(), env.chain_bytes().unwrap());

    // And `expected_hash` is exactly `link_hash` over those bytes.
    assert_eq!(
        env.expected_hash(GENESIS).unwrap(),
        link_hash(GENESIS, &env.chain_bytes().unwrap())
    );

    // The domain tag is load-bearing: the same JSON without it hashes differently.
    let undomained = link_hash(GENESIS, &env.chain_bytes().unwrap()[CHAIN_DOMAIN.len()..]);
    assert_ne!(env.expected_hash(GENESIS).unwrap(), undomained);
}

/// A v5 envelope never carries envelope-level redaction (the export refuses one), but the
/// field is typed and inside the hash: a line that gains or loses it no longer links.
#[test]
fn an_envelope_redaction_field_is_typed_and_hash_covered() {
    let wire = serde_json::json!({
        "protocol_version": 5,
        "entry_id": "e1",
        "source": "daemon",
        "recorded_at": "2026-09-04T00:00:00Z",
        "seq": 0,
        "prev": GENESIS,
        "hash": "",
        "body": {
            "kind": "baseline",
            "content": {"manifest_sha256": "aa".repeat(32), "file_count": 1}
        },
        "redaction": {
            "spans": [{"kind": "anthropic_key", "start": 0, "end": 38}]
        }
    });
    let env: Envelope = serde_json::from_value(wire.clone()).unwrap();
    let round_trip = serde_json::to_value(&env).unwrap();
    assert_eq!(
        round_trip["redaction"], wire["redaction"],
        "the envelope audit is typed data, not an ignored extension"
    );

    let mut without_audit = round_trip;
    without_audit.as_object_mut().unwrap().remove("redaction");
    let without_audit: Envelope = serde_json::from_value(without_audit).unwrap();
    assert_ne!(
        env.expected_hash(GENESIS).unwrap(),
        without_audit.expected_hash(GENESIS).unwrap(),
        "changing the envelope audit changes the link"
    );

    // Keep the link construction in this test honest: neither parser supplied a hash.
    assert_eq!(
        env.link,
        Link {
            seq: 0,
            prev: GENESIS.into(),
            hash: String::new()
        }
    );
}

#[test]
fn redaction_audit_rejects_unknown_fields_and_untyped_offsets() {
    use goalcup_protocol::redaction::RedactionAudit;

    assert!(
        serde_json::from_value::<RedactionAudit>(serde_json::json!({
            "spans": [{"kind": "email", "start": 0, "end": 1, "digest": "oracle"}]
        }))
        .is_err(),
        "legacy or invented span fields are not ignored"
    );
    assert!(
        serde_json::from_value::<RedactionAudit>(serde_json::json!({
            "spans": [{"kind": "email", "start": "0", "end": 1}]
        }))
        .is_err(),
        "offsets are typed integers"
    );
}

#[test]
fn recorded_at_is_rfc3339_and_parses_to_seconds() {
    let env = Envelope {
        protocol_version: M2_PROTOCOL_VERSION,
        entry_id: "e1".to_string(),
        source: "daemon".to_string(),
        recorded_at: "2026-09-04T12:00:00Z".to_string(),
        link: Link {
            seq: 7,
            prev: GENESIS.to_string(),
            hash: "aa".repeat(32),
        },
        body: baseline_body(),
        redaction: None,
    };
    assert_eq!(env.recorded_at_secs(), Some(1_788_523_200));

    let text = serde_json::to_string(&env).unwrap();
    assert!(text.contains(r#""recorded_at":"2026-09-04T12:00:00Z""#));
    let back: Envelope = serde_json::from_str(&text).unwrap();
    assert_eq!(
        back, env,
        "Envelope round-trips and is now PartialEq on a string clock"
    );

    let broken = Envelope {
        recorded_at: "not a time".to_string(),
        ..env
    };
    assert_eq!(
        broken.recorded_at_secs(),
        None,
        "a bad clock is None, never 0"
    );
}

#[test]
fn is_excluded_is_anchored_at_the_workspace_root() {
    use goalcup_protocol::seal::is_excluded;
    let p = |s: &str| vec![s.to_string()];

    // Exact and prefix.
    assert!(is_excluded(".git", &p(".git")));
    assert!(is_excluded(".git/HEAD", &p(".git")));
    assert!(is_excluded(
        ".opencode/node_modules/x/y.js",
        &p(".opencode/node_modules")
    ));

    // The attack this closes: `mkdir -p src/.git && cp -r prewritten/* src/.git/` would be
    // excluded from the seal AND from attribution by an any-component clause.
    assert!(
        !is_excluded("src/.git/HEAD", &p(".git")),
        "a nested .git is the agent's work until a pattern says otherwise"
    );
    assert!(!is_excluded("a/b/__pycache__/x.pyc", &p("__pycache__")));

    // Any-depth is available, but only when the manifest asks for it.
    assert!(is_excluded("a/b/__pycache__/x.pyc", &p("**/__pycache__")));
    assert!(is_excluded("__pycache__/x.pyc", &p("**/__pycache__")));

    // And still never a substring test.
    assert!(!is_excluded("targeted.py", &p("target")));
    assert!(!is_excluded("", &p("")));
}

// ---------------------------------------------------------------------------------------
// The chain as rotating parts (D55). Every fixture is a real `TraceExport` in a temporary
// directory; every assertion reads the part files back off disk.
// ---------------------------------------------------------------------------------------

mod parts {
    use goalcup_protocol::capture::{
        CaptureRecord, CaptureTime, GapReason, M2_PROTOCOL_VERSION, TERMINAL_RESERVE_BYTES,
    };
    use goalcup_protocol::chain::Link;
    use goalcup_protocol::envelope::{Envelope, EventBody};
    use goalcup_protocol::export::{
        CHAIN_FILE, ExportError, PART_MAX_AGE, PART_MAX_BYTES, ResumeSeam, TraceExport,
        chain_parts, parse_part_name, part_file_name, set_resume_seam_for_test,
    };
    use goalcup_protocol::provenance::{Rfc3339Utc, TerminationMethodV1, TerminationOutcomeV1};
    use goalcup_protocol::seal::SealManifest;
    use std::path::Path;
    use std::time::{Duration, Instant};

    const ENTRY: &str = "entry-parts-0001";
    const QUOTA: u64 = TERMINAL_RESERVE_BYTES * 64;
    /// Small enough that every flushed record or two closes a part.
    const SMALL_PART: u64 = 600;
    const LONG_AGE: Duration = Duration::from_secs(3600);

    fn manifest() -> SealManifest {
        SealManifest {
            entry_id: ENTRY.to_string(),
            sealed_at: "2026-10-14T12:00:00Z".to_string(),
            workspace: "/ws".to_string(),
            files: Vec::new(),
            excluded: Vec::new(),
            git: None,
            home: None,
        }
    }

    fn at(mono_ns: u64) -> CaptureTime {
        CaptureTime {
            wall: "2026-10-14T12:00:01Z".to_string(),
            mono_ns,
            clock_id: "daemon-parts".to_string(),
        }
    }

    fn gap(mono_ns: u64) -> Envelope {
        Envelope {
            protocol_version: M2_PROTOCOL_VERSION,
            entry_id: ENTRY.to_string(),
            source: "daemon".to_string(),
            recorded_at: "2026-10-14T12:00:01Z".to_string(),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body: EventBody::Capture {
                record: CaptureRecord::Gap {
                    stream: None,
                    observed: at(mono_ns),
                    reason: GapReason::StorageIo,
                    lost_bytes: None,
                },
            },
            redaction: None,
        }
    }

    fn fresh(root: &Path, part_bytes: Option<u64>) -> TraceExport {
        let mut export = TraceExport::create(root).unwrap();
        if let Some(bytes) = part_bytes {
            export.set_part_limits(bytes, LONG_AGE);
        }
        export.configure_capture_quota(QUOTA).unwrap();
        export
            .write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest())
            .unwrap();
        export
    }

    /// Append `count` gaps from `from`, flushing (and so offering a rotation) after each.
    fn write_gaps(export: &mut TraceExport, from: u64, count: u64) {
        for n in from..from + count {
            export.append_chain(&mut gap(n)).unwrap();
            export.sync_capture().unwrap();
            export.rotate_if_due(Instant::now()).unwrap();
        }
    }

    /// A chain of the baseline and `count` gaps, rotated into parts of `SMALL_PART` bytes.
    fn rotated(root: &Path, count: u64) -> TraceExport {
        let mut export = fresh(root, Some(SMALL_PART));
        export.sync_capture().unwrap();
        export.rotate_if_due(Instant::now()).unwrap();
        write_gaps(&mut export, 1, count);
        export
    }

    fn concatenated(root: &Path) -> Vec<u8> {
        chain_parts(root)
            .unwrap()
            .iter()
            .flat_map(|part| std::fs::read(&part.path).unwrap())
            .collect()
    }

    fn confirmed() -> TerminationOutcomeV1 {
        TerminationOutcomeV1::Confirmed {
            requested_at: Rfc3339Utc::try_from("2026-10-14T12:00:02Z").unwrap(),
            confirmed_at: Rfc3339Utc::try_from("2026-10-14T12:00:03Z").unwrap(),
            method: TerminationMethodV1::ProcessSignal,
        }
    }

    #[test]
    fn chain_parts_names_are_canonical() {
        assert_eq!(PART_MAX_BYTES, 256 << 20);
        assert_eq!(PART_MAX_AGE, Duration::from_secs(30 * 60));
        assert_eq!(part_file_name(0), CHAIN_FILE);
        assert_eq!(part_file_name(42), "chain.00000000000000000042.jsonl");
        assert_eq!(part_file_name(u64::MAX), "chain.18446744073709551615.jsonl");
        for start in [0, 1, 42, 256 << 20, u64::MAX] {
            assert_eq!(parse_part_name(&part_file_name(start)), Some(start));
        }
        for name in [
            "chain.00000000000000000000.jsonl",
            "chain.0000000000000000042.jsonl",
            "chain.000000000000000000042.jsonl",
            "chain.+0000000000000000042.jsonl",
            "chain.-0000000000000000042.jsonl",
            "chain.42.jsonl",
            "chain.18446744073709551616.jsonl",
            "chain.jsonl.tmp",
            "acks.jsonl",
            "chain..jsonl",
        ] {
            assert_eq!(parse_part_name(name), None, "{name}");
        }
    }

    #[test]
    fn a_chain_rotates_at_the_part_size_after_a_sync() {
        let dir = tempfile::tempdir().unwrap();
        let mut export = fresh(dir.path(), Some(SMALL_PART));
        // Past the size, but nothing is durable yet: no rotation.
        while export.capture_storage().used_bytes < SMALL_PART {
            export.append_chain(&mut gap(1)).unwrap();
        }
        assert!(!export.rotate_if_due(Instant::now()).unwrap());
        assert_eq!(chain_parts(dir.path()).unwrap().len(), 1);

        // At the durable frontier it rotates, and the new part starts where the first ends.
        export.sync_capture().unwrap();
        let end = export.capture_storage().used_bytes;
        assert!(export.rotate_if_due(Instant::now()).unwrap());
        let parts = chain_parts(dir.path()).unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|part| (part.start, part.len))
                .collect::<Vec<_>>(),
            vec![(0, end), (end, 0)]
        );
        assert_eq!(parts[1].path, dir.path().join(part_file_name(end)));
        // An empty part never rotates.
        export.sync_capture().unwrap();
        assert!(!export.rotate_if_due(Instant::now()).unwrap());

        // The next record lands in the new part; the first part is closed.
        let link = export.append_chain(&mut gap(2)).unwrap();
        let tail: Envelope = serde_json::from_slice(
            std::fs::read(&parts[1].path)
                .unwrap()
                .strip_suffix(b"\n")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(tail.link, link);
        assert_eq!(std::fs::metadata(&parts[0].path).unwrap().len(), end);

        // The age limit rotates a non-empty part too.
        export.set_part_limits(u64::MAX, Duration::ZERO);
        export.sync_capture().unwrap();
        assert!(export.rotate_if_due(Instant::now()).unwrap());
        assert_eq!(chain_parts(dir.path()).unwrap().len(), 3);

        // A closed chain never rotates.
        export.append_chain(&mut gap(3)).unwrap();
        export
            .write_local_finish(&manifest(), at(9), "participant_stop", confirmed())
            .unwrap();
        assert!(!export.rotate_if_due(Instant::now()).unwrap());
        assert_eq!(chain_parts(dir.path()).unwrap().len(), 3);

        // With the shipped limits a young, small part stays one file.
        let single = tempfile::tempdir().unwrap();
        let mut export = fresh(single.path(), None);
        write_gaps(&mut export, 1, 5);
        assert_eq!(chain_parts(single.path()).unwrap().len(), 1);
    }

    #[test]
    fn parts_concatenate_to_the_single_file_chain() {
        let single = tempfile::tempdir().unwrap();
        let mut export = fresh(single.path(), None);
        write_gaps(&mut export, 1, 12);
        drop(export);

        let split = tempfile::tempdir().unwrap();
        drop(rotated(split.path(), 12));

        let parts = chain_parts(split.path()).unwrap();
        assert!(parts.len() >= 4, "{parts:?}");
        assert_eq!(chain_parts(single.path()).unwrap().len(), 1);
        assert_eq!(
            concatenated(split.path()),
            std::fs::read(single.path().join(CHAIN_FILE)).unwrap()
        );
    }

    #[test]
    fn resume_reads_every_part_and_appends_to_the_last() {
        let dir = tempfile::tempdir().unwrap();
        let export = rotated(dir.path(), 10);
        let head = export.head().unwrap();
        let used = export.capture_storage().used_bytes;
        drop(export);
        let before = chain_parts(dir.path()).unwrap();
        assert!(before.len() >= 3, "{before:?}");

        let mut resumed = TraceExport::resume(dir.path()).unwrap();
        assert_eq!(resumed.head(), Some(head.clone()));
        assert_eq!(resumed.capture_storage().used_bytes, used);
        resumed.configure_capture_quota(QUOTA).unwrap();
        let link = resumed.append_chain(&mut gap(99)).unwrap();
        assert_eq!(
            (link.seq, link.prev.as_str()),
            (head.0 + 1, head.1.as_str())
        );
        resumed.sync_capture().unwrap();
        drop(resumed);

        // Only the last part grew.
        let after = chain_parts(dir.path()).unwrap();
        assert_eq!(after.len(), before.len());
        for (old, new) in before.iter().zip(&after).take(before.len() - 1) {
            assert_eq!((old.start, old.len), (new.start, new.len));
        }
        assert!(after.last().unwrap().len > before.last().unwrap().len);
        assert_eq!(
            TraceExport::resume(dir.path()).unwrap().head(),
            Some((link.seq, link.hash))
        );
    }

    #[test]
    fn resume_starts_from_the_first_surviving_part() {
        let dir = tempfile::tempdir().unwrap();
        drop(rotated(dir.path(), 12));
        let parts = chain_parts(dir.path()).unwrap();
        assert!(parts.len() >= 4, "{parts:?}");

        let full = TraceExport::resume(dir.path()).unwrap();
        let full_head = full.head().unwrap();
        let full_used = full.capture_storage().used_bytes;
        drop(full);

        // The two oldest parts were acknowledged and removed.
        std::fs::remove_file(&parts[0].path).unwrap();
        std::fs::remove_file(&parts[1].path).unwrap();
        assert!(matches!(
            TraceExport::create(dir.path()),
            Err(ExportError::ChainExists)
        ));

        let mut pruned = TraceExport::resume(dir.path()).unwrap();
        // The same head, so the same next seq, and logical offsets that did not move.
        assert_eq!(pruned.head(), Some(full_head.clone()));
        assert_eq!(pruned.capture_storage().used_bytes, full_used);
        pruned.configure_capture_quota(QUOTA).unwrap();
        // The pruned baseline still counts: there is never a second one.
        assert!(matches!(
            pruned.write_baseline_for_protocol(M2_PROTOCOL_VERSION, &manifest()),
            Err(ExportError::BaselineAlreadyChained)
        ));
        let link = pruned.append_chain(&mut gap(99)).unwrap();
        assert_eq!(link.seq, full_head.0 + 1);
        assert_eq!(link.prev, full_head.1);
        pruned
            .write_local_finish(&manifest(), at(100), "participant_stop", confirmed())
            .unwrap();
        let head = pruned.head().unwrap();
        drop(pruned);

        // What it appended verifies: a second resume re-validates every surviving link.
        let again = TraceExport::resume(dir.path()).unwrap();
        assert_eq!(again.head(), Some(head));
        let surviving = chain_parts(dir.path()).unwrap();
        assert_eq!(surviving[0].start, parts[2].start);
    }

    #[test]
    fn a_gap_between_parts_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        drop(rotated(dir.path(), 10));
        let parts = chain_parts(dir.path()).unwrap();
        assert!(parts.len() >= 3, "{parts:?}");
        std::fs::remove_file(&parts[1].path).unwrap();

        let error = chain_parts(dir.path()).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData, "{error}");
        assert!(matches!(
            TraceExport::resume(dir.path()),
            Err(ExportError::PartGap { .. })
        ));
    }

    /// A rotated chain whose last part is empty: the writer rotated and wrote nothing more.
    fn ending_in_an_empty_part(root: &Path) -> TraceExport {
        let mut export = rotated(root, 8);
        let mut n = 100;
        while chain_parts(root).unwrap().last().unwrap().len > 0 {
            export.append_chain(&mut gap(n)).unwrap();
            export.sync_capture().unwrap();
            export.rotate_if_due(Instant::now()).unwrap();
            n += 1;
        }
        export
    }

    #[test]
    fn a_part_deleted_during_resume_is_still_read() {
        let dir = tempfile::tempdir().unwrap();
        drop(rotated(dir.path(), 12));
        let parts = chain_parts(dir.path()).unwrap();
        assert!(parts.len() >= 4, "{parts:?}");
        let full = TraceExport::resume(dir.path()).unwrap().head();

        // The uploader removes two acknowledged parts while the resume holds them open.
        let doomed: Vec<_> = parts[..2].iter().map(|part| part.path.clone()).collect();
        set_resume_seam_for_test(ResumeSeam::Opened, move || {
            for path in doomed {
                std::fs::remove_file(path).unwrap();
            }
        });
        let resumed = TraceExport::resume(dir.path()).unwrap();
        assert_eq!(resumed.head(), full);
        assert!(!parts[0].path.exists() && !parts[1].path.exists());
        drop(resumed);
        // The next resume starts at the first surviving part, at the same head.
        assert_eq!(TraceExport::resume(dir.path()).unwrap().head(), full);
    }

    #[test]
    fn a_part_deleted_before_it_is_opened_is_listed_again() {
        let dir = tempfile::tempdir().unwrap();
        drop(rotated(dir.path(), 12));
        let parts = chain_parts(dir.path()).unwrap();
        let full = TraceExport::resume(dir.path()).unwrap().head();

        // Removed between the listing and the open: the second listing no longer names it.
        let doomed = parts[0].path.clone();
        set_resume_seam_for_test(ResumeSeam::Listed, move || {
            std::fs::remove_file(doomed).unwrap();
        });
        let resumed = TraceExport::resume(dir.path()).unwrap();
        assert_eq!(resumed.head(), full);
        assert_eq!(chain_parts(dir.path()).unwrap()[0].start, parts[1].start);

        // A middle part removed the same way is a hole, never skipped.
        drop(resumed);
        let parts = chain_parts(dir.path()).unwrap();
        let doomed = parts[1].path.clone();
        set_resume_seam_for_test(ResumeSeam::Listed, move || {
            std::fs::remove_file(doomed).unwrap();
        });
        assert!(matches!(
            TraceExport::resume(dir.path()),
            Err(ExportError::PartGap { .. })
        ));
    }

    #[test]
    fn resume_appends_to_an_empty_last_part() {
        for pruned in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let export = ending_in_an_empty_part(dir.path());
            let head = export.head().unwrap();
            drop(export);
            let parts = chain_parts(dir.path()).unwrap();
            assert!(parts.len() >= 3, "{parts:?}");
            if pruned {
                std::fs::remove_file(&parts[0].path).unwrap();
            }

            let mut resumed = TraceExport::resume(dir.path()).unwrap();
            assert_eq!(resumed.head(), Some(head.clone()), "pruned: {pruned}");
            resumed.configure_capture_quota(QUOTA).unwrap();
            let link = resumed.append_chain(&mut gap(999)).unwrap();
            assert_eq!(link.prev, head.1);
            drop(resumed);
            let after = chain_parts(dir.path()).unwrap();
            assert_eq!(after.len(), parts.len() - usize::from(pruned));
            assert!(
                after.last().unwrap().len > 0,
                "the append went to the empty part"
            );
        }
    }

    #[test]
    fn only_an_empty_surviving_part_is_a_pruned_head() {
        let dir = tempfile::tempdir().unwrap();
        drop(ending_in_an_empty_part(dir.path()));
        let parts = chain_parts(dir.path()).unwrap();
        for part in &parts[..parts.len() - 1] {
            std::fs::remove_file(&part.path).unwrap();
        }
        assert!(matches!(
            TraceExport::resume(dir.path()),
            Err(ExportError::PrunedHead)
        ));
    }

    #[test]
    fn a_zero_named_part_is_ignored_and_a_symlinked_part_refused() {
        let dir = tempfile::tempdir().unwrap();
        drop(rotated(dir.path(), 10));
        let parts = chain_parts(dir.path()).unwrap();
        let head = TraceExport::resume(dir.path()).unwrap().head();

        // `chain.<twenty zeros>.jsonl` is not a part name: neither reader looks at it.
        std::fs::write(
            dir.path().join("chain.00000000000000000000.jsonl"),
            b"not a record\n",
        )
        .unwrap();
        assert_eq!(chain_parts(dir.path()).unwrap(), parts);
        assert_eq!(TraceExport::resume(dir.path()).unwrap().head(), head);

        // A part replaced by a symbolic link to the same bytes is not followed.
        let elsewhere = tempfile::tempdir().unwrap();
        let copy = elsewhere.path().join("part");
        std::fs::copy(&parts[1].path, &copy).unwrap();
        std::fs::remove_file(&parts[1].path).unwrap();
        std::os::unix::fs::symlink(&copy, &parts[1].path).unwrap();
        assert!(matches!(
            TraceExport::resume(dir.path()),
            Err(ExportError::Capability(_))
        ));
    }
}
