//! Header streams: the request and response heads of every recorded exchange, with credential
//! values masked before they reach the chain.
//!
//! Each recorded exchange on an opened host gets one `WireRequest` and one `WireResponse` stream
//! whose protocol hint is [`HEADERS_HINT`]. Its bytes are [`masked_head`]: the start line as
//! received, every header line as received, with the value of a credential-named header replaced
//! by [`MASK`]. Inside the other values, URLs have their userinfo and credential-named query
//! values masked, and a `Bearer` or `Basic` credential is masked. The daemon's redactor then runs
//! on the stream like any other.
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

/// Authorization schemes whose credential is the rest of the value. Masked in any header.
const CREDENTIAL_SCHEMES: [&str; 2] = ["bearer", "basic"];

/// Is this header name one whose value is a credential?
fn is_credential_header(name: &[u8]) -> bool {
    let name = String::from_utf8_lossy(name.trim_ascii()).to_ascii_lowercase();
    CREDENTIAL_NAMES.contains(&name.as_str()) || CREDENTIAL_PARTS.iter().any(|p| name.contains(p))
}

/// Is this query parameter name one whose value is a credential? The header rule, plus the
/// signature parameters of signed URLs. The name is percent-decoded first, so `%6Bey` is `key`.
fn is_credential_query(name: &[u8]) -> bool {
    let decoded = percent_decode(name);
    let lower = String::from_utf8_lossy(&decoded).to_ascii_lowercase();
    is_credential_header(&decoded) || lower == "sig" || lower.contains("signature")
}

/// The bytes with each `%XX` escape decoded once. A malformed escape is kept as received.
fn percent_decode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some(&byte) = rest.first() {
        match (
            rest.get(1).copied().and_then(hex_value),
            rest.get(2).copied().and_then(hex_value),
        ) {
            (Some(high), Some(low)) if byte == b'%' => {
                out.push(high * 16 + low);
                rest = &rest[3..];
            }
            _ => {
                out.push(byte);
                rest = &rest[1..];
            }
        }
    }
    out
}

/// The value of one hexadecimal digit.
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// The position of the first occurrence of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The head with every credential masked. Names, start line and every other line are kept as
/// received, including their line endings. Input is one head, through its blank line.
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
        if content.contains(&b'\r') {
            // A bare CR can start a new line for a parser that reads it as a line break, so a
            // credential could sit after it. The whole line is masked.
            out.extend_from_slice(MASK);
            out.extend_from_slice(eol);
            started = true;
            in_masked_value = true;
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
            } else {
                out.extend_from_slice(&mask_value(content));
            }
            out.extend_from_slice(eol);
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
            Some(colon) => {
                out.extend_from_slice(&content[..=colon]);
                out.extend_from_slice(&mask_value(&content[colon + 1..]));
                out.extend_from_slice(eol);
                in_masked_value = false;
            }
            None => {
                out.extend_from_slice(&mask_value(content));
                out.extend_from_slice(eol);
                in_masked_value = false;
            }
        }
    }
    out
}

/// The request or status line with credentials in its target masked. Only the target (the
/// second space-separated token) can carry a query or userinfo.
fn mask_start_line(line: &[u8]) -> Vec<u8> {
    let Some(start) = line.iter().position(|&b| b == b' ').map(|i| i + 1) else {
        return line.to_vec();
    };
    let end = line[start..]
        .iter()
        .position(|&b| b == b' ')
        .map_or(line.len(), |i| start + i);
    [&line[..start], &mask_url(&line[start..end]), &line[end..]].concat()
}

/// A header value with the credentials in it masked: each URL's userinfo and credential-named
/// query values, and the credential of a `Bearer` or `Basic` value. Everything else is kept.
fn mask_value(value: &[u8]) -> Vec<u8> {
    let start = value
        .iter()
        .position(|&b| b != b' ' && b != b'\t')
        .unwrap_or(value.len());
    if let Some(credential) = scheme_credential(&value[start..]) {
        // The scheme and the white space after it are kept, the credential is masked.
        let mut out = value[..value.len() - credential.len()].to_vec();
        out.extend_from_slice(MASK);
        return out;
    }
    let mut out = Vec::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        if is_url_delimiter(value[index]) {
            out.push(value[index]);
            index += 1;
            continue;
        }
        let run_end = value[index..]
            .iter()
            .position(|&b| is_url_delimiter(b))
            .map_or(value.len(), |i| index + i);
        out.extend_from_slice(&mask_url(&value[index..run_end]));
        index = run_end;
    }
    out
}

/// The bytes that end a URL inside a header value.
fn is_url_delimiter(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'"' | b'<' | b'>')
}

