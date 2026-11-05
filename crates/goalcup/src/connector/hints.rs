//! Host hints and refusals from the harness's config files and named variables (PRD-11 sections
//! 2 and 3). Each format becomes a `serde_json::Value` for one key walker. A linked, oversized,
//! unreadable or unparsable file is skipped (a memory guard) and named in `unread`; a skipped hint
//! is a log line naming the file or variable, the key and the reason. No value reaches either.

use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::hosts::{self, Decision, HostList, Source};
use super::{CaSettings, Connector, Env, Host, Refusal, SettingRow, launch};
use crate::native::pattern;

/// Reads one start's hint and refusal files and keeps what PRD-10 shows: each file there but not
/// read, and the log lines.
pub(crate) struct Reader<'a> {
    pub(crate) env: &'a Env,
    pub(crate) home: &'a Path,
    pub(crate) unread: Vec<PathBuf>,
    pub(crate) log: Vec<String>,
}

impl Reader<'_> {
    /// The `config` hosts named by `host_hints` keys and `hint_env` variables, and `local_model`
    /// notes. `hosts` already holds every `never` name, so `decide` is the one never test.
    pub(crate) fn collect(&mut self, c: &Connector, hosts: &HostList) -> (Vec<Host>, Vec<Value>) {
        // Each value once, with where it came from: file and key, or the variable.
        let mut values: Vec<(String, String)> = Vec::new();
        for row in &c.host_hints {
            for (doc, file) in self.docs(&row.file) {
                let from = format!("{}: {}", file.display(), row.key);
                let found = at(&doc, &row.key).into_iter().filter_map(Value::as_str);
                values.extend(found.map(|v| (from.clone(), v.to_string())));
            }
        }
        let named = c
            .hint_env
            .iter()
            .filter_map(|n| Some((n.clone(), self.env.vars.get(n)?)));
        values.extend(named.map(|(n, v)| (n, v.clone())));
        let mut seen = std::collections::HashSet::new();
        values.retain(|(_, v)| seen.insert(v.clone()));
        let (mut found, mut notes) = (Vec::new(), Vec::new());
        let here = |name: &str| {
            let addrs = (name, 443).to_socket_addrs().into_iter().flatten();
            hosts::resolves_only_here(&addrs.map(|a| a.ip()).collect::<Vec<IpAddr>>())
        };
        let never = |h: &Host| hosts.decide(&h.name) == Decision::Never;
        for (from, value) in &values {
            let Some((https, host, port)) = authority(value) else {
                self.log.push(format!("{from}: not a URL, skipped"));
                continue;
            };
            let checked = hosts::check(host, Source::Config);
            if checked.as_ref().map_or_else(|s| s.local, |h| here(&h.name)) {
                let (l, r) = [("", ""), ("[", "]")][host.contains(':') as usize];
                let note = json!({ "ev": "note", "what": "local_model", "host": format!("{l}{host}{r}:{port}") });
                notes.extend((!notes.contains(&note)).then_some(note));
                continue;
            }
            let why = match checked {
                Err(skipped) => skipped.reason,
                Ok(h) if never(&h) => "a host never decrypted".into(),
                Ok(_) if !https => "plain http, not https".into(),
                Ok(h) => {
                    found.push(h);
                    continue;
                }
            };
            self.log.push(format!("{from}: {why}, skipped"));
        }
        (found, notes)
    }

    /// The first `refuse_if_set` key holding a value (not null, "", [] or {}; `false` is a value):
    /// file and key, never the value.
    pub(crate) fn refusal(&mut self, c: &Connector) -> Option<Refusal> {
        let empty = [json!(null), json!(""), json!([]), json!({})];
        for row in &c.refuse_if_set {
            for (doc, file) in self.docs(&row.file) {
                let set = |k: &&String| at(&doc, k).into_iter().any(|v| !empty.contains(v));
                if let Some(key) = row.keys.iter().find(set).cloned() {
                    return Some(Refusal::SettingSet { file, key });
                }
            }
        }
        None
    }

    /// Each readable file a `file` template names (its last component a PRD-04 pattern), and its
    /// path. A file there but not read goes in `unread` once; a folder whose name fits is skipped.
    fn docs(&mut self, template: &str) -> Vec<(Value, PathBuf)> {
        let path = PathBuf::from(launch::fill(template, self.home, &self.env.workspace));
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let (Some(dir), Ok(glob)) = (path.parent(), pattern::parse(&name)) else {
            return Vec::new();
        };
        let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
        let entries = entries.filter(|e| !e.file_type().is_ok_and(|t| t.is_dir()));
        let fits = |p: &PathBuf| p.file_name().is_some_and(|n| glob.matches(Path::new(n)));
        let mut paths: Vec<PathBuf> = entries.map(|e| e.path()).filter(fits).collect();
        paths.sort();
        let mut out = Vec::new();
        for path in paths {
            match read(&path) {
                Some(doc) => out.push((doc, path)),
                None if !self.unread.contains(&path) => self.unread.push(path),
                None => {}
            }
        }
        out
    }
}

