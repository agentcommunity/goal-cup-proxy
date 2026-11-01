//! `goalcup-verify`: `check`, `dump`, `secrets` and `usage` (PRD-08 section 1). Exit codes: 0
//! whole; 1 honest but incomplete (every loss named, or unsealed, live or pruned); 2 a failure;
//! 3 no usable report (the input cannot be read, the command line is wrong, or an internal
//! error). `--help` and `--version` exit 0.

use clap::{Parser, Subcommand};
use goalcup_format::Record;
use goalcup_format::dump::Dumper;
use goalcup_verify::{input, legs, report, secrets, usage};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "goalcup-verify",
    version,
    about = "Proves a goalcup record whole and names every loss."
)]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Checks an entry, rec, run or fetched folder, or one part file.
    Check(CheckArgs),
    /// One JSON line per record. Payload is printed only with --text.
    Dump(DumpArgs),
    /// The secret scan: known values and secret shapes, in records and in raw files.
    Secrets(secrets::Args),
    /// The usage reader.
    #[command(disable_help_flag = true)]
    Usage {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
}

#[derive(clap::Args)]
struct DumpArgs {
    path: PathBuf,
    /// A stream id, or BOOT.SID for one boot.
    #[arg(long)]
    sid: Option<String>,
    /// Only records of this kind (part, open, data, end, gap, mask, event, link).
    #[arg(long)]
    kind: Option<String>,
    /// Print DATA payload (`text` or `hex`).
    #[arg(long)]
    text: bool,
}

#[derive(clap::Args)]
struct CheckArgs {
    path: PathBuf,
    /// Print the report (S6) as JSON to stdout.
    #[arg(long)]
    json: bool,
    /// The entry is still recording: read to the durable point.
    #[arg(long)]
    live: bool,
    /// The input is a server copy.
    #[arg(long)]
    server: bool,
    /// The local entry to compare the server copy with.
    #[arg(long, value_name = "ENTRY_DIR")]
    local: Option<PathBuf>,
    /// Known secret values, one per line: adds the `secrets` check to every entry.
    #[arg(long, value_name = "FILE")]
    known: Option<PathBuf>,
    /// The pad's list of harness files on disk, `[{"path": "<source>:<path>", "size": n}]`:
    /// each needs a native stream of its size or a snapshot stream.
    #[arg(long, value_name = "MANIFEST")]
    harness_files: Option<PathBuf>,
}

fn main() -> ExitCode {
    // A panic prints where it happened, never its message: that could hold record text.
    std::panic::set_hook(Box::new(|info| {
        let at = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()));
        let at = at.unwrap_or_else(|| "an unknown place".into());
        eprintln!("goalcup-verify: internal error at {at}");
    }));
    std::panic::catch_unwind(run).unwrap_or(ExitCode::from(3))
}

fn run() -> ExitCode {
    // `parse()` would exit 2 on a usage error, and 2 means a failed record. `try_parse` hands the
    // error back so it leaves under 3; `--help` and `--version` come back as errors too, print to
    // stdout and exit 0, which `use_stderr()` tells apart.
    let args = match Args::try_parse_from(std::env::args_os()) {
        Ok(a) => a,
        Err(e) => {
            let usage_error = e.use_stderr();
            let _ = e.print();
            return ExitCode::from(if usage_error { 3 } else { 0 });
        }
    };
    match args.cmd {
        Cmd::Check(a) => check(&a),
        Cmd::Dump(d) => dump(&d.path, d.sid.as_deref(), d.kind.as_deref(), d.text),
        Cmd::Secrets(a) => secrets::run(&a).unwrap_or_else(|e| no_report(&e)),
        Cmd::Usage { args } => usage::run(args),
    }
}

/// A plain line for exit 3.
fn no_report(e: &dyn std::fmt::Display) -> ExitCode {
    eprintln!("goalcup-verify: {e}");
    ExitCode::from(3)
}

fn check(a: &CheckArgs) -> ExitCode {
    // A `--local` entry pruned of every part file (PRD-06 section 7) keeps an empty `rec/`: it is
    // no entry and `server_equals_local` skips it as pruned. Any other resolve error is exit 3.
    let local = a.local.as_ref().map(|l| {
        let rec = goalcup_format::read::part_files(&l.join("rec"));
        let all_pruned = rec.is_ok_and(|p| p.is_empty()).then(input::Input::default);
        input::resolve(l, false, false).or_else(|e| all_pruned.ok_or(e))
    });
    let known = a.known.as_deref().map(secrets::Rules::new).transpose();
    let files = a
        .harness_files
        .as_deref()
        .map(legs::read_manifest)
        .transpose();
    let found = input::resolve(&a.path, a.live, a.server);
    let (input, l, k, f) = match (found, local.transpose(), known, files) {
        (Ok(i), Ok(l), Ok(k), Ok(f)) => (i, l, k, f),
        (Err(e), ..) | (_, Err(e), ..) | (_, _, Err(e), _) | (.., Err(e)) => return no_report(&e),
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    let now = now.map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
    let r = report::build(
        &input,
        a.live.then_some(now),
        l.as_ref(),
        k.as_ref(),
        f.as_deref(),
    );
    eprint!("{}", r.text);
    if a.json {
        let _ = writeln!(std::io::stdout(), "{}", r.json);
    }
    ExitCode::from(r.exit)
}

fn dump(path: &std::path::Path, sid: Option<&str>, kind: Option<&str>, text: bool) -> ExitCode {
    let pick = match sid.map(|s| match s.split_once('.') {
        Some((b, n)) => Some((Some(b.parse::<u64>().ok()?), n.parse::<u64>().ok()?)),
        None => Some((None, s.parse::<u64>().ok()?)),
    }) {
        Some(None) => return no_report(&"--sid takes N or BOOT.N"),
        p => p.flatten(),
    };
    let input = match input::resolve(path, false, false) {
        Ok(i) => i,
        Err(e) => return no_report(&e),
    };
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    let mut broke = false;
    for e in &input.entries {
        let mut rd = match input::open(e) {
            Ok(r) => r,
            Err(e) => return no_report(&e),
        };
        let (mut d, mut boot) = (Dumper::default(), 0u64);
        let mut tail_line = None;
        for r in rd.by_ref() {
            let it = match r {
                Ok(it) => it,
                Err(e) => {
                    tail_line = Some(json!({"k": "tail", "error": e.to_string()}));
                    broke = true;
                    break;
                }
            };
            if let Record::Part { boot: b, .. } = it.rec {
                boot = b;
            }
            let mut v = d.line(&it);
            let keep_sid = pick.is_none_or(|(b, n)| {
                b.is_none_or(|b| b == boot) && v.get("sid").and_then(Value::as_u64) == Some(n)
            });
            let keep_kind = kind.is_none_or(|k| v.get("k").and_then(Value::as_str) == Some(k));
            if let (Some(m), false) = (v.as_object_mut(), text) {
                m.remove("text");
                m.remove("hex");
            }
            if keep_sid && keep_kind && writeln!(out, "{v}").is_err() {
                return ExitCode::SUCCESS;
            }
        }
        let t = rd.tail();
        let (open, cut) = (t.open_frame, t.truncated);
        if tail_line.is_none() && (open || cut.is_some()) {
            tail_line = Some(json!({"k": "tail", "open_frame": open, "truncated_at": cut}));
        }
        if let Some(l) = tail_line
            && writeln!(out, "{l}").is_err()
        {
            return ExitCode::SUCCESS;
        }
    }
    let _ = out.flush();
    ExitCode::from(if broke { 2 } else { 0 })
}
