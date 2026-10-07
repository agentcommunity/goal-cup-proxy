//! `goalcup ui` — hands the participant's browser the URL the daemon gave, and builds none
//! of its own.
//!
//! # One place decides how the page is addressed
//!
//! `WorkspaceInfo.status_url` is the page with the entry id already in its query, because
//! `/v0/entries/current` is authenticated and a browser has no token — the daemon is what
//! tells the page which entry it is looking at. This command opens that string byte for
//! byte. A CLI that appended `?entry=<id>` itself would be the second place that decides
//! how the page is addressed, and the first one to drift from the daemon that serves it.
//!
//! # The URL comes off the disk, not out of the daemon
//!
//! Nothing on the control API's read surface carries a status URL: `StatusPayload` has no
//! such field and neither does `EntrySummary`. The one struct that does comes back from
//! `create_workspace`, which is not a call this command may make. So the URL is read from
//! the entry cache `goalcup start` wrote (see [`crate::entries`]), which is the same seam
//! `goalcup status` reads its pinned run key from and the same reason: `ui` is a fresh
//! process and `start` is long gone. An entry this machine has no record of has no URL
//! either, and that is a refusal (rung 1) rather than a URL this command made up.
//!
//! # The URL is checked before a browser is handed it
//!
//! The cache is a file on the participant's disk, and the string in it was written by a
//! `goalcup start` that believed whatever answered on the control port at the time. Neither
//! is a promise. `open::that` hands its argument to the platform's opener, which will follow
//! `https://evil.example/...` to the network and a `file://` or a custom scheme somewhere
//! worse, so an edited record — or a rogue process that bound the control port before the
//! daemon did — would be choosing what this command opens.
//!
//! So the string is checked before anything is done with it, by
//! [`crate::browser::check_page_url`]: `http://`, a loopback host written out, and an
//! explicit port. Anything else is a refusal (rung 1) that names the URL, so the participant
//! sees exactly what their machine had on file. The check guards `--print` as well as the
//! open: a URL this command will not open is not one it will hand over as "the page for your
//! run" either, and the refusal prints it anyway, so nothing is hidden by guarding both.
//!
//! **The rule is not written here.** [`crate::browser`] holds it and the four commands that
//! open a page all ask it, so no command can refuse a cached string that another opens
//! without a word.
//!
//! # The page itself is bundled by the half that serves it
//!
//! This crate ships no `assets/`. `goalcup-daemon` holds `assets/status.html` and serves it
//! at `/v0/ui` (D25), so there is one copy of the page and one thing that can be out of
//! date with the daemon answering for it.

use crate::CliError;
use crate::commands::status::put;
use crate::entries::read_entry;
use crate::retry::retry_busy;
use goalcup_protocol::control::DaemonClient;
use std::io::Write;
use std::path::Path;

/// `goalcup ui [<entry>] [--print]`.
///
/// `opener` is a parameter so the shipped binary hands over `open::that` and a test hands
/// over something that records what it was asked to open — the assertion this command is
/// mostly about is which string reached the browser.
///
/// A browser that could not be opened is not a failure. The URL is printed either way, so a
/// headless machine and a machine with no default browser both end up with the one thing
/// the participant came for.
pub fn ui_with(
    client: &dyn DaemonClient,
    cfg: &Path,
    entry: Option<&str>,
    print_only: bool,
    opener: &dyn Fn(&str) -> std::io::Result<()>,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let entry_id = match entry {
        Some(entry_id) => entry_id.to_string(),
        // The authenticated route, which is the daemon's own answer to "which run is
        // open" — and not this machine's newest record, because `ui` is about what is
        // being served right now.
        None => match retry_busy(|| client.current_entry())? {
            Some(entry_id) => entry_id,
            None => {
                return Err(CliError::Refused(
                    "this daemon is carrying no run right now, so there is no page to open. \
                     `goalcup entries` lists what it has, and `goalcup ui <entry>` opens \
                     the page for a run this machine started."
                        .to_string(),
                ));
            }
        },
    };
    let url = read_entry(cfg, &entry_id)?.status_url;
    crate::browser::check_page_url(&url, crate::browser::PageKind::Daemon).map_err(|e| {
        CliError::Refused(format!(
            "the page this machine has on file for entry {entry_id} is not one `goalcup ui` \
             will open. {e}"
        ))
    })?;
    if print_only {
        return put(out, &format!("{url}\n"));
    }
    match crate::browser::open_page(&url, crate::browser::PageKind::Daemon, opener)? {
        None => put(out, &format!("opening {url}\n")),
        Some(note) => put(out, &format!("{url}\n{note}\n")),
    }
}
