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
//!    (`SUPPORTED_SCHEMA_VERSIONS`). The executable pin's retired keys (`sha256`, `version`) are
//!    refused here with their own sentence (D53), before the strict deserialization would call
//!    them a newer schema's, and so is a `[capture.credential]` table (D61).
//! 2. It is then deserialized with `deny_unknown_fields` everywhere. An unknown key, and an
//!    unknown value of a closed-set field, are refused with one sentence ([`NEWER_SENTENCE`]):
//!    serde's own "unknown variant" text never reaches the user.
//! 3. [`HarnessManifest::validate`] runs the checks in a fixed order, so the first refusal
//!    is deterministic: executable, args and goal, env, home, CA, identity,
//!    native, argv guard, termination, hosts, the delivery-agnostic rule (F7), the
//!    cross-field rules, and then every rule again on the recipe each `[capture.platform.<key>]`
//!    resolves to (an override may replace only env entries).
//!
//! [`Origin`] is the one observable predicate that relaxes anything: `Dev` (an unpinned
//! directory under `GOALCUP_CONNECTORS_DIR` loaded with `--dev`, for kit cases and the
//! controlled workload) skips the interpreter refusal, the absolute-path `args` refusal and the
//! loopback-host refusal (the kit's stand-ins are `*.localhost`). Env values, seeds, the deny
//! lists and every cross-field rule stay strict for both.

use crate::ProtocolError;
use crate::capture_policy::{
    CaMode, EnvContext, MAX_CAPTURE_ARG_BYTES, MAX_CAPTURE_ARGS, MAX_CAPTURE_ENV,
    MAX_CAPTURE_ENV_KEY_BYTES, MAX_CAPTURE_ENV_VALUE_BYTES, MAX_CAPTURE_HOSTS,
    MAX_CAPTURE_PATHS_PER_HOST, MAX_CONTINUATION_BYTES, MAX_GOAL_FLAG_BYTES, arg_path_denied,
    ca_env_problem, capture_arg_denied, capture_path_problem, env_key_denied, env_value_denied,
    goal_flag_denied, has_duplicate, is_capture_host, is_interpreter_program, is_tracker_host,
    local_host_problem, mentions_goalcup, seed_content_problem, startup_file_problem,
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
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

/// The folder under the tracker's configuration directory that holds every home kept across
/// runs: a shared config home (D39) and a shared run `HOME` (D59).
pub const CAPTURE_HOMES_DIR: &str = "capture-homes";

closed_enum! {
    /// What `HOME` holds for a run.
    RunHome, "capture.home.run_home", special: &[];
    /// An empty directory (Codex: keeps `~/.agents/skills` and shell startup files out).
    Empty = "empty",
    /// A directory whose only entry is `Library/Keychains`, linked to the user's own (Claude
    /// Code: subscription login, no dotfiles).
    KeychainLink = "keychain_link",
    /// One empty directory per connector kept across runs, so a harness with no `home.env`
    /// keeps its login (D59). Held to the rules of a shared config home: runs on it are
    /// serialized, and the daemon refuses it outside `<config_dir>/`[`CAPTURE_HOMES_DIR`], at
    /// the participant's `HOME` or at a `real_dirs` path.
    Shared = "shared",
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

/// `capture.ca.mode` is the policy type itself: it decides which file the launch hands the
/// variables. It refuses an unknown value as `closed_enum!` does.
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

closed_enum! {
    /// The platform a `[capture.platform.<key>]` override applies to: `std::env::consts::OS`,
    /// `-`, `std::env::consts::ARCH`. The declaration order is the order the platform entries
    /// are validated in.
    PlatformKey, "capture.platform", special: &[];
    LinuxX86_64 = "linux-x86_64",
    LinuxAarch64 = "linux-aarch64",
    MacosAarch64 = "macos-aarch64",
    MacosX86_64 = "macos-x86_64",
}

impl PlatformKey {
    /// The platform this build runs on, or `None` on a target the closed set does not name
    /// (which then runs the base values).
    pub fn current() -> Option<PlatformKey> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => Some(Self::LinuxX86_64),
            ("linux", "aarch64") => Some(Self::LinuxAarch64),
            ("macos", "aarch64") => Some(Self::MacosAarch64),
            ("macos", "x86_64") => Some(Self::MacosX86_64),
            _ => None,
        }
    }
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
    /// the controlled workload). Relaxes the interpreter refusal, the absolute-path `args`
    /// refusal and the loopback-host refusal, and nothing else.
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
    /// How core messages name the harness to the participant. Optional; see [`ParticipantSpec`].
    #[serde(default)]
    pub participant: Option<ParticipantSpec>,
}

/// The most commands `capture.configure` may hold.
pub const MAX_CONFIGURE_COMMANDS: usize = 4;
/// The most arguments one `capture.configure` command may hold.
pub const MAX_CONFIGURE_ARGS: usize = 16;
/// The longest argument of a `capture.configure` command.
pub const MAX_CONFIGURE_ARG_BYTES: usize = 256;
/// The words no `.`-separated part of a `capture.configure` argument may be (compared ignoring
/// case and leading dashes): a configure command changes how the harness stores its session,
/// never its model, provider, endpoint or credentials (D61).
pub const CONFIGURE_DENIED_KEYS: &[&str] = &[
    "model", "provider", "base_url", "api_key", "key", "token", "secret",
];

/// The most characters a `participant.display_name` may hold.
pub const MAX_DISPLAY_NAME_CHARS: usize = 40;

/// The most characters a `participant.first_screen` may hold.
pub const MAX_FIRST_SCREEN_CHARS: usize = 200;

