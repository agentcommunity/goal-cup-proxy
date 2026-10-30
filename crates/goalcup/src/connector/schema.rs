//! The connector schema (PRD-11 section 2) and the loader's checks, which are that section's
//! rules and nothing else.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ConnectorError, ConnectorSource, Form, HostList, launch};

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
                    && prefix
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_lowercase)
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

/// Parses and checks one connector file. `file` names it in errors.
pub fn parse(bytes: &[u8], file: &str) -> Result<Connector, ConnectorError> {
    let mut connector: Connector = from_toml(bytes, file)?;
    check(&connector, file)?;
    connector.sha256 = hex::encode(Sha256::digest(bytes));
    Ok(connector)
}

/// TOML bytes into `T`; an error names the line and never quotes a value.
pub(crate) fn from_toml<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    file: &str,
) -> Result<T, ConnectorError> {
    let parse_error = |line, message| ConnectorError::Parse {
        file: file.to_string(),
        line,
        message,
    };
    let text = std::str::from_utf8(bytes).map_err(|_| parse_error(0, "not UTF-8".into()))?;
    toml::from_str(text).map_err(|e| {
        let line = e
            .span()
            .and_then(|s| text.get(..s.start))
            .map_or(0, |before| before.matches('\n').count() + 1);
        parse_error(line, safe_message(e.message()))
    })
}

/// The parser's message without any value it quoted: serde writes a string value in double
/// quotes, and other values after "invalid type", "invalid value" or "unknown variant".
fn safe_message(message: &str) -> String {
    let first = message.lines().next().unwrap_or("");
    for what in ["invalid type", "invalid value", "invalid length", "unknown variant"] {
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

/// Whether the code owns `name` (PRD-11 section 3): it removes and sets the proxy and CA
/// variables, sets `NODE_USE_ENV_PROXY`, never sets the variables that turn TLS checks off, and
/// leaves `HOME` to `home`. No `env` or `ca_env` entry names one; a `home` key names only `HOME`.
fn code_owned(name: &str) -> bool {
    launch::REMOVED.contains(&name)
        || [
            "NODE_USE_ENV_PROXY",
            "HOME",
            "SSL_CERT_DIR",
            "CURL_CA_BUNDLE",
            "NODE_TLS_REJECT_UNAUTHORIZED",
        ]
        .contains(&name)
}

/// Variable name parts that point a harness at a model address.
const MODEL_ADDRESS: &[&str] = &["BASE_URL", "API_BASE", "ENDPOINT", "_HOST", "PROXY"];

/// The loader rules of PRD-11 section 2. `file` names the connector in errors.
fn check(c: &Connector, file: &str) -> Result<(), ConnectorError> {
    let invalid = |key: String, reason| ConnectorError::Invalid {
        file: file.to_string(),
        key,
        reason,
    };
    let address = |key| ConnectorError::ModelAddress {
        file: file.to_string(),
        key,
    };
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
            return Err(address(format!("env.{name}")));
        }
    }
    for (list, rows) in [("no_prompts", &c.no_prompts), ("update_off", &c.update_off)] {
        for (i, row) in rows.iter().enumerate() {
            if row.value.contains("://") {
                return Err(address(format!("{list}[{i}].value")));
            }
        }
    }

    // Variables the code sets stay the code's.
    for name in c.env.keys() {
        if code_owned(name)
            || c.home.contains_key(name)
            || c.ca_env.contains(name)
            || c.unset.contains(name)
        {
            return Err(invalid(
                format!("env.{name}"),
                "the code sets this variable; use home, ca_env or unset",
            ));
        }
    }
    for (i, name) in c.ca_env.iter().enumerate() {
        if code_owned(name) || c.home.contains_key(name) {
            return Err(invalid(
                format!("ca_env[{i}]"),
                "the code sets this variable; ca_env names the harness's own CA variable",
            ));
        }
    }
    for name in c.home.keys() {
        if name != "HOME" && code_owned(name) {
            return Err(invalid(
                format!("home.{name}"),
                "the code sets this variable; home names the harness's own folder variables",
            ));
        }
    }

    // Every path stays under the isolated home or the workspace.
    for (name, value) in &c.home {
        if under(value, &["{home}"]).is_none() {
            return Err(invalid(
                format!("home.{name}"),
                "must start with {home}, with no `.`, `..` or empty component",
            ));
        }
    }
    for (list, args) in [("args", &c.args), ("resume", &c.resume)] {
        for (i, arg) in args.iter().enumerate() {
            if !templates_ok(arg) {
                return Err(invalid(
                    format!("{list}[{i}]"),
                    "the only template here is {workspace}",
                ));
            }
        }
    }
    if let Some(dir) = &c.skills_dir
        && under(dir, &["{home}"]).is_none()
    {
        return Err(invalid(
            "skills_dir".into(),
            "must be under {home}, no `.`, `..` or empty component; nothing goes in the workspace",
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
    for (key, path) in files {
        let star_early = under(path, &["{home}", "{workspace}"]).map(|rest| {
            let parts: Vec<&str> = rest.split('/').collect();
            parts[..parts.len() - 1].iter().any(|p| p.contains('*'))
        });
        if star_early != Some(false) {
            return Err(invalid(
                key,
                "must start with {home} or {workspace}, with no `.`, `..` or empty component, \
                 no other template, and a `*` only in the last component",
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
        if under(&row.root, &["{home}", "{workspace}"]).is_none() {
            return Err(invalid(
                format!("native[{i}].root"),
                "must start with {home} or {workspace}, with no `.`, `..` or empty component",
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

/// The path after a leading root template from `roots`, when it stays under that root: each
/// component after it is not empty, `.` or `..`, and holds no brace or NUL.
fn under<'a>(path: &'a str, roots: &[&str]) -> Option<&'a str> {
    let rest = roots.iter().find_map(|root| path.strip_prefix(root))?;
    let clean = |part: &str| {
        !part.is_empty() && part != "." && part != ".." && !part.contains(['{', '}', '\0'])
    };
    let stays = rest.is_empty() || rest.strip_prefix('/').is_some_and(|r| r.split('/').all(clean));
    stays.then_some(rest)
}

/// Every brace in `s` is a `{workspace}` template.
fn templates_ok(s: &str) -> bool {
    !s.replace("{workspace}", "").contains(['{', '}'])
}

/// PRD-04 section 2: relative, no `..`, `.`, empty component, braces or NUL; `**` only whole.
fn pattern_ok(pattern: &str) -> bool {
    !pattern.contains(['{', '}', '\0'])
        && pattern.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && (part == "**" || !part.contains("**"))
        })
}
