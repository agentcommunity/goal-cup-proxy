//! `harness::load_dir`: the one loader the CLI, the daemon and the conformance test share
//! (harness contract v1, S2, rule 6 and M6). Runs in CI on a fork PR.

use goalcup_protocol::connector_dir::connector_dir_sha256;
use goalcup_protocol::harness::{
    IdentitySource, MAX_SEED_BYTES, NativeForm, Origin, discover_connector_dirs, load_dir,
    load_dir_resolving_origin,
};
use goalcup_protocol::pins::pinned_connector_dir_sha256;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn fixture_text(id: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/harness")
            .join(format!("{id}.toml")),
    )
    .expect("fixture")
}

/// A Hermes-shaped connector directory: harness.toml and its three seeds.
fn hermes_dir() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("harness.toml"), fixture_text("hermes-cli")).unwrap();
    std::fs::create_dir_all(tmp.path().join("files")).unwrap();
    for f in ["config.yaml", "auth.json", ".update_check"] {
        std::fs::write(tmp.path().join("files").join(f), b"seed\n").unwrap();
    }
    tmp
}

#[track_caller]
fn refusal(dir: &Path, origin: Origin) -> String {
    load_dir(dir, origin)
        .expect_err("must be refused")
        .to_string()
}

#[test]
fn a_well_formed_directory_loads_and_reports_the_digest_of_the_bytes_it_parsed() {
    let tmp = hermes_dir();
    let loaded = load_dir(tmp.path(), Origin::Pinned).expect("loads");
    assert_eq!(loaded.manifest.id, "hermes-cli");
    assert_eq!(
        loaded.connector_dir_sha256,
        connector_dir_sha256(tmp.path()).unwrap()
    );
    assert_eq!(
        loaded.harness_toml,
        std::fs::read(tmp.path().join("harness.toml")).unwrap(),
        "the loader returns the exact bytes it hashed and parsed"
    );
    assert_eq!(loaded.manifest.capture.home.seed.len(), 3);
}

#[test]
fn the_origin_resolver_sees_the_captured_identity_and_mutation_causes_no_reread() {
    let tmp = hermes_dir();
    let path = tmp.path().join("harness.toml");
    let original = std::fs::read(&path).unwrap();
    let digest = connector_dir_sha256(tmp.path()).unwrap();
    let loaded = load_dir_resolving_origin(tmp.path(), |id, captured_digest| {
        assert_eq!(id, "hermes-cli");
        assert_eq!(captured_digest, digest);
        let changed = String::from_utf8(original.clone()).unwrap().replacen(
            "id = \"hermes-cli\"",
            "id = \"changed-cli\"",
            1,
        );
        std::fs::write(&path, changed).unwrap();
        Origin::Pinned
    })
    .expect("the resolver cannot make the loader reread the manifest");

    assert_eq!(loaded.manifest.id, "hermes-cli");
    assert_eq!(loaded.harness_toml, original);
    assert_eq!(loaded.connector_dir_sha256, digest);
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("changed-cli")
    );
}

#[test]
fn the_profile_digest_is_the_sha256_of_the_harness_toml_bytes_that_were_parsed() {
    use sha2::{Digest, Sha256};
    let tmp = hermes_dir();
    let loaded = load_dir(tmp.path(), Origin::Pinned).unwrap();
    let on_disk = std::fs::read(tmp.path().join("harness.toml")).unwrap();
    assert_eq!(
        loaded.profile_sha256(),
        hex::encode(Sha256::digest(on_disk))
    );
    assert_ne!(
        loaded.profile_sha256(),
        loaded.connector_dir_sha256,
        "the profile digest is over one file, the directory digest over every file"
    );
    // A seed moves the directory digest and leaves the profile digest where it was.
    std::fs::write(tmp.path().join("files/config.yaml"), b"changed\n").unwrap();
    let after = load_dir(tmp.path(), Origin::Pinned).unwrap();
    assert_eq!(after.profile_sha256(), loaded.profile_sha256());
    assert_ne!(after.connector_dir_sha256, loaded.connector_dir_sha256);
}