/// `[participant]`: what the participant is told about the harness, in the harness's own words,
/// so core messages never hard-code a harness name. Both strings are printed to a terminal, so
/// [`display_name_problem`] and [`first_screen_problem`] refuse anything that is not plain text.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ParticipantSpec {
    /// The harness's name as a person says it. Core code uses it in place of the connector id.
    pub display_name: String,
    /// What the harness shows first, and what to do about it (a folder-trust question, for
    /// example). Shown just before the harness opens. Absent when the harness asks nothing.
    #[serde(default)]
    pub first_screen: Option<String>,
}

/// Why `s` is not fit to print to a terminal as one line of plain text, or `None` when it is.
///
/// An allow-list, not a deny-list: only printable ASCII (0x20 to 0x7E) is accepted. That closes
/// every control, escape, invisible, bidirectional, tag and odd-space character at once, with no
/// Unicode table to keep complete. The text also has 1 to `max` characters, no leading or
/// trailing space, and never two spaces in a row.
fn plain_line_problem(s: &str, max: usize) -> Option<String> {
    let n = s.chars().count();
    if s.trim_matches(' ').is_empty() {
        return Some("must not be empty".to_string());
    }
    if n > max {
        return Some(format!("must be 1 to {max} characters; this has {n}"));
    }
    if !s.chars().all(|c| matches!(c, ' '..='~')) {
        return Some(
            "must be plain printable ASCII on one line (letters, digits, punctuation and single \
             spaces)"
                .to_string(),
        );
    }
    if s.starts_with(' ') || s.ends_with(' ') {
        return Some("must not start or end with a space".to_string());
    }
    if s.contains("  ") {
        return Some("must not hold two spaces in a row".to_string());
    }
    None
}

/// Why `s` is not a valid `participant.display_name`: 1 to 40 printable characters.
pub fn display_name_problem(s: &str) -> Option<String> {
    plain_line_problem(s, MAX_DISPLAY_NAME_CHARS)
}

/// Why `s` is not a valid `participant.first_screen`: one plain line of at most 200 characters.
pub fn first_screen_problem(s: &str) -> Option<String> {
    plain_line_problem(s, MAX_FIRST_SCREEN_CHARS)
}

/// `[capture]`: the **profile**, the whole recipe for capturing one harness.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CaptureSpec {
    /// The measured headless argument template: ends in `--` (the goal follows as one
    /// argument) unless [`Self::goal_flag`] names the slot. With [`Self::interactive`] there
    /// is no slot, and the arguments (none at all is allowed) start the harness's chat.
    pub args: Vec<String>,
    /// The harness runs in its normal chat mode in the participant's terminal and the
    /// participant types the prompt (D57): no goal slot, identity `none`.
    #[serde(default)]
    pub interactive: bool,
    /// Appended after [`Self::args`] when a run is continued, under the same rules (D51).
    #[serde(default)]
    pub resume_args: Vec<String>,
    /// Commands of the harness's own program that put a setting the capture needs into its
    /// home (D61): each an argument list, run with the harness's environment and no proxy, by
    /// `goalcup prep` after the participant's own setup and by `goalcup start` before each
    /// launch, so each must be safe to run again. Chat mode only; at most
    /// [`MAX_CONFIGURE_COMMANDS`], each held to the `args` rules, with no template token. Their
    /// output goes to a log only.
    #[serde(default)]
    pub configure: Vec<Vec<String>>,
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
    /// Name prefixes of the harness's own variables (`HERMES_`) that `env` may not set: switches
    /// such as a safe mode that would silently stop capture. Data, so no harness is named in the
    /// policy; `home.env` and `ca.env` are pinned by the launch and unaffected.
    #[serde(default)]
    pub reserved_env_prefixes: Vec<String>,
    pub executable: ExecutableSpec,
    pub home: HomeSpec,
    pub ca: CaSpec,
    pub identity: IdentitySpec,
    /// Exactly one entry in schema 1 (an array so that 1..=N is a later additive widening).
    #[serde(default)]
    pub native: Vec<NativeSpec>,
    #[serde(default)]
    pub argv_guard: Option<ArgvGuard>,
    #[serde(default)]
    pub termination: Option<TerminationSpec>,
    /// The hosts the proxy terminates TLS for. After loading this also holds the row of each
    /// [`Self::providers`] name, appended in order.
    #[serde(default)]
    pub hosts: Vec<HostSpec>,
    /// Names in the shared provider table ([`provider_hosts`]): a model provider's host row,
    /// reviewed once in core, that the loader appends to [`Self::hosts`].
    #[serde(default)]
    pub providers: Vec<String>,
    /// Per-platform replacements for env entries. Everything else is the same recipe on every
    /// platform.
    #[serde(default)]
    pub platform: BTreeMap<PlatformKey, PlatformOverride>,
}

/// `[capture.executable]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ExecutableSpec {
    /// A bare name resolved on `PATH`. Whatever build it names runs: the tracer records that
    /// build's digests (`Launch.harness_build`, D53) and pins none.
    pub program: String,
    /// The arguments that make the program print its version on the first line of stdout
    /// (`["--version"]`); empty when the harness has no such command. The launcher runs them
    /// once before the harness and records the line as `Launch.harness_version`, reported by
    /// the harness rather than proven; without them it records `unreported`.
    #[serde(default)]
    pub version_args: Vec<String>,
}

