//! The shared test surface for `goalcup-cli`; a test file pulls it in with `mod support;`.
//!
//! A helper written twice under two names is two tests that agree today and drift tomorrow,
//! so a helper a second call site needs is added here rather than at the call site. A name
//! keeps its signature; a body may grow, and the item's own documentation says why.
//!
//! Rules the items follow:
//!
//! * A helper does what its signature says. There is no `mock(info)` here: `MockDaemonClient`
//!   builds its own `WorkspaceInfo`, and one that took an `info` and ignored it would lie
//!   about what the mock hands back.
//! * A fixture never runs a real harness. The shipped recipes launch `claude` and `codex`,
//!   which hang on a machine that has one installed and fail on one that has not, so a
//!   rewritten recipe runs `sh` or `true` instead, and
//!   [`clean_env_with_no_harness_installed`] keeps D12's version gate from measuring the
//!   developer's own harness.
//! * A rewritten connector no longer matches the manifest pin: `pin_state` calls it
//!   `Altered` and a start against it needs `--dev`. That is the door behaving as it
//!   should, not a weakening of the pin.
//! * Manifests are edited line by line, section by section, never through a TOML round
//!   trip, which would rewrite every comment in the shipped rulebook the fixtures keep.
//! * An environment starts empty ([`clean_env`] is `PATH` and nothing else) and a home is a
//!   throwaway directory, so no test inherits the developer's `GOALCUP_*` variables, harness
//!   home or credentials.
//! * A wait is on a condition, not a duration: [`PATIENCE`] is a ceiling reached only when
//!   the condition cannot come true, and a wait that gives up names what it waited for.
//!   [`never_appears`] is the one negative wait, and its window is a fixed three seconds.
//! * The interrupt flag is process-global, so the tests that meet it hold
//!   [`one_interrupt_at_a_time`].
//!
//! `dead_code` is allowed throughout: every test binary in this directory compiles all of
//! this file, and no single one uses all of it.

#![allow(dead_code)]

use goalcup_protocol::control;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// One clock for every render. A test that formatted `now` would pass on the day it was
/// written and be a different test tomorrow.
pub const NOW: &str = "2026-09-05T00:00:00Z";

/// The longest any wait in this module gives a condition to come true. Generous, because it
/// is only ever reached when the condition is not going to come true at all.
pub const PATIENCE: Duration = Duration::from_secs(20);

// ------------------------------------------------------------------ the tree

/// The workspace root: `CARGO_MANIFEST_DIR/../..`.
pub fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

/// The shipped `connectors/` — in a checkout, this repository's.
pub fn connectors_dir() -> PathBuf {
    repo_root().join("connectors")
}

/// The shipped connector `id`, loaded from [`connectors_dir`].
pub fn connector(id: &str) -> goalcup_cli::connector::Connector {
    match goalcup_cli::connector::load(&connectors_dir(), id) {
        Ok(c) => c,
        Err(e) => panic!("the shipped connector {id}: {e}"),
    }
}

/// `connectors/<id>/` copied to `<tmp>/<id>/`, whole — the manifest and the `files/` beside
/// it — so a test can edit a copy without touching the pinned original.
pub fn copy_connector(tmp: &tempfile::TempDir, id: &str) {
    copy_tree(&connectors_dir().join(id), &tmp.path().join(id));
}

/// Every shipped connector copied under `dir`, which is created if it is not there.
///
/// [`copy_connector`] puts a single connector at the root of a `TempDir`; the layouts
/// `config::connectors_dir` looks for relative to the executable need the whole set several
/// directories down.
pub fn copy_connectors_into(dir: &Path) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        panic!("{}: {e}", dir.display());
    }
    let shipped = connectors_dir();
    let entries = match std::fs::read_dir(&shipped) {
        Ok(entries) => entries,
        Err(e) => panic!("{}: {e}", shipped.display()),
    };
    for entry in entries.flatten() {
        if entry.path().is_dir()
            && let Some(name) = entry.path().file_name()
        {
            copy_tree(&entry.path(), &dir.join(name));
        }
    }
}

/// A connectors directory holding one connector whose manifest is **not** the one this
/// build pins: `connectors/<id>/`, copied, with a comment appended.
///
/// A comment and not a value, deliberately. What the release-hash allowlist compares is the
/// file's bytes, so the smallest possible edit is the honest fixture: it proves the digest
/// is over the file rather than over a re-serialisation of the parsed manifest, and it
/// leaves a manifest that still parses and still passes `validate()` — which is exactly the
/// case a participant produces, and exactly the case that would otherwise pass unnoticed.
///
/// Leaked like [`tmp_cfg`], so the caller gets a `PathBuf` without holding a guard.
pub fn connectors_dir_with_edited_manifest(id: &str) -> PathBuf {
    let root = empty_cfg();
    copy_tree(&connectors_dir().join(id), &root.join(id));
    let manifest = root.join(id).join("manifest.toml");
    let text = match std::fs::read_to_string(&manifest) {
        Ok(text) => text,
        Err(e) => panic!("{}: {e}", manifest.display()),
    };
    let edited = format!("{text}\n# edited by a test: this manifest is no longer the pinned one\n");
    if let Err(e) = std::fs::write(&manifest, edited) {
        panic!("{}: {e}", manifest.display());
    }
    root
}

/// A connectors directory holding one connector whose **launch recipe** runs a shell that
/// writes `proof.txt` into the workspace and exits, instead of the harness itself.
///
/// `goalcup start` in launch mode spawns the recipe and waits for it,
/// and the shipped recipes spawn `claude` and `codex`: a test that ran one would
/// either hang on a machine where the harness is installed or fail on a machine where it is
/// not, and neither outcome says anything about the command under test. The edit is scoped to
/// the `[launch]` table — `[resume]` keeps the shipped program and arguments — so the rest of
/// the manifest, `[preflight]` and `[arming]` included, is the shipped rulebook.
pub fn connectors_dir_with_a_harness_that_exits(id: &str) -> PathBuf {
    // The rendered argv and the rendered `[launch.env]` are both written to `proof.txt`,
    // rather than a fixed `ok`, because what the command-level test needs to know is that
    // what the plan built is what the child received. `$@` is the argv after the script —
    // the connector's own `{artifact_root}` and `{session_id}` templates, kept here so the
    // rendering is the shipped one — and `$CLAUDE_CONFIG_DIR` is the pin `[launch.env]` sets
    // from `{harness_home}`. A fixture that only proved a process ran would pass just as
    // happily on a launch that rendered nothing.
    connectors_dir_with_a_rewritten_launch(
        id,
        "sh",
        Some(
            r#"["-c", "echo \"$CLAUDE_CONFIG_DIR\" > proof.txt; echo \"$@\" >> proof.txt", "dump", "--settings", "{artifact_root}/settings-launch.json", "--session-id", "{session_id}"]"#,
        ),
        None,
    )
}