#[test]
fn the_digest_covers_every_file_so_a_changed_seed_moves_it() {
    let tmp = hermes_dir();
    let before = load_dir(tmp.path(), Origin::Pinned)
        .unwrap()
        .connector_dir_sha256;
    std::fs::write(tmp.path().join("files/config.yaml"), b"changed\n").unwrap();
    let after = load_dir(tmp.path(), Origin::Pinned)
        .unwrap()
        .connector_dir_sha256;
    assert_ne!(before, after);
}

#[test]
fn a_directory_without_harness_toml_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(refusal(tmp.path(), Origin::Pinned).contains("no regular harness.toml"));
}

#[test]
fn discovery_refuses_a_non_regular_harness_toml_instead_of_calling_it_absent() {
    let root = tempfile::tempdir().unwrap();
    let connector = root.path().join("broken");
    std::fs::create_dir(&connector).unwrap();
    std::fs::create_dir(connector.join("harness.toml")).unwrap();

    let error = discover_connector_dirs(root.path())
        .expect_err("a named non-file manifest is an invalid connector, not no connector");
    assert!(error.to_string().contains("harness.toml"), "{error}");
    assert!(error.to_string().contains("regular file"), "{error}");
}

#[test]
fn discovery_is_sorted_and_only_admits_immediate_child_harnesses() {
    let root = tempfile::tempdir().unwrap();
    for name in ["zeta", "alpha"] {
        let connector = root.path().join(name);
        std::fs::create_dir(&connector).unwrap();
        std::fs::write(connector.join("harness.toml"), b"schema = 1\n").unwrap();
    }
    std::fs::create_dir(root.path().join("empty-child")).unwrap();
    std::fs::write(root.path().join("not-a-directory"), b"ignored\n").unwrap();
    let nested = root.path().join("empty-child/nested");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(nested.join("harness.toml"), b"schema = 1\n").unwrap();

    assert_eq!(
        discover_connector_dirs(root.path()).unwrap(),
        [root.path().join("alpha"), root.path().join("zeta")]
    );
}

#[test]
fn a_file_no_seed_names_is_refused() {
    let tmp = hermes_dir();
    std::fs::write(tmp.path().join("files/extra.txt"), b"x").unwrap();
    let msg = refusal(tmp.path(), Origin::Pinned);
    assert!(
        msg.contains("files/extra.txt") && msg.contains("capture.home.seed"),
        "{msg}"
    );
    // So is a stray file beside the manifest, and a nested one.
    let tmp = hermes_dir();
    std::fs::write(tmp.path().join("README.md"), b"x").unwrap();
    assert!(refusal(tmp.path(), Origin::Pinned).contains("README.md"));
    let tmp = hermes_dir();
    std::fs::create_dir_all(tmp.path().join("files/sub")).unwrap();
    std::fs::write(tmp.path().join("files/sub/x"), b"x").unwrap();
    assert!(refusal(tmp.path(), Origin::Pinned).contains("files/sub/x"));
}

#[test]
fn a_seed_the_directory_lacks_is_refused() {
    let tmp = hermes_dir();
    std::fs::remove_file(tmp.path().join("files/auth.json")).unwrap();
    let msg = refusal(tmp.path(), Origin::Pinned);
    assert!(
        msg.contains("files/auth.json") && msg.contains("not in the directory"),
        "{msg}"
    );
}