/// The credential of a `Bearer` or `Basic` value, when the body starts with one of those schemes
/// followed by white space and a non-empty credential. The scheme is compared case-insensitively.
fn scheme_credential(body: &[u8]) -> Option<&[u8]> {
    let scheme_end = body.iter().position(|&b| b == b' ' || b == b'\t')?;
    let scheme = String::from_utf8_lossy(&body[..scheme_end]).to_ascii_lowercase();
    if !CREDENTIAL_SCHEMES.contains(&scheme.as_str()) {
        return None;
    }
    let credential_start = scheme_end
        + body[scheme_end..]
            .iter()
            .position(|&b| b != b' ' && b != b'\t')?;
    Some(&body[credential_start..])
}

/// A URL, or an origin-form or absolute-form request target, with its userinfo and each
/// credential-named query value masked. The userinfo is the part of the authority before its
/// last `@`. A `://` after a `?` belongs to the query and does not start a scheme.
fn mask_url(url: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(url.len());
    let mut rest = url;
    let before_query = url.iter().position(|&b| b == b'?').unwrap_or(url.len());
    if let Some(scheme_end) = find(&url[..before_query], b"://") {
        let authority_start = scheme_end + 3;
        let authority_len = url[authority_start..]
            .iter()
            .position(|&b| matches!(b, b'/' | b'?' | b'#'))
            .unwrap_or(url.len() - authority_start);
        let authority_end = authority_start + authority_len;
        let authority = &url[authority_start..authority_end];
        out.extend_from_slice(&url[..authority_start]);
        match authority.iter().rposition(|&b| b == b'@') {
            Some(at) => {
                out.extend_from_slice(MASK);
                out.extend_from_slice(&authority[at..]);
            }
            None => out.extend_from_slice(authority),
        }
        rest = &url[authority_end..];
    }
    out.extend_from_slice(&mask_query(rest));
    out
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
    fn a_credential_query_value_in_another_header_is_masked() {
        let head = b"HTTP/1.1 302 Found\r\n\
            Location: https://storage.example/obj?X-Goog-Signature=abc123def456&X-Goog-Expires=3600\r\n\
            Referer: https://app.example/cb?access_token=SECRET-9&x=1\r\n\
            Link: <https://a.example/n?sig=SECRET-8>; rel=\"next\"\r\n\
            \r\n";
        let out = text(head);
        assert!(!out.contains("SECRET"), "{out}");
        assert!(!out.contains("abc123def456"), "{out}");
        for expected in [
            "Location: https://storage.example/obj?X-Goog-Signature=[masked]&X-Goog-Expires=3600\r\n",
            "Referer: https://app.example/cb?access_token=[masked]&x=1\r\n",
            "Link: <https://a.example/n?sig=[masked]>; rel=\"next\"\r\n",
        ] {
            assert!(out.contains(expected), "{expected:?} not in {out}");
        }
    }

    #[test]
    fn a_bearer_or_basic_credential_in_any_header_is_masked() {
        let out =
            text(b"GET / HTTP/1.1\r\nX-Trace: Bearer SECRET-7\r\nX-Other: basic SECRET-8\r\n\r\n");
        assert_eq!(
            out,
            "GET / HTTP/1.1\r\nX-Trace: Bearer [masked]\r\nX-Other: basic [masked]\r\n\r\n"
        );
    }

    #[test]
    fn a_percent_encoded_credential_name_is_masked() {
        let out = text(b"GET /v1/x?%6Bey=SECRET-1&%73ig=SECRET-2&page=2 HTTP/1.1\r\n\r\n");
        assert_eq!(
            out,
            "GET /v1/x?%6Bey=[masked]&%73ig=[masked]&page=2 HTTP/1.1\r\n\r\n"
        );
    }

    #[test]
    fn userinfo_in_an_absolute_target_is_masked() {
        let out = text(
            b"GET https://svc-user:SECRET-PASS@model.example/v1/x HTTP/1.1\r\nHost: model.example\r\n\r\n",
        );
        assert_eq!(
            out,
            "GET https://[masked]@model.example/v1/x HTTP/1.1\r\nHost: model.example\r\n\r\n"
        );
        assert!(!out.contains("SECRET"), "{out}");
    }

    #[test]
    fn a_query_before_a_nested_url_keeps_its_credentials_masked() {
        let out = text(b"GET /x?key=SECRET-1&next=https://a.example/b HTTP/1.1\r\n\r\n");
        assert_eq!(
            out,
            "GET /x?key=[masked]&next=https://a.example/b HTTP/1.1\r\n\r\n"
        );
    }

    #[test]
    fn a_bare_cr_cannot_hide_a_credential_header() {
        let out = text(
            b"GET / HTTP/1.1\r\nX-Trace: 1\rAuthorization: Bearer SECRET-3\r\nAccept: */*\r\n\r\n",
        );
        assert_eq!(out, "GET / HTTP/1.1\r\n[masked]\r\nAccept: */*\r\n\r\n");
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
