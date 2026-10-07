//! `goalcup auth` — the device grant, and the `0600` rule.
//!
//! This is a device-authorization flow, and it is one for two reasons that are not about
//! convenience.
//!
//! The first is `docs/RULES.md`: *"Competition terms and the short rule summary are
//! accepted on the website before a device authorization code is shown."* The acceptance
//! happens on a page this CLI does not draw, in a browser this CLI does not control, and
//! the code the participant is about to type only exists on the other side of it. A flow
//! this CLI could complete on its own would be a flow that accepted the terms on the
//! participant's behalf.
//!
//! The second is that **the CLI must never see a password.** The participant types the code
//! into their own browser, already signed in to their own account; nothing on this side ever
//! holds the credential that signed them in. `RULES.md` also requires a verified email, and
//! [`Grant::email_verified`] carries the server's answer about that — carried, not enforced
//! here, because the server issuing the grant is the authority on its own account records
//! and a second opinion in this file could only ever disagree with it.
//!
//! # What is stored, and where
//!
//! One file, [`crate::config::grant_path`] — `<config dir>/grant.json` — written `0600`
//! through [`crate::config::write_secret_file`], which creates a fresh file and renames it
//! rather than opening whatever is already there. **It never goes near a workspace.** The
//! grant is what starts runs; a copy under the folder the agent under measurement can read
//! would be that agent holding the credential that creates entries.
//!
//! The grant token reaches stdout under no circumstance: [`run`] prints the user code and
//! the verification URI and nothing else, [`Grant`]'s `Debug` redacts the token, and the
//! refusals below name the server and the URL rather than quoting a body back.
//!
//! # The one route in this tree that leaves the machine
//!
//! Every other client in `goalcup` speaks to `127.0.0.1`. This one speaks to the /goal Cup
//! server, and what crosses that wire is the device code on the way out and the grant on
//! the way back — the credential that creates entries, in both directions. Three rules fall
//! out of that, and all three are enforced before or around the request rather than
//! documented next to it:
//!
//! 1. **[`check_server_url`] runs before any socket is opened.** A non-loopback `http://`
//!    URL is refused naming the scheme, because sending the device code in the clear is not
//!    something a participant can be asked to consent to by typo. Loopback `http://` stays
//!    allowed: it is this crate's own tests and the development server, and nothing there
//!    leaves the machine.
//! 2. **TLS is linked.** `crates/goalcup-cli/Cargo.toml` turns ureq's `tls` feature on for
//!    exactly this. Without it an `https://` URL comes back as ureq's "unknown scheme",
//!    which this command would have reported as a server that did not answer.
//! 3. **Redirects are not followed** (`redirects(0)`). The device code rides in
//!    [`HEADER_DEVICE_CODE`], and ureq strips `Authorization` across a redirect but knows
//!    nothing about that header: a `302` would have re-sent it to whatever host the
//!    `Location` named. A `3xx` is a refusal here instead.
//!
//! # Nothing here talks to the daemon
//!
//! A grant is what `goalcup start` later hands the daemon on `WorkspaceOpts.grant_token`,
//! and the daemon is what exchanges it. This module reads and writes one file and speaks to
//! the /goal Cup server; it opens no control connection and reads no `control.json`.

use crate::CliError;
use crate::config::{grant_path, write_secret_file};
use goalcup_protocol::control::{ErrorBody, format_z};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

/// Where the server is, when `--server` was not given. Read by `main.rs`, which is the only
/// place a flag and an environment variable are reconciled.
pub const SERVER_ENV: &str = "GOALCUP_SERVER";

/// `POST` — hand back a device code, a user code and the page to type it on.
///
/// Versioned, like every other route this tree speaks: the daemon's control API is `/v0/…`
/// and a server that changes this shape changes the prefix rather than the meaning of an
/// unversioned path an older `goalcup` is still posting to.
pub const ROUTE_DEVICE: &str = "/v0/device";

/// `POST` — has the code been entered yet? Versioned; see [`ROUTE_DEVICE`].
pub const ROUTE_TOKEN: &str = "/v0/token";