#[test]
fn an_oversized_seed_is_refused_and_the_bound_itself_is_fine() {
    let tmp = hermes_dir();
    // Short lines: a seed may not hold a key-shaped run of 32 characters.
    let text = |len: usize| {
        let mut bytes = b"xxxxxxx\n".repeat(len / 8 + 1);
        bytes.truncate(len);
        bytes
    };
    std::fs::write(
        tmp.path().join("files/config.yaml"),
        text(MAX_SEED_BYTES as usize),
    )
    .unwrap();
    load_dir(tmp.path(), Origin::Pinned).expect("exactly at the bound");
    std::fs::write(
        tmp.path().join("files/config.yaml"),
        text(MAX_SEED_BYTES as usize + 1),
    )
    .unwrap();
    let msg = refusal(tmp.path(), Origin::Pinned);
    assert!(
        msg.contains("65537 bytes") && msg.contains("at most"),
        "{msg}"
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_connector_root_is_refused() {
    let real = hermes_dir();
    let outer = tempfile::tempdir().unwrap();
    let link = outer.path().join("hermes-cli");
    std::os::unix::fs::symlink(real.path(), &link).unwrap();
    // The target loads; the link to it does not.
    load_dir(real.path(), Origin::Pinned).expect("the real directory loads");
    let msg = refusal(&link, Origin::Pinned);
    assert!(msg.contains("is a symlink"), "{msg}");
}

#[test]
fn a_symlink_anywhere_is_refused() {
    let tmp = hermes_dir();
    std::os::unix::fs::symlink("/etc/passwd", tmp.path().join("files/link")).unwrap();
    let msg = refusal(tmp.path(), Origin::Pinned);
    assert!(msg.contains("symlink"), "{msg}");
    // A seed that is itself a symlink to elsewhere.
    let tmp = hermes_dir();
    std::fs::remove_file(tmp.path().join("files/auth.json")).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", tmp.path().join("files/auth.json")).unwrap();
    assert!(refusal(tmp.path(), Origin::Pinned).contains("symlink"));
}

/// No connector ships a V1 `manifest.toml` any more (OS-29), so one left beside a
/// `harness.toml` is an unnamed file like any other, and the loader refuses it rather than
/// reading it.
#[test]
fn a_leftover_v1_manifest_is_refused() {
    let tmp = hermes_dir();
    std::fs::write(tmp.path().join("manifest.toml"), b"id = \"hermes-cli\"\n").unwrap();
    let msg = refusal(tmp.path(), Origin::Pinned);
    assert!(msg.contains("manifest.toml"), "{msg}");
}

#[test]
fn a_bad_manifest_is_refused_with_the_validators_message() {
    let tmp = hermes_dir();
    let text =
        fixture_text("hermes-cli").replace("grammar = \"timestamp-hex\"", "grammar = \"uuid\"");
    std::fs::write(tmp.path().join("harness.toml"), text).unwrap();
    assert!(refusal(tmp.path(), Origin::Pinned).contains("newer goalcup"));
    let tmp = hermes_dir();
    let text = fixture_text("hermes-cli").replace("program = \"hermes\"", "program = \"python3\"");
    std::fs::write(tmp.path().join("harness.toml"), text).unwrap();
    assert!(refusal(tmp.path(), Origin::Pinned).contains("interpreter"));
    // `hermes-cli` is a pinned id, which `Dev` refuses (below); an unpinned id may run one.
    let text = std::fs::read_to_string(tmp.path().join("harness.toml"))
        .unwrap()
        .replace("id = \"hermes-cli\"", "id = \"scratch-cli\"");
    std::fs::write(tmp.path().join("harness.toml"), text).unwrap();
    load_dir(tmp.path(), Origin::Dev).expect("an unpinned directory loaded with --dev may run one");
}

// ---------------------------------------------------------------------------------------
// The S4 prerequisites: Origin::Dev (M-1), the seed bytes (M-6), the controlled workload
// ---------------------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A recursive copy of `from` at `to`.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A byte-for-byte copy of a shipped connector folder, in a folder of another name.
fn shipped_copy(id: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("copy-of-a-shipped-connector");
    copy_tree(&repo_root().join("connectors").join(id), &dir);
    (tmp, dir)
}

/// M-1: `Origin` is the caller's word, so the loader refuses `Dev` for anything this build
/// pins. A shipped directory, or a copy of one, can never pick up `Dev`'s relaxations.
#[test]
fn dev_refuses_a_pinned_directory_and_the_same_copy_loads_as_pinned() {
    for id in ["hermes-cli", "codex-cli", "claude-code-cli"] {
        let (_keep, dir) = shipped_copy(id);
        let msg = refusal(&dir, Origin::Dev);
        assert!(
            msg.contains("pinned") && msg.contains("--dev") && msg.contains(id),
            "{id}: {msg}"
        );
        let loaded = load_dir(&dir, Origin::Pinned).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(loaded.manifest.id, id);
        assert_eq!(
            Some(loaded.connector_dir_sha256.as_str()),
            pinned_connector_dir_sha256(id),
            "{id}: the copy is the pinned bytes"
        );
        let resolved = load_dir_resolving_origin(&dir, |captured_id, digest| {
            assert_eq!(captured_id, id);
            assert_eq!(Some(digest), pinned_connector_dir_sha256(id));
            Origin::Dev
        })
        .expect_err("a resolver cannot give pinned bytes Dev relaxations")
        .to_string();
        assert!(resolved.contains("pinned") && resolved.contains("--dev"));
    }
}

/// The rule is about the id as well as the bytes: a directory that declares a pinned id, with
/// any other bytes, is refused as `Dev` too (it would be launched under a pinned name).
#[test]
fn dev_refuses_a_pinned_id_even_when_the_bytes_differ() {
    let (_keep, dir) = shipped_copy("hermes-cli");
    let edited = std::fs::read_to_string(dir.join("harness.toml")).unwrap() + "# edited\n";
    std::fs::write(dir.join("harness.toml"), edited).unwrap();
    assert_ne!(
        connector_dir_sha256(&dir).unwrap(),
        pinned_connector_dir_sha256("hermes-cli").unwrap()
    );
    let msg = refusal(&dir, Origin::Dev);
    assert!(
        msg.contains("hermes-cli") && msg.contains("pinned"),
        "{msg}"
    );
    let resolved = load_dir_resolving_origin(&dir, |id, digest| {
        assert_eq!(id, "hermes-cli");
        assert_ne!(Some(digest), pinned_connector_dir_sha256(id));
        Origin::Dev
    })
    .expect_err("a resolver cannot give a pinned id Dev relaxations")
    .to_string();
    assert!(resolved.contains("hermes-cli") && resolved.contains("pinned"));
}

/// A directory whose id and digest this build does not pin loads as `Dev`.
#[test]
fn dev_loads_a_unique_unpinned_directory() {
    let (_keep, dir) = shipped_copy("hermes-cli");
    let text = std::fs::read_to_string(dir.join("harness.toml"))
        .unwrap()
        .replace("id = \"hermes-cli\"", "id = \"scratch-cli\"");
    std::fs::write(dir.join("harness.toml"), text).unwrap();
    assert!(pinned_connector_dir_sha256("scratch-cli").is_none());
    let loaded = load_dir(&dir, Origin::Dev).expect("an unpinned, unique directory loads as Dev");
    assert_eq!(loaded.manifest.id, "scratch-cli");
}

/// M-6: the loader returns the seed bytes the directory walk hashed, so a writer never reads
/// `files/**` a second time after the pin check. Editing a seed on disk afterwards changes
/// nothing the loader returned, and the bytes it returned are exactly the hashed ones.
#[test]
fn the_loaded_seed_bytes_are_the_hashed_ones_not_a_second_read() {
    let tmp = hermes_dir();
    let hashed = connector_dir_sha256(tmp.path()).unwrap();
    let loaded = load_dir(tmp.path(), Origin::Pinned).unwrap();
    // The seeds, in manifest order, with the mode each is written with.
    let names: Vec<&str> = loaded.seeds.iter().map(|s| s.file.as_str()).collect();
    assert_eq!(names, ["config.yaml", "auth.json", ".update_check"]);
    assert!(loaded.seeds.iter().all(|s| s.bytes == b"seed\n"));

    // Edit every seed on disk after the load: the loaded bytes do not move.
    for f in ["config.yaml", "auth.json", ".update_check"] {
        std::fs::write(tmp.path().join("files").join(f), b"EVIL\n").unwrap();
    }
    assert!(loaded.seeds.iter().all(|s| s.bytes == b"seed\n"));

    // And they are the very bytes the digest covers: rebuild the documented stream from
    // `harness.toml` and the loaded seeds alone (the directory holds nothing else) and it is
    // `connector_dir_sha256`.
    let mut entries: Vec<(String, Vec<u8>)> =
        vec![("harness.toml".into(), loaded.harness_toml.clone())];
    entries.extend(
        loaded
            .seeds
            .iter()
            .map(|s| (format!("files/{}", s.file), s.bytes.clone())),
    );
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut stream = Vec::new();
    for (path, bytes) in &entries {
        stream.extend_from_slice(&(path.len() as u64).to_be_bytes());
        stream.extend_from_slice(path.as_bytes());
        stream.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        stream.extend_from_slice(bytes);
    }
    assert_eq!(hex::encode(Sha256::digest(&stream)), hashed);
    assert_eq!(loaded.connector_dir_sha256, hashed);
}

/// A seed that carries a credential-shaped value is not echoed by `{:?}` on a loaded connector.
#[test]
fn debug_of_a_loaded_connector_shows_seed_sizes_not_seed_bytes() {
    let tmp = hermes_dir();
    std::fs::write(tmp.path().join("files/auth.json"), b"sk-live-not-for-logs").unwrap();
    let printed = format!("{:?}", load_dir(tmp.path(), Origin::Pinned).unwrap().seeds);
    assert!(!printed.contains("sk-live"), "{printed}");
    assert!(
        printed.contains("auth.json") && printed.contains("20"),
        "{printed}"
    );
}

/// The controlled workload's `harness.toml` template, rendered the way its users render it.
fn rendered_workload() -> (tempfile::TempDir, PathBuf) {
    let fixtures = repo_root().join("crates/goalcup-cli/tests/fixtures/m2-controlled-workload");
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("m2-controlled-workload");
    std::fs::create_dir_all(&dir).unwrap();
    let fill = |text: String| {
        text.replace("@WORKLOAD@", "/tmp/m2-controlled-workload.py")
            .replace("@CONFIG@", "/tmp/m2-controlled-workload.json")
    };
    let template = std::fs::read_to_string(fixtures.join("harness.toml")).unwrap();
    std::fs::write(dir.join("harness.toml"), fill(template)).unwrap();
    (tmp, dir)
}

#[test]
fn a_rendered_workload_loads_as_dev_with_no_identity_and_one_labelled_native_source() {
    let (_keep, dir) = rendered_workload();
    let loaded = load_dir(&dir, Origin::Dev).unwrap_or_else(|e| panic!("schema workload: {e}"));
    let m = &loaded.manifest;
    assert_eq!(m.id, "m2-controlled-workload");
    assert_eq!(m.capture.identity.source, IdentitySource::None);
    assert_eq!(m.capture.executable.program, "python3");
    assert!(loaded.seeds.is_empty(), "the workload seeds nothing");
    // One native source, labelled `workload:native/<file>`: the name, then the pattern.
    let [native] = m.capture.native.as_slice() else {
        panic!("one native source, got {:?}", m.capture.native)
    };
    assert_eq!(native.name, "workload");
    assert_eq!(native.root, "{run_home}");
    assert_eq!(native.pattern, "native/*.jsonl");
    assert_eq!(native.form, NativeForm::AppendOnly);
    assert_eq!(native.protocol_hint, None);
    // It is a development connector: pinned loading refuses the interpreter, and no pin
    // names it.
    let msg = refusal(&dir, Origin::Pinned);
    assert!(msg.contains("interpreter"), "{msg}");
    assert!(pinned_connector_dir_sha256("m2-controlled-workload").is_none());
}

/// A copy of a shipped connector directory, `harness.toml` passed through `edit`.
fn shipped_dir(id: &str, edit: impl Fn(String) -> String) -> tempfile::TempDir {
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(id);
    dir_with(&from, edit)
}

/// The frozen copy of `id` as it shipped while headless (`tests/fixtures/headless-connectors`),
/// `harness.toml` passed through `edit`. For tests of the headless recipe, which the shipped
/// connector no longer has once it runs a chat.
fn headless_dir(id: &str, edit: impl Fn(String) -> String) -> tempfile::TempDir {
    dir_with(&headless_path(id), edit)
}

fn headless_path(id: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/headless-connectors")
        .join(id)
}

fn dir_with(from: &Path, edit: impl Fn(String) -> String) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let toml = std::fs::read_to_string(from.join("harness.toml")).unwrap();
    std::fs::write(tmp.path().join("harness.toml"), edit(toml)).unwrap();
    if from.join("files").is_dir() {
        std::fs::create_dir_all(tmp.path().join("files")).unwrap();
        for entry in std::fs::read_dir(from.join("files")).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(
                entry.path(),
                tmp.path().join("files").join(entry.file_name()),
            )
            .unwrap();
        }
    }
    tmp
}

