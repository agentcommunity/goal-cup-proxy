//! `goalcup-mcp` — the process: resolve a configuration directory, then read a line and
//! write a line until stdin ends.
//!
//! Everything that decides anything is [`goalcup_mcp::handle_request`], which is pure. This
//! file holds the two things that cannot be: where the configuration directory is, and what
//! time it is.
//!
//! # The exit codes
//!
//! `0` on a clean end of input, `1` on a refusal — including a usage error. `Parser::parse`
//! exits `2` on a usage error and `2` is a `Fail` verdict in this tree's ladder, so a
//! mistyped flag would have been indistinguishable, to a script reading the number, from an
//! entry that did not verify. `goalcup`'s own `main` makes the same move for the same
//! reason. `--help` and `--version` arrive as clap errors too; they are not failures, so
//! they print on stdout and exit `0`, which is what `Error::use_stderr` distinguishes.
//!
//! # It never writes anything anywhere
//!
//! Not a log, not a cache, not a lock. The only thing that leaves this process is one JSON
//! line per request on stdout.
//!
//! # What ends the session, and what does not
//!
//! End of input ends it. A bad LINE does not: one that is longer than
//! [`goalcup_mcp::MAX_DOCUMENT_BYTES`] or is not UTF-8 is answered `-32700` and the loop
//! reads the next one. The alternative — reading every line with `BufRead::lines`, which is
//! unbounded and returns an `Err` on invalid UTF-8 — would mean one byte from anything
//! upstream takes the run's status away for the rest of the run.
#![forbid(unsafe_code)]

use clap::Parser;
use goalcup_mcp::{MAX_DOCUMENT_BYTES, StatusSource, handle_request, unreadable_line_response};
use goalcup_protocol::control::format_z;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

/// Where the tracker keeps its own state. The same variable `goalcup` reads.
const CONFIG_DIR_ENV: &str = "GOALCUP_CONFIG_DIR";

/// The most of one request line this process holds, which is the cap it holds one cached
/// document with. Two numbers for "one JSON value in memory" would be one number too many.
const MAX_LINE_BYTES: usize = MAX_DOCUMENT_BYTES as usize;

/// Read-only /goal Cup status for an agent, over MCP on stdio.
#[derive(Parser)]
#[command(
    name = "goalcup-mcp",
    version,
    after_help = "It is shipped and never auto-registered: a participant who wants their \
                  agent to have these tools registers this binary themselves. It reads the \
                  tracker's signed status cache, opens no socket, and changes nothing."
)]
struct Args {
    /// The tracker's configuration directory. Defaults to $GOALCUP_CONFIG_DIR, else the
    /// platform's per-user application directory with `goalcup` as the last component.
    #[arg(long, value_name = "DIR")]
    config_dir: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            let usage_error = e.use_stderr();
            // clap writes `--help` and `--version` to stdout and a usage error to stderr,
            // and `print()` honours that split.
            let _ = e.print();
            return ExitCode::from(u8::from(usage_error));
        }
    };
    let config_dir = match args.config_dir {
        Some(dir) => dir,
        None => match goalcup_platform::config_dir() {
            Some(dir) => dir,
            None => {
                return refuse(&format!(
                    "no configuration directory: neither ${CONFIG_DIR_ENV} nor a platform \
                 configuration root is set (Windows: $LOCALAPPDATA or $USERPROFILE; macOS: \
                 $HOME; Unix: $XDG_CONFIG_HOME or $HOME). Pass --config-dir, or set \
                 ${CONFIG_DIR_ENV} to the directory /goal Cup keeps its state in."
                ));
            }
        },
    };
    // The keys are read once, at startup, which is when `goalcup start` has already written
    // them: this process is spawned by a harness that the participant started after their
    // run existed.
    let source = match StatusSource::open(&config_dir) {
        Ok(source) => source,
        Err(e) => return refuse(&format!("{}: {e}", config_dir.display())),
    };
    serve(&source)
}

