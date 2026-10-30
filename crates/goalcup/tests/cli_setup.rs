//! `goalcup setup` without questions (PRD-10 section 7, acceptance 6, seam S15): no terminal means
//! no question, no open and no write; `--yes` writes only into the isolated home and prints it as
//! the last folder path; no `goalcup mcp` text and no `PROBLEM:` line; `--record-host` refuses an
//! IP, a loopback name and a `never` host. Facts only: exit codes, trees, parsed settings files.
//! The case "an n answer writes `update_off` only" waits for the interactive setup slice.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::home::{TempHome, guard};
use goalcup::cli::{isolated_home, record_hosts, setup_json, text};
use goalcup::connector::{self, Connector, Decision, HostList, builtin};
use serde_json::Value;

/// The first built-in connector with `update_off` and `no_prompts` rows, all in JSON or TOML
/// files, which the test parses back.
fn connector() -> Connector {
    let parsed = |c: &Connector| {
        let mut rows = c.update_off.iter().chain(&c.no_prompts);
        rows.all(|r| r.file.ends_with(".json") || r.file.ends_with(".toml"))
    };
    builtin::CONNECTORS
        .iter()
        .map(|(id, _)| connector::load(id, None).expect("a built-in connector loads"))
        .find(|c| !c.update_off.is_empty() && !c.no_prompts.is_empty() && parsed(c))
        .expect("a built-in connector with both kinds of rows")
}

/// A folder outside both homes holding `bin/<program>`, a fake harness that logs its arguments.
struct Fake(tempfile::TempDir);

impl Fake {
    fn new(program: &str) -> Fake {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let log = dir.path().join("argv.log").display().to_string();
        let script = format!("#!/bin/sh\necho \"$*\" >> '{log}'\necho 'fake 9.9.9'\n");
        fs::write(bin.join(program), script).unwrap();
        fs::set_permissions(bin.join(program), fs::Permissions::from_mode(0o755)).unwrap();
        Fake(dir)
    }

    fn path(&self) -> String {
        format!("{}:/usr/bin:/bin", self.0.path().join("bin").display())
    }

    fn argv(&self) -> Vec<String> {
        let log = fs::read_to_string(self.0.path().join("argv.log")).unwrap_or_default();
        log.lines().map(str::to_owned).collect()
    }
}

fn setup(home: &TempHome, path: &str, args: &[&str]) -> (Output, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_goalcup"));
    home.apply(&mut cmd)
        .env("GOALCUP_SERVER", "off")
        .env("PATH", path);
    let out = cmd.arg("setup").args(args).current_dir(home.home());
    let out = out.stdin(Stdio::null()).output().expect("goalcup runs");
    let all = format!("{}{}", lossy(&out.stdout), lossy(&out.stderr));
    (out, all)
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Every path under `dir`: a file's bytes, or None for a folder.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    for path in fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()) {
        if path.is_dir() {
            out.extend(snapshot(&path));
        }
        out.insert(path.clone(), fs::read(&path).ok());
    }
    out
}

/// A settings file parsed into JSON values, TOML through serde.
fn parse(file: &Path) -> Value {
    let text = fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    match file.extension().and_then(|x| x.to_str()) {
        Some("toml") => serde_json::to_value(text.parse::<toml::Table>().unwrap()).unwrap(),
        _ => serde_json::from_str(&text).unwrap(),
    }
}

#[test]
fn no_terminal_asks_nothing_opens_nothing_writes_nothing() {
    let _guard = guard();
    let (home, c) = (TempHome::new(), connector());
    let fake = Fake::new(&c.program);
    let before = snapshot(home.home());
    let (out, all) = setup(&home, &fake.path(), &["--harness", &c.id]);
    assert_eq!(out.status.code(), Some(0), "{all}");
    assert_eq!(snapshot(home.home()), before, "{all}");
    assert!(fake.argv().is_empty(), "the harness ran: {:?}", fake.argv());
    assert!(!all.contains("goalcup mcp"), "{all}");
}

