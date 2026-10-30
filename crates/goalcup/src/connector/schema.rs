//! The connector schema (PRD-11 section 2) and the loader's checks, which are that section's
//! rules and nothing else.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ConnectorError, ConnectorSource, Form, HostList};

/// One connector file. PRD-10 reads its participant-facing fields directly. `id`, `name` and
/// `program` are required; every other key is optional.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Connector {
    pub id: String,
    pub name: String,
    pub program: String,
    /// A prefix `program --version` must contain.
    pub version: Option<String>,
    pub args: Vec<String>,
    pub resume: Vec<String>,
    /// Variables pointed into the isolated home. Template `{home}`.
    pub home: BTreeMap<String, String>,
    pub env: BTreeMap<String, String>,
    pub ca_env: Vec<String>,
    pub unset: Vec<String>,
    pub refuse_if_set: Vec<RefuseRow>,
    pub host_hints: Vec<HintRow>,
    pub hint_env: Vec<String>,
    pub secret_files: Vec<String>,
    /// Arguments without the program; the watchdog runs `program` followed by them.
    pub before: Vec<Vec<String>>,
    /// Arguments without the program, as in `before`.
    pub after: Vec<Vec<String>>,
    pub native: Vec<NativeRow>,
    pub mask: Vec<MaskRule>,
    pub first_screen: Option<String>,
    pub no_prompts: Vec<SettingRow>,
    pub update_off: Vec<SettingRow>,
    pub skills_dir: Option<String>,

    /// Built in or a file. Set by `load`.
    #[serde(skip)]
    pub source: ConnectorSource,
    /// SHA-256 of the exact TOML bytes, 64 hex.
    #[serde(skip)]
    pub sha256: String,
    /// The shared host list in force (the built-in list or a `--hosts` file).
    #[serde(skip)]
    pub hosts: HostList,
}

/// Before launch, a non-empty value at any of `keys` in `file` refuses the start.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefuseRow {
    pub file: String,
    pub keys: Vec<String>,
}

/// A config file key whose URL value adds a host to the `config` list.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HintRow {
    pub file: String,
    pub key: String,
}

/// A setting written into the isolated home (`no_prompts`, `update_off`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingRow {
    pub file: String,
    pub key: String,
    pub value: String,
}

/// One session-file row as written in the connector.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRow {
    pub name: String,
    pub root: String,
    pub pattern: String,
    #[serde(default)]
    pub form: Form,
    #[serde(default = "yes")]
    pub warn_missing: bool,
}

fn yes() -> bool {
    true
}

/// A connector mask rule in PRD-05's grammar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawMaskRule", into = "RawMaskRule")]
pub struct MaskRule {
    /// `^[a-z][a-z0-9_]{0,47}$`: what every marker of this rule names.
    pub kind: String,
    pub pattern: MaskPattern,
}

/// The two rule forms. `object` values are refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaskPattern {
    /// `"field": value`, field `[A-Za-z0-9_-]{1,64}`.
    JsonField { field: String, value: FieldValue },
    /// `prefix[a-z0-9-]+: value`, prefix `[a-z][a-z0-9-]{0,63}`.
    HeaderPrefix(String),
}

/// The value half of a JSON field rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldValue {
    /// A non-empty string without escapes.
    String,
    /// A string without escapes, else a bare token (a number).
    Scalar,
}

/// A mask row as written in TOML or JSON, before it is checked.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMaskRule {
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    json_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<FieldValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    header_prefix: Option<String>,
}

impl TryFrom<RawMaskRule> for MaskRule {
    type Error = &'static str;

    fn try_from(raw: RawMaskRule) -> Result<MaskRule, &'static str> {
        let mut k = raw.kind.bytes();
        let kind_ok = raw.kind.len() <= 48
            && k.next().is_some_and(|b| b.is_ascii_lowercase())
            && k.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !kind_ok {
            return Err("mask: kind must match ^[a-z][a-z0-9_]{0,47}$");
        }
        let pattern = match (raw.json_field, raw.value, raw.header_prefix) {
            (Some(field), Some(value), None) => {
                let ok = (1..=64).contains(&field.len())
                    && field
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
                if !ok {
                    return Err("mask: json_field must match ^[A-Za-z0-9_-]{1,64}$");
                }
                MaskPattern::JsonField { field, value }
            }
            (None, None, Some(prefix)) => {
                let ok = prefix.len() <= 64
                    && prefix.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                    && prefix
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
                if !ok {
                    return Err("mask: header_prefix must match ^[a-z][a-z0-9-]{0,63}$");
                }
                MaskPattern::HeaderPrefix(prefix)
            }
            _ => {
                return Err(
                    "mask: a rule has json_field with value (string or scalar), or header_prefix",
                );
            }
        };
        Ok(MaskRule {
            kind: raw.kind,
            pattern,
        })
    }
}

