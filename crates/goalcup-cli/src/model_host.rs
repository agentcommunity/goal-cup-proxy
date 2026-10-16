//! The model service a harness's own config names (D62): `capture.model_host_from`.
//!
//! The participant picks the model service in the harness, and the harness saves its address
//! in its own config. `goalcup start` and each `goalcup continue` read that one value, before
//! anything is created, and the proxy intercepts its host for that run beside the profile's
//! `hosts`. Every other host stays an opaque tunnel, counted and never opened. There is no
//! provider list: whatever service the participant set up is the one recorded.
//!
//! The reader is small on purpose. It opens one file (no symlink, at most
//! [`MAX_CONFIG_BYTES`]) in the shared config home, finds one dotted key in it and keeps only
//! that value. Nothing else of the file is kept or said, and only the host and port of the value
//! are ever logged: a userinfo (`user:pass@`), the path and the query are dropped before any
//! line is written. YAML is read by the block-mapping lookup below (no YAML crate); JSON and
//! TOML by the parsers this binary already links.
//!
//! A value the tracer cannot record (missing, not a URL, plain `http`, loopback, an address
//! literal, an authentication host) is refused with the reason and the way to change it
//! ([`crate::participant::cannot_record_model_service`]); `goalcup prep` makes the same check
//! and keeps the home not ready until the harness names a service that can be recorded. What is
//! checked is the address the config names. A login the harness refreshes at another host (an
//! OAuth provider's token endpoint) is not checked here (D62).

use crate::CliError;
use crate::connector::HarnessConnector;
use crate::proxy::api::{CapturePath, CapturePolicy};
use crate::proxy::connect::ApprovedHost;
use goalcup_protocol::capture_policy::{is_capture_host, local_host_problem, mentions_goalcup};
use goalcup_protocol::harness::{CAPTURE_HOMES_DIR, ModelHostFrom};
use std::path::Path;

/// The largest config file the reader opens.
pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// The request paths, under the service's base path, that carry a known wire format, and the
/// decoder hint each is recorded with. The hint names a wire format, not a provider. A path
/// under the base with none of these is recorded with no hint (capture-only).
const WIRE_PATHS: &[(&str, &str)] = &[
    ("/chat/completions", "openai-chat-completions"),
    ("/messages", "anthropic-messages"),
    ("/responses", "openai-responses"),
];

/// The decoder hints a model host read from the harness's config is recorded with, for
/// `goalcup doctor`.
pub fn wire_hints() -> impl Iterator<Item = &'static str> {
    WIRE_PATHS.iter().map(|(_, hint)| *hint)
}

/// The model service a run records: the host and port of the config's URL, and its path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelHost {
    /// Lowercase DNS name, checked by [`is_capture_host`].
    pub host: String,
    pub port: u16,
    /// The URL's path, without its query or fragment; may be empty.
    pub base_path: String,
    /// Where it was read: `<key> in <file>`.
    pub from: String,
}

impl ModelHost {
    /// `host:port`, the only part of the value that is ever logged.
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// The run's detail line (`host.log`, or the headless start's output).
    pub fn detail(&self) -> String {
        format!("model service: {} ({})", self.authority(), self.from)
    }

    /// The proxy's host rule: each [`WIRE_PATHS`] entry under the base path (and under
    /// `<base>/v1`, where an SDK adds the version itself) with its hint, then every other path
    /// under the base with none.
    pub fn approved(&self) -> ApprovedHost {
        let base = self.base_path.trim_end_matches('/');
        let mut paths = Vec::new();
        for prefix in [base.to_string(), format!("{base}/v1")] {
            for (suffix, hint) in WIRE_PATHS {
                paths.push(CapturePath {
                    path: format!("{prefix}{suffix}"),
                    prefix: false,
                    protocol_hint: Some((*hint).to_string()),
                });
            }
        }
        // The rest of the base, with no hint. A URL with no path records the versioned API
        // (`/v1/`), never every request on the host (a reviewed row may not use `/` either).
        let rest = if base.is_empty() {
            "/v1/".to_string()
        } else {
            format!("{base}/")
        };
        paths.push(CapturePath {
            path: rest,
            prefix: true,
            protocol_hint: None,
        });
        ApprovedHost {
            host: self.host.clone(),
            policy: CapturePolicy { paths },
        }
    }
}

/// Why the config's value cannot be recorded, in words a participant reads: what the harness
/// is set to use (`scheme://host:port` or a short description, never the rest of the value) and
/// the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unrecordable {
    pub shown: String,
    pub reason: String,
}

impl Unrecordable {
    /// "`<name>` is set to use `<shown>`, which Goal Cup cannot record: `<reason>`."
    pub fn sentence(&self, name: &str) -> String {
        format!(
            "{name} is set to use {}, which Goal Cup cannot record: {}.",
            self.shown, self.reason
        )
    }
}

