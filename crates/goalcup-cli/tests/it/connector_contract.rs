//! The connector contract, static half: every rule a connector folder is held to that needs no
//! process, run over every folder under `connectors/` ([`shipped_connector_ids`]).
//!
//! A new harness is one folder (a `harness.toml`, its seed files and its pin line) and needs no
//! edit here: each rule reads the connector's own facts from its manifest (its home variable,
//! its reserved prefixes, its argv guard, its CA rows, its credential, its seeds) and judges
//! them. The dynamic half, one participant-shaped run of each connector through the real
//! binary, is `tests/full/connector_contract.rs`. Engine behaviour is tested elsewhere, on the
//! fixture connectors under `tests/fixtures/connectors/`, so no rule here restates one
//! connector's values: those are its `harness.toml`, pinned by digest, and its goldens.

use crate::common::m2_profile_support as s;
use crate::common::support;

use goalcup_cli::connector::HarnessConnector;
use goalcup_cli::launch::{
    CAPTURE_NO_PROXY, CaptureLaunch, LaunchScope, StderrUse, StdoutUse, StreamPlan,
    build_capture_launch, build_resumed_launch, child_secret_for_harness, harness_arg_refusal,
};
use goalcup_cli::preflight::{Level, harness_pin_gate};
use goalcup_protocol::capture_policy::{SHARED_CA_ENV, capture_arg_denied};
use goalcup_protocol::harness::{
    ConfigHome, HarnessManifest, IdentitySource, Origin, PlatformKey, Strategy,
};
use goalcup_protocol::manifest::LaunchMode;
use goalcup_protocol::pins::shipped_connector_ids;
use goalcup_protocol::template::{TemplateVars, render};
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// The shipped connector `id`, loaded the way `goalcup start` loads it (no `--dev`).
fn load(id: &str) -> HarnessConnector {
    support::connector(id)
}

/// The value the plan tests hand a credential: synthetic, of the launcher's grammar.
const KEY: &str = crate::common::secret_support::KEY;

/// `--goal`-shaped on purpose: a headless connector places it as one element.
const GOAL: &str = "--tui=hello goal";

fn rendered(templates: &[String], vars: &TemplateVars) -> Vec<String> {
    templates.iter().map(|t| render(t, vars).unwrap()).collect()
}

// ---------------------------------------------------------------------------------------
// The folder and its pin.
// ---------------------------------------------------------------------------------------

/// Every connector loads from its folder alone, as the directory this build pins (PRD 5.4):
/// the digest is over the whole directory, lowercase hex, equal to the pin, and the pin gate
/// passes without `--dev`. Its `harness.toml` with one comment appended, or any of its seed
/// files with one byte appended, is refused at load under both doors, naming the digest found.
#[test]
fn every_connector_loads_as_its_pinned_directory_and_an_edit_is_refused() {
    for id in shipped_connector_ids() {
        let c = load(&id);
        assert_eq!(c.loaded.manifest.id, id, "the folder declares its own id");
        let digest = &c.loaded.connector_dir_sha256;
        // Lowercase hex, and the length alone does not say that: the pins are compared
        // case-insensitively, so a digest that started arriving uppercase from here would
        // keep passing while every printed digest changed shape.
        assert_eq!(digest.len(), 64, "{id}");
        assert!(
            digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "{id}'s digest is lowercase hex: {digest}"
        );
        assert_eq!(
            Some(digest.as_str()),
            goalcup_protocol::pins::pinned_connector_dir_sha256(&id),
            "{id} must match goalcup_protocol::pins::pinned_connector_dir_sha256"
        );
        let gate = harness_pin_gate(&c, false);
        assert_eq!(gate.level, Level::Pass, "{id}: {}", gate.detail);

        let edited = support::connectors_dir_with_edited_harness(&id);
        assert_altered_is_refused(&edited, &id);
        // The gap the directory pin exists to close: a seed is copied into the run's home, so
        // an edit there is an edit to what the run does, under a harness.toml that is
        // unchanged byte for byte.
        for seed in &c.loaded.seeds {
            let tmp = tempfile::tempdir().unwrap();
            support::copy_connector(&tmp, &id);
            let file = tmp.path().join(&id).join("files").join(&seed.file);
            let mut body = std::fs::read(&file).unwrap();
            body.push(b'\n');
            std::fs::write(&file, &body).unwrap();
            assert_eq!(
                std::fs::read(tmp.path().join(&id).join("harness.toml")).unwrap(),
                c.loaded.harness_toml,
                "{id}: harness.toml is untouched, and a digest over it alone sees nothing"
            );
            assert_altered_is_refused(tmp.path(), &id);
        }
    }
}

