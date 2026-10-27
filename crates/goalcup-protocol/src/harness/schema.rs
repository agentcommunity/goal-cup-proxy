//! The typed manifest: the closed-set values, the `harness.toml` structs and the redaction
//! rule forms.

use super::NEWER_SENTENCE;
use super::parse::refuse;
use crate::ProtocolError;
use crate::capture_policy::{CaMode, has_duplicate};
use serde::Deserialize;
use std::collections::BTreeMap;

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

/// What a manifest written for headless mode is told. Every harness runs in its chat mode in
/// the participant's terminal (D57): the participant types the prompt, the launch names no
/// session, and the sessions are the files the native source finds.
pub const CHAT_MODE_ONLY: &str = "chat mode only: Goal Cup Proxy runs every harness in its chat \
     mode (`interactive = true`, `identity.source = \"none\"`), where the participant types the \
     prompt; headless mode, with its goal slot and session identities, was removed";

/// The `capture.identity.source` values of headless mode, each told [`CHAT_MODE_ONLY`]: the
/// one list the typed decode and the generic-table refusal both read.
pub(super) const HEADLESS_IDENTITY_SOURCES: &[(&str, &str)] = &[
    ("preassigned", CHAT_MODE_ONLY),
    ("stdout_json", CHAT_MODE_ONLY),
    ("stderr_line", CHAT_MODE_ONLY),
];

closed_enum! {
    /// Where the run's session identity comes from. Chat mode names none: the sessions are the
    /// files the native source finds. The headless sources are refused by name, here and
    /// before deserialization.
    IdentitySource, "capture.identity.source",
        special: HEADLESS_IDENTITY_SOURCES;
    /// The harness has no session identity the launch names.
    None = "none",
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
    /// What runs in the harness home just before each launch (D72). Optional; see
    /// [`LaunchPrepSpec`].
    #[serde(default)]
    pub launch_prep: Option<LaunchPrepSpec>,
}

/// `[launch_prep]` (D72): what `goalcup start` and each `goalcup continue` do in the harness
/// home after `capture.configure` and before the launch, and after the harness exits, for a
/// harness that keeps a process of its own between launches (a gateway) or trusts a certificate
/// file only through its own config. Never run by `goalcup prep`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LaunchPrepSpec {
    /// A file name in the config home that is given the run's recording-proxy CA (alone, PEM,
    /// mode 0600) at every start and continue, written through a temporary file and a rename,
    /// so a link at that name is replaced, never followed. `{ca_file}` names its absolute path.
    #[serde(default)]
    pub ca_file: Option<String>,
    /// Commands of the harness's own program, run as `capture.configure` runs them, in order,
    /// after it. Held to the `capture.configure` rules, plus the two templates
    /// [`LAUNCH_PREP_TOKENS`], each a whole argument right after the key it fills. A command
    /// that fails is written to `host.log` and never stops the run.
    #[serde(default)]
    pub commands: Vec<Vec<String>>,
    /// Commands of the harness's own program, run as `commands` run, in order, each time the
    /// harness exits (D72, 0.7.1). After its own quit, a crash or a signal they run before
    /// goalcup waits for the recording proxy to drain: a gateway the harness left running would
    /// hold the proxy's connections open. After goalcup's stop at the deadline they run once the
    /// proxy was stopped within its short bound and the part sealed, and only end what the
    /// harness left. Held to the `capture.configure` rules, with no template at all. A command
    /// that fails or runs too long is written to `host.log` and never stops the run; goalcup then
    /// stops the proxy within its short bound instead of waiting on those connections.
    #[serde(default)]
    pub after_exit: Vec<Vec<String>>,
}

/// The template `[launch_prep]` commands may name for the CA file's absolute path.
pub const LAUNCH_PREP_CA_FILE: &str = "{ca_file}";
/// The template `[launch_prep]` commands may name for the model service's URL as goalcup read
/// it (`capture.model_host_from`): `https://`, the host, the port unless 443, and the path.
pub const LAUNCH_PREP_MODEL_BASE_URL: &str = "{model_base_url}";
/// The two `[launch_prep]` templates (D72), each with the deny-list word (lower case, no `-` or
/// `_`) the deny lists then admit in the key argument before it, and the last `.`-separated part
/// that key must have, exactly. The key holds no `=`, so it carries no value of its own.
pub const LAUNCH_PREP_TOKENS: &[(&str, &str, &str)] = &[
    (LAUNCH_PREP_MODEL_BASE_URL, "baseurl", "base_url"),
    (LAUNCH_PREP_CA_FILE, "cacert", "ssl_ca_cert"),
];