/// Whether a URL is `https`, and its host and port, the userinfo dropped first. `None` without a
/// `://`, or when the authority holds a backslash, a space or a control byte (PRD-17 N1).
pub(super) fn authority(value: &str) -> Option<(bool, &str, u16)> {
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

/// One regular file of at most 1 MiB, never through a link (PRD-17 N7), as UTF-8 with a BOM
/// stripped, read by its extension: JSON (comments and trailing commas allowed, as the OpenCode
/// family reads every config file), TOML, YAML, else dotenv.
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
        Some("json" | "jsonc") => serde_json::from_str(&strip_jsonc(text)).ok(),
        Some("toml") => toml::from_str(text).ok(),
        Some("yaml" | "yml") => Some(yaml(text)),
        _ => Some(dotenv(text)),
    }
}

/// One address's `ca_settings` rows, or why none can be written; and for an entry holding its own
/// CA, the file to fill with that CA's PEM and then the run bundle (`LaunchPlan::finish_env`).
pub(crate) struct Trust {
    pub(crate) url: String,
    pub(crate) rows: Result<Vec<SettingRow>, String>,
    pub(crate) combine: Option<(PathBuf, Vec<u8>)>,
}

/// The rows giving each address the harness uses the run bundle (`ca_settings`): the CA key on
/// each participant entry holding an address, and a `goalcup-trust-*` entry for every other
/// address used; when the file names none, one for each `fallback` address. An entry holding its
/// own CA points at a file in the isolated home holding that CA and the run bundle; its own file
/// is never changed, and one that cannot be read leaves the entry as it is. An address is an
/// `https` URL whose host `hosts` decrypts, so a host left to the OS trust store (local, private,
/// never) keeps it. An own CA path is read as the harness reads it ([`own_path`]).
pub(crate) fn trust_rows(t: &CaSettings, home: &Path, env: &Env, hosts: &HostList) -> Vec<Trust> {
    let file = launch::fill(&t.file, home, Path::new(""));
    let user = env.vars.get("HOME").map(Path::new);
    let beside = Path::new(&file).parent().unwrap_or(Path::new(""));
    let own_path = |own: &str| own_path(own, user, beside);
    let doc = read(Path::new(&file)).unwrap_or_default();
    let address = |v: &Value| {
        let url = v.as_str()?.trim();
        let (https, host, _) = authority(url)?;
        let decrypted = matches!(hosts.decide(host), Decision::Listed { .. });
        (https && hosts::check(host, Source::Config).is_ok() && decrypted).then(|| url.to_string())
    };
    let same = |url: &str| url.trim_end_matches('/').to_ascii_lowercase();
    let row = |key: String, value: &str| SettingRow {
        file: t.file.clone(),
        key,
        value: value.to_string(),
    };
    let (mut trusts, mut have) = (Vec::new(), Vec::new());
    for pattern in &t.entries {
        let parts = pattern.split('.').count();
        for (path, value) in at_paths(&doc, pattern) {
            let ours = path.split('.').any(|p| p.starts_with("goalcup-"));
            // A key holding a `.` would not write back to the same entry.
            let (Some(url), false, true) = (address(value), ours, path.split('.').count() == parts)
            else {
                continue;
            };
            have.push(same(&url));
            let ca = format!("{}.{}", path.rsplit_once('.').unwrap().0, t.key);
            let mut own = at(&doc, &ca).into_iter().filter_map(Value::as_str);
            let own = own.find(|v| !v.trim().is_empty() && *v != t.value);
            let (rows, combine) = match own.map(|own| combined(home, &path, own, own_path)) {
                None => (Ok(vec![row(ca, &t.value)]), None),
                Some(Ok((to, pem))) => (Ok(vec![row(ca, &to.to_string_lossy())]), Some((to, pem))),
                Some(Err(why)) => (Err(format!("{file}: {ca}: {why}")), None),
            };
            trusts.push(Trust { url, rows, combine });
        }
    }
    let mut used: Vec<String> = t
        .uses
        .iter()
        .flat_map(|k| at(&doc, k))
        .filter_map(address)
        .collect();
    if have.is_empty() && used.is_empty() {
        used = t
            .fallback
            .iter()
            .filter_map(|u| address(&Value::from(&**u)))
            .collect();
    }
    let mut names = Vec::new();
    for url in used {
        if have.contains(&same(&url)) {
            continue;
        }
        have.push(same(&url));
        let rest = url
            .split_once("://")
            .map_or(&*url, |r| r.1)
            .to_ascii_lowercase();
        let name: Vec<&str> = rest
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|p| !p.is_empty())
            .collect();
        let mut name = name.join("-");
        // Addresses that differ only in punctuation share a name: the later one adds a hash.
        if names.contains(&name) {
            name = format!("{name}-{}", short(&url));
        }
        names.push(name.clone());
        let key = t.add.replace('*', &format!("goalcup-trust-{name}"));
        let ca = format!("{}.{}", key.rsplit_once('.').unwrap().0, t.key);
        let rows = Ok(vec![row(key, &url), row(ca, &t.value)]);
        trusts.push(Trust {
            url,
            rows,
            combine: None,
        });
    }
    trusts
}