/// `dir/<id>` is a pinned id holding bytes that are not its pin: `load_harness` refuses it as
/// rung 1 under both doors, naming the digest the directory actually has.
fn assert_altered_is_refused(dir: &Path, id: &str) {
    let found = goalcup_protocol::connector_dir::connector_dir_sha256(&dir.join(id)).unwrap();
    assert_ne!(
        Some(found.as_str()),
        goalcup_protocol::pins::pinned_connector_dir_sha256(id)
    );
    for dev in [false, true] {
        let error = match goalcup_cli::connector::load_harness(dir, id, dev) {
            Ok(_) => panic!("{id}: an altered pinned connector must not load (dev {dev})"),
            Err(error) => error,
        };
        assert_eq!(error.exit_code(), 1, "{error}");
        let said = error.to_string();
        assert!(
            said.contains("exact pinned bytes") && said.contains(&found),
            "{id}, dev {dev}: {said}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// The manifest's own facts.
// ---------------------------------------------------------------------------------------

/// D1 and D22: no connector sets a model base URL, and a `native_pid` kill names the binary
/// that is the session.
#[test]
fn every_connector_sets_no_model_base_url_and_names_its_kill_target() {
    for id in shipped_connector_ids() {
        let m = load(&id).loaded.manifest;
        let c = &m.capture;
        let mut env: Vec<&String> = c.env.keys().collect();
        env.extend(c.platform.values().flat_map(|p| p.env.keys()));
        assert!(
            !env.iter()
                .any(|k| k.to_ascii_uppercase().ends_with("_BASE_URL")),
            "{id}: D1, the tracker never sets a model base URL"
        );
        let t = m.termination();
        if t.strategy() == Strategy::NativePid {
            assert!(
                t.native_binary.is_some(),
                "{id}: D22, a native_pid kill names the binary that is the session"
            );
        }
    }
}

/// Every host a connector names is one the recording proxy accepts, and the proxy can mint
/// its per-launch CA for the set.
#[test]
fn every_capture_host_is_one_the_proxy_accepts() {
    use goalcup_cli::proxy::tls::{ProxyCa, is_config_host};
    for id in shipped_connector_ids() {
        let c = load(&id).loaded.manifest.capture;
        let approved: Vec<String> = c.hosts.iter().map(|h| h.host.clone()).collect();
        for h in approved.iter().chain(c.auth_hosts.iter()) {
            assert!(is_config_host(h), "{id}: `{h}`");
        }
        ProxyCa::generate(&approved).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

/// `m` with one more `capture.env` entry, or one more leading argument, validated as the
/// pinned connector would be: `Some(why)` when it is refused.
fn refusal_with(m: &HarnessManifest, edit: impl FnOnce(&mut HarnessManifest)) -> Option<String> {
    let mut probe = m.clone();
    edit(&mut probe);
    probe.validate(Origin::Pinned).err().map(|e| e.to_string())
}

/// The probes a connector's own manifest calls for: its home variable, every reserved prefix,
/// every CA variable (which belongs in `capture.ca`, never `capture.env`), and every option its
/// argv guard names, spelled as a harness's own parser would read it. Each is refused when the
/// connector itself tries to set it, so a new connector's own rules are judged with no fixed
/// list to extend (the #187 review: a fixed probe list never saw connector #4's own names).
#[test]
fn every_connector_refuses_its_own_home_variable_reserved_prefixes_and_guarded_options() {
    for id in shipped_connector_ids() {
        let m = load(&id).loaded.manifest;
        let c = &m.capture;
        let mut names: Vec<String> = c.home.env.iter().cloned().collect();
        names.extend(c.reserved_env_prefixes.iter().map(|p| format!("{p}X")));
        names.extend(c.ca.env.iter().cloned());
        for name in &names {
            let refused = refusal_with(&m, |p| {
                p.capture.env.insert(name.clone(), "1".to_string());
            });
            assert!(
                refused.is_some(),
                "{id}: its own `{name}` was admitted in capture.env"
            );
        }
        for arg in own_guard_probes(&m) {
            assert!(
                harness_arg_refusal(c, &arg).is_some(),
                "{id}: its argv guard does not name `{arg}`"
            );
            let refused = refusal_with(&m, |p| p.capture.args.insert(0, arg.clone()));
            assert!(
                refused.is_some(),
                "{id}: `{arg}` was admitted in capture.args"
            );
        }
    }
}

/// Every option the connector's `argv_guard` names, as the harness would read it.
fn own_guard_probes(m: &HarnessManifest) -> Vec<String> {
    let Some(g) = m.capture.argv_guard.as_ref() else {
        return Vec::new();
    };
    let mut probes = Vec::new();
    for long in &g.long {
        probes.push(format!("--{long}"));
        probes.push(format!("--{long}=x"));
        if g.long_abbreviations && long.len() > 1 {
            probes.push(format!("--{}", &long[..long.len() - 1]));
        }
    }
    probes.extend(g.short.iter().map(|s| format!("-{s}")));
    probes
}

/// Every argument a launch may pass, `args` then `resume_args`, rendered for a run, passes the
/// connector's own argv guard and the shared deny list; a chat-mode connector can be continued
/// (it has `resume_args`), and a headless one places its goal after them.
#[test]
fn every_connector_argv_passes_its_own_guard_and_the_shared_deny_list() {
    let l = s::Layout::new();
    let vars = s::vars(&l);
    for id in shipped_connector_ids() {
        let c = load(&id).loaded.manifest.capture;
        if c.interactive {
            assert!(
                !c.resume_args.is_empty(),
                "{id}: a chat-mode connector says how `goalcup continue` reopens its session"
            );
        }
        let args = rendered(&c.args, &vars);
        let resume = rendered(&c.resume_args, &vars);
        for arg in args.iter().chain(resume.iter()) {
            assert_eq!(harness_arg_refusal(&c, arg), None, "{id}: `{arg}`");
            assert_eq!(capture_arg_denied(arg), None, "{id}: `{arg}`");
        }
    }
}

/// Every connector's stream and identity plans follow from its manifest by one rule. A key
/// scan runs exactly when a credential is passed, and reads both streams. A chat-mode
/// connector (D57) leaves the terminal to the harness: both streams inherited, and no identity
/// at launch, because its sessions are the files its native source finds.
#[test]
fn every_connector_has_the_stream_and_identity_plan_its_manifest_asks_for() {
    let mut chat = 0;
    for id in shipped_connector_ids() {
        let capture = load(&id).loaded.manifest.capture;
        let plan = StreamPlan::for_spec(&capture).unwrap_or_else(|e| panic!("{id}: {e}"));
        let scope = LaunchScope::for_spec(&capture).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(plan.scan_for_key, capture.credential.is_some(), "{id}");
        if plan.scan_for_key {
            // The scan reads both streams, so neither may go straight to the terminal.
            assert_ne!(plan.stdout, StdoutUse::Inherit, "{id}");
            assert_ne!(plan.stderr, StderrUse::Inherit, "{id}");
            if capture.identity.source != IdentitySource::StdoutJson {
                assert_eq!(plan.stdout, StdoutUse::Forward, "{id}");
            }
        }
        // When the session is bound follows from the identity carrier alone.
        let bound = match capture.identity.source {
            IdentitySource::None => LaunchScope::None,
            IdentitySource::Preassigned => LaunchScope::Preassigned,
            IdentitySource::StdoutJson => LaunchScope::FromStdout,
            IdentitySource::StderrLine => LaunchScope::AtFinish,
            other => panic!("{id}: an identity source this test does not know: {other:?}"),
        };
        assert_eq!(scope, bound, "{id}");
        if capture.interactive {
            chat += 1;
            assert_eq!(plan.stdout, StdoutUse::Inherit, "{id}");
            assert_eq!(plan.stderr, StderrUse::Inherit, "{id}");
            assert_eq!(scope, LaunchScope::None, "{id}");
            assert!(capture.credential.is_none(), "{id}: chat mode takes no key");
        }
    }
    assert!(
        chat > 0,
        "the product is chat mode: at least one connector is"
    );
}

// ---------------------------------------------------------------------------------------
// The launch plan, against a fake executable.
// ---------------------------------------------------------------------------------------

/// A tracker environment a participant could have: their identity, a proxy, a base URL, a
/// provider key of their own, an injection variable, and the connector's own credential.
fn hostile_parent(c: &goalcup_protocol::harness::CaptureSpec) -> BTreeMap<String, String> {
    let mut parent = s::parent_env();
    for (k, v) in [
        ("HTTPS_PROXY", "http://evil.example:3128"),
        ("NO_PROXY", "*"),
        ("OPENAI_BASE_URL", "http://evil.example"),
        ("ANTHROPIC_BASE_URL", "http://evil.example"),
        ("OPENAI_API_KEY", "sk-not-a-real-key-0123456789"),
        ("NODE_OPTIONS", "--require /evil.js"),
        ("SSL_CERT_FILE", "/evil/roots.pem"),
        ("TERM", "xterm-256color"),
        ("LANG", "en_US.UTF-8"),
    ] {
        parent.insert(k.to_string(), v.to_string());
    }
    if let Some(home) = &c.home.env {
        parent.insert(home.clone(), "/participant/real-home".to_string());
    }
    for name in &c.ca.env {
        parent.insert(name.clone(), "/evil/ca.pem".to_string());
    }
    if let Some(credential) = &c.credential {
        parent.insert(credential.env.clone(), KEY.to_string());
    }
    parent
}

/// Every connector's launch environment is built from nothing (D36): only the connector's own
/// entries, the launch's pins (`HOME`, `PATH`, the loopback proxy and its exemptions, the
/// harness home, the CA rows) and the identity and terminal names are set, each pin has the
/// launch's value and not the participant's, and no model base URL is set. A provider key the
/// connector names is in no argv, no plan `Debug` and no plan environment, and the parent
/// environment is left exactly as it was.
#[test]
fn every_connector_launch_environment_is_built_from_nothing() {
    for id in shipped_connector_ids() {
        let c = load(&id);
        let capture = &c.loaded.manifest.capture;
        let l = s::Layout::new();
        let exe = s::fake_harness(&l.bin);
        let resolved = s::resolved(&exe);
        let proxy = s::proxy_url();
        let parent = hostile_parent(capture);
        let original = parent.clone();
        let bundle = l.root.path().join("bundle.pem");
        std::fs::write(&bundle, b"roots\n").unwrap();
        let goal = GOAL.as_bytes();
        let plan = build_capture_launch(
            &c,
            &s::vars(&l),
            &l.workspace,
            &CaptureLaunch {
                goal: (!capture.interactive).then_some(goal),
                ca_bundle: Some(&bundle),
                ..s::chat_launch(&l, &resolved, &proxy, &parent)
            },
        )
        .unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(parent, original, "{id}: the parent env is untouched");

        let own = capture.resolved_env(PlatformKey::current());
        let mut allowed: BTreeSet<String> = own.keys().cloned().collect();
        for pin in [
            "HOME",
            "PATH",
            "HTTPS_PROXY",
            "https_proxy",
            "NO_PROXY",
            "no_proxy",
            "USER",
            "LOGNAME",
            "TMPDIR",
        ] {
            allowed.insert(pin.to_string());
        }
        allowed.extend(capture.home.env.iter().cloned());
        allowed.extend(capture.ca.env.iter().cloned());
        if capture.interactive {
            allowed.extend(goalcup_cli::launch::CHAT_PASSTHROUGH_ENV.map(str::to_string));
        }
        for key in plan.env_set.keys() {
            assert!(
                allowed.contains(key) || (capture.interactive && key.starts_with("LC_")),
                "{id}: `{key}` is in the launch environment"
            );
            assert!(
                !key.to_ascii_uppercase().ends_with("_BASE_URL"),
                "{id}: `{key}`"
            );
        }
        let get = |k: &str| plan.env_set.get(k).map(String::as_str);
        assert_eq!(get("HOME"), l.run_home.to_str(), "{id}");
        assert_eq!(get("HTTPS_PROXY"), Some(proxy.as_str()), "{id}");
        assert_eq!(get("https_proxy"), Some(proxy.as_str()), "{id}");
        assert_eq!(get("NO_PROXY"), Some(CAPTURE_NO_PROXY), "{id}");
        if let Some(home) = &capture.home.env {
            assert_eq!(get(home), l.harness_home.to_str(), "{id}: `{home}`");
        }
        for name in &capture.ca.env {
            let want = if SHARED_CA_ENV.contains(&name.as_str()) {
                &bundle
            } else {
                &l.ca
            };
            assert_eq!(get(name), want.to_str(), "{id}: `{name}`");
        }
        for (k, v) in &own {
            assert_eq!(get(k), Some(v.as_str()), "{id}: its own `{k}`");
        }

        if let Some(credential) = &capture.credential {
            let name = credential.env.as_str();
            assert!(!plan.env_set.contains_key(name), "{id}: `{name}`");
            assert!(plan.env_set.values().all(|v| v != KEY), "{id}");
            assert!(plan.argv.iter().all(|a| !a.contains(KEY)), "{id}");
            for text in [format!("{plan:?}"), format!("{plan:#?}")] {
                assert!(!text.contains(KEY) && !text.contains(&KEY[..8]), "{id}");
            }
            let secret = child_secret_for_harness(capture, &parent)
                .unwrap_or_else(|e| panic!("{id}: {e}"))
                .expect("the connector names a credential");
            assert_eq!(secret.name(), name, "{id}");
            assert!(!format!("{secret:?}").contains(KEY), "{id}");
        }
    }
}

/// What a launch places, by mode (D57). A chat-mode connector places no goal: its argv is its
/// rendered `args` alone, it is a `Tui` launch (the terminal's foreground group), a goal is
/// refused, and a continued launch is its `args` then its `resume_args`. A headless connector
/// refuses a launch with no goal, and places the goal as exactly one element after its `args`,
/// whatever the goal looks like.
#[test]
fn every_connector_places_its_goal_as_its_mode_says() {
    for id in shipped_connector_ids() {
        let c = load(&id);
        let capture = &c.loaded.manifest.capture;
        let l = s::Layout::new();
        let vars = s::vars(&l);
        let exe = s::fake_harness(&l.bin);
        let resolved = s::resolved(&exe);
        let proxy = s::proxy_url();
        let parent = hostile_parent(capture);
        let none = s::chat_launch(&l, &resolved, &proxy, &parent);
        let some = s::launch(&l, GOAL.as_bytes(), &resolved, &proxy, &parent);
        let args = rendered(&capture.args, &vars);
        if capture.interactive {
            let plan = build_capture_launch(&c, &vars, &l.workspace, &none)
                .unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(plan.argv, args, "{id}: the rendered args alone");
            assert_eq!(plan.capture, None, "{id}: no goal slot");
            assert_eq!(plan.mode, LaunchMode::Tui, "{id}");
            let refused = build_capture_launch(&c, &vars, &l.workspace, &some)
                .expect_err("a goal for a chat-mode connector");
            assert!(refused.to_string().contains("chat mode"), "{id}: {refused}");
            let resumed = build_resumed_launch(&c, &vars, &l.workspace, &none)
                .unwrap_or_else(|e| panic!("{id}: {e}"));
            let mut want = args.clone();
            want.extend(rendered(&capture.resume_args, &vars));
            assert_eq!(resumed.argv, want, "{id}: args, then resume_args");
        } else {
            let refused = build_capture_launch(&c, &vars, &l.workspace, &none)
                .expect_err("no goal for a headless connector");
            assert!(refused.to_string().contains("headless"), "{id}: {refused}");
            let plan = build_capture_launch(&c, &vars, &l.workspace, &some)
                .unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(&plan.argv[..args.len()], args.as_slice(), "{id}");
            let holding: Vec<&String> = plan.argv.iter().filter(|a| a.contains(GOAL)).collect();
            assert_eq!(holding.len(), 1, "{id}: one element holds the goal");
            let slot = plan.capture.expect("a goal slot");
            assert_eq!(&plan.argv[slot.goal_index], holding[0], "{id}");
            assert_eq!(
                &plan.argv[slot.goal_index][slot.goal_prefix..],
                GOAL,
                "{id}"
            );
        }
    }
}

/// The CA files `goalcup start` hands, built the way it builds them (`ca_bundle::bundle_for`,
/// then the launch): every certificate in the file each `capture.ca.env` variable is pointed at.
fn handed_ca_files(c: &HarnessConnector, root: &Path) -> (BTreeMap<String, Vec<Vec<u8>>>, Vec<u8>) {
    use goalcup_cli::ca_bundle::{bundle_for, certificates};
    let l = s::Layout::new();
    let exe = s::fake_harness(&l.bin);
    let resolved = s::resolved(&exe);
    let ca_dir = root.join("ca");
    std::fs::create_dir(&ca_dir).unwrap();
    let proxy_ca = goalcup_cli::proxy::tls::ProxyCa::generate(&["api.example.org".to_string()])
        .expect("a proxy CA");
    let ca = proxy_ca
        .write_certificate(&ca_dir)
        .expect("the proxy CA file");
    let capture = &c.loaded.manifest.capture;
    let bundle = bundle_for(capture, &ca_dir, proxy_ca.certificate_der()).expect("the bundle");
    let parent = s::parent_env();
    let proxy = s::proxy_url();
    let plan = build_capture_launch(
        c,
        &s::vars(&l),
        &l.workspace,
        &CaptureLaunch {
            goal: (!capture.interactive).then_some(GOAL.as_bytes()),
            ca_certificate: &ca,
            ca_bundle: bundle.as_deref(),
            ..s::chat_launch(&l, &resolved, &proxy, &parent)
        },
    )
    .unwrap();
    let files = capture
        .ca
        .env
        .iter()
        .map(|name| {
            let file = plan.env_set.get(name).expect("every CA variable is set");
            (name.clone(), certificates(&std::fs::read(file).unwrap()))
        })
        .collect();
    (files, proxy_ca.certificate_der().to_vec())
}

/// Which file `goalcup start` hands each CA variable (D41, WP12): a harness's own variable, in
/// either mode, gets a file holding the proxy CA and nothing else, so a replace-mode harness
/// fails closed on a connection that skips the proxy; a shared row (`SSL_CERT_FILE`,
/// `REQUESTS_CA_BUNDLE`) gets the proxy CA first and then the system's roots.
#[test]
fn every_connector_hands_each_ca_variable_the_file_its_row_names() {
    for id in shipped_connector_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let (files, proxy) = handed_ca_files(&load(&id), &root);
        assert!(
            !files.is_empty(),
            "{id}: the harness is pointed at the proxy CA"
        );
        for (name, certs) in files {
            if SHARED_CA_ENV.contains(&name.as_str()) {
                assert_eq!(certs[0], proxy, "{id}: {name} has the proxy CA first");
                assert!(certs.len() > 1, "{id}: {name} keeps the system's roots");
            } else {
                assert_eq!(
                    certs,
                    std::slice::from_ref(&proxy),
                    "{id}: {name} trusts the proxy CA alone"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// The harness home.
// ---------------------------------------------------------------------------------------

/// A per-run home is made fresh inside the run directory as exactly the connector's `dirs` and
/// seed files, private (0700 directories, each seed at its declared mode), each seed byte for
/// byte the connector's `files/` copy. A shared home declares no per-run layout. No seed names a
/// model base URL outside a comment.
#[test]
fn every_connector_home_is_exactly_what_its_manifest_declares() {
    for id in shipped_connector_ids() {
        let c = load(&id);
        let home = &c.loaded.manifest.capture.home;
        for seed in &c.loaded.seeds {
            let text = String::from_utf8_lossy(&seed.bytes).to_ascii_lowercase();
            for line in text.lines().filter(|l| !l.trim_start().starts_with('#')) {
                assert!(
                    !line.contains("base_url") && !line.contains("baseurl"),
                    "{id}: the seed {} names a base URL: {line}",
                    seed.file
                );
            }
        }
        if home.config_home == ConfigHome::Shared {
            assert!(home.per_run_dir.is_none(), "{id}");
            assert!(home.dirs.is_empty(), "{id}");
            assert!(home.seed.is_empty(), "{id}");
            continue;
        }
        let tmp = tempfile::tempdir().unwrap();
        let run = std::fs::canonicalize(tmp.path())
            .unwrap()
            .join("goalcup-run");
        std::fs::create_dir(&run).unwrap();
        let made = goalcup_cli::per_run_home::create_per_run_home(&run, home, &c.loaded.seeds)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(made.starts_with(&run), "{id}: {}", made.display());
        let mode = |p: &Path| std::fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&made), 0o700, "{id}");
        let mut want: BTreeSet<String> = BTreeSet::new();
        for dir in &home.dirs {
            for (i, _) in dir.match_indices('/').chain([(dir.len(), "")]) {
                want.insert(dir[..i].to_string());
            }
        }
        for seed in &c.loaded.seeds {
            want.insert(seed.file.clone());
            for (i, _) in seed.file.match_indices('/') {
                want.insert(seed.file[..i].to_string());
            }
            let path = made.join(&seed.file);
            let declared = u32::from_str_radix(seed.mode.as_str(), 8).unwrap();
            assert_eq!(mode(&path), declared, "{id}: {}", seed.file);
            assert_eq!(
                std::fs::read(&path).unwrap(),
                std::fs::read(c.dir.join("files").join(&seed.file)).unwrap(),
                "{id}: {} is the connector's file, byte for byte",
                seed.file
            );
        }
        let mut found = BTreeSet::new();
        walk(&made, &made, &mut found);
        assert_eq!(
            found, want,
            "{id}: the home holds what the manifest declares"
        );
        for entry in &found {
            let path = made.join(entry);
            if path.is_dir() {
                assert_eq!(mode(&path), 0o700, "{id}: {entry}");
            }
        }
    }
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeSet<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        out.insert(path.strip_prefix(root).unwrap().display().to_string());
        if path.is_dir() {
            walk(root, &path, out);
        }
    }
}

// ---------------------------------------------------------------------------------------
// What doctor says.
// ---------------------------------------------------------------------------------------

/// `goalcup doctor --harness <id>` reports the connector alone, with no daemon, and names every
/// protocol hint it declares: decoded or capture-only, never left out.
#[test]
fn doctor_reports_every_connector_and_each_of_its_protocol_hints() {
    use goalcup_cli::commands::doctor::{DoctorOptions, run_with};
    for id in shipped_connector_ids() {
        let cfg = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        run_with(
            None,
            &support::connectors_dir(),
            cfg.path(),
            &BTreeMap::new(),
            &DoctorOptions {
                json: false,
                harness: Some(id.clone()),
            },
            &mut out,
        )
        .unwrap_or_else(|e| panic!("{id}: {e}"));
        let said = String::from_utf8(out).unwrap();
        assert!(said.contains(&format!("{id} only")), "{id}: {said}");
        let capture = load(&id).loaded.manifest.capture;
        let hints = capture
            .hosts
            .iter()
            .filter_map(|h| h.protocol_hint.as_deref())
            .chain(
                capture
                    .native
                    .iter()
                    .filter_map(|n| n.protocol_hint.as_deref()),
            );
        for hint in hints {
            assert!(
                said.contains(hint),
                "{id}: `{hint}` is not reported: {said}"
            );
        }
    }
}
