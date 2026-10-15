//! `goalcup-verify` — the offline verifier of a protocol-v5 capture trace. **There is no
//! `goalcup verify` sub-command**: one verifier, one set of flags, one set of exit codes, and
//! this binary is it.
//!
//! Streams: stdout carries the report and nothing else; everything a person is told (the
//! development-flag notices, the errors, the one-line summary) is stderr.
//!
//! `goalcup-verify --capture-integrity TRACE` checks a capture trace's integrity. Stdout is
//! the integrity report. Its exits are the report's own: 0 complete valid local evidence
//! (the anchor may stay `local_only`), 1 incomplete or unsealed coverage, 2 invalid evidence
//! (a torn tail included). 3 means no usable report: a trace that could not be read, a chain
//! that is not v5, or an invocation the parser refuses (both modes, neither, or an unknown
//! flag). A conflicting invocation is refused by the parser before any output file is opened.
//!
//! `goalcup-verify --research-json TRACE --output REPORT.ndjson` writes the private research
//! report of a capture trace (`docs/research/m2-report-format.md`). The output is a new
//! private file outside the trace, and stdout stays empty. Its exits are the report's own:
//! 0 complete, 1 incomplete or partial, 2 invalid evidence, 3 no usable report. A report
//! that could not be finished has no summary line, and exits 3.
#![forbid(unsafe_code)]

use clap::Parser;
use goalcup_verifier::VerifyOptions;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "goalcup-verify",
    about = "Verify a /goal Cup capture trace",
    // The version this build carries, which is also the `verifier_version` every report
    // records.
    version,
    // Exactly one of the two modes.
    group(clap::ArgGroup::new("mode").required(true).args(["capture_integrity", "research_json"]))
)]
struct Args {
    /// Check a protocol-v5 capture trace's integrity (bounded, streaming). The
    /// report goes to stdout; the exit code is the report's.
    #[arg(long = "capture-integrity", value_name = "TRACE")]
    capture_integrity: Option<PathBuf>,
    /// Write the private research report of a protocol-v5 capture trace to --output:
    /// one NDJSON line per decoded observation, then one summary line. The exit
    /// code is the report's.
    #[arg(long = "research-json", value_name = "TRACE", requires = "output")]
    research_json: Option<PathBuf>,
    /// With --research-json: the report file to create, named *.ndjson and outside the
    /// trace. An existing file is refused.
    #[arg(long, value_name = "OUTPUT", requires = "research_json")]
    output: Option<PathBuf>,
    /// Development only, repeatable: accept signatures under this Ed25519 public key
    /// (64 hex characters) for this run alone.
    ///
    /// This is how a trace signed by a development signer (whose seed is public) is verified
    /// without the development anchor entering `SERVER_PUBLIC_KEYS`, where neither a reader
    /// of the constant nor a reader of a report could tell it from a production one. The
    /// report records `key_source: "supplied"`.
    #[arg(long = "dev-key", value_name = "HEX", value_parser = server_public_key)]
    dev_key: Vec<String>,
}

/// A 64-character hex Ed25519 public key, refused by clap before any trace is read.
///
/// A mistyped key that reached the reader would resolve to no key at all and be reported as
/// an anchor nobody could check — a wrong statement about the trace, when the mistake was in
/// the invocation.
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

/// The options of either mode (`--capture-integrity` or `--research-json`).
///
/// A supplied key says so on stderr, is accepted for this run and is reported as
/// `key_source: "supplied"`.
fn capture_options(args: Args) -> VerifyOptions {
    if !args.dev_key.is_empty() {
        eprintln!(
            "goalcup-verify: --dev-key supplied {} key(s) this build does not pin; a receipt \
             or acknowledgement verified under one borrows its trust from this invocation, \
             and the report records key_source: \"supplied\"",
            args.dev_key.len()
        );
    }
    VerifyOptions {
        allow_descriptor_key: false,
        extra_server_keys: args.dev_key,
    }
}

/// `--capture-integrity TRACE`: the bounded v5 integrity report on stdout, its exit code.
fn capture_integrity(trace: &Path, args: Args) -> std::process::ExitCode {
    let options = capture_options(args);
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
            for why in &report.git_unreadable {
                eprintln!("goalcup-verify: {why}");
            }
            if report.findings.iter().any(|f| f == "chain_pruned_locally") {
                eprintln!(
                    "goalcup-verify: the chain's first parts were pruned on this machine: \
                     verify the server copy"
                );
            }
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
    let options = capture_options(args);
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
    // `Args::parse()` would exit(2) on a usage error, and 2 is "invalid evidence": a
    // mistyped flag would be indistinguishable from a trace that failed verification.
    // `try_parse` hands the error back so it can leave under 3, the code this module
    // promises for "no usable report". `--help` and `--version` arrive here as errors too;
    // they are not failures, so they print to stdout and exit 0, which is what
    // `Error::use_stderr()` distinguishes.
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            let usage_error = e.use_stderr();
            let _ = e.print();
            return std::process::ExitCode::from(if usage_error { 3 } else { 0 });
        }
    };
    if let Some(trace) = args.capture_integrity.clone() {
        return capture_integrity(&trace, args);
    }
    if let (Some(trace), Some(output)) = (args.research_json.clone(), args.output.clone()) {
        return research_json(&trace, &output, args);
    }
    // The parser requires one of the two modes, and `--research-json` requires `--output`.
    eprintln!(
        "goalcup-verify: name --capture-integrity TRACE or --research-json TRACE --output OUTPUT"
    );
    std::process::ExitCode::from(3)
}