/// A connectors directory whose launch recipe **sleeps** rather than exiting.
///
/// The abandon path kills the child it spawned, and a
/// fixture that exits by itself cannot tell a kill that worked from a kill that never
/// happened: the process is gone either way. This one is still running when the refusal
/// arrives, so "the pid is gone afterwards" is a claim about the CLI.
pub fn connectors_dir_with_a_harness_that_sleeps(id: &str) -> PathBuf {
    connectors_dir_with_a_rewritten_launch(id, "sh", Some(r#"["-c", "sleep 300"]"#), None)
}

/// A connectors directory whose launch recipe owns a process group and forks into it.
///
/// `LaunchMode::Server` is the one mode `spawn` calls
/// `process_group(0)` for, so it is the only way to get a handle carrying a real `pgid` —
/// and the recipe backgrounds a `sleep` and writes its pid to `pid.txt` in the workspace, so
/// a test can ask whether the abandon reached the whole group or only the leader. `wait`
/// keeps the leader alive until it is signalled.
pub fn connectors_dir_with_a_group_owning_harness(id: &str) -> PathBuf {
    connectors_dir_with_a_rewritten_launch(
        id,
        "sh",
        Some(r#"["-c", "sleep 300 & printf %s $! > pid.txt; wait"]"#),
        Some("server"),
    )
}

/// A TUI wrapper with no private process group that forks the manifest-pinned native
/// process. The pid file lets a refusal test wait until the native child definitely exists.
pub fn connectors_dir_with_a_tui_native_child(id: &str) -> PathBuf {
    let root = connectors_dir_with_a_rewritten_launch(
        id,
        "sh",
        Some(r#"["-c", "sleep 300 & printf %s $! > pid.txt; wait"]"#),
        Some("tui"),
    );
    let manifest = root.join(id).join("manifest.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let text = text.replace("native_binary = \"codex\"", "native_binary = \"sleep\"");
    std::fs::write(&manifest, text).unwrap();
    root
}

/// A group-less TUI whose native child exists immediately but never acquires its
/// manifest-pinned `comm` while the test runs, so launch-time discovery misses it however
/// long the bounded window turns out to be. The child holds its `exec` until a file named
/// `release-native` appears in its working directory, which nothing creates (the 300 s
/// poll limit only keeps a leaked fixture from living forever): no timing guess decides
/// whether discovery sees `perl` or `sleep`. The pid file makes the undiscovered survivor
/// observable without relying on timing after the command returns.
pub fn connectors_dir_with_an_unresolved_tui_native_child(id: &str) -> PathBuf {
    let root = connectors_dir_with_a_rewritten_launch(
        id,
        "sh",
        Some(
            r#"["-c", "/usr/bin/perl -e 'open(F,qq(>pid.txt)); print F $$; close F; select undef,undef,undef,0.02 until -e qq(release-native) or ++$n > 15000; exec qq(/bin/sleep),qq(300)' & wait"]"#,
        ),
        Some("tui"),
    );
    let manifest = root.join(id).join("manifest.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let text = text.replace("native_binary = \"codex\"", "native_binary = \"sleep\"");
    std::fs::write(&manifest, text).unwrap();
    root
}

/// The resume-side form of [`connectors_dir_with_an_unresolved_tui_native_child`].
/// `[launch]` still selects the group-less `sh` program and `[resume]` supplies the held
/// exec recipe, so the process exists throughout discovery without ever matching `sleep`.
pub fn connectors_dir_with_an_unresolved_tui_native_resume(id: &str) -> PathBuf {
    let root = connectors_dir_with_an_unresolved_tui_native_child(id);
    let manifest = root.join(id).join("manifest.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let mut section = String::new();
    let mut out = String::new();
    let mut dropping_the_rest_of_args = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if dropping_the_rest_of_args {
            dropping_the_rest_of_args = !trimmed.ends_with(']');
            continue;
        }
        if trimmed.starts_with('[') {
            section = trimmed.to_string();
        }
        if section == "[resume]" && trimmed.starts_with("args =") {
            out.push_str(
                r#"args = ["-c", "/usr/bin/perl -e 'open(F,qq(>pid.txt)); print F $$; close F; select undef,undef,undef,0.02 until -e qq(release-native) or ++$n > 15000; exec qq(/bin/sleep),qq(300)' & wait"]"#,
            );
            out.push('\n');
            dropping_the_rest_of_args = !trimmed.ends_with(']');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    std::fs::write(&manifest, out).unwrap();
    root
}

/// A group-less native-pid plan whose bare program cannot resolve, so `spawn` returns
/// before a `Child` exists. Both launch and resume use `[launch].program`, making this the
/// exact fixture for the speculative pre-spawn cleanup guard.
pub fn connectors_dir_with_an_unspawnable_tui_native(id: &str) -> PathBuf {
    connectors_dir_with_a_rewritten_launch(
        id,
        "goalcup-test-program-that-does-not-exist",
        None,
        Some("tui"),
    )
}

/// A connectors directory whose **resume** recipe is the shipped one, run by a program that
/// exits at once.
///
/// `goalcup resume` relaunches from `[resume] args` and waits for what it spawned, and the
/// shipped recipes spawn `claude` and `codex` — a test that ran one would hang on a machine
/// where the harness is installed and fail on a machine where it is not.
///
/// **Only `[launch] program` is rewritten, and `[resume] args` is left exactly as shipped**,
/// which is the point of this fixture rather than a fourth `sh -c`: `launch::plan` takes the
/// program from `[launch]` and the arguments from `[resume]`, so the argv under test is the
/// shipped rulebook's and the thing running it is not a harness. `true` and not `sh`,
/// because `sh --settings … --resume …` writes a usage error into the terminal the test is
/// running in, and `true` ignores every argument and exits 0.
pub fn connectors_dir_with_a_resume_that_exits(id: &str) -> PathBuf {
    connectors_dir_with_a_rewritten_launch(id, "true", None, None)
}

/// A connectors directory whose `[launch.env]` pins one extra variable.
///
/// `launch::plan` refuses a manifest that pins a name
/// D23's denylist covers, and `--dev` does not open that door — a fact only a manifest this
/// build does not pin can express, because no shipped one pins such a name and this module's
/// other fixtures rewrite `[launch] program` rather than `[launch.env]`.
///
/// Appended after the `[launch.env]` header, line by line, for the reason
/// [`rewrite_launch_into`] gives: a TOML round trip would rewrite every comment in the
/// shipped rulebook.
pub fn connectors_dir_with_launch_env(id: &str, key: &str, value: &str) -> PathBuf {
    let root = empty_cfg();
    copy_tree(&connectors_dir().join(id), &root.join(id));
    let manifest = root.join(id).join("manifest.toml");
    let text = match std::fs::read_to_string(&manifest) {
        Ok(text) => text,
        Err(e) => panic!("{}: {e}", manifest.display()),
    };
    let mut out = String::new();
    let mut pinned = false;
    for line in text.lines() {
        out.push_str(line);
        out.push('\n');
        if line.trim() == "[launch.env]" {
            out.push_str(&format!("{key} = \"{value}\"\n"));
            pinned = true;
        }
    }
    assert!(pinned, "`{id}` has no [launch.env] section to pin into");
    if let Err(e) = std::fs::write(&manifest, out) {
        panic!("{}: {e}", manifest.display());
    }
    root
}

/// The one rewriter behind the fixtures above: `[launch] program` becomes `program`,
/// `args` becomes `args_toml` where one is given, and `mode` becomes `mode` where one is
/// named.
///
/// `args_toml` is `None` for a fixture that wants the shipped arguments run by a different
/// program — [`connectors_dir_with_a_resume_that_exits`], whose whole subject is the argv it
/// leaves alone.
///
/// Section-scoped, line by line, so a `program` or an `args` under `[resume]` is left alone
/// — and `[launch.status_line]` is its own section name, so it is left alone too. A TOML
/// round trip through `ConnectorManifest` would rewrite every comment in the shipped file,
/// which is the rulebook these fixtures are trying to keep.
fn connectors_dir_with_a_rewritten_launch(
    id: &str,
    program: &str,
    args_toml: Option<&str>,
    mode: Option<&str>,
) -> PathBuf {
    let root = empty_cfg();
    rewrite_launch_into(&root, id, program, args_toml, mode);
    root
}

/// The same rewrite, into a connectors directory the caller names.
///
/// [`connector_with_fake_harness`] takes the test's own `TempDir`, so its connectors directory
/// lives beside the configuration directory rather than in a leaked one of its own: one
/// directory for one chained run. A second copy of the rewriter is the drift this module
/// exists to prevent.
fn rewrite_launch_into(
    root: &Path,
    id: &str,
    program: &str,
    args_toml: Option<&str>,
    mode: Option<&str>,
) {
    copy_tree(&connectors_dir().join(id), &root.join(id));
    let manifest = root.join(id).join("manifest.toml");
    let text = match std::fs::read_to_string(&manifest) {
        Ok(text) => text,
        Err(e) => panic!("{}: {e}", manifest.display()),
    };
    let mut section = String::new();
    let mut out = String::new();
    let mut dropping_the_rest_of_args = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if dropping_the_rest_of_args {
            dropping_the_rest_of_args = !trimmed.ends_with(']');
            continue;
        }
        if trimmed.starts_with('[') {
            section = trimmed.to_string();
        }
        if section == "[launch]" {
            if trimmed.starts_with("program =") {
                out.push_str(&format!("program = \"{program}\"\n"));
                continue;
            }
            if trimmed.starts_with("mode =")
                && let Some(mode) = mode
            {
                out.push_str(&format!("mode = \"{mode}\"\n"));
                continue;
            }
            if trimmed.starts_with("args =")
                && let Some(args_toml) = args_toml
            {
                out.push_str(&format!("args = {args_toml}\n"));
                // The shipped list may run over several lines; the rest of it goes too.
                dropping_the_rest_of_args = !trimmed.ends_with(']');
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if let Err(e) = std::fs::write(&manifest, out) {
        panic!("{}: {e}", manifest.display());
    }
}

/// A connectors directory under `tmp` whose `[launch] program` is this crate's
/// `tests/fixtures/fake-harness.sh`, with **every other line of the rulebook as shipped**.
///
/// Unlike [`connectors_dir_with_a_harness_that_exits`], which writes a file and stops, the
/// script leaves what the tracker's seams need something to read: a transcript where the
/// tailer looks for it and a file in the workspace for the seal to find. The script's own
/// header says what it is given and what it leaves.
///
/// **The edit is `sh` in front of both recipes and nothing else.** `ConnectorManifest`
/// refuses a path as `[launch] program` — it must be a bare name resolved on `PATH` at
/// preflight — so the program becomes `sh` and the script's absolute path is inserted as the
/// FIRST argument of `[launch] args` and of `[resume] args`. Every shipped argument stays,
/// in its shipped order, still templated: the argv the fixture parses is the argv a real
/// launch and a real resume render, with one path in front of it. That is what lets one
/// chained test drive `goalcup start` and then `goalcup resume` through the same script,
/// which is the whole reason `[resume]` is rewritten at all rather than left alone.
pub fn connector_with_fake_harness(id: &str, tmp: &tempfile::TempDir) -> PathBuf {
    let root = tmp.path().join("connectors");
    if let Err(e) = std::fs::create_dir_all(&root) {
        panic!("{}: {e}", root.display());
    }
    let script = fixture("fake-harness.sh");
    if !script.is_file() {
        panic!("{} is not there", script.display());
    }
    rewrite_launch_into(&root, id, "sh", None, None);
    prepend_to_both_recipes(&root.join(id).join("manifest.toml"), &script);
    root
}

/// `script` inserted as the first element of `[launch] args` and `[resume] args`, leaving
/// every shipped argument where it is.
///
/// Line by line and section-scoped, for the reason [`rewrite_launch_into`] gives: a TOML
/// round trip rewrites every comment in the file, and those comments are the rulebook these
/// fixtures are trying to keep. Both lists are one line in every shipped manifest, and a
/// list that is not is a panic rather than a silent half-edit.
fn prepend_to_both_recipes(manifest: &Path, script: &Path) {
    let text = match std::fs::read_to_string(manifest) {
        Ok(text) => text,
        Err(e) => panic!("{}: {e}", manifest.display()),
    };
    let mut section = String::new();
    let mut out = String::new();
    let mut edits = 0;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed.to_string();
        }
        if (section == "[launch]" || section == "[resume]") && trimmed.starts_with("args = [") {
            assert!(
                trimmed.ends_with(']'),
                "{}: {section} args runs over more than one line, which this fixture does \
                 not edit",
                manifest.display()
            );
            let rest = trimmed.trim_start_matches("args = [");
            let separator = if rest == "]" { "" } else { ", " };
            out.push_str(&format!(
                "args = [\"{}\"{separator}{rest}\n",
                script.display()
            ));
            edits += 1;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    assert_eq!(
        edits,
        2,
        "{}: expected one `args` line under [launch] and one under [resume]",
        manifest.display()
    );
    if let Err(e) = std::fs::write(manifest, out) {
        panic!("{}: {e}", manifest.display());
    }
}

/// `from` copied to `to`, recursively, creating what it needs. File modes come along, which
/// is what keeps an executable shim executable.
fn copy_tree(from: &Path, to: &Path) {
    if let Err(e) = std::fs::create_dir_all(to) {
        panic!("{}: {e}", to.display());
    }
    let entries = match std::fs::read_dir(from) {
        Ok(entries) => entries,
        Err(e) => panic!("{}: {e}", from.display()),
    };
    for entry in entries.flatten() {
        let source = entry.path();
        let Some(name) = source.file_name() else {
            continue;
        };
        let target = to.join(name);
        if source.is_dir() {
            copy_tree(&source, &target);
        } else if let Err(e) = std::fs::copy(&source, &target) {
            panic!("{} -> {}: {e}", source.display(), target.display());
        }
    }
}

/// `tests/fixtures/<name>`, this crate's own.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// The verifier's own committed `crates/goalcup-verifier/tests/fixtures/<name>/`, read and
/// never written.
pub fn fixture_trace(name: &str) -> PathBuf {
    repo_root()
        .join("crates")
        .join("goalcup-verifier")
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Every file under `<repo_root>/<rel>`, recursively, sorted.
pub fn walk(rel: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(&repo_root().join(rel), &mut out);
    out.sort();
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Every file under `root`, as workspace-relative strings, sorted.
///
/// A path outside the workspace is kept whole rather than dropped: a helper that silently
/// omitted a file would turn "this directory holds exactly these" into a claim about a
/// subset.
pub fn relative_files(root: &Path) -> Vec<String> {
    let base = repo_root();
    let mut out = Vec::new();
    collect(root, &mut out);
    let mut named: Vec<String> = out
        .iter()
        .map(|p| {
            p.strip_prefix(&base)
                .unwrap_or(p.as_path())
                .display()
                .to_string()
        })
        .collect();
    named.sort();
    named
}

// ------------------------------------------------------------------ config dirs

/// A configuration directory that outlives the test: a `TempDir` deliberately leaked, so a
/// helper can hand back a `PathBuf` without the caller having to keep a guard alive for it.
///
/// Leaked and not cleaned, because the alternative is a directory removed the moment this
/// function returns and a test that reads an empty path for reasons nothing in it explains.
/// The operating system's temporary directory is where it lands.
pub fn tmp_cfg() -> PathBuf {
    let dir = match tempfile::tempdir() {
        Ok(dir) => dir.keep(),
        Err(e) => panic!("a temporary configuration directory: {e}"),
    };
    if let Err(e) = std::fs::create_dir_all(dir.join("entries")) {
        panic!("the entries directory under {}: {e}", dir.display());
    }
    dir
}

/// [`tmp_cfg`] without the `entries/` directory: a configuration directory a `goalcup
/// start` has never run against.
pub fn empty_cfg() -> PathBuf {
    match tempfile::tempdir() {
        Ok(dir) => dir.keep(),
        Err(e) => panic!("a temporary configuration directory: {e}"),
    }
}

/// [`clean_env`] with a `PATH` no harness is on.
///
/// D12's version gate measures the harness **on the machine the test
/// is running on**: a developer with `codex` at 0.143.0 and a connector tested at `=0.153.0`
/// fails every command test that loads that connector, and the failure is about their
/// laptop rather than about the code. A binary that is not on `PATH` at all is the case the
/// gate answers with a `Warn` — "a machine without the harness installed is a report, not a
/// broken tracker" — so that is the case these tests ask for, deliberately and in one place.
///
/// It does not weaken the launch: `launch::spawn` resolves its program on **this process's**
/// `PATH`, not on the map handed to `start_with`, so a test whose recipe runs `sh` still
/// runs `sh`. The gate itself is tested where it belongs, against synthetic manifests, in
/// `tests/preflight_doctor.rs`.
pub fn clean_env_with_no_harness_installed() -> BTreeMap<String, String> {
    let mut env = clean_env();
    env.insert("PATH".to_string(), "/nonexistent".to_string());
    env
}

/// `PATH` and nothing else, ever. A child launched with the real environment inherits the
/// developer's `GOALCUP_*` variables, their harness home and their credentials, and a test
/// that passed because of one of those is a test about the machine it ran on.
pub fn clean_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string());
    env.insert("PATH".to_string(), path);
    env
}

/// The document a signed-in participant's `~/.claude.json` looks like to
/// `connectors/claude-code-cli/manifest.toml`'s `[home_seed]` — one place, so a key the
/// manifest adds is added once and every test that seeds a home gets it.
///
/// It carries **every name on that `keys` list**, in the manifest's own order, plus the
/// `trust_keys` name on a project entry of its own. Every value is invented: nothing here is
/// an account, a token or a path anybody has. What the seed does with them is the shape rule
/// in `goalcup_protocol::events::SeededKey::new` — booleans, numbers and short strings are
/// recorded with their value, and `oauthAccount`, an object, is recorded by name only — so
/// the shapes matter as much as the presence and are chosen to cover both sides of it.
///
/// `projects` is deliberately NOT on the manifest's `keys` list: it is here so a test can
/// assert the seed copies the list it was given and not everything it finds, and the
/// `hasTrustDialogAccepted` inside it is the `trust_keys` name carried by a directory that
/// is not this run's. The entry the seeding adds for THIS run's workspace is the tracker's
/// own decision rather than a copy, and the chain record keeps it on its own `written` list.
pub fn signed_in_claude_json() -> serde_json::Value {
    serde_json::json!({
        "hasCompletedOnboarding": true,
        "lastOnboardingVersion": "2.1.259",
        "theme": "dark",
        "oauthAccount": {
            "accountUuid": "00000000-0000-4000-8000-000000000000",
            "emailAddress": "participant@example.test",
            "organizationName": "Example Test Org",
        },
        "userID": "test-user",
        "firstStartTime": "2026-01-14T09:12:03.000Z",
        "firstStartVersion": "2.0.117",
        "installMethod": "native",
        "autoUpdates": true,
        "numStartups": 7,
        // The participant's answers to the two prompts a fresh `CLAUDE_CONFIG_DIR` shows once
        // onboarding and login are past: the fullscreen-renderer upsell and the Claude in Chrome
        // one.
        "fullscreenUpsellSeenCount": 3,
        "hasCompletedClaudeInChromeOnboarding": true,
        "claudeInChromeDefaultEnabled": true,
        "cachedChromeExtensionInstalled": true,
        // Deliberately here and NOT in `[home_seed] keys`: what the seed writes is the
        // manifest's list, and not everything the source file happens to hold.
        "projects": {
            "/somewhere/else": { "allowedTools": [], "hasTrustDialogAccepted": true },
        },
    })
}

/// A fake participant home whose `.claude.json` carries every key
/// `connectors/claude-code-cli/manifest.toml`'s `[home_seed]` requires.
///
/// The values are invented and only their PRESENCE matters: `goalcup start` refuses when
/// `hasCompletedOnboarding` or `oauthAccount` is absent, because a per-run configuration
/// directory without them meets a theme picker and then a login screen. A test that pointed
/// `$HOME` at the developer's own home would read their real account file, so this writes a
/// throwaway one under the test's temporary directory instead.
///
/// Returns the home. Give it to [`clean_env_with_home`] and hand that to `start_with`.
pub fn home_with_a_signed_in_harness(tmp: &Path) -> PathBuf {
    let home = tmp.join("participant-home");
    std::fs::create_dir_all(&home).unwrap();
    write_signed_in_claude_json(&home);
    home
}

/// [`signed_in_claude_json`] written into `home/.claude.json`, the first candidate on
/// `[home_seed] from`.
///
/// Split out of [`home_with_a_signed_in_harness`] because the real-daemon tests do not get
/// to choose their home: [`goalcup`] pins `$HOME` to `<root>/home`, which is the same empty
/// directory [`spawn_daemon`] was configured with, and this is how that one is signed in.
pub fn write_signed_in_claude_json(home: &Path) {
    if let Err(e) = std::fs::create_dir_all(home) {
        panic!("{}: {e}", home.display());
    }
    let path = home.join(".claude.json");
    let text = signed_in_claude_json().to_string();
    if let Err(e) = std::fs::write(&path, text) {
        panic!("{}: {e}", path.display());
    }
}

/// Sign in the home the real-daemon end-to-end tests run under, and return it.
///
/// `<root>/home` is what [`goalcup`] exports as `$HOME` to every `goalcup` it spawns, so
/// this is the file `[home_seed]` reads. Without it `goalcup start` refuses — the run would
/// meet a theme picker and a login screen — and the entry it had already created is
/// abandoned.
///
/// It never touches the developer's own home. `<root>` is a `tempfile::TempDir`.
pub fn sign_in_the_run_home(root: &Path) -> PathBuf {
    let home = root.join("home");
    write_signed_in_claude_json(&home);
    home
}

/// [`clean_env_with_no_harness_installed`] with `$HOME` pointed at a home the test owns.
pub fn clean_env_with_home(home: &Path) -> BTreeMap<String, String> {
    let mut env = clean_env_with_no_harness_installed();
    env.insert("HOME".to_string(), home.display().to_string());
    env
}

/// The two above in one call, for a test that only needs a start of `claude-code-cli` to
/// reach the launch rather than to say anything about the seed.
pub fn clean_env_with_a_signed_in_home(tmp: &Path) -> BTreeMap<String, String> {
    clean_env_with_home(&home_with_a_signed_in_harness(tmp))
}

/// The environment a planned launch starts its child from: [`clean_env`], scrubbed.
///
/// The scrub is `launch::scrubbed_env` (D23's denylist), the same call `goalcup start` makes on the
/// participant's own environment — so a test plans against the shape a real launch has,
/// rather than against a map that skipped the step.
pub fn launch_env() -> BTreeMap<String, String> {
    goalcup_cli::launch::scrubbed_env(&clean_env())
}

// ------------------------------------------------------------------ templates

/// The `{name}` placeholders in `template`, in order of appearance, with duplicates kept.
///
/// A `{` with no `}` after it contributes nothing: an unterminated placeholder is not a
/// name, and reporting the rest of the string as one would make an assertion about
/// which names a recipe uses depend on where the template happened to end.
pub fn brace_tokens(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        match rest.find('}') {
            Some(close) => {
                out.push(rest[..close].to_string());
                rest = &rest[close + 1..];
            }
            None => break,
        }
    }
    out
}

// ------------------------------------------------------------------ control documents

/// The options `goalcup start` sends, with a grant that is obviously not a real one.
pub fn workspace_opts() -> control::WorkspaceOpts {
    control::WorkspaceOpts {
        connector_id: "claude-code-cli".to_string(),
        manifest_sha256: "11".repeat(32),
        workspace_parent: None,
        attach: false,
        deadline_hours: 168,
        grant_token: "test-grant".to_string(),
        arming_timeout_seconds: None,
        harness_version: None,
    }
}

/// The port the fixture's daemon reserved, LEARNED from the operating system rather than
/// named.
///
/// `goalcup start` runs `preflight::server_port_gate` over this number, which BINDS the port
/// it is asked about — so a fixed number fails every start test carrying this fixture on a
/// machine where anything at all holds it. A learned port is free by construction.
///
/// One per process rather than one per call, so the `WorkspaceInfo` a test builds twice
/// describes one reservation.
pub fn spike_server_port() -> u16 {
    static PORT: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
    *PORT.get_or_init(a_free_port)
}

/// A `WorkspaceInfo` with every field set, its four paths under one leaked `TempDir`, so a
/// command under test can actually open what it is told about.
///
/// Three fields are filled in rather than left `None` or empty, because each left an
/// assertion vacuous:
///
/// * `server_port` is `Some(spike_server_port())`. A launch and a resume "agreeing" about
///   `{server_port}` while it was `None` were agreeing about a pair of empty strings, while a
///   real resume rendered `--port ""`. A test that wants the missing-port case sets it back
///   to `None` itself, in one line, at the call site that is about it.
/// * `installed_files` is this fixture's two-file installation answer: the artifact-root
///   `settings-launch.json` the launch recipe passes to `--settings`, and the workspace
///   `.claude/settings.json` beside it. The transcript-only harness does not install hook
///   receivers; these entries exercise the daemon's signed installation metadata shape.
/// * `workspace_relative` is the `Workspace`-scoped subset of that list, sorted — derived
///   here the way `goalcup_daemon::workspace::Installation::from_files` derives it, because
///   a fixture whose two fields disagreed would be one no daemon could ever answer with.
pub fn workspace_info() -> control::WorkspaceInfo {
    use goalcup_protocol::manifest::InstallScope;

    let root = empty_cfg();
    let workspace = root.join("workspace");
    let artifact_root = root.join("artifacts");
    let harness_home = root.join("home");
    for dir in [&workspace, &artifact_root, &harness_home] {
        if let Err(e) = std::fs::create_dir_all(dir) {
            panic!("{}: {e}", dir.display());
        }
    }
    let installed_files = vec![
        control::InstalledFile {
            path: artifact_root
                .join("settings-launch.json")
                .display()
                .to_string(),
            scope: InstallScope::ArtifactRoot,
            relative_path: "settings-launch.json".to_string(),
            sha256: "33".repeat(32),
            bytes: 512,
        },
        control::InstalledFile {
            path: workspace
                .join(".claude")
                .join("settings.json")
                .display()
                .to_string(),
            scope: InstallScope::Workspace,
            relative_path: ".claude/settings.json".to_string(),
            sha256: "44".repeat(32),
            bytes: 256,
        },
    ];
    let mut workspace_relative: Vec<String> = installed_files
        .iter()
        .filter(|f| f.scope == InstallScope::Workspace)
        .map(|f| f.relative_path.clone())
        .collect();
    workspace_relative.sort();
    control::WorkspaceInfo {
        entry_id: "entry-0000".to_string(),
        process_generation: 1,
        workspace: workspace.display().to_string(),
        // Same string as `workspace`: the fixture names a directory it made under the
        // caller's `tmp`, so there is nothing for a canonicalisation to change.
        workspace_requested: workspace.display().to_string(),
        artifact_root: artifact_root.display().to_string(),
        harness_home: harness_home.display().to_string(),
        run_public_key: "22".repeat(32),
        // The PAGE with the id in the query, which is what `control/handlers.rs` builds:
        // `/v0/entries/current` is authenticated and a browser has no token, so the page is
        // told which entry it is looking at. `goalcup ui` opens this string byte for byte.
        status_url: format!(
            "http://127.0.0.1:53310{}?entry=entry-0000",
            control::ROUTE_UI
        ),
        status_cache_path: artifact_root.join("status.json").display().to_string(),
        status_line_path: artifact_root.join("status-line.txt").display().to_string(),
        session_id: "session-0000".to_string(),
        server_port: Some(spike_server_port()),
        installed_files,
        workspace_relative,
        created_at: NOW.to_string(),
    }
}

// ------------------------------------------------------------------ a start, mocked

/// A mock holding one armed entry for `id`, and `tmp` made into a configuration directory a
/// `goalcup start` can run against: a `grant.json` at `0600` and an `entries/` beside it.
///
/// The grant is written because `start_with`'s first refusal is a machine that has never run
/// `goalcup auth`, and every caller of this helper hands `tmp.path()` over as the
/// configuration directory. It is an obviously fake grant — the token says so — and it is
/// written through `config::write_secret_file`, which is the same path `goalcup auth` takes,
/// so a test that asserts nothing world-readable was written sees the real thing.
pub fn mock_armed(tmp: &tempfile::TempDir, id: &str) -> goalcup_cli::daemon::MockDaemonClient {
    write_grant(tmp.path());
    goalcup_cli::daemon::MockDaemonClient::armed_ok(tmp.path(), id)
}

/// The bearer [`write_grant`] puts on disk, named so a test can grep a stream for it.
///
/// One spelling, in the place that writes it, rather than the literal at each call site.
pub const GRANT_TOKEN: &str = "test-grant";

/// The grant `mock_armed` leaves behind, as `goalcup auth` would have written it.
///
/// `goalcup start`'s first refusal is a machine that has never run `goalcup auth`, and a
/// test that drives the `goalcup` BINARY against a real daemon needs the same grant
/// `mock_armed` writes, without a mock beside it.
pub fn write_grant(cfg: &Path) {
    // The document `goalcup auth` writes (`commands::auth::Grant`, `v1-compat` only), spelled
    // out so this helper builds in the default configuration too.
    let grant = serde_json::json!({
        "server": "https://example.invalid",
        "grant_token": GRANT_TOKEN,
        "subject": "a test",
        "email_verified": true,
        "scopes": ["entry:create"],
        "expires_at": "2027-01-01T00:00:00Z",
        "stored_at": NOW,
    });
    let bytes = match serde_json::to_vec_pretty(&grant) {
        Ok(bytes) => bytes,
        Err(e) => panic!("the test grant: {e}"),
    };
    if let Err(e) =
        goalcup_cli::config::write_secret_file(&goalcup_cli::config::grant_path(cfg), &bytes)
    {
        panic!("the test grant under {}: {e}", cfg.display());
    }
}

/// `tmp` as a configuration directory that already holds the record one `goalcup start`
/// would have left for `info`'s entry, and the path to it.
///
/// The pinned run key is what matters: `goalcup status` and `goalcup seal` are fresh processes,
/// the key they check a signed document against is the one `start` wrote down here, and a
/// test that handed them a configuration directory without this file would be testing the
/// refusal rather than the render.
///
/// `connector_id` is the one field of [`goalcup_cli::entries::EntryRecord`] that
/// `WorkspaceInfo` cannot answer for — the daemon does not echo it back on the workspace —
/// so it is written as the shipped id `mock_armed` is called with in every caller of this
/// helper. A caller that needs another id uses [`set_cached_connector`].
pub fn cfg_with_entry(tmp: &Path, info: &control::WorkspaceInfo) -> PathBuf {
    let record = goalcup_cli::entries::EntryRecord {
        entry_id: info.entry_id.clone(),
        process_generation: info.process_generation,
        pending_process_registration: None,
        pending_process_registration_origin: None,
        pending_process_cleanup: None,
        pending_process_cleanup_origin: None,
        pending_unresolved_native_cleanup: None,
        resume_recovery: None,
        connector_id: "claude-code-cli".to_string(),
        run_public_key: info.run_public_key.clone(),
        workspace: info.workspace.clone(),
        artifact_root: info.artifact_root.clone(),
        harness_home: info.harness_home.clone(),
        session_id: info.session_id.clone(),
        last_pid: None,
        workspace_relative: info.workspace_relative.clone(),
        status_url: info.status_url.clone(),
        created_at: info.created_at.clone(),
        dev: false,
    };
    if let Err(e) = goalcup_cli::entries::write_entry(tmp, &record) {
        panic!("the cached entry record under {}: {e}", tmp.display());
    }
    tmp.to_path_buf()
}

/// What one `goalcup start` was asked to do, with the two flags a test must never leave on.
///
/// `open_browser` is `false`: a test that opened the participant's browser would open the
/// developer's, and `--no-open` is the same door a headless machine goes through. `dev` is
/// `false`, so the default options are a judged run's — a test that wants the door open opens
/// it in one line at the call site that is about it. No `goal_file`, so the run is a legacy
/// one; a v5 test sets it.
pub fn start_options(id: &str) -> goalcup_cli::commands::start::StartOptions {
    goalcup_cli::commands::start::StartOptions {
        harness: id.to_string(),
        attach: false,
        print_launch: false,
        open_browser: false,
        arming_timeout: None,
        dev: false,
        goal_file: None,
        capture_reserve_bytes: goalcup_protocol::capture::DEFAULT_CAPTURE_RESERVE_BYTES,
        capture_claude_allowed_tool: None,
    }
}

/// A mock holding one resumable entry for `id`, with `tmp` made into a configuration
/// directory the way [`mock_armed`] makes one.
///
/// It differs from `mock_armed` in one thing: the **server port**. The daemon remembers the
/// port it reserved at creation and hands the same one back on a resume, and `armed_ok`
/// answers `None`, which lets a fixture "agree" about a pair of empty strings while a real
/// resume renders `--port ""`. So a connector whose `[resume]` recipe names `{server_port}`
/// gets one here and every other connector still gets `None`.
///
/// The number is **learned rather than named**. `preflight::server_port_gate` binds the port
/// it is asked about, so a fixed number would fail on a machine where something holds it.
/// [`a_free_port`] asks the operating system for a number nobody has.
pub fn mock_resumable(tmp: &tempfile::TempDir, id: &str) -> goalcup_cli::daemon::MockDaemonClient {
    let m = mock_armed(tmp, id);
    if reserves_a_port(id) {
        m.set_server_port(Some(a_free_port()));
    }
    // A resume catches the tail up before it answers, and a count of zero is a count a
    // test cannot tell from a field nobody set.
    m.set_caught_up_events(3);
    m
}

/// Whether connector `id`'s launch or resume recipe renders `{server_port}` — the same
/// question `launch::plan` asks, and the same one the daemon reserves on.
fn reserves_a_port(id: &str) -> bool {
    let m = connector(id).manifest;
    let names_it = |a: &Vec<String>| a.iter().any(|t| t.contains("{server_port}"));
    names_it(&m.launch.args) || names_it(&m.resume.args)
}

/// A loopback port nothing holds, learned by binding one and letting it go.
///
/// The listener is dropped before the number is returned, which is what makes it free rather
/// than held — the same question `preflight::port_is_free` asks, asked in the other
/// direction.
pub fn a_free_port() -> u16 {
    match std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()) {
        Ok(addr) => addr.port(),
        Err(e) => panic!("a loopback port to learn a free number from: {e}"),
    }
}

/// What one `goalcup resume` was asked to do, with the flag a test must never leave on.
///
/// `open_browser` is `false` for the reason [`start_options`] gives: a test that opened the
/// participant's browser would open the developer's.
#[cfg(feature = "v1-compat")]
pub fn resume_options() -> goalcup_cli::commands::resume::ResumeOptions {
    goalcup_cli::commands::resume::ResumeOptions {
        print_launch: false,
        open_browser: false,
    }
}

/// Rewrite `last_pid` on a cached entry record, so a test can put a live or a dead holder in
/// front of the pre-POST lock check.
///
/// It goes through [`goalcup_cli::entries::read_entry`] and `write_entry` rather than
/// editing the JSON, so a field added to `EntryRecord` later travels through this helper
/// untouched.
pub fn set_cached_last_pid(cfg: &Path, entry_id: &str, pid: u32) {
    let mut record = match goalcup_cli::entries::read_entry(cfg, entry_id) {
        Ok(record) => record,
        Err(e) => panic!(
            "the cached record for {entry_id} under {}: {e}",
            cfg.display()
        ),
    };
    record.last_pid = Some(pid);
    if let Err(e) = goalcup_cli::entries::write_entry(cfg, &record) {
        panic!("rewriting the cached record for {entry_id}: {e}");
    }
}

/// The same, for `connector_id`.
///
/// [`cfg_with_entry`] writes `claude-code-cli` because `WorkspaceInfo` carries no connector
/// id. `goalcup resume` reads the field — it is how the command knows which manifest holds
/// the `lock_paths[]` and the `[resume]` recipe — so the two-line rewrite is written once,
/// here, instead of at each call site.
pub fn set_cached_connector(cfg: &Path, entry_id: &str, connector_id: &str) {
    let mut record = match goalcup_cli::entries::read_entry(cfg, entry_id) {
        Ok(record) => record,
        Err(e) => panic!(
            "the cached record for {entry_id} under {}: {e}",
            cfg.display()
        ),
    };
    record.connector_id = connector_id.to_string();
    if let Err(e) = goalcup_cli::entries::write_entry(cfg, &record) {
        panic!("rewriting the cached record for {entry_id}: {e}");
    }
}

/// The same, for `session_id` — the record as it stands from the FIRST resume onwards.
///
/// [`cfg_with_entry`] writes `WorkspaceInfo.session_id`, the id the daemon
/// MINTED at creation, and `goalcup resume` writes the daemon's BOUND id back at the end of
/// every relaunch. On `codex-cli` the harness mints its own thread, so the
/// two are different strings — which `MockDaemonClient`'s `bound_session_of` models on
/// purpose — and a `lock_paths[]` entry naming `{session_id}` renders one path before the
/// first resume and another after it. A test about a lock the harness is HOLDING is a test
/// about the second state, and it says so here rather than by writing a lock file at a name
/// no harness would make.
pub fn set_cached_session_id(cfg: &Path, entry_id: &str, session_id: &str) {
    let mut record = match goalcup_cli::entries::read_entry(cfg, entry_id) {
        Ok(record) => record,
        Err(e) => panic!(
            "the cached record for {entry_id} under {}: {e}",
            cfg.display()
        ),
    };
    record.session_id = session_id.to_string();
    if let Err(e) = goalcup_cli::entries::write_entry(cfg, &record) {
        panic!("rewriting the cached record for {entry_id}: {e}");
    }
}

/// A mock holding one entry that is ready to be sealed, with `tmp` made into a
/// configuration directory the way [`mock_armed`] makes one.
///
/// It adds **nothing to the fake**: `armed_ok` already answers the first seal and a
/// `conflict` on every one after it, and `make_sealed_manifest_differ_on` and
/// `seal_once_then_conflict` cover the rest. What it adds is on disk, because the seal's
/// subject is a workspace and a directory the daemon exported:
///
/// * **One file in the workspace**, `hello.py`. Exactly one, deliberately: the mock's sealed
///   manifest starts empty, so a test that wants a disagreement asks for one by name and
///   reads a report with that disagreement in it and nothing else.
/// * **`<artifact_root>/trace`**, holding the four documents `TraceExport` writes. `goalcup
///   seal` writes not one byte in there, and the only way to assert that is against a
///   directory that already has bytes in it.
pub fn mock_sealable(tmp: &tempfile::TempDir, id: &str) -> goalcup_cli::daemon::MockDaemonClient {
    use goalcup_protocol::export::{CHAIN_FILE, DESCRIPTOR_FILE, RECEIPT_FILE, SEAL_FILE};

    let m = mock_armed(tmp, id);
    let info = m.info();
    put(&Path::new(&info.workspace).join("hello.py"), b"print(1)\n");
    let trace = Path::new(&info.artifact_root).join("trace");
    put(&trace.join(CHAIN_FILE), b"{\"seq\":1}\n");
    put(&trace.join(RECEIPT_FILE), b"{}\n");
    put(&trace.join(SEAL_FILE), b"{}\n");
    put(&trace.join(DESCRIPTOR_FILE), b"entry_id = \"mock\"\n");
    m
}

/// `bytes` at `path`, with the directories above it, or a panic naming the path.
fn put(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        panic!("{}: {e}", parent.display());
    }
    if let Err(e) = std::fs::write(path, bytes) {
        panic!("{}: {e}", path.display());
    }
}

/// What the daemon answers a resume of `info`'s entry with: the same run, caught up, with
/// no process still holding it.
pub fn resume_info(info: &control::WorkspaceInfo) -> control::ResumeInfo {
    control::ResumeInfo {
        entry_id: info.entry_id.clone(),
        process_generation: info.process_generation.saturating_add(1),
        session_id: info.session_id.clone(),
        caught_up_events: 0,
        workspace: info.workspace.clone(),
        artifact_root: info.artifact_root.clone(),
        harness_home: info.harness_home.clone(),
        status_line_path: info.status_line_path.clone(),
        server_port: info.server_port,
        last_process: None,
    }
}

// ------------------------------------------------------------------ the binaries

/// The binary `name`, beside the one cargo built for this package.
///
/// `CARGO_BIN_EXE_goalcup` is the only variable cargo sets for this crate. The others are
/// built by `cargo test --workspace --offline` — the gate command — and land in the same
/// directory, so they are resolved by name there. A missing one fails loudly naming the
/// command that builds it rather than skipping silently.
fn sibling(name: &str) -> PathBuf {
    let me = PathBuf::from(env!("CARGO_BIN_EXE_goalcup"));
    let path = me.with_file_name(name);
    assert!(
        path.is_file(),
        "{} is not built. This test drives the shipped binaries, which \
         `cargo test --workspace --offline` builds; one crate at a time does not.",
        path.display()
    );
    path
}

/// The `goalcup-daemon` binary, checked once to have been built with this test crate's own
/// feature set.
///
/// Cargo links `target/debug/goalcup-daemon` from whichever build of it ran last, and a
/// `-p goalcup-cli` run never builds it. A `v1-compat` daemon beside default CLI tests would mask a
/// default-only regression, and a default daemon beside `v1-compat` tests would fail in a confusing
/// way, so a mismatch stops here and names the command that builds the right one.
pub fn daemon_binary() -> PathBuf {
    static CHECKED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    let path = sibling("goalcup-daemon");
    CHECKED.get_or_init(|| {
        let out = std::process::Command::new(&path)
            .arg("--version")
            .output()
            .unwrap_or_else(|e| panic!("{} did not run: {e}", path.display()));
        let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let built_v1 = said.contains("+v1-compat");
        let wants_v1 = cfg!(feature = "v1-compat");
        assert_eq!(
            built_v1,
            wants_v1,
            "{} reports {said:?}, but these tests are built {} the `v1-compat` feature. Build the \
             matching daemon first: `cargo build --workspace --offline{}`",
            path.display(),
            if wants_v1 { "with" } else { "without" },
            if wants_v1 {
                " --features v1-compat"
            } else {
                ""
            },
        );
    });
    path
}

/// The `goalcup-verify` binary.
pub fn verify_binary() -> PathBuf {
    sibling("goalcup-verify")
}

// ------------------------------------------------------------------ waits and pids

/// Wait until `path` exists, or fail naming it.
pub fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("{} never appeared within {PATIENCE:?}", path.display());
}

/// A pid that is certainly not running: one this process started and reaped.
///
/// Not a large constant, which is a guess about what the machine has not allocated yet, and
/// not this process's own pid minus one, which may well be a live sibling. The child is
/// `goalcup --version`, which opens nothing.
pub fn a_pid_that_is_certainly_dead() -> u32 {
    let mut child = match std::process::Command::new(env!("CARGO_BIN_EXE_goalcup"))
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => panic!("a short-lived child to take a dead pid from: {e}"),
    };
    let pid = child.id();
    if let Err(e) = child.wait() {
        panic!("reaping the short-lived child: {e}");
    }
    pid
}

/// `cargo tree -p <pkg> --offline`, so a task can assert what a crate does and does not
/// link rather than reading its manifest and hoping.
pub fn cargo_tree(pkg: &str) -> String {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = match std::process::Command::new(cargo)
        .args(["tree", "-p", pkg, "--offline"])
        .current_dir(repo_root())
        .output()
    {
        Ok(out) => out,
        Err(e) => panic!("`cargo tree -p {pkg} --offline`: {e}"),
    };
    assert!(
        out.status.success(),
        "`cargo tree -p {pkg} --offline` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

// ------------------------------------------------------------------ a daemon that records

/// A listener on the loopback that answers a fixed script and remembers what it was asked.
///
/// The bearer rule is a WIRE property — `MockDaemonClient` is in-process and has no headers
/// to hold — so the only honest place to assert "public routes carry no token and every
/// other route carries one" is against a real socket and a real `HttpDaemonClient`. This is
/// that socket.
///
/// It speaks the smallest HTTP/1.1 a `ureq` request needs: one request per connection,
/// `Content-Length` on the way out, `Connection: close` so nothing is pooled, and — the one
/// detail that is not cosmetic — **the whole request body is read before a response is
/// written**. A server that answers early leaves those bytes in its receive queue and its
/// close sends an RST, which ureq 2.12 meets with an `.expect()` inside the library
/// (`commands::auth` documents the same hazard from the other side).
pub struct RecordingDaemon {
    base: String,
    log: std::sync::Arc<std::sync::Mutex<Recorded>>,
}

/// What the listener has been asked so far.
#[derive(Default)]
struct Recorded {
    /// The raw request target — path AND query, exactly as it arrived — and the
    /// `Authorization` header, or `None` where the request carried none.
    last: Option<(String, Option<String>)>,
    last_method: Option<String>,
    last_body: Option<String>,
    attempts: usize,
}

/// What a [`RecordingDaemon`] answers with.
enum Answer {
    /// `200` and an empty JSON object. Typed calls fail to parse it and this is deliberate:
    /// the tests that use it assert what went OUT, and a fixture body per route would be a
    /// second copy of the daemon's own answers.
    Empty,
    /// The same refusal every time, in the contract's `ErrorBody` shape.
    Refusal {
        status: u16,
        code: String,
        message: String,
    },
    /// `409`/`busy` once, then [`Answer::Empty`] — the transient the daemon raises on every
    /// mutating route while an entry is mid-pump.
    BusyOnce,
    /// `200` and a JSON string of this many bytes, for the client's response cap.
    Oversized(usize),
    /// `200`, a `Content-Length` far larger than the bytes that follow it, and then the
    /// close — a daemon that stopped mid-answer.
    Truncated,
    /// `200` and a body that is not UTF-8.
    NonUtf8,
    /// One exact wire response, including deliberately missing or repeated version headers.
    Raw {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// A deliberately invalid 204 body that uses transfer framing and carries no
    /// `Content-Length`, for the client boundary that `ureq` normalizes away.
    RawChunked {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
}

/// One raw HTTP response assembled by the test listener. `None` deliberately omits
/// `Content-Length`, which is how the chunked-204 fixture reaches `ureq` unchanged.
type ScriptedResponse = (u16, Vec<(String, String)>, Vec<u8>, Option<usize>);

impl RecordingDaemon {
    /// `http://127.0.0.1:<port>`, with no trailing slash: a route is appended to it whole.
    pub fn base(&self) -> String {
        self.base.clone()
    }

    /// The raw target and the `Authorization` header of the last request served.
    pub fn last(&self) -> (String, Option<String>) {
        match self.log.lock() {
            Ok(log) => match &log.last {
                Some(last) => last.clone(),
                None => panic!("nothing has been asked of this recording daemon yet"),
            },
            Err(e) => panic!("the recording daemon's log: {e}"),
        }
    }

    /// The HTTP method of the last request, so a read-only control call cannot be tested as
    /// merely "a request with no body".
    pub fn last_method(&self) -> String {
        match self.log.lock() {
            Ok(log) => match &log.last_method {
                Some(method) => method.clone(),
                None => panic!("nothing has been asked of this recording daemon yet"),
            },
            Err(e) => panic!("the recording daemon's log: {e}"),
        }
    }

    /// The body of the last request, so a test can assert that `POST /v0/workspaces`
    /// carries a bare `WorkspaceOpts`.
    pub fn last_body(&self) -> Option<String> {
        match self.log.lock() {
            Ok(log) => log.last_body.clone(),
            Err(e) => panic!("the recording daemon's log: {e}"),
        }
    }

    /// Requests served, so a retry is counted rather than inferred.
    pub fn attempts(&self) -> usize {
        match self.log.lock() {
            Ok(log) => log.attempts,
            Err(e) => panic!("the recording daemon's log: {e}"),
        }
    }
}

/// A daemon that answers `200 {}` and remembers everything.
pub fn recording_daemon() -> RecordingDaemon {
    serve(Answer::Empty)
}

/// A daemon that answers this refusal, in the contract's `ErrorBody` shape, every time.
pub fn erroring_daemon(status: u16, code: &str, message: &str) -> RecordingDaemon {
    serve(Answer::Refusal {
        status,
        code: code.to_string(),
        message: message.to_string(),
    })
}

/// Answers `409`/`busy` once and `200` thereafter, for the retry.
pub fn recording_daemon_busy_once() -> RecordingDaemon {
    serve(Answer::BusyOnce)
}

/// A daemon whose `200` is larger than any answer a control route has.
///
/// The client refuses a response body over its cap rather than
/// reading one into memory whole, and that refusal has no other fixture. `bytes` is the
/// length of the JSON string it answers with, so a caller names a size on either side of
/// the cap and does not have to know how the body is framed.
pub fn oversized_daemon(bytes: usize) -> RecordingDaemon {
    serve(Answer::Oversized(bytes))
}

/// A daemon that promises a body and then stops partway through it.
///
/// It declares a `Content-Length` it does not send and
/// closes, which is the wire shape of a daemon killed mid-answer; the client must call that
/// `Malformed` and not `Unreachable`, because "the daemon is not reachable" is the sentence
/// a participant acts on by restarting a daemon that was in fact answering.
pub fn truncated_daemon() -> RecordingDaemon {
    serve(Answer::Truncated)
}

/// A daemon whose `200` is not UTF-8; see
/// [`truncated_daemon`] for why this is not an unreachable daemon either.
pub fn non_utf8_daemon() -> RecordingDaemon {
    serve(Answer::NonUtf8)
}

/// A daemon whose response is named byte-for-byte by a wire-contract test.
pub fn raw_response_daemon(
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> RecordingDaemon {
    serve(Answer::Raw {
        status,
        headers,
        body,
    })
}

/// A response whose bytes are chunked without a `Content-Length`.  This is a separate raw
/// fixture because the normal listener deliberately writes lengths for every ordinary test.
pub fn raw_chunked_response_daemon(
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> RecordingDaemon {
    serve(Answer::RawChunked {
        status,
        headers,
        body,
    })
}

/// One listener, one thread, one connection at a time.
fn serve(answer: Answer) -> RecordingDaemon {
    let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(e) => panic!("a loopback listener for a recording daemon: {e}"),
    };
    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(e) => panic!("the recording daemon's own address: {e}"),
    };
    let log = std::sync::Arc::new(std::sync::Mutex::new(Recorded::default()));
    let served = std::sync::Arc::clone(&log);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let Some((method, target, authorization, body)) = read_request(&mut stream) else {
                continue;
            };
            let count = match served.lock() {
                Ok(mut log) => {
                    log.attempts += 1;
                    log.last = Some((target, authorization));
                    log.last_method = Some(method);
                    log.last_body = body;
                    log.attempts
                }
                Err(_) => continue,
            };
            let (status, headers, body, declared) = scripted(&answer, count);
            let headers = headers
                .into_iter()
                .map(|(name, value)| format!("{name}: {value}\r\n"))
                .collect::<String>();
            let content_length = declared
                .map(|length| format!("Content-Length: {length}\r\n"))
                .unwrap_or_default();
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                 {headers}{content_length}Connection: close\r\n\r\n",
            );
            use std::io::Write;
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        }
    });
    RecordingDaemon {
        base: format!("http://127.0.0.1:{port}"),
        log,
    }
}

/// The answer for the `count`-th request served: the status, the bytes that go on the wire,
/// and an optional `Content-Length` declared for them.
///
/// The declared length is a separate number rather than `body.len()` because one fixture
/// exists to make the two disagree: [`Answer::Truncated`] promises a body it does not send.
fn scripted(answer: &Answer, count: usize) -> ScriptedResponse {
    let whole = |status: u16, body: Vec<u8>| {
        let declared = body.len();
        (
            status,
            vec![(
                "x-goalcup-control-version".to_string(),
                goalcup_protocol::control::CONTROL_API_VERSION.to_string(),
            )],
            body,
            Some(declared),
        )
    };
    match answer {
        Answer::Empty => whole(200, b"{}".to_vec()),
        Answer::Refusal {
            status,
            code,
            message,
        } => whole(*status, error_body(code, message).into_bytes()),
        Answer::BusyOnce if count == 1 => whole(
            409,
            error_body("busy", "this entry is held by a call already running").into_bytes(),
        ),
        Answer::BusyOnce => whole(200, b"{}".to_vec()),
        Answer::Oversized(bytes) => whole(200, format!("\"{}\"", "x".repeat(*bytes)).into_bytes()),
        // Eleven bytes of a body that says it is four kibibytes long, and then the close.
        Answer::Truncated => (
            200,
            vec![(
                "x-goalcup-control-version".to_string(),
                goalcup_protocol::control::CONTROL_API_VERSION.to_string(),
            )],
            b"{\"entries\":".to_vec(),
            Some(4096),
        ),
        // A quoted JSON string whose two middle bytes are not UTF-8 at all.
        Answer::NonUtf8 => whole(200, vec![b'"', 0xff, 0xfe, b'"']),
        Answer::Raw {
            status,
            headers,
            body,
        } => (*status, headers.clone(), body.clone(), Some(body.len())),
        Answer::RawChunked {
            status,
            headers,
            body,
        } => {
            let mut chunked = format!("{:X}\r\n", body.len()).into_bytes();
            chunked.extend_from_slice(body);
            chunked.extend_from_slice(b"\r\n0\r\n\r\n");
            let mut headers = headers.clone();
            headers.push(("Transfer-Encoding".to_string(), "chunked".to_string()));
            (*status, headers, chunked, None)
        }
    }
}

/// The contract's failure shape, built by hand so a test names the wire and not a builder.
fn error_body(code: &str, message: &str) -> String {
    let body = control::ErrorBody {
        error: control::ErrorDetail {
            code: code.to_string(),
            message: message.to_string(),
            detail: serde_json::Value::Null,
        },
    };
    match serde_json::to_string(&body) {
        Ok(text) => text,
        Err(e) => panic!("the contract's own error shape: {e}"),
    }
}

/// The request target, the `Authorization` header and the body, or `None` on a connection
/// that carried no request at all.
fn read_request(
    stream: &mut std::net::TcpStream,
) -> Option<(String, String, Option<String>, Option<String>)> {
    use std::io::Read;

    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    // Byte at a time to the end of the headers, so not one byte of the body is consumed
    // before its length is known. Kilobytes, once per request.
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return None,
            Ok(_) => head.push(byte[0]),
        }
    }
    let head = String::from_utf8_lossy(&head).to_string();
    let mut lines = head.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let mut authorization = None;
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("authorization") {
            authorization = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("content-length") {
            length = value.parse().unwrap_or(0);
        }
    }
    // Every byte of it, before a single byte of the response goes out.
    let mut body = vec![0u8; length];
    if length > 0 && stream.read_exact(&mut body).is_err() {
        return None;
    }
    let body = (length > 0).then(|| String::from_utf8_lossy(&body).to_string());
    Some((method, target, authorization, body))
}

// ------------------------------------------------- a mock whose handle is readable

/// [`MockDaemonClient`], with the [`ProcessHandle`] it was handed kept where a test can read
/// it — and, optionally, a file it waits for before it answers `register_process` at all.
///
/// A child the daemon **refused** to take still has to be cleaned up, and the mock's own
/// `last_registered_pid` reads `state.processes`, which a refused call never reaches, so
/// there is no way to name the pid the CLI then has to clean up. This wrapper records the
/// handle on the way past, before the answer, so a refusal is exactly as readable as an
/// acceptance.
///
/// [`Self::answer_register_only_after`] exists for the other half. A test about killing a
/// process TREE needs the tree to exist, and the CLI reaches `register_process` a millisecond
/// after the spawn — before a shell has forked anything. A daemon that takes a moment before
/// refusing is what a real one over a socket does anyway, and it makes the fixture's fork a
/// fact rather than a race.
///
/// It is a wrapper rather than a method on the mock: a fake that only tests need belongs
/// beside the tests.
pub struct WatchedDaemon<'a> {
    inner: &'a goalcup_cli::daemon::MockDaemonClient,
    seen: std::sync::Mutex<Vec<goalcup_protocol::process::ProcessHandle>>,
    created_at: std::sync::Mutex<Option<u64>>,
    wait_for: std::sync::Mutex<Option<PathBuf>>,
    /// A file created once the inner daemon has accepted `register_process`: the
    /// deterministic "the hand-off is over" signal for a harness that must act after it.
    touch_after_register: std::sync::Mutex<Option<PathBuf>>,
    /// One accepted request whose response the transport loses. The second identical
    /// request is the daemon's durable exact replay, not a second registration.
    lose_next_accepted_process_reply: std::sync::Mutex<Option<AcceptedProcessReplyLoss>>,
    /// While set, every exact replay of the accepted registration loses its response too.
    hold_accepted_process_reply_loss: std::sync::Mutex<bool>,
    accepted_lost_process: std::sync::Mutex<Option<goalcup_protocol::process::ProcessRegistration>>,
    /// The next `Malformed` refusal is a verified daemon-side 400 rather than a response
    /// the client failed to interpret after admission.
    framed_process_malformed_refusal: std::sync::Mutex<bool>,
    /// One-shot fault after the immutable body is already cached and before the mocked
    /// registration response returns: replace `entries/` with a file so the cleanup-state
    /// rewrite fails while the test can later restore and inspect the original pending row.
    break_entry_cache: std::sync::Mutex<Option<(PathBuf, PathBuf)>>,
}

