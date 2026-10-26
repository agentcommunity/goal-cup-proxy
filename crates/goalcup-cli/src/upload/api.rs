//! The upload API's documents, field for field as docs/UPLOAD-API.md spells them.

use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::receipt::StartReceipt;
use serde::{Deserialize, Serialize};

/// An error answer's body. Missing fields read as empty: a proxy's HTML error page is still an
/// error, with the status code alone.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

/// This program's version, as `POST /v1/device`, an open and a continue send it: under
/// `proxy_version`, and under its name from before the rename to Goal Cup Proxy,
/// `tracer_version`, with the same value, so a server of either age reads it. Read back, either
/// name serves; when both are present they must agree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "VersionFields", try_from = "VersionFields")]
pub struct ProxyVersion(pub String);

impl ProxyVersion {
    /// The version of this build.
    pub fn current() -> ProxyVersion {
        ProxyVersion(crate::CLI_VERSION.to_string())
    }
}

impl From<&str> for ProxyVersion {
    fn from(version: &str) -> ProxyVersion {
        ProxyVersion(version.to_string())
    }
}

/// [`ProxyVersion`] on the wire: both names.
#[derive(Serialize, Deserialize)]
struct VersionFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proxy_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tracer_version: Option<String>,
}

impl From<ProxyVersion> for VersionFields {
    fn from(version: ProxyVersion) -> VersionFields {
        VersionFields {
            tracer_version: Some(version.0.clone()),
            proxy_version: Some(version.0),
        }
    }
}

impl TryFrom<VersionFields> for ProxyVersion {
    type Error = String;

    fn try_from(fields: VersionFields) -> Result<ProxyVersion, String> {
        match (fields.proxy_version, fields.tracer_version) {
            (Some(new), Some(old)) if new != old => {
                Err("proxy_version and tracer_version differ".to_string())
            }
            (Some(version), _) | (None, Some(version)) => Ok(ProxyVersion(version)),
            (None, None) => Err("missing field `proxy_version`".to_string()),
        }
    }
}

/// `POST /v1/runs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRunRequest {
    pub connector_id: String,
    pub manifest_sha256: String,
    pub connector_dir_sha256: String,
    pub workspace: String,
    pub session_id: String,
    pub harness_version: String,
    #[serde(flatten)]
    pub proxy_version: ProxyVersion,
    pub repo_url: String,
    /// What the machine is, without who it is ([`goalcup_platform::machine::facts`]). Left out
    /// when a caller has none to send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<goalcup_platform::machine::MachineFacts>,
    /// The run came through `--dev` (a connector this build does not pin). Sent only when true;
    /// the server refuses it for a competition run.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dev: bool,
}

/// `POST /v1/runs/{run_id}/continue`: a new segment of a run that is still open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinueRequest {
    /// The run's latest entry as this machine knows it; the server answers `409` to any other.
    pub previous_entry_id: String,
    pub connector_id: String,
    pub manifest_sha256: String,
    pub connector_dir_sha256: String,
    pub workspace: String,
    pub session_id: String,
    pub harness_version: String,
    #[serde(flatten)]
    pub proxy_version: ProxyVersion,
    /// As [`OpenRunRequest::dev`]: the segment came through `--dev`. Sent only when true.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dev: bool,
}

/// The answer to [`ContinueRequest`]: the new entry, and the run's own start and deadline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinueRun {
    pub entry_id: String,
    /// The segment's number: 1 for the first continue.
    pub segment: u64,
    pub official_start: String,
    pub deadline_at: String,
    /// Whole seconds from the server's clock to `deadline_at` when it answered (0 once it has
    /// come). Goal Cup Proxy counts the run down from this, never from this machine's date. An
    /// older server leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_left: Option<u64>,
}

/// The answer to [`OpenRunRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRun {
    pub run_id: String,
    pub entry_id: String,
    pub official_start: String,
    pub deadline_at: String,
    /// Whole seconds from the server's clock to `deadline_at` when it answered (0 once it has
    /// come). Goal Cup Proxy counts the run down from this, never from this machine's date. An
    /// older server leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_left: Option<u64>,
}

/// `POST /v1/runs/{run_id}/receipt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptRequest {
    pub entry_id: String,
    pub chain_head: String,
}

/// The answer to [`ReceiptRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptAnswer {
    pub start_receipt: StartReceipt,
}

/// `POST /v1/runs/{run_id}/abandon`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbandonRequest {
    pub reason: String,
}

/// `GET /v1/runs/{run_id}`: where the server is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunPosition {
    pub run_id: String,
    pub entry_id: String,
    pub status: String,
    pub official_start: String,
    pub deadline_at: String,
    /// Bytes of the entry's `chain.jsonl` the server holds.
    pub offset: u64,
    /// The `seq` the next chunk starts at: 0 before the first chunk.
    pub next_seq: u64,
    /// The chain hash of the last stored record; the genesis hash before any.
    pub head: String,
    pub last_ack: Option<AckCheckpoint>,
    /// The server's clock is at or past `deadline_at`: the run is over.
    #[serde(default)]
    pub stop: bool,
    /// The end a final heartbeat committed, if one has.
    #[serde(default, rename = "final")]
    pub committed: Option<ChainEnd>,
}