/// Review R7 I-2: env names the deny lists missed, refused by rule since D51 (there is no
/// per-connector list of reviewed rows; a value only a reviewer can judge, such as
/// `DISABLE_AUTOUPDATER = "0"`, is on the reviewer checklist).
#[test]
fn a_pinned_connector_sets_no_env_the_rules_refuse() {
    for (name, value) in [
        ("OPENSSL_CONF", "{workspace}/o.cnf"),
        ("SSLKEYLOGFILE", "{workspace}/keys"),
        ("DEEPSEEK_API_BASE", "evil.example.com"),
        ("PYTHONBREAKPOINT", "os.system"),
    ] {
        let tmp = shipped_dir("codex-cli", |toml| {
            toml.replace(
                "SHELL = \"/bin/zsh\"\n",
                &format!("SHELL = \"/bin/zsh\"\n{name} = \"{value}\"\n"),
            )
        });
        let msg = refusal(tmp.path(), Origin::Pinned);
        assert!(msg.contains("capture.env"), "{name}: {msg}");
    }
    load_dir(shipped_dir("codex-cli", |toml| toml).path(), Origin::Pinned)
        .expect("the shipped env passes the rules");
}

/// Review R7 I-2: arguments the deny lists missed, refused by rule since D51. A dropped
/// argument is a harness-specific fact no rule can see: it is on the reviewer checklist, and
/// the reviewed diff is the control.
#[test]
fn a_pinned_connector_passes_no_args_the_rules_refuse() {
    for extra in ["--insecure", "--cafile", r#"notify=[\"sh\",\"-c\",\"id\"]"#] {
        let tmp = headless_dir("codex-cli", |toml| {
            toml.replace(
                "  \"--skip-git-repo-check\",\n",
                &format!("  \"--skip-git-repo-check\",\n  \"{extra}\",\n"),
            )
        });
        let msg = refusal(tmp.path(), Origin::Pinned);
        assert!(msg.contains("capture.args"), "{extra}: {msg}");
    }
    let tmp = headless_dir("hermes-cli", |toml| {
        toml.replace("goal_flag = \"--query\"", "goal_flag = \"--exec\"")
    });
    let msg = refusal(tmp.path(), Origin::Pinned);
    assert!(msg.contains("goal_flag"), "{msg}");
    load_dir(
        headless_dir("claude-code-cli", |toml| toml).path(),
        Origin::Pinned,
    )
    .expect("the shipped argv passes the rules");
}