/// Where the device code rides on [`ROUTE_TOKEN`]: a request header, and **neither a
/// request body nor a query string.**
///
/// Not a query string, because a URL is what ends up in a proxy log, a browser history and
/// a shell's `history` file, and the device code is the thing that becomes a grant.
///
/// Not a body, and that half is a ureq 2.12 hazard rather than a preference. A server that
/// answers before it has read the request body leaves those bytes unread in its receive
/// queue, and closing then sends an RST rather than a FIN. ureq takes a length-delimited
/// body by handing the socket back to its pool first — `reset()`, which is a `setsockopt`
/// — and that syscall fails on a socket the peer has reset. The failure surfaces as
/// `Error encountered in a header: Invalid argument (os error 22)` and, on the branch that
/// buffers a short body, as an outright **panic inside the library**
/// (`ureq-2.12.1/src/response.rs:403`, an `.expect()` the crate itself flags as a TODO).
/// This CLI does not panic; a request with no body cannot reach that path at all, because
/// there is nothing for a peer to leave unread.
pub const HEADER_DEVICE_CODE: &str = "X-Goalcup-Device-Code";

/// How many times `goalcup auth` asks before it gives up and says so.
///
/// A device code lives about ten minutes and the server sets the polling interval; at the
/// usual five seconds this is that window and a little over. The bound exists so a server
/// answering `authorization_pending` forever ends as a refusal with the URL printed again
/// rather than as a terminal nobody can get out of.
pub const DEFAULT_POLL_LIMIT: u32 = 140;

/// The two answers that mean "keep asking" rather than "this will never work".
const PENDING: &str = "authorization_pending";
const SLOW_DOWN: &str = "slow_down";

/// What `slow_down` adds to the wait, and the ceiling the server cannot push past. A server
/// that asked for an hour between polls would otherwise hang this command for an hour.
const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);
const MAX_INTERVAL_SECONDS: u64 = 30;

/// The wall clock a device grant is bounded by, whatever the server asks for.
///
/// A device code lives about ten minutes ([`DEFAULT_POLL_LIMIT`] says so), and
/// [`DEFAULT_POLL_LIMIT`] bounds the number of requests rather than the time they span. A
/// server answering `slow_down` walks the wait to [`MAX_INTERVAL_SECONDS`] and would then
/// hold the participant's terminal for a hundred and forty polls at thirty seconds — about
/// seventy minutes on a code that expired an hour before. So the deadline is read at the top
/// of the loop, and repeated `slow_down` cannot stretch the window it is answering into.
pub const POLL_DEADLINE: Duration = Duration::from_secs(10 * 60);

/// What the wait between polls is when the server did not name one.
///
/// **Not zero.** An absent `interval` that meant zero would mean "as fast as this CLI
/// likes": `poll_limit` back-to-back requests finishing in well under a second, so the
/// ten-minute window [`DEFAULT_POLL_LIMIT`] is counted for would not exist, and a server that
/// omitted the field would be hammered. Five seconds is RFC 8628's own default and the number
/// that bound is counted against.
pub const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 5;

/// The most of any response body this command reads.
///
/// Taken from [`crate::config::CONTROL_FILE_MAX_BYTES`] so the two caps cannot drift: the
/// reason is the same one, which is that the size of a document this process did not write
/// is not this process's to trust. A device-flow answer is a handful of short fields.
const MAX_RESPONSE_BYTES: u64 = crate::config::CONTROL_FILE_MAX_BYTES;

/// Loopback is not where this one goes, so both halves are bounded: a server that accepts
/// the connection and then says nothing must not hold the terminal open.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

// ------------------------------------------------------------------ the document

/// The grant this machine starts runs with, as it is stored.
///
/// `Serialize`/`Deserialize` are the file format: this struct **is** `grant.json`, and a
/// field added later is added here. `server` is recorded with the rest of it so a
/// participant who authenticated against one server and points `--server` at another gets a
/// grant that says which one issued it rather than two indistinguishable files.
#[derive(Clone, Serialize, Deserialize)]
pub struct Grant {
    /// The server that issued it, as it was given — trailing slash already trimmed.
    pub server: String,
    /// The secret. Never printed, never logged, never in an error message.
    pub grant_token: String,
    /// Who the server says this is.
    pub subject: String,
    /// `RULES.md`: a verified email is required. The server's answer, carried and not
    /// second-guessed here.
    pub email_verified: bool,
    /// What the grant is good for, in the server's own words.
    pub scopes: Vec<String>,
    /// When the server says it stops working.
    pub expires_at: String,
    /// When this file was written, RFC-3339 UTC — the one spelling every timestamp in this
    /// tree uses (`goalcup_protocol::control::format_z`).
    pub stored_at: String,
}

