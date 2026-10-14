//! The participant's approval in the browser: a run, or a login, is only made once the
//! participant has said yes on the Goal Cup site.
//!
//! [`approve`] asks the upload server for a device code (`POST /v1/device`), shows the
//! participant a short user code and an address, opens the browser at that address, and polls
//! (`POST /v1/device/token`) until the server answers approved, denied or expired. A `run`
//! approval returns the run's credential ([`RunCredential`]); a `login` approval returns the
//! account handle and nothing secret. The wire is in docs/UPLOAD-API.md.
//!
//! # What is shown, and what is not
//!
//! The participant sees the user code (`BCDF-GHJK`) and the address. The device code, which is
//! what the poll presents to collect the result, is held in memory and sent in a request body;
//! it is never printed, logged, put in a URL or repeated in an error.
//!
//! # What an approval costs, and what a failure leaves behind
//!
//! Nothing is created on this machine before [`approve`] returns an approval: the caller
//! writes its files only from the `Ok` it gets. Denied, expired, out of time, or a server that
//! cannot be reached: each is a refusal that says nothing was created. Ctrl-C needs
//! little: `goalcup login` takes the default SIGINT, which ends the process with nothing to clean
//! up, and `goalcup start`, which catches Ctrl-C into a flag (`arming`), is answered by the poll
//! loop looking at that flag.

use crate::CliError;
use crate::account::credential::RunCredential;
use crate::server::{checked_account, checked_server_url};
use crate::upload::http::{ServerClient, ServerError};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::time::{Duration, Instant};

/// The characters a user code is made of.
const USER_CODE_ALPHABET: &str = "BCDFGHJKLMNPQRSTVWXZ";

/// The poll interval when the server names none, and the bounds on one it does.
const DEFAULT_INTERVAL_SECS: u64 = 5;
const MIN_INTERVAL_SECS: u64 = 1;
const MAX_INTERVAL_SECS: u64 = 60;
/// What a `429` adds to the interval.
const SLOW_DOWN_SECS: u64 = 5;
/// The longest an approval is waited for, whatever the server says.
const MAX_WAIT_SECS: u64 = 15 * 60;

/// What the participant is approving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// `goalcup login`: the account, nothing else.
    Login,
    /// A run, in the competition or as practice.
    Run { practice: bool },
}

/// What an approval returned.
pub enum Approved {
    /// The handle of the account that approved a login.
    Login { handle: String },
    /// The credential of an approved run.
    Run(RunCredential),
}

impl std::fmt::Debug for Approved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Approved::Login { handle } => f.debug_struct("Login").field("handle", handle).finish(),
            Approved::Run(credential) => f.debug_tuple("Run").field(credential).finish(),
        }
    }
}

#[derive(Serialize)]
struct CreateBody<'a> {
    purpose: &'a str,
    mode: &'a str,
    connector_id: &'a str,
    tracer_version: &'a str,
}

/// The answer to `POST /v1/device`. No `Debug`: it holds the device code.
#[derive(Deserialize)]
struct Created {
    device_code: String,
    user_code: String,
    verification_uri_complete: String,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Serialize)]
struct PollBody<'a> {
    device_code: &'a str,
}

/// The answer to `POST /v1/device/token` when it is a success: `202` pending, or `200`.
#[derive(Deserialize)]
struct Polled {
    status: String,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    credential: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    account: Option<Account>,
}

#[derive(Deserialize)]
struct Account {
    handle: String,
}

