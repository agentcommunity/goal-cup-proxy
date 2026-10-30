//! `goalcup-verify`: `check`, `dump` and `usage` (PRD-08 section 1).
//!
//! Exit codes: 0 whole; 1 honest but incomplete (every loss named, or unsealed, live or pruned);
//! 2 a failure; 3 no usable report (the input cannot be read, the command line is wrong, or an
//! internal error). `--help` and `--version` exit 0.

use clap::{Parser, Subcommand};
use goalcup_format::Record;
use goalcup_format::dump::Dumper;
use goalcup_verify::{input, report, usage};
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
    Check {
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
        /// The local entry to compare the server copy with (not read yet).
        #[arg(long, value_name = "ENTRY_DIR")]
        local: Option<PathBuf>,
        /// Known secret values, one per line (not read yet).
        #[arg(long, value_name = "FILE")]
        known: Option<PathBuf>,
        /// The pad's list of harness files on disk (not read yet).
        #[arg(long, value_name = "MANIFEST")]
        harness_files: Option<PathBuf>,
    },
    /// One JSON line per record. Payload is printed only with --text.
    Dump {
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
    },
    /// The usage reader.
    #[command(disable_help_flag = true)]
    Usage {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
}

fn main() -> ExitCode {
    // A panic prints where it happened, never its message: that could hold record text.
    std::panic::set_hook(Box::new(|info| {
        let at = info.location().map_or_else(
            || "an unknown place".into(),
            |l| format!("{}:{}", l.file(), l.line()),
        );
        eprintln!("goalcup-verify: internal error at {at}");
    }));
    std::panic::catch_unwind(run).unwrap_or(ExitCode::from(3))
}

fn run() -> ExitCode {
    let argv: Vec<OsString> = std::env::args_os().collect();
    // `parse()` would exit 2 on a usage error, and 2 means a failed record. `try_parse` hands the
    // error back so it leaves under 3; `--help` and `--version` come back as errors too, print to
    // stdout and exit 0, which `use_stderr()` tells apart.
    let args = match Args::try_parse_from(&argv) {
        Ok(a) => a,
        Err(e) => {
            let usage_error = e.use_stderr();
            let _ = e.print();
            return ExitCode::from(if usage_error { 3 } else { 0 });
        }
    };
    match args.cmd {
        // `--local`, `--known` and `--harness-files` are taken now and read by the checks that
        // need them, which are `skip` until a later slice builds them.
        Cmd::Check {
            path,
            json,
            live,
            server,
            ..
        } => check(&path, json, live, server),
        Cmd::Dump {
            path,
            sid,
            kind,
            text,
        } => dump(&path, sid.as_deref(), kind.as_deref(), text),
        Cmd::Usage { args } => usage::run(args),
    }
}

fn check(path: &std::path::Path, json: bool, live: bool, server: bool) -> ExitCode {
    let input = match input::resolve(path, live, server) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("goalcup-verify: {e}");
            return ExitCode::from(3);
        }
    };
    let now = live.then(|| {
        let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
        d.map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
    });
    let r = report::build(&input, now);
    eprint!("{}", r.text);
    if json {
        let _ = writeln!(std::io::stdout(), "{}", r.json);
    }
    ExitCode::from(r.exit)
}

fn dump(path: &std::path::Path, sid: Option<&str>, kind: Option<&str>, text: bool) -> ExitCode {
    let pick = match sid.map(|s| match s.split_once('.') {
        Some((b, n)) => b
            .parse::<u64>()
            .ok()
            .zip(n.parse::<u64>().ok())
            .map(|(b, n)| (Some(b), n)),
        None => s.parse::<u64>().ok().map(|n| (None, n)),
    }) {
        Some(None) => {
            eprintln!("goalcup-verify: --sid takes N or BOOT.N");
            return ExitCode::from(3);
        }
        p => p.flatten(),
    };
    let input = match input::resolve(path, false, false) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("goalcup-verify: {e}");
            return ExitCode::from(3);
        }
    };
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    let mut broke = false;
    for e in &input.entries {
        let (mut rd, meter) = match input::open(e) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("goalcup-verify: {e}");
                return ExitCode::from(3);
            }
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
        let (open, cut) = (meter.open_frame.get(), rd.tail().truncated);
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
