//! Parsing `harness.toml`: the schema dispatch, the refusals read from the generic table
//! (retired pin keys, the credential table, the removed headless mode) and the strict
//! deserialization.

use super::*;
use crate::ProtocolError;

// ---------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------

/// One refusal of `harness.toml`.
pub(super) fn refuse(field: impl Into<String>, reason: impl Into<String>) -> ProtocolError {
    ProtocolError::Field {
        field: field.into(),
        reason: reason.into(),
    }
}

/// The table header in force at byte `at` of `text`, for the message only.
fn table_path_at(text: &str, at: usize) -> String {
    let before = &text[..at.min(text.len())];
    for line in before.lines().rev() {
        let t = line.trim();
        if let Some(inner) = t.strip_prefix("[[").and_then(|r| r.split("]]").next()) {
            return format!("[[{}]]", inner.trim());
        }
        if let Some(inner) = t.strip_prefix('[').and_then(|r| r.split(']').next()) {
            return format!("[{}]", inner.trim());
        }
    }
    "the top level".to_string()
}

/// Turn a `toml` error into a refusal; an unknown key becomes the schema-1 sentence.
fn map_toml_error(text: &str, e: &toml::de::Error) -> ProtocolError {
    let msg = e.message();
    let (place, line) = match e.span() {
        Some(span) => (
            table_path_at(text, span.start),
            text[..span.start.min(text.len())].matches('\n').count() + 1,
        ),
        None => ("the file".to_string(), 0),
    };
    let near = if line > 0 {
        format!("{place}, near line {line}")
    } else {
        place
    };
    if let Some(rest) = msg.strip_prefix("unknown field `") {
        let key = rest.split('`').next().unwrap_or("");
        return refuse(
            "harness.toml",
            format!("`{key}` (in {near}) is {NEWER_SENTENCE}"),
        );
    }
    refuse("harness.toml", format!("{msg} (in {near})"))
}

impl HarnessManifest {
    /// Parse `harness.toml`: the schema dispatch (rule 1) and the strict deserialization
    /// (rule 2), then the rows of `capture.providers` appended to `capture.hosts`. Does not
    /// validate; call [`Self::validate`].
    pub fn from_toml_str(text: &str) -> Result<HarnessManifest, ProtocolError> {
        let table: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| map_toml_error(text, &e))?;
        match table.get("schema_version") {
            None => {
                return Err(refuse(
                    "schema_version",
                    "`connectors/<id>/harness.toml` has no `schema_version`. A harness manifest \
                     starts with `schema_version = 1`; see docs/CONTRACTS.md#harness-contract-v1.",
                ));
            }
            Some(toml::Value::Integer(n))
                if u32::try_from(*n).is_ok_and(|v| SUPPORTED_SCHEMA_VERSIONS.contains(&v)) => {}
            Some(other) => {
                return Err(refuse(
                    "schema_version",
                    format!(
                        "This build reads connector schema 1; the file declares {other}. \
                         {MIGRATION_SENTENCE}"
                    ),
                ));
            }
        }
        refuse_retired_pin_keys(&table)?;
        refuse_credential_table(&table)?;
        refuse_headless(&table)?;
        let mut manifest =
            toml::from_str::<HarnessManifest>(text).map_err(|e| map_toml_error(text, &e))?;
        manifest.capture.add_provider_hosts()?;
        Ok(manifest)
    }
}

/// The keys the executable pin used to need, refused by name (D53): goalcup records the
/// build that runs, so a manifest that still pins one is told to drop the line, not that it is
/// from a newer schema.
const RETIRED_PIN_KEYS: &[&str] = &["sha256", "version"];

