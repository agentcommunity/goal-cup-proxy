//! Whitelisted, sanitized capture metadata for HTTP exchanges, and the capture clock.
//!
//! This is the allowlist. Every [`CaptureMeta`] the HTTP relay submits is built
//! here, only from values that have already passed it:
//!
//! - `method`: an HTTP token of bounded length (the request line's own grammar);
//! - `host`: the relay context's host, the name the connection was admitted for, never a
//!   request header's spelling;
//! - `path`: the request target's path with the **entire** query and fragment removed, and
//!   only while it fits one metadata string; a longer path is matched in full but retained as
//!   `None`, never cut;
//! - `status`: the final response's three-digit code;
//! - `content_type`: the media type's `type/subtype` essence, lowercased, with every parameter
//!   (charset, boundary, anything else) dropped;
//! - `protocol_hint`: the matched [`CapturePath`]'s declared hint, copied, never inferred;
//! - `label`: display text composed from the values above and nothing else: `"{method} {path}"`
//!   or `"{status} {path}"`, or, when that exceeds one metadata string, `<long path>` in place
//!   of the path. A path is never cut, so no secret in it is split into a part a scrub rule no
//!   longer recognizes. A label is never a join key, and a selected exchange is never
//!   refused for the length of its own path.
//!
//! No header value, cookie, credential, query parameter or hash of any of them is an input to
//! a builder here, so none can reach a frame through this module.

use crate::proxy::api::{CapturePath, CapturePolicy};
use goalcup_protocol::capture::{
    CaptureMeta, CaptureTime, MAX_METADATA_STRING_BYTES, WireIdentity,
};
use goalcup_protocol::clock::ContinuousOrigin;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// The longest content-type essence retained. A real media type is a few dozen bytes.
const MAX_CONTENT_TYPE_BYTES: usize = 127;

/// The longest method retained. Real methods are a handful of bytes.
const MAX_METHOD_BYTES: usize = 64;

/// A placeholder clock name. [`crate::proxy::api::CaptureEmitter`] stamps every frame it
/// builds with its current producer ID as the clock, overwriting this value.
const PROCESS_CLOCK: &str = "goalcup-proxy-process";

/// Now, as a capture observation: RFC 3339 UTC wall time with nanoseconds, and nanoseconds on
/// one process-wide monotonic clock whose zero is the first observation this process made. The
/// monotonic clock counts sleep ([`goalcup_protocol::clock`]).
///
/// Every relay in one proxy process that reads this function shares one monotonic base, so
/// two observations of one producer compare meaningfully whichever relay made them.
/// `clock_id` is a placeholder the emitter replaces with the producer ID.
pub fn capture_time() -> CaptureTime {
    static BASE: OnceLock<ContinuousOrigin> = OnceLock::new();
    let base = *BASE.get_or_init(ContinuousOrigin::now);
    let mono_ns = base.elapsed_ns();
    // A clock before the epoch reads as the epoch: never a panic on the relay path.
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let whole = goalcup_protocol::control::format_z(since.as_secs());
    let wall = format!("{}.{:09}Z", &whole[..19], since.subsec_nanos());
    CaptureTime {
        wall,
        mono_ns,
        clock_id: PROCESS_CLOCK.to_string(),
    }
}

/// A request target reduced to what may be retained: the path without query or fragment,
/// and, for an absolute-form target, the authority it named (compared, never retained).
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SafeTarget {
    pub(crate) path: String,
    pub(crate) authority: Option<String>,
}

/// Reduce a request target (RFC 9112 §3.2) to its query- and fragment-free path.
///
/// Origin-form (`/p?q`) keeps `/p`; absolute-form (`https://h/p?q`) keeps `/p` and reports
/// `h` for the Host comparison. Asterisk- and authority-form targets have no capturable
/// path (`None`). The path's length is bounded only by the head bound; what may be retained
/// of it is decided by [`ExchangeMeta`].
pub(crate) fn safe_target(target: &str) -> Option<SafeTarget> {
    let (authority, rest) = if target.starts_with('/') {
        (None, target)
    } else {
        let scheme = target.get(..8).map(str::to_ascii_lowercase)?;
        let skip = if scheme.starts_with("http://") {
            7
        } else if scheme == "https://" {
            8
        } else {
            return None;
        };
        let after = &target[skip..];
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        (Some(after[..end].to_string()), &after[end..])
    };
    let path = &rest[..rest.find(['?', '#']).unwrap_or(rest.len())];
    let path = if path.is_empty() { "/" } else { path };
    if !path.bytes().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    Some(SafeTarget {
        path: path.to_string(),
        authority,
    })
}

