//! `goalcup-verify <trace-dir>` — the offline verifier. **There is no `goalcup
//! verify` sub-command**: one verifier, one set of flags, one set of exit codes, and this
//! binary is it. The one other caller, the V1 `goalcup doctor --verify`, linked this crate
//! and called `verify_trace_with_options` in process rather than shelling out here, so the
//! two could not diverge; D48 removed it.
//!
//! Exit codes: the verdict's own (0 Pass, 1 Flag, 2 Fail), and **3** for anything that
//! stopped a verdict from being produced at all. A trace that could not be read is not a
//! trace that failed verification, and the two must not share an exit code.
//!
//! Streams: **stdout carries the verdict document and nothing else.** Without `--json`
//! that document is the JSON itself; with `--json <path>` the document goes to the file
//! and stdout stays empty, so `goalcup-verify d --json v.json | jq .` reads an empty
//! stream instead of a summary line. Everything a person is told — the development-flag
//! notices, the errors, the one-line summary of a `--json` run — is stderr.
//!
//! `goalcup-verify --capture-integrity TRACE` is the separate mode for a protocol-v5
//! capture trace. It never reaches the legacy pipeline, and it is routed before
//! `Pipeline::load`. Stdout is the integrity report and nothing else, and stderr carries
//! the notices and a one-line summary. Its exits are the report's own: 0 complete valid
//! local evidence (the anchor may stay `local_only`), 1 incomplete or unsealed coverage,
//! 2 invalid evidence (a torn tail included). 3 means no usable report: a trace that
//! could not be read, a chain that is not v5, or an invocation that names both modes.
//! A conflicting invocation is refused by the parser before any output file is opened.
//!
//! `goalcup-verify --research-json TRACE --output REPORT.ndjson` writes the private research
//! report of a protocol-v5 capture trace (`docs/research/m2-report-format.md`), routed here
//! the same way. The output is a new private file outside the trace, and stdout stays empty.
//! Its exits are the report's own: 0 complete, 1 incomplete or partial, 2 invalid evidence,
//! 3 no usable report. A report that could not be finished has no summary line, and exits 3.
#![forbid(unsafe_code)]

use clap::Parser;
use goalcup_verifier::VerifyOptions;
use goalcup_verifier::checks::Severity;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "goalcup-verify",
    about = "Verify a /goal Cup trace directory",
    // The version this build carries, which is also the `verifier_version` every verdict
    // records.
    version
)]
struct Args {
    /// Directory holding `trace.toml`.
    #[arg(
        required_unless_present_any = ["capture_integrity", "research_json"],
        conflicts_with_all = ["capture_integrity", "research_json"]
    )]
    trace_dir: Option<PathBuf>,
    /// Check a protocol-v5 capture trace's integrity instead (bounded, streaming). The
    /// report goes to stdout; the exit code is the report's.
    #[arg(
        long = "capture-integrity",
        value_name = "TRACE",
        conflicts_with = "json"
    )]
    capture_integrity: Option<PathBuf>,
    /// Write the private research report of a protocol-v5 capture trace to --output
    /// instead: one NDJSON line per decoded observation, then one summary line. The exit
    /// code is the report's.
    #[arg(
        long = "research-json",
        value_name = "TRACE",
        requires = "output",
        conflicts_with_all = ["capture_integrity", "json"]
    )]
    research_json: Option<PathBuf>,
    /// With --research-json: the report file to create, named *.ndjson and outside the
    /// trace. An existing file is refused.
    #[arg(long, value_name = "OUTPUT", requires = "research_json")]
    output: Option<PathBuf>,
    /// Write the verdict JSON here instead of stdout. Stdout then stays empty and the
    /// one-line summary goes to stderr.
    #[arg(long)]
    json: Option<PathBuf>,
    /// Development only: check acknowledgement signatures against the key the trace
    /// descriptor names, even when this build does not recognise it. The descriptor is
    /// written by the participant, so a run using this flag proves nothing about who
    /// signed — the verdict records `key_source: "descriptor"` to say so.
    #[arg(long = "dev-descriptor-key")]
    dev_descriptor_key: bool,
    /// Development only, and it covers two things. First: report a connector manifest this
    /// build does not pin as a Flag instead of a Fail. The manifest is the rulebook — the
    /// human rule, the exclusion catalogue and the tool classes — so a run using this flag
    /// proves nothing about which rules judged the entry, and the finding says which
    /// reading applied. Second: suppress `placeholder_server_key`, the Flag a trace raises
    /// when the M1 placeholder anchor (whose seed is published in this repository) is what
    /// verified its receipt or its acknowledgements — which is the expected thing while
    /// developing a connector and never acceptable when judging an entry.
    ///
    /// It does not suppress `server_key_unpinned`.
    #[arg(long = "dev")]
    dev: bool,
    /// Development only, repeatable: accept signatures under this Ed25519 public key
    /// (64 hex characters) for this run alone.
    ///
    /// This is how a trace signed by `goalcup-devserver` — whose seed is public and in the
    /// repository on purpose — is verified without the development anchor entering
    /// `SERVER_PUBLIC_KEYS`, where neither a reader of the constant nor a reader of a
    /// verdict could tell it from a production one. The verdict records
    /// `key_source: "supplied"`.
    #[arg(long = "dev-key", value_name = "HEX", value_parser = server_public_key)]
    dev_key: Vec<String>,
}