/// `[capture.platform.<key>]`: what a platform may change. Only env entries; every other
/// field, the executable included, is the base table's. The resolved recipe is validated as a
/// whole.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PlatformOverride {
    /// Entries added to `capture.env`, replacing a same-named one.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl CaptureSpec {
    /// `capture.env` for `key`: the base entries, then the override's, which replace a
    /// same-named one.
    pub fn resolved_env(&self, key: impl Into<Option<PlatformKey>>) -> BTreeMap<String, String> {
        let mut env = self.env.clone();
        if let Some(over) = key.into().and_then(|k| self.platform.get(&k)) {
            env.extend(over.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        env
    }

    /// The base recipe with `key`'s override applied and no `platform` table: what the
    /// validator sees for that platform.
    fn resolved_for(&self, key: PlatformKey) -> CaptureSpec {
        CaptureSpec {
            env: self.resolved_env(key),
            platform: BTreeMap::new(),
            ..self.clone()
        }
    }
}

/// `[capture.home]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct HomeSpec {
    pub run_home: RunHome,
    pub config_home: ConfigHome,
    /// The variable pointed at the config home. Absent for a harness with no such variable: it
    /// then keeps its state under the private run `HOME`, so its native root is `{run_home}/...`,
    /// it takes no seeds or dirs, and its config home is `per_run`.
    #[serde(default)]
    pub env: Option<String>,
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
    /// Where under the config home the harness reads its skills. Declarative: the daemon does
    /// not use it.
    #[serde(default)]
    pub skills_dir: Option<String>,
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

/// `[capture.ca]`: the variables the launch points at a certificate file, and which file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CaSpec {
    /// One or two CA variables ([`ca_env_problem`]): a [`SHARED_CA_ENV`] row, or the harness's
    /// own variable with a [`CA_ENV_SUFFIXES`] ending.
    ///
    /// [`SHARED_CA_ENV`]: crate::capture_policy::SHARED_CA_ENV
    /// [`CA_ENV_SUFFIXES`]: crate::capture_policy::CA_ENV_SUFFIXES
    pub env: Vec<String>,
    /// `extend` or `replace` ([`CaMode`]). A harness's own variable is handed the proxy's CA
    /// alone in either mode. A [`SHARED_CA_ENV`] row takes `extend` and is handed the system's
    /// roots plus the proxy's CA, so a `replace` profile never names one and stays fail-closed.
    ///
    /// [`SHARED_CA_ENV`]: crate::capture_policy::SHARED_CA_ENV
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
    /// `stdout_json` only: how long the launch waits for the identity event before recording
    /// the launch without one, 1 to [`MAX_STDOUT_WAIT_SECONDS`]. Default
    /// [`DEFAULT_STDOUT_WAIT_SECONDS`].
    #[serde(default)]
    pub wait_seconds: Option<u64>,
}

/// The default of `capture.identity.wait_seconds`: the two minutes Codex's `thread.started`
/// was measured to need at worst.
pub const DEFAULT_STDOUT_WAIT_SECONDS: u64 = 120;
/// The bound of `capture.identity.wait_seconds`.
pub const MAX_STDOUT_WAIT_SECONDS: u64 = 600;

impl IdentitySpec {
    /// How long a `stdout_json` launch waits for its identity event.
    pub fn stdout_wait(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.wait_seconds.unwrap_or(DEFAULT_STDOUT_WAIT_SECONDS))
    }
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
    /// Read only the files new or written since the run started (D57): what admits
    /// `identity.source = "none"` on a shared config home.
    #[serde(default)]
    pub since_run_start: bool,
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

/// The shared provider host table, `crates/goalcup-protocol/providers.toml`, compiled in.
const PROVIDERS_TOML: &str = include_str!("../providers.toml");

/// The shared provider host table: each provider's name and its one `[[capture.hosts]]` row
/// (WP12). A connector names rows with `capture.providers`, so a provider's host, paths and
/// hint are written once, not once per connector. The table is code-reviewed data, like the
/// pins; every host rule still applies to a row once a connector names it.
pub fn provider_hosts() -> Result<&'static BTreeMap<String, HostSpec>, ProtocolError> {
    static TABLE: std::sync::OnceLock<Result<BTreeMap<String, HostSpec>, String>> =
        std::sync::OnceLock::new();
    TABLE
        .get_or_init(|| {
            let table: BTreeMap<String, HostSpec> =
                toml::from_str(PROVIDERS_TOML).map_err(|e| e.message().to_string())?;
            match table.keys().find(|name| !is_provider_name(name)) {
                Some(bad) => Err(format!(
                    "`{bad}` is not a provider name (`^[a-z][a-z0-9-]{{0,31}}$`)"
                )),
                None => Ok(table),
            }
        })
        .as_ref()
        .map_err(|why| refuse("capture.providers", format!("the provider table {why}")))
}

fn is_provider_name(name: &str) -> bool {
    let mut b = name.bytes();
    name.len() <= 32
        && b.next().is_some_and(|c| c.is_ascii_lowercase())
        && b.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

impl CaptureSpec {
    /// Append the row of each [`Self::providers`] name to [`Self::hosts`]. An unknown or
    /// repeated name is refused.
    fn add_provider_hosts(&mut self) -> Result<(), ProtocolError> {
        if let Some(d) = has_duplicate(self.providers.iter().map(String::as_str)) {
            return Err(refuse("capture.providers", format!("`{d}` is named twice")));
        }
        if self.providers.is_empty() {
            return Ok(());
        }
        let table = provider_hosts()?;
        for name in &self.providers {
            let Some(row) = table.get(name) else {
                return Err(refuse(
                    "capture.providers",
                    format!(
                        "`{name}` is not in the shared provider table ({}); a new provider is a \
                         row in crates/goalcup-protocol/providers.toml",
                        table.keys().cloned().collect::<Vec<_>>().join(", ")
                    ),
                ));
            };
            self.hosts.push(row.clone());
        }
        Ok(())
    }
}

/// `[redaction]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RedactionSpec {
    #[serde(default)]
    pub extra_patterns: Vec<ExtraPattern>,
}

/// The longest JSON field name or header prefix a rule may name.
pub const MAX_REDACTION_NAME_BYTES: usize = 64;