/// The file in the isolated home that the entry at `path`, holding its own CA `own`, points at
/// instead, and that CA's PEM. The participant's path is kept beside it, so a later start, which
/// reads the file's path in the entry, still finds their CA.
fn combined(
    home: &Path,
    path: &str,
    own: &str,
    own_path: impl Fn(&str) -> PathBuf,
) -> Result<(PathBuf, Vec<u8>), &'static str> {
    let to = home
        .join(".goalcup-ca")
        .join(format!("{}.pem", short(path)));
    let from = to.with_extension("from");
    let theirs = match Path::new(own) == to {
        true => std::fs::read_to_string(&from).map_err(|_| "its own CA path is lost")?,
        false => own.to_string(),
    };
    let pem = read_ca(&own_path(&theirs)).ok_or("its own CA file cannot be read")?;
    if theirs == own {
        launch::replace(&from, own.as_bytes()).map_err(|_| "cannot keep its own CA path")?;
    }
    Ok((to, pem))
}

/// A participant's CA path as Hermes opens it: a leading `~/` from their HOME, a relative path
/// from the config file's folder `beside`.
fn own_path(own: &str, user: Option<&Path>, beside: &Path) -> PathBuf {
    match (own.strip_prefix("~/"), user) {
        (Some(rest), Some(user)) => user.join(rest),
        _ => beside.join(own),
    }
}

/// 8 hex of the SHA-256 of `s`: a name part that stays the same from start to start.
fn short(s: &str) -> String {
    hex::encode(&Sha256::digest(s.as_bytes())[..4])
}

/// A participant's CA file: a regular file of at most 1 MiB, opened without blocking (a link is
/// followed: the file is theirs).
fn read_ca(path: &Path) -> Option<Vec<u8>> {
    let file = (std::fs::OpenOptions::new().read(true))
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    (meta.is_file() && meta.len() <= 1 << 20).then_some(())?;
    let mut pem = Vec::new();
    file.take(1 << 20).read_to_end(&mut pem).ok()?;
    Some(pem)
}

/// The values at a dotted key; a segment may be or hold a `*` glob (any key or index, `*_URL`).
fn at<'a>(doc: &'a Value, key: &str) -> Vec<&'a Value> {
    at_paths(doc, key).into_iter().map(|(_, v)| v).collect()
}

/// `at`, each value with its dotted path (a list index as its number).
fn at_paths<'a>(doc: &'a Value, key: &str) -> Vec<(String, &'a Value)> {
    let mut found = vec![(String::new(), doc)];
    for seg in key.split('.') {
        let glob = pattern::parse(seg).ok();
        // The glob matches one path component, so a `/` in a key is not a separator to it.
        let one = |k: &str| PathBuf::from(k.replace('/', "\u{2215}"));
        let fits = |k: &str| k == seg || glob.as_ref().is_some_and(|g| g.matches(&one(k)));
        let mut next = Vec::new();
        for (path, value) in found {
            let join = |k: &str| match path.is_empty() {
                true => k.to_string(),
                false => format!("{path}.{k}"),
            };
            match value {
                Value::Object(map) => {
                    next.extend(map.iter().filter(|e| fits(e.0)).map(|e| (join(e.0), e.1)))
                }
                Value::Array(items) => {
                    let items = items.iter().enumerate().map(|(i, v)| (i.to_string(), v));
                    next.extend(items.filter(|e| fits(&e.0)).map(|e| (join(&e.0), e.1)));
                }
                _ => {}
            }
        }
        found = next;
    }
    found
}

