//! The sealed-connection list (D74): every tunnel the recording proxy relayed unread, noted as
//! host, port, reason, times and bytes each way, and written once at finish to
//! `tunnels-<entry_id>.jsonl` in the run's directory. It is a side artifact with its own format
//! tag, outside the v5 chain: nothing hashes it and the v5 verifier never reads it.
//!
//! The list is bounded: past [`MAX_TUNNEL_NOTES`] rows a tunnel is only counted, so an agent that
//! opens thousands of connections (a package install loop) costs a fixed amount of memory and a
//! finish that writes at most that many lines.

use super::connect::{TunnelEnd, TunnelReason, Tunneled};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The first line's `format`.
pub const TUNNELS_FORMAT: &str = "goalcup-tunnels-v1";

/// The most tunnels one proxy run lists; the rest are counted in the last line's `dropped`.
pub const MAX_TUNNEL_NOTES: usize = 10_000;

/// How many hosts the finish line names.
const SUMMARY_HOSTS: usize = 5;

/// One sealed connection. The bytes are `None` when the tunnel ended in an I/O error, or was cut
/// open at the stop: then how much passed is not known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelNote {
    pub host: String,
    pub port: u16,
    pub reason: &'static str,
    pub opened: SystemTime,
    pub closed: SystemTime,
    pub up_bytes: Option<u64>,
    pub down_bytes: Option<u64>,
}

impl TunnelNote {
    /// The note of a tunnel that ran to its end.
    pub(crate) fn of(tunnel: &Tunneled) -> Self {
        let (up_bytes, down_bytes) = match tunnel.end {
            TunnelEnd::Closed {
                client_to_upstream,
                upstream_to_client,
            } => (Some(client_to_upstream), Some(upstream_to_client)),
            TunnelEnd::Error(_) => (None, None),
        };
        Self {
            host: tunnel.host.clone(),
            port: tunnel.port,
            reason: reason_word(tunnel.reason),
            opened: tunnel.opened,
            closed: tunnel.closed,
            up_bytes,
            down_bytes,
        }
    }
}

/// The file's word for why a connection was tunnelled.
pub(crate) fn reason_word(reason: TunnelReason) -> &'static str {
    match reason {
        TunnelReason::AuthHost => "sign_in",
        TunnelReason::NotApproved => "not_model_host",
        TunnelReason::SniAbsent => "fail_open_no_sni",
        TunnelReason::SniMismatch => "fail_open_sni_mismatch",
        TunnelReason::NotTls => "fail_open_not_tls",
        TunnelReason::ClientHelloUnparsed => "fail_open_unparsed",
    }
}

/// The notes of one proxy run, shared by its connection tasks.
#[derive(Debug, Default)]
pub struct TunnelNotes {
    notes: Mutex<Vec<TunnelNote>>,
    dropped: AtomicU64,
}

