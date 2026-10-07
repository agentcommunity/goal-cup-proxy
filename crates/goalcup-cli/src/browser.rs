//! The one place this CLI hands a URL to the participant's browser.
//!
//! # Why there is exactly one
//!
//! Four commands open a page: `goalcup ui` and `goalcup resume` open the status URL out of
//! the entry cache, `goalcup start` opens the one `create_workspace` just answered with, and
//! `goalcup auth` opens the `verification_uri` a device-flow server put in a response body.
//! `open::that` hands its argument to the platform's opener, which will follow an
//! `https://` to the network, a `file://` to the disk and a registered custom scheme
//! somewhere worse — so every one of those four strings decides what a participant's
//! machine does next, and not one of them was written by this process.
//!
//! A rule that holds at one of four call sites is not a rule, so the check is [`open_page`]
//! and the four sites call it.
//!
//! # Two shapes, because two of these URLs are different things
//!
//! [`PageKind::Daemon`] is a page this machine serves: the daemon binds a loopback address,
//! builds the URL from it, and the browser is being sent back to the same machine it is
//! running on. So: `http://`, a loopback host written out, and an explicit port. Nothing
//! else is a page this daemon serves, and an `https://` in that field is a record that was
//! edited or a process that answered on the control port before the daemon did.
//!
//! [`PageKind::Verification`] is the device-flow page, which is legitimately somewhere else:
//! the participant accepts the /goal Cup terms on the /goal Cup server. So `https://` to a
//! host that is not this machine — the real case — or the loopback `http://` shape, which is
//! the development server and this crate's own mock. `https://` to loopback is not on the
//! list because nothing in this tree serves one and a rule with an unused third arm is a
//! rule nobody checked.
//!
//! Both lists are closed. Anything outside them is a refusal (rung 1) that names the URL, so
//! the participant sees exactly what their machine was about to be handed.
//!
//! # The host list is written out
//!
//! `127.0.0.1`, `::1`, `localhost` — rather than derived from `IpAddr::is_loopback`, because
//! this is the one place `localhost` is allowed: the daemon builds its page URL from the
//! address it is listening on, a participant may have one that spells the name, and the
//! string is going to a browser rather than onto a request carrying the control bearer.
//! `daemon::is_loopback_base` refuses the name for exactly the opposite reason, and the two
//! rules are different on purpose.
//!
//! Userinfo is a refusal and not something to strip: `http://127.0.0.1@evil.example:80/` has
//! a host that is not this machine, and a reader who saw the first token in it would think
//! otherwise. The port must be there and must be a number a listener could hold — an
//! implicit `:80` is not a page this daemon serves.
//!
//! Parsed by hand rather than by pulling a URL crate in, for the reason
//! `daemon::is_loopback_base` parses its own by hand: the whole question is a handful of
//! facts about a short string, and a dependency added to answer it would be a dependency the
//! shipped binary carries for one `if`.

use crate::CliError;

/// Which of the two rules a URL is being held to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    /// A page the daemon on this machine serves: `WorkspaceInfo.status_url`, and the copy
    /// of it in the entry cache.
    Daemon,
    /// A device-flow `verification_uri`, which came off the network from the server
    /// `--server` (or `$GOALCUP_SERVER`) named.
    Verification,
}

impl PageKind {
    /// The shape this kind allows, in the words the refusal uses.
    fn shape(self) -> &'static str {
        match self {
            PageKind::Daemon => {
                "the status page is served by the daemon on this machine, so the URL must \
                 read `http://` then a loopback host written out — 127.0.0.1, [::1] or \
                 localhost — then an explicit port"
            }
            PageKind::Verification => {
                "the /goal Cup terms are accepted on the /goal Cup server, so the URL must \
                 read `https://` then a host that is not this machine — or the development \
                 server's shape, `http://` then a loopback host and an explicit port"
            }
        }
    }

    /// What a URL outside the shape means, which is not the same fact for the two kinds.
    fn cause(self) -> &'static str {
        match self {
            PageKind::Daemon => {
                "A record naming anything else was edited after `goalcup start` wrote it, or \
                 was written while something other than the daemon held the control port."
            }
            PageKind::Verification => {
                "A server naming anything else is not the /goal Cup server: point --server \
                 (or $GOALCUP_SERVER) at it and run `goalcup auth` again."
            }
        }
    }
}

