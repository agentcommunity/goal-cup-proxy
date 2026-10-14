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
