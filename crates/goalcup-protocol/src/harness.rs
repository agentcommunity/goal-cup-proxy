//! The harness manifest, `connectors/<id>/harness.toml`, schema 1 (harness contract v1, S2;
//! `docs/DECISIONS.md` D42, D43).
//!
//! A harness is data: this file is its whole capture recipe, and its `[capture]` table is the
//! **profile** ([`CaptureSpec`]). The module defines the types, the loader that reads one
//! connector directory, and [`HarnessManifest::validate`], which refuses every manifest the
//! refusal table names. The trust tables it checks against are in [`crate::capture_policy`]; a
//! manifest can only be checked against them, never widen them.
//!
//! "Schema 1" is this file's format; the V1 manifest was the legacy `manifest.toml`, whose V1
//! path D48 removed and whose last shipped copies OS-29 deleted. The loader refuses one. The
//! CLI and the daemon load a connector through [`load_dir`], the one loader all three callers
//! share.
//!
//! # Parsing
//!
//! 1. The text is parsed as a generic TOML table and `schema_version` is read first
//!    ([`SUPPORTED_SCHEMA_VERSIONS`]).
//! 2. It is then deserialized with `deny_unknown_fields` everywhere. An unknown key, and an
//!    unknown value of a closed-set field, are refused with one sentence ([`NEWER_SENTENCE`]):
//!    serde's own "unknown variant" text never reaches the user.
//! 3. [`HarnessManifest::validate`] runs the checks in a fixed order, so the first refusal
//!    is deterministic: executable, args and goal, env, home, credential, CA, identity,
//!    native, argv guard, termination, hosts, redaction, the delivery-agnostic rule (F7), and
//!    the cross-field rules.
//!
//! [`Origin`] is the one observable predicate that relaxes anything: `Dev` (an unpinned
//! directory under `GOALCUP_CONNECTORS_DIR` loaded with `--dev`, for kit cases and the
//! controlled workload) skips the interpreter refusal and the absolute-path `args` refusal.
//! Env values, seeds, the deny lists and every cross-field rule stay strict for both.

use crate::ProtocolError;
use crate::capture_policy::{
    CREDENTIAL_ENV, CaMode, EXTRA_PATTERN_SOURCES, EnvContext, MAX_CAPTURE_ARG_BYTES,
    MAX_CAPTURE_ARGS, MAX_CAPTURE_ENV, MAX_CAPTURE_ENV_KEY_BYTES, MAX_CAPTURE_ENV_VALUE_BYTES,
    MAX_CAPTURE_HOSTS, MAX_CAPTURE_PATHS_PER_HOST, MAX_CONTINUATION_BYTES, MAX_GOAL_FLAG_BYTES,
    arg_path_denied, ca_registry_mode, capture_arg_denied, capture_path_problem, env_key_denied,
    env_value_denied, has_duplicate, is_capture_host, is_interpreter_program, is_tracker_host,
    mentions_goalcup, startup_file_problem,
};
use crate::connector_dir::hash_connector_dir_listing;
use crate::hints::valid_hint;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The schema versions this build reads. Reader policy: these only.
pub(crate) const SUPPORTED_SCHEMA_VERSIONS: &[u32] = &[1];

/// The tail of every "this is not schema 1" refusal, for keys and for values alike.
pub const NEWER_SENTENCE: &str = "not part of connector schema 1: a field from a newer schema needs a newer goalcup, and a misspelling is refused rather than ignored.";

/// The tail of the `schema_version` refusals.
const MIGRATION_SENTENCE: &str = "A newer schema needs a newer goalcup; an older one is migrated by the PR that retired it (docs/CONTRACTS.md#migrating-from-manifesttoml).";

/// The most a seed file may weigh, in bytes.
pub const MAX_SEED_BYTES: u64 = 64 * 1024;

// ---------------------------------------------------------------------------------------
// Closed-set values
// ---------------------------------------------------------------------------------------

/// The refusal for a value outside a closed set: names the value and the field and says what
/// schema 1 accepts. `special` is a more specific message for one named value.
fn closed_value_message(
    field: &str,
    value: &str,
    accepted: &[&str],
    special: &[(&str, &str)],
) -> String {
    if let Some((_, why)) = special.iter().find(|(v, _)| *v == value) {
        return (*why).to_string();
    }
    format!(
        "value `{value}` of `{field}` is {NEWER_SENTENCE} Schema 1 accepts: {}.",
        accepted
            .iter()
            .map(|a| format!("`{a}`"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// A closed-set enum whose `Deserialize` refuses an unknown value with [`NEWER_SENTENCE`].
macro_rules! closed_enum {
    (
        $(#[$meta:meta])*
        $name:ident, $field:literal, special: $special:expr;
        $( $(#[$vmeta:meta])* $variant:ident = $text:literal ),+ $(,)?
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[non_exhaustive]
        pub enum $name { $( $(#[$vmeta])* $variant ),+ }

        impl $name {
            /// The spellings schema 1 accepts, in declaration order.
            pub(crate) const VALUES: &'static [&'static str] = &[$($text),+];

            /// The TOML spelling.
            pub const fn as_str(self) -> &'static str {
                match self { $( Self::$variant => $text ),+ }
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                match s.as_str() {
                    $( $text => Ok(Self::$variant), )+
                    _ => Err(serde::de::Error::custom(closed_value_message(
                        $field, &s, Self::VALUES, $special,
                    ))),
                }
            }
        }
    };
}

closed_enum! {
    /// What `HOME` holds for a run.
    RunHome, "capture.home.run_home", special: &[];
    /// An empty directory (Codex: keeps `~/.agents/skills` and shell startup files out).
    Empty = "empty",
    /// A directory whose only entry is `Library/Keychains`, linked to the user's own (Claude
    /// Code: subscription login, no dotfiles).
    KeychainLink = "keychain_link",
}

closed_enum! {
    /// Which isolated harness home `home.env` points at.
    ConfigHome, "capture.home.config_home", special: &[];
    /// One isolated home shared by every run of this harness; runs against it are serialized.
    Shared = "shared",
    /// A home minted fresh inside the run directory, holding only the manifest's seeds.
    PerRun = "per_run",
}

closed_enum! {
    /// The mode a seed file is written with.
    SeedMode, "capture.home.seed.mode", special: &[];
    /// Owner read and write (the default).
    Rw0600 = "0600",
    /// Owner read only.
    Ro0400 = "0400",
}

/// `capture.ca.mode` is the policy type itself, so a manifest's declaration and a
/// [`CA_REGISTRY`](crate::capture_policy::CA_REGISTRY) row are compared as one value. It refuses an
/// unknown value as `closed_enum!` does.
impl<'de> Deserialize<'de> for CaMode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        match s.as_str() {
            "extend" => Ok(CaMode::Extend),
            "replace" => Ok(CaMode::Replace),
            _ => Err(serde::de::Error::custom(closed_value_message(
                "capture.ca.mode",
                &s,
                CaMode::VALUES,
                &[],
            ))),
        }
    }
}

closed_enum! {
    /// Where the run's session identity comes from.
    IdentitySource, "capture.identity.source", special: &[];
    /// The tracker mints it and passes it as `{session_id}`.
    Preassigned = "preassigned",
    /// A JSON event on the first stdout lines names it.
    StdoutJson = "stdout_json",
    /// A line on stderr carries it after a fixed prefix.
    StderrLine = "stderr_line",
    /// The harness has no session identity; its native store is per run.
    None = "none",
}

closed_enum! {
    /// When the identity is known.
    Binding, "capture.identity.binding", special: &[];
    AtLaunch = "at_launch",
    Deferred = "deferred",
}

closed_enum! {
    /// The grammar of an identity read from a stderr line.
    IdGrammar, "capture.identity.grammar", special: &[];
    /// `\d{8}_\d{6}_[0-9a-f]{6}`.
    TimestampHex = "timestamp-hex",
    /// 1..=128 of `[A-Za-z0-9_-]` (a lowercase UUID is a token).
    Token = "token",
}

closed_enum! {
    /// How the native store is read.
    NativeForm, "capture.native.form",
        special: &[("sqlite", "a SQLite store is a core capability, not schema 1 (D42): a new native form is a separate core PR")];
    /// Files that only grow.
    AppendOnly = "append_only",
    /// Files read whole once they settle.
    Snapshot = "snapshot",
}

closed_enum! {
    /// How the harness is stopped.
    Strategy, "capture.termination.strategy", special: &[];
    ProcessGroup = "process_group",
    NativePid = "native_pid",
}

// ---------------------------------------------------------------------------------------
// The manifest
// ---------------------------------------------------------------------------------------

/// Which directory a manifest was loaded from. The one predicate that relaxes a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Origin {
    /// An in-tree or pin-checked connector directory.
    Pinned,
    /// An unpinned directory under `GOALCUP_CONNECTORS_DIR`, loaded with `--dev` (kit cases,
    /// the controlled workload). Relaxes the interpreter refusal and the absolute-path `args`
    /// refusal, and nothing else.
    Dev,
}

/// `connectors/<id>/harness.toml`, schema 1.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HarnessManifest {
    pub schema_version: u32,
    pub id: String,
    pub capture: CaptureSpec,
    #[serde(default)]
    pub redaction: Option<RedactionSpec>,
}

/// `[capture]`: the **profile**, the whole recipe for capturing one harness.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CaptureSpec {
    /// The measured headless argument template: ends in `--` (the goal follows as one
    /// argument) unless [`Self::goal_flag`] names the slot.
    pub args: Vec<String>,
    /// The long option that carries the goal as one `--flag=<goal>` argument.
    #[serde(default)]
    pub goal_flag: Option<String>,
    /// Bounded, declarative provenance for how the run continues without input.
    pub continuation: String,
    /// Hosts that always stay opaque tunnels (sign-in and token refresh). Never intercepted.
    #[serde(default)]
    pub auth_hosts: Vec<String>,
    /// Process-scoped settings, keys literal and values templated.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub executable: ExecutableSpec,
    pub home: HomeSpec,
    #[serde(default)]
    pub credential: Option<CredentialSpec>,
    pub ca: CaSpec,
    pub identity: IdentitySpec,
    /// Exactly one entry in schema 1 (an array so that 1..=N is a later additive widening).
    #[serde(default)]
    pub native: Vec<NativeSpec>,
    #[serde(default)]
    pub argv_guard: Option<ArgvGuard>,
    #[serde(default)]
    pub termination: Option<TerminationSpec>,
    /// The hosts the proxy terminates TLS for.
    #[serde(default)]
    pub hosts: Vec<HostSpec>,
}

/// `[capture.executable]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ExecutableSpec {
    /// A bare name resolved on `PATH`.
    pub program: String,
    /// The version the recipe was measured at; recorded as `Launch.harness_version`.
    pub version: String,
    /// SHA-256 of the measured executable's bytes, lowercase hex.
    pub sha256: String,
}

/// `[capture.home]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HomeSpec {
    pub run_home: RunHome,
    pub config_home: ConfigHome,
    /// The variable pointed at the config home.
    pub env: String,
    /// The per-run config home's directory name; `per_run` only. Default `harness-home`.
    #[serde(default)]
    pub per_run_dir: Option<String>,
    /// Directories made 0700 under the per-run config home; `per_run` only.
    #[serde(default)]
    pub dirs: Vec<String>,
    /// Files written (`create_new`, `O_NOFOLLOW`) under the per-run config home; `per_run` only.
    #[serde(default)]
    pub seed: Vec<SeedFile>,
    /// `{user_home}/<rel>` directories the daemon refuses a config home at or under.
    pub real_dirs: Vec<String>,
}

impl HomeSpec {
    /// The per-run config home's directory name.
    pub fn per_run_dir_name(&self) -> &str {
        self.per_run_dir.as_deref().unwrap_or("harness-home")
    }
}

/// One seed file: `file` is relative under the connector's `files/` and is also its
/// destination under the config home.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SeedFile {
    pub file: String,
    #[serde(default)]
    pub mode: Option<SeedMode>,
}