/// A 64-character hex Ed25519 public key, refused by clap before any trace is read.
///
/// The same shape `TraceDescriptor::load` accepts, so a key that is legal on the command
/// line is legal in a descriptor. A mistyped key that reached `verify_trace` would resolve
/// to no key at all and be reported as an anchor nobody could check — a wrong statement
/// about the trace, when the mistake was in the invocation.
fn server_public_key(value: &str) -> Result<String, String> {
    if value.len() != 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "an Ed25519 public key is 64 hex characters; this is {} character(s){}",
            value.len(),
            if value.chars().all(|c| c.is_ascii_hexdigit()) {
                ""
            } else {
                ", and not all of them are hex"
            }
        ));
    }
    Ok(value.to_string())
}

/// The severity as the verdict file spells it, not as `Debug` renders it.
///
/// Read back out of `Severity`'s own serialisation rather than matched by hand, so the
/// summary line and the document can never disagree about the word: a `match` here would
/// be a second place that decides how a severity is spelled. The fallback is unreachable
/// — `Severity` is a unit enum with a `snake_case` rename — and exists so no summary line
/// can panic a run that has a verdict to report.
fn severity_word(severity: Severity) -> String {
    match serde_json::to_value(severity) {
        Ok(serde_json::Value::String(word)) => word,
        _ => "unknown".to_string(),
    }
}

/// The options of a protocol-v5 mode (`--capture-integrity` or `--research-json`).
///
/// The development options keep their meaning, and each one says so on stderr. A supplied
/// key is accepted for this run and reported as `key_source: "supplied"`. `--dev`
/// suppresses the placeholder-key disclosure. A v5 capture trace has no descriptor, so
/// `--dev-descriptor-key` has nothing to admit.
fn capture_options(mode: &str, args: Args) -> VerifyOptions {
    if args.dev {
        eprintln!(
            "goalcup-verify: --dev is set, so the placeholder-server-key finding is \
             suppressed; this run proves nothing about who signed the receipt or \
             acknowledgements it checks"
        );
    }
    if !args.dev_key.is_empty() {
        eprintln!(
            "goalcup-verify: --dev-key supplied {} key(s) this build does not pin; a receipt \
             or acknowledgement verified under one borrows its trust from this invocation, \
             and the report records key_source: \"supplied\"",
            args.dev_key.len()
        );
    }
    if args.dev_descriptor_key {
        eprintln!(
            "goalcup-verify: --dev-descriptor-key has no effect with {mode}: a protocol-v5 \
             capture trace carries no descriptor key"
        );
    }
    VerifyOptions {
        allow_descriptor_key: args.dev_descriptor_key,
        extra_server_keys: args.dev_key,
        dev_mode: args.dev,
    }
}

/// `--capture-integrity TRACE`: the bounded v5 integrity report on stdout, its exit code.
fn capture_integrity(trace: &Path, args: Args) -> std::process::ExitCode {
    let options = capture_options("--capture-integrity", args);
    match goalcup_verifier::capture_integrity_document(trace, &options) {
        Ok((report, text)) => {
            println!("{text}");
            let code = report.exit_code();
            let integrity = serde_json::to_value(report.integrity).unwrap_or_default();
            let anchor = serde_json::to_value(report.anchor).unwrap_or_default();
            eprintln!(
                "goalcup-verify: capture integrity {} (anchor {}, exit {code})",
                integrity.as_str().unwrap_or("unknown"),
                anchor.as_str().unwrap_or("unknown")
            );
            std::process::ExitCode::from(code)
        }
        Err(e) => {
            eprintln!("goalcup-verify: {e}");
            std::process::ExitCode::from(3)
        }
    }
}

/// `--research-json TRACE --output OUTPUT`: the research report into a new private file,
/// its exit code. Nothing goes to stdout.
fn research_json(trace: &Path, output: &Path, args: Args) -> std::process::ExitCode {
    let options = capture_options("--research-json", args);
    let canonical = match std::fs::canonicalize(trace) {
        Ok(canonical) => canonical,
        Err(e) => {
            eprintln!("goalcup-verify: {}: {e}", trace.display());
            return std::process::ExitCode::from(3);
        }
    };
    let file = match create_report_file(&canonical, output) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("goalcup-verify: {}: {e}", output.display());
            return std::process::ExitCode::from(3);
        }
    };
    let mut writer = std::io::BufWriter::new(file);
    match goalcup_verifier::write_research_report(trace, &options, &mut writer) {
        Ok(result) => {
            eprintln!(
                "goalcup-verify: research report -> {}: capture {}, analysis {}, exit {}",
                output.display(),
                result.summary["capture"]["integrity"]
                    .as_str()
                    .unwrap_or("unknown"),
                result.summary["analysis"]["state"]
                    .as_str()
                    .unwrap_or("unknown"),
                result.exit_code
            );
            std::process::ExitCode::from(result.exit_code)
        }
        Err(e) => {
            // Whatever reached the file has no summary line, which marks it incomplete.
            eprintln!(
                "goalcup-verify: {e}; {} has no summary line and is incomplete",
                output.display()
            );
            std::process::ExitCode::from(3)
        }
    }
}

