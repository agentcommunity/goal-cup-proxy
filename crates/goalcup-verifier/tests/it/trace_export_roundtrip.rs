//! The seam between the two crates, asserted rather than assumed.

use crate::common as support;

/// The seam, asserted end to end: what `TraceExport` writes is what `TraceDescriptor`
/// reads.
#[test]
fn a_directory_the_protocol_crate_wrote_loads_through_the_verifiers_descriptor() {
    use goalcup_protocol::export::*;
    let tmp = tempfile::tempdir().unwrap();
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors/claude-code-cli/manifest.toml");
    let x = TraceExport::create(tmp.path()).unwrap();
    x.write_manifest(&std::fs::read(&manifest).unwrap())
        .unwrap();
    x.write_descriptor(&TraceDescriptorDoc {
        connector: "claude-code-cli".to_string(),
        entry_id: "e1".to_string(),
        workspace: "/ws".to_string(),
        tracked_sessions: vec!["s1".to_string()],
        harness_version: "2.1.259".to_string(),
        connector_manifest: "manifest.toml".to_string(),
        chain: Some(CHAIN_FILE.to_string()),
        receipt: None,
        baseline: None,
        acks: None,
        seal: None,
        server_public_key: None,
        expected_reread: false,
    })
    .unwrap();

    let d = goalcup_verifier::trace::TraceDescriptor::load(tmp.path()).unwrap();
    assert!(!d.is_legacy());
    assert_eq!(
        d.chain.as_ref().map(|path| path.display()),
        Some(CHAIN_FILE)
    );
    assert_eq!(d.entry_id, "e1");
    assert!(
        d.leg1.is_empty(),
        "a chain trace declares no [[leg1]] stores"
    );
    // `leg2` is optional: were it required, this file could not be loaded at all — the
    // writer would write a document its own reader refused.
    assert!(d.leg2.is_none(), "a chain trace declares no [leg2] table");
    assert!(!d.expected_reread);
}

/// The edge: `chain` is an `Option` so the three M0
/// descriptors keep loading, which means the refusal has to be spelled out instead. A
/// descriptor that names no chain and no leg is not a trace with nothing in it; it is a
/// trace whose evidence nobody said where to find.
#[test]
fn a_descriptor_that_names_neither_a_chain_nor_both_legs_is_refused() {
    let base = r#"
connector = "claude-code-cli"
entry_id = "e1"
workspace = "/ws"
tracked_sessions = ["s1"]
harness_version = "2.1.259"
connector_manifest = "manifest.toml"
"#;
    let load = |extra: &str| {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("trace.toml"), format!("{base}{extra}")).unwrap();
        goalcup_verifier::trace::TraceDescriptor::load(tmp.path())
            .map(|_| ())
            .map_err(|e| e.to_string())
    };

    let neither = load("").unwrap_err();
    assert!(neither.contains("[[leg1]]"), "{neither}");

    let no_leg2 = load(
        r#"
[[leg1]]
name = "main"
kind = "raw_jsonl"
path = "main.jsonl"
"#,
    )
    .unwrap_err();
    assert!(no_leg2.contains("[leg2]"), "{no_leg2}");

    // A chain answers for both legs at once.
    load("chain = \"chain.jsonl\"\n").unwrap();
}

/// `TraceDescriptor` refuses a key it does not read, and the three committed M0
/// descriptors are proof that the refusal is not aimed at them.
///
/// The retired `events` key is the case that motivated it: serde would ignore it, so a fixture
/// that still named an event stream would load with none — no heartbeats, no tail notice, and
/// four planted findings gone from a verdict that still said `fail` for other reasons. The
/// stricter reading is only safe if it is asserted against the descriptors that have to
/// keep working, so both halves are one test.
#[test]
fn the_three_legacy_descriptors_load_and_a_retired_key_is_refused() {
    for connector in ["claude-code-cli", "codex-cli"] {
        let fixture = support::materialize_fixture(&format!("tests/traces/{connector}"));
        let d = goalcup_verifier::trace::TraceDescriptor::load(fixture.path())
            .unwrap_or_else(|e| panic!("{connector}: {e}"));
        assert!(d.is_legacy(), "{connector} declares [[leg1]], not a chain");
        assert!(!d.leg1.is_empty());
        assert!(d.leg2.is_some());
    }

    let tmp = support::materialize_fixture("tests/traces/claude-code-cli");
    let base = std::fs::read_to_string(tmp.path().join("trace.toml")).unwrap();
    // The same descriptor, plus the key the chain retired. `load` never opens the files a
    // descriptor names, so the relative paths inside it are left exactly as committed and
    // nothing under `spikes/` is read or written.
    std::fs::write(
        tmp.path().join("trace.toml"),
        base.replacen("[[leg1]]", "events = \"events.jsonl\"\n\n[[leg1]]", 1),
    )
    .unwrap();
    let err = goalcup_verifier::trace::TraceDescriptor::load(tmp.path())
        .expect_err("a key the verifier does not read must be refused, not ignored");
    assert!(
        err.to_string().contains("events"),
        "the refusal names the key: {err}"
    );
}

/// A chain trace declares no stores. Both halves are refused, and the refusal names what
/// it found.
///
/// This is the side channel: a descriptor could name a chain and a `[[leg1]]` file and a
/// `[leg2]` file, and a verifier that read all three would let a participant hand over a
/// chain the server had acknowledged and, beside it, a store file holding whatever they
/// liked. Every check downstream would read those bytes as
/// leg-1 evidence, and no link in the chain covers a single one of them. There is no honest
/// way to merge the two: the chain is the evidence, and a file beside it is a claim with
/// nothing behind it.
#[test]
fn a_chain_trace_that_also_declares_stores_is_refused() {
    use goalcup_protocol::export::*;

    let write = |dir: &std::path::Path, extra: &str| {
        let x = TraceExport::create(dir).unwrap();
        x.write_descriptor(&TraceDescriptorDoc {
            connector: "claude-code-cli".to_string(),
            entry_id: "e1".to_string(),
            workspace: "/ws".to_string(),
            tracked_sessions: vec!["s1".to_string()],
            harness_version: "2.1.259".to_string(),
            connector_manifest: "manifest.toml".to_string(),
            chain: Some(CHAIN_FILE.to_string()),
            receipt: None,
            baseline: None,
            acks: None,
            seal: None,
            server_public_key: None,
            expected_reread: false,
        })
        .unwrap();
        let path = dir.join(DESCRIPTOR_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text + extra).unwrap();
        goalcup_verifier::trace::TraceDescriptor::load(dir)
    };

    let one_store = tempfile::tempdir().unwrap();
    let err = write(
        one_store.path(),
        "\n[[leg1]]\nname = \"main\"\nkind = \"raw_jsonl\"\npath = \"main.jsonl\"\n",
    )
    .expect_err("a chain trace must not also declare a [[leg1]] store");
    let message = err.to_string();
    assert!(
        message.contains('1') && message.contains("leg1"),
        "the refusal names what it found: {message}"
    );

    let a_post_stream = tempfile::tempdir().unwrap();
    let err = write(a_post_stream.path(), "\n[leg2]\npath = \"hooks.jsonl\"\n")
        .expect_err("a chain trace must not also declare a [leg2] table");
    assert!(
        err.to_string().contains("leg2"),
        "the refusal names what it found: {err}"
    );

    // And the shape the exporter actually writes still loads, so the gate is aimed at the
    // side channel rather than at the daemon's own output.
    let clean = tempfile::tempdir().unwrap();
    let d = write(clean.path(), "").expect("a chain trace with no stores loads");
    assert!(d.leg1.is_empty() && d.leg2.is_none());
}
