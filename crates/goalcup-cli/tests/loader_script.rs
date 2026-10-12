//! The owner's key loader
//! (`scripts/with-deepseek-key.sh`). Every key and every other secret here is synthetic, in
//! temporary files this test makes; the loader is run under `/bin/sh` against a fake `goalcup`
//! that records what it was started with.
#![cfg(unix)]

mod secret_support;
use secret_support::KEY;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const OTHER: &str = "sk-other-secret-that-must-never-be-exported-9f8e";
const OPENAI: &str = "sk-openai-key-that-must-never-be-exported-1a2b";

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/with-deepseek-key.sh")
}

struct World {
    dir: tempfile::TempDir,
}

impl World {
    fn new() -> World {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let fake = bin.join("goalcup");
        std::fs::write(
            &fake,
            "#!/bin/sh -p\nenv > \"$OUT/env\"\nprintf '%s\\n' \"$@\" > \"$OUT/argv\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        World { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// A key file with `text`, mode 0600.
    fn env_file(&self, text: &str) -> PathBuf {
        let path = self.path("key.env");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    /// Run the loader as `sh <shell_args> script <args>`.
    fn run(&self, shell_args: &[&str], args: &[&str], extra_env: &[(&str, &str)]) -> Output {
        self.run_with("/bin/sh", shell_args, args, extra_env)
    }

    /// [`World::run`] under any interpreter; an empty `interpreter` runs the script file itself,
    /// as its shebang line says.
    fn run_with(
        &self,
        interpreter: &str,
        shell_args: &[&str],
        args: &[&str],
        extra_env: &[(&str, &str)],
    ) -> Output {
        let _ = std::fs::remove_file(self.path("env"));
        let _ = std::fs::remove_file(self.path("argv"));
        let mut command = if interpreter.is_empty() {
            Command::new(script())
        } else {
            let mut c = Command::new(interpreter);
            c.args(shell_args).arg(script());
            c
        };
        command
            .args(args)
            .env_clear()
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.path("bin").display()),
            )
            .env("OUT", self.dir.path());
        for (k, v) in extra_env {
            command.env(k, v);
        }
        command.output().unwrap()
    }

    fn ran(&self) -> bool {
        self.path("argv").exists()
    }
}

fn start_args<'a>(env_file: &'a Path, goal: &'a str) -> Vec<&'a str> {
    vec![
        env_file.to_str().unwrap(),
        "start",
        "--harness",
        "hermes-cli",
        "--goal-file",
        goal,
    ]
}

fn printed(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn assert_no_key(what: &str, text: &str) {
    for form in [KEY, &KEY[..8], &KEY[KEY.len() - 8..]] {
        assert!(!text.contains(form), "{what} shows a form of the key");
    }
}

/// A multi-variable file: the loader reads only the `DEEPSEEK_API_KEY` line, exports exactly that
/// one variable, and starts exactly `goalcup start --harness hermes-cli` with the given goal.
#[test]
fn a_multi_variable_env_file_exports_only_the_deepseek_key() {
    let w = World::new();
    let file = w.env_file(&format!(
        "# a comment\nOTHER_SECRET={OTHER}\nOPENAI_API_KEY={OPENAI}\nFOO=bar baz\n\
         DEEPSEEK_API_KEY_OLD=sk-old-value\n#DEEPSEEK_API_KEY=sk-commented-out-value\n\
         DEEPSEEK_API_KEY=\"{KEY}\"\nLAST=1"
    ));
    let out = w.run(&[], &start_args(&file, "/tmp/goal.txt"), &[]);
    assert!(out.status.success(), "{}", printed(&out));
    let env = std::fs::read_to_string(w.path("env")).unwrap();
    assert!(
        env.lines().any(|l| l == format!("DEEPSEEK_API_KEY={KEY}")),
        "{env}"
    );
    for never in [
        OTHER,
        OPENAI,
        "sk-old-value",
        "sk-commented-out-value",
        "OTHER_SECRET",
        "FOO=",
        "LAST=",
    ] {
        assert!(
            !env.contains(never),
            "the child environment holds `{never}`"
        );
    }
    assert_eq!(
        std::fs::read_to_string(w.path("argv")).unwrap(),
        "start\n--harness\nhermes-cli\n--goal-file\n/tmp/goal.txt\n"
    );
    assert_no_key("the loader's output", &printed(&out));
}

/// Under `sh -x`, an inherited `SHELLOPTS=xtrace` or a `BASH_ENV` that turns tracing on, nothing
/// the loader prints holds a form of the key.
#[test]
fn tracing_never_prints_the_key() {
    let w = World::new();
    let file = w.env_file(&format!("DEEPSEEK_API_KEY={KEY}\n"));
    let trace_on = w.path("trace-on.sh");
    std::fs::write(&trace_on, "set -x\n").unwrap();
    for (what, shell_args, env) in [
        ("sh -x", vec!["-x"], vec![]),
        ("sh -xv", vec!["-x", "-v"], vec![]),
        ("SHELLOPTS=xtrace", vec![], vec![("SHELLOPTS", "xtrace")]),
        (
            "BASH_ENV and ENV turn it on",
            vec![],
            vec![
                ("BASH_ENV", trace_on.to_str().unwrap()),
                ("ENV", trace_on.to_str().unwrap()),
            ],
        ),
        (
            "sh -x with SHELLOPTS",
            vec!["-x"],
            vec![("SHELLOPTS", "xtrace:verbose")],
        ),
    ] {
        let out = w.run(&shell_args, &start_args(&file, "/tmp/goal.txt"), &env);
        assert!(
            out.status.success(),
            "{what}: {:?} {}",
            out.status,
            printed(&out)
        );
        assert!(w.ran(), "{what}");
        assert_no_key(what, &printed(&out));
    }
}

/// Only `goalcup start --harness hermes-cli` is ever executed, and only with the two goal-file
/// options: any other command, a second or other harness, or any other option is refused before
/// the key is read, and nothing runs.
#[test]
fn only_the_fixed_hermes_start_is_ever_executed() {
    let w = World::new();
    let file = w.env_file(&format!("DEEPSEEK_API_KEY={KEY}\n"));
    let f = file.to_str().unwrap();
    let cases: Vec<(&str, Vec<&str>)> = vec![
        ("another command", vec![f, "env"]),
        ("a shell", vec![f, "sh", "-c", "env"]),
        ("another goalcup command", vec![f, "seal"]),
        (
            "another harness",
            vec![f, "start", "--harness", "codex-cli"],
        ),
        (
            "a second harness",
            vec![
                f,
                "start",
                "--harness",
                "hermes-cli",
                "--harness",
                "codex-cli",
            ],
        ),
        (
            "a second harness spelled with =",
            vec![f, "start", "--harness", "hermes-cli", "--harness=codex-cli"],
        ),
        (
            "the harness after start's other option",
            vec![f, "start", "--dev", "--harness", "hermes-cli"],
        ),
        (
            "an extra option",
            vec![f, "start", "--harness", "hermes-cli", "--dev"],
        ),
        (
            "an extra word",
            vec![
                f,
                "start",
                "--harness",
                "hermes-cli",
                "--goal-file",
                "g",
                "x",
            ],
        ),
        (
            "a dangling option",
            vec![f, "start", "--harness", "hermes-cli", "--goal-file"],
        ),
        ("no command", vec![f]),
        ("nothing", vec![]),
    ];
    for (what, args) in cases {
        let out = w.run(&[], &args, &[]);
        // Exit 1 is the refusal rung; 2 is the verifier's `Fail`, and a wrapper must not read a
        // refused loader as one.
        assert_eq!(
            out.status.code(),
            Some(1),
            "{what}: was accepted or exited oddly"
        );
        assert!(!w.ran(), "{what}: goalcup was started");
        assert_no_key(what, &printed(&out));
    }
    // The permitted options.
    let out = w.run(
        &[],
        &[
            f,
            "start",
            "--harness",
            "hermes-cli",
            "--goal-file=/tmp/g",
            "--capture-reserve-bytes",
            "4194304",
        ],
        &[],
    );
    assert!(out.status.success(), "{}", printed(&out));
    assert!(w.ran());
}

/// Every unusable key file is refused without a value in the message, and nothing runs.
#[test]
fn an_unusable_key_file_is_refused_without_echoing_a_value() {
    let w = World::new();
    let line = format!("DEEPSEEK_API_KEY={KEY}");
    let cases: Vec<(&str, String)> = vec![
        ("no key line", format!("OTHER={OTHER}\n")),
        ("duplicate", format!("{line}\n{line}\n")),
        ("export form", format!("export {line}\n")),
        ("leading space", format!(" {line}\n")),
        ("spaces around =", format!("DEEPSEEK_API_KEY = {KEY}\n")),
        ("single quotes", format!("DEEPSEEK_API_KEY='{KEY}'\n")),
        ("CRLF", format!("{line}\r\n")),
        ("trailing space", format!("{line} \n")),
        ("trailing comment", format!("{line} # note\n")),
        ("empty", "DEEPSEEK_API_KEY=\n".to_string()),
        ("short", "DEEPSEEK_API_KEY=sk-short\n".to_string()),
        ("bad character", format!("DEEPSEEK_API_KEY={KEY}$\n")),
        (
            "command substitution",
            "DEEPSEEK_API_KEY=$(touch /tmp/pwned-by-the-loader-test)\n".to_string(),
        ),
    ];
    for (what, text) in cases {
        let file = w.env_file(&text);
        let out = w.run(&[], &start_args(&file, "/tmp/goal.txt"), &[]);
        assert_eq!(
            out.status.code(),
            Some(1),
            "{what}: was accepted or exited oddly"
        );
        assert!(!w.ran(), "{what}: goalcup was started");
        assert_no_key(what, &printed(&out));
    }
    assert!(!Path::new("/tmp/pwned-by-the-loader-test").exists());

    // Mode: anything but 0600.
    for mode in [0o644, 0o640, 0o660, 0o400, 0o700] {
        let file = w.env_file(&format!("{line}\n"));
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
        let out = w.run(&[], &start_args(&file, "/tmp/goal.txt"), &[]);
        assert_eq!(out.status.code(), Some(1), "mode {mode:o} was accepted");
        assert!(!w.ran());
        assert_no_key("a mode refusal", &printed(&out));
    }
    // And a file that is not there.
    let out = w.run(
        &[],
        &start_args(&w.path("absent.env"), "/tmp/goal.txt"),
        &[],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(!w.ran());
}

/// The script's first statement turns tracing off, before anything reads the key.
#[test]
fn the_first_statement_turns_tracing_off() {
    let text = std::fs::read_to_string(script()).unwrap();
    let first = text
        .lines()
        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .unwrap();
    assert_eq!(first.trim(), "set +x");
    let read = text
        .find("IFS= read -r line")
        .expect("the loader reads the key");
    assert!(text.find("set +x").unwrap() < read);
    assert!(text.contains("unset SHELLOPTS"));
    assert!(
        !text.contains("source ") && !text.contains("\n. "),
        "never sources the file"
    );
}

/// Every command a hostile environment could have replaced with an exported shell function: the
/// builtins the loader uses after the key is read and the externals it uses before.
const COMMANDS: [&str; 18] = [
    "read", "[", "test", "export", "unset", "exec", "command", "echo", "printf", "case", "sed",
    "grep", "stat", "cut", "tr", "head", "cat", "awk",
];

/// An exported function named for `name` that prints whatever shell variable could hold the key,
/// then runs the real command.
fn hostile_function(name: &str) -> Option<(String, String)> {
    // Keywords cannot be replaced by a function.
    if name == "case" {
        return None;
    }
    let builtin = matches!(
        name,
        "read" | "[" | "test" | "export" | "unset" | "exec" | "command" | "echo" | "printf"
    );
    let real = if builtin {
        format!("builtin '{name}' \"$@\"")
    } else {
        format!("/usr/bin/{name} \"$@\"")
    };
    // Printed after the real command ran, so what the command just set is in the variables.
    Some((
        format!("BASH_FUNC_{name}%%"),
        format!(
            "() {{ {real}; __rc=$?; echo \"LEAK:${{line-}}${{val-}}${{v-}}${{DEEPSEEK_API_KEY-}}\" >&2; return $__rc; }}"
        ),
    ))
}

fn hostile_functions() -> Vec<(String, String)> {
    COMMANDS
        .iter()
        .filter_map(|n| hostile_function(n))
        .collect()
}

fn as_refs(env: &[(String, String)]) -> Vec<(&str, &str)> {
    env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

/// The documented invocation is `/bin/sh -p`, and under it neither a
/// `BASH_ENV` that traces every command, nor an exported shell function standing in for any command
/// the loader runs, can print the key. Privileged mode skips `BASH_ENV`, `ENV`, `SHELLOPTS` and the
/// import of exported functions; the shebang says `-p`, so running the file does the same.
#[test]
fn the_documented_invocation_ignores_hostile_environments() {
    let w = World::new();
    let file = w.env_file(&format!("OTHER_SECRET={OTHER}\nDEEPSEEK_API_KEY={KEY}\n"));
    let args = start_args(&file, "/tmp/goal.txt");
    let tracer = w.path("tracer.sh");
    std::fs::write(
        &tracer,
        "set -x\ntrap 'echo \"TRACE:${val-}${v-}${line-}${DEEPSEEK_API_KEY-}\" >&2' DEBUG\n",
    )
    .unwrap();
    let functions = hostile_functions();
    let mut hostile: Vec<(String, String)> = functions.clone();
    hostile.push(("BASH_ENV".into(), tracer.display().to_string()));
    hostile.push(("ENV".into(), tracer.display().to_string()));
    hostile.push(("SHELLOPTS".into(), "xtrace".into()));
    let env = as_refs(&hostile);
    for (what, interpreter, shell_args) in [
        ("/bin/sh -p", "/bin/sh", vec!["-p"]),
        ("/bin/bash -p", "/bin/bash", vec!["-p"]),
        ("the file itself (its shebang)", "", vec![]),
    ] {
        let out = w.run_with(interpreter, &shell_args, &args, &env);
        assert!(
            out.status.success(),
            "{what}: {:?} {}",
            out.status,
            printed(&out)
        );
        assert!(w.ran(), "{what}");
        let said = printed(&out);
        assert!(
            !said.contains("LEAK:") && !said.contains("TRACE:"),
            "{what}: a hostile hook ran"
        );
        assert_no_key(what, &said);
        let child = std::fs::read_to_string(w.path("env")).unwrap();
        assert!(
            child
                .lines()
                .any(|l| l == format!("DEEPSEEK_API_KEY={KEY}")),
            "{what}"
        );
        assert!(
            !child.contains(OTHER),
            "{what}: another secret was exported"
        );
    }
}

/// A copy of the loader without its re-exec guard: what the script was before it re-executed itself under `-p`.
fn unguarded_copy(w: &World) -> PathBuf {
    let text = std::fs::read_to_string(script()).unwrap();
    let guard = text
        .lines()
        .find(|l| l.starts_with("case $- in *p*)"))
        .expect("the re-exec guard");
    let path = w.path("unguarded.sh");
    std::fs::write(&path, text.replace(guard, ":")).unwrap();
    path
}

/// The positive controls: a loader with no guard, run without `-p`, does print the key under the
/// same environments (a bash `sh` imports exported functions, and `bash` honours `BASH_ENV`), so
/// the tests above and below are not passing because the vectors do nothing.
#[test]
fn an_unguarded_loader_without_privileged_mode_does_leak() {
    let w = World::new();
    let file = w.env_file(&format!("DEEPSEEK_API_KEY={KEY}\n"));
    let args = start_args(&file, "/tmp/goal.txt");
    let unguarded = unguarded_copy(&w);
    // Not `echo` or `printf`: the function's own body calls `echo`, which would recurse.
    let functions: Vec<(String, String)> = hostile_functions()
        .into_iter()
        .filter(|(name, _)| !name.contains("echo") && !name.contains("printf"))
        .collect();
    let run = |interpreter: &str, env: &[(&str, &str)]| {
        let mut c = Command::new(interpreter);
        c.arg(&unguarded)
            .args(&args)
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", w.path("bin").display()))
            .env("OUT", w.dir.path());
        for (k, v) in env {
            c.env(k, v);
        }
        printed(&c.output().unwrap())
    };
    // macOS's `/bin/sh` is bash and imports exported functions; a dash `/bin/sh` (Debian, Ubuntu)
    // never does, so there the vector is shown under `/bin/bash`, which always imports them.
    let sh_is_bash = Command::new("/bin/sh")
        .args(["-c", "printf %s \"${BASH_VERSION-}\""])
        .env_clear()
        .output()
        .is_ok_and(|o| !o.stdout.is_empty());
    let plain = if sh_is_bash { "/bin/sh" } else { "/bin/bash" };
    let said = run(plain, &as_refs(&functions));
    assert!(
        said.contains("LEAK:") && said.contains(&KEY[..8]),
        "an exported function did not run under plain `{plain}`: {said}"
    );
    let tracer = w.path("tracer.sh");
    std::fs::write(
        &tracer,
        "trap 'echo \"TRACE:${val-}${v-}${line-}${DEEPSEEK_API_KEY-}\" >&2' DEBUG\n",
    )
    .unwrap();
    let said = run("/bin/bash", &[("BASH_ENV", tracer.to_str().unwrap())]);
    assert!(
        said.contains("TRACE:") && said.contains(&KEY[..8]),
        "a BASH_ENV DEBUG trap did not run under plain `bash`: {said}"
    );
}

/// Invoked without `-p` (the way a careless operator would), the loader runs
/// itself again under `/bin/sh -p` before it reads anything, so the same hostile environments
/// print nothing, and the child still gets exactly the one variable.
#[test]
fn a_plain_invocation_re_execs_privileged_and_prints_nothing() {
    let w = World::new();
    let file = w.env_file(&format!("OTHER_SECRET={OTHER}\nDEEPSEEK_API_KEY={KEY}\n"));
    let args = start_args(&file, "/tmp/goal.txt");
    let tracer = w.path("tracer.sh");
    std::fs::write(
        &tracer,
        "set -x\ntrap 'echo \"TRACE:${val-}${v-}${line-}${DEEPSEEK_API_KEY-}\" >&2' DEBUG\n",
    )
    .unwrap();
    // Not `echo` or `printf`: their bodies call `echo`, and a plain shell that imported them would
    // recurse until it crashed, before the loader's first line, which proves nothing about it.
    let mut hostile: Vec<(String, String)> = hostile_functions()
        .into_iter()
        .filter(|(name, _)| !name.contains("echo") && !name.contains("printf"))
        .collect();
    hostile.push(("BASH_ENV".into(), tracer.display().to_string()));
    hostile.push(("ENV".into(), tracer.display().to_string()));
    hostile.push(("SHELLOPTS".into(), "xtrace".into()));
    let env = as_refs(&hostile);
    for (what, interpreter, shell_args) in [
        ("/bin/sh", "/bin/sh", vec![]),
        ("/bin/bash", "/bin/bash", vec![]),
        ("/bin/sh -x", "/bin/sh", vec!["-x"]),
        ("/bin/bash -x", "/bin/bash", vec!["-x"]),
    ] {
        let out = w.run_with(interpreter, &shell_args, &args, &env);
        assert!(
            out.status.success(),
            "{what}: {:?} {}",
            out.status,
            printed(&out)
        );
        assert!(w.ran(), "{what}");
        let said = printed(&out);
        assert!(!said.contains("LEAK:"), "{what}: an exported function ran");
        assert_no_key(what, &said);
        let child = std::fs::read_to_string(w.path("env")).unwrap();
        assert!(
            child
                .lines()
                .any(|l| l == format!("DEEPSEEK_API_KEY={KEY}")),
            "{what}"
        );
        assert!(
            !child.contains(OTHER),
            "{what}: another secret was exported"
        );
    }
}

/// No external command runs between the key being read and the `exec`: everything after the
/// marker is a shell builtin, so there is no command a replaced `PATH` entry or function could
/// stand in for while the key is in a variable.
#[test]
fn nothing_external_runs_while_the_key_is_in_memory() {
    let text = std::fs::read_to_string(script()).unwrap();
    assert_eq!(text.lines().next(), Some("#!/bin/sh -p"));
    let (_, after) = text
        .split_once("# KEY-IN-MEMORY: no external command below this line.")
        .expect("the marker");
    let code: Vec<&str> = after
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect();
    let code = code.join("\n");
    for external in [
        "sed", "grep", "stat", "cut", "tr", "awk", "head", "tail", "cat", "expr", "date",
    ] {
        assert!(
            !code
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|w| w == external),
            "`{external}` appears after the key is read"
        );
    }
    assert!(code.contains("exec goalcup start --harness hermes-cli"));
    // And the usage the script documents is the privileged one.
    assert!(text.contains("/bin/sh -p scripts/with-deepseek-key.sh"));
}