impl SeedFile {
    /// The mode the file is written with.
    pub(crate) fn mode(&self) -> SeedMode {
        self.mode.unwrap_or(SeedMode::Rw0600)
    }
}

/// `[capture.credential]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CredentialSpec {
    /// A name on [`CREDENTIAL_ENV`].
    pub env: String,
}

/// `[capture.ca]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CaSpec {
    pub env: Vec<String>,
    pub mode: CaMode,
}

/// `[capture.identity]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct IdentitySpec {
    pub source: IdentitySource,
    /// Required unless `source = "none"`.
    #[serde(default)]
    pub binding: Option<Binding>,
    /// `stdout_json` only: the dot path whose value selects the identity event.
    #[serde(default)]
    pub match_field: Option<String>,
    /// `stdout_json` only: the value `match_field` must hold.
    #[serde(default)]
    pub match_value: Option<String>,
    /// `stdout_json` only: the dot path of the identity string.
    #[serde(default)]
    pub id_field: Option<String>,
    /// `stderr_line` only: the fixed prefix before the identity.
    #[serde(default)]
    pub prefix: Option<String>,
    /// `stderr_line` only: the identity's grammar.
    #[serde(default)]
    pub grammar: Option<IdGrammar>,
}

/// `[[capture.native]]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct NativeSpec {
    pub name: String,
    pub root: String,
    pub pattern: String,
    pub form: NativeForm,
    /// `append_only` only: the protocol hint, the same record field (`CaptureMeta.protocol_hint`)
    /// a `[[capture.hosts]]` entry names, with the same grammar ([`valid_hint`]).
    #[serde(default)]
    pub protocol_hint: Option<String>,
}

/// `[capture.argv_guard]`: the generic form of a harness that acts on some of its own argv
/// before its parser runs. It can only make the author's own recipe stricter.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ArgvGuard {
    /// Long option names the harness acts on, without `--`.
    #[serde(default)]
    pub long: Vec<String>,
    /// Short options acted on before the parser, one character each.
    #[serde(default)]
    pub short: Vec<String>,
    /// Short options that take a value, which ends a cluster scan.
    #[serde(default)]
    pub short_with_value: Vec<String>,
    /// `true` when the parser accepts any prefix of a long option (argparse without
    /// `allow_abbrev=False`).
    #[serde(default)]
    pub long_abbreviations: bool,
}

impl ArgvGuard {
    /// Why `arg` is refused by this guard, or `None`. An `--name[=value]` is refused when
    /// `name` is, or with `long_abbreviations` is a prefix of, a `long` entry (also when the
    /// prefix is ambiguous: a recipe that needs a parser to guess is not one to ship). A
    /// single-dash cluster is refused when it names a `short` option before a
    /// `short_with_value` option swallows the rest of the token.
    pub fn denies(&self, arg: &str) -> Option<&'static str> {
        if let Some(long) = arg.strip_prefix("--") {
            let name = long.split('=').next().unwrap_or("");
            if !name.is_empty()
                && self.long.iter().any(|acted| {
                    if self.long_abbreviations {
                        acted.starts_with(name)
                    } else {
                        acted == name
                    }
                })
            {
                return Some(
                    "is, or abbreviates, an option the harness acts on before its parser (a \
                     profile, a mode or help)",
                );
            }
            return None;
        }
        if let Some(cluster) = arg.strip_prefix('-') {
            for c in cluster.chars() {
                let one = c.to_string();
                if self.short.contains(&one) {
                    return Some(
                        "names a short option the harness acts on before its parser (a \
                         profile or help)",
                    );
                }
                if self.short_with_value.contains(&one) {
                    break;
                }
            }
        }
        None
    }
}

/// `[capture.termination]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TerminationSpec {
    #[serde(default)]
    pub strategy: Option<Strategy>,
    /// Required with `native_pid`, refused otherwise: a `comm` name.
    #[serde(default)]
    pub native_binary: Option<String>,
    #[serde(default)]
    pub grace_seconds: Option<u64>,
}

impl TerminationSpec {
    /// The strategy, `process_group` by default.
    pub fn strategy(&self) -> Strategy {
        self.strategy.unwrap_or(Strategy::ProcessGroup)
    }

    /// The seconds between the polite stop and the kill, 10 by default.
    pub fn grace_seconds(&self) -> u64 {
        self.grace_seconds.unwrap_or(10)
    }
}

/// `[[capture.hosts]]`: one host the proxy intercepts, and the request paths it records.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HostSpec {
    pub host: String,
    #[serde(default)]
    pub evidence_paths: Vec<String>,
    #[serde(default)]
    pub prefix_paths: Vec<String>,
    /// An open decoder hint. An unknown one is capture-only, not refused (D43, ruling 8).
    #[serde(default)]
    pub protocol_hint: Option<String>,
}

/// `[redaction]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RedactionSpec {
    #[serde(default)]
    pub extra_patterns: Vec<ExtraPattern>,
}

