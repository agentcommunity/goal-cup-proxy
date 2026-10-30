//! Metadata masking (PRD-05 section 7): OPEN meta, END tails and upstream-text events, in place,
//! same length, before they are written.
//!
//! The header, scheme, URL and query rules are copied from the 0.7 proxy's header masking, with
//! the `[masked]` marker replaced by `*` of the same byte length. PRD-03 parses heads; this file
//! sees headers only as `[name, value]` arrays. Object keys and header names are never masked.
//! Nothing here fails or panics: a value it cannot keep as text is starred whole.

use serde_json::Value;

/// Header names whose values are always masked, compared lower-cased.
const CREDENTIAL_NAMES: [&str; 2] = ["cookie", "set-cookie"];

/// Any header name containing one of these is masked, compared lower-cased.
const CREDENTIAL_PARTS: [&str; 5] = ["key", "token", "secret", "password", "auth"];

/// Authorization schemes whose credential is the rest of the value. Masked in any header.
const CREDENTIAL_SCHEMES: [&str; 2] = ["bearer", "basic"];

/// The HTTP/2 host pseudo-header. Its name holds `auth`, but it is the host, which is never
/// masked: only a userinfo before its last `@` is.
const AUTHORITY: &str = ":authority";

/// Meta keys whose values are never masked.
const CLEAR_KEYS: [&str; 5] = ["method", "status", "host", "port", "alpn"];

/// The connector name rules: `header_prefix` values and `json_field` names, lower-cased.
#[derive(Debug, Default)]
pub(crate) struct Names {
    pub(crate) header_prefixes: Vec<String>,
    pub(crate) field_names: Vec<String>,
}

impl Names {
    /// Is this name, lower-cased and trimmed, one whose value is a credential?
    fn is_credential(&self, name: &[u8]) -> bool {
        let name = String::from_utf8_lossy(name.trim_ascii()).to_ascii_lowercase();
        CREDENTIAL_NAMES.contains(&name.as_str())
            || CREDENTIAL_PARTS.iter().any(|p| name.contains(p))
            || self
                .header_prefixes
                .iter()
                .any(|p| name.starts_with(p.as_str()))
            || self.field_names.contains(&name)
    }

    /// A query key: the name rule, `sig`, or any name holding `signature`, percent-decoded once.
    fn is_credential_query(&self, name: &[u8]) -> bool {
        let decoded = percent_decode(name);
        let lower = String::from_utf8_lossy(&decoded).to_ascii_lowercase();
        self.is_credential(&decoded) || lower == "sig" || lower.contains("signature")
    }
}

/// Masks `meta` in place (section 7). `scrub` is the whole-record masker on one string; an
/// answer of another byte length stars that string whole.
///
/// An object with `headers` gets a `masked` list of the header and trailer names whose value
/// changed, in order, a name once per pair (two `cookie` headers list it twice). `masked` is
/// PRD-01's OPEN meta field, so an object with only `trailers` gets none.
pub(crate) fn mask_meta(meta: &mut Value, names: &Names, scrub: &dyn Fn(&str) -> String) {
    walk(meta, names, scrub);
}

/// Steps 1 to 5 over one value and everything under it.
fn walk(meta: &mut Value, names: &Names, scrub: &dyn Fn(&str) -> String) {
    let mut stack: Vec<&mut Value> = vec![meta];
    while let Some(value) = stack.pop() {
        let map = match value {
            Value::String(s) => {
                scrub_in_place(s, scrub);
                continue;
            }
            Value::Array(items) => {
                stack.extend(items.iter_mut());
                continue;
            }
            Value::Object(map) => map,
            Value::Null | Value::Bool(_) | Value::Number(_) => continue,
        };
        let mut masked = Vec::new();
        for key in ["headers", "trailers"] {
            if let Some(Value::Array(pairs)) = map.get_mut(key) {
                for pair in pairs.iter_mut() {
                    mask_pair(pair, names, scrub, &mut masked);
                }
            }
        }
        if let Some(Value::String(path)) = map.get_mut("path") {
            scrub_in_place(path, scrub);
            mask_bytes_of(path, mask_userinfo);
        }
        let holds_headers = map.contains_key("headers");
        if holds_headers {
            let list = masked.into_iter().map(Value::String).collect();
            map.insert("masked".to_string(), Value::Array(list));
        }
        for (key, child) in map.iter_mut() {
            let done = match key.as_str() {
                "headers" | "trailers" => child.is_array(),
                "path" => child.is_string(),
                "masked" => holds_headers,
                other => CLEAR_KEYS.contains(&other),
            };
            if !done {
                stack.push(child);
            }
        }
    }
}