#[derive(Clone, Copy)]
enum AcceptedProcessReplyLoss {
    Unreachable,
    Malformed,
}

impl AcceptedProcessReplyLoss {
    fn error(self) -> control::ControlError {
        match self {
            Self::Unreachable => control::ControlError::Unreachable(
                "the daemon accepted `/process` but its response was lost".to_string(),
            ),
            Self::Malformed => control::ControlError::Malformed(
                "the daemon accepted `/process` but sent a malformed response".to_string(),
            ),
        }
    }
}

/// `inner`, watched. Hand the result to `start_with` in place of the mock itself.
pub fn watched(inner: &goalcup_cli::daemon::MockDaemonClient) -> WatchedDaemon<'_> {
    WatchedDaemon {
        inner,
        seen: std::sync::Mutex::new(Vec::new()),
        created_at: std::sync::Mutex::new(None),
        wait_for: std::sync::Mutex::new(None),
        touch_after_register: std::sync::Mutex::new(None),
        lose_next_accepted_process_reply: std::sync::Mutex::new(None),
        hold_accepted_process_reply_loss: std::sync::Mutex::new(false),
        accepted_lost_process: std::sync::Mutex::new(None),
        framed_process_malformed_refusal: std::sync::Mutex::new(false),
        break_entry_cache: std::sync::Mutex::new(None),
    }
}