/// Every field as a derive would print it, except `grant_token`, which prints
/// `<redacted>`.
///
/// This struct holds the credential that creates entries. A `{:?}` that carried it — into
/// an error, a `goalcup doctor` dump, a log line somebody added later — would hand it to
/// whoever reads that output. `ControlFile` redacts the daemon's bearer for the same
/// reason and in the same shape.
impl std::fmt::Debug for Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grant")
            .field("server", &self.server)
            .field("grant_token", &"<redacted>")
            .field("subject", &self.subject)
            .field("email_verified", &self.email_verified)
            .field("scopes", &self.scopes)
            .field("expires_at", &self.expires_at)
            .field("stored_at", &self.stored_at)
            .finish()
    }
}

/// What one `goalcup auth` run was asked to do.
pub struct AuthOptions {
    /// The /goal Cup server, e.g. `https://…`. Resolved by `main.rs` from `--server` or
    /// [`SERVER_ENV`].
    pub server: String,
    /// Hand the verification URI to the participant's browser. `--no-open` clears it, which
    /// is the headless machine's path: the URL and the code are printed either way.
    pub open_browser: bool,
    /// How many times to ask before refusing. See [`DEFAULT_POLL_LIMIT`].
    pub poll_limit: u32,
    /// The wall clock the whole poll is bounded by, whatever the server asks for. See
    /// [`POLL_DEADLINE`], which is what `main.rs` puts here; a field and not a flag, because
    /// a participant has nothing to decide about it and a test needs a tiny one.
    pub poll_deadline: Duration,
}

// ------------------------------------------------------------------ the flow

/// Run the device flow against `opts.server` and store the grant under `cfg`.
///
/// `out` is where the user code and the verification URI go — a parameter and not
/// `println!`, so a test can read exactly what a participant would see and assert that the
/// grant is not in it.
pub fn run(
    cfg: &Path,
    opts: &AuthOptions,
    out: &mut dyn std::io::Write,
) -> Result<Grant, CliError> {
    let base = opts.server.trim_end_matches('/').to_string();
    // Both of these are decided before a socket is opened. A URL this command must not
    // send a device code to is refused with no request made at all, and a poll budget of
    // zero is a malformed flag rather than a flow that asks nothing and then reports that
    // the code was never entered.
    check_server_url(&base)?;
    if opts.poll_limit == 0 {
        return Err(CliError::Refused(format!(
            "--poll-limit 0 asks `goalcup auth` to never ask the server whether the code \
             was entered. Give at least 1, or leave the flag off for {DEFAULT_POLL_LIMIT}."
        )));
    }
    let loopback = is_loopback_url(&base);
    let agent = ureq::AgentBuilder::new()
        // No pooling. Every request here is one of a handful, seconds apart, and a pooled
        // connection the server closed between two of them is a transport error this
        // command would have reported as "the server did not answer".
        .max_idle_connections(0)
        .max_idle_connections_per_host(0)
        // The device code travels in a request header, and ureq's redirect handling strips
        // `Authorization` and `Cookie` and nothing else — a `302` would have re-sent
        // `X-Goalcup-Device-Code` to whatever host the `Location` named. `post` turns a
        // `3xx` into a refusal naming the server instead.
        .redirects(0)
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build();

    let device = start_device_flow(&agent, &base)?;
    say(
        out,
        "Accept the /goal Cup terms in your browser, then enter this code:",
    )?;
    say(out, &format!("  {}", device.verification_uri))?;
    say(out, &format!("  code: {}", device.user_code))?;
    // Checked in [`start_device_flow`], so nothing here can refuse. Not a failure either
    // way: the URL and the code are already on the screen, which is the whole of what a
    // headless machine gets, and this run can still finish.
    if opts.open_browser
        && let Some(note) = crate::browser::open_page(
            &device.verification_uri,
            crate::browser::PageKind::Verification,
            &crate::browser::system_opener,
        )?
    {
        say(out, &note)?;
    }
    say(out, "Waiting for the code to be entered…")?;

    let granted = poll_for_grant(
        &agent,
        &base,
        &device,
        opts.poll_limit,
        opts.poll_deadline,
        loopback,
    )?;
    let grant = Grant {
        server: base,
        grant_token: granted.grant_token,
        subject: granted.subject,
        email_verified: granted.email_verified,
        scopes: granted.scopes,
        expires_at: granted.expires_at,
        stored_at: format_z(wall_clock_secs()),
    };
    write_secret_file(&grant_path(cfg), &serde_json::to_vec_pretty(&grant)?)?;
    say(out, "Stored.")?;
    Ok(grant)
}