/// The host part of an authority (`host`, `host:port`, `[v6]` or `[v6]:port`), lowercased.
fn host_part(authority: &str) -> String {
    let authority = authority.trim();
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// Does a request's Host (and absolute-form authority, if any) name the host this connection
/// was admitted for? Ports are ignored and names compare case-insensitively. A missing or
/// repeated Host header is a mismatch.
pub(crate) fn host_matches(expected: &str, host_headers: &[String], target: &SafeTarget) -> bool {
    let expected = host_part(expected);
    let [host] = host_headers else {
        return false;
    };
    !expected.is_empty()
        && host_part(host) == expected
        && target
            .authority
            .as_deref()
            .is_none_or(|authority| host_part(authority) == expected)
}

/// The first capture path of the matched host profile that selects `path`, exactly or as an
/// explicit prefix, against the path without its query or fragment. An empty policy selects
/// nothing.
pub(crate) fn select<'a>(policy: &'a CapturePolicy, path: &str) -> Option<&'a CapturePath> {
    policy.paths.iter().find(|candidate| {
        if candidate.prefix {
            path.starts_with(candidate.path.as_str())
        } else {
            path == candidate.path
        }
    })
}

/// An HTTP token (RFC 9110 §5.6.2).
pub(crate) fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// An HTTP method as it may be retained: a token of bounded length.
pub(crate) fn is_method(value: &str) -> bool {
    value.len() <= MAX_METHOD_BYTES && is_token(value)
}

/// The `type/subtype` essence of a Content-Type value, lowercased, with parameters dropped.
/// Anything that is not a well-formed media type yields `None`.
pub(crate) fn content_type_essence(value: &str) -> Option<String> {
    let essence = value.split(';').next()?.trim();
    let (kind, subtype) = essence.split_once('/')?;
    (essence.len() <= MAX_CONTENT_TYPE_BYTES && is_token(kind) && is_token(subtype))
        .then(|| essence.to_ascii_lowercase())
}

/// What one selected exchange may retain about its request, fixed when the request line is
/// read. Response metadata and the WebSocket hand-off both derive from it, so neither selects
/// again from the host policy after the request target has been consumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExchangeMeta {
    pub(crate) connection: String,
    pub(crate) exchange: u64,
    pub(crate) host: String,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) protocol_hint: Option<String>,
}

/// `"{prefix} {path}"`, or `"{prefix} <long path>"` when that exceeds one metadata string.
fn label(prefix: &str, path: &str) -> String {
    if prefix.len() + 1 + path.len() > MAX_METADATA_STRING_BYTES {
        format!("{prefix} <long path>")
    } else {
        format!("{prefix} {path}")
    }
}

impl ExchangeMeta {
    /// The path as it may be retained: whole, or not at all.
    fn retained_path(&self) -> Option<String> {
        (self.path.len() <= MAX_METADATA_STRING_BYTES).then(|| self.path.clone())
    }

    fn wire(&self) -> Option<WireIdentity> {
        Some(WireIdentity {
            connection: self.connection.clone(),
            exchange: self.exchange,
            message_index: None,
        })
    }

    /// The request stream's metadata; also `Upgrade.capture` for an accepted upgrade.
    pub(crate) fn request(&self, content_type: Option<String>) -> CaptureMeta {
        CaptureMeta {
            wire: self.wire(),
            label: label(&self.method, &self.path),
            protocol_hint: self.protocol_hint.clone(),
            method: Some(self.method.clone()),
            host: Some(self.host.clone()),
            path: self.retained_path(),
            status: None,
            content_type,
        }
    }

