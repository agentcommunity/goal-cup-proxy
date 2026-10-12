//! Security boundary: participant descriptors name only no-follow descendants of one pinned
//! trace root. Fixtures are materialized into temporary roots; recorded bytes never change.

mod support;

use std::path::Path;

fn m0() -> tempfile::TempDir {
    support::materialize_fixture("tests/traces/claude-code-cli")
}

fn compliant() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    support::compliant::generate(dir.path());
    dir
}

fn rewrite(root: &Path, from: &str, to: &str) {
    let descriptor = root.join("trace.toml");
    let text = std::fs::read_to_string(&descriptor).unwrap();
    assert_eq!(text.matches(from).count(), 1, "fixture needs one {from:?}");
    std::fs::write(&descriptor, text.replacen(from, to, 1)).unwrap();
}

fn remove_line(root: &Path, needle: &str) {
    let descriptor = root.join("trace.toml");
    let text = std::fs::read_to_string(&descriptor).unwrap();
    assert_eq!(
        text.matches(needle).count(),
        1,
        "fixture needs one {needle:?}"
    );
    let retained = text
        .lines()
        .filter(|line| !line.contains(needle))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&descriptor, format!("{retained}\n")).unwrap();
}

fn containment_error(root: &Path) {
    let error =
        goalcup_verifier::verify_trace(root).expect_err("escaped descriptor path is refused");
    assert!(
        error.to_string().contains("relative descendant")
            || error.to_string().contains("non-empty relative path")
            || error.to_string().contains("not a regular file")
            || error.to_string().contains("hard linked")
            || error.to_string().contains("io "),
        "the root boundary, rather than outside bytes, reports the path: {error}"
    );
    assert!(
        !error.to_string().contains("OUTSIDE_SENTINEL"),
        "outside bytes must not become a parser error: {error}"
    );
}

#[test]
fn every_descriptor_evidence_field_rejects_parent_absolute_and_normalized_path_grammar() {
    let legacy = [
        "connector_manifest = \"manifest.toml\"",
        "path = \"main-session.jsonl\"",
        "path = \"subagent-a8ca2ea8d4a8a1673.jsonl\"",
        "path = \"a9-other-folder.jsonl\"",
        "path = \"hooks.jsonl\"",
        "path = \"rawbody-goal-turn.request.json\"",
        "path = \"rawbody-goal-turn.response.json\"",
        "path = \"rawbody-killed-turn-orphan.request.json\"",
        "path = \"rawbody-subagent.request.json\"",
    ];
    let modern = [
        "connector_manifest = \"manifest.toml\"",
        "chain = \"chain.jsonl\"",
        "receipt = \"receipt.json\"",
        "baseline = \"baseline.json\"",
        "acks = \"acks.jsonl\"",
        "seal = \"seal.json\"",
    ];
    for path in [
        "../outside",
        "/tmp/outside",
        "a//b",
        "a/./b",
        "a/",
        "a\\u0000b",
        "C:\\\\outside",
        "//server/share",
    ] {
        for original in legacy {
            let trace = m0();
            // Replace just the quoted value while retaining the particular evidence field.
            let replacement = format!("{}\"{path}\"", original.split_once('"').unwrap().0);
            rewrite(trace.path(), original, &replacement);
            containment_error(trace.path());
        }
        for original in modern {
            let trace = compliant();
            let replacement = format!("{}\"{path}\"", original.split_once('"').unwrap().0);
            rewrite(trace.path(), original, &replacement);
            containment_error(trace.path());
        }
    }
}