impl WatchedDaemon<'_> {
    /// Every `register_process` offer, including offers the daemon refused as Busy.
    pub fn registered_handles(&self) -> Vec<goalcup_protocol::process::ProcessHandle> {
        match self.seen.lock() {
            Ok(seen) => seen.clone(),
            Err(e) => panic!("the watched daemon's handles: {e}"),
        }
    }

    /// The handle of the last `register_process`, whatever the answer to it was.
    pub fn registered_handle(&self) -> Option<goalcup_protocol::process::ProcessHandle> {
        match self.seen.lock() {
            Ok(seen) => seen.last().cloned(),
            Err(e) => panic!("the watched daemon's handles: {e}"),
        }
    }

    /// The wall clock, in seconds, at the moment `create_workspace` answered — a timestamp
    /// taken after the workspace exists and before anything is spawned.
    pub fn workspace_created_at(&self) -> u64 {
        match self.created_at.lock() {
            Ok(at) => match *at {
                Some(at) => at,
                None => panic!("no workspace has been created through this daemon yet"),
            },
            Err(e) => panic!("the watched daemon's clock: {e}"),
        }
    }

    /// Wait for `path` to exist before answering `register_process`.
    pub fn answer_register_only_after(&self, path: &Path) {
        match self.wait_for.lock() {
            Ok(mut wait_for) => *wait_for = Some(path.to_path_buf()),
            Err(e) => panic!("the watched daemon's wait: {e}"),
        }
    }

    /// Create `path` as soon as the inner daemon has accepted `register_process`. A harness
    /// that polls for the file acts on the far side of the hand-off by construction, with no
    /// guess about how long the CLI's pre-registration work (native-process discovery under
    /// load, for one) takes.
    pub fn touch_after_register(&self, path: &Path) {
        match self.touch_after_register.lock() {
            Ok(mut touch) => *touch = Some(path.to_path_buf()),
            Err(e) => panic!("the watched daemon's touch: {e}"),
        }
    }

    /// Accept the next process registration in the inner daemon but report its response as
    /// unreachable.  A later exact body receives the cached success, modelling the server's
    /// durable `Registered` replay without teaching the general-purpose mock a transport.
    pub fn lose_next_accepted_process_reply(&self) {
        match self.lose_next_accepted_process_reply.lock() {
            Ok(mut lose) => *lose = Some(AcceptedProcessReplyLoss::Unreachable),
            Err(e) => panic!("the watched daemon's lost-reply flag: {e}"),
        }
    }

    /// Accept the next registration, then lose every exact replay response until released.
    pub fn lose_accepted_process_replies_until_released(&self) {
        self.hold_accepted_process_reply_loss(
            AcceptedProcessReplyLoss::Unreachable,
            "the watched daemon's lost-reply flag",
        );
    }

    /// Accept the next registration, then report every exact replay response as malformed
    /// until released. This models malformed headers and bodies after durable admission.
    pub fn malform_accepted_process_replies_until_released(&self) {
        self.hold_accepted_process_reply_loss(
            AcceptedProcessReplyLoss::Malformed,
            "the watched daemon's malformed-reply flag",
        );
    }

    /// Mark the next mocked malformed process refusal as a parsed daemon-side validation
    /// response.  The raw HTTP client test binds this semantic distinction to a framed 400;
    /// this wrapper lets command tests assert the corresponding child/cache lifecycle.
    pub fn frame_next_process_malformed_refusal(&self) {
        match self.framed_process_malformed_refusal.lock() {
            Ok(mut framed) => *framed = true,
            Err(e) => panic!("the watched daemon's framed-malformed flag: {e}"),
        }
    }

    pub fn break_entry_cache_after_register(&self, cfg: &Path) {
        let entries = cfg.join("entries");
        let held = cfg.join("entries-held-by-test");
        match self.break_entry_cache.lock() {
            Ok(mut fault) => *fault = Some((entries, held)),
            Err(e) => panic!("the watched daemon's cache fault: {e}"),
        }
    }

    /// Let the next exact replay receive the durable acknowledgement.
    pub fn release_accepted_process_reply(&self) {
        match self.hold_accepted_process_reply_loss.lock() {
            Ok(mut hold) => *hold = false,
            Err(e) => panic!("the watched daemon's held-reply flag: {e}"),
        }
        match self.lose_next_accepted_process_reply.lock() {
            Ok(mut loss) => *loss = None,
            Err(e) => panic!("the watched daemon's lost-reply flag: {e}"),
        }
    }

    fn hold_accepted_process_reply_loss(&self, loss: AcceptedProcessReplyLoss, label: &str) {
        match self.lose_next_accepted_process_reply.lock() {
            Ok(mut next) => *next = Some(loss),
            Err(e) => panic!("{label}: {e}"),
        }
        match self.hold_accepted_process_reply_loss.lock() {
            Ok(mut hold) => *hold = true,
            Err(e) => panic!("{label}: {e}"),
        }
    }
}