/// One `[name, value]` pair. The name stays; the value is masked by steps 1 to 3, and the name
/// goes into `masked` when any byte changed. A pair of another shape has every string scrubbed.
fn mask_pair(
    pair: &mut Value,
    names: &Names,
    scrub: &dyn Fn(&str) -> String,
    masked: &mut Vec<String>,
) {
    let Value::Array(items) = pair else {
        return walk(pair, names, scrub);
    };
    let (name, value) = match items.as_mut_slice() {
        [Value::String(name), Value::String(value), rest @ ..] => {
            for extra in rest {
                walk(extra, names, scrub);
            }
            (name, value)
        }
        _ => {
            for item in items.iter_mut() {
                walk(item, names, scrub);
            }
            return;
        }
    };
    let before = value.clone();
    let authority = name.eq_ignore_ascii_case(AUTHORITY);
    if value.contains(['\r', '\n']) || (!authority && names.is_credential(name.as_bytes())) {
        *value = "*".repeat(value.len());
    } else {
        scrub_in_place(value, scrub);
        mask_bytes_of(value, |bytes| {
            if authority && let Some(at) = bytes.iter().rposition(|&b| b == b'@') {
                bytes[..at].fill(b'*');
            }
            mask_value(bytes, names);
        });
    }
    if *value != before {
        masked.push(name.clone());
    }
}

/// `s` becomes `scrub(s)`, or all `*` when the answer has another byte length.
fn scrub_in_place(s: &mut String, scrub: &dyn Fn(&str) -> String) {
    let out = scrub(s);
    *s = if out.len() == s.len() {
        out
    } else {
        "*".repeat(s.len())
    };
}

/// Runs a byte rule over `s`. Every rule stars whole ASCII-bounded runs, so the text stays UTF-8;
/// if it did not, the value is starred whole.
fn mask_bytes_of(s: &mut String, rule: impl FnOnce(&mut [u8])) {
    let mut bytes = std::mem::take(s).into_bytes();
    rule(&mut bytes);
    *s = String::from_utf8(bytes).unwrap_or_else(|e| "*".repeat(e.as_bytes().len()));
}

/// Step 3: the credential of a `Bearer` or `Basic` value, else each URL's userinfo and
/// credential query values.
fn mask_value(value: &mut [u8], names: &Names) {
    let start = value
        .iter()
        .position(|&b| b != b' ' && b != b'\t')
        .unwrap_or(value.len());
    if let Some(offset) = scheme_credential(&value[start..]) {
        value[start + offset..].fill(b'*');
        return;
    }
    let mut index = 0;
    while index < value.len() {
        if is_url_delimiter(value[index]) {
            index += 1;
            continue;
        }
        let end = value[index..]
            .iter()
            .position(|&b| is_url_delimiter(b))
            .map_or(value.len(), |i| index + i);
        let url = &mut value[index..end];
        let rest = mask_userinfo_end(url);
        mask_query(&mut url[rest..], names);
        index = end;
    }
}

/// The bytes that end a URL inside a header value.
fn is_url_delimiter(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'"' | b'<' | b'>')
}

/// Where the credential starts, when `body` is `Bearer` or `Basic`, white space and a non-empty
/// credential. The scheme is compared case-insensitively.
fn scheme_credential(body: &[u8]) -> Option<usize> {
    let scheme_end = body.iter().position(|&b| b == b' ' || b == b'\t')?;
    let scheme = String::from_utf8_lossy(&body[..scheme_end]).to_ascii_lowercase();
    if !CREDENTIAL_SCHEMES.contains(&scheme.as_str()) {
        return None;
    }
    let gap = body[scheme_end..]
        .iter()
        .position(|&b| b != b' ' && b != b'\t')?;
    Some(scheme_end + gap)
}

/// Step 4: a URL's userinfo starred, nothing else.
fn mask_userinfo(url: &mut [u8]) {
    mask_userinfo_end(url);
}

/// Stars the userinfo (the authority up to its last `@`) and returns where the authority ends,
/// or 0 when there is no `://` before the first `?` (a `://` after `?` is query).
fn mask_userinfo_end(url: &mut [u8]) -> usize {
    let before_query = url.iter().position(|&b| b == b'?').unwrap_or(url.len());
    let Some(scheme_end) = url[..before_query].windows(3).position(|w| w == b"://") else {
        return 0;
    };
    let start = scheme_end + 3;
    let end = url[start..]
        .iter()
        .position(|&b| matches!(b, b'/' | b'?' | b'#'))
        .map_or(url.len(), |i| start + i);
    if let Some(at) = url[start..end].iter().rposition(|&b| b == b'@') {
        url[start..start + at].fill(b'*');
    }
    end
}