#[test]
fn yes_writes_the_settings_only_into_the_isolated_home() {
    let _guard = guard();
    let (home, c) = (TempHome::new(), connector());
    let fake = Fake::new(&c.program);
    let before = snapshot(home.home());
    let (out, all) = setup(&home, &fake.path(), &["--harness", &c.id, "--yes"]);
    assert_eq!(out.status.code(), Some(0), "{all}");
    assert!(!all.contains(text::PROBLEM), "{all}");
    assert!(!all.contains("goalcup mcp"), "{all}");
    assert_eq!(fake.argv(), ["--version"]);

    let iso = isolated_home(&home.goalcup_home(), &c.id);
    let lock = PathBuf::from(format!("{}.goalcup.lock", iso.display()));
    for (path, now) in snapshot(home.home()) {
        let kept = before.get(&path) == Some(&now);
        let ours = path.starts_with(&iso) || iso.starts_with(&path);
        let ours = ours || path == setup_json(&home.goalcup_home()) || path == lock;
        assert!(kept || ours, "setup wrote {}", path.display());
    }
    for row in c.update_off.iter().chain(&c.no_prompts) {
        let file = PathBuf::from(row.file.replace("{home}", &iso.to_string_lossy()));
        let doc = parse(&file);
        let got = row.key.split('.').try_fold(&doc, |v, k| v.get(k));
        let got = got.map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned));
        assert_eq!(got.as_deref(), Some(row.value.as_str()), "{}", row.key);
    }

    let last_folder = |out: &Output| {
        let stdout = lossy(&out.stdout);
        let quoted = stdout.split('"').skip(1).step_by(2);
        quoted.filter(|p| Path::new(p).is_dir()).last().map(PathBuf::from)
    };
    assert_eq!(last_folder(&out), Some(iso.clone()), "{all}");

    // Point 6: with no row writable, setup.json names none and the home is still printed last.
    let files: Vec<PathBuf> = (c.update_off.iter().chain(&c.no_prompts))
        .map(|r| PathBuf::from(r.file.replace("{home}", &iso.to_string_lossy())))
        .collect();
    files.iter().for_each(|f| fs::write(f, "[").unwrap());
    let (out, all) = setup(&home, &fake.path(), &["--harness", &c.id, "--yes"]);
    assert_eq!(out.status.code(), Some(0), "{all}");
    let saved = fs::read(setup_json(&home.goalcup_home())).unwrap();
    let saved: Value = serde_json::from_slice(&saved).expect("setup.json parses");
    assert_eq!(saved["update_off"], Value::Array(vec![]), "{saved}");
    assert_eq!(saved["no_prompts"], Value::Array(vec![]), "{saved}");
    assert!(files.iter().all(|f| fs::read(f).unwrap() == b"["), "a row was written");
    assert_eq!(last_folder(&out), Some(iso), "{all}");
}

#[test]
fn yes_without_the_program_on_path_writes_nothing() {
    let _guard = guard();
    let (home, c) = (TempHome::new(), connector());
    let empty = tempfile::tempdir().unwrap();
    let before = snapshot(home.home());
    let path = empty.path().to_string_lossy().into_owned();
    let (out, all) = setup(&home, &path, &["--harness", &c.id, "--yes"]);
    assert_eq!(out.status.code(), Some(1), "{all}");
    assert_eq!(snapshot(home.home()), before, "{all}");
}

#[test]
fn record_host_refuses_an_address_this_computer_and_a_never_host() {
    let _guard = guard();
    let home = TempHome::new();
    let list = HostList::from_file(builtin::HOSTS.as_bytes(), "hosts").unwrap();
    let raw: toml::Table = builtin::HOSTS.parse().unwrap();
    let names = raw["never"].as_array().unwrap();
    let never = (names.iter())
        .filter_map(|v| v.as_str())
        .filter(|n| !n.contains('*'))
        .find(|n| list.decide(n) == Decision::Never)
        .expect("a never host in the built-in list");
    let file = record_hosts(&home.goalcup_home());
    for bad in ["203.0.113.7", "localhost", never] {
        let (out, all) = setup(&home, "/usr/bin:/bin", &["--record-host", bad]);
        assert_eq!(out.status.code(), Some(1), "{bad}: {all}");
        assert!(!all.contains(bad), "the refusal repeats {bad}: {all}");
        assert!(!file.exists(), "{bad}");
    }
    for _ in 0..2 {
        let args = ["--record-host", "llm.example.com"];
        let (out, all) = setup(&home, "/usr/bin:/bin", &args);
        assert_eq!(out.status.code(), Some(0), "{all}");
        let text = fs::read_to_string(&file).unwrap();
        assert_eq!(text.lines().filter(|l| *l == "llm.example.com").count(), 1);
    }
}