/// The grant this machine holds, or `None` if it has never authenticated.
///
/// A missing file is `None` and not an error: "you are not signed in" is an answer
/// `goalcup doctor` and `goalcup start` both came for, and a file that cannot be read for
/// any other reason is a [`CliError::Io`] naming the path.
pub fn load_grant(cfg: &Path) -> Result<Option<Grant>, CliError> {
    let path = grant_path(cfg);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(serde_json::from_str(&text)?)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(CliError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// Remove the stored grant. `true` if there was one, `false` if there was not.
///
/// Removing a file that is not there is not a failure — a participant who runs this twice
/// is asking for the same end state both times, and the boolean is how a caller says
/// "there was nothing to remove" without an error nobody can act on.
pub fn logout(cfg: &Path) -> Result<bool, CliError> {
    let path = grant_path(cfg);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(CliError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

// ------------------------------------------------------------------ the two routes

/// `POST /device` — the code the participant is about to read off their own screen.
fn start_device_flow(agent: &ureq::Agent, base: &str) -> Result<DeviceBody, CliError> {
    let url = format!("{base}{ROUTE_DEVICE}");
    let (status, text) = post(agent, &url, None)?;
    if !(200..300).contains(&status) {
        let (code, message) = refusal(&text);
        return Err(CliError::Refused(format!(
            "{url} refused to start a device authorization: {message} ({code})"
        )));
    }
    let device: DeviceBody = serde_json::from_str(&text)?;
    if device.verification_uri.is_empty() || device.user_code.is_empty() {
        return Err(CliError::Internal(format!(
            "{url} answered without a verification URI and a user code, so there is nothing \
             to show the participant"
        )));
    }
    // The one string in this flow that came off the network and is then handed to the
    // participant's browser. [`check_server_url`] vets the server
    // before the socket; nothing vetted the URI the response body asks the browser to
    // visit, so a typo'd or hostile server needed one answer to invoke a local scheme
    // handler. Refused before it is printed as well as before it is opened: a URI this
    // command will not open is not one it will tell a participant to go to.
    crate::browser::check_page_url(
        &device.verification_uri,
        crate::browser::PageKind::Verification,
    )?;
    Ok(device)
}

/// `POST /token`, up to `poll_limit` times, until the code has been entered.
///
/// The bound is the whole point: a server that answers `authorization_pending` forever ends
/// here as a [`CliError::Refused`] naming the page again, and not as a hung terminal.
fn poll_for_grant(
    agent: &ureq::Agent,
    base: &str,
    device: &DeviceBody,
    poll_limit: u32,
    poll_deadline: Duration,
    loopback: bool,
) -> Result<GrantBody, CliError> {
    let url = format!("{base}{ROUTE_TOKEN}");
    let mut wait = poll_interval(device.interval, loopback);
    let deadline = std::time::Instant::now() + poll_deadline;
    for round in 0..poll_limit {
        if round > 0 {
            std::thread::sleep(wait);
        }
        // Read at the top of the loop, so a `slow_down` that walked the wait to its ceiling
        // cannot buy one more request past the window. See [`POLL_DEADLINE`].
        if std::time::Instant::now() >= deadline {
            return Err(CliError::Refused(format!(
                "the code was never entered within {}s, which is as long as a device code \
                 lives. Run `goalcup auth` again: it prints a fresh code, and this one is \
                 no longer worth waiting on.",
                poll_deadline.as_secs()
            )));
        }
        let (status, text) = post(agent, &url, Some(&device.device_code))?;
        // A `5xx` is the server saying it is broken, and the body behind one is a proxy's
        // HTML as often as it is JSON. Naming the server is rung 1 — wait and run `goalcup
        // auth` again — where letting serde fail on the body would have left it under rung
        // 3, which says this CLI is the thing that is broken.
        if (500..600).contains(&status) {
            return Err(CliError::Refused(format!(
                "{url} answered {status}. That is the /goal Cup server failing, not the \
                 code: wait a few minutes and run `goalcup auth` again."
            )));
        }
        let answer: TokenBody = serde_json::from_str(&text)?;
        if let Some(grant_token) = answer.grant_token {
            let (Some(subject), Some(expires_at)) = (answer.subject, answer.expires_at) else {
                return Err(CliError::Internal(format!(
                    "{url} issued a grant without a subject and an expiry, which is not a \
                     grant this build can store"
                )));
            };
            return Ok(GrantBody {
                grant_token,
                subject,
                email_verified: answer.email_verified,
                scopes: answer.scopes,
                expires_at,
            });
        }
        let Some(error) = answer.error else {
            return Err(CliError::Internal(format!(
                "{url} answered with neither a grant nor an error"
            )));
        };
        let (code, message) = error_of(&error, &text);
        match code.as_str() {
            PENDING => {}
            // The server asking to be asked less often. Honoured, up to the ceiling.
            SLOW_DOWN => {
                wait = (wait + SLOW_DOWN_STEP).min(Duration::from_secs(MAX_INTERVAL_SECONDS));
            }
            // Anything else is the server saying this will never work — an expired code, a
            // participant who declined. Asking again would only spend the rest of the
            // budget on the same answer.
            _ => {
                return Err(CliError::Refused(format!(
                    "{url} refused the device code: {message} ({code}). Start again with \
                     `goalcup auth`, accept the terms at {} and enter the code it prints.",
                    device.verification_uri
                )));
            }
        }
    }
    Err(CliError::Refused(format!(
        "the code was never entered. Accept the /goal Cup terms at {} and enter {}, then \
         run `goalcup auth` again.",
        device.verification_uri, device.user_code
    )))
}

// ------------------------------------------------------------------ wire shapes

/// What `POST /device` answers.
#[derive(Deserialize)]
struct DeviceBody {
    device_code: String,
    user_code: String,
    verification_uri: String,
    /// Seconds between polls, or `None` when the server did not say.
    ///
    /// `Option` and not a `#[serde(default)]` `u64`, because those two are not the same
    /// answer: the default would make an absent field mean zero, and zero means "ask again
    /// immediately". [`poll_interval`] is where the difference is decided.
    interval: Option<u64>,
}

/// What `POST /token` answers — a grant, or an error, and this shape reads both.
///
/// Every field is optional because the two answers share one route, and share a status code
/// on some servers and not on others. Which of the two arrived is decided by the presence of
/// `grant_token`, never by the HTTP status.
#[derive(Deserialize)]
struct TokenBody {
    #[serde(default)]
    grant_token: Option<String>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    email_verified: bool,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    expires_at: Option<String>,
    /// A bare `"authorization_pending"` from an OAuth-shaped server, or the object
    /// [`ErrorBody`] carries. [`error_of`] reads both.
    #[serde(default)]
    error: Option<serde_json::Value>,
}

/// A grant that arrived complete, before it is dated and stored.
struct GrantBody {
    grant_token: String,
    subject: String,
    email_verified: bool,
    scopes: Vec<String>,
    expires_at: String,
}

// ------------------------------------------------------------------ plumbing

/// How long to wait between two polls, given what the server asked for.
///
/// - **`None`** — the server said nothing: [`DEFAULT_POLL_INTERVAL_SECONDS`]. An absent
///   field is not a request to be asked as fast as the network allows.
/// - **`Some(0)`** — honoured **only** against a loopback server, which is this crate's own
///   mock and the development server. From anywhere else it is treated as unsaid, because a
///   zero from across the network turns [`DEFAULT_POLL_LIMIT`] into a burst rather than the
///   ten-minute window it was counted for.
/// - **`Some(n)`** — honoured up to [`MAX_INTERVAL_SECONDS`]. A server that asked for an
///   hour between polls would otherwise hang this command for an hour.
///
/// `loopback` is a parameter and not a re-derivation from the URL so this is one pure
/// function with a table of cases, testable without a socket.
pub fn poll_interval(interval: Option<u64>, loopback: bool) -> Duration {
    match interval {
        Some(0) if loopback => Duration::ZERO,
        Some(seconds) if seconds > 0 => Duration::from_secs(seconds.min(MAX_INTERVAL_SECONDS)),
        // `None`, and a `Some(0)` from a server that is not on this machine.
        _ => Duration::from_secs(DEFAULT_POLL_INTERVAL_SECONDS),
    }
}

/// `POST url`, carrying `device_code` in [`HEADER_DEVICE_CODE`] when there is one, and
/// returning the status and the response text.
///
/// **No request body**, on either route — see [`HEADER_DEVICE_CODE`] for the whole of why.
///
/// A `4xx` is **not** an error here: the device flow's "keep waiting" answer is a `400` on
/// an OAuth-shaped server and a `200` on others, and a helper that turned one of those into
/// a failure would make this command's behaviour depend on which. The callers read the
/// status for what they each care about — `start_device_flow` wants a `2xx`,
/// `poll_for_grant` singles out a `5xx`.
///
/// A `3xx` **is** an error, and it is one here rather than in either caller, because the
/// reason is about this function: the agent is built with `redirects(0)`, so a `3xx` is
/// handed back rather than followed, and following it would have re-sent
/// [`HEADER_DEVICE_CODE`] to whatever host the `Location` named.
///
/// A transport failure is a [`CliError::Refused`], because "the server did not answer" is
/// fixed by checking the connection or `--server` and not by anything inside this binary.
///
/// The body is read through a [`MAX_RESPONSE_BYTES`] `take`. Nothing about the size of what
/// the far end sends is this process's to trust, and a server answering a gigabyte would
/// otherwise be a gigabyte in this process's memory before serde saw the first byte.
fn post(
    agent: &ureq::Agent,
    url: &str,
    device_code: Option<&str>,
) -> Result<(u16, String), CliError> {
    use std::io::Read;

    let mut request = agent.post(url).set("Accept", "application/json");
    if let Some(code) = device_code {
        request = request.set(HEADER_DEVICE_CODE, code);
    }
    let response = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(ureq::Error::Transport(t)) => {
            return Err(CliError::Refused(format!(
                "{url} did not answer: {t}. Check the connection, or point --server (or \
                 ${SERVER_ENV}) at the /goal Cup server."
            )));
        }
    };
    let status = response.status();
    if (300..400).contains(&status) {
        return Err(CliError::Refused(format!(
            "{url} answered {status} and pointed somewhere else. `goalcup auth` does not \
             follow a redirect, because the device code rides in a request header and \
             following one would hand it to whatever host the server named. Point --server \
             (or ${SERVER_ENV}) at the /goal Cup server itself."
        )));
    }
    // One byte past the cap, so a body exactly at it still reads and anything larger is
    // caught having read the cap and no more — the shape `read_control_file` uses.
    let mut text = String::new();
    let read = response
        .into_reader()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|source| CliError::Io {
            path: url.to_string(),
            source,
        })?;
    if read as u64 > MAX_RESPONSE_BYTES {
        return Err(CliError::Refused(format!(
            "{url} answered with more than {MAX_RESPONSE_BYTES} bytes, which is more than \
             any device-flow answer. Point --server (or ${SERVER_ENV}) at the /goal Cup \
             server."
        )));
    }
    Ok((status, text))
}

// ------------------------------------------------------------------ where it may connect

/// Refuse a URL this command must not send a device code to, **before any request**.
///
/// `https://` anywhere is fine. `http://` is fine to a loopback host and to nothing else:
/// a plain `http://` URL to a server across the network would put the device code — in a
/// request header — and the grant that comes back on the wire in the clear, and no `--yes`
/// exists for that because a participant cannot consent to it by typing the wrong prefix.
/// Loopback `http://` stays allowed because that is this crate's own mock server and the
/// development server, neither of which leaves the machine.
///
/// Any other scheme is refused naming it, rather than handed to ureq to answer with
/// "unknown scheme" — which is a transport error this command would have reported as a
/// server that did not answer.
///
/// It is a refusal (rung 1) in all three shapes: the fix is a different `--server`.
pub fn check_server_url(base: &str) -> Result<(), CliError> {
    let Some((scheme, rest)) = base.split_once("://") else {
        return Err(CliError::Refused(format!(
            "{base} names no scheme, so `goalcup auth` cannot tell whether talking to it \
             would send the device code in the clear. Give an `https://` URL for --server \
             (or ${SERVER_ENV})."
        )));
    };
    let host = host_of(authority_of(rest));
    match scheme.to_ascii_lowercase().as_str() {
        "https" => Ok(()),
        "http" if is_loopback_host(host) => Ok(()),
        "http" => Err(CliError::Refused(format!(
            "{base} is `http://` to {host}, which is not a loopback address. `goalcup auth` \
             sends the device code and receives the grant over this connection, so a \
             cleartext `http://` to anything but this machine is refused before any request \
             is made. Use `https://`."
        ))),
        other => Err(CliError::Refused(format!(
            "{base} uses the `{other}://` scheme, which `goalcup auth` does not speak. Use \
             `https://`, or `http://` to a loopback address for a development server."
        ))),
    }
}

/// Is `base` pointed at this machine? Read by [`run`] once, and carried into
/// [`poll_interval`] as the one place a `Some(0)` interval is honoured.
///
/// A URL that got past [`check_server_url`] and is not loopback is `https://`, so this is
/// "the development server and the tests" and not a second security decision.
fn is_loopback_url(base: &str) -> bool {
    match base.split_once("://") {
        Some((_, rest)) => is_loopback_host(host_of(authority_of(rest))),
        None => false,
    }
}

/// Everything before the first `/`, `?` or `#` — the authority, userinfo and port included.
fn authority_of(rest: &str) -> &str {
    match rest.find(['/', '?', '#']) {
        Some(at) => &rest[..at],
        None => rest,
    }
}

/// The host out of an authority: `user:pw@[::1]:8080` is `[::1]`.
///
/// Userinfo is dropped from the front and the port from the back, and a bracketed IPv6
/// literal is kept whole so the colons inside it are not mistaken for a port separator.
fn host_of(authority: &str) -> &str {
    let after_userinfo = match authority.rsplit_once('@') {
        Some((_, host)) => host,
        None => authority,
    };
    if after_userinfo.starts_with('[') {
        return match after_userinfo.find(']') {
            Some(close) => &after_userinfo[..=close],
            None => after_userinfo,
        };
    }
    match after_userinfo.split_once(':') {
        Some((host, _)) => host,
        None => after_userinfo,
    }
}

/// `localhost`, or an address the operating system would route back to this machine.
///
/// `127.0.0.1` is the usual one and it is not the only one: the whole of `127.0.0.0/8` is
/// loopback, and so is `::1`. `std::net::IpAddr::is_loopback` is the rule rather than a
/// string comparison this file would have had to get right twice.
fn is_loopback_host(host: &str) -> bool {
    let bare = host.strip_prefix('[').unwrap_or(host);
    let bare = bare.strip_suffix(']').unwrap_or(bare);
    if bare.eq_ignore_ascii_case("localhost") {
        return true;
    }
    bare.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// The `code` and `message` behind a failure, whichever of the two shapes it arrived in: a
/// bare `{"error":"authorization_pending"}`, or the [`ErrorBody`] every route in this tree's
/// own control API answers with.
fn error_of(value: &serde_json::Value, text: &str) -> (String, String) {
    if let serde_json::Value::String(code) = value {
        return (code.clone(), code.clone());
    }
    refusal(text)
}

/// An [`ErrorBody`] read for its `code` and `message`, or a stand-in pair when the body is
/// not one.
///
/// The two named fields and not the whole body: a server that answered something this CLI
/// does not understand gets a sentence saying so, rather than having its bytes printed into
/// a participant's terminal on the chance that they mean something.
fn refusal(text: &str) -> (String, String) {
    match serde_json::from_str::<ErrorBody>(text) {
        Ok(body) => (body.error.code, body.error.message),
        Err(_) => (
            "unknown".to_string(),
            "the server refused without saying why".to_string(),
        ),
    }
}

/// One line to `out`. A write that fails is a [`CliError::Io`] naming the stream rather
/// than a line silently dropped: the user code is the whole of what this command produces,
/// and a participant who never saw it has nothing to type.
fn say(out: &mut dyn std::io::Write, line: &str) -> Result<(), CliError> {
    writeln!(out, "{line}").map_err(|source| CliError::Io {
        path: "the command's output".to_string(),
        source,
    })
}

/// The wall clock in seconds. A clock somehow before the epoch reads as the epoch rather
/// than panicking; nothing downstream would believe either number.
fn wall_clock_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