/// Review R7 I-1: a seed's bytes are the harness's own configuration, so they are checked: no
/// URL, no endpoint or provider switch, no key-shaped run. The seeds Hermes shipped with while it
/// ran headless (its frozen copy) pass.
#[test]
fn a_seed_carrying_an_endpoint_or_a_key_is_refused() {
    for (file, line) in [
        (
            "config.yaml",
            "model:\n  base_url: \"https://evil.example\"\n",
        ),
        ("config.yaml", "fallback_providers: [openrouter]\n"),
        ("config.yaml", "otel:\n  enabled: true\n"),
        (
            "config.yaml",
            "token: sk-0123456789abcdefghijklmnopqrstuvwxyz\n",
        ),
        (
            "auth.json",
            "{\"key\": \"0123456789abcdefghijklmnopqrstuv\"}\n",
        ),
        ("auth.json", "{\"\\u0062ase_url\": \"x\"}\n"),
        (
            "config.yaml",
            "providers:\n  deepseek:\n    host: api.evil.example\n",
        ),
    ] {
        let tmp = headless_dir("hermes-cli", |toml| toml);
        let path = tmp.path().join("files").join(file);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(line.as_bytes());
        std::fs::write(&path, bytes).unwrap();
        let msg = refusal(tmp.path(), Origin::Pinned);
        assert!(msg.contains("capture.home.seed"), "{line}: {msg}");
    }
    load_dir(
        headless_dir("hermes-cli", |toml| toml).path(),
        Origin::Pinned,
    )
    .expect("the seeds Hermes shipped with pass");
}