/// The model service `connector`'s config names, for one segment, or `None` for a connector
/// without `capture.model_host_from`. Reads the file once; refuses a value it cannot record with
/// the reason and the way to change it ([`crate::participant::cannot_record_model_service`]).
pub fn for_run(cfg: &Path, connector: &HarnessConnector) -> Result<Option<ModelHost>, CliError> {
    check(cfg, connector).map_err(|why| {
        CliError::Refused(crate::participant::cannot_record_model_service(
            &why.sentence(connector.display_name()),
            &connector.loaded.manifest.id,
        ))
    })
}

/// [`for_run`] without the refusal around it, for `goalcup prep`.
pub fn check(cfg: &Path, connector: &HarnessConnector) -> Result<Option<ModelHost>, Unrecordable> {
    let manifest = &connector.loaded.manifest;
    let Some(from) = &manifest.capture.model_host_from else {
        return Ok(None);
    };
    let home = cfg.join(CAPTURE_HOMES_DIR).join(&manifest.id);
    read(&home, from, &manifest.capture.auth_hosts).map(Some)
}

/// [`check`] on a given config home.
pub fn read(
    home: &Path,
    from: &ModelHostFrom,
    auth_hosts: &[String],
) -> Result<ModelHost, Unrecordable> {
    let source = format!("`{}` in {}", from.key, from.file);
    let refuse = |shown: String, reason: String| Unrecordable { shown, reason };
    let Some(value) = read_value(&home.join(&from.file), &from.file, &from.key) else {
        return Err(refuse(
            "no model service".into(),
            format!("{source} is not set"),
        ));
    };
    let Some(url) = parse_url(&value) else {
        return Err(refuse(
            "an address Goal Cup cannot read".into(),
            format!("{source} is not an http or https URL"),
        ));
    };
    let shown = url.shown();
    let ip = url.host.parse::<std::net::IpAddr>().ok();
    let reason = if !url.https {
        Some("it is plain http, which goes around the recorder")
    } else if ip.is_some_and(|ip| ip.is_loopback())
        || (ip.is_none() && local_host_problem(&url.host).is_some())
    {
        Some("it is on this computer or the local network, which is never recorded")
    } else if ip.is_some() {
        Some("it is an IP address, and Goal Cup records a service by its DNS name")
    } else if !is_capture_host(&url.host) {
        Some("it is not a DNS name Goal Cup can record")
    } else if mentions_goalcup(&url.host) {
        Some("it is Goal Cup's own host")
    } else if auth_hosts.contains(&url.host) {
        Some("it is a sign-in host, which is never recorded")
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(refuse(shown, reason.into()));
    }
    Ok(ModelHost {
        host: url.host,
        port: url.port,
        base_path: url.path,
        from: format!("{} in {}", from.key, from.file),
    })
}

/// The string at `key` in the file at `path` (named `name` in the manifest), or `None` when the
/// file is missing, too large, a symlink or unparseable, or the key is absent or not a string.
fn read_value(path: &Path, name: &str, key: &str) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_CONFIG_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let parts: Vec<&str> = key.split('.').collect();
    let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("json") => json_lookup(text, &parts),
        Some("toml") => toml_lookup(text, &parts),
        Some("yaml" | "yml") => yaml_lookup(text, &parts),
        _ => None,
    }
}

fn json_lookup(text: &str, parts: &[&str]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut at = &value;
    for part in parts {
        at = at.as_object()?.get(*part)?;
    }
    at.as_str().map(str::to_string)
}

fn toml_lookup(text: &str, parts: &[&str]) -> Option<String> {
    let table: toml::Table = text.parse().ok()?;
    let (last, tables) = parts.split_last()?;
    let mut at = &table;
    for part in tables {
        at = at.get(*part)?.as_table()?;
    }
    at.get(*last)?.as_str().map(str::to_string)
}

/// The scalar at `parts` in a YAML document of block mappings (`key:` lines nested by
/// indentation), as a harness's `yaml.safe_dump` writes them. A flow collection, an anchor, a
/// tag or a block scalar at the key is `None`; so is a tab in the indentation.
fn yaml_lookup(text: &str, parts: &[&str]) -> Option<String> {
    let mut depth = 0;
    // The indentation of the last key matched (`None`: the document's top level), and of the
    // keys of the mapping searched now (set by its first line).
    let mut parent: Option<usize> = None;
    let mut level: Option<usize> = None;
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let body = line.trim_start_matches(' ');
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        if body.starts_with('\t') {
            return None;
        }
        let indent = line.len() - body.len();
        if indent == 0 && (body == "---" || body.starts_with("--- ") || body == "...") {
            if depth > 0 {
                return None;
            }
            continue;
        }
        if parent.is_some_and(|p| indent <= p) {
            // The mapping that should hold the next part has ended without it.
            return None;
        }
        let at = *level.get_or_insert(indent);
        if indent > at {
            // Inside the value of a key this lookup does not follow.
            continue;
        }
        if indent < at {
            return None;
        }
        let Some((key, rest)) = yaml_key(body) else {
            continue;
        };
        if key != parts[depth] {
            continue;
        }
        if depth + 1 == parts.len() {
            return yaml_scalar(rest);
        }
        let rest = rest.trim();
        if !rest.is_empty() && !rest.starts_with('#') {
            // A value on the same line: not a nested mapping.
            return None;
        }
        depth += 1;
        parent = Some(indent);
        level = None;
    }
    None
}