impl control::DaemonClient for WatchedDaemon<'_> {
    fn create_workspace(
        &self,
        harness: &str,
        opts: &control::WorkspaceOpts,
    ) -> Result<control::WorkspaceInfo, control::ControlError> {
        let info = self.inner.create_workspace(harness, opts);
        match self.created_at.lock() {
            Ok(mut at) => *at = Some(wall_clock_secs()),
            Err(e) => panic!("the watched daemon's clock: {e}"),
        }
        info
    }

    fn arm(&self, entry_id: &str) -> Result<(), control::ControlError> {
        self.inner.arm(entry_id)
    }

    fn wait_start_signal(
        &self,
        entry_id: &str,
        timeout: Duration,
    ) -> Result<control::StartSignal, control::ControlError> {
        self.inner.wait_start_signal(entry_id, timeout)
    }

    fn status(&self, entry_id: &str) -> Result<control::SignedStatus, control::ControlError> {
        self.inner.status(entry_id)
    }

    fn entries(&self) -> Result<Vec<control::EntrySummary>, control::ControlError> {
        self.inner.entries()
    }

    fn resume(
        &self,
        entry_id: &str,
        request: &control::ResumeRequest,
    ) -> Result<control::ResumeInfo, control::ControlError> {
        self.inner.resume(entry_id, request)
    }

    fn resume_preview(
        &self,
        entry_id: &str,
    ) -> Result<control::ResumePreviewInfo, control::ControlError> {
        self.inner.resume_preview(entry_id)
    }

    fn seal(&self, entry_id: &str) -> Result<control::SealResponse, control::ControlError> {
        self.inner.seal(entry_id)
    }

    fn register_process(
        &self,
        entry_id: &str,
        registration: &goalcup_protocol::process::ProcessRegistration,
    ) -> Result<(), control::ControlError> {
        // Recorded BEFORE the answer, because the case this exists for is the one where the
        // answer is a refusal.
        match self.seen.lock() {
            Ok(mut seen) => seen.push(registration.handle.clone()),
            Err(e) => panic!("the watched daemon's handles: {e}"),
        }
        let wait_for = match self.wait_for.lock() {
            Ok(wait_for) => wait_for.clone(),
            Err(e) => panic!("the watched daemon's wait: {e}"),
        };
        if let Some(path) = wait_for {
            wait_for_file(&path);
        }
        let cache_fault = match self.break_entry_cache.lock() {
            Ok(mut fault) => fault.take(),
            Err(e) => panic!("the watched daemon's cache fault: {e}"),
        };
        if let Some((entries, held)) = cache_fault {
            std::fs::rename(&entries, &held).expect("the test parks the readable entry cache");
            std::fs::write(&entries, b"not a directory")
                .expect("the test makes the cleanup rewrite fail");
        }
        {
            let accepted = match self.accepted_lost_process.lock() {
                Ok(accepted) => accepted.clone(),
                Err(e) => panic!("the watched daemon's accepted process: {e}"),
            };
            if accepted.as_ref() == Some(registration) {
                let hold = match self.hold_accepted_process_reply_loss.lock() {
                    Ok(hold) => *hold,
                    Err(e) => panic!("the watched daemon's held-reply flag: {e}"),
                };
                if hold {
                    let loss = match self.lose_next_accepted_process_reply.lock() {
                        Ok(loss) => *loss,
                        Err(e) => panic!("the watched daemon's lost-reply flag: {e}"),
                    };
                    return Err(loss
                        .expect("a held accepted reply has a named response loss")
                        .error());
                }
                return Ok(());
            }
        }
        self.inner.register_process(entry_id, registration)?;
        let touch = match self.touch_after_register.lock() {
            Ok(touch) => touch.clone(),
            Err(e) => panic!("the watched daemon's touch: {e}"),
        };
        if let Some(path) = touch {
            std::fs::write(&path, b"registered").expect("the signal that the hand-off is over");
        }
        let loss = match self.lose_next_accepted_process_reply.lock() {
            Ok(loss) => *loss,
            Err(e) => panic!("the watched daemon's lost-reply flag: {e}"),
        };
        if let Some(loss) = loss {
            match self.accepted_lost_process.lock() {
                Ok(mut accepted) => *accepted = Some(registration.clone()),
                Err(e) => panic!("the watched daemon's accepted process: {e}"),
            }
            let hold = match self.hold_accepted_process_reply_loss.lock() {
                Ok(hold) => *hold,
                Err(e) => panic!("the watched daemon's held-reply flag: {e}"),
            };
            if !hold {
                match self.lose_next_accepted_process_reply.lock() {
                    Ok(mut loss) => *loss = None,
                    Err(e) => panic!("the watched daemon's lost-reply flag: {e}"),
                }
            }
            return Err(loss.error());
        }
        Ok(())
    }

    fn register_process_with_admission(
        &self,
        entry_id: &str,
        registration: &goalcup_protocol::process::ProcessRegistration,
    ) -> Result<(), control::ProcessRegistrationError> {
        match self.register_process(entry_id, registration) {
            Err(error @ control::ControlError::Malformed(_)) => {
                let framed = match self.framed_process_malformed_refusal.lock() {
                    Ok(mut framed) => std::mem::take(&mut *framed),
                    Err(e) => panic!("the watched daemon's framed-malformed flag: {e}"),
                };
                if framed {
                    Err(control::ProcessRegistrationError::DefinitelyRefused(error))
                } else {
                    Err(control::ProcessRegistrationError::from_unclassified(error))
                }
            }
            Err(error) => Err(control::ProcessRegistrationError::from_unclassified(error)),
            Ok(()) => Ok(()),
        }
    }

    fn cancel_process_handoff(
        &self,
        entry_id: &str,
        request: &control::ProcessHandoffCancelRequest,
    ) -> Result<control::ProcessHandoffCancelInfo, control::ControlError> {
        self.inner.cancel_process_handoff(entry_id, request)
    }

    fn health(&self) -> Result<control::DaemonHealth, control::ControlError> {
        self.inner.health()
    }

    fn current_entry(&self) -> Result<Option<String>, control::ControlError> {
        self.inner.current_entry()
    }

    fn abandon(&self, entry_id: &str, reason: &str) -> Result<(), control::ControlError> {
        self.inner.abandon(entry_id, reason)
    }
}