/// The most commands `capture.configure` may hold.
pub const MAX_CONFIGURE_COMMANDS: usize = 4;
/// The most arguments one `capture.configure` command may hold.
pub const MAX_CONFIGURE_ARGS: usize = 16;
/// The longest argument of a `capture.configure` command.
pub const MAX_CONFIGURE_ARG_BYTES: usize = 256;
/// The words no `.`- or `=`-separated part of a `capture.configure` argument may be (compared
/// ignoring case and leading dashes): a configure command changes how the harness stores its
/// session, never its model, provider, endpoint or credentials (D61).
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
    /// The settings in the harness's own config that make it run without permission prompts
    /// (D67): a harness that asks stalls, because the participant may type only "keep going".
    /// `goalcup prep` reads them in the shared config home and, when one is not set to an
    /// accepted value, recommends it with a link to [`Self::setup_section`]. Goal Cup never
    /// writes them. At most [`MAX_NO_PROMPT_SETTINGS`].
    #[serde(default)]
    pub no_prompts: Vec<NoPromptSetting>,
    /// The anchor of this harness's section in the participant's setup guide,
    /// `docs/HARNESS-SETUP.md` (fix plan v8), for example `codex`. `goalcup prep` links there,
    /// so core code names no harness. Held to [`setup_section_problem`].
    #[serde(default)]
    pub setup_section: Option<String>,
    /// Where the participant adds Goal Cup's time-left tool, `goalcup mcp` (fix plan v9), in the
    /// harness's own config. `goalcup prep` prints the lines with goalcup's absolute path filled
    /// in. Goal Cup never writes them (D67).
    #[serde(default)]
    pub time_left_tool: Option<TimeLeftTool>,
}

/// `[participant.time_left_tool]` (fix plan v9): the harness's config file that lists its MCP
/// servers, and how the harness hands a server `GOALCUP_TIME_LEFT_FILE`. The snippet's shape
/// follows the file's extension (`.toml`, `.json`, `.yaml`/`.yml`), so core code names no harness.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TimeLeftTool {
    /// The config file, relative to the home `goalcup prep` opens the harness in (its shared
    /// config home, else its shared run `HOME`). Held to the rules of `participant.no_prompts`.
    pub file: String,
    /// The key of the file's table of MCP servers. TOML reads it as dotted table names; in JSON
    /// and YAML it is one top-level key, written as it is (`amp.mcpServers` is one JSON key).
    pub key: String,
    /// How the harness passes `GOALCUP_TIME_LEFT_FILE` to the server.
    #[serde(default)]
    pub pass_env: PassEnv,
}

/// How a harness hands an MCP server it starts the variables of its own environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PassEnv {
    /// The server gets the harness's whole environment: the snippet adds nothing.
    #[default]
    Inherited,
    /// The server gets only variables named in an `env_vars` list: the snippet names it there.
    EnvVars,
    /// The server gets only the variables of an `env` map, whose values may be `${VAR}`: the
    /// snippet maps it to `${GOALCUP_TIME_LEFT_FILE}`.
    EnvTemplate,
}

/// The most characters a `participant.setup_section` may hold.
pub const MAX_SETUP_SECTION_CHARS: usize = 40;

/// The most settings `participant.no_prompts` may list.
pub const MAX_NO_PROMPT_SETTINGS: usize = 4;
/// The most accepted values one `participant.no_prompts` setting may list.
pub const MAX_NO_PROMPT_VALUES: usize = 4;
/// The longest accepted value of a `participant.no_prompts` setting.
pub const MAX_NO_PROMPT_VALUE_CHARS: usize = 64;

/// One `[[participant.no_prompts]]` row: a scalar at `key` in `file` (in the shared config home)
/// that is one of `values` when the harness runs without permission prompts. Read like
/// [`ModelHostFrom`], and held to its file and key rules.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct NoPromptSetting {
    /// The config file, relative to the shared config home; `.yaml`, `.yml`, `.json` or `.toml`.
    pub file: String,
    /// The setting's key, its table names joined with `.`.
    pub key: String,
    /// The values that mean "no prompt", as text (`true` for a boolean); the first is the one
    /// recommended.
    pub values: Vec<String>,
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

/// Why `s` is not a valid `participant.no_prompts` value: one plain line of at most 64
/// characters (it is printed in prep's recommendation).
pub fn no_prompt_value_problem(s: &str) -> Option<String> {
    plain_line_problem(s, MAX_NO_PROMPT_VALUE_CHARS)
}

/// Why `s` is not a valid `participant.setup_section`: a heading anchor as GitHub writes it,
/// 1 to [`MAX_SETUP_SECTION_CHARS`] lower-case letters, digits and single `-` between them, so
/// it goes into a link unchanged.
pub fn setup_section_problem(s: &str) -> Option<String> {
    let n = s.chars().count();
    if n == 0 || n > MAX_SETUP_SECTION_CHARS {
        return Some(format!(
            "must be 1 to {MAX_SETUP_SECTION_CHARS} characters; this has {n}"
        ));
    }
    let shaped = s.split('-').all(|part| {
        !part.is_empty() && part.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9'))
    });
    (!shaped).then(|| {
        "must be a heading anchor: lower-case letters and digits, joined by single `-`".to_string()
    })
}