/// One connector redaction rule. The regex must be one of
/// [`EXTRA_PATTERN_SOURCES`]: a new recognizer is a core PR.
///
/// The schema types are `#[non_exhaustive]` (design 4.1: a new optional field stays schema 1),
/// so a downstream crate parses them and never builds one from a literal:
///
/// ```compile_fail
/// let _ = goalcup_protocol::harness::ExtraPattern { kind: "k".into(), regex: "r".into() };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ExtraPattern {
    pub kind: String,
    pub regex: String,
}

impl HarnessManifest {
    /// The termination spec with its defaults applied.
    pub fn termination(&self) -> TerminationSpec {
        self.capture.termination.clone().unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------

/// One refusal of `harness.toml`.
fn refuse(field: impl Into<String>, reason: impl Into<String>) -> ProtocolError {
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
    /// (rule 2). Does not validate; call [`Self::validate`].
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
        toml::from_str::<HarnessManifest>(text).map_err(|e| map_toml_error(text, &e))
    }
}

// ---------------------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------------------

/// Template variables an `args` element or an env value may use. `{session_id}` additionally
/// needs `identity.source = "preassigned"` (a cross-field rule).
const TEMPLATE_TOKENS: &[&str] = &["workspace", "port", "session_id"];

/// A template that may name only [`TEMPLATE_TOKENS`].
fn check_template(field: &str, what: &str, value: &str) -> Result<(), ProtocolError> {
    let tokens = crate::template::tokens(value).map_err(|why| {
        refuse(
            field,
            format!("{what} {why}: use `{{{{` and `}}}}` for literal braces"),
        )
    })?;
    for t in tokens {
        if !TEMPLATE_TOKENS.contains(&t) {
            return Err(refuse(
                field,
                format!(
                    "{what} names `{{{t}}}`, which is not a token here: only {} are",
                    TEMPLATE_TOKENS
                        .iter()
                        .map(|t| format!("`{{{t}}}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
    }
    Ok(())
}

/// Whether `value` uses `{name}` as a token.
fn uses_token(value: &str, name: &str) -> bool {
    crate::template::tokens(value).is_ok_and(|t| t.contains(&name))
}

fn is_id(s: &str) -> bool {
    let mut b = s.bytes();
    (2..=48).contains(&s.len())
        && b.next().is_some_and(|c| c.is_ascii_lowercase())
        && b.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn has_control(s: &str) -> bool {
    s.chars().any(char::is_control)
}

/// One relative path component: 1..=64 of `[A-Za-z0-9._-]`, never `.` or `..`.
fn is_path_component(c: &str) -> bool {
    !c.is_empty()
        && c.len() <= 64
        && c != "."
        && c != ".."
        && c.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A relative path of at most `max` components, each [`is_path_component`].
fn relative_path_problem(path: &str, max: usize) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > max {
        return Some(format!("has more than {max} components"));
    }
    if parts.iter().any(|c| !is_path_component(c)) {
        return Some(
            "must be relative, of components of `[A-Za-z0-9._-]` (1 to 64 bytes), with no `.` or `..`"
                .to_string(),
        );
    }
    None
}

/// A dot path of at most 4 segments of `[A-Za-z0-9_]`.
fn is_dot_path(s: &str) -> bool {
    let segs: Vec<&str> = s.split('.').collect();
    (1..=4).contains(&segs.len())
        && segs
            .iter()
            .all(|g| !g.is_empty() && g.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}

/// A single-component glob matcher: `*` any run (including none), `?` one character.
fn glob_matches(name: &str, pattern: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let (mut i, mut j) = (0usize, 0usize);
    let (mut star, mut matched) = (None, 0usize);
    while i < name.len() {
        if j < pattern.len() && (pattern[j] == '?' || pattern[j] == name[i]) {
            i += 1;
            j += 1;
        } else if j < pattern.len() && pattern[j] == '*' {
            star = Some(j);
            matched = i;
            j += 1;
        } else if let Some(position) = star {
            j = position + 1;
            matched += 1;
            i = matched;
        } else {
            return false;
        }
    }
    while j < pattern.len() && pattern[j] == '*' {
        j += 1;
    }
    j == pattern.len()
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// How a native `root` splits: which home it names and the relative path under it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RootHome {
    Config,
    Run,
}

fn split_native_root(root: &str) -> Result<(RootHome, &str), String> {
    let (home, rest) = if let Some(r) = root.strip_prefix("{config_home}") {
        (RootHome::Config, r)
    } else if let Some(r) = root.strip_prefix("{run_home}") {
        (RootHome::Run, r)
    } else {
        return Err("must start with `{config_home}` or `{run_home}`".to_string());
    };
    if rest.is_empty() {
        return Ok((home, ""));
    }
    let Some(rel) = rest.strip_prefix('/') else {
        return Err(
            "must be the home token alone, or the token, `/` and a relative path".to_string(),
        );
    };
    if let Some(why) = relative_path_problem(rel, 8) {
        return Err(format!("relative path {why}"));
    }
    Ok((home, rel))
}

/// The pattern components of a native `pattern`, `{id}` read as `*`, or the reason it is not
/// the `NativeSourceSpec` grammar.
fn native_pattern_components(pattern: &str, source_has_id: bool) -> Result<Vec<String>, String> {
    if pattern.is_empty() || pattern.len() > 1024 {
        return Err("is 1 to 1024 bytes".to_string());
    }
    if pattern.starts_with('/') {
        return Err("is relative to its root".to_string());
    }
    let ids = pattern.matches("{id}").count();
    if ids > 1 {
        return Err("holds more than one `{id}`".to_string());
    }
    if source_has_id && ids == 0 {
        return Err(
            "needs `{id}`: the identity source names a session, so the pattern must scope to it"
                .to_string(),
        );
    }
    if !source_has_id && ids == 1 {
        return Err("names `{id}`, but `identity.source = \"none\"` has no session id".to_string());
    }
    let stripped = pattern.replace("{id}", "");
    if stripped.contains(['{', '}']) {
        return Err("may use no brace but `{id}`".to_string());
    }
    let mut out = Vec::new();
    for part in pattern.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err("has an empty component, or `.` or `..`".to_string());
        }
        if part.contains("**") {
            return Err(
                "holds `**`: recursive globs are not supported, name each level".to_string(),
            );
        }
        let literal = part.replace("{id}", "");
        if !literal
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'*' | b'?'))
        {
            return Err(
                "has a component outside `[A-Za-z0-9._-]`, `*`, `?` and `{id}`".to_string(),
            );
        }
        out.push(part.replace("{id}", "*"));
    }
    if out.len() > 8 {
        return Err("has more than eight components".to_string());
    }
    Ok(out)
}

impl HarnessManifest {
    /// Refuse every manifest the refusal table names. The order is fixed, so the first
    /// refusal is deterministic: executable, args and goal, env, home, credential, CA,
    /// identity, native, argv guard, termination, hosts, redaction, the delivery-agnostic
    /// rule, then the cross-field rules.
    ///
    /// The CLI calls it at load and again while building a launch; the daemon calls it in
    /// `capture_harness`. Neither trusts the other's check.
    pub fn validate(&self, origin: Origin) -> Result<(), ProtocolError> {
        if !SUPPORTED_SCHEMA_VERSIONS.contains(&self.schema_version) {
            return Err(refuse(
                "schema_version",
                format!(
                    "This build reads connector schema 1; the manifest declares {}. \
                     {MIGRATION_SENTENCE}",
                    self.schema_version
                ),
            ));
        }
        if !is_id(&self.id) {
            return Err(refuse(
                "id",
                format!("`{}` must match `^[a-z][a-z0-9-]{{1,47}}$`", self.id),
            ));
        }
        self.check_executable(origin)?;
        self.check_args_and_goal(origin)?;
        self.check_env()?;
        self.check_home()?;
        self.check_credential()?;
        self.check_ca()?;
        self.check_identity()?;
        self.check_native()?;
        self.check_argv_guard()?;
        self.check_termination()?;
        self.check_hosts()?;
        self.check_redaction()?;
        self.check_delivery_agnostic()?;
        self.check_cross_field()?;
        Ok(())
    }

    fn check_executable(&self, origin: Origin) -> Result<(), ProtocolError> {
        let exe = &self.capture.executable;
        let program_ok = (1..=64).contains(&exe.program.len())
            && exe
                .program
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if !program_ok {
            return Err(refuse(
                "capture.executable.program",
                format!(
                    "`{}` must be a bare name resolved on PATH: 1 to 64 of `[A-Za-z0-9._-]`, no path",
                    exe.program
                ),
            ));
        }
        if origin == Origin::Pinned && is_interpreter_program(&exe.program) {
            return Err(refuse(
                "capture.executable.program",
                format!(
                    "`{}` is an interpreter, and an interpreter's `args` would be the program: a \
                     pinned connector names the harness's own entry point (a kit case under \
                     `--dev` may run one)",
                    exe.program
                ),
            ));
        }
        let mut v = exe.version.bytes();
        let version_ok = exe.version.len() <= 32
            && v.next().is_some_and(|b| b.is_ascii_digit())
            && v.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'));
        if !version_ok {
            return Err(refuse(
                "capture.executable.version",
                format!(
                    "`{}` must match `^[0-9][0-9A-Za-z.+-]{{0,31}}$`",
                    exe.version
                ),
            ));
        }
        if !is_hex64(&exe.sha256) {
            return Err(refuse(
                "capture.executable.sha256",
                "must be 64 lowercase hex characters",
            ));
        }
        Ok(())
    }

    fn check_args_and_goal(&self, origin: Origin) -> Result<(), ProtocolError> {
        let c = &self.capture;
        if c.args.is_empty() || c.args.len() > MAX_CAPTURE_ARGS {
            return Err(refuse(
                "capture.args",
                format!("must hold 1 to {MAX_CAPTURE_ARGS} arguments"),
            ));
        }
        for a in &c.args {
            if a.len() > MAX_CAPTURE_ARG_BYTES || has_control(a) {
                return Err(refuse(
                    "capture.args",
                    "an argument is longer than the bound or contains a control character",
                ));
            }
            if let Some(why) = capture_arg_denied(a) {
                return Err(refuse("capture.args", format!("`{a}` {why}")));
            }
            // N1: the one observable predicate. A pinned connector never needs an absolute
            // path; the controlled workload and the kit substitute them (`Origin::Dev`).
            if origin == Origin::Pinned
                && let Some(why) = arg_path_denied(a)
            {
                return Err(refuse("capture.args", format!("`{a}` {why}")));
            }
            check_template("capture.args", &format!("`{a}`"), a)?;
        }
        if let Some(flag) = &c.goal_flag {
            let mut b = flag.bytes();
            let plain = flag.len() <= MAX_GOAL_FLAG_BYTES
                && b.next() == Some(b'-')
                && b.next() == Some(b'-')
                && b.next().is_some_and(|x| x.is_ascii_lowercase())
                && b.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == b'-');
            if !plain {
                return Err(refuse(
                    "capture.goal_flag",
                    format!(
                        "`{flag}` must be one long option name: `--`, a lowercase letter, then \
                         lowercase letters, digits and `-`, at most {MAX_GOAL_FLAG_BYTES} \
                         bytes, with no `=` and no value"
                    ),
                ));
            }
            if let Some(why) = capture_arg_denied(flag) {
                return Err(refuse("capture.goal_flag", format!("`{flag}` {why}")));
            }
        }
        // The goal is placed exactly one way: after a closing `--`, or inside `goal_flag`.
        let slots = usize::from(c.args.last().is_some_and(|a| a == "--"))
            + usize::from(c.goal_flag.is_some());
        if slots != 1 {
            return Err(refuse(
                "capture.goal_flag",
                "the goal is placed exactly one way: `args` end in `--` and the goal follows as \
                 one argument, xor `goal_flag` names the option that carries it",
            ));
        }
        if c.continuation.trim().is_empty()
            || c.continuation.len() > MAX_CONTINUATION_BYTES
            || has_control(&c.continuation)
        {
            return Err(refuse(
                "capture.continuation",
                format!(
                    "must be one line of declarative provenance, 1 to {MAX_CONTINUATION_BYTES} bytes"
                ),
            ));
        }
        Ok(())
    }

    fn check_env(&self) -> Result<(), ProtocolError> {
        let env = &self.capture.env;
        if env.len() > MAX_CAPTURE_ENV {
            return Err(refuse(
                "capture.env",
                format!("holds more than {MAX_CAPTURE_ENV} entries"),
            ));
        }
        for (k, v) in env {
            let mut b = k.bytes();
            let well_formed = k.len() <= MAX_CAPTURE_ENV_KEY_BYTES
                && b.next()
                    .is_some_and(|x| x.is_ascii_alphabetic() || x == b'_')
                && b.all(|x| x.is_ascii_alphanumeric() || x == b'_');
            if !well_formed {
                return Err(refuse(
                    "capture.env",
                    format!("`{k}` is not a variable name"),
                ));
            }
            if let Some(why) = env_key_denied(k, EnvContext::CaptureEnv) {
                return Err(refuse("capture.env", format!("`{k}` {why}")));
            }
            if v.len() > MAX_CAPTURE_ENV_VALUE_BYTES || has_control(v) {
                return Err(refuse(
                    "capture.env",
                    format!("`{k}`'s value is longer than the bound or holds a control character"),
                ));
            }
            // Template tokens are checked on the template, before rendering.
            check_template("capture.env", &format!("`{k}`'s value"), v)?;
            if let Some(why) = env_value_denied(k, v) {
                return Err(refuse("capture.env", format!("`{k}`'s value {why}")));
            }
        }
        Ok(())
    }

    fn check_home(&self) -> Result<(), ProtocolError> {
        let h = &self.capture.home;
        let mut b = h.env.bytes();
        let env_ok = h.env.len() <= 64
            && b.next().is_some_and(|x| x.is_ascii_uppercase())
            && b.all(|x| x.is_ascii_uppercase() || x.is_ascii_digit() || x == b'_');
        if !env_ok {
            return Err(refuse(
                "capture.home.env",
                format!("`{}` must match `^[A-Z][A-Z0-9_]{{0,63}}$`", h.env),
            ));
        }
        if let Some(why) = env_key_denied(&h.env, EnvContext::HomeEnv) {
            return Err(refuse("capture.home.env", format!("`{}` {why}", h.env)));
        }
        if let Some(dir) = &h.per_run_dir {
            let mut b = dir.bytes();
            let ok = dir.len() <= 32
                && b.next()
                    .is_some_and(|x| x.is_ascii_lowercase() || x.is_ascii_digit())
                && b.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == b'-');
            if !ok {
                return Err(refuse(
                    "capture.home.per_run_dir",
                    format!("`{dir}` must match `^[a-z0-9][a-z0-9-]{{0,31}}$`"),
                ));
            }
        }
        if h.dirs.len() > 8 {
            return Err(refuse("capture.home.dirs", "holds more than 8 entries"));
        }
        for d in &h.dirs {
            if let Some(why) = relative_path_problem(d, 4) {
                return Err(refuse("capture.home.dirs", format!("`{d}` {why}")));
            }
            if let Some(why) = startup_file_problem(d) {
                return Err(refuse("capture.home.dirs", format!("`{d}` {why}")));
            }
        }
        if h.seed.len() > 16 {
            return Err(refuse("capture.home.seed", "holds more than 16 files"));
        }
        for s in &h.seed {
            if let Some(why) = relative_path_problem(&s.file, 4) {
                return Err(refuse("capture.home.seed", format!("`{}` {why}", s.file)));
            }
            // I5a: a seed that is a startup file is code the participant's tools run.
            if let Some(why) = startup_file_problem(&s.file) {
                return Err(refuse("capture.home.seed", format!("`{}` {why}", s.file)));
            }
        }
        if let Some(d) = has_duplicate(h.seed.iter().map(|s| s.file.as_str())) {
            return Err(refuse(
                "capture.home.seed",
                format!("`{d}` is seeded twice"),
            ));
        }
        if h.real_dirs.is_empty() || h.real_dirs.len() > 4 {
            return Err(refuse(
                "capture.home.real_dirs",
                "must hold 1 to 4 directories the config home may not sit in",
            ));
        }
        for r in &h.real_dirs {
            let Some(rel) = r.strip_prefix("{user_home}/") else {
                return Err(refuse(
                    "capture.home.real_dirs",
                    format!("`{r}` must be `{{user_home}}/` and a relative path"),
                ));
            };
            if let Some(why) = relative_path_problem(rel, 8) {
                return Err(refuse("capture.home.real_dirs", format!("`{r}` {why}")));
            }
        }
        Ok(())
    }

    fn check_credential(&self) -> Result<(), ProtocolError> {
        if let Some(c) = &self.capture.credential
            && !CREDENTIAL_ENV.contains(&c.env.as_str())
        {
            return Err(refuse(
                "capture.credential.env",
                format!(
                    "`{}` is not one of the provider credential variables a capture profile may \
                     name ({}): a new provider is a core PR with a DECISIONS entry",
                    c.env,
                    CREDENTIAL_ENV.join(", ")
                ),
            ));
        }
        Ok(())
    }

    fn check_ca(&self) -> Result<(), ProtocolError> {
        let ca = &self.capture.ca;
        if ca.env.is_empty() || ca.env.len() > 2 {
            return Err(refuse(
                "capture.ca.env",
                "must name 1 or 2 CA variables, or the harness could not trust the proxy",
            ));
        }
        if let Some(d) = has_duplicate(ca.env.iter().map(String::as_str)) {
            return Err(refuse("capture.ca.env", format!("`{d}` is named twice")));
        }
        for name in &ca.env {
            let Some(registry_mode) = ca_registry_mode(name) else {
                return Err(refuse(
                    "capture.ca.env",
                    format!(
                        "`{name}` is not a CA variable in the registry: a new CA variable is a \
                         policy row and a DECISIONS entry, and SSL_CERT_FILE, SSL_CERT_DIR, \
                         CURL_CA_BUNDLE and REQUESTS_CA_BUNDLE stay refused because they \
                         replace the roots of every tool child"
                    ),
                ));
            };
            if registry_mode != ca.mode {
                return Err(refuse(
                    "capture.ca.mode",
                    format!(
                        "`{name}` is a `{}`-mode variable in the registry, and the manifest \
                         declares `{}`",
                        registry_mode.as_str(),
                        ca.mode.as_str()
                    ),
                ));
            }
        }
        Ok(())
    }

    fn check_identity(&self) -> Result<(), ProtocolError> {
        let i = &self.capture.identity;
        // Which sibling fields each source requires and refuses.
        let json = (&i.match_field, &i.match_value, &i.id_field);
        let line = (&i.prefix, &i.grammar);
        match i.source {
            IdentitySource::StdoutJson => {
                let (Some(mf), Some(mv), Some(idf)) = json else {
                    return Err(refuse(
                        "capture.identity",
                        "`stdout_json` needs `match_field`, `match_value` and `id_field`",
                    ));
                };
                for (name, v) in [("match_field", mf), ("id_field", idf)] {
                    if !is_dot_path(v) {
                        return Err(refuse(
                            format!("capture.identity.{name}"),
                            format!(
                                "`{v}` must be a dot path of at most 4 segments of `[A-Za-z0-9_]`"
                            ),
                        ));
                    }
                }
                if mv.is_empty() || mv.len() > 64 || has_control(mv) {
                    return Err(refuse(
                        "capture.identity.match_value",
                        "must be 1 to 64 bytes without a control character",
                    ));
                }
                if line.0.is_some() || line.1.is_some() {
                    return Err(refuse(
                        "capture.identity",
                        "`prefix` and `grammar` belong to `stderr_line`, not `stdout_json`",
                    ));
                }
            }
            IdentitySource::StderrLine => {
                let (Some(prefix), Some(_)) = line else {
                    return Err(refuse(
                        "capture.identity",
                        "`stderr_line` needs `prefix` and `grammar`",
                    ));
                };
                if prefix.is_empty()
                    || prefix.len() > 32
                    || !prefix.bytes().all(|b| (0x20..0x7f).contains(&b))
                {
                    return Err(refuse(
                        "capture.identity.prefix",
                        "must be 1 to 32 printable ASCII bytes",
                    ));
                }
                if json.0.is_some() || json.1.is_some() || json.2.is_some() {
                    return Err(refuse(
                        "capture.identity",
                        "`match_field`, `match_value` and `id_field` belong to `stdout_json`, \
                         not `stderr_line`",
                    ));
                }
            }
            IdentitySource::Preassigned | IdentitySource::None => {
                if json.0.is_some()
                    || json.1.is_some()
                    || json.2.is_some()
                    || line.0.is_some()
                    || line.1.is_some()
                {
                    return Err(refuse(
                        "capture.identity",
                        format!(
                            "`{}` takes no `match_*`, `id_field`, `prefix` or `grammar`",
                            i.source.as_str()
                        ),
                    ));
                }
            }
        }
        // The admitted (source, binding) pairs of schema 1. Widening a row is a core PR that
        // adds a kit case and stays schema 1.
        let supported = match (i.source, i.binding) {
            (IdentitySource::Preassigned, Some(Binding::AtLaunch))
            | (IdentitySource::StdoutJson, Some(Binding::AtLaunch))
            | (IdentitySource::StderrLine, Some(Binding::Deferred))
            | (IdentitySource::None, None) => true,
            (IdentitySource::None, Some(_)) => {
                return Err(refuse(
                    "capture.identity.binding",
                    "`none` has no session to bind, so it takes no `binding`",
                ));
            }
            (_, None) => {
                return Err(refuse(
                    "capture.identity.binding",
                    "is required unless `source = \"none\"`",
                ));
            }
            _ => false,
        };
        if !supported {
            return Err(refuse(
                "capture.identity",
                format!(
                    "`{}` with binding `{}` is not supported by this build: schema 1 admits \
                     preassigned+at_launch, stdout_json+at_launch, stderr_line+deferred and none",
                    i.source.as_str(),
                    i.binding.map_or("-", Binding::as_str)
                ),
            ));
        }
        Ok(())
    }

    fn check_native(&self) -> Result<(), ProtocolError> {
        let c = &self.capture;
        let n = match c.native.as_slice() {
            [one] => one,
            [] => {
                return Err(refuse(
                    "capture.native",
                    "a harness without a native session store is refused: schema 1 needs exactly \
                     one `[[capture.native]]`. Whether such a harness is capturable at all is a \
                     core decision about claim level, not a manifest option",
                ));
            }
            _ => {
                return Err(refuse(
                    "capture.native",
                    "schema 1 holds exactly one `[[capture.native]]`",
                ));
            }
        };
        let valid_name = (1..=64).contains(&n.name.len())
            && n.name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if !valid_name {
            return Err(refuse(
                "capture.native.name",
                format!("`{}` must be 1 to 64 of `[A-Za-z0-9._-]`", n.name),
            ));
        }
        split_native_root(&n.root)
            .map_err(|why| refuse("capture.native.root", format!("`{}` {why}", n.root)))?;
        native_pattern_components(&n.pattern, c.identity.source != IdentitySource::None)
            .map_err(|why| refuse("capture.native.pattern", format!("`{}` {why}", n.pattern)))?;
        match (&n.protocol_hint, n.form) {
            (Some(_), NativeForm::Snapshot) => {
                return Err(refuse(
                    "capture.native.protocol_hint",
                    "applies to `append_only` stores only",
                ));
            }
            (Some(h), NativeForm::AppendOnly) if !valid_hint(h) => {
                return Err(refuse(
                    "capture.native.protocol_hint",
                    format!("`{h}` must match `^[a-z][a-z0-9.-]{{0,63}}$`"),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn check_argv_guard(&self) -> Result<(), ProtocolError> {
        let Some(g) = &self.capture.argv_guard else {
            return Ok(());
        };
        if g.long.len() > 16 || g.short.len() > 16 || g.short_with_value.len() > 16 {
            return Err(refuse(
                "capture.argv_guard",
                "a list holds more than 16 entries",
            ));
        }
        for l in &g.long {
            let mut b = l.bytes();
            let ok = l.len() <= 32
                && b.next().is_some_and(|x| x.is_ascii_lowercase())
                && b.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == b'-');
            if !ok {
                return Err(refuse(
                    "capture.argv_guard.long",
                    format!(
                        "`{l}` must be an option name without `--`: `^[a-z][a-z0-9-]{{0,31}}$`"
                    ),
                ));
            }
        }
        for (field, list) in [
            ("short", &g.short),
            ("short_with_value", &g.short_with_value),
        ] {
            for s in list {
                if s.chars().count() != 1 || !s.bytes().all(|b| b.is_ascii_alphanumeric()) {
                    return Err(refuse(
                        format!("capture.argv_guard.{field}"),
                        format!("`{s}` must be one ASCII letter or digit"),
                    ));
                }
            }
        }
        for a in &self.capture.args {
            if let Some(why) = g.denies(a) {
                return Err(refuse("capture.args", format!("`{a}` {why}")));
            }
        }
        if let Some(flag) = &self.capture.goal_flag
            && let Some(why) = g.denies(flag)
        {
            return Err(refuse("capture.goal_flag", format!("`{flag}` {why}")));
        }
        Ok(())
    }

    fn check_termination(&self) -> Result<(), ProtocolError> {
        let Some(t) = &self.capture.termination else {
            return Ok(());
        };
        let strategy = t.strategy();
        match (&t.native_binary, strategy) {
            (None, Strategy::NativePid) => {
                return Err(refuse(
                    "capture.termination.native_binary",
                    "is required with `native_pid`: the npm wrapper hides the binary, so name it",
                ));
            }
            (Some(_), Strategy::ProcessGroup) => {
                return Err(refuse(
                    "capture.termination.native_binary",
                    "is refused with `process_group`",
                ));
            }
            (Some(b), Strategy::NativePid) => {
                let ok = (1..=32).contains(&b.len())
                    && b.bytes()
                        .all(|x| x.is_ascii_alphanumeric() || matches!(x, b'.' | b'_' | b'-'));
                if !ok {
                    return Err(refuse(
                        "capture.termination.native_binary",
                        format!("`{b}` must be a `comm` name: `^[A-Za-z0-9._-]{{1,32}}$`"),
                    ));
                }
            }
            (None, Strategy::ProcessGroup) => {}
        }
        if let Some(g) = t.grace_seconds
            && !(1..=60).contains(&g)
        {
            return Err(refuse(
                "capture.termination.grace_seconds",
                format!("{g} must be 1 to 60"),
            ));
        }
        Ok(())
    }

    fn check_hosts(&self) -> Result<(), ProtocolError> {
        let c = &self.capture;
        if c.hosts.is_empty() || c.hosts.len() > MAX_CAPTURE_HOSTS {
            return Err(refuse(
                "capture.hosts",
                format!("must hold 1 to {MAX_CAPTURE_HOSTS} hosts"),
            ));
        }
        if let Some(d) = has_duplicate(c.hosts.iter().map(|h| h.host.as_str())) {
            return Err(refuse(
                "capture.hosts",
                format!("`{d}` has two rules, which are ambiguous"),
            ));
        }
        if c.auth_hosts.len() > MAX_CAPTURE_HOSTS {
            return Err(refuse(
                "capture.auth_hosts",
                format!("holds more than {MAX_CAPTURE_HOSTS} hosts"),
            ));
        }
        if let Some(d) = has_duplicate(c.auth_hosts.iter().map(String::as_str)) {
            return Err(refuse(
                "capture.auth_hosts",
                format!("`{d}` is listed twice"),
            ));
        }
        for a in &c.auth_hosts {
            if !is_capture_host(a) {
                return Err(refuse(
                    "capture.auth_hosts",
                    format!("`{a}` is not a lowercase DNS name without port or trailing dot"),
                ));
            }
        }
        for h in &c.hosts {
            if !is_capture_host(&h.host) {
                return Err(refuse(
                    "capture.hosts",
                    format!(
                        "`{}` is not a lowercase DNS name without port or trailing dot",
                        h.host
                    ),
                ));
            }
            if c.auth_hosts.contains(&h.host) {
                return Err(refuse(
                    "capture.auth_hosts",
                    format!(
                        "`{}` is an authentication host, which always tunnels, so it cannot \
                         also be intercepted: a model host belongs in `hosts`",
                        h.host
                    ),
                ));
            }
            let paths = h.evidence_paths.len() + h.prefix_paths.len();
            if paths == 0 || paths > MAX_CAPTURE_PATHS_PER_HOST {
                return Err(refuse(
                    "capture.hosts",
                    format!(
                        "`{}` must scope 1 to {MAX_CAPTURE_PATHS_PER_HOST} paths; a host with \
                         none would be decrypted and record nothing",
                        h.host
                    ),
                ));
            }
            if let Some(d) = has_duplicate(
                h.evidence_paths
                    .iter()
                    .chain(&h.prefix_paths)
                    .map(String::as_str),
            ) {
                return Err(refuse(
                    "capture.hosts",
                    format!(
                        "`{}` lists path `{d}` twice; a path is either exact or a prefix",
                        h.host
                    ),
                ));
            }
            for p in h.evidence_paths.iter().chain(&h.prefix_paths) {
                if let Some(why) = capture_path_problem(p) {
                    return Err(refuse(
                        "capture.hosts",
                        format!("`{}` path `{p}` {why}", h.host),
                    ));
                }
            }
            if h.prefix_paths.iter().any(|p| p == "/") {
                return Err(refuse(
                    "capture.hosts",
                    format!(
                        "`{}` prefix `/` records every request on the host, which is not a \
                         scoped path",
                        h.host
                    ),
                ));
            }
            // An unknown hint is NOT refused: it means capture-only (D43, ruling 8). Only the
            // grammar is checked, so a misspelling is a visible name and not garbage.
            if let Some(hint) = &h.protocol_hint
                && !valid_hint(hint)
            {
                return Err(refuse(
                    "capture.hosts.protocol_hint",
                    format!("`{hint}` must match `^[a-z][a-z0-9.-]{{0,63}}$`"),
                ));
            }
        }
        Ok(())
    }

    fn check_redaction(&self) -> Result<(), ProtocolError> {
        let Some(r) = &self.redaction else {
            return Ok(());
        };
        for p in &r.extra_patterns {
            let mut b = p.kind.bytes();
            let kind_ok = p.kind.len() <= 48
                && b.next().is_some_and(|x| x.is_ascii_lowercase())
                && b.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == b'_');
            if !kind_ok {
                return Err(refuse(
                    "redaction.extra_patterns",
                    format!("kind `{}` must match `^[a-z][a-z0-9_]{{0,47}}$`", p.kind),
                ));
            }
            if !EXTRA_PATTERN_SOURCES.contains(&p.regex.as_str()) {
                return Err(refuse(
                    "redaction.extra_patterns",
                    format!(
                        "`{}`'s regex is not one the recorder's streaming redactor can hold: a \
                         new recognizer is a core PR (daemon `EXTRA_SHAPES` and its mirror \
                         `capture_policy::EXTRA_PATTERN_SOURCES`)",
                        p.kind
                    ),
                ));
            }
        }
        Ok(())
    }

    /// The delivery-agnostic rule (F7): a harness never names the tracker, a server or an
    /// upload. Schema 1 has no field for one, and `deny_unknown_fields` keeps it that way.
    fn check_delivery_agnostic(&self) -> Result<(), ProtocolError> {
        let c = &self.capture;
        for a in &c.args {
            if mentions_goalcup(a) {
                return Err(refuse(
                    "capture.args",
                    format!("`{a}` names the tracker: a harness is delivery-agnostic"),
                ));
            }
        }
        if let Some(f) = &c.goal_flag
            && mentions_goalcup(f)
        {
            return Err(refuse(
                "capture.goal_flag",
                "names the tracker: a harness is delivery-agnostic",
            ));
        }
        for (k, v) in &c.env {
            if k.to_ascii_uppercase().starts_with("GOALCUP_") || mentions_goalcup(v) {
                return Err(refuse(
                    "capture.env",
                    format!("`{k}` names the tracker: a harness is delivery-agnostic"),
                ));
            }
        }
        let hosts = c
            .hosts
            .iter()
            .map(|h| ("capture.hosts", h.host.as_str()))
            .chain(
                c.auth_hosts
                    .iter()
                    .map(|h| ("capture.auth_hosts", h.as_str())),
            );
        for (field, host) in hosts {
            if mentions_goalcup(host) || is_tracker_host(host) {
                return Err(refuse(
                    field,
                    format!("`{host}` is the tracker's own host: a harness is delivery-agnostic"),
                ));
            }
        }
        Ok(())
    }

    fn check_cross_field(&self) -> Result<(), ProtocolError> {
        let c = &self.capture;
        let i = &c.identity;
        let per_run = c.home.config_home == ConfigHome::PerRun;

        // Credential: two identity sources are exercised in schema 1, and both need a per-run
        // home. A `stdout_json` identity is read by the same stdout reader that scans for key
        // fragments (`capture_tee::tee_marked_capture_with_identity`).
        if c.credential.is_some() {
            match i.source {
                IdentitySource::StderrLine | IdentitySource::StdoutJson => {}
                IdentitySource::Preassigned | IdentitySource::None => {
                    return Err(refuse(
                        "capture.credential",
                        format!(
                            "a credential with `{}` identity is not supported by this build: \
                             schema 1 admits one only with `stderr_line` or `stdout_json` (a \
                             later core PR can widen the table with a kit case)",
                            i.source.as_str()
                        ),
                    ));
                }
            }
            if !per_run {
                return Err(refuse(
                    "capture.credential",
                    "requires `capture.home.config_home = \"per_run\"`: the key-fragment scan \
                     covers the run directory only (D41)",
                ));
            }
        }

        // `{session_id}` and `preassigned` imply each other.
        let uses_session = c.args.iter().any(|a| uses_token(a, "session_id"))
            || c.env.values().any(|v| uses_token(v, "session_id"));
        let args_use_session = c.args.iter().any(|a| uses_token(a, "session_id"));
        match i.source {
            IdentitySource::Preassigned if !args_use_session => {
                return Err(refuse(
                    "capture.identity.source",
                    "`preassigned` needs `{session_id}` in `args`: otherwise the minted id \
                     reaches nothing",
                ));
            }
            IdentitySource::Preassigned => {}
            _ if uses_session => {
                return Err(refuse(
                    "capture.args",
                    "`{session_id}` needs `identity.source = \"preassigned\"`: otherwise the \
                     id is not one the tracker minted",
                ));
            }
            _ => {}
        }

        // Per-run-only fields.
        let h = &c.home;
        for (field, present) in [
            ("seed", !h.seed.is_empty()),
            ("dirs", !h.dirs.is_empty()),
            ("per_run_dir", h.per_run_dir.is_some()),
        ] {
            if present && !per_run {
                return Err(refuse(
                    format!("capture.home.{field}"),
                    "requires `capture.home.config_home = \"per_run\"`",
                ));
            }
        }

        // `none` must read a per-run root: another run's sessions are not this run's.
        let native = &c.native[0];
        let (root_home, root_rel) = split_native_root(&native.root).expect("checked by native");
        if i.source == IdentitySource::None && root_home == RootHome::Config && !per_run {
            return Err(refuse(
                "capture.native.root",
                "with `identity.source = \"none\"` the root must be per run: `{run_home}/...`, \
                 or `{config_home}/...` with `config_home = \"per_run\"`; an unscoped source in \
                 a shared home would read other runs' sessions",
            ));
        }

        // M10 and S2 review M-5: a native pattern may not sweep a seed file or a credential the
        // manifest seeds. A `{config_home}` root is the seeds' own directory. A `{run_home}` root
        // is another directory while the homes are disjoint, but where the per-run config home
        // sits relative to the run home is not this crate's to assume, so a `{run_home}` root is
        // checked both ways the two could meet: the same directory (the seeds read as paths under
        // the run home) and nested (`<per_run_dir>/<seed>` under it). Components compare
        // lower-cased, because APFS is case-insensitive and `Config.YAML` is `config.yaml`.
        let seed_homes: Vec<Vec<String>> = match root_home {
            RootHome::Config => vec![Vec::new()],
            RootHome::Run => vec![Vec::new(), vec![h.per_run_dir_name().to_ascii_lowercase()]],
        };
        let pattern = native_pattern_components(&native.pattern, i.source != IdentitySource::None)
            .expect("checked by native");
        let mut components: Vec<String> = if root_rel.is_empty() {
            Vec::new()
        } else {
            root_rel.split('/').map(str::to_ascii_lowercase).collect()
        };
        components.extend(pattern.iter().map(|p| p.to_ascii_lowercase()));
        for s in &h.seed {
            for home in &seed_homes {
                let dest: Vec<String> = home
                    .iter()
                    .cloned()
                    .chain(s.file.split('/').map(str::to_ascii_lowercase))
                    .collect();
                if dest.len() == components.len()
                    && dest
                        .iter()
                        .zip(&components)
                        .all(|(d, p)| glob_matches(d, p))
                {
                    let coincide = if root_home == RootHome::Run {
                        " (the per-run config home may be, or sit under, the run home)"
                    } else {
                        ""
                    };
                    return Err(refuse(
                        "capture.native.pattern",
                        format!(
                            "`{}` under `{}` matches the seed `{}`{coincide}: a native source \
                             may not read a file the tracker seeds",
                            native.pattern, native.root, s.file
                        ),
                    ));
                }
            }
        }

        // The variables this manifest pins are distinct, and none is a `capture.env` key.
        let mut pinned: Vec<(&str, String)> =
            vec![("capture.home.env", c.home.env.to_ascii_uppercase())];
        if let Some(cr) = &c.credential {
            pinned.push(("capture.credential.env", cr.env.to_ascii_uppercase()));
        }
        for e in &c.ca.env {
            pinned.push(("capture.ca.env", e.to_ascii_uppercase()));
        }
        for (n, (field, name)) in pinned.iter().enumerate() {
            if let Some((other, _)) = pinned[..n].iter().find(|(_, o)| o == name) {
                return Err(refuse(
                    *field,
                    format!(
                        "`{name}` is also named in `{other}`: each pinned variable is named once"
                    ),
                ));
            }
            if c.env.keys().any(|k| k.to_ascii_uppercase() == *name) {
                return Err(refuse(
                    "capture.env",
                    format!("`{name}` is pinned by `{field}`, so `capture.env` may not set it"),
                ));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------
// The loader
// ---------------------------------------------------------------------------------------

/// A connector directory, loaded: the harness manifest, the exact bytes of `harness.toml` that
/// went into the directory digest, the seed bytes the same walk read, and the digest.
///
/// Everything a caller writes or parses came out of the one read the digest covers, so a
/// second read of the directory can never authorize bytes the digest did not cover.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LoadedConnector {
    pub manifest: HarnessManifest,
    /// The `harness.toml` bytes that were hashed and parsed: one read, so a second read can
    /// never authorize bytes the digest did not cover.
    pub harness_toml: Vec<u8>,
    /// The manifest's seed files, in manifest order, with the bytes the directory walk hashed.
    /// The per-run home writer writes these and never re-reads `files/**` (M-6).
    pub seeds: Vec<LoadedSeed>,
    /// SHA-256 of the whole directory, lowercase hex ([`crate::connector_dir::connector_dir_sha256`]).
    /// This is what `Launch.connector_dir_sha256` records.
    pub connector_dir_sha256: String,
}

/// One seed file as the directory walk read it.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LoadedSeed {
    /// The manifest's `capture.home.seed[].file`: the path under `files/`, and the destination
    /// under the per-run config home.
    pub file: String,
    /// The mode it is written with.
    pub mode: SeedMode,
    /// Its bytes, as hashed.
    pub bytes: Vec<u8>,
}

/// Discover schema-1 connector directories as sorted immediate children of `root`.
///
/// A child directory with no `harness.toml` is not a schema-1 connector and is skipped. Once the
/// name exists it must be a regular file: a directory, symlink, device or other non-regular entry
/// is invalid rather than silently indistinguishable from absence. Root and iterator-entry errors
/// propagate; filesystem enumeration order never selects a connector.
pub fn discover_connector_dirs(root: &Path) -> Result<Vec<PathBuf>, ProtocolError> {
    let entries = std::fs::read_dir(root).map_err(|source| ProtocolError::Io {
        path: root.display().to_string(),
        source,
    })?;
    discover_connector_dirs_from(root, entries.map(|entry| entry.map(|entry| entry.path())))
}

fn discover_connector_dirs_from(
    root: &Path,
    entries: impl IntoIterator<Item = std::io::Result<PathBuf>>,
) -> Result<Vec<PathBuf>, ProtocolError> {
    let mut discovered = Vec::new();
    for child in entries {
        let child = child.map_err(|source| ProtocolError::Io {
            path: root.display().to_string(),
            source,
        })?;
        let child_kind = std::fs::symlink_metadata(&child).map_err(|source| ProtocolError::Io {
            path: child.display().to_string(),
            source,
        })?;
        if child_kind.file_type().is_symlink() {
            return Err(ProtocolError::UnhashableEntry {
                path: child.display().to_string(),
                reason: "an immediate connector child is a symlink, so its bytes are outside the \
                         directory being discovered"
                    .into(),
            });
        }
        if !child_kind.is_dir() {
            continue;
        }

        let manifest = child.join("harness.toml");
        match std::fs::symlink_metadata(&manifest) {
            Ok(metadata) if metadata.is_file() => discovered.push(child),
            Ok(_) => {
                return Err(ProtocolError::UnhashableEntry {
                    path: manifest.display().to_string(),
                    reason: "harness.toml exists but is not a regular file".into(),
                });
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ProtocolError::Io {
                    path: manifest.display().to_string(),
                    source,
                });
            }
        }
    }
    discovered.sort();
    Ok(discovered)
}

/// Prints the name, the mode and the size of a seed, never its bytes: a seed can be a credential
/// stand-in, and a `{:?}` on a loaded connector reaches terminals and logs.
impl std::fmt::Debug for LoadedSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LoadedSeed {{ file: {:?}, mode: {:?}, bytes: <{} bytes> }}",
            self.file,
            self.mode,
            self.bytes.len()
        )
    }
}

impl LoadedConnector {
    /// SHA-256 of the captured `harness.toml` bytes, lowercase hex: the profile digest. The
    /// bytes are the ones the directory digest covers and the manifest was parsed from, so the
    /// two digests describe one read.
    pub fn profile_sha256(&self) -> String {
        hex::encode(Sha256::digest(&self.harness_toml))
    }
}

/// Load one connector directory: the one loader the CLI, the daemon and the conformance test
/// share.
///
/// # `origin`
///
/// `origin` is the caller's word, and nothing in a manifest can claim `Dev`, so the loader
/// holds the line itself (S2 review M-1): `load_dir(_, Origin::Dev)` refuses a directory whose
/// id this build pins, or whose digest equals any pin. `Dev` is for an unpinned directory under
/// `GOALCUP_CONNECTORS_DIR` loaded with `--dev` (the CLI is the only door: the daemon cannot see
/// `--dev`). The origin is not recorded in `Launch` (the v5 shape is closed until the tag);
/// `Launch.connector_dir_sha256` against the pins says the same to a verifier.
///
/// `Pinned` does not check the pin: the caller compares [`LoadedConnector::connector_dir_sha256`]
/// with [`crate::pins`], as the daemon's preflight and the CLI's `pin_state` do.
///
/// # Reading
///
/// * It hashes the directory and parses the exact `harness.toml` bytes that went into the
///   hash, then validates for `origin`.
/// * It refuses any regular file outside `harness.toml` and the `files/**` entries
///   `capture.home.seed` names (a leftover V1 `manifest.toml` included, OS-29).
/// * Every seed must exist under `files/` and weigh at most [`MAX_SEED_BYTES`].
///
/// A symlink anywhere in the directory, or as the directory itself, is refused.
pub fn load_dir(dir: &Path, origin: Origin) -> Result<LoadedConnector, ProtocolError> {
    load_dir_resolving_origin(dir, |_, _| origin)
}

/// Load a directory reached through [`discover_connector_dirs`] and apply the protocol-owned
/// folder/id/digest admission rule before validation.
///
/// `unpinned_origin` is the caller's one remaining policy choice. The CLI supplies `Dev` only
/// when the human typed `--dev`; the daemon supplies `Dev` for its development-connector path.
/// A pinned row always validates as [`Origin::Pinned`].
pub fn load_discovered_dir(
    dir: &Path,
    unpinned_origin: Origin,
) -> Result<LoadedConnector, ProtocolError> {
    let folder_name = dir.file_name().unwrap_or_else(|| OsStr::new(""));
    load_dir_resolving_origin_fallible(dir, |id, digest| {
        match crate::pins::connector_folder_admission(folder_name, id, digest)? {
            crate::pins::ConnectorFolderAdmission::Pinned => Ok(Origin::Pinned),
            crate::pins::ConnectorFolderAdmission::Unpinned => Ok(unpinned_origin),
        }
    })
}

/// Load one connector directory and resolve its validation origin from the identity and digest
/// captured by that same load.
///
/// The resolver is called exactly once, after the directory has been walked, hashed and its
/// captured `harness.toml` bytes parsed, but before the pinned-directory guard and manifest
/// validation run. Its arguments are the captured manifest id and connector-directory digest.
/// Filesystem changes made by the resolver are not reread: the returned [`LoadedConnector`], its
/// validation and the pinned-id/digest guard all continue to use the bytes already captured.
///
/// This is the discovery form of [`load_dir`]. Callers that already know the origin should use
/// [`load_dir`]; callers discovering several directories can choose `Pinned` or `Dev` from each
/// directory's authoritative captured identity without parsing `harness.toml` a second time.
pub fn load_dir_resolving_origin(
    dir: &Path,
    resolve_origin: impl FnOnce(&str, &str) -> Origin,
) -> Result<LoadedConnector, ProtocolError> {
    load_dir_resolving_origin_fallible(dir, |id, digest| Ok(resolve_origin(id, digest)))
}

fn load_dir_resolving_origin_fallible(
    dir: &Path,
    resolve_origin: impl FnOnce(&str, &str) -> Result<Origin, ProtocolError>,
) -> Result<LoadedConnector, ProtocolError> {
    // The root itself: the walk below refuses a symlink INSIDE the directory, but `read_dir`
    // follows a link given as the root, which would load and pin bytes from somewhere else.
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(refuse(
                "connector_dir",
                format!(
                    "{} is a symlink: a connector directory is pinned by its own bytes, and a \
                     symlink's bytes are somewhere else",
                    dir.display()
                ),
            ));
        }
        _ => {}
    }
    // The seeds are read by the same walk that hashes them (M-6): everything under `files/` is
    // kept, and the loop below refuses any file that is not a named seed.
    let mut listing =
        hash_connector_dir_listing(dir, |p| p == "harness.toml" || p.starts_with("files/"))?;
    let harness_toml = listing.captured.remove("harness.toml").ok_or_else(|| {
        refuse(
            "connector_dir",
            format!("{} contains no regular harness.toml", dir.display()),
        )
    })?;
    let text =
        std::str::from_utf8(&harness_toml).map_err(|_| refuse("harness.toml", "is not UTF-8"))?;
    let manifest = HarnessManifest::from_toml_str(text)?;
    let origin = resolve_origin(&manifest.id, &listing.sha256)?;
    if origin == Origin::Dev {
        refuse_dev_for_a_pinned_directory(&manifest.id, &listing.sha256)?;
    }
    manifest.validate(origin)?;

    let seeds: Vec<String> = manifest
        .capture
        .home
        .seed
        .iter()
        .map(|s| format!("files/{}", s.file))
        .collect();
    for (path, _) in &listing.files {
        let allowed = path == "harness.toml" || seeds.iter().any(|s| s == path);
        if !allowed {
            return Err(refuse(
                "connector_dir",
                format!(
                    "`{path}` is not named by `capture.home.seed`: every file under `files/` \
                     must be a seed, and nothing else may sit in a connector directory"
                ),
            ));
        }
    }
    for seed in &seeds {
        match listing.files.iter().find(|(p, _)| p == seed) {
            None => {
                return Err(refuse(
                    "capture.home.seed",
                    format!("`{seed}` is named by the manifest but is not in the directory"),
                ));
            }
            Some((_, len)) if *len > MAX_SEED_BYTES => {
                return Err(refuse(
                    "capture.home.seed",
                    format!("`{seed}` is {len} bytes; a seed is at most {MAX_SEED_BYTES}"),
                ));
            }
            Some(_) => {}
        }
    }
    let seeds = manifest
        .capture
        .home
        .seed
        .iter()
        .map(|s| {
            let bytes = listing
                .captured
                .remove(&format!("files/{}", s.file))
                .expect("every seed was listed, so the walk captured it");
            LoadedSeed {
                file: s.file.clone(),
                mode: s.mode(),
                bytes,
            }
        })
        .collect();
    Ok(LoadedConnector {
        manifest,
        harness_toml,
        seeds,
        connector_dir_sha256: listing.sha256,
    })
}

/// `Dev` loads an unpinned directory only: a pinned id or a pinned digest means the directory is
/// (or claims to be) a shipped connector, which loads as `Pinned` and never picks up `Dev`'s
/// relaxations (S2 review M-1).
fn refuse_dev_for_a_pinned_directory(id: &str, digest: &str) -> Result<(), ProtocolError> {
    let id_pinned = crate::pins::pinned_connector_dir_sha256(id).is_some();
    let digest_pinned = crate::pins::CONNECTOR_DIR_SHA256S
        .iter()
        .any(|(pinned_id, _)| crate::pins::connector_dir_is_pinned(pinned_id, digest));
    if id_pinned || digest_pinned {
        return Err(refuse(
            "connector_dir",
            format!(
                "`{id}` is pinned by this build{}: `--dev` loads only an unpinned directory, and \
                 a pinned connector loads as itself, checked against its pin (a development \
                 copy needs its own id)",
                if id_pinned {
                    ""
                } else {
                    " (this directory's digest is a pinned one)"
                }
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    #[test]
    fn discovery_propagates_an_iterator_entry_error_instead_of_flattening_it() {
        let root = Path::new("/connector-root");
        let entries = vec![Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "synthetic unreadable directory entry",
        ))];

        let error = discover_connector_dirs_from(root, entries)
            .expect_err("an unreadable child is not an absent child");
        assert!(matches!(error, ProtocolError::Io { path, source }
            if path == root.display().to_string()
                && source.kind() == std::io::ErrorKind::PermissionDenied));
    }
}