/// The wall clock in seconds, the same reading `goalcup start` stamps a launch with.
pub fn wall_clock_secs() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(e) => panic!("a clock before the epoch: {e}"),
    }
}

/// Wait until `pid` is out of the process table, or fail naming it.
///
/// A zombie counts as gone — `process::is_alive` says so — because the caller that owns the
/// `Child` reaps it, and an orphaned grandchild is reaped by init a moment after it dies.
pub fn wait_until_gone(pid: u32) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if !goalcup_protocol::process::is_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("pid {pid} was still running {PATIENCE:?} after the command returned");
}

/// A connectors directory whose launch recipe raises `SIGINT` at **this process**, a beat
/// after it starts, and then exits.
///
/// A launched TUI shares the participant's terminal, so the Ctrl-C they type at their
/// harness reaches `goalcup start` as well — and the flag it sets must not turn an armed,
/// recorded run into "interrupted before the run began" and ask the daemon to abandon a
/// stamped entry.
///
/// Waiting for the hand-off is what puts the signal on its far side rather than in a race
/// with it. `$PPID` is the shell's own parent — this test binary — because a launch in `LaunchMode::Tui` is a
/// direct child, and the signal is raised at the pid rather than at the group so it reaches
/// the CLI whatever mode the shipped recipe declares.
///
/// Nothing here guesses a delay. The harness polls its working directory (the workspace)
/// for a `registered` file and raises the signal only once it exists. The test creates it
/// through [`WatchedDaemon::touch_after_register`], i.e. after the daemon has accepted
/// `register_process` however long the CLI's pre-registration work (including bounded native
/// identity discovery under load) took. The 20 s poll cap only stops a harness whose test
/// never wired the signal from living forever.
pub fn connectors_dir_with_a_harness_that_interrupts_its_parent(id: &str) -> PathBuf {
    connectors_dir_with_a_rewritten_launch(
        id,
        "sh",
        Some(
            r#"["-c", "i=0; while [ ! -e registered ] && [ $i -lt 400 ]; do sleep 0.05; i=$((i+1)); done; kill -INT $PPID"]"#,
        ),
        None,
    )
}

/// A connectors directory whose **resume** relaunch outlives the command that spawned it,
/// and the marker file it touches half a second in — returned together, because a test can
/// only ask "was the child stopped?" of a path it knows.
///
/// The tests that use it are about a child the daemon never accepted: a Ctrl-C in the
/// window before `register_process`, and a `register_process` that refuses. Neither can name
/// the child's pid — the first because no handle is ever offered, the second only through
/// [`watched`] — and [`connectors_dir_with_a_resume_that_exits`] cannot answer the question
/// at all, because a fixture that exits by itself is gone whether it was killed or not (the
/// reason [`connectors_dir_with_a_harness_that_sleeps`] gives).
///
/// So the fixture makes the *survival* observable rather than the death: it waits half a
/// second, touches the marker, and only then sleeps. A killed child never reaches the touch,
/// so an absent marker is a claim about the CLI and not about how fast the machine is.
/// `exec` on the final sleep so the process the CLI holds IS the sleeping one — a shell that
/// forked it would leave a `sleep 300` behind for five minutes after every run.
///
/// This is the ONE fixture in the family that rewrites `[resume] args`, and it has to.
/// `manifest::validate` refuses a `[launch] program` that is a path — "a bare program name
/// resolved on PATH at preflight" — so the sleeping program cannot be a script this test
/// wrote, and no bare name on every machine ignores an arbitrary argv and sleeps (`sleep`
/// wants a number, `sh` wants a script, `cat` wants files). So the program is `sh` and the
/// resume argv is `sh`'s. Nothing here asserts anything about that argv — the tests that are
/// about the shipped `[resume]` recipe use
/// [`connectors_dir_with_a_resume_that_exits`], which is the one that leaves it alone.
pub fn connectors_dir_with_a_resume_that_outlives_a_kill(id: &str) -> (PathBuf, PathBuf) {
    let survived = empty_cfg().join("survived.txt");
    let root = connectors_dir_with_a_rewritten_launch(id, "sh", None, None);
    let manifest = root.join(id).join("manifest.toml");
    let text = match std::fs::read_to_string(&manifest) {
        Ok(text) => text,
        Err(e) => panic!("{}: {e}", manifest.display()),
    };
    let mut section = String::new();
    let mut out = String::new();
    let mut dropping_the_rest_of_args = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if dropping_the_rest_of_args {
            dropping_the_rest_of_args = !trimmed.ends_with(']');
            continue;
        }
        if trimmed.starts_with('[') {
            section = trimmed.to_string();
        }
        if section == "[resume]" && trimmed.starts_with("args =") {
            out.push_str(&format!(
                "args = [\"-c\", \"sleep 0.5; : > {} ; exec sleep 300\"]\n",
                survived.display()
            ));
            dropping_the_rest_of_args = !trimmed.ends_with(']');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if let Err(e) = std::fs::write(&manifest, out) {
        panic!("{}: {e}", manifest.display());
    }
    (root, survived)
}

/// Fail if `path` ever appears — the negative half of [`wait_for_file`].
///
/// Three seconds and not [`PATIENCE`]: the marker
/// [`connectors_dir_with_a_resume_that_outlives_a_kill`] writes is touched half a second in,
/// so the window is six times the margin, and a negative assertion that waited twenty
/// seconds would be paid on every green run.
pub fn never_appears(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if path.exists() {
            panic!(
                "{} appeared: the relaunched child was left running",
                path.display()
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A binary-wide lock, held by every test that drives a command through the interrupt flag.
///
/// `arming::INTERRUPTED` is process-global — a signal
/// handler has nowhere else to put what it saw — and `cargo test` runs one binary's tests on
/// a pool of threads, so a test that raises `SIGINT` sets the flag its siblings read and
/// those siblings kill children nobody interrupted. `tests/start_interrupt.rs` answers this
/// by being a whole binary holding one test; a resume's interrupt window belongs beside the
/// rest of the resume tests, so this serialises them instead.
///
/// A poisoned lock is taken anyway: the panic that poisoned it has already failed its own
/// test, and turning every sibling into a second failure hides which one it was.
pub fn one_interrupt_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    match LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// The same, for `dev` — the flag `goalcup start --dev` leaves in the record.
///
/// `cfg_with_entry` writes `false`, which is the ordinary case. The seal prints
/// `goalcup-verify --dev` iff this is set, and a flag with only one reachable value is a flag
/// nothing checks.
pub fn set_cached_dev(cfg: &Path, entry_id: &str, dev: bool) {
    let mut record = match goalcup_cli::entries::read_entry(cfg, entry_id) {
        Ok(record) => record,
        Err(e) => panic!(
            "the cached record for {entry_id} under {}: {e}",
            cfg.display()
        ),
    };
    record.dev = dev;
    if let Err(e) = goalcup_cli::entries::write_entry(cfg, &record) {
        panic!("rewriting the cached record for {entry_id}: {e}");
    }
}

/// A real [`goalcup_protocol::seal::SealManifest`] at `<artifact_root>/trace/seal.json`,
/// where `TraceExport` puts one.
///
/// `goalcup seal`'s conflict branch — the entry was already sealed, at its deadline or by an
/// earlier run — reads that file and compares this machine's scan against it, so a test of
/// that branch needs the real document rather than the `{}` placeholder `mock_sealable`
/// writes.
pub fn plant_exported_seal(artifact_root: &str, manifest: &goalcup_protocol::seal::SealManifest) {
    let path = Path::new(artifact_root)
        .join("trace")
        .join(goalcup_protocol::export::SEAL_FILE);
    let bytes = match serde_json::to_vec_pretty(manifest) {
        Ok(bytes) => bytes,
        Err(e) => panic!("the planted seal manifest: {e}"),
    };
    put(&path, &bytes);
}

// ------------------------------------------------ a mock with the daemon's files in place

/// A mock holding one armed entry for `id`, with the files the daemon writes in place.
///
/// **The daemon's own files are in place.** `hook_install[]` is on the daemon's side, and the
/// CLI writes nothing into a workspace ever: the rendered `settings-launch.json` in the
/// artifact root is what the launch passes to `--settings`. It is rendered from the
/// **shipped** template through the same variables a launch renders, so nothing here is a
/// second spelling of what the daemon writes.
///
/// **`{artifact_root}/raw-bodies` exists here because this fixture makes it exist:** the
/// manifest points `OTEL_LOG_RAW_API_BODIES` at it, and the mock does not create it.
#[cfg(feature = "v1-compat")]
pub fn mock_with_daemon_files(
    tmp: &tempfile::TempDir,
    id: &str,
) -> goalcup_cli::daemon::MockDaemonClient {
    let m = mock_armed(tmp, id);
    let artifact_root = PathBuf::from(&m.info().artifact_root);
    let raw_bodies = artifact_root.join("raw-bodies");
    if let Err(e) = std::fs::create_dir_all(&raw_bodies) {
        panic!("{}: {e}", raw_bodies.display());
    }
    m.set_workspace_files(install_hook_files(id, &m.info()));
    m
}

/// The connector's `hook_install[]` rows, rendered and written where each one's scope says,
/// and described the way the daemon describes them on the workspace answer.
///
/// The templates are the **shipped** ones (`connectors/<id>/files/`), rendered through
/// `goalcup_cli::vars::from_workspace` — the same eight variables the launch recipe renders
/// against — so a value in one of these files is the one the run actually has.
#[cfg(feature = "v1-compat")]
fn install_hook_files(id: &str, info: &control::WorkspaceInfo) -> Vec<control::InstalledFile> {
    use goalcup_protocol::manifest::InstallScope;

    let c = connector(id);
    let vars = goalcup_cli::vars::from_workspace(info);
    let mut out = Vec::new();
    for row in &c.manifest.hook_install {
        let template = connectors_dir().join(id).join("files").join(&row.template);
        let body = match std::fs::read_to_string(&template) {
            Ok(body) => body,
            Err(e) => panic!("{}: {e}", template.display()),
        };
        let rendered = match goalcup_protocol::template::render(&body, &vars) {
            Ok(rendered) => rendered,
            Err(e) => panic!("{}: {e}", template.display()),
        };
        let root = match row.scope {
            InstallScope::Workspace => Path::new(&info.workspace),
            _ => Path::new(&info.artifact_root),
        };
        let path = root.join(&row.path);
        put(&path, rendered.as_bytes());
        out.push(control::InstalledFile {
            path: path.display().to_string(),
            scope: row.scope,
            relative_path: row.path.clone(),
            sha256: goalcup_daemon::workspace::sha256_hex(rendered.as_bytes()),
            bytes: rendered.len() as u64,
        });
    }
    out
}

/// Every path under `root`, with its size and its modification time, sorted.
///
/// It covers the whole root rather than the exported trace alone, so a file written BESIDE
/// the trace is seen, and an mtime moves even when the bytes do not, so a file rewritten with
/// the same bytes is seen too.
///
/// A file whose metadata cannot be read is recorded as such rather than dropped, so a
/// helper that went blind cannot read as a directory that did not change.
pub fn artifact_snapshot(root: &Path) -> Vec<(String, String)> {
    let mut paths = Vec::new();
    collect(root, &mut paths);
    let mut out: Vec<(String, String)> = paths
        .iter()
        .map(|p| {
            let name = p
                .strip_prefix(root)
                .unwrap_or(p.as_path())
                .display()
                .to_string();
            let said = match std::fs::metadata(p) {
                Ok(m) => format!(
                    "{} bytes, mtime {:?}",
                    m.len(),
                    m.modified().ok().map(|t| t
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0))
                ),
                Err(e) => format!("unreadable: {e}"),
            };
            (name, said)
        })
        .collect();
    out.sort();
    out
}

// ------------------------------------------------------------------ the real daemon
//
// Everything in this section exists for `tests/real_daemon_end_to_end.rs`, the one test in
// this crate that drives the SHIPPED BINARIES rather than a client in this process.
//
// The plumbing is deliberately close to `crates/goalcup-devserver/tests/support/gate.rs`,
// which does the same job for the daemon's own gates, and is a second copy rather than a
// shared module: the two crates cannot see each other's test trees, and neither may take a
// dependency to reach one.

/// The longest either child process is given to print a line this module is waiting for.
/// Generous, because it is only ever reached when the child is not going to print at all.
const CHILD_PATIENCE: Duration = Duration::from_secs(60);

/// The round the fixture daemon is told it is running, through its `[event]` section.
///
/// Configured rather than left out so the path through `status::event_status_for` that a real
/// deployment takes is the one exercised end to end, and so a signed status carrying the
/// FALLBACK is a failure here rather than a shape nobody looked at.
const ROUND_T0: &str = "2026-10-01T00:00:00Z";
const ROUND_CLOSES: &str = "2026-10-08T00:00:00Z";
const ROUND_RULES: &str = "rules-2026-10-01";

/// A child's piped stream, read on its own thread and handed over one line at a time, under a
/// deadline.
///
/// `BufReader::lines()` straight off a child pipe blocks forever when the child never prints,
/// and a child that refuses its configuration prints nothing at all on the stream being read:
/// a one-line refusal on the OTHER stream turns into a run that hangs until something outside
/// cargo kills it, with no message about which process it was waiting for. Every read here
/// fails instead, naming what it wanted.
///
/// The reader thread outlives this module's interest on purpose. It keeps consuming after the
/// receiver is gone, so a child writing more than a pipe buffer's worth is never blocked by
/// nobody reading it.
struct ChildLines {
    rx: std::sync::mpsc::Receiver<String>,
}

impl ChildLines {
    fn spawn<R: std::io::Read + Send + 'static>(stream: R) -> ChildLines {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let mut listening = true;
            for line in std::io::BufReader::new(stream)
                .lines()
                .map_while(Result::ok)
            {
                if listening {
                    listening = tx.send(line).is_ok();
                }
            }
        });
        ChildLines { rx }
    }

    /// The next line, or a failure naming `what` this module was waiting for.
    fn next_line(&self, what: &str) -> String {
        match self.rx.recv_timeout(CHILD_PATIENCE) {
            Ok(line) => line,
            Err(e) => panic!("waited {CHILD_PATIENCE:?} for {what} and got {e}"),
        }
    }

    /// The first line `f` answers for, within the same deadline over the whole search: a child
    /// that prints an endless stream of lines nobody wants is as stuck as one that prints none.
    fn find(&self, what: &str, f: impl Fn(&str) -> Option<String>) -> String {
        let deadline = Instant::now() + CHILD_PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(line) => {
                    if let Some(found) = f(&line) {
                        return found;
                    }
                }
                Err(e) => panic!("waited {CHILD_PATIENCE:?} for {what} and got {e}"),
            }
        }
    }
}