impl From<MaskRule> for RawMaskRule {
    fn from(rule: MaskRule) -> RawMaskRule {
        let (json_field, value, header_prefix) = match rule.pattern {
            MaskPattern::JsonField { field, value } => (Some(field), Some(value), None),
            MaskPattern::HeaderPrefix(prefix) => (None, None, Some(prefix)),
        };
        RawMaskRule {
            kind: rule.kind,
            json_field,
            value,
            header_prefix,
        }
    }
}

/// The `id` key of a connector file, without the other checks.
pub(crate) fn id_of(text: &str) -> Option<String> {
    let table: toml::Table = toml::from_str(text).ok()?;
    table.get("id")?.as_str().map(str::to_string)
}

/// Parses and checks one connector file. `file` names it in errors.
pub fn parse(bytes: &[u8], file: &str) -> Result<Connector, ConnectorError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ConnectorError::Parse {
        file: file.to_string(),
        line: 0,
        message: "not UTF-8".to_string(),
    })?;
    let mut connector: Connector = from_toml(text, file)?;
    check(&connector).map_err(|e| e.in_file(file))?;
    connector.sha256 = hex::encode(Sha256::digest(bytes));
    connector.source = ConnectorSource::Builtin;
    Ok(connector)
}

/// TOML into `T`; an error names the line and never quotes a value.
pub(crate) fn from_toml<T: serde::de::DeserializeOwned>(
    text: &str,
    file: &str,
) -> Result<T, ConnectorError> {
    toml::from_str(text).map_err(|e| {
        let line = e
            .span()
            .and_then(|s| text.get(..s.start))
            .map_or(0, |before| before.matches('\n').count() + 1);
        ConnectorError::Parse {
            file: file.to_string(),
            line,
            message: safe_message(e.message()),
        }
    })
}

/// The parser's message without any value it quoted: serde writes a string value in double
/// quotes and other values after "invalid type" or "invalid value".
fn safe_message(message: &str) -> String {
    let first = message.lines().next().unwrap_or("");
    for what in ["invalid type", "invalid value", "invalid length"] {
        if first.starts_with(what) {
            return match first.find(", expected") {
                Some(i) => format!("{what}{}", &first[i..]),
                None => what.to_string(),
            };
        }
    }
    match first.find('"') {
        Some(i) => first[..i].trim_end().to_string(),
        None => first.to_string(),
    }
}

fn invalid(key: String, reason: &'static str) -> ConnectorError {
    ConnectorError::Invalid {
        file: String::new(),
        key,
        reason,
    }
}

/// Variables the code sets itself (PRD-11 section 3); they never go in `env`.
const CODE_SET: &[&str] = &[
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
    "REQUESTS_CA_BUNDLE",
    "NODE_USE_ENV_PROXY",
    "HOME",
];

/// Variable name parts that point a harness at a model address.
const MODEL_ADDRESS: &[&str] = &["BASE_URL", "API_BASE", "ENDPOINT", "_HOST", "PROXY"];