/// One connector redaction rule: a typed form, never a regex. The recorder derives the regex
/// ([`Self::regex`]) and its bounded streaming recognizer from the form, so each form is
/// reviewed once in core and a connector only names its own field or header. One row is one
/// of:
///
/// * `{ kind, json_field = "plan_type", value = "string" }`: the value of one JSON field,
///   [`RedactionRule::JsonField`];
/// * `{ kind, header_prefix = "x-anthropic-billing-" }`: lowercase `name: value` text whose name
///   starts with the prefix, wherever it is recorded, [`RedactionRule::HeaderPrefix`].
///
/// A rule is checked when it is built (parse or [`Self::new`]), so an `ExtraPattern` that
/// exists is valid: its kind, field and prefix keep the grammar the recognizers rely on. Its
/// fields are private and read through [`Self::kind`] and [`Self::rule`], so nothing changes a
/// rule after the check either. A downstream crate parses rules or calls [`Self::new`], and
/// never builds one from a literal:
///
/// ```compile_fail
/// use goalcup_protocol::harness::{ExtraPattern, RedactionRule};
/// let _ = ExtraPattern { kind: "k".into(), rule: RedactionRule::HeaderPrefix("x-".into()) };
/// ```
///
/// and never changes one after the check:
///
/// ```compile_fail
/// use goalcup_protocol::harness::{ExtraPattern, RedactionRule};
/// let mut p = ExtraPattern::new("k", RedactionRule::HeaderPrefix("x-a".into())).unwrap();
/// p.rule = RedactionRule::HeaderPrefix("x.".into());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "ExtraPatternRow")]
#[non_exhaustive]
pub struct ExtraPattern {
    /// The audit kind: what every marker and audit span of this rule names.
    kind: String,
    rule: RedactionRule,
}

/// The form of one [`ExtraPattern`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RedactionRule {
    /// `"FIELD"\s*:\s*VALUE`. The span is the value alone, so the record stays JSON.
    JsonField { field: String, value: FieldValue },
    /// `PREFIX[a-z0-9-]+:\s*[^\r\n"]+`. The span is the whole `name: value` text, CR and LF
    /// excluded. No header is recorded, so this is not a header filter: it matches lowercase
    /// text anywhere in recorded bytes, not anchored to a line or word start, and it ends at a
    /// CR, an LF or a `"` (docs/CONNECTOR-SCHEMA.md#redaction).
    HeaderPrefix(String),
}

closed_enum! {
    /// The value half of a [`RedactionRule::JsonField`]. The regex group is the value.
    FieldValue, "redaction.extra_patterns.value", special: &[];
    /// `(\{(?:[^{}]|\{(?:[^{}]|\{[^{}]*\})*\})*\})`: an object nested at most three deep.
    Object = "object",
    /// `"([^"\\]+)"`: a non-empty string without escapes; the span is its interior.
    String = "string",
    /// `("[^"\\]*"|[^,}\s]+)`: a string without escapes, else a bare token (a number).
    Scalar = "scalar",
}

impl FieldValue {
    /// The regex of the value, with the span as group 1.
    fn regex(self) -> &'static str {
        match self {
            FieldValue::Object => r#"(\{(?:[^{}]|\{(?:[^{}]|\{[^{}]*\})*\})*\})"#,
            FieldValue::String => r#""([^"\\]+)""#,
            FieldValue::Scalar => r#"("[^"\\]*"|[^,}\s]+)"#,
        }
    }
}

impl ExtraPattern {
    /// A rule, or why it is refused. `kind` matches `^[a-z][a-z0-9_]{0,47}$`. A JSON field is 1
    /// to 64 bytes of `[A-Za-z0-9_-]`, and a header prefix is 1 to 64 bytes of `[a-z0-9-]`
    /// that start with a letter: plain ASCII with no regex metacharacter, no quote, backslash
    /// or line end, which is what lets the recognizer compare it byte for byte.
    pub fn new(kind: impl Into<String>, rule: RedactionRule) -> Result<ExtraPattern, String> {
        let kind = kind.into();
        let mut b = kind.bytes();
        let kind_ok = kind.len() <= 48
            && b.next().is_some_and(|x| x.is_ascii_lowercase())
            && b.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == b'_');
        if !kind_ok {
            return Err(format!(
                "redaction.extra_patterns: kind `{kind}` must match `^[a-z][a-z0-9_]{{0,47}}$`"
            ));
        }
        let (what, name, ok) = match &rule {
            RedactionRule::JsonField { field, .. } => (
                "json_field",
                field,
                field
                    .bytes()
                    .all(|x| x.is_ascii_alphanumeric() || x == b'_' || x == b'-'),
            ),
            RedactionRule::HeaderPrefix(prefix) => (
                "header_prefix",
                prefix,
                prefix
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_lowercase)
                    && prefix
                        .bytes()
                        .all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == b'-'),
            ),
        };
        if !ok || name.is_empty() || name.len() > MAX_REDACTION_NAME_BYTES {
            let grammar = if what == "json_field" {
                "[A-Za-z0-9_-]{1,64}"
            } else {
                "[a-z][a-z0-9-]{0,63}"
            };
            return Err(format!(
                "redaction.extra_patterns: `{kind}`'s {what} `{name}` must match `^{grammar}$`"
            ));
        }
        Ok(ExtraPattern { kind, rule })
    }

    /// The audit kind: what every marker and audit span of this rule names.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The rule's form and the name it carries. Read-only: the fields are private, so a rule
    /// keeps the grammar [`Self::new`] checked for as long as it exists.
    pub fn rule(&self) -> &RedactionRule {
        &self.rule
    }

    /// The regex the form stands for, in `regex` crate syntax (Unicode mode). Both of the
    /// recorder's redactors compile it; its streaming recognizer models it exactly.
    pub fn regex(&self) -> String {
        match &self.rule {
            RedactionRule::JsonField { field, value } => {
                format!(r#""{field}"\s*:\s*{}"#, value.regex())
            }
            RedactionRule::HeaderPrefix(prefix) => {
                format!(r#"{prefix}[a-z0-9-]+:\s*[^\r\n"]+"#)
            }
        }
    }
}

/// One `extra_patterns` row as written in TOML, before it is checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtraPatternRow {
    kind: String,
    #[serde(default)]
    json_field: Option<String>,
    #[serde(default)]
    value: Option<FieldValue>,
    #[serde(default)]
    header_prefix: Option<String>,
    /// The retired spelling, read only to name it in the refusal.
    #[serde(default)]
    regex: Option<String>,
}