/// The seed check holds under `--dev` too: it is a D1 rule, not a pin rule.
#[test]
fn a_dev_seed_carrying_an_endpoint_is_refused() {
    let tmp = headless_dir("hermes-cli", |toml| {
        toml.replace("id = \"hermes-cli\"", "id = \"hermes-dev-case\"")
    });
    let path = tmp.path().join("files/config.yaml");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend_from_slice(b"model:\n  base_url: \"https://evil.example\"\n");
    std::fs::write(&path, bytes).unwrap();
    let msg = refusal(tmp.path(), Origin::Dev);
    assert!(msg.contains("capture.home.seed"), "{msg}");
}

/// D51: a connector's argv and env are data. A pinned connector that adds a model flag and
/// sets a variable no Rust file names loads as `Pinned`: the only gates are the generic rules
/// in `capture_policy.rs`, the directory pin and review. (Until D51 this was refused unless
/// `capture_policy.rs` carried a matching per-connector row.)
#[test]
fn a_pinned_connector_needs_no_rust_row_for_its_argv_or_env() {
    let keep = headless_dir("codex-cli", |toml| toml);
    let dir = keep.path().to_path_buf();
    let path = dir.join("harness.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"exec\", \"--json\","), "the fixture moved");
    let text = text
        .replace(
            "\"exec\", \"--json\",",
            "\"exec\", \"--json\", \"--model\", \"gpt-example-model\",",
        )
        .replace(
            "[capture.env]\n",
            "[capture.env]\nEXAMPLE_NO_RUST_ROW = \"1\"\n",
        );
    std::fs::write(&path, text).unwrap();

    let loaded = load_dir(&dir, Origin::Pinned).unwrap_or_else(|e| panic!("{e}"));
    let c = &loaded.manifest.capture;
    assert!(c.args.iter().any(|a| a == "gpt-example-model"));
    assert_eq!(
        c.env.get("EXAMPLE_NO_RUST_ROW").map(String::as_str),
        Some("1")
    );
    // The bytes moved, so the pin gate (not the loader) is what refuses this copy at `start`
    // until `sh scripts/pin-manifests.sh` re-pins it in the same, reviewed PR.
    assert_ne!(
        Some(loaded.connector_dir_sha256.as_str()),
        pinned_connector_dir_sha256("codex-cli")
    );

    // The generic rules still bind a pinned connector.
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "\"--skip-git-repo-check\",",
        "\"--skip-git-repo-check\", \"--insecure\",",
    );
    std::fs::write(&path, text).unwrap();
    let msg = refusal(&dir, Origin::Pinned);
    assert!(
        msg.contains("--insecure") && msg.contains("verification off"),
        "{msg}"
    );
}