#[cfg(unix)]
#[test]
fn every_evidence_reader_refuses_final_and_intermediate_symlinks() {
    use std::os::unix::fs::symlink;

    let legacy = [
        "connector_manifest = \"manifest.toml\"",
        "path = \"main-session.jsonl\"",
        "path = \"subagent-a8ca2ea8d4a8a1673.jsonl\"",
        "path = \"a9-other-folder.jsonl\"",
        "path = \"hooks.jsonl\"",
        "path = \"rawbody-goal-turn.request.json\"",
        "path = \"rawbody-goal-turn.response.json\"",
        "path = \"rawbody-killed-turn-orphan.request.json\"",
        "path = \"rawbody-subagent.request.json\"",
    ];
    let modern = [
        "connector_manifest = \"manifest.toml\"",
        "chain = \"chain.jsonl\"",
        "receipt = \"receipt.json\"",
        "baseline = \"baseline.json\"",
        "acks = \"acks.jsonl\"",
        "seal = \"seal.json\"",
    ];
    for original in legacy.into_iter().chain(modern) {
        for intermediate in [false, true] {
            let trace = if original.starts_with("chain")
                || original.starts_with("receipt")
                || original.starts_with("baseline")
                || original.starts_with("acks")
                || original.starts_with("seal")
            {
                compliant()
            } else {
                m0()
            };
            let outside = tempfile::tempdir().unwrap();
            let name = "outside.json";
            std::fs::write(outside.path().join(name), "OUTSIDE_SENTINEL").unwrap();
            let replacement = if intermediate {
                symlink(outside.path(), trace.path().join("outside-dir")).unwrap();
                "outside-dir/outside.json"
            } else {
                symlink(outside.path().join(name), trace.path().join("outside.json")).unwrap();
                "outside.json"
            };
            let replacement = format!("{}\"{replacement}\"", original.split_once('"').unwrap().0);
            rewrite(trace.path(), original, &replacement);
            containment_error(trace.path());
        }
    }
}

/// A hard link is not a symlink: descriptor-relative no-follow traversal opens it unless
/// the final retained handle proves it is the trace tree's only name for that inode.
#[cfg(unix)]
#[test]
fn every_descriptor_evidence_reader_refuses_hard_linked_leaves() {
    let legacy = [
        "connector_manifest = \"manifest.toml\"",
        "path = \"main-session.jsonl\"",
        "path = \"subagent-a8ca2ea8d4a8a1673.jsonl\"",
        "path = \"a9-other-folder.jsonl\"",
        "path = \"hooks.jsonl\"",
        "path = \"rawbody-goal-turn.request.json\"",
        "path = \"rawbody-goal-turn.response.json\"",
        "path = \"rawbody-killed-turn-orphan.request.json\"",
        "path = \"rawbody-subagent.request.json\"",
    ];
    let modern = [
        "connector_manifest = \"manifest.toml\"",
        "chain = \"chain.jsonl\"",
        "receipt = \"receipt.json\"",
        "baseline = \"baseline.json\"",
        "acks = \"acks.jsonl\"",
        "seal = \"seal.json\"",
    ];
    for original in legacy.into_iter().chain(modern) {
        let trace = if original.starts_with("chain")
            || original.starts_with("receipt")
            || original.starts_with("baseline")
            || original.starts_with("acks")
            || original.starts_with("seal")
        {
            compliant()
        } else {
            m0()
        };
        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join("evidence.json");
        std::fs::write(&sentinel, "OUTSIDE_SENTINEL").unwrap();
        let linked = trace.path().join("hard-linked.json");
        std::fs::hard_link(&sentinel, &linked).unwrap();
        let replacement = format!(
            "{}\"hard-linked.json\"",
            original.split_once('\"').unwrap().0
        );
        rewrite(trace.path(), original, &replacement);
        containment_error(trace.path());
    }
}

#[cfg(unix)]
#[test]
fn selected_trace_root_and_raw_directory_symlinks_are_refused() {
    use std::os::unix::fs::symlink;

    let parent = tempfile::tempdir().unwrap();
    let trace = m0();
    let link = parent.path().join("trace-link");
    symlink(trace.path(), &link).unwrap();
    for suffix in ["", "/", "/."] {
        containment_error(Path::new(&format!("{}{}", link.display(), suffix)));
    }

    let trace = m0();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(
        outside.path().join("rawbody-goal-turn.request.json"),
        "OUTSIDE_SENTINEL",
    )
    .unwrap();
    symlink(outside.path(), trace.path().join("raw-link")).unwrap();
    rewrite(
        trace.path(),
        "path = \"rawbody-goal-turn.request.json\"",
        "path = \"raw-link/rawbody-goal-turn.request.json\"",
    );
    containment_error(trace.path());
}