/// `key: rest` of one mapping line; `None` for a sequence item or a line with no key.
fn yaml_key(body: &str) -> Option<(String, &str)> {
    if body == "-" || body.starts_with("- ") || body.starts_with(['?', '[', '{']) {
        return None;
    }
    if let Some(quote) = body.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let close = body[1..].find(quote)? + 1;
        let rest = body[close + 1..].trim_start_matches(' ');
        let rest = rest.strip_prefix(':')?;
        return Some((body[1..close].to_string(), rest));
    }
    let bytes = body.as_bytes();
    let colon = (0..bytes.len())
        .find(|&i| bytes[i] == b':' && (i + 1 == bytes.len() || bytes[i + 1] == b' '))?;
    Some((body[..colon].trim_end().to_string(), &body[colon + 1..]))
}

/// A plain, single-quoted or double-quoted scalar on one line; `None` for anything else.
fn yaml_scalar(rest: &str) -> Option<String> {
    let s = rest.trim();
    if let Some(inner) = s.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => return Some(out),
                '\\' => out.push(chars.next()?),
                c => out.push(c),
            }
        }
        return None;
    }
    if let Some(inner) = s.strip_prefix('\'') {
        let mut out = String::new();
        let mut chars = inner.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    out.push('\'');
                    continue;
                }
                return Some(out);
            }
            out.push(c);
        }
        return None;
    }
    let plain = match s.find(" #") {
        Some(at) => s[..at].trim_end(),
        None => s,
    };
    if plain.is_empty()
        || plain.starts_with(['|', '>', '{', '[', '&', '*', '!', '%', '@', '`', '#'])
        || plain == "~"
        || plain.eq_ignore_ascii_case("null")
    {
        return None;
    }
    Some(plain.to_string())
}

/// What the reader keeps of a URL: never its userinfo or query.
struct Url {
    https: bool,
    host: String,
    port: u16,
    path: String,
}

impl Url {
    /// `scheme://host:port`, the only form a refusal shows.
    fn shown(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        if self.host.contains(':') {
            format!("{scheme}://[{}]:{}", self.host, self.port)
        } else {
            format!("{scheme}://{}:{}", self.host, self.port)
        }
    }
}