/// The opener the shipped binary hands over. A function rather than a closure at four call
/// sites, so `open::that` is named once in this crate.
pub fn system_opener(url: &str) -> std::io::Result<()> {
    open::that(url)
}

/// Refuse a URL that must not be handed to a browser, naming it.
///
/// Asked at the moment the string enters a command as well as at the moment it is opened:
/// `goalcup start` asks before the URL is written into the entry cache, `goalcup resume`
/// before it asks the daemon anything, and `goalcup auth` before the URI is printed as the
/// page to visit. A URL this CLI will not open is not one it will hand over in a sentence
/// either, and refusing where the value enters is what keeps a bad one out of the record.
pub fn check_page_url(url: &str, kind: PageKind) -> Result<(), CliError> {
    if allowed(url, kind) {
        return Ok(());
    }
    Err(CliError::Refused(format!(
        "{url} is not a page `goalcup` will hand to a browser: {}. {}",
        kind.shape(),
        kind.cause()
    )))
}

/// Check, then open.
///
/// `Ok(None)` when the opener took it. `Ok(Some(note))` when no browser could be opened,
/// which is **not** a failure: every caller has already printed the URL, which is the whole
/// of what a headless machine gets, and the note says why nothing appeared.
pub fn open_page(
    url: &str,
    kind: PageKind,
    opener: &dyn Fn(&str) -> std::io::Result<()>,
) -> Result<Option<String>, CliError> {
    check_page_url(url, kind)?;
    match opener(url) {
        Ok(()) => Ok(None),
        Err(e) => Ok(Some(format!("  (no browser could be opened: {e})"))),
    }
}

/// Whether `url` is inside `kind`'s closed list. See the module header for both.
fn allowed(url: &str, kind: PageKind) -> bool {
    if let Some(rest) = url.strip_prefix("http://") {
        return match host_and_port(rest) {
            Some((host, port)) => is_loopback_name(host) && port > 0,
            None => false,
        };
    }
    if kind == PageKind::Verification
        && let Some(rest) = url.strip_prefix("https://")
    {
        return match host_and_port_optional(rest) {
            Some((host, _)) => !is_loopback_name(host),
            None => false,
        };
    }
    false
}

/// The authority: everything before the first `/`, `?` or `#`.
fn authority_of(rest: &str) -> &str {
    match rest.find(['/', '?', '#']) {
        Some(at) => &rest[..at],
        None => rest,
    }
}

/// The host and the explicit port out of an authority, or `None` when there is userinfo, no
/// port, or a port no listener could hold.
fn host_and_port(rest: &str) -> Option<(&str, u16)> {
    let (host, port) = split_authority(authority_of(rest))?;
    let port = port?.parse::<u16>().ok()?;
    Some((host, port))
}

/// The host out of an authority, with the port optional — `https://` names a default one.
fn host_and_port_optional(rest: &str) -> Option<(&str, Option<u16>)> {
    let (host, port) = split_authority(authority_of(rest))?;
    let port = match port {
        Some(port) => Some(port.parse::<u16>().ok()?),
        None => None,
    };
    if host.is_empty() {
        return None;
    }
    Some((host, port))
}

/// `host` and, where one is written, the port text. `None` for an authority carrying
/// userinfo or an unclosed IPv6 literal.
fn split_authority(authority: &str) -> Option<(&str, Option<&str>)> {
    if authority.contains('@') {
        return None;
    }
    if let Some(inside) = authority.strip_prefix('[') {
        let (host, tail) = inside.split_once(']')?;
        return match tail.strip_prefix(':') {
            Some(port) => Some((host, Some(port))),
            None if tail.is_empty() => Some((host, None)),
            None => None,
        };
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host, Some(port))),
        None => Some((authority, None)),
    }
}

/// The three spellings of this machine a browser understands.
fn is_loopback_name(host: &str) -> bool {
    host == "127.0.0.1" || host == "::1" || host.eq_ignore_ascii_case("localhost")
}