/// A child a test owns until it has explicitly observed and reaped its success path.
///
/// `Child` itself does not terminate on drop.  Process-identity fixtures intentionally keep
/// real stand-ins alive while asserting that their identities are never signalled, so every
/// assertion between `spawn` and an explicit reap needs this failure-path ownership too.
pub struct ChildGuard(Option<std::process::Child>);

impl ChildGuard {
    pub fn new(child: std::process::Child) -> Self {
        Self(Some(child))
    }

    pub fn id(&self) -> u32 {
        self.0
            .as_ref()
            .expect("a released fixture child has no pid")
            .id()
    }

    /// The normal success path still reaps explicitly.  [`Drop`] is only the assertion or
    /// panic path, where leaving the process alive would taint PID-sensitive sibling tests.
    pub fn reap(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.reap();
    }
}

/// The local upload signer, running as the shipped binary. Killed on `Drop`.
pub struct DevServerProcess {
    child: std::process::Child,
    url: String,
    key: String,
}

impl DevServerProcess {
    /// Where it is listening, as `http://127.0.0.1:<port>`.
    pub fn url(&self) -> String {
        self.url.clone()
    }

    /// The verifying key it announced on its own banner.
    ///
    /// Read off the child's stderr rather than taken from
    /// [`goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY`], which is the constant a test then
    /// compares it against: a helper that returned the expected answer would make that
    /// comparison assert nothing at all.
    pub fn public_key_hex(&self) -> String {
        self.key.clone()
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for DevServerProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `goalcup-devserver` on a loopback port of the operating system's choosing.
///
/// **No `--clock`.** `api_gate.rs` opens the window ten minutes below the run, but that is the
/// wrong thing to copy here: a record in `[started_at - ARMING_TOLERANCE_SECONDS,
/// started_at)` is a Pass (`pre_start_records`) and only one earlier than that is a Flag, so
/// an honest real-time run needs no margin. A copied offset would move every record above
/// `started_at` and test a shape no participant produces.
pub fn spawn_devserver() -> DevServerProcess {
    let mut child = match std::process::Command::new(sibling("goalcup-devserver"))
        .args(["--addr", "127.0.0.1:0"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => panic!("the devserver starts: {e}"),
    };
    let Some(stderr) = child.stderr.take() else {
        panic!("the devserver's stderr is piped");
    };
    // The banner, the key line and the bound address are all on stderr by design: there is no
    // quiet mode, because an operator who cannot see what the key is worth is the one failure
    // that crate can cause.
    let lines = ChildLines::spawn(stderr);
    let key = lines.find("the devserver's signing key", |line| {
        line.strip_prefix("signing key ")
            .and_then(|rest| rest.split_whitespace().next())
            .map(str::to_string)
    });
    let url = lines.find("where the devserver is listening", |line| {
        line.strip_prefix("listening on ").map(str::to_string)
    });
    DevServerProcess { child, url, key }
}

/// The capture daemon, running as the shipped binary against a configuration this module
/// wrote. Killed on `Drop`.
pub struct DaemonProcess {
    child: std::process::Child,
    config_dir: PathBuf,
    control_addr: String,
    control_file: PathBuf,
}

impl DaemonProcess {
    /// The daemon's `config_dir`, which is also the CLI's `$GOALCUP_CONFIG_DIR`: one
    /// directory holds `control.json`, the grant and the entry cache, and that identity is
    /// the whole reason a participant's CLI can find a running daemon at all.
    pub fn config_dir(&self) -> PathBuf {
        self.config_dir.clone()
    }

    /// The loopback address the control API bound, as the daemon printed it.
    pub fn control_addr(&self) -> String {
        self.control_addr.clone()
    }

    /// The `control.json` it wrote, as the daemon printed it.
    pub fn control_file(&self) -> PathBuf {
        self.control_file.clone()
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `bin` started under a configuration written into `root`, uploading to `upstream`.
///
/// What it builds under `root`:
///
/// | Path | What it is |
/// |---|---|
/// | `cfg/` | `config_dir`: `control.json`, and the CLI's `$GOALCUP_CONFIG_DIR` |
/// | `artifacts/` | `artifact_root`: every entry's directory and its exported trace |
/// | `workspaces-target/` | where a workspace really lands |
/// | `workspaces` | a **symlink** to it, and what the daemon is configured with |
/// | `home/` | `$HOME` for the daemon and for every `goalcup` this module runs |
///
/// **The symlinked workspace parent is the point, not an accident of the platform.** A shell —
/// and a real harness reporting `process.cwd()` — recomputes its working directory
/// physically, while the daemon holds the path it was configured with. `claude_slug` is
/// derived from one of those and the transcript is written under the other, so a daemon that
/// used the configured spelling rather than `PathVars::workspace_canonical` would look for the
/// transcript under a name nothing writes to and leg-1 capture would be silently zero. On
/// macOS a temporary directory is that shape already (`/var` links to `/private/var`); this
/// makes it that shape on every platform, so the test cannot pass for a reason that is about
/// the machine it ran on.
///
/// `$HOME` is pinned to an empty directory for a related reason: `claude-code-cli`'s
/// `foreign_scan[]` globs `{home}/.claude/projects/*/*.jsonl`, and a daemon that inherited the
/// developer's home would scan the developer's own sessions into this run's findings.
///
/// The cadences are turned down so a test waits milliseconds, and the health thresholds are
/// pushed past any clock a test reaches.
pub fn spawn_daemon(bin: &Path, root: &Path, upstream: &str) -> DaemonProcess {
    let config_dir = root.join("cfg");
    let artifact_root = root.join("artifacts");
    let target = root.join("workspaces-target");
    let link = root.join("workspaces");
    let home = root.join("home");
    for dir in [&config_dir, &artifact_root, &target, &home] {
        if let Err(e) = std::fs::create_dir_all(dir) {
            panic!("{}: {e}", dir.display());
        }
    }
    #[cfg(unix)]
    if let Err(e) = std::os::unix::fs::symlink(&target, &link) {
        panic!("{} -> {}: {e}", link.display(), target.display());
    }
    let connectors = connectors_dir();
    let key = goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY;
    let text = format!(
        r#"artifact_root = "{artifacts}"
connectors_dir = "{connectors}"
config_dir = "{config}"
workspace_parent = "{workspaces}"
control_addr = "127.0.0.1:0"
upload_url = "{upstream}"
server_public_key = "{key}"

poll_interval_ms = 100
upload_interval_ms = 200
status_interval_ms = 100
stall_after_seconds = 1000000000.0
silence_window_seconds = 1000000000.0
ack_lag_seconds = 1000000000.0
clock_skew_seconds = 1000000000.0
disk_reserve_bytes = 0

[event]
t0 = "{ROUND_T0}"
start_window_closes_at = "{ROUND_CLOSES}"
rules_version = "{ROUND_RULES}"
"#,
        artifacts = artifact_root.display(),
        connectors = connectors.display(),
        config = config_dir.display(),
        workspaces = link.display(),
    );
    let path = root.join("goalcup-daemon.toml");
    if let Err(e) = std::fs::write(&path, text) {
        panic!("{}: {e}", path.display());
    }
    let mut child = match std::process::Command::new(bin)
        .arg("--config")
        .arg(&path)
        .arg("--foreground")
        .env("HOME", &home)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => panic!("{} starts: {e}", bin.display()),
    };
    let Some(stdout) = child.stdout.take() else {
        panic!("the daemon's stdout is piped");
    };
    // Drained for the life of the process, so a refusal on the other stream is not a hang.
    if let Some(stderr) = child.stderr.take() {
        ChildLines::spawn(stderr);
    }
    let lines = ChildLines::spawn(stdout);
    let control_addr = lines.next_line("the daemon's control address");
    let control_file = PathBuf::from(lines.next_line("the daemon's control file path"));
    DaemonProcess {
        child,
        config_dir,
        control_addr,
        control_file,
    }
}

/// `tests/fixtures/fake-claude.sh` installed as `claude` under `<root>/bin`, and that
/// directory's path.
///
/// The name is `connectors/claude-code-cli/manifest.toml`'s `detection.binary`, which is also
/// its `[launch] program`, so both the D12 version probe and the launch resolve to this file
/// and the shipped connector directory is left byte for byte alone. That last part is not a
/// nicety: a rewritten manifest is a rulebook this build does not pin, `manifest_not_pinned`
/// is a Flag even under `--dev`, and a trace captured under one can never reach a Pass. It is
/// the difference between this fixture and [`connector_with_fake_harness`], whose test does
/// not need a verdict.
pub fn fake_claude_on_path(root: &Path) -> PathBuf {
    let bin = root.join("bin");
    if let Err(e) = std::fs::create_dir_all(&bin) {
        panic!("{}: {e}", bin.display());
    }
    let script = fixture("fake-claude.sh");
    let installed = bin.join("claude");
    if let Err(e) = std::fs::copy(&script, &installed) {
        panic!("{} -> {}: {e}", script.display(), installed.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755))
        {
            panic!("{}: {e}", installed.display());
        }
    }
    bin
}

/// The `goalcup` binary, with a test's own directories resolved for it and nothing of the
/// developer's machine left in the environment.
///
/// `env_clear` and then four variables: `PATH` with `bin` in front (so `claude` is the
/// fixture), `HOME` pointing inside the run, `$GOALCUP_CONFIG_DIR` at the daemon's own
/// `config_dir` — the canonical-location rule, and the whole reason this process can find that
/// daemon — and `$GOALCUP_CONNECTORS_DIR` at the SHIPPED connectors, which is the same
/// directory the daemon was configured with. The last part matters: `goalcup start` sends the
/// digest of the manifest it read, and a CLI reading a different copy from the daemon's would
/// be refused at `create_workspace` rather than tested.
///
/// `root` is [`spawn_daemon`]'s `root` — the whole run's directory — and NOT a home directory:
/// `$HOME` is `root/home` inside it, which is the same empty directory the daemon was given.
pub fn goalcup(cfg: &Path, bin: &Path, root: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup"));
    let inherited = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string());
    command
        .env_clear()
        .env("PATH", format!("{}:{inherited}", bin.display()))
        .env("HOME", root.join("home"))
        .env(goalcup_cli::config::CONFIG_DIR_ENV, cfg)
        .env(goalcup_cli::config::CONNECTORS_DIR_ENV, connectors_dir())
        .stdin(std::process::Stdio::null());
    command
}

/// Wait until the daemon's signed status says this entry is durably `Started` with its bound
/// session id.
///
/// A condition and never a duration: the daemon runs its own loop in
/// another process, and a `sleep` long enough to be reliable on a loaded machine is a test that
/// costs that long every time it passes.
///
/// `StartSignalSeen` is intentionally not enough. It says the arming witness landed, but the
/// receipt/bind work that publishes a durable `Started` status can still append the first
/// autonomous health link. Callers that take a byte-exact chain snapshot must wait through that
/// transition, otherwise an unrelated health tail races their read. A blank session id is the
/// analogous half-written state: a Started entry without its bound session is not a resume- or
/// seal-ready run.
///
/// [`control::ArmingState::Refused`] is a failure here and not something to keep waiting on: it
/// is the daemon saying this gate will never arm, which a machine reader can tell from "still
/// pending".
pub fn wait_for_arming(client: &dyn control::DaemonClient, entry_id: &str) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let last = match client.status(entry_id) {
            Ok(signed) => {
                match signed.payload.arming {
                    control::ArmingState::Started
                        if signed
                            .payload
                            .session_id
                            .as_deref()
                            .is_some_and(|session| !session.trim().is_empty()) =>
                    {
                        return;
                    }
                    control::ArmingState::Refused => panic!(
                        "the daemon refused entry {entry_id}'s gate: {}",
                        signed.payload.detail
                    ),
                    control::ArmingState::Armed
                    | control::ArmingState::StartSignalSeen
                    | control::ArmingState::Started => {}
                }
                format!(
                    "{:?}, session {:?} — {}",
                    signed.payload.arming, signed.payload.session_id, signed.payload.detail
                )
            }
            Err(e) => format!("the status route answered {e}"),
        };
        assert!(
            Instant::now() < deadline,
            "waited {PATIENCE:?} for entry {entry_id} to see its start signal; last: {last}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Wait until the exported chain holds `records` leg-1 records and `posts` leg-2 posts.
///
/// Waited for by CONTENT and not by a duration: sealing while records are still on disk and
/// not yet chained produces a trace missing leg-1 records and a verdict that reads them as
/// `unexplained_seal_file` — a failure of the test's pacing rather than of the daemon.
///
/// The failure prints the chain a line at a time, because the two ways this can go wrong look
/// identical from a counter: nothing was captured, and something was captured under a source or
/// a kind the count did not expect. No payload is ever printed — a failure message that quoted
/// a record would put the run's own text into a test log.
pub fn wait_for_chain(trace_dir: &Path, records: usize, posts: usize) {
    let deadline = Instant::now() + PATIENCE;
    let mut seen = (0, 0);
    loop {
        seen = chained(trace_dir).unwrap_or(seen);
        if seen.0 >= records && seen.1 >= posts {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the chain at {} holds {} leg-1 records and {} leg-2 posts, not {records} and \
             {posts}.\n{}",
            trace_dir.display(),
            seen.0,
            seen.1,
            summarise(trace_dir)
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// The capture facts a resume test can observe without printing the captured payload.
///
/// `resume_markers` and `resume_records` are deliberately separate from the two aggregate
/// counters. A replay of the first launch can grow the transcript count, but the complete
/// five-marker identity pattern across four resume-bearing leg-1 records is evidence the
/// harness's resume invocation reached the transcript tailer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureCounts {
    leg1_records: usize,
    leg2_posts: usize,
    resume_markers: usize,
    resume_records: usize,
}

impl CaptureCounts {
    fn has_resumed_batch_since(self, before: Self, records: usize, posts: usize) -> bool {
        self.leg1_records >= before.leg1_records.saturating_add(records)
            && self.leg2_posts >= before.leg2_posts.saturating_add(posts)
            && self.resume_markers >= before.resume_markers.saturating_add(records)
            && self.resume_records >= before.resume_records.saturating_add(RESUME_BEARING_RECORDS)
    }
}

/// Wait until a resumed harness contributes a complete new capture batch.
///
/// The caller snapshots the first launch after it is fully captured, before starting the
/// resume. The new batch needs both leg deltas and a complete set of newly observed transcript
/// identities marked `*-resume-*`; aggregate link counts, a partial identity set, or too few
/// resume-bearing records alone would permit a replayed first batch to pass. `records` is the
/// expected complete marker count for the fixture's five-record batch; its unmarked mode opener
/// leaves four resume-bearing records.
pub fn wait_for_resumed_capture(
    trace_dir: &Path,
    before: CaptureCounts,
    records: usize,
    posts: usize,
) {
    let deadline = Instant::now() + PATIENCE;
    let mut seen = before;
    loop {
        seen = capture_counts(trace_dir).unwrap_or(seen);
        if seen.has_resumed_batch_since(before, records, posts) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the resumed chain at {} has capture counts {seen:?} after baseline {before:?}; \
             it needs {records} new leg-1 records, {posts} new leg-2 posts, and a new \
             leg-1 transcript batch with the fixture's resume identity.\n{}",
            trace_dir.display(),
            summarise(trace_dir)
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// How many leg-1 records, leg-2 posts, distinct resume-identity markers, and resume-bearing
/// leg-1 records the exported chain holds.
///
/// `EventBody` is adjacently tagged and NOT flattened, so the discriminant is `body.kind`;
/// `Link` is flattened, which is why `seq` is top-level.
pub fn capture_counts(trace_dir: &Path) -> Option<CaptureCounts> {
    let text =
        std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).ok()?;
    let mut resume_markers = [false; RESUME_IDENTITY_MARKERS.len()];
    let mut counts = CaptureCounts {
        leg1_records: 0,
        leg2_posts: 0,
        resume_markers: 0,
        resume_records: 0,
    };
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match v["body"]["kind"].as_str() {
            Some("leg1_record") => {
                counts.leg1_records += 1;
                if let Some(payload) = v["body"]["content"]["payload"].as_str() {
                    let mut record_has_resume_marker = false;
                    for (seen, marker) in resume_markers.iter_mut().zip(RESUME_IDENTITY_MARKERS) {
                        if payload.contains(marker) {
                            *seen = true;
                            record_has_resume_marker = true;
                        }
                    }
                    if record_has_resume_marker {
                        counts.resume_records += 1;
                    }
                }
            }
            Some("leg2_post") => counts.leg2_posts += 1,
            _ => {}
        }
    }
    counts.resume_markers = resume_markers.into_iter().filter(|seen| *seen).count();
    Some(counts)
}

const RESUME_IDENTITY_MARKERS: [&str; 5] = [
    "prompt-1-resume-",
    "toolu-1-resume-",
    "uuid-a1-resume-",
    "uuid-tr1-resume-",
    "prompt-2-resume-",
];
const RESUME_BEARING_RECORDS: usize = 4;

fn chained(trace_dir: &Path) -> Option<(usize, usize)> {
    capture_counts(trace_dir).map(|counts| (counts.leg1_records, counts.leg2_posts))
}

/// [`summarise`] for a caller outside this module: a failure about the chain's SHAPE — a
/// link that is missing, or one too many — is diagnosed from the kinds and never from a
/// payload, and this is the one renderer of them.
pub fn summarise_chain(trace_dir: &Path) -> String {
    summarise(trace_dir)
}

/// The chain as one line per link: sequence, source, kind, and no payload.
fn summarise(trace_dir: &Path) -> String {
    std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
        .unwrap_or_default()
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap_or_default();
            format!("  seq {} {} {}", v["seq"], v["source"], v["body"]["kind"])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Wait until the highest acknowledged sequence reaches the chain's head, which after a seal is
/// the seal link itself.
///
/// Without it a trace can be exported with its last link unanchored, which is a real finding
/// about a run that was still uploading and a false one about a run that has finished. The
/// daemon's upload cadence closes the gap; this waits for the evidence.
pub fn wait_for_seal_ack(trace_dir: &Path) {
    let acked = || -> Option<u64> {
        let text =
            std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::ACKS_FILE)).ok()?;
        text.lines()
            .filter_map(|l| serde_json::from_str::<goalcup_protocol::ack::AckCheckpoint>(l).ok())
            .map(|ack| ack.seq)
            .max()
    };
    let head = || -> Option<u64> {
        let text =
            std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).ok()?;
        let last = text.lines().rfind(|l| !l.trim().is_empty())?;
        serde_json::from_str::<serde_json::Value>(last).ok()?["seq"].as_u64()
    };
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let (Some(h), Some(a)) = (head(), acked())
            && a >= h
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "waited {PATIENCE:?} for the seal link to be acknowledged (chain head {:?}, \
             highest ack {:?})",
            head(),
            acked()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

// V4 workspace-submission process fixtures for `tests/finish_matrix.rs`, the whole-flow fault
// matrix.
//
// Modelled closely on `crates/goalcup-devserver/tests/support/gate.rs`'s
// `Devserver::start_workspace` / `write_workspace_submission_config` (this crate's tests
// cannot see that file, so this is a second copy, not a shared one), and on this file's own
// `spawn_daemon` / `DaemonProcess`, made RESTART-capable: [`spawn_workspace_daemon`] is
// called once to start and again, against the same [`WorkspaceDaemonConfig`], to restart
// after a kill.

/// A V4 workspace-submission daemon configuration written to disk once and reused across a
/// restart: [`spawn_workspace_daemon`] is called a second time against the same value to
/// restart the same durably configured daemon after [`DaemonProcess::stop`]-by-`Drop` or an
/// explicit kill.
pub struct WorkspaceDaemonConfig {
    pub config_path: PathBuf,
    pub config_dir: PathBuf,
    pub home: PathBuf,
}

/// The V4 configuration file [`spawn_workspace_daemon`] starts: [`spawn_daemon`]'s legacy
/// `[event]`-only shape, plus the `[receipt_binding]` block a workspace-submission daemon
/// requires — the same fields
/// `crates/goalcup-devserver/tests/support/gate.rs::write_workspace_submission_config` writes,
/// so this gate exercises the real operator shape and not a narrower one invented for the
/// test.
pub fn write_workspace_submission_config(root: &Path, upstream: &str) -> WorkspaceDaemonConfig {
    let config_dir = root.join("cfg");
    let artifact_root = root.join("artifacts");
    let target = root.join("workspaces-target");
    let link = root.join("workspaces");
    let home = root.join("home");
    for dir in [&config_dir, &artifact_root, &target, &home] {
        if let Err(e) = std::fs::create_dir_all(dir) {
            panic!("{}: {e}", dir.display());
        }
    }
    // The symlinked workspace parent, for the reason `spawn_daemon` above sets one up: a
    // daemon that bound the entry to the configured spelling rather than the
    // canonical one would derive `claude_slug` from one name while the transcript is written
    // under the other.
    #[cfg(unix)]
    if let Err(e) = std::os::unix::fs::symlink(&target, &link) {
        panic!("{} -> {}: {e}", link.display(), target.display());
    }
    let connectors = connectors_dir();
    let key = goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY;
    let text = format!(
        r#"artifact_root = "{artifacts}"
connectors_dir = "{connectors}"
config_dir = "{config}"
workspace_parent = "{workspaces}"
control_addr = "127.0.0.1:0"
upload_url = "{upstream}"
server_public_key = "{key}"

poll_interval_ms = 100
upload_interval_ms = 200
status_interval_ms = 100
stall_after_seconds = 1000000000.0
silence_window_seconds = 1000000000.0
ack_lag_seconds = 1000000000.0
clock_skew_seconds = 1000000000.0
disk_reserve_bytes = 0

[event]
t0 = "{ROUND_T0}"
start_window_closes_at = "{ROUND_CLOSES}"
rules_version = "{ROUND_RULES}"

[receipt_binding]
mode = "workspace_submission"
questionnaire_version = "goal-cup-2026-v1"
submission_profile = "workspace_snapshot_v1"

[receipt_binding.limit_profile]
id = "goal-cup-v1-matrix"

[receipt_binding.effective_limits]
object_count = 4096
total_bytes = 1073741824
pre_cutoff_drain = 5000
process_walk = 64

[receipt_binding.capture_limits]
version = 1
namespace_nodes = 8192
inventory_bytes = 8388608
manifest_bytes = 8388608
"#,
        artifacts = artifact_root.display(),
        connectors = connectors.display(),
        config = config_dir.display(),
        workspaces = link.display(),
    );
    let path = root.join("goalcup-daemon.toml");
    if let Err(e) = std::fs::write(&path, text) {
        panic!("{}: {e}", path.display());
    }
    WorkspaceDaemonConfig {
        config_path: path,
        config_dir,
        home,
    }
}

/// Start — or, called again after a kill against the same [`WorkspaceDaemonConfig`],
/// RESTART — the daemon. Restart-capable because it names no state of its own: everything
/// durable is the config file and the directories under it, both already on disk from the
/// first call.
pub fn spawn_workspace_daemon(bin: &Path, layout: &WorkspaceDaemonConfig) -> DaemonProcess {
    let mut child = match std::process::Command::new(bin)
        .arg("--config")
        .arg(&layout.config_path)
        .arg("--foreground")
        .env("HOME", &layout.home)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => panic!("{} starts: {e}", bin.display()),
    };
    let Some(stdout) = child.stdout.take() else {
        panic!("the daemon's stdout is piped");
    };
    // Drained for the life of the process, so a refusal on the other stream is not a hang.
    if let Some(stderr) = child.stderr.take() {
        ChildLines::spawn(stderr);
    }
    let lines = ChildLines::spawn(stdout);
    let control_addr = lines.next_line("the daemon's control address");
    let control_file = PathBuf::from(lines.next_line("the daemon's control file path"));
    DaemonProcess {
        child,
        config_dir: layout.config_dir.clone(),
        control_addr,
        control_file,
    }
}

/// `goalcup-devserver`, started for the V4 workspace-submission receipt contract
/// (`--receipt-mode new-event --questionnaire-version goal-cup-2026-v1`), with an optional
/// short server-owned qualification window for a signed short-deadline fixture (`None` keeps
/// the default 168-hour window). Mirrors
/// `crates/goalcup-devserver/tests/support/gate.rs::Devserver::start_workspace`, plus the
/// `--qualification-window-seconds` flag that fixed-args helper never passes.
pub fn spawn_workspace_devserver(qualification_window_seconds: Option<u64>) -> DevServerProcess {
    let mut command = std::process::Command::new(sibling("goalcup-devserver"));
    command
        .args(["--addr", "127.0.0.1:0"])
        .args(["--receipt-mode", "new-event"])
        .args(["--questionnaire-version", "goal-cup-2026-v1"]);
    if let Some(seconds) = qualification_window_seconds {
        command.args(["--qualification-window-seconds", &seconds.to_string()]);
    }
    let mut child = match command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => panic!("the devserver starts: {e}"),
    };
    let Some(stderr) = child.stderr.take() else {
        panic!("the devserver's stderr is piped");
    };
    // The banner, the key line and the bound address are all on stderr by design: there is no
    // quiet mode, because an operator who cannot see what the key is worth is the one failure
    // that crate can cause.
    let lines = ChildLines::spawn(stderr);
    let key = lines.find("the devserver's signing key", |line| {
        line.strip_prefix("signing key ")
            .and_then(|rest| rest.split_whitespace().next())
            .map(str::to_string)
    });
    let url = lines.find("where the devserver is listening", |line| {
        line.strip_prefix("listening on ").map(str::to_string)
    });
    DevServerProcess { child, url, key }
}

/// [`WorkspaceOpts`](control::WorkspaceOpts) with the shipped connector's REAL manifest
/// digest — the daemon's own admission check — and not [`workspace_opts`]'s fixed placeholder
/// digest, which is for `MockDaemonClient` fixtures and which a real daemon refuses.
pub fn real_workspace_opts(connector_id: &str) -> control::WorkspaceOpts {
    let dir = connectors_dir().join(connector_id);
    let bytes = match std::fs::read(dir.join("manifest.toml")) {
        Ok(bytes) => bytes,
        Err(e) => panic!("the shipped manifest for {connector_id}: {e}"),
    };
    control::WorkspaceOpts {
        connector_id: connector_id.to_string(),
        manifest_sha256: goalcup_daemon::workspace::sha256_hex(&bytes),
        workspace_parent: None,
        attach: false,
        deadline_hours: 168,
        grant_token: "grant-finish-matrix".to_string(),
        arming_timeout_seconds: None,
        harness_version: Some("0.0.0-matrix".to_string()),
    }
}

/// Wait for one of the daemon's own cadences to produce a fact, and answer it. A condition,
/// never a duration, for the reason
/// `crates/goalcup-devserver/tests/support/gate.rs::wait_for` gives: the daemon runs its own
/// loop in another process, and a `sleep` long enough to be reliable on a loaded machine is a
/// test that costs that long every time it passes.
pub fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(value) = f() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "waited {PATIENCE:?} for {what} and it did not happen"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod capture_counts_tests {
    use super::CaptureCounts;

    #[test]
    fn a_duplicate_batch_is_not_a_resumed_capture_without_resume_transcript_records() {
        let first_launch = CaptureCounts {
            leg1_records: 5,
            leg2_posts: 0,
            resume_markers: 0,
            resume_records: 0,
        };
        let replayed_first_launch = CaptureCounts {
            leg1_records: 10,
            leg2_posts: 0,
            resume_markers: 0,
            resume_records: 0,
        };
        let one_resume_record_and_four_replays = CaptureCounts {
            leg1_records: 10,
            leg2_posts: 0,
            resume_markers: 1,
            resume_records: 1,
        };
        let three_resume_records_and_two_replays = CaptureCounts {
            leg1_records: 10,
            leg2_posts: 0,
            resume_markers: 5,
            resume_records: 3,
        };
        let resumed_launch = CaptureCounts {
            resume_markers: 5,
            resume_records: 4,
            ..replayed_first_launch
        };

        assert!(
            !replayed_first_launch.has_resumed_batch_since(first_launch, 5, 0),
            "a replay can grow the aggregate transcript count but cannot prove a resumed harness ran"
        );
        assert!(
            !one_resume_record_and_four_replays.has_resumed_batch_since(first_launch, 5, 0),
            "one resume-marked record plus four replays cannot prove a complete resume batch"
        );
        assert!(
            !three_resume_records_and_two_replays.has_resumed_batch_since(first_launch, 5, 0),
            "three resume-bearing records plus two replays cannot prove a complete resume batch"
        );
        assert!(resumed_launch.has_resumed_batch_since(first_launch, 5, 0));
    }
}

// ---------------------------------------------------------------------------------------