/// Ask the server for approval of `purpose`, show the participant what to check, open
/// `open(uri)` once, and wait for the answer.
///
/// `connector_id` is the harness connector the run is for (`"none"` for a login). `open` is
/// handed the approval address and says whether a browser opened; when it did not, the
/// participant is asked to open the address themselves. Everything shown goes to `out`.
pub fn approve(
    server: &ServerClient,
    purpose: Purpose,
    connector_id: &str,
    open: &dyn Fn(&str) -> bool,
    out: &mut dyn Write,
) -> Result<Approved, CliError> {
    let (purpose_name, mode) = match purpose {
        Purpose::Login => ("login", "practice"),
        Purpose::Run { practice: false } => ("run", "competition"),
        Purpose::Run { practice: true } => ("run", "practice"),
    };
    let created: Created = server
        .json(
            "POST",
            "/v1/device",
            Some(&CreateBody {
                purpose: purpose_name,
                mode,
                connector_id,
                tracer_version: crate::CLI_VERSION,
            }),
        )
        .map_err(|e| match e {
            ServerError::Transport(why) => CliError::Refused(format!(
                "the Goal Cup server at {} could not be reached ({why}). Nothing was created; \
                 check your connection and run the command again.",
                server.base()
            )),
            other => CliError::Refused(format!(
                "{other}. Nothing was created; run the command again."
            )),
        })?;
    check_created(&created)?;
    // The code's life began when the server made it, not when the participant has read this.
    let wait = created
        .expires_in
        .unwrap_or(MAX_WAIT_SECS)
        .min(MAX_WAIT_SECS);
    let deadline = Instant::now() + Duration::from_secs(wait);

    let say = |out: &mut dyn Write, line: String| {
        writeln!(out, "{line}").map_err(|source| CliError::Io {
            path: "stdout".to_string(),
            source,
        })
    };
    say(
        out,
        format!(
            "Open {} and check that it shows the code {}.",
            created.verification_uri_complete, created.user_code
        ),
    )?;
    if !open(&created.verification_uri_complete) {
        say(
            out,
            "A browser did not open on its own; open that address yourself.".into(),
        )?;
    }
    say(out, "Waiting for approval…".into())?;

    let mut interval = Duration::from_secs(
        created
            .interval
            .unwrap_or(DEFAULT_INTERVAL_SECS)
            .clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS),
    );
    loop {
        // `goalcup start` catches Ctrl-C into a flag (`arming`); `goalcup login` does not, and
        // its default Ctrl-C ends the process. Either way nothing was created.
        if crate::arming::interrupted() {
            return Err(interrupted());
        }
        let polled: Result<Polled, ServerError> = server.json(
            "POST",
            "/v1/device/token",
            Some(&PollBody {
                device_code: &created.device_code,
            }),
        );
        match polled {
            Ok(answer) => match answer.status.as_str() {
                "pending" => {}
                "approved" => return finish(answer, purpose),
                _ => {
                    return Err(CliError::Internal(
                        "the server answered the approval poll with a status this tracker does \
                         not know"
                            .into(),
                    ));
                }
            },
            Err(ServerError::Status { status: 403, .. }) => {
                return Err(CliError::Refused(
                    "the approval was denied in the browser. Nothing was created; run the \
                     command again to start over."
                        .into(),
                ));
            }
            Err(ServerError::Status { status: 410, .. }) => return Err(expired()),
            Err(ServerError::Status { status: 429, .. }) => {
                interval += Duration::from_secs(SLOW_DOWN_SECS);
            }
            // The server may be away for a moment: keep asking until the code runs out.
            Err(ServerError::Transport(_)) => {}
            Err(ServerError::Status { status, .. }) if status >= 500 => {}
            Err(other) => return Err(CliError::Refused(format!("{other}"))),
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(expired());
        }
        sleep(interval.min(deadline - now));
    }
}

fn interrupted() -> CliError {
    CliError::Refused(
        "interrupted while waiting for approval. Nothing was created, and no run was opened."
            .into(),
    )
}

fn expired() -> CliError {
    CliError::Refused(
        "the approval code expired before it was approved. Nothing was created; run the \
         command again to start again."
            .into(),
    )
}

/// Wait `d`, in short steps, ending early when a Ctrl-C was caught meanwhile. A function of its
/// own so the one place this module sleeps is one place.
fn sleep(d: Duration) {
    const STEP: Duration = Duration::from_millis(100);
    let until = Instant::now() + d;
    loop {
        let now = Instant::now();
        if now >= until || crate::arming::interrupted() {
            return;
        }
        std::thread::sleep(STEP.min(until - now));
    }
}

fn bad_answer(what: &str) -> CliError {
    CliError::Internal(format!(
        "the server's answer to the approval request is not one this tracker accepts ({what}). \
         Nothing was created; run the command again, and report it if it keeps happening."
    ))
}

/// The server's answer to `POST /v1/device`, checked before anything of it is shown or sent.
fn check_created(created: &Created) -> Result<(), CliError> {
    let code = created.user_code.as_bytes();
    let user_code_ok = code.len() == 9
        && code.iter().enumerate().all(|(i, b)| {
            if i == 4 {
                *b == b'-'
            } else {
                USER_CODE_ALPHABET.as_bytes().contains(b)
            }
        });
    if !user_code_ok {
        return Err(bad_answer("the user code"));
    }
    let device = created.device_code.as_bytes();
    if device.len() != 64
        || !device
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        return Err(bad_answer("the device code"));
    }
    // The address is `<uri>?code=<user code>`: the part before the query must be a server
    // address (https anywhere, http only to this machine, no user info), the query plain text.
    let (base, query) = match created.verification_uri_complete.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (created.verification_uri_complete.as_str(), None),
    };
    let query_ok = query.is_none_or(|q| {
        !q.contains(['#', '@']) && !q.chars().any(|c| c.is_control() || c.is_whitespace())
    });
    if !query_ok || checked_server_url(base).is_err() {
        return Err(bad_answer("the approval address"));
    }
    Ok(())
}

/// The `200 approved` answer, as the `purpose` asked for.
fn finish(answer: Polled, asked: Purpose) -> Result<Approved, CliError> {
    match (asked, answer.purpose.as_deref()) {
        (Purpose::Login, Some("login")) => {
            let handle = answer
                .account
                .ok_or_else(|| bad_answer("no account handle"))?
                .handle;
            let handle = checked_account(&handle).map_err(|_| bad_answer("the account handle"))?;
            Ok(Approved::Login { handle })
        }
        (Purpose::Run { .. }, Some("run")) => {
            let credential = answer
                .credential
                .ok_or_else(|| bad_answer("no run credential"))?;
            let expires_at = answer
                .expires_at
                .ok_or_else(|| bad_answer("no end time for the run credential"))?;
            let credential = RunCredential::new(credential, expires_at)
                .map_err(|_| bad_answer("the run credential"))?;
            Ok(Approved::Run(credential))
        }
        _ => Err(bad_answer("an approval for something else than was asked")),
    }
}