/// Stars the value of each credential query parameter after the first `?`. Parameters split on
/// `&`; one without `=` has no value.
fn mask_query(target: &mut [u8], names: &Names) {
    let Some(question) = target.iter().position(|&b| b == b'?') else {
        return;
    };
    let mut start = question + 1;
    while start <= target.len() {
        let end = target[start..]
            .iter()
            .position(|&b| b == b'&')
            .map_or(target.len(), |i| start + i);
        let pair = &mut target[start..end];
        if let Some(equals) = pair.iter().position(|&b| b == b'=')
            && names.is_credential_query(&pair[..equals])
        {
            pair[equals + 1..].fill(b'*');
        }
        start = end + 1;
    }
}

/// The bytes with each `%XX` escape decoded once. A malformed escape is kept as received.
fn percent_decode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some((&byte, tail)) = rest.split_first() {
        if let (b'%', [high, low, after @ ..]) = (byte, tail)
            && let (Some(high), Some(low)) = (hex_value(*high), hex_value(*low))
        {
            out.push(high * 16 + low);
            rest = after;
        } else {
            out.push(byte);
            rest = tail;
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

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    fn identity(s: &str) -> String {
        s.to_string()
    }

    fn strings(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(items) => items.iter().for_each(|i| strings(i, out)),
            Value::Object(map) => map.values().for_each(|i| strings(i, out)),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    /// Runs `mask_meta` with `scrub` and checks every string kept its byte length and every
    /// header name stayed. Returns the masked meta.
    fn run(
        meta: Value,
        prefixes: &[&str],
        fields: &[&str],
        scrub: &dyn Fn(&str) -> String,
    ) -> Value {
        let rules = Names {
            header_prefixes: prefixes.iter().map(|s| s.to_string()).collect(),
            field_names: fields.iter().map(|s| s.to_string()).collect(),
        };
        let mut out = meta.clone();
        mask_meta(&mut out, &rules, scrub);
        let mut masked_out = out.clone();
        if let Some(map) = masked_out.as_object_mut() {
            map.remove("masked");
        }
        let (mut a, mut b) = (Vec::new(), Vec::new());
        strings(&meta, &mut a);
        strings(&masked_out, &mut b);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.len(), y.len(), "{x:?} -> {y:?}");
        }
        assert_eq!(names(&meta), names(&out));
        out
    }

    fn names(meta: &Value) -> Vec<Value> {
        meta["headers"]
            .as_array()
            .map(|h| h.iter().map(|p| p[0].clone()).collect())
            .unwrap_or_default()
    }

    fn head(headers: &[(&str, &str)]) -> Value {
        let pairs: Vec<Value> = headers.iter().map(|(n, v)| json!([n, v])).collect();
        json!({"method": "POST", "path": "/v1/x", "ver": "1.1", "headers": pairs})
    }

    fn value_of<'a>(meta: &'a Value, name: &str) -> &'a str {
        let pair = meta["headers"]
            .as_array()
            .and_then(|h| h.iter().find(|p| p[0] == name));
        pair.and_then(|p| p[1].as_str()).unwrap_or("<absent>")
    }

    fn stars(s: &str) -> bool {
        !s.is_empty() && s.bytes().all(|b| b == b'*')
    }

    #[test]
    fn name_rules_mask_the_whole_value_and_list_the_name() {
        let starred = [
            "Authorization",
            "x-api-key",
            "x-goog-api-key",
            "Cookie",
            "set-cookie",
            "x-my-auth-token",
            "PROXY-AUTHORIZATION",
            "x-client-secret",
            "x-db-password",
            "x-acme-session",
            "x-tool-turn-state",
            "x-tool-installation-id",
            "x-ratelimit-remaining-tokens",
        ];
        let clear = [
            "user-agent",
            "anthropic-version",
            "x-request-id",
            "thread-id",
            "content-type",
        ];
        let mut headers: Vec<(&str, &str)> = starred.iter().map(|n| (*n, "val-1234")).collect();
        headers.extend(clear.iter().map(|n| (*n, "val-1234")));
        let meta = head(&headers);
        let out = run(
            meta,
            &["x-acme-"],
            &["x-tool-turn-state", "x-tool-installation-id"],
            &identity,
        );
        for name in starred {
            assert!(stars(value_of(&out, name)), "{name}");
        }
        for name in clear {
            assert_eq!(value_of(&out, name), "val-1234", "{name}");
        }
        assert_eq!(out["masked"], json!(starred));
        assert_eq!(out["method"], "POST");
        assert_eq!(out["path"], "/v1/x");
    }

    #[test]
    fn nothing_masked_lists_nothing() {
        let out = run(
            head(&[("content-type", "application/json")]),
            &[],
            &[],
            &identity,
        );
        assert_eq!(out["masked"], json!([]));
        let out = run(json!({"path": "/x"}), &[], &[], &identity);
        assert!(out.get("masked").is_none());
    }

    #[test]
    fn scheme_url_and_query_rules_inside_other_values() {
        let cases = [
            ("Bearer SECRET-7", "Bearer ********"),
            ("  basic SECRET-8", "  basic ********"),
            (
                "https://storage.example/o?X-Goog-Signature=abc123&X-Goog-Expires=3600",
                "https://storage.example/o?X-Goog-Signature=******&X-Goog-Expires=3600",
            ),
            (
                "<https://a.example/n?sig=S1>; rel=\"next\"",
                "<https://a.example/n?sig=**>; rel=\"next\"",
            ),
            (
                "/v1/x?%6Bey=S2&%73ig=S3&access_token=S4&page=2",
                "/v1/x?%6Bey=**&%73ig=**&access_token=**&page=2",
            ),
            (
                "https://svc:p@ss@model.example/v1",
                "https://********@model.example/v1",
            ),
            (
                "https://h.example/cb?next=https://u:pw@b.example/&token=T1",
                "https://h.example/cb?next=https://u:pw@b.example/&token=**",
            ),
            (
                "/cb?key=S5&next=https://a.example/b",
                "/cb?key=**&next=https://a.example/b",
            ),
        ];
        for (input, expected) in cases {
            let out = run(head(&[("x-link", input)]), &[], &[], &identity);
            assert_eq!(value_of(&out, "x-link"), expected);
            assert_eq!(out["masked"], json!(["x-link"]));
        }
    }

    #[test]
    fn a_cr_or_lf_masks_the_whole_value() {
        for input in ["1\rAuthorization: Bearer S", "a\nb", "\r"] {
            let out = run(head(&[("x-trace", input)]), &[], &[], &identity);
            assert!(stars(value_of(&out, "x-trace")), "{input:?}");
        }
    }

    #[test]
    fn path_and_pseudo_path_lose_userinfo_only() {
        let meta = json!({
            "method": "GET",
            "path": "https://svc-user:PASS@model.example/v1/x",
            "headers": [[":path", "https://u:PW@h.example/x"], [":authority", "h.example"]],
        });
        let out = run(meta, &[], &[], &identity);
        assert_eq!(out["path"], "https://*************@model.example/v1/x");
        assert_eq!(value_of(&out, ":path"), "https://****@h.example/x");
        assert_eq!(value_of(&out, ":authority"), "h.example");
        assert_eq!(out["masked"], json!([":path"]));
        let out = run(
            head(&[(":authority", "u:pw@h.example")]),
            &[],
            &[],
            &identity,
        );
        assert_eq!(value_of(&out, ":authority"), "****@h.example");
        let native = json!({"path": "/home/u/.agent/sessions/2026/rollout-1.jsonl"});
        assert_eq!(run(native.clone(), &[], &[], &identity), native);
    }

    #[test]
    fn every_string_goes_through_scrub_except_names_and_clear_keys() {
        let fake = |s: &str| s.replace("QQQQ", "****");
        let meta = json!({
            "method": "QQQQ",
            "host": "QQQQ.example",
            "status": 200,
            "port": 443,
            "note": {"QQQQ": ["see QQQQ here"]},
            "headers": [["x-QQQQ", "a QQQQ b"], ["x-plain", "ok"]],
        });
        let out = run(meta, &[], &[], &fake);
        assert_eq!(out["method"], "QQQQ");
        assert_eq!(out["host"], "QQQQ.example");
        assert_eq!(out["note"], json!({"QQQQ": ["see **** here"]}));
        assert_eq!(value_of(&out, "x-QQQQ"), "a **** b");
        assert_eq!(out["masked"], json!(["x-QQQQ"]));
        let wrong_length = |s: &str| format!("{s}!");
        let out = run(json!({"note": "abc"}), &[], &[], &wrong_length);
        assert_eq!(out["note"], "***");
    }

    #[test]
    fn trailers_and_odd_shapes_never_fail() {
        let meta = json!({
            "trailers": [["x-goog-api-key", "k1"], ["grpc-status", "0"], "loose", [1, 2], []],
            "headers": [["only-name"], 7],
        });
        let out = run(meta, &[], &[], &identity);
        assert_eq!(out["trailers"][0][1], "**");
        assert_eq!(out["trailers"][1][1], "0");
        assert_eq!(out["masked"], json!(["x-goog-api-key"]));
    }
}