    /// The final response's metadata: the same typed exchange, labelled with its own
    /// request's safe path, so no response is associated by label with another request.
    pub(crate) fn response(&self, status: u16, content_type: Option<String>) -> CaptureMeta {
        CaptureMeta {
            wire: self.wire(),
            label: label(&status.to_string(), &self.path),
            protocol_hint: self.protocol_hint.clone(),
            method: None,
            host: Some(self.host.clone()),
            path: self.retained_path(),
            status: Some(status),
            content_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_and_fragment_are_removed_entirely() {
        let t = safe_target("/v1/responses?sig=SECRET&x=1#frag").unwrap();
        assert_eq!(t.path, "/v1/responses");
        assert_eq!(t.authority, None);
        assert_eq!(safe_target("/v1/a#only-fragment").unwrap().path, "/v1/a");
        assert_eq!(safe_target("/?q").unwrap().path, "/");
        let abs = safe_target("HTTPS://Api.Example.Test:443?token=SECRET").unwrap();
        assert_eq!(abs.path, "/");
        assert_eq!(abs.authority.as_deref(), Some("Api.Example.Test:443"));
        assert_eq!(safe_target("http://h/v1/x?y").unwrap().path, "/v1/x");
        assert!(safe_target("*").is_none());
        assert!(safe_target("api.example.test:443").is_none());
    }

    #[test]
    fn long_paths_bound_the_metadata_without_refusing_the_exchange() {
        let meta = |path: &str| ExchangeMeta {
            connection: "c0ffee00c0ffee00c0ffee00c0ffee00".into(),
            exchange: 1,
            host: "api.example.test".into(),
            method: "POST".into(),
            path: safe_target(path).unwrap().path,
            protocol_hint: None,
        };
        let fits = format!("/{}", "a".repeat(MAX_METADATA_STRING_BYTES - 1));
        let over = format!("/{}", "a".repeat(MAX_METADATA_STRING_BYTES));
        for (path, retained) in [(&fits, true), (&over, false)] {
            let m = meta(&format!("{path}?q=SECRET"));
            let (request, response) = (m.request(None), m.response(200, None));
            for built in [&request, &response] {
                built.validate().unwrap();
                assert_eq!(built.path.is_some(), retained);
            }
            assert_eq!(request.label, "POST <long path>");
            assert_eq!(response.label, "200 <long path>");
        }
        // A label that fits keeps the whole path; one byte more names none of it.
        let whole = format!(
            "/{}",
            "a".repeat(MAX_METADATA_STRING_BYTES - "POST /".len())
        );
        assert_eq!(meta(&whole).request(None).label, format!("POST {whole}"));
        assert_eq!(
            meta(&format!("{whole}a")).request(None).label,
            "POST <long path>"
        );
    }

    #[test]
    fn host_comparison_ignores_port_and_case_but_not_the_name() {
        let origin = safe_target("/").unwrap();
        let one = |h: &str| vec![h.to_string()];
        assert!(host_matches(
            "api.example.test",
            &one("API.example.test:443"),
            &origin
        ));
        assert!(host_matches(
            "api.example.test",
            &one("api.example.test"),
            &origin
        ));
        assert!(!host_matches(
            "api.example.test",
            &one("evil.test"),
            &origin
        ));
        assert!(!host_matches("api.example.test", &[], &origin));
        let twice = vec!["api.example.test".to_string(); 2];
        assert!(!host_matches("api.example.test", &twice, &origin));
        let abs = safe_target("https://evil.test/v1").unwrap();
        assert!(!host_matches(
            "api.example.test",
            &one("api.example.test"),
            &abs
        ));
        assert!(host_matches("[::1]", &one("[::1]:8443"), &origin));
    }

    #[test]
    fn content_type_keeps_the_essence_only() {
        assert_eq!(
            content_type_essence("Text/Event-Stream; charset=utf-8").as_deref(),
            Some("text/event-stream")
        );
        assert_eq!(
            content_type_essence("multipart/form-data; boundary=SECRET").as_deref(),
            Some("multipart/form-data")
        );
        assert_eq!(content_type_essence("garbage"), None);
        assert_eq!(content_type_essence("a b/c"), None);
    }

    #[test]
    fn selection_is_exact_or_explicit_prefix_in_policy_order() {
        let policy = CapturePolicy {
            paths: vec![
                CapturePath {
                    path: "/v1/responses".into(),
                    prefix: false,
                    protocol_hint: Some("openai-responses".into()),
                },
                CapturePath {
                    path: "/v1/".into(),
                    prefix: true,
                    protocol_hint: None,
                },
            ],
        };
        let hint = select(&policy, "/v1/responses").and_then(|p| p.protocol_hint.as_deref());
        assert_eq!(hint, Some("openai-responses"));
        assert!(select(&policy, "/v1/responses/x").is_some_and(|p| p.prefix));
        assert!(select(&policy, "/v2/responses").is_none());
        assert!(select(&CapturePolicy::default(), "/v1/responses").is_none());
    }

    #[test]
    fn capture_time_is_valid_and_monotonic() {
        let a = capture_time();
        let b = capture_time();
        a.validate().unwrap();
        b.validate().unwrap();
        assert!(b.mono_ns >= a.mono_ns);
        assert_eq!(a.wall.len(), "2026-09-27T10:00:00.000000000Z".len());
    }
}
