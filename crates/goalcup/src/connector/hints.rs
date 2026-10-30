//! Host hints and refusals from the harness's config files and named variables (PRD-11 sections
//! 2 and 3). Each format becomes a `serde_json::Value` for one key walker. A linked, oversized,
//! unreadable or unparsable file is skipped (a memory guard); no value reaches a log or an error.

use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::hosts::{self, Decision, Source};
use super::{Connector, Env, Host, Refusal, launch};
use crate::native::pattern;

/// The `config` hosts named by `host_hints` keys and `hint_env` variables, and `local_model` notes.
pub(crate) fn collect(connector: &Connector, env: &Env, home: &Path) -> (Vec<Host>, Vec<Value>) {
    let mut values: Vec<String> = Vec::new();
    for row in &connector.host_hints {
        for (doc, _) in docs(&row.file, env, home) {
            let found = at(&doc, &row.key).into_iter().filter_map(Value::as_str);
            values.extend(found.map(String::from));
        }
    }
    let named = connector.hint_env.iter().filter_map(|n| env.vars.get(n));
    values.extend(named.cloned());
    let (mut hosts, mut notes) = (Vec::new(), Vec::new());
    for (https, host, port) in values.iter().filter_map(|v| authority(v)) {
        let local = match hosts::check(host, Source::Config) {
            Err(skipped) => skipped.local,
            Ok(checked) => {
                let addrs = (checked.name.as_str(), 443).to_socket_addrs();
                let addrs: Vec<IpAddr> = addrs.into_iter().flatten().map(|a| a.ip()).collect();
                let here = hosts::resolves_only_here(&addrs);
                let never = connector.hosts.decide(&checked.name) == Decision::Never
                    || checked.name.eq_ignore_ascii_case(&env.server_host);
                if https && !here && !never {
                    hosts.push(checked);
                }
                here
            }
        };
        let (l, r) = [("", ""), ("[", "]")][host.contains(':') as usize];
        let shown = format!("{l}{host}{r}:{port}");
        let note = serde_json::json!({ "what": "local_model", "host": shown });
        if local && !notes.contains(&note) {
            notes.push(note);
        }
    }
    (hosts, notes)
}

/// The first `refuse_if_set` key that holds a non-empty value: file and key, never the value.
pub(crate) fn refusal(connector: &Connector, env: &Env, home: &Path) -> Option<Refusal> {
    let set = |v: &&Value| !v.is_null() && v.as_str() != Some("");
    for row in &connector.refuse_if_set {
        for (doc, file) in docs(&row.file, env, home) {
            if let Some(key) = row.keys.iter().find(|k| at(&doc, k).iter().any(set)) {
                let key = key.clone();
                return Some(Refusal::SettingSet { file, key });
            }
        }
    }
    None
}

/// Whether a URL is `https`, and its host and port, the userinfo dropped first. `None` without a
/// `://`, or when the authority holds a backslash, a space or a control byte (PRD-17 N1).
fn authority(value: &str) -> Option<(bool, &str, u16)> {
    let (scheme, rest) = value.trim().split_once("://")?;
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let clean = |b: u8| b > b' ' && b != 0x7f && b != b'\\';
    authority.bytes().all(clean).then_some(())?;
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = match hostport.rsplit_once(':') {
        Some((host, port)) if !port.contains(']') => (host, Some(port)),
        _ => (hostport, None),
    };
    let https = scheme.eq_ignore_ascii_case("https");
    let port = port.map_or(Some(if https { 443 } else { 80 }), |p| p.parse().ok())?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    Some((https, host, port))
}

/// Each readable file a `file` template names (its last component a PRD-04 pattern), and its path.
fn docs(template: &str, env: &Env, home: &Path) -> Vec<(Value, PathBuf)> {
    let path = PathBuf::from(launch::fill(template, home, &env.workspace));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let (Some(dir), Ok(glob)) = (path.parent(), pattern::parse(&name)) else {
        return Vec::new();
    };
    let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
    let fits = |p: &PathBuf| p.file_name().is_some_and(|n| glob.matches(Path::new(n)));
    let mut paths: Vec<PathBuf> = entries.map(|e| e.path()).filter(fits).collect();
    paths.sort();
    let readable = |p: PathBuf| Some((read(&p)?, p));
    paths.into_iter().filter_map(readable).collect()
}

/// One regular file of at most 1 MiB, never through a link (PRD-17 N7), as UTF-8 with a BOM
/// stripped, read by its extension: JSON, JSON with comments, TOML, YAML, else dotenv.
fn read(path: &Path) -> Option<Value> {
    let file = (std::fs::OpenOptions::new().read(true))
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    (meta.is_file() && meta.len() <= 1 << 20).then_some(())?;
    let text = std::io::read_to_string(file.take(1 << 20)).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => serde_json::from_str(text).ok(),
        Some("jsonc") => serde_json::from_str(&strip_jsonc(text)).ok(),
        Some("toml") => toml::from_str(text).ok(),
        Some("yaml" | "yml") => Some(yaml(text)),
        _ => Some(dotenv(text)),
    }
}

