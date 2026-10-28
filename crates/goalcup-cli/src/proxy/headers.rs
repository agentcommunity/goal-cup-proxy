//! Header streams: the request and response heads of every recorded exchange, with credential
//! values masked before they reach the chain.
//!
//! Each recorded exchange on an opened host gets one `WireRequest` and one `WireResponse` stream
//! whose protocol hint is [`HEADERS_HINT`]. Its bytes are [`masked_head`]: the start line as
//! received (credential-named query values masked) and every header line as received, with the
//! value of a credential-named header replaced by [`MASK`]. The daemon's redactor then runs on the
//! stream like any other.
//!
//! A header stream has no method, and the hint is an artifact hint, so it is never a model call
//! ([`crate::proxy::metadata::ExchangeMeta::counts_as_model_call`]).

use goalcup_protocol::capture::HTTP_HEAD_HINT;

/// The protocol hint of a header stream. The name is the protocol's own artifact hint.
pub const HEADERS_HINT: &str = HTTP_HEAD_HINT;

/// What a masked value is replaced with.
pub const MASK: &[u8] = b"[masked]";

/// Header names whose values are always masked, compared case-insensitively.
const CREDENTIAL_NAMES: [&str; 7] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
    "x-goog-api-key",
];

/// Any header name containing one of these is masked, compared case-insensitively.
const CREDENTIAL_PARTS: [&str; 5] = ["key", "token", "secret", "password", "auth"];

/// Is this header name one whose value is a credential?
fn is_credential_header(name: &[u8]) -> bool {
    let name = String::from_utf8_lossy(name.trim_ascii()).to_ascii_lowercase();
    CREDENTIAL_NAMES.contains(&name.as_str()) || CREDENTIAL_PARTS.iter().any(|p| name.contains(p))
}

/// Is this query parameter name one whose value is a credential? The header rule, plus the
/// signature parameters of signed URLs.
fn is_credential_query(name: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(name).to_ascii_lowercase();
    is_credential_header(name) || lower == "sig" || lower.contains("signature")
}

/// The head with every credential value masked. Names, start line and every other line are kept
/// as received, including their line endings. Input is one head, through its blank line.
pub fn masked_head(head: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(head.len());
    let mut started = false;
    // The previous header was masked, so a folded continuation line belongs to its value.
    let mut in_masked_value = false;
    for line in head.split_inclusive(|&b| b == b'\n') {
        let end = line
            .iter()
            .rposition(|&b| b != b'\r' && b != b'\n')
            .map_or(0, |i| i + 1);
        let (content, eol) = line.split_at(end);
        if content.is_empty() {
            // Blank lines before the start line are tolerated; the blank line ends the head.
            out.extend_from_slice(line);
            if started {
                break;
            }
            continue;
        }
        if !started {
            started = true;
            out.extend_from_slice(&mask_start_line(content));
            out.extend_from_slice(eol);
            continue;
        }
        if matches!(content[0], b' ' | b'\t') {
            if in_masked_value {
                out.extend_from_slice(MASK);
                out.extend_from_slice(eol);
            } else {
                out.extend_from_slice(line);
            }
            continue;
        }
        match content.iter().position(|&b| b == b':') {
            Some(colon) if is_credential_header(&content[..colon]) => {
                out.extend_from_slice(&content[..colon]);
                out.extend_from_slice(b": ");
                out.extend_from_slice(MASK);
                out.extend_from_slice(eol);
                in_masked_value = true;
            }
            _ => {
                out.extend_from_slice(line);
                in_masked_value = false;
            }
        }
    }
    out
}

/// The request or status line with credential-named query values masked. Only the target (the
/// second space-separated token) can carry a query.
fn mask_start_line(line: &[u8]) -> Vec<u8> {
    let Some(start) = line.iter().position(|&b| b == b' ').map(|i| i + 1) else {
        return line.to_vec();
    };
    let end = line[start..]
        .iter()
        .position(|&b| b == b' ')
        .map_or(line.len(), |i| start + i);
    [&line[..start], &mask_query(&line[start..end]), &line[end..]].concat()
}

/// A target with each credential-named query parameter's value replaced. Parameters are split on
/// `&`; a parameter without `=` has no value to mask.
fn mask_query(target: &[u8]) -> Vec<u8> {
    let Some(question) = target.iter().position(|&b| b == b'?') else {
        return target.to_vec();
    };
    let (path, query) = target.split_at(question + 1);
    let mut out = path.to_vec();
    for (index, pair) in query.split(|&b| b == b'&').enumerate() {
        if index > 0 {
            out.push(b'&');
        }
        match pair.iter().position(|&b| b == b'=') {
            Some(equals) if is_credential_query(&pair[..equals]) => {
                out.extend_from_slice(&pair[..=equals]);
                out.extend_from_slice(MASK);
            }
            _ => out.extend_from_slice(pair),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(head: &[u8]) -> String {
        String::from_utf8(masked_head(head)).expect("a masked head stays text")
    }

    #[test]
    fn a_credential_header_value_is_masked() {
        let head = b"POST /v1/messages HTTP/1.1\r\n\
            Host: api.example.test\r\n\
            Authorization: Bearer tok-SECRET-1\r\n\
            x-api-key: sk-SECRET-2\r\n\
            x-goog-api-key: gk-SECRET-3\r\n\
            Cookie: sid=SECRET-4\r\n\
            x-my-auth-token: at-SECRET-5\r\n\
            PROXY-AUTHORIZATION: Basic SECRET-6\r\n\
            User-Agent: example-agent/2.1.312\r\n\
            anthropic-version: 2023-06-01\r\n\
            x-request-id: req-1\r\n\
            \r\n";
        let out = text(head);
        assert!(!out.contains("SECRET"), "{out}");
        for masked in [
            "Authorization: [masked]\r\n",
            "x-api-key: [masked]\r\n",
            "x-goog-api-key: [masked]\r\n",
            "Cookie: [masked]\r\n",
            "x-my-auth-token: [masked]\r\n",
            "PROXY-AUTHORIZATION: [masked]\r\n",
        ] {
            assert!(out.contains(masked), "{masked:?} not in {out}");
        }
        for kept in [
            "POST /v1/messages HTTP/1.1\r\n",
            "Host: api.example.test\r\n",
            "User-Agent: example-agent/2.1.312\r\n",
            "anthropic-version: 2023-06-01\r\n",
            "x-request-id: req-1\r\n",
            "\r\n",
        ] {
            assert!(out.contains(kept), "{kept:?} not in {out}");
        }
    }

    #[test]
    fn a_credential_query_value_in_the_start_line_is_masked() {
        let head = b"GET /v1/models?key=SECRET-1&alt=json&api_key=SECRET-2&sig=SECRET-3&page=2 HTTP/1.1\r\nHost: x\r\n\r\n";
        let out = text(head);
        assert!(
            out.starts_with(
                "GET /v1/models?key=[masked]&alt=json&api_key=[masked]&sig=[masked]&page=2 HTTP/1.1\r\n"
            ),
            "{out}"
        );
        assert!(!out.contains("SECRET"), "{out}");
    }

    #[test]
    fn line_endings_and_a_bare_lf_are_kept_as_received() {
        let out = text(b"GET / HTTP/1.1\nAuthorization: x\nAccept: */*\n\n");
        assert_eq!(
            out,
            "GET / HTTP/1.1\nAuthorization: [masked]\nAccept: */*\n\n"
        );
    }
}