/// An `http` or `https` URL with a host, or `None`.
fn parse_url(value: &str) -> Option<Url> {
    let value = value.trim();
    let (https, rest) = match value.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => (true, rest),
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => (false, rest),
        _ => return None,
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let path_end = tail.find(['?', '#']).unwrap_or(tail.len());
    let path = &tail[..path_end];
    if path.bytes().any(|b| b <= b' ' || b == 0x7f) || path.len() > 512 {
        return None;
    }
    // A userinfo is dropped here, before anything can show it.
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = if let Some(literal) = hostport.strip_prefix('[') {
        let (literal, after) = literal.split_once(']')?;
        let port = match after {
            "" => None,
            p => Some(p.strip_prefix(':')?),
        };
        (literal.to_ascii_lowercase(), port)
    } else {
        match hostport.rsplit_once(':') {
            Some((host, port)) => (host.to_ascii_lowercase(), Some(port)),
            None => (hostport.to_ascii_lowercase(), None),
        }
    };
    let host = host.strip_suffix('.').unwrap_or(&host).to_string();
    if host.is_empty() || host.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return None;
    }
    let port = match port {
        None => {
            if https {
                443
            } else {
                80
            }
        }
        Some(p) if !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()) => {
            match p.parse::<u16>() {
                Ok(0) | Err(_) => return None,
                Ok(port) => port,
            }
        }
        Some(_) => return None,
    };
    Some(Url {
        https,
        host,
        port,
        path: path.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_yaml_reader_finds_one_nested_scalar_and_nothing_else() {
        let doc = "\
# a harness config
model:
  default: some-model
  provider: some-provider
  base_url: https://api.models.example.test/v1   # set by setup
  nested:
    base_url: https://wrong.example
other:
  base_url: https://other.example
sessions:
  write_json_snapshots: true
";
        assert_eq!(
            yaml_lookup(doc, &["model", "base_url"]).as_deref(),
            Some("https://api.models.example.test/v1")
        );
        assert_eq!(
            yaml_lookup(doc, &["model", "nested", "base_url"]).as_deref(),
            Some("https://wrong.example")
        );
        assert_eq!(yaml_lookup(doc, &["model", "api_key"]), None);
        assert_eq!(yaml_lookup(doc, &["model"]), None, "a mapping is no scalar");
        assert_eq!(
            yaml_lookup(
                "model:\n  base_url: \"https://q.example/v1\"\n",
                &["model", "base_url"]
            )
            .as_deref(),
            Some("https://q.example/v1")
        );
        assert_eq!(
            yaml_lookup(
                "model:\n  'base_url': 'https://it''s.example'\n",
                &["model", "base_url"]
            )
            .as_deref(),
            Some("https://it's.example")
        );
        assert_eq!(
            yaml_lookup("model: gpt-5\nbase_url: x\n", &["model", "base_url"]),
            None,
            "a string model holds no key"
        );
        assert_eq!(
            yaml_lookup(
                "model:\nbase_url: https://top.example\n",
                &["model", "base_url"]
            ),
            None,
            "a key of the top level is not the model's"
        );
        assert_eq!(
            yaml_lookup(
                "model: {base_url: https://flow.example}\n",
                &["model", "base_url"]
            ),
            None
        );
        assert_eq!(
            yaml_lookup(
                "model:\n  base_url: |\n    https://block.example\n",
                &["model", "base_url"]
            ),
            None
        );
    }

    #[test]
    fn json_and_toml_are_read_by_their_parsers() {
        assert_eq!(
            json_lookup(
                r#"{"model":{"base_url":"https://j.example/v1"}}"#,
                &["model", "base_url"]
            )
            .as_deref(),
            Some("https://j.example/v1")
        );
        assert_eq!(
            json_lookup(r#"{"model":{"base_url":3}}"#, &["model", "base_url"]),
            None
        );
        assert_eq!(
            toml_lookup(
                "[model]\nbase_url = \"https://t.example\"\n",
                &["model", "base_url"]
            )
            .as_deref(),
            Some("https://t.example")
        );
        assert_eq!(
            toml_lookup("base_url = \"https://t.example\"\n", &["base_url"]).as_deref(),
            Some("https://t.example")
        );
    }

    #[test]
    fn a_url_keeps_its_host_port_and_path_and_drops_its_userinfo() {
        let url = parse_url("https://user:hunter2@Models.Example.TEST.:8443/v1/?x=1#f").unwrap();
        assert!(url.https);
        assert_eq!(url.host, "models.example.test");
        assert_eq!(url.port, 8443);
        assert_eq!(url.path, "/v1/");
        assert_eq!(url.shown(), "https://models.example.test:8443");
        assert!(!url.shown().contains("hunter2"));
        assert_eq!(parse_url("https://openrouter.ai/api/v1").unwrap().port, 443);
        assert_eq!(
            parse_url("http://localhost:11434/v1").unwrap().shown(),
            "http://localhost:11434"
        );
        assert_eq!(
            parse_url("http://[::1]:8080").unwrap().shown(),
            "http://[::1]:8080"
        );
        for bad in [
            "api.models.example.test",
            "ftp://x.example",
            "https://",
            "https://x:0",
            "https://x:99999",
            "https://x:a",
        ] {
            assert!(parse_url(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_host_rule_hints_known_wire_paths_and_records_the_rest_of_the_base() {
        let host = ModelHost {
            host: "models.example.test".into(),
            port: 443,
            base_path: "/v1/".into(),
            from: "model.base_url in config.yaml".into(),
        };
        let rule = host.approved();
        assert_eq!(rule.host, "models.example.test");
        let hint = |path: &str| {
            crate::proxy::metadata::select(&rule.policy, path).map(|p| p.protocol_hint.clone())
        };
        assert_eq!(
            hint("/v1/chat/completions"),
            Some(Some("openai-chat-completions".into()))
        );
        assert_eq!(
            hint("/v1/messages"),
            Some(Some("anthropic-messages".into()))
        );
        assert_eq!(hint("/v1/models"), Some(None));
        assert_eq!(hint("/v2/chat/completions"), None);
        // A URL with no path records the versioned API, never every path on the host.
        let bare = ModelHost {
            base_path: String::new(),
            ..host.clone()
        }
        .approved();
        let bare_hint =
            |path: &str| crate::proxy::metadata::select(&bare.policy, path).map(|p| p.prefix);
        assert_eq!(bare_hint("/v1/messages"), Some(false));
        assert_eq!(bare_hint("/v1/models"), Some(true));
        assert_eq!(bare_hint("/healthz"), None);
        assert!(!bare.policy.paths.iter().any(|p| p.path == "/"));
        assert_eq!(
            host.detail(),
            "model service: models.example.test:443 (model.base_url in config.yaml)"
        );
    }
}