/// A chain position: the `seq` and `hash` of one record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainEnd {
    pub seq: u64,
    pub head: String,
}

/// `POST /v1/runs/{run_id}/heartbeat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    pub entry_id: String,
    /// The last whole record of the local chain.
    pub seq: u64,
    pub head: String,
    /// The bytes of the local chain up to the end of that record.
    pub offset_local: u64,
    /// The run has ended: this commits `(seq, head)` as the last record the server accepts.
    #[serde(rename = "final")]
    pub is_final: bool,
    /// The commit the workspace repository had checked out when the run ended.
    pub repo_head: Option<String>,
    /// How the tracker sees its own recording: `ok`, `gaps`, `failed` or `unknown`. Left out
    /// by a heartbeat that has nothing to say (`goalcup upload`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording: Option<String>,
    /// Model requests seen so far, cumulative. Left out with `recording`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_requests: Option<u64>,
    /// The disk and goalcup's own processes. Left out with `recording`, and by a run that
    /// has nothing to read them from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vitals: Option<HeartbeatVitals>,
}

/// The `vitals` of a heartbeat (`docs/UPLOAD-API.md`). A number that could not be read is left
/// out, never guessed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatVitals {
    /// Free bytes on the volume of the run's files (beyond the run's reservation, which is a
    /// file there).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_free_bytes: Option<u64>,
    /// [`crate::run::reserve::short_of_disk`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_short: Option<bool>,
    /// `"disk"` while goalcup has paused the agent for a nearly full disk, else `null`.
    #[serde(default)]
    pub agent_paused: Option<String>,
    /// Summed over this `goalcup` process (with its recording proxy) and the daemon it started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rss_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_files: Option<u64>,
}

/// The answer to a heartbeat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatAnswer {
    /// The run is over: stop now.
    pub stop: bool,
    pub deadline_at: String,
    /// Whole seconds from the server's clock to `deadline_at` when it answered (0 once it has
    /// come). Goal Cup Proxy counts the run down from this, never from this machine's date. An
    /// older server leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds_left: Option<u64>,
    /// Why `stop` is true when the deadline is not all of it: `deadline` or `replaced`. An
    /// older server leaves it out.
    #[serde(default)]
    pub reason: Option<String>,
}

/// The answer to a stored chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkAnswer {
    pub ack: AckCheckpoint,
    pub offset: u64,
    pub next_seq: u64,
    pub head: String,
}

/// `POST /v1/runs/{run_id}/seal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealRequest {
    pub entry_id: String,
    pub seal_json: String,
    pub baseline_json: String,
}

/// A bare `{"status": …}` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusAnswer {
    pub status: String,
    /// The run's own status, when the answer names it (a seal's does): `open`, `sealed`,
    /// `incomplete`.
    #[serde(default)]
    pub run_status: Option<String>,
}

impl StatusAnswer {
    /// The server says the run itself is over: sealed or incomplete.
    pub fn run_ended(&self) -> bool {
        matches!(self.run_status.as_deref(), Some("sealed" | "incomplete"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn continue_request(version: &str) -> ContinueRequest {
        ContinueRequest {
            previous_entry_id: "e0000000000000000".into(),
            connector_id: "c".into(),
            manifest_sha256: "0".repeat(64),
            connector_dir_sha256: "0".repeat(64),
            workspace: "/w".into(),
            session_id: "s".into(),
            harness_version: "h".into(),
            dev: false,
            proxy_version: version.into(),
        }
    }

    /// The version goes out under both names with the same value, so a server from before the
    /// rename and one after it both read it.
    #[test]
    fn the_version_is_sent_under_both_names() {
        let sent = serde_json::to_value(continue_request("1.2.3-test")).unwrap();
        assert_eq!(sent["proxy_version"], "1.2.3-test");
        assert_eq!(sent["tracer_version"], "1.2.3-test");
    }

    /// Either name reads back; both must agree; one of them must be there.
    #[test]
    fn the_version_reads_from_either_name() {
        let mut body = serde_json::to_value(continue_request("1.2.3-test")).unwrap();
        let back: ContinueRequest = serde_json::from_value(body.clone()).unwrap();
        assert_eq!(back.proxy_version, ProxyVersion::from("1.2.3-test"));
        for (keep, drop) in [
            ("proxy_version", "tracer_version"),
            ("tracer_version", "proxy_version"),
        ] {
            let mut one = body.clone();
            one.as_object_mut().unwrap().remove(drop);
            assert!(one.get(keep).is_some());
            let back: ContinueRequest = serde_json::from_value(one).unwrap();
            assert_eq!(back.proxy_version.0, "1.2.3-test");
        }
        body["tracer_version"] = "9.9.9-test".into();
        assert!(serde_json::from_value::<ContinueRequest>(body.clone()).is_err());
        let fields = body.as_object_mut().unwrap();
        fields.remove("tracer_version");
        fields.remove("proxy_version");
        assert!(serde_json::from_value::<ContinueRequest>(body).is_err());
    }
}