/// `[capture]`: the **profile**, the whole recipe for capturing one harness.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CaptureSpec {
    /// The arguments (none at all is allowed) that start the harness's chat. There is no goal
    /// slot: the participant types the prompt (D57).
    pub args: Vec<String>,
    /// Must be `true`: the harness runs in its normal chat mode in the participant's terminal
    /// (D57). Anything else is refused ([`CHAT_MODE_ONLY`]); the field stays so that every
    /// manifest says so.
    #[serde(default)]
    pub(super) interactive: bool,
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
    /// The hosts the recording proxy terminates TLS for. After loading this also holds the row of
    /// each [`Self::providers`] name, appended in order.
    #[serde(default)]
    pub hosts: Vec<HostSpec>,
    /// Names in the shared provider table ([`provider_hosts`]): a model provider's host row,
    /// reviewed once in core, that the loader appends to [`Self::hosts`].
    #[serde(default)]
    pub providers: Vec<String>,
    /// Where the harness's own config names its model service (D62). `goalcup start` and each
    /// `goalcup continue` read that one value, and its host is intercepted for that run beside
    /// [`Self::hosts`]. With it, `hosts` may be empty.
    #[serde(default)]
    pub model_host_from: Option<ModelHostFrom>,
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
    /// A bare name resolved on `PATH`. Whatever build it names runs: goalcup records that
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
    pub(super) fn resolved_for(&self, key: PlatformKey) -> CaptureSpec {
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
    /// [`ca_env_problem`]: crate::capture_policy::ca_env_problem
    pub env: Vec<String>,
    /// `extend` or `replace` ([`CaMode`]). A harness's own variable is handed the recording proxy's
    /// CA alone in either mode. A [`SHARED_CA_ENV`] row takes `extend` and is handed the system's
    /// roots plus the recording proxy's CA, so a `replace` profile never names one and stays
    /// fail-closed.
    ///
    /// [`SHARED_CA_ENV`]: crate::capture_policy::SHARED_CA_ENV
    pub mode: CaMode,
}

/// `[capture.identity]`: `source = "none"`, the one value chat mode has.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct IdentitySpec {
    pub source: IdentitySource,
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
    ///
    /// [`valid_hint`]: crate::hints::valid_hint
    #[serde(default)]
    pub protocol_hint: Option<String>,
    /// Read only the files new or written since the run started (D57): what admits
    /// `identity.source = "none"` on a shared config home.
    #[serde(default)]
    pub since_run_start: bool,
    /// Whether `goalcup status` says so when model calls are recorded and no file of this
    /// source is (the "no session file" line). Default `true`; `false` for a harness whose
    /// supported versions include ones that write no such file (D72). The file is still read
    /// whenever it is written.
    #[serde(default = "yes")]
    pub warn_missing: bool,
}

fn yes() -> bool {
    true
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

/// `[[capture.hosts]]`: one host the recording proxy intercepts, and the request paths it records.
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

/// `capture.model_host_from`: the one value in the harness's own config that names its model
/// service, as a URL (D62). Goal Cup Proxy reads only this key, at start and at each continue.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ModelHostFrom {
    /// The config file, relative to the shared config home. Its extension says how it is read:
    /// [`MODEL_HOST_FILE_EXTENSIONS`].
    pub file: String,
    /// The value's key, its table names joined with `.` (`model.base_url`).
    pub key: String,
}

/// The file extensions `capture.model_host_from.file` may have: YAML, JSON or TOML.
pub const MODEL_HOST_FILE_EXTENSIONS: &[&str] = &["yaml", "yml", "json", "toml"];

/// The most `.`-separated parts `capture.model_host_from.key` may have.
pub const MAX_MODEL_HOST_KEY_PARTS: usize = 8;

/// Words that no `.`, `-` or `_` separated part of the model-host file's name, and no part of
/// its key, may be (any case): goalcup reads a service address, never a key file or a key.
pub(super) const MODEL_HOST_DENIED_WORDS: &[&str] = &[
    "auth",
    "credential",
    "credentials",
    "secret",
    "secrets",
    "token",
    "tokens",
    "key",
    "keys",
    "api_key",
    "env",
];

/// The shared provider host table, `crates/goalcup-protocol/providers.toml`, compiled in.
const PROVIDERS_TOML: &str = include_str!("../../providers.toml");

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
    pub(super) fn add_provider_hosts(&mut self) -> Result<(), ProtocolError> {
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
    /// command's and each `launch_prep` command's (`commands`, then `after_exit`), each with its
    /// field.
    pub(super) fn every_arg(&self) -> impl Iterator<Item = (&'static str, &String)> {
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
            .chain(
                self.launch_prep
                    .iter()
                    .flat_map(|p| p.commands.iter().flatten())
                    .map(|a| ("launch_prep.commands", a)),
            )
            .chain(
                self.launch_prep
                    .iter()
                    .flat_map(|p| p.after_exit.iter().flatten())
                    .map(|a| ("launch_prep.after_exit", a)),
            )
    }

    /// Whether a missing native file is worth a `PROBLEM:` line: some native source has
    /// `warn_missing` on (D72).
    pub fn warns_missing_native(&self) -> bool {
        self.capture.native.iter().any(|n| n.warn_missing)
    }

    /// The termination spec with its defaults applied.
    pub fn termination(&self) -> TerminationSpec {
        self.capture.termination.clone().unwrap_or_default()
    }
}