/// JSON with `//` and `/* */` comments and trailing commas removed outside strings.
pub(super) fn strip_jsonc(text: &str) -> String {
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

/// Block mappings and sequences nested by indentation (a `- ` item at its key's indent or deeper),
/// scalars as strings; an empty flow `[]` or `{}` is an empty list or map, any other flow
/// collection stays text, and a block scalar (`|`, `>`) is left out.
/// The small block lookup of PRD-17 N7.
fn yaml(text: &str) -> Value {
    let mut root = Value::Object(Map::new());
    let (mut open, mut block) = (Vec::<(usize, String)>::new(), usize::MAX);
    for line in text.lines() {
        let mut line = line.to_string();
        let body = line.trim_start_matches(' ');
        let indent = line.len() - body.len();
        if body.is_empty() || body.starts_with('#') || indent > block {
            continue;
        }
        block = usize::MAX;
        if body == "-" || body.starts_with("- ") {
            // A new element of the parent's list; its own `key: rest`, if any, follows below.
            let rest = &body[1..];
            let scalar = yaml_key(rest).map_or_else(|| yaml_scalar(rest.trim()), |_| None);
            open.retain(|(i, _)| *i <= indent);
            let parent = open.last().map_or(String::new(), |(_, p)| p.clone());
            let slot = root.pointer_mut(&parent).map(|v| filled(v, json!([])));
            let Some(Value::Array(list)) = slot else {
                continue;
            };
            open.push((indent + 1, format!("{parent}/{}", list.len())));
            list.push(scalar.unwrap_or(Value::Null));
            line.replace_range(indent..=indent, " ");
        }
        let Some((indent, key, rest)) = yaml_key(&line) else {
            continue;
        };
        open.retain(|(i, _)| *i < indent);
        let parent = open.last().map_or(String::new(), |(_, p)| p.clone());
        let slot = root.pointer_mut(&parent).map(|v| filled(v, json!({})));
        let Some(Value::Object(map)) = slot else {
            continue;
        };
        let rest = rest.trim();
        if rest.is_empty() || rest.starts_with('#') {
            // RFC 6901: a `/` or `~` in a key is escaped, so its children are still read.
            let token = key.replace('~', "~0").replace('/', "~1");
            open.push((indent, format!("{parent}/{token}")));
            map.insert(key, Value::Null);
        } else if rest.starts_with(['|', '>']) {
            block = indent;
        } else if let Some(scalar) = yaml_scalar(rest) {
            map.insert(key, scalar);
        }
    }
    root
}

/// `v`, set to `empty` first while it is null: a key whose children have not been read yet.
fn filled(v: &mut Value, empty: Value) -> &mut Value {
    if v.is_null() {
        *v = empty;
    }
    v
}

/// The indent, key and rest of a `key: rest` line; `None` for a comment, a sequence item or a
/// line with no key.
pub(super) fn yaml_key(line: &str) -> Option<(usize, String, &str)> {
    let body = line.trim_start_matches(' ');
    let b = body.as_bytes();
    let colon = (0..b.len()).find(|&i| b[i] == b':' && b.get(i + 1).is_none_or(|&n| n == b' '))?;
    let key = body[..colon].trim_end().trim_matches(['"', '\'']);
    let plain = !key.is_empty() && !key.starts_with(['-', '?', '[', '{', '#', '\t']);
    plain.then(|| (line.len() - body.len(), key.to_string(), &body[colon + 1..]))
}

/// A plain or quoted scalar on one line, a plain `[]` or `{}` as an empty list or map; `None`
/// for anything else.
fn yaml_scalar(s: &str) -> Option<Value> {
    if let Some(quote) = s.chars().next().filter(|c| *c == '"' || *c == '\'') {
        return s[1..]
            .split_once(quote)
            .map(|(inner, _)| Value::from(inner));
    }
    let plain = s.find(" #").map_or(s, |at| s[..at].trim_end());
    match plain {
        "[]" => Some(json!([])),
        "{}" => Some(json!({})),
        _ if plain.is_empty() || plain == "~" || plain.eq_ignore_ascii_case("null") => None,
        _ => Some(Value::from(plain)),
    }
}