impl TryFrom<ExtraPatternRow> for ExtraPattern {
    type Error = String;

    fn try_from(row: ExtraPatternRow) -> Result<ExtraPattern, String> {
        let kind = row.kind;
        if row.regex.is_some() {
            return Err(format!(
                "redaction.extra_patterns: `{kind}` has a `regex`, which schema 1 no longer \
                 reads: name the form instead, `json_field` with `value`, or `header_prefix` \
                 (docs/CONNECTOR-SCHEMA.md#redaction)"
            ));
        }
        let rule = match (row.json_field, row.value, row.header_prefix) {
            (Some(field), Some(value), None) => RedactionRule::JsonField { field, value },
            (None, None, Some(prefix)) => RedactionRule::HeaderPrefix(prefix),
            _ => {
                return Err(format!(
                    "redaction.extra_patterns: `{kind}` must be one form: `json_field` with \
                     `value`, or `header_prefix` alone"
                ));
            }
        };
        ExtraPattern::new(kind, rule)
    }
}

impl HarnessManifest {
    /// Every argument the launch may pass, `args`, `resume_args`, then each `configure`
    /// command's, each with its field.
    fn every_arg(&self) -> impl Iterator<Item = (&'static str, &String)> {
        let c = &self.capture;
        c.args
            .iter()
            .map(|a| ("capture.args", a))
            .chain(c.resume_args.iter().map(|a| ("capture.resume_args", a)))
            .chain(
                c.configure
                    .iter()
                    .flatten()
                    .map(|a| ("capture.configure", a)),
            )
    }

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
        let mut manifest =
            toml::from_str::<HarnessManifest>(text).map_err(|e| map_toml_error(text, &e))?;
        manifest.capture.add_provider_hosts()?;
        Ok(manifest)
    }
}

/// The keys the executable pin used to need, refused by name (D53): the tracer records the
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
                        "`{retired}` is no longer a connector field (D53): the tracer records the \
                         build that runs instead of pinning one; delete the line."
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// What a `[capture.credential]` table is told (D61). The table once named a provider key the
/// launcher read from its own environment and handed to the harness; the tracer no longer
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

// ---------------------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------------------

/// Template variables an `args` element may use. `{session_id}` additionally needs
/// `identity.source = "preassigned"` (a cross-field rule).
const ARG_TOKENS: &[&str] = &["workspace", "session_id"];

/// Template variables a `capture.env` value may use: the argument tokens, and the two homes the
/// launch makes for the run (`{config_home}`, the isolated harness home, and `{run_home}`, the
/// private `HOME`), so a harness with a second state variable can point it inside them. Each
/// renders to a tracker-made private path; the value rule still refuses a value that starts
/// with `/` or `~` or holds `..`, so a value cannot leave them.
const ENV_TOKENS: &[&str] = &["workspace", "session_id", "config_home", "run_home"];