/// The values at a dotted key; a segment may be or hold a `*` glob (any key or index, `*_URL`).
fn at<'a>(doc: &'a Value, key: &str) -> Vec<&'a Value> {
    let mut found = vec![doc];
    for seg in key.split('.') {
        let glob = pattern::parse(seg).ok();
        let fits = |k: &str| k == seg || glob.as_ref().is_some_and(|g| g.matches(Path::new(k)));
        let mut next = Vec::new();
        for value in found {
            match value {
                Value::Object(map) => next.extend(map.iter().filter(|e| fits(e.0)).map(|e| e.1)),
                Value::Array(items) => {
                    let items = items.iter().enumerate();
                    next.extend(items.filter(|e| fits(&e.0.to_string())).map(|e| e.1));
                }
                _ => {}
            }
        }
        found = next;
    }
    found
}

/// JSON with `//` and `/* */` comments and trailing commas removed outside strings.
fn strip_jsonc(text: &str) -> String {
    let (mut out, mut chars, mut in_string) = (String::new(), text.chars().peekable(), false);
    while let Some(c) = chars.next() {
        in_string ^= c == '"';
        match (c, chars.peek().copied()) {
            ('\\', _) if in_string => out.extend([Some(c), chars.next()].into_iter().flatten()),
            _ if in_string => out.push(c),
            ('/', Some('/')) => while chars.next_if(|&n| n != '\n').is_some() {},
            ('/', Some('*')) => {
                chars.next();
                let mut prev = None;
                let _ = chars.find(|&n| prev.replace(n) == Some('*') && n == '/');
            }
            (']' | '}', _) => {
                out.truncate(out.trim_end().trim_end_matches(',').len());
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// `KEY=value` lines, with an optional `export ` and the quotes stripped.
fn dotenv(text: &str) -> Value {
    let pair = |line: &str| {
        let (key, value) = line.trim().trim_start_matches("export ").split_once('=')?;
        let (key, value) = (key.trim(), value.trim());
        let quoted = |q: char| value.strip_prefix(q)?.strip_suffix(q);
        let value = Value::String(quoted('"').or(quoted('\'')).unwrap_or(value).into());
        (!key.is_empty() && !key.starts_with('#')).then(|| (key.to_string(), value))
    };
    Value::Object(text.lines().filter_map(pair).collect())
}

/// Block mappings nested by indentation, as `yaml.safe_dump` writes them, scalars as strings;
/// sequences and flow collections are left out. Adapted from 0.7's `model_host.rs:682`.
fn yaml(text: &str) -> Value {
    let mut root = Value::Object(Map::new());
    let mut open: Vec<(usize, String)> = Vec::new();
    for (indent, key, rest) in text.lines().filter_map(yaml_key) {
        open.retain(|(i, _)| *i < indent);
        let parent = open.last().map_or(String::new(), |(_, p)| p.clone());
        let Some(Value::Object(map)) = root.pointer_mut(&parent) else {
            continue;
        };
        let rest = rest.trim();
        if rest.is_empty() || rest.starts_with('#') {
            open.push((indent, format!("{parent}/{key}")));
            map.insert(key, Value::Object(Map::new()));
        } else if let Some(scalar) = yaml_scalar(rest) {
            map.insert(key, Value::String(scalar));
        }
    }
    root
}

/// The indent, key and rest of a `key: rest` line; `None` for a comment, a sequence item or a
/// line with no key.
fn yaml_key(line: &str) -> Option<(usize, String, &str)> {
    let body = line.trim_start_matches(' ');
    let b = body.as_bytes();
    let colon = (0..b.len()).find(|&i| b[i] == b':' && b.get(i + 1).is_none_or(|&n| n == b' '))?;
    let key = body[..colon].trim_end().trim_matches(['"', '\'']);
    let plain = !key.is_empty() && !key.starts_with(['-', '?', '[', '{', '#', '\t']);
    plain.then(|| (line.len() - body.len(), key.to_string(), &body[colon + 1..]))
}

/// A plain or quoted scalar on one line; `None` for anything else.
fn yaml_scalar(s: &str) -> Option<String> {
    if let Some(quote) = s.chars().next().filter(|c| *c == '"' || *c == '\'') {
        return s[1..].split_once(quote).map(|(inner, _)| inner.to_string());
    }
    let plain = s.find(" #").map_or(s, |at| s[..at].trim_end());
    let null = plain.is_empty() || plain == "~" || plain.eq_ignore_ascii_case("null");
    (!null).then(|| plain.to_string())
}