/// Create the report file: named `*.ndjson`, outside the (canonical) trace directory, new
/// (an existing file is refused, never overwritten) and readable by its owner only.
fn create_report_file(trace: &Path, output: &Path) -> std::io::Result<std::fs::File> {
    use std::io::{Error, ErrorKind};
    if output
        .extension()
        .is_none_or(|extension| extension != "ndjson")
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "the research report is NDJSON: name the output *.ndjson",
        ));
    }
    let parent = match output.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    if std::fs::canonicalize(parent)?.starts_with(trace) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "the report must be written outside the trace it reads",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(output)
}

fn main() -> std::process::ExitCode {
    // `Args::parse()` would exit(2) on a usage error, and 2 is the `Fail` verdict — a
    // mistyped flag would be indistinguishable from a trace that failed verification.
    // `try_parse` hands the error back so it can leave under 3, the code this module
    // promises for "no verdict was produced". `--help` and `--version` arrive here as
    // errors too; they are not failures, so they print to stdout and exit 0, which is what
    // `Error::use_stderr()` distinguishes.
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            let usage_error = e.use_stderr();
            let _ = e.print();
            return std::process::ExitCode::from(if usage_error { 3 } else { 0 });
        }
    };
    // The capture modes are routed here, before the legacy pipeline loads anything.
    if let Some(trace) = args.capture_integrity.clone() {
        return capture_integrity(&trace, args);
    }
    if let (Some(trace), Some(output)) = (args.research_json.clone(), args.output.clone()) {
        return research_json(&trace, &output, args);
    }
    let Some(trace_dir) = args.trace_dir.clone() else {
        // The parser requires a trace directory unless a capture mode is named.
        eprintln!(
            "goalcup-verify: name a trace directory, --capture-integrity TRACE, or \
             --research-json TRACE --output OUTPUT"
        );
        return std::process::ExitCode::from(3);
    };
    // Every development flag says so on stderr, and stdout stays the verdict document and
    // nothing else. A verdict is a file people pass around: a reader who sees one has no
    // way to know how it was produced, so the run that produced it announces itself where
    // the person who ran it will see it.
    if args.dev {
        eprintln!(
            "goalcup-verify: --dev is set, so a connector manifest this build does not pin \
             is a Flag rather than a Fail and the placeholder-server-key Flag is \
             suppressed; this run proves nothing about which rules judged the entry, nor \
             about who signed its start"
        );
    }
    if !args.dev_key.is_empty() {
        eprintln!(
            "goalcup-verify: --dev-key supplied {} key(s) this build does not pin; a trace \
             verified under one borrows its trust from this invocation, and the verdict \
             records key_source: \"supplied\"",
            args.dev_key.len()
        );
    }
    let options = VerifyOptions {
        allow_descriptor_key: args.dev_descriptor_key,
        extra_server_keys: args.dev_key,
        dev_mode: args.dev,
    };
    let (verdict, text) =
        match goalcup_verifier::verify_trace_document_with_options(&trace_dir, options) {
            Ok(document) => document,
            Err(e) => {
                eprintln!("goalcup-verify: {e}");
                return std::process::ExitCode::from(3);
            }
        };
    match &args.json {
        Some(path) => {
            if let Err(e) = std::fs::write(path, &text) {
                eprintln!("goalcup-verify: writing {}: {e}", path.display());
                return std::process::ExitCode::from(3);
            }
            // Stderr, and the serialised severity word: on stdout a pipe into `jq` would
            // read the summary as the document, and `{:?}` would print `Fail` where the
            // file it announces says `"overall": "fail"`.
            eprintln!(
                "goalcup-verify: {} {} -> {}",
                verdict.connector_id,
                severity_word(verdict.overall),
                path.display()
            );
        }
        None => {
            println!("{text}");
            // The same summary line, on the same stream, for the invocation that has no file
            // to name: the default run — the one `goalcup seal` prints and a participant
            // types — says something a person can read. Stdout stays exactly one document
            // either way: this goes to stderr, like every other line meant for a person.
            eprintln!(
                "goalcup-verify: {} {}",
                verdict.connector_id,
                severity_word(verdict.overall)
            );
        }
    }
    std::process::ExitCode::from(verdict.exit_code() as u8)
}