#[cfg(unix)]
#[test]
fn fixed_trace_descriptor_leaf_symlink_is_refused() {
    use std::os::unix::fs::symlink;

    let trace = m0();
    let outside = tempfile::tempdir().unwrap();
    let descriptor = trace.path().join("trace.toml");
    let outside_descriptor = outside.path().join("trace.toml");
    std::fs::rename(&descriptor, &outside_descriptor).unwrap();
    symlink(&outside_descriptor, &descriptor).unwrap();

    containment_error(trace.path());
}

#[cfg(unix)]
#[test]
fn special_leaf_files_are_not_evidence() {
    let trace = m0();
    let fifo = trace.path().join("fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    rewrite(
        trace.path(),
        "connector_manifest = \"manifest.toml\"",
        "connector_manifest = \"fifo\"",
    );
    containment_error(trace.path());

    let trace = m0();
    let socket = trace.path().join("socket");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    rewrite(
        trace.path(),
        "connector_manifest = \"manifest.toml\"",
        "connector_manifest = \"socket\"",
    );
    containment_error(trace.path());
}

#[test]
fn declared_missing_optional_evidence_is_an_error_but_absence_keeps_the_v13_finding() {
    let declared = compliant();
    rewrite(
        declared.path(),
        "receipt = \"receipt.json\"",
        "receipt = \"missing.json\"",
    );
    containment_error(declared.path());

    let absent = compliant();
    remove_line(absent.path(), "receipt = \"receipt.json\"");
    let verdict = goalcup_verifier::verify_trace(absent.path()).expect("None receipt is a verdict");
    assert!(
        verdict
            .review_queue
            .iter()
            .any(|finding| finding.code == "receipt_missing")
    );
}

#[test]
fn declared_missing_acknowledgements_error_but_absence_loads_an_empty_stream() {
    let declared = compliant();
    rewrite(
        declared.path(),
        "acks = \"acks.jsonl\"",
        "acks = \"missing.jsonl\"",
    );
    let declared = goalcup_verifier::trace::TraceDescriptor::load(declared.path()).unwrap();
    assert!(
        declared.load_acks().is_err(),
        "Some(missing) acknowledgements must remain an I/O error"
    );

    let absent = compliant();
    remove_line(absent.path(), "acks = \"acks.jsonl\"");
    let absent = goalcup_verifier::trace::TraceDescriptor::load(absent.path()).unwrap();
    assert!(
        absent.load_acks().unwrap().is_empty(),
        "None acknowledgements must remain an empty stream"
    );
}

#[test]
fn materializer_rewrites_only_quoted_path_values_and_copies_exact_bytes() {
    let source = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(source.path().join("one")).unwrap();
    std::fs::create_dir_all(source.path().join("two")).unwrap();
    std::fs::write(source.path().join("one/same.json"), b"one\n").unwrap();
    std::fs::write(source.path().join("two/same.json"), b"two\n").unwrap();
    let original = "# keep this comment\n connector_manifest   =   \"one/same.json\" # retain\nentry_id = \"non-path value\"\n[leg2]\n path=\"two/same.json\" # retain too\n";
    std::fs::write(source.path().join("trace.toml"), original).unwrap();
    let copied = support::materialize_trace(source.path());
    let text = std::fs::read_to_string(copied.path().join("trace.toml")).unwrap();
    assert!(
        text.starts_with("# keep this comment\n connector_manifest   =   \"same.json\" # retain\n")
    );
    assert!(text.contains("entry_id = \"non-path value\""));
    assert!(text.contains(" path=\"evidence/1/same.json\" # retain too"));
    assert_eq!(
        std::fs::read(copied.path().join("same.json")).unwrap(),
        b"one\n"
    );
    assert_eq!(
        std::fs::read(copied.path().join("evidence/1/same.json")).unwrap(),
        b"two\n"
    );
}