/// A template that may name only `allowed` tokens.
fn check_template(
    field: &str,
    what: &str,
    value: &str,
    allowed: &[&str],
) -> Result<(), ProtocolError> {
    let tokens = crate::template::tokens(value).map_err(|why| {
        refuse(
            field,
            format!("{what} {why}: use `{{{{` and `}}}}` for literal braces"),
        )
    })?;
    for t in tokens {
        if !allowed.contains(&t) {
            return Err(refuse(
                field,
                format!(
                    "{what} names `{{{t}}}`, which is not a token here: only {} are",
                    allowed
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

/// One argument of `capture.args` or `capture.resume_args` under the D51 rules, refused as
/// `field`.
fn check_arg(field: &str, origin: Origin, a: &str) -> Result<(), ProtocolError> {
    if a.len() > MAX_CAPTURE_ARG_BYTES || has_control(a) {
        return Err(refuse(
            field,
            "an argument is longer than the bound or contains a control character",
        ));
    }
    // A `Dev` argument that is an absolute path is one the kit or the controlled workload
    // substituted (`@STAND_IN@`); its directory names (a checkout under `.../hook-fix/`) are
    // not flags, so the word rules do not read them.
    let substituted_path = origin == Origin::Dev && a.starts_with('/');
    if !substituted_path && let Some(why) = capture_arg_denied(a) {
        return Err(refuse(field, format!("`{a}` {why}")));
    }
    // N1: the one observable predicate. A pinned connector never needs an absolute path; the
    // controlled workload and the kit substitute them (`Origin::Dev`).
    if origin == Origin::Pinned
        && let Some(why) = arg_path_denied(a)
    {
        return Err(refuse(field, format!("`{a}` {why}")));
    }
    check_template(field, &format!("`{a}`"), a, ARG_TOKENS)
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

/// A single-component glob matcher: `*` any run (including none), `?` one character. The
/// matcher of a `[[capture.native]]` pattern's components.
pub fn glob_matches(name: &str, pattern: &str) -> bool {
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

/// The only `capture.executable.version_args` a connector may name: one of these, alone. A
/// wider grammar would let a reviewed connector run `update`, `login` or `mcp` through the
/// probe, which runs before the capture with no proxy and no record.
const VERSION_ARGS: &[&str] = &["--version", "-v", "-V", "version"];

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

/// A refusal of the recipe `key` resolves to, renamed to the override that caused it: the
/// override changes only env, so the base table already passed every other rule, and the base
/// rule's reason is kept.
fn platform_refusal(key: PlatformKey, e: ProtocolError) -> ProtocolError {
    let ProtocolError::Field { field, reason } = e else {
        return e;
    };
    let at = format!("capture.platform.{}", key.as_str());
    if field == "capture.env" {
        refuse(format!("{at}.env"), reason)
    } else {
        refuse(at, format!("{field}: {reason}"))
    }
}

impl HarnessManifest {
    /// Refuse every manifest the refusal table names. The order is fixed, so the first
    /// refusal is deterministic: executable, args and goal, env, home, CA,
    /// identity, native, argv guard, termination, hosts, the delivery-agnostic
    /// rule, the cross-field rules, and then each `[capture.platform.<key>]` in [`PlatformKey`]'s
    /// declaration order (`linux-x86_64`, `linux-aarch64`, `macos-aarch64`, `macos-x86_64`),
    /// whatever order the file uses.
    ///
    /// A platform is checked as the whole recipe it resolves to, by these same rules, so an
    /// override can reach nothing the base table could not; its refusal names
    /// `capture.platform.<key>` and keeps the base rule's reason.
    ///
    /// The CLI calls it at load and again while building a launch; the daemon calls it in
    /// `capture_harness`. Neither trusts the other's check.
    pub fn validate(&self, origin: Origin) -> Result<(), ProtocolError> {
        self.validate_recipe(origin)?;
        self.for_each_platform(|resolved| resolved.validate_recipe(origin))?;
        self.check_participant()
    }

    /// The name core messages use for this harness: `participant.display_name`, else the id.
    pub fn display_name(&self) -> &str {
        self.participant
            .as_ref()
            .map_or(self.id.as_str(), |p| p.display_name.as_str())
    }

    /// The harness's first-screen hint, when its manifest has one.
    pub fn first_screen(&self) -> Option<&str> {
        self.participant
            .as_ref()
            .and_then(|p| p.first_screen.as_deref())
    }

    /// `[participant]`: both strings reach a terminal, so they are held to plain text.
    fn check_participant(&self) -> Result<(), ProtocolError> {
        let Some(p) = &self.participant else {
            return Ok(());
        };
        if let Some(why) = display_name_problem(&p.display_name) {
            return Err(refuse("participant.display_name", why));
        }
        if let Some(why) = p.first_screen.as_deref().and_then(first_screen_problem) {
            return Err(refuse("participant.first_screen", why));
        }
        Ok(())
    }

    /// Run `check` on the recipe each `[capture.platform.<key>]` resolves to.
    fn for_each_platform(
        &self,
        check: impl Fn(&HarnessManifest) -> Result<(), ProtocolError>,
    ) -> Result<(), ProtocolError> {
        for key in self.capture.platform.keys() {
            let resolved = HarnessManifest {
                capture: self.capture.resolved_for(*key),
                ..self.clone()
            };
            check(&resolved).map_err(|e| platform_refusal(*key, e))?;
        }
        Ok(())
    }

    fn validate_recipe(&self, origin: Origin) -> Result<(), ProtocolError> {
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
        self.check_configure(origin)?;
        self.check_env()?;
        self.check_home()?;
        self.check_ca()?;
        self.check_identity()?;
        self.check_native()?;
        self.check_argv_guard()?;
        self.check_termination()?;
        self.check_hosts(origin)?;
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
        match exe.version_args.as_slice() {
            [] => {}
            [one] if VERSION_ARGS.contains(&one.as_str()) => {}
            _ => {
                return Err(refuse(
                    "capture.executable.version_args",
                    format!(
                        "must be empty or exactly one of {}: the probe runs before the capture, \
                         with no proxy and no record",
                        VERSION_ARGS
                            .iter()
                            .map(|a| format!("`{a}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
        }
        Ok(())
    }

    fn check_args_and_goal(&self, origin: Origin) -> Result<(), ProtocolError> {
        let c = &self.capture;
        // An interactive harness may open its chat with no argument at all (D57).
        if (c.args.is_empty() && !c.interactive) || c.args.len() > MAX_CAPTURE_ARGS {
            return Err(refuse(
                "capture.args",
                format!(
                    "must hold 1 to {MAX_CAPTURE_ARGS} arguments (0 to {MAX_CAPTURE_ARGS} with \
                     `interactive`)"
                ),
            ));
        }
        for a in &c.args {
            check_arg("capture.args", origin, a)?;
        }
        if c.args.len() + c.resume_args.len() > MAX_CAPTURE_ARGS {
            return Err(refuse(
                "capture.resume_args",
                format!("with `args`, holds more than {MAX_CAPTURE_ARGS} arguments"),
            ));
        }
        for a in &c.resume_args {
            check_arg("capture.resume_args", origin, a)?;
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
            if let Some(why) = capture_arg_denied(flag).or_else(|| goal_flag_denied(flag)) {
                return Err(refuse("capture.goal_flag", format!("`{flag}` {why}")));
            }
        }
        // The goal is placed exactly one way: after a closing `--`, or inside `goal_flag`. An
        // interactive profile has no slot: the participant types the prompt (D57).
        if c.interactive {
            if c.goal_flag.is_some() {
                return Err(refuse(
                    "capture.goal_flag",
                    "an `interactive` profile takes no goal: the participant types the prompt \
                     in the harness",
                ));
            }
        } else if usize::from(c.args.last().is_some_and(|a| a == "--"))
            + usize::from(c.goal_flag.is_some())
            != 1
        {
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

    /// `capture.configure` (D61): a few short commands of the harness's own program, each held
    /// to the `args` rules (the core deny policy and the pinned path rule here, the argument
    /// guard and the delivery rule with every other argument), with no template at all: a
    /// command runs before any run value exists, so it cannot name the goal, the workspace or a
    /// session.
    fn check_configure(&self, origin: Origin) -> Result<(), ProtocolError> {
        const FIELD: &str = "capture.configure";
        let commands = &self.capture.configure;
        // The commands run in the home `goalcup prep` opens and with the environment of that
        // launch, which only a chat-mode profile has (D57).
        if !commands.is_empty() && !self.capture.interactive {
            return Err(refuse(
                FIELD,
                "is for a chat-mode profile (`interactive = true`): its commands run with the \
                 environment and in the home that `goalcup prep` opens the harness with",
            ));
        }
        if commands.len() > MAX_CONFIGURE_COMMANDS {
            return Err(refuse(
                FIELD,
                format!("holds more than {MAX_CONFIGURE_COMMANDS} commands"),
            ));
        }
        for command in commands {
            if command.is_empty() || command.len() > MAX_CONFIGURE_ARGS {
                return Err(refuse(
                    FIELD,
                    format!("a command must hold 1 to {MAX_CONFIGURE_ARGS} arguments"),
                ));
            }
            for a in command {
                if a.is_empty() || a.len() > MAX_CONFIGURE_ARG_BYTES {
                    return Err(refuse(
                        FIELD,
                        format!("an argument must be 1 to {MAX_CONFIGURE_ARG_BYTES} bytes"),
                    ));
                }
                if a.contains(['{', '}']) {
                    return Err(refuse(
                        FIELD,
                        format!(
                            "`{a}` holds a brace: a configure command takes no template (no goal, \
                             workspace or session), because it runs before any run exists"
                        ),
                    ));
                }
                check_arg(FIELD, origin, a)?;
                // D61: a configure command changes how the harness stores its session, never how
                // the agent behaves or what it signs in with. Each `.`-separated part of an
                // argument is compared whole, ignoring case and leading dashes.
                if let Some(word) =
                    a.split('.')
                        .map(|part| part.trim_start_matches('-'))
                        .find(|part| {
                            CONFIGURE_DENIED_KEYS
                                .iter()
                                .any(|denied| part.eq_ignore_ascii_case(denied))
                        })
                {
                    return Err(refuse(
                        FIELD,
                        format!(
                            "`{a}` names `{word}`. A configure command may change only how the \
                             harness stores its session. It may not set the model, the \
                             provider, an address, or a key, token or secret."
                        ),
                    ));
                }
            }
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
        let reserved = &self.capture.reserved_env_prefixes;
        if reserved.len() > 4 {
            return Err(refuse(
                "capture.reserved_env_prefixes",
                "holds more than 4 prefixes",
            ));
        }
        for p in reserved {
            let mut b = p.bytes();
            let ok = (2..=32).contains(&p.len())
                && p.ends_with('_')
                && b.next().is_some_and(|x| x.is_ascii_uppercase())
                && b.all(|x| x.is_ascii_uppercase() || x.is_ascii_digit() || x == b'_');
            if !ok {
                return Err(refuse(
                    "capture.reserved_env_prefixes",
                    format!("`{p}` must match `^[A-Z][A-Z0-9_]{{0,30}}_$`"),
                ));
            }
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
            // The connector's own home variable keeps the cross-field reason, which names the
            // field that pins it, ahead of the generic home-relocator rule.
            let upper = k.to_ascii_uppercase();
            if self
                .capture
                .home
                .env
                .as_deref()
                .is_some_and(|h| h.eq_ignore_ascii_case(&upper))
            {
                return Err(refuse(
                    "capture.env",
                    format!(
                        "`{upper}` is pinned by `capture.home.env`, so `capture.env` may not set it"
                    ),
                ));
            }
            if let Some(why) = env_key_denied(k, EnvContext::CaptureEnv) {
                return Err(refuse("capture.env", format!("`{k}` {why}")));
            }
            if let Some(p) = reserved.iter().find(|p| upper.starts_with(p.as_str())) {
                return Err(refuse(
                    "capture.env",
                    format!(
                        "`{k}` is in `{p}`, the harness's own namespace, which this connector \
                         reserves (`capture.reserved_env_prefixes`)"
                    ),
                ));
            }
            if v.len() > MAX_CAPTURE_ENV_VALUE_BYTES || has_control(v) {
                return Err(refuse(
                    "capture.env",
                    format!("`{k}`'s value is longer than the bound or holds a control character"),
                ));
            }
            // Template tokens are checked on the template, before rendering.
            check_template("capture.env", &format!("`{k}`'s value"), v, ENV_TOKENS)?;
            if let Some(why) = env_value_denied(k, v) {
                return Err(refuse("capture.env", format!("`{k}`'s value {why}")));
            }
        }
        Ok(())
    }

    fn check_home(&self) -> Result<(), ProtocolError> {
        let h = &self.capture.home;
        if let Some(env) = &h.env {
            let mut b = env.bytes();
            let env_ok = env.len() <= 64
                && b.next().is_some_and(|x| x.is_ascii_uppercase())
                && b.all(|x| x.is_ascii_uppercase() || x.is_ascii_digit() || x == b'_');
            if !env_ok {
                return Err(refuse(
                    "capture.home.env",
                    format!("`{env}` must match `^[A-Z][A-Z0-9_]{{0,63}}$`"),
                ));
            }
            if let Some(why) = env_key_denied(env, EnvContext::HomeEnv) {
                return Err(refuse("capture.home.env", format!("`{env}` {why}")));
            }
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
        if let Some(d) = &h.skills_dir
            && let Some(why) = relative_path_problem(d, 4)
        {
            return Err(refuse("capture.home.skills_dir", format!("`{d}` {why}")));
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
            if let Some(why) = ca_env_problem(name, ca.mode) {
                return Err(refuse("capture.ca.env", format!("`{name}` {why}")));
            }
        }
        Ok(())
    }

    fn check_identity(&self) -> Result<(), ProtocolError> {
        let i = &self.capture.identity;
        if self.capture.interactive && i.source != IdentitySource::None {
            return Err(refuse(
                "capture.identity.source",
                "an `interactive` profile takes `none`: its sessions are the files its native \
                 source finds, not one the launch names",
            ));
        }
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
                if i.wait_seconds
                    .is_some_and(|w| !(1..=MAX_STDOUT_WAIT_SECONDS).contains(&w))
                {
                    return Err(refuse(
                        "capture.identity.wait_seconds",
                        format!("must be 1 to {MAX_STDOUT_WAIT_SECONDS} seconds"),
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
                if json.0.is_some()
                    || json.1.is_some()
                    || json.2.is_some()
                    || i.wait_seconds.is_some()
                {
                    return Err(refuse(
                        "capture.identity",
                        "`match_field`, `match_value`, `id_field` and `wait_seconds` belong to \
                         `stdout_json`, not `stderr_line`",
                    ));
                }
            }
            IdentitySource::Preassigned | IdentitySource::None => {
                if json.0.is_some()
                    || json.1.is_some()
                    || json.2.is_some()
                    || line.0.is_some()
                    || line.1.is_some()
                    || i.wait_seconds.is_some()
                {
                    return Err(refuse(
                        "capture.identity",
                        format!(
                            "`{}` takes no `match_*`, `id_field`, `prefix`, `grammar` or \
                             `wait_seconds`",
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
        for (field, a) in self.every_arg() {
            if let Some(why) = g.denies(a) {
                return Err(refuse(field, format!("`{a}` {why}")));
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

    fn check_hosts(&self, origin: Origin) -> Result<(), ProtocolError> {
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
            if origin == Origin::Pinned
                && let Some(why) = local_host_problem(a)
            {
                return Err(refuse("capture.auth_hosts", format!("`{a}` {why}")));
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
            // The kit and the controlled workload (`Origin::Dev`) serve `*.localhost` stand-ins.
            if origin == Origin::Pinned
                && let Some(why) = local_host_problem(&h.host)
            {
                return Err(refuse("capture.hosts", format!("`{}` {why}", h.host)));
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

    /// The delivery-agnostic rule (F7): a harness never names the tracker, a server or an
    /// upload. Schema 1 has no field for one, and `deny_unknown_fields` keeps it that way.
    fn check_delivery_agnostic(&self) -> Result<(), ProtocolError> {
        let c = &self.capture;
        for (field, a) in self.every_arg() {
            if mentions_goalcup(a) {
                return Err(refuse(
                    field,
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
        let run_shared = c.home.run_home == RunHome::Shared;

        // `{session_id}` and `preassigned` imply each other.
        let resume_uses_session = c.resume_args.iter().any(|a| uses_token(a, "session_id"));
        let uses_session = c.args.iter().any(|a| uses_token(a, "session_id"))
            || resume_uses_session
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
                    if resume_uses_session && !args_use_session {
                        "capture.resume_args"
                    } else {
                        "capture.args"
                    },
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

        // `none` must read a per-run root, or only what was written since the run started:
        // another run's sessions are not this run's, and runs on a shared home are serialized.
        let native = &c.native[0];
        let (root_home, root_rel) = split_native_root(&native.root).expect("checked by native");
        let root_shared = match root_home {
            RootHome::Config => !per_run,
            RootHome::Run => run_shared,
        };
        if i.source == IdentitySource::None && root_shared && !native.since_run_start {
            return Err(refuse(
                "capture.native.root",
                "with `identity.source = \"none\"` the root must be per run: `{run_home}/...` \
                 with a per-run `run_home`, or `{config_home}/...` with `config_home = \
                 \"per_run\"`, unless the native source declares `since_run_start = true`; an \
                 unscoped source in a shared home would read other runs' sessions",
            ));
        }

        // A shared run `HOME` is for a harness with no home variable (D59). With one, the
        // harness's state is the config home's to share, and two persistent homes would leave
        // which one holds the login unclear.
        if run_shared && h.env.is_some() {
            return Err(refuse(
                "capture.home.run_home",
                "`shared` is for a harness without `capture.home.env`: one with a home variable \
                 shares its config home (`config_home = \"shared\"`) instead",
            ));
        }

        // Without `home.env` nothing points the harness at the config home, so anything the
        // manifest puts there, or expects to read from there, would never be used. Refused
        // rather than admitted to record nothing.
        if h.env.is_none() {
            const WHY: &str = "requires `capture.home.env`: without it nothing points the harness \
                               at the config home, so the harness keeps its state under the \
                               private run `HOME`";
            if !per_run {
                return Err(refuse(
                    "capture.home.config_home",
                    format!("`shared` {WHY}"),
                ));
            }
            for (field, present) in [
                ("seed", !h.seed.is_empty()),
                ("dirs", !h.dirs.is_empty()),
                ("per_run_dir", h.per_run_dir.is_some()),
            ] {
                if present {
                    return Err(refuse(format!("capture.home.{field}"), WHY));
                }
            }
            if root_home == RootHome::Config {
                return Err(refuse(
                    "capture.native.root",
                    format!("a `{{config_home}}` root {WHY}: use `{{run_home}}/...`"),
                ));
            }
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
        let mut pinned: Vec<(&str, String)> = Vec::new();
        if let Some(env) = &c.home.env {
            pinned.push(("capture.home.env", env.to_ascii_uppercase()));
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
            if let Some(why) = seed_content_problem(&bytes) {
                return Err(refuse("capture.home.seed", format!("`{}` {why}", s.file)));
            }
            Ok(LoadedSeed {
                file: s.file.clone(),
                mode: s.mode(),
                bytes,
            })
        })
        .collect::<Result<_, _>>()?;
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