/// The loader rules of PRD-11 section 2.
fn check(c: &Connector) -> Result<(), ConnectorError> {
    let id_ok = (1..=32).contains(&c.id.len())
        && c.id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !id_ok {
        return Err(invalid("id".into(), "must match [a-z0-9-]{1,32}"));
    }
    if c.name.is_empty() || c.program.is_empty() {
        return Err(invalid("name, program".into(), "both are required"));
    }

    // No model address in connector data.
    for (name, value) in &c.env {
        let upper = name.to_ascii_uppercase();
        if MODEL_ADDRESS.iter().any(|p| upper.contains(p)) || value.contains("://") {
            return Err(ConnectorError::ModelAddress {
                file: String::new(),
                key: format!("env.{name}"),
            });
        }
    }
    for (list, rows) in [("no_prompts", &c.no_prompts), ("update_off", &c.update_off)] {
        for (i, row) in rows.iter().enumerate() {
            if row.value.contains("://") {
                return Err(ConnectorError::ModelAddress {
                    file: String::new(),
                    key: format!("{list}[{i}].value"),
                });
            }
        }
    }

    for name in c.env.keys() {
        let taken = CODE_SET.contains(&name.as_str())
            || c.home.contains_key(name)
            || c.ca_env.contains(name)
            || c.unset.contains(name);
        if taken {
            return Err(invalid(
                format!("env.{name}"),
                "the code sets this variable; use home, ca_env or unset",
            ));
        }
    }

    for (name, value) in &c.home {
        if !value.starts_with("{home}") || !templates_ok(value, &["home"]) {
            return Err(invalid(format!("home.{name}"), "must start with {home}"));
        }
    }
    for (list, args) in [("args", &c.args), ("resume", &c.resume)] {
        for (i, arg) in args.iter().enumerate() {
            if !templates_ok(arg, &["workspace"]) {
                return Err(invalid(
                    format!("{list}[{i}]"),
                    "the only template here is {workspace}",
                ));
            }
        }
    }
    if let Some(dir) = &c.skills_dir
        && (!dir.starts_with("{home}") || !templates_ok(dir, &["home"]))
    {
        return Err(invalid(
            "skills_dir".into(),
            "must be under {home}; nothing is added to the workspace",
        ));
    }

    let mut files: Vec<(String, &str)> = Vec::new();
    for (i, row) in c.refuse_if_set.iter().enumerate() {
        files.push((format!("refuse_if_set[{i}].file"), &row.file));
    }
    for (i, row) in c.host_hints.iter().enumerate() {
        files.push((format!("host_hints[{i}].file"), &row.file));
    }
    for (i, file) in c.secret_files.iter().enumerate() {
        files.push((format!("secret_files[{i}]"), file));
    }
    for (list, rows) in [("no_prompts", &c.no_prompts), ("update_off", &c.update_off)] {
        for (i, row) in rows.iter().enumerate() {
            files.push((format!("{list}[{i}].file"), &row.file));
        }
    }
    for (key, file) in files {
        let Some(rest) = rooted(file) else {
            return Err(invalid(key, "must start with {home} or {workspace}"));
        };
        let parts: Vec<&str> = rest.split('/').collect();
        let star_early = parts[..parts.len() - 1].iter().any(|p| p.contains('*'));
        if star_early || rest.contains(['{', '}']) {
            return Err(invalid(
                key,
                "a `*` only in the last component, and no other template",
            ));
        }
    }

    let mut names = BTreeSet::new();
    for (i, row) in c.native.iter().enumerate() {
        if !names.insert(row.name.as_str()) {
            return Err(invalid(
                format!("native[{i}].name"),
                "native row names are unique within a connector",
            ));
        }
        if rooted(&row.root).is_none_or(|rest| rest.contains(['{', '}'])) {
            return Err(invalid(
                format!("native[{i}].root"),
                "must start with {home} or {workspace}",
            ));
        }
        if !pattern_ok(&row.pattern) {
            return Err(invalid(
                format!("native[{i}].pattern"),
                "relative, no `..`, `.`, empty component, braces or NUL, `**` a whole component",
            ));
        }
    }
    Ok(())
}

/// The path after a leading `{home}` or `{workspace}`, when it has one.
fn rooted(path: &str) -> Option<&str> {
    path.strip_prefix("{home}")
        .or_else(|| path.strip_prefix("{workspace}"))
        .filter(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Every brace in `s` is one of the `allowed` templates.
fn templates_ok(s: &str, allowed: &[&str]) -> bool {
    let mut rest = s;
    while let Some(i) = rest.find(['{', '}']) {
        let after = &rest[i..];
        let Some(name) = allowed
            .iter()
            .find(|name| after.starts_with(&format!("{{{name}}}")))
        else {
            return false;
        };
        rest = &after[name.len() + 2..];
    }
    true
}

/// PRD-04 section 2: relative, no `..`, `.`, empty component, braces or NUL; `**` only whole.
fn pattern_ok(pattern: &str) -> bool {
    !pattern.contains(['{', '}', '\0'])
        && pattern.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && (part == "**" || !part.contains("**"))
        })
}