/// Read a line, write a line. A blank line is skipped rather than answered, so a client
/// that pads its stream does not get a parse error back for the padding.
///
/// Only two things end this loop: the end of input, and a failure of the streams themselves.
/// A line that could not be held or could not be read is a `-32700` and the next line.
fn serve(source: &StatusSource) -> ExitCode {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut out = std::io::stdout();
    loop {
        let response = match read_line(&mut input, MAX_LINE_BYTES) {
            Ok(Line::End) => return ExitCode::SUCCESS,
            Ok(Line::Text(line)) => {
                if line.trim().is_empty() {
                    continue;
                }
                match handle_request(&line, source, &now()) {
                    Some(response) => response,
                    None => continue,
                }
            }
            Ok(Line::TooLong(bytes)) => unreadable_line_response(&format!(
                "that line is {bytes} bytes and this server holds at most {MAX_LINE_BYTES} \
                 bytes of one request, so it did not read as JSON. The line has been \
                 discarded and this server is still answering."
            )),
            Ok(Line::NotUtf8) => unreadable_line_response(
                "that line is not UTF-8, so it did not read as JSON. The line has been \
                 discarded and this server is still answering.",
            ),
            Err(e) => return refuse(&format!("stdin could not be read: {e}")),
        };
        if let Err(e) = writeln!(out, "{response}") {
            return refuse(&format!("stdout could not be written: {e}"));
        }
        if let Err(e) = out.flush() {
            return refuse(&format!("stdout could not be flushed: {e}"));
        }
    }
}

/// One line of input, or the reason there is no line to hand over.
enum Line {
    /// A line, without its newline.
    Text(String),
    /// Stdin ended.
    End,
    /// The line was longer than the cap, in bytes. It has been read to its newline and
    /// dropped — draining rather than stopping is what keeps the NEXT line lined up with the
    /// next request, instead of the tail of this one being parsed as one.
    TooLong(usize),
    /// The line's bytes are not UTF-8. Also drained.
    NotUtf8,
}

/// [`BufRead::read_line`] with a cap, and a `TooLong` where it would have grown the buffer.
///
/// Written out rather than composed from `take` and `lines`, because the two things this
/// needs — a bound and a survivable failure — are exactly the two the standard combinators
/// do not have: `lines` grows without limit and turns invalid UTF-8 into an `Err` that ends
/// the iterator, and a `take` that ran out mid-line would hand the tail of a long line back
/// as the next request.
///
/// A line with no trailing newline at the end of input is still a line.
fn read_line(input: &mut impl BufRead, cap: usize) -> std::io::Result<Line> {
    let mut held: Vec<u8> = Vec::new();
    let mut bytes: usize = 0;
    let mut over = false;
    let mut any = false;
    loop {
        let (taken, ended) = {
            let available = match input.fill_buf() {
                Ok(available) => available,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if available.is_empty() {
                break;
            }
            any = true;
            let (chunk, ended) = match available.iter().position(|b| *b == b'\n') {
                Some(at) => (&available[..=at], true),
                None => (available, false),
            };
            bytes = bytes.saturating_add(chunk.len());
            match bytes > cap {
                // Past the cap the bytes are read and dropped: what is held is freed here
                // and nothing more is kept, so a line of any length costs the cap and no
                // more.
                true => {
                    over = true;
                    held = Vec::new();
                }
                false => held.extend_from_slice(chunk),
            }
            (chunk.len(), ended)
        };
        input.consume(taken);
        if ended {
            break;
        }
    }
    if !any {
        return Ok(Line::End);
    }
    if over {
        return Ok(Line::TooLong(bytes));
    }
    match String::from_utf8(held) {
        Ok(mut text) => {
            if text.ends_with('\n') {
                text.pop();
            }
            if text.ends_with('\r') {
                text.pop();
            }
            Ok(Line::Text(text))
        }
        Err(_) => Ok(Line::NotUtf8),
    }
}

/// One line on stderr — never on stdout, which carries the protocol — and rung 1.
fn refuse(said: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "goalcup-mcp: {said}");
    ExitCode::from(1)
}

/// This machine's clock, in the one spelling this tree gives a timestamp.
///
/// A clock before the epoch answers with the empty string, which `stale_seconds` reads as
/// "unknown" and every tool reports as offline. That is the honest answer for a machine
/// whose clock cannot be compared with the daemon's, and it is not a reason to stop
/// answering.
fn now() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(since) => format_z(since.as_secs()),
        Err(_) => String::new(),
    }
}