impl TunnelNotes {
    /// Keeps `note`, or only counts it once [`MAX_TUNNEL_NOTES`] are kept.
    pub fn push(&self, note: TunnelNote) {
        let mut notes = self.notes.lock().unwrap_or_else(|e| e.into_inner());
        if notes.len() < MAX_TUNNEL_NOTES {
            notes.push(note);
        } else {
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// The kept notes in the order they ended, and how many were only counted.
    pub fn snapshot(&self) -> (Vec<TunnelNote>, u64) {
        let notes = self.notes.lock().unwrap_or_else(|e| e.into_inner());
        (notes.clone(), self.dropped.load(Ordering::SeqCst))
    }
}

/// `tunnels-<entry_id>.jsonl`: one file per entry, so a continued run's segments each keep their
/// own.
pub fn file_name(entry_id: &str) -> String {
    format!("tunnels-{entry_id}.jsonl")
}

/// Writes the list to `path`, once, mode 0600: a first line with the format and the entry, one
/// line per note, and a last line with the dropped count. A partial file is never left: the
/// bytes go to a temporary file that is renamed into place.
pub fn write(
    path: &Path,
    entry_id: &str,
    notes: &[TunnelNote],
    dropped: u64,
) -> Result<(), crate::CliError> {
    let mut text = String::new();
    let mut line = |value: serde_json::Value| {
        text.push_str(&value.to_string());
        text.push('\n');
    };
    line(serde_json::json!({ "format": TUNNELS_FORMAT, "entry_id": entry_id }));
    for note in notes {
        line(serde_json::json!({
            "host": note.host,
            "port": note.port,
            "reason": note.reason,
            "opened": utc(note.opened),
            "closed": utc(note.closed),
            "up_bytes": note.up_bytes,
            "down_bytes": note.down_bytes,
        }));
    }
    line(serde_json::json!({ "dropped": dropped }));
    crate::config::write_secret_file(path, text.as_bytes())
}

fn utc(at: SystemTime) -> String {
    // A clock before the epoch reads as the epoch, as the capture times do.
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    goalcup_protocol::control::format_z(since.as_secs())
}

/// The one finish line: `sealed hosts: <host>:<port> x<n> <KiB> KiB, …` for the
/// [`SUMMARY_HOSTS`] hosts with the most bytes both ways, and where the rest are listed. `None`
/// when nothing was tunnelled.
pub fn summary_line(entry_id: &str, notes: &[TunnelNote], dropped: u64) -> Option<String> {
    if notes.is_empty() && dropped == 0 {
        return None;
    }
    let mut hosts = std::collections::BTreeMap::<(&str, u16), (u64, u64)>::new();
    for note in notes {
        let (count, bytes) = hosts.entry((&note.host, note.port)).or_default();
        *count += 1;
        *bytes += note.up_bytes.unwrap_or(0) + note.down_bytes.unwrap_or(0);
    }
    let mut hosts: Vec<_> = hosts.into_iter().collect();
    // Most bytes first; the map's order breaks ties, so the line is the same for the same notes.
    hosts.sort_by_key(|(_, (_, bytes))| std::cmp::Reverse(*bytes));
    let named: Vec<String> = hosts
        .iter()
        .take(SUMMARY_HOSTS)
        .map(|((host, port), (count, bytes))| {
            format!("{host}:{port} x{count} {} KiB", bytes / 1024)
        })
        .collect();
    let mut line = format!("sealed hosts: {}", named.join(", "));
    let more = hosts.len().saturating_sub(SUMMARY_HOSTS);
    if more > 0 || dropped > 0 {
        let mut rest = Vec::new();
        if more > 0 {
            rest.push(format!("+{more} more"));
        }
        if dropped > 0 {
            rest.push(format!("{dropped} past the list's {MAX_TUNNEL_NOTES} rows"));
        }
        line.push_str(&format!(
            " ({}, list in {})",
            rest.join(", "),
            file_name(entry_id)
        ));
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(host: &str, bytes: u64) -> TunnelNote {
        TunnelNote {
            host: host.into(),
            port: 443,
            reason: "not_model_host",
            opened: UNIX_EPOCH,
            closed: UNIX_EPOCH,
            up_bytes: Some(bytes / 2),
            down_bytes: Some(bytes - bytes / 2),
        }
    }

    #[test]
    fn notes_stop_at_the_bound_and_count_the_rest() {
        let notes = TunnelNotes::default();
        for _ in 0..MAX_TUNNEL_NOTES + 5 {
            notes.push(note("registry.npmjs.org", 1));
        }
        let (kept, dropped) = notes.snapshot();
        assert_eq!(kept.len(), MAX_TUNNEL_NOTES);
        assert_eq!(dropped, 5);
    }

    #[test]
    fn the_file_is_tagged_one_line_per_tunnel_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(file_name("e0123456789abcdef"));
        let mut cut = note("intranet.corp", 0);
        (cut.up_bytes, cut.down_bytes) = (None, None);
        let notes = [note("github.com", 10), note("pypi.org", 20), cut];
        write(&path, "e0123456789abcdef", &notes, 0).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0]["format"], TUNNELS_FORMAT);
        assert_eq!(lines[0]["entry_id"], "e0123456789abcdef");
        assert_eq!(lines[1]["host"], "github.com");
        assert_eq!(lines[1]["opened"], "1970-01-01T00:00:00Z");
        assert_eq!(lines[3]["up_bytes"], serde_json::Value::Null);
        assert_eq!(lines[4], serde_json::json!({ "dropped": 0 }));
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn the_summary_names_the_biggest_hosts() {
        let notes = [
            note("registry.npmjs.org", 300 * 1024),
            note("github.com", 2 * 1024 * 1024),
        ];
        let line = summary_line("e0123456789abcdef", &notes, 0).unwrap();
        assert_eq!(
            line,
            "sealed hosts: github.com:443 x1 2048 KiB, registry.npmjs.org:443 x1 300 KiB"
        );
        assert_eq!(summary_line("e0123456789abcdef", &[], 0), None);
    }
}