/// Refuse `capture.executable.{sha256,version}` and `capture.platform.<key>.{sha256,version}`
/// with one sentence each, read from the generic table before the strict deserialization.
fn refuse_retired_pin_keys(table: &toml::Table) -> Result<(), ProtocolError> {
    let Some(capture) = table.get("capture").and_then(toml::Value::as_table) else {
        return Ok(());
    };
    let mut places: Vec<(String, &toml::Table)> = Vec::new();
    if let Some(executable) = capture.get("executable").and_then(toml::Value::as_table) {
        places.push(("capture.executable".to_string(), executable));
    }
    if let Some(platforms) = capture.get("platform").and_then(toml::Value::as_table) {
        for (key, value) in platforms {
            if let Some(over) = value.as_table() {
                places.push((format!("capture.platform.{key}"), over));
            }
        }
    }
    for (at, keys) in places {
        for retired in RETIRED_PIN_KEYS {
            if keys.contains_key(*retired) {
                return Err(refuse(
                    format!("{at}.{retired}"),
                    format!(
                        "`{retired}` is no longer a connector field (D53): Goal Cup Proxy records \
                         the build that runs instead of pinning one; delete the line."
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// What a `[capture.credential]` table is told (D61). The table once named a provider key the
/// launcher read from its own environment and handed to the harness; goalcup no longer
/// sees, forwards or stores one, so the table is refused by name rather than as a newer
/// schema's field.
pub const CREDENTIAL_REFUSAL: &str =
    "a connector may not pass a provider key; the harness keeps its own login in its home";

/// Refuse `capture.credential` and `capture.platform.<key>.credential` (D61), read from the
/// generic table before the strict deserialization.
fn refuse_credential_table(table: &toml::Table) -> Result<(), ProtocolError> {
    let Some(capture) = table.get("capture").and_then(toml::Value::as_table) else {
        return Ok(());
    };
    if capture.contains_key("credential") {
        return Err(refuse("capture.credential", CREDENTIAL_REFUSAL));
    }
    if let Some(platforms) = capture.get("platform").and_then(toml::Value::as_table) {
        for (key, value) in platforms {
            if value
                .as_table()
                .is_some_and(|t| t.contains_key("credential"))
            {
                return Err(refuse(
                    format!("capture.platform.{key}.credential"),
                    CREDENTIAL_REFUSAL,
                ));
            }
        }
    }
    Ok(())
}

/// The `capture.identity.source` values of the removed headless mode.
const HEADLESS_IDENTITY_SOURCES: &[&str] = &["preassigned", "stdout_json", "stderr_line"];

/// The `[capture.identity]` keys of the removed headless sources.
const HEADLESS_IDENTITY_KEYS: &[&str] = &[
    "binding",
    "match_field",
    "match_value",
    "id_field",
    "prefix",
    "grammar",
    "wait_seconds",
];

/// Refuse a manifest written for the removed headless mode ([`CHAT_MODE_ONLY`]), read from the
/// generic table before the strict deserialization would call its fields a newer schema's:
/// `capture.interactive` absent or not `true`, a `capture.goal_flag`, a headless identity source,
/// or an identity field beyond `source`.
fn refuse_headless(table: &toml::Table) -> Result<(), ProtocolError> {
    let Some(capture) = table.get("capture").and_then(toml::Value::as_table) else {
        return Ok(());
    };
    if capture.get("interactive") != Some(&toml::Value::Boolean(true)) {
        return Err(refuse("capture.interactive", CHAT_MODE_ONLY));
    }
    if capture.contains_key("goal_flag") {
        return Err(refuse("capture.goal_flag", CHAT_MODE_ONLY));
    }
    let Some(identity) = capture.get("identity").and_then(toml::Value::as_table) else {
        return Ok(());
    };
    if let Some(source) = identity.get("source").and_then(toml::Value::as_str)
        && HEADLESS_IDENTITY_SOURCES.contains(&source)
    {
        return Err(refuse("capture.identity.source", CHAT_MODE_ONLY));
    }
    if let Some(key) = HEADLESS_IDENTITY_KEYS
        .iter()
        .find(|key| identity.contains_key(**key))
    {
        return Err(refuse(format!("capture.identity.{key}"), CHAT_MODE_ONLY));
    }
    Ok(())
}
