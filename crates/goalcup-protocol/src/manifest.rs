//! Connector manifest, schema v0. Declarative TOML: all harness knowledge that is
//! not a parser lives here.

use crate::ProtocolError;
use crate::connector_dir::hash_connector_dir_listing;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The connector-directory digest and the exact `manifest.toml` bytes that contributed to
/// it. The daemon uses this result to compile trusted foreign scans: parsing a second path
/// read after accepting the directory digest could authorize bytes that were never hashed.
pub fn connector_dir_sha256_with_manifest(dir: &Path) -> Result<(String, Vec<u8>), ProtocolError> {
    let (sha256, manifest) = hash_connector_dir(dir)?;
    let manifest = manifest.ok_or_else(|| ProtocolError::Field {
        field: "connector_dir".to_string(),
        reason: format!("{} contains no regular manifest.toml", dir.display()),
    })?;
    Ok((sha256, manifest))
}

fn hash_connector_dir(dir: &Path) -> Result<(String, Option<Vec<u8>>), ProtocolError> {
    let mut listing = hash_connector_dir_listing(dir, |path| path == "manifest.toml")?;
    Ok((listing.sha256, listing.captured.remove("manifest.toml")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Full,
    ElevatedReview,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorType {
    Ordinal,
    Rowid,
    SqlitePk,
    SessionSeq,
    InodeOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KillStrategy {
    ProcessGroup,
    NativePid,
    WrapperUnsafe,
}

/// How a store's bytes are laid out on disk. An unknown `mode` fails at load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreMode {
    /// One JSON object per line, appended.
    Jsonl,
    /// A directory the daemon walks; each file is a record source.
    Dir,
    /// A table in a SQLite database. `Store.table` names it.
    Sqlite,
}

/// How a resume launch names the session it resumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionArgStyle {
    /// `codex resume <id>`.
    Positional,
    /// `--resume <id>`, `--session <id>`.
    Flag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    /// The harness owns the terminal. The CLI spawns it in the foreground and must not
    /// put it in a new process group, or it loses the tty (SIGTTIN on the first read).
    Tui,
    /// The harness is a headless local server the participant then talks to over a socket.
    ///
    /// **No V1 connector launches this way.** A participant launched this way has nothing to
    /// type into. The variant is for a harness that is genuinely headless, and for the one
    /// thing it decides: such a launch may own its process group, because it has no tty to
    /// lose. `hermes-cli` names it only because `[launch]` requires a mode and its
    /// capture-only manifest must carry one; the capture launch never reads it, and Hermes is
    /// not a socket server.
    Server,
    /// Desktop attach mode: nothing is spawned; the folder is armed and the participant
    /// opens a session in it (D7).
    AttachOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallScope {
    /// Inside the tracker-created workspace. The agent can read it; that is expected.
    Workspace,
    /// Inside the artifact root, which is never an ancestor of the workspace.
    ArtifactRoot,
    /// Inside the dedicated per-run harness home. Never the participant's real home.
    HarnessHome,
}

/// How a store's rows are separated from foreign ones at capture time (D20).
/// `Store.partition_key` keeps the human-readable name the verifier reports; this
/// carries the strategy the daemon executes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum PartitionSpec {
    /// The record itself carries the value: compare `path` against the tracked set.
    Direct { path: String },
    /// The file's first matching record decides for the whole file (Codex `session_meta`).
    FileFirstRecord {
        select_path: String,
        select_value: String,
        value_path: String,
    },
    /// One record type teaches an aggregate's value; every other record inherits it
    /// through `aggregate_path`.
    AggregateLearned {
        aggregate_path: String,
        teach_select_path: String,
        teach_select_value: String,
        value_path: String,
        /// The store the teaching record lives in, when it is not this one. Without this
        /// field `AggregateLearned` cannot express a strategy where a store partitions on a
        /// record type it never sees. `None` means the teaching record is in this store.
        #[serde(default)]
        teach_store: Option<String>,
    },
}

/// One predicate over a leg-1 user-slot record, used by the positive test (D18). `field`
/// is a dotted path into the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "match", rename_all = "snake_case")]
pub enum MatchRule {
    FieldEquals { field: String, value: String },
    FieldPresent { field: String },
    FieldTrue { field: String },
    FieldAbsentOrNull { field: String },
}

/// The named scoping clauses of D18. Each is implemented once in the verifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopingClause {
    ParentIdIsNull,
    DelegateFromIsNull,
    NotSubagentThreadSource,
    PastSubagentHistoryStartOrdinal,
    DirectoryEqualsWorkspace,
    FirstSessionMetaIsTracked,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanRuleSpec {
    pub id: String,
    #[serde(default)]
    pub positive: Vec<MatchRule>,
    #[serde(default)]
    pub scoping: Vec<ScopingClause>,
    /// Which leg-2 field is byte-exact against the typed text.
    pub byte_exact_field: String,
    /// The leg-2 hook event that carries a submitted prompt.
    pub leg2_event: String,
    /// The identifier both legs stamp on the same submission.
    pub pair_key: String,
    // The exclusion-catalogue keys are accepted and ignored so recorded V1 manifests still
    // parse and stay bound to their receipt's manifest_sha256; shipped manifests carry
    // neither key.
    #[serde(default, rename = "exclusions", skip_serializing)]
    pub legacy_exclusions: Option<LegacyIgnored>,
    #[serde(default, rename = "reportable_exclusions", skip_serializing)]
    pub legacy_reportable_exclusions: Option<LegacyIgnored>,
}

/// Which tool names count as a **write** for seal attribution (D11).
///
/// D11 says a sealed file must be explained by "a traced edit, write, or shell command"; a
/// `Read`, a `grep` or an `ls` explains nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolClasses {
    /// Tools that can create or modify a file.
    #[serde(default)]
    pub write: Vec<String>,
    /// The subset of `write` that runs a shell command. A shell call attributes a file only
    /// when the file's workspace-relative path appears in the command text.
    #[serde(default)]
    pub shell: Vec<String>,
}

/// Where to look for sessions belonging to another installed harness (D10).
///
/// The paths named here are deliberately **outside** the per-run pins: an unpinned
/// installation under the participant's real `{home}` is exactly where a second session
/// working in a bound workspace would land.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForeignScan {
    /// The harness whose stores these are — may be this connector or another.
    pub harness: String,
    /// Path glob, in the `path_template` token syntax.
    pub path_glob: String,
    /// Dotted field holding the session's working directory.
    pub directory_field: String,
    /// Dotted field holding the session identifier.
    pub session_id_field: String,
    /// How the declared source represents one session identity.
    #[serde(default)]
    pub reader: ForeignScanReader,
    /// Which entry-owned root may render this declaration.
    #[serde(default)]
    pub root: ForeignScanRoot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ForeignScanReader {
    FileFirstRecord {},
    /// The first complete pair of declared identity fields within a bounded JSONL prefix.
    ///
    /// Claude Code writes session bookkeeping before the first record that also names its
    /// working directory.  This is deliberately separate from [`Self::FileFirstRecord`],
    /// whose one-line semantics remain the partition-compatible default for Codex.
    FileFirstIdentity {},
    SqliteSession {
        table: String,
        session_id_column: String,
        directory_column: String,
    },
}

impl Default for ForeignScanReader {
    fn default() -> Self {
        Self::FileFirstRecord {}
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForeignScanRoot {
    #[default]
    Home,
    OwnConfigDir,
}

/// Connector-declared secret patterns beyond the shared set
/// (`goalcup_daemon::redact::patterns`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionSpec {
    #[serde(default)]
    pub extra_patterns: Vec<RedactionPattern>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionPattern {
    pub kind: String,
    pub regex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Normalisation {
    /// Strip exactly one trailing newline before any byte comparison (V7).
    pub trailing_newline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    #[serde(rename = "type")]
    pub kind: CursorType,
    /// The column or dotted record path the cursor reads. `rowid` for a rowid cursor,
    /// `payload.ordinal` for Codex, `id` for a `sqlite_pk`, `seq` for a `session_seq`.
    #[serde(default)]
    pub column: Option<String>,
    /// For `session_seq` only: the column that groups the sequence (`session_id`).
    #[serde(default)]
    pub group_column: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Store {
    pub name: String,
    pub mode: StoreMode,
    pub path_template: String,
    pub cursor: Cursor,
    /// SQLite only: the table this store reads. One database can hold several stores.
    #[serde(default)]
    pub table: Option<String>,
    /// The human-readable name of the partition, which V10 reports (D20).
    #[serde(default)]
    pub partition_key: Option<String>,
    /// The strategy the daemon executes to resolve that name to a value on a row.
    #[serde(default)]
    pub partition: Option<PartitionSpec>,
    /// Why this store needs no `partition_key`. Exactly one of `partition_key` and this
    /// must be present, or `validate()` refuses the manifest.
    ///
    /// A manifest defect belongs in the loader, stated once by the connector author, not
    /// re-derived from bytes by every verification.
    #[serde(default)]
    pub unpartitioned_reason: Option<String>,
    #[serde(default)]
    pub subagent_path_template: Option<String>,
    #[serde(default)]
    pub spill_path_template: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detection {
    pub binary: String,
    pub version_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arming {
    /// The event that proves leg 2 is live for the tracked workspace (D16).
    pub signal: String,
    /// **Arming-timeout precedence, normative.** This is the connector's default. The
    /// daemon's own config may override it per machine
    /// (`goalcup_daemon::config::DaemonConfig.arming_timeout_seconds`). The CLI's
    /// `--arming-timeout` flag, carried as
    /// `control::WorkspaceOpts.arming_timeout_seconds`, overrides both, per start.
    /// The arming gate refuses on whichever number won; health only ever reports
    /// `arming_pending`. Nothing else restates this rule; it references it.
    pub timeout_seconds: u64,
    /// `recipe`: accepted and ignored so recorded V1 manifests still parse and stay bound to
    /// their receipt's manifest_sha256.
    #[serde(default, rename = "recipe", skip_serializing)]
    pub legacy_recipe: Option<LegacyIgnored>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kill {
    pub strategy: KillStrategy,
    /// The `comm` of the process that actually holds the session, for `native_pid`.
    /// D22: `kill -9` on the Codex npm wrapper left the native binary holding the lock.
    #[serde(default)]
    pub native_binary: Option<String>,
    /// Seconds between SIGTERM and SIGKILL at seal.
    #[serde(default = "default_grace")]
    pub grace_seconds: u64,
}

fn default_grace() -> u64 {
    10
}

/// The Claude Code status line (D25). Written into the `--settings` file as
/// `{"type":"command","command":<rendered>}` so the line carries no secret: the
/// command is a `cat` of a file the daemon rewrites on every state change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusLine {
    pub settings_key: String,
    pub command_template: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub mode: LaunchMode,
    /// Resolved on `PATH` at preflight; never a path baked into the manifest.
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Process-scoped pins. Values are templates; keys are literal.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Variables removed from the inherited environment before spawning, because they
    /// switch capture off or redirect the model (`RULES.md`, D1).
    #[serde(default)]
    pub env_remove: Vec<String>,
    /// Arguments and subcommands a participant may not add (`RULES.md`).
    #[serde(default)]
    pub banned_args: Vec<String>,
    /// The environment variable that relocates the harness's home, if it has one.
    #[serde(default)]
    pub home_env: Option<String>,
    #[serde(default)]
    pub status_line: Option<StatusLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resume {
    /// The full argument vector for a resume launch, templated like `Launch.args`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Recorded so `goalcup doctor` can explain the difference without re-reading argv.
    pub session_arg_style: SessionArgStyle,
}

/// One folder-scoped file the **daemon** renders and writes (D5).
/// The CLI writes none of these; it is told where they landed on `WorkspaceInfo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookInstall {
    /// Path relative to `scope`'s root, forward slashes. Templated.
    pub path: String,
    /// File name under `connectors/<id>/files/`. Its body is templated with the same
    /// `{name}` syntax.
    pub template: String,
    pub scope: InstallScope,
    #[serde(default = "default_file_mode")]
    pub mode: u32,
    /// Written only when the connector's primary discovery path fails.
    #[serde(default)]
    pub optional: bool,
    /// Restoring this file invalidates the harness's hook trust, so a restore raises
    /// `arming_lost` rather than assuming capture continues (D6, Codex).
    #[serde(default)]
    pub restore_invalidates_trust: bool,
    /// When set, the SHA-256 of the *other* installed file in the same scope is exposed
    /// to this file's template under this variable name and pinned into the shim, so a
    /// hot-reload deletion surfaces as `config_mismatch` on the next post.
    #[serde(default)]
    pub pin_hash_as: Option<String>,
    /// The **harness** owns this file, not the tracker: write it only when it is not
    /// already there, and then never watch it, never restore it, never chain a deviation
    /// for it.
    ///
    /// # Why a connector needs this
    ///
    /// `$CODEX_HOME/config.toml` is Codex's own configuration file, and Codex writes to it
    /// while it runs: `[tui.model_availability_nux]` is a per-model counter for a tip it
    /// shows a few times, keyed by the model slug the account resolves at launch, which no
    /// manifest can pre-write because nothing knows it in advance. The tracker seeds two
    /// settings into that same file (directory trust and the update check), and a
    /// tracker-owned file is watched and restored — so Codex writes its counter, the watch
    /// wipes it, and Codex writes it again on the next launch. That is one
    /// `ConfigDeviation` and one `unattributed_config_change` **Fail per launch, on every
    /// honest run, for the life of the connector**: the verifier crying wolf on the one
    /// check that catches a real config swap. The restore also re-shows the tip modal, and
    /// a modal has already eaten a submit Enter and cost an entry.
    ///
    /// # What it costs
    ///
    /// One bounded blind spot, written down in `docs/CHEAT-CHECKLIST.md`: an unwatched
    /// `config.toml` can be edited mid-run without chaining an event.
    ///
    /// # Write-if-absent, not overwrite
    ///
    /// Overwriting would be one branch cheaper and would wipe the counter on every launch,
    /// which is the modal again. Leaving a file the home already holds is also what
    /// `[home_seed]` already does for `auth.json` in this same directory, and says so on
    /// the start line.
    ///
    /// The digest such a file is disclosed under on `WorkspaceInfo.installed_files` is
    /// therefore what the tracker RENDERED, which is what it wrote when it wrote at all.
    /// Nothing compares that digest against the bytes on disk — the config watch is the
    /// only thing that would, and this flag is what takes the file off its list.
    #[serde(default)]
    pub harness_owned: bool,
}

fn default_file_mode() -> u32 {
    0o644
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preflight {
    /// Environment variables whose mere presence refuses the start (`RULES.md`).
    #[serde(default)]
    pub banned_env: Vec<String>,
    /// Paths that must exist and be writable, templated.
    #[serde(default)]
    pub required_paths: Vec<String>,
    /// Ports the launch recipe needs free.
    #[serde(default)]
    pub free_ports: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionPolicy {
    pub self_updating: bool,
    pub sample_interval_seconds: u64,
    pub record_field: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppWideException {
    pub file: String,
    pub key: String,
    pub expected_diff: String,
    pub reversal: String,
    pub disclosure: String,
}

/// What the V1 `goalcup start` and `goalcup resume` copied out of the participant's own
/// app-wide harness configuration into the per-run home, before the harness was spawned.
/// D48 removed that seeding: a capture launch carries no home seed, and the daemon refuses
/// one that does.
///
/// # Why a connector needs this at all
///
/// A per-run home is a **fresh** configuration directory, and a harness that keeps its
/// first-run state there — "has this person been onboarded", "which theme", "who is signed
/// in" — meets a brand-new user on every single start: a theme picker and then a login
/// wall, on a machine whose participant has been signed in for months. Nothing in the run
/// is worth anything if the first minute of the trace is onboarding UI, and no participant
/// can be asked to log in again per entry.
///
/// # It is a READ of the app-wide file, and never a write
///
/// D21 lists the app-wide **writes** the tracker permits, each with a reversal procedure.
/// This is not one of them and never becomes one: the files named in [`HomeSeed::from`] are
/// opened read-only, the keys named in [`HomeSeed::keys`] are copied out, and the only file
/// written is [`HomeSeed::to`] inside the per-run harness home the daemon created for this
/// entry. A connector cannot express a write here — there is no field for one — and the
/// CLI's seeding path never opens a `{home}` path for writing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeSeed {
    /// Where the participant's real configuration is, in **candidate order**: the first one
    /// that exists and carries every [`HomeSeed::required`] key is the one read.
    ///
    /// A list and not a single path because one harness can keep the same document in two
    /// places across versions — Claude Code writes `~/.claude.json` by default and
    /// `<CLAUDE_CONFIG_DIR>/.claude.json` when that variable is set, and a machine that has
    /// been through both carries both files with only one of them current. Every entry must
    /// begin `{home}/`; `validate()` refuses any other token, so a connector cannot point
    /// this at the workspace, at the artifact root, or at an absolute path of its choosing.
    ///
    /// `serde(default)`, with `to` and `keys`: a connector that copies
    /// whole files and merges no keys declares none of the three, and `validate()` refuses
    /// any other combination of them.
    #[serde(default)]
    pub from: Vec<String>,
    /// The file to write, **relative to the per-run harness home**. Not a template: the
    /// destination root is never the connector's to choose. `validate()` refuses an absolute
    /// path, a `..` segment and a `{token}`. `serde(default)` — see [`HomeSeed::from`].
    #[serde(default)]
    pub to: String,
    /// The keys copied, in the order a reader should think about them. A key the source file
    /// does not carry is skipped rather than written as null — a harness reading an explicit
    /// null is not a harness reading an absent key. `serde(default)` — see [`HomeSeed::from`].
    #[serde(default)]
    pub keys: Vec<String>,
    /// The subset of [`HomeSeed::keys`] without which seeding is pointless, so `goalcup
    /// start` refuses instead of launching into an onboarding screen. Must be a subset of
    /// `keys`: a key nothing copies cannot be a key the run requires.
    #[serde(default)]
    pub required: Vec<String>,
    /// The field in the destination document that is keyed by DIRECTORY — Claude Code's
    /// `projects` — under which [`HomeSeed::trust_keys`] are written for this run's own
    /// workspace.
    ///
    /// # Why this is not a copy, and not a decision about anybody's other directories
    ///
    /// Claude Code asks a per-project question the first time it opens a directory ("Is this
    /// a project you created or one you trust?"), and its default answer is **No, exit**.
    /// No key copied out of the participant's file can answer it: every run's workspace is a
    /// path that did not exist when that file was written. The tracker created that
    /// directory itself, empty, for this run and this run alone, so it is the one
    /// directory whose trust question the tracker can answer without speaking for anybody.
    ///
    /// It is written **only** for this entry's workspace, **only** into the per-run home,
    /// never for another path and never into the participant's own file. `None` for a
    /// connector whose harness asks no such question.
    #[serde(default)]
    pub trust_field: Option<String>,
    /// The booleans set to `true` under `trust_field[<this run's workspace>]`. Empty unless
    /// `trust_field` is declared, and refused when it is not.
    #[serde(default)]
    pub trust_keys: Vec<String>,
    /// The DIRECTORY under `{home}` that [`HomeSeed::files`] are copied out of, e.g.
    /// `{home}/.codex`. Validated exactly as [`HomeSeed::from`] is: `{home}` and no other
    /// token, rooted at `{home}/`, no `..`. Declared with `files` or not at all.
    #[serde(default)]
    pub from_dir: Option<String>,
    /// WHOLE FILES copied verbatim out of [`HomeSeed::from_dir`] into the per-run harness
    /// home, at the same relative name, mode `0600`.
    ///
    /// # Why a whole file, when everything else here is a key
    ///
    /// Codex keeps its credential in `$CODEX_HOME/auth.json`, and `CODEX_HOME` is the
    /// per-run home the daemon mints — so a fresh entry meets a sign-in screen on a machine
    /// whose participant signed in months ago. That file is not a document to merge keys
    /// out of; it is one file, and copying part of it would produce something the harness
    /// cannot read. It is **the participant's own credential, on the participant's own
    /// machine, copied into a directory the tracker made for their own run**, and it is
    /// never uploaded, never chained and never named in a trace. The alternative a
    /// participant may prefer is in README and DOGFOOD: sign in inside the entry's own home
    /// with `CODEX_HOME=<entry home> codex login`.
    ///
    /// The copy is skipped where the destination already exists — on a resume the harness
    /// may have refreshed its own token, and this must not put an older one back — and a
    /// declared file that is not on the machine is reported on the seeding line rather than
    /// refused: an unseeded home meets a sign-in screen a person can still answer, and
    /// refusing the start would take the choice away.
    ///
    /// Additive and `serde(default)`: a connector that declares none copies none.
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorManifest {
    pub id: String,
    pub family: String,
    pub tier: Tier,
    /// D12: `=X` or `>=X`, the version(s) the connector was tested at. Not a pin:
    /// `X` is a floor below which preflight refuses, and a version above an `=`
    /// range is allowed and reported as untested. The grammar and the three-way answer live
    /// in `goalcup_cli::version`.
    pub tested_version_range: String,
    pub detection: Detection,
    pub arming: Arming,
    pub kill: Kill,
    pub launch: Launch,
    pub resume: Resume,
    #[serde(default)]
    pub preflight: Preflight,
    /// The folder-scoped files the daemon writes into the workspace, the artifact root
    /// and the per-run harness home. Rendered by the daemon, never by the CLI.
    #[serde(default)]
    pub hook_install: Vec<HookInstall>,
    pub version: VersionPolicy,
    pub normalisation: Normalisation,
    /// Required by V1; an absent section is the all-empty default, which is never serialized.
    #[serde(default, skip_serializing_if = "HumanRuleSpec::is_unset")]
    pub human_rule: HumanRuleSpec,
    #[serde(default)]
    pub join_key: Option<String>,
    #[serde(default)]
    pub store: Vec<Store>,
    #[serde(default)]
    pub unsafe_cursors: Vec<String>,
    #[serde(default)]
    pub lock_paths: Vec<String>,
    #[serde(default)]
    pub config_allowlist: Vec<String>,
    #[serde(default)]
    pub app_wide_exception: Option<AppWideException>,
    /// What the CLI copies out of the participant's real configuration into the per-run home
    /// before the harness is spawned, so a fresh home is not a first-run one.
    /// Additive and `serde(default)`: a connector that declares none is not seeded.
    #[serde(default)]
    pub home_seed: Option<HomeSeed>,
    /// Paths the seal does not list, matched by [`crate::seal::is_excluded`].
    ///
    /// **Workspace-root-anchored, and NOT a glob.** A pattern matches a path it equals or
    /// is a leading component-prefix of; `**/` at the front opts into any depth. So
    /// `.opencode/node_modules` is right and `.opencode/node_modules/` matches nothing —
    /// which `validate()` refuses, because the failure is otherwise silent and lands on an
    /// honest run's seal.
    #[serde(default)]
    pub workspace_exclusions: Vec<String>,
    #[serde(default)]
    pub archive_sources: Vec<String>,
    /// Which tools count as a write for seal attribution (D11).
    #[serde(default)]
    pub tool_classes: ToolClasses,
    /// Where to look for another installed harness's sessions.
    #[serde(default)]
    pub foreign_scan: Vec<ForeignScan>,
    /// Connector-declared secret patterns beyond the shared set
    /// (`goalcup_daemon::redact::patterns`).
    #[serde(default)]
    pub redaction: RedactionSpec,
    /// Heartbeat interval override; the daemon falls back to
    /// `DaemonConfig.heartbeat_seconds` (60). The verifier reads the same number when it
    /// classifies a hole in the heartbeat series.
    #[serde(default)]
    pub heartbeat_seconds: Option<u64>,
    /// Historical capture keys are accepted only so a recorded V1 manifest remains readable.
    /// Their values are deliberately uninterpreted and never serialized.
    #[serde(default, rename = "capture", skip_serializing)]
    pub legacy_capture: Option<LegacyIgnored>,
    #[serde(default, rename = "capture_only", skip_serializing)]
    pub legacy_capture_only: Option<LegacyIgnored>,
    // The hook-leg sections: each one is accepted and ignored so recorded V1 manifests still
    // parse and stay bound to their receipt's manifest_sha256. Shipped manifests carry none
    // of them; any other unknown key is still refused by `deny_unknown_fields`.
    #[serde(default, rename = "start_signal", skip_serializing)]
    pub legacy_start_signal: Option<LegacyIgnored>,
    #[serde(default, rename = "blocking", skip_serializing)]
    pub legacy_blocking: Option<LegacyIgnored>,
    #[serde(default, rename = "receiver", skip_serializing)]
    pub legacy_receiver: Option<LegacyIgnored>,
    #[serde(default, rename = "hook_silence_policy", skip_serializing)]
    pub legacy_hook_silence_policy: Option<LegacyIgnored>,
    #[serde(default, rename = "leg2_partition", skip_serializing)]
    pub legacy_leg2_partition: Option<LegacyIgnored>,
}

impl HumanRuleSpec {
    /// The absent section: every field empty, and it is then not written back out.
    pub fn is_unset(&self) -> bool {
        self == &HumanRuleSpec::default()
    }
}

/// A manifest key this build accepts and ignores so recorded V1 manifests still parse and
/// stay bound to their receipt's manifest_sha256.
///
/// Whatever the key held is read and dropped ([`serde::de::IgnoredAny`]); nothing in this
/// build reads it back, and it is never serialised.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LegacyIgnored;

impl<'de> Deserialize<'de> for LegacyIgnored {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        serde::de::IgnoredAny::deserialize(d)?;
        Ok(LegacyIgnored)
    }
}

/// The complete token set any **path** template may use.
///
/// `workspace` is the tracker-created working directory; `home` is the participant's real
/// home, used only where a connector must baseline an app-wide file it did not write;
/// `config_dir` is the harness's per-run configuration root — the directory
/// `launch.home_env` points at (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`), or, for a harness with
/// no single home variable, the per-run harness home the daemon created;
/// `artifact_root` is the run's artifact directory; `slug`, `session_id`, `yyyy`, `mm`,
/// `dd` are per-session and per-date components.
///
/// No manifest ever names an environment variable. The daemon resolves the roots first and
/// then renders, so a participant cannot move a tailed store by exporting a variable.
///
/// `path_tokens_used()` must return a subset of this list or `validate()` fails.
pub const PATH_TOKENS: &[&str] = &[
    "workspace",
    "home",
    "config_dir",
    "artifact_root",
    "slug",
    "session_id",
    "yyyy",
    "mm",
    "dd",
];

impl ConnectorManifest {
    pub fn from_toml_str(s: &str) -> Result<ConnectorManifest, ProtocolError> {
        Ok(toml::from_str(s)?)
    }

    pub fn store_named(&self, name: &str) -> Option<&Store> {
        self.store.iter().find(|s| s.name == name)
    }

    /// D19's real rule: `unsafe_cursors[]` names **cursors**, not stores. An entry
    /// matches when it equals the cursor's column, the cursor's type name, or either of
    /// those qualified by the store name (`<store>.<column>`, `<store>.<type>`).
    ///
    /// The naive check — "is the store's *name* on the list" — passes for the wrong
    /// reason for a manifest that declares `unsafe_cursors = ["event.seq"]` and names a
    /// store `event`, and would reject a perfectly safe store called `event.seq`.
    pub fn cursor_is_unsafe(&self, s: &Store) -> bool {
        let kind = cursor_type_name(s.cursor.kind);
        let mut candidates = vec![kind.to_string(), format!("{}.{kind}", s.name)];
        if let Some(col) = &s.cursor.column {
            candidates.push(col.clone());
            candidates.push(format!("{}.{col}", s.name));
        }
        self.unsafe_cursors
            .iter()
            .any(|u| candidates.iter().any(|c| c == u))
    }

    /// Every `{name}` a launch, resume, env, preflight or hook-install-**path** template
    /// refers to. A test asserts the set is a subset of what the tracker supplies, so a
    /// manifest can never introduce a placeholder no code fills in.
    ///
    /// It does not scan the hook-install **bodies** under `connectors/<id>/files/`. The
    /// body-only variable `entry_id` therefore appears in the supplied list and never in
    /// this set.
    pub fn template_vars_used(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for a in &self.launch.args {
            scan_braces(a, &mut out);
        }
        for v in self.launch.env.values() {
            scan_braces(v, &mut out);
        }
        for a in &self.resume.args {
            scan_braces(a, &mut out);
        }
        if let Some(sl) = &self.launch.status_line {
            scan_braces(&sl.command_template, &mut out);
        }
        for p in &self.preflight.required_paths {
            scan_braces(p, &mut out);
        }
        for h in &self.hook_install {
            scan_braces(&h.path, &mut out);
        }
        out
    }

    /// Every `{name}` a **path** template refers to: the store paths, the lock paths, the
    /// app-wide config allowlist and the foreign-harness scan globs.
    ///
    /// `validate()` refuses any token outside `PATH_TOKENS`, so a typo is an error and not a
    /// silently non-matching glob, and `PathVars` fills exactly those nine names.
    pub fn path_tokens_used(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for s in &self.store {
            scan_braces(&s.path_template, &mut out);
            if let Some(p) = &s.subagent_path_template {
                scan_braces(p, &mut out);
            }
            if let Some(p) = &s.spill_path_template {
                scan_braces(p, &mut out);
            }
        }
        for p in &self.lock_paths {
            scan_braces(p, &mut out);
        }
        for p in &self.config_allowlist {
            scan_braces(p, &mut out);
        }
        // `ForeignScan.path_glob` says in terms that it is in this token syntax, so it is
        // validated by the same rule rather than by a second one.
        for f in &self.foreign_scan {
            scan_braces(&f.path_glob, &mut out);
        }
        out
    }

    /// Refuses a manifest that omits something D18/D19/D20/D22 make mandatory.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        let field = |f: &str, r: &str| ProtocolError::Field {
            field: f.to_string(),
            reason: r.to_string(),
        };
        if self.id.is_empty() {
            return Err(field("id", "must not be empty"));
        }
        if self.family.is_empty() {
            return Err(field("family", "must not be empty"));
        }
        if self.store.is_empty() {
            return Err(field("store", "at least one store must be declared"));
        }
        if self.human_rule.byte_exact_field.is_empty() {
            return Err(field(
                "human_rule.byte_exact_field",
                "must name a leg-2 field",
            ));
        }
        if self.human_rule.pair_key.is_empty() {
            return Err(field(
                "human_rule.pair_key",
                "must name the two-leg join key",
            ));
        }
        // Uniqueness. Each of these two lists is looked up **by key** somewhere —
        // `store_named()`, the daemon's install walk — and a
        // duplicate key means the lookup silently returns the first of two rows while the
        // author believes both are in force.
        unique(
            "store",
            self.store.iter().map(|s| s.name.as_str()),
            "store names must be unique: `store_named` returns the first match, so a \
             duplicate silently disables the second declaration",
        )?;
        unique(
            "hook_install",
            self.hook_install.iter().map(|h| h.path.as_str()),
            "hook_install paths must be unique: two rows writing one path means the \
             second silently overwrites the first",
        )?;
        for s in &self.store {
            if self.cursor_is_unsafe(s) {
                return Err(field(
                    "unsafe_cursors",
                    &format!(
                        "store `{}` reads a cursor named in unsafe_cursors[] (D19)",
                        s.name
                    ),
                ));
            }
            if s.mode == StoreMode::Sqlite && s.table.is_none() {
                return Err(field("store.table", "a sqlite store must name its table"));
            }
            // A cursor with no column is a cursor the tailer cannot advance: all five
            // types read a named column or dotted record path, and the manifest is the
            // only place that name exists.
            if s.cursor.column.is_none() {
                return Err(field(
                    "store.cursor.column",
                    &format!(
                        "store `{}` declares a `{}` cursor and no cursor.column; every \
                         cursor type reads a named column or record path",
                        s.name,
                        cursor_type_name(s.cursor.kind),
                    ),
                ));
            }
            if s.cursor.kind == CursorType::SessionSeq && s.cursor.group_column.is_none() {
                return Err(field(
                    "store.cursor.group_column",
                    &format!(
                        "store `{}` reads a session_seq cursor, which is unique per \
                         session only, so it must name the group_column that scopes it",
                        s.name
                    ),
                ));
            }
            if s.partition_key.is_some() && s.partition.is_none() {
                return Err(field(
                    "store.partition",
                    "a store that names a partition_key must declare the strategy the daemon runs (D20)",
                ));
            }
            // D20: a store either says how its rows are separated from foreign ones, or says
            // in words why they need no separating. Silence is the one answer not allowed.
            if s.partition_key.is_some() == s.unpartitioned_reason.is_some() {
                return Err(field(
                    "store.partition_key",
                    &format!(
                        "store `{}` must declare exactly one of partition_key and \
                         unpartitioned_reason (D20)",
                        s.name
                    ),
                ));
            }
            // A `teach_store` names the store the teaching record lives in, and the
            // daemon has to open that store to learn the value before it can partition
            // this one. A name no `[[store]]` declares is a strategy nothing can execute:
            // the daemon would find no teacher, learn no directory, and — silently — keep
            // or drop every row of this store on a partition value it never resolved.
            if let Some(PartitionSpec::AggregateLearned {
                teach_store: Some(t),
                ..
            }) = &s.partition
                && !self.store.iter().any(|o| &o.name == t)
            {
                return Err(field(
                    "store.partition.teach_store",
                    &format!(
                        "store `{}` names teach_store `{t}`, which is not a store this \
                         manifest declares",
                        s.name
                    ),
                ));
            }
        }
        if self.kill.strategy == KillStrategy::NativePid && self.kill.native_binary.is_none() {
            return Err(field(
                "kill.native_binary",
                "D22: a native_pid strategy must name the binary that holds the session",
            ));
        }
        if self.launch.mode != LaunchMode::AttachOnly && self.launch.program.is_empty() {
            return Err(field(
                "launch.program",
                "a launch recipe must name a program",
            ));
        }
        if self.launch.program.contains('/') {
            return Err(field(
                "launch.program",
                "must be a bare program name resolved on PATH at preflight, not a path",
            ));
        }
        for k in self.launch.env.keys() {
            if k.ends_with("_BASE_URL") {
                return Err(field(
                    "launch.env",
                    "D1: the tracker never sets a model base URL",
                ));
            }
        }
        for h in &self.hook_install {
            if h.template.contains('/') || h.template.contains("..") {
                return Err(field(
                    "hook_install.template",
                    "a template names one file under connectors/<id>/files/, not a path",
                ));
            }
            // `path` was rendered under `h.scope`'s root with no filesystem check of its own
            // (the V1 renderer, removed by OS-31, joined it straight on). An absolute
            // path ignores that root entirely and a `..` segment walks out of it, so both
            // are refused here rather than trusted to every connector author.
            if h.path.starts_with('/') {
                return Err(field(
                    "hook_install.path",
                    &format!(
                        "`{}` is absolute; it must be relative to its scope's root",
                        h.path
                    ),
                ));
            }
            if h.path.split('/').any(|seg| seg == "..") {
                return Err(field(
                    "hook_install.path",
                    &format!("`{}` walks out of its scope's root with `..`", h.path),
                ));
            }
            // `config_allowlist[]` is never restored and `hook_install[]` always is, so the
            // same file on both lists is a manifest asking the daemon to do two contradictory
            // things to one file.
            //
            // `config_allowlist[]` entries are full path templates (they may carry a leading
            // `{workspace}`, `{home}` or `{config_dir}` scope token), while
            // `hook_install[].path` is always scope-relative — the V1 renderer (removed by
            // OS-31) joined it onto `h.scope`'s root without ever seeing a token. Comparing
            // the raw strings therefore misses an overlap such as
            // `{workspace}/.opencode/plugin/goalcup-tracker.ts` against
            // `.opencode/plugin/goalcup-tracker.ts`: they name the same file and differ only
            // by that anchor. `allowlist_entry_names_hook_install_path` strips exactly the
            // token `h.scope` maps to (never any of the other two — see its own doc comment
            // for why that distinction is load-bearing) before comparing; the untokenised
            // exact match is still covered, since a raw `h.path` with no leading token
            // strips nothing and compares equal to itself unchanged.
            if self
                .config_allowlist
                .iter()
                .any(|a| a == &h.path || allowlist_entry_names_hook_install_path(a, h))
            {
                return Err(field(
                    "config_allowlist",
                    &format!(
                        "`{}` is on config_allowlist[] and is also a hook_install[] path; \
                         it cannot be both never-restored and restore-on-deviation (D20/D21)",
                        h.path
                    ),
                ));
            }
        }
        // Every path template token must be one of the nine the tracker fills.
        for token in self.path_tokens_used() {
            if !PATH_TOKENS.contains(&token.as_str()) {
                return Err(field(
                    "path_template",
                    &format!(
                        "refers to {{{token}}}, which is not one of PATH_TOKENS ({})",
                        PATH_TOKENS.join(", ")
                    ),
                ));
            }
        }
        // A shell tool must also be a write tool, or attribution would consult a
        // command text for a call it has already refused to consider.
        for s in &self.tool_classes.shell {
            if !self.tool_classes.write.contains(s) {
                return Err(field(
                    "tool_classes.shell",
                    &format!("`{s}` is a shell tool but is not in tool_classes.write"),
                ));
            }
        }
        // An exclusion pattern that cannot match anything is refused at load.
        //
        // `seal::is_excluded` is workspace-root-anchored: a pattern matches a path when it
        // EQUALS it or is a leading path prefix of it, and the byte after the pattern must
        // be `/`. So `.opencode/node_modules/` — the spelling a person writes without
        // thinking, because that is how a directory is written everywhere else — matches
        // nothing at all. Without a load error the cost lands on an honest run: a seal
        // listing every harness-written file under that directory, each one failing
        // attribution.
        //
        // The same defect `PATH_TOKENS` exists to prevent one field over: a typo produces a
        // silently non-matching glob rather than an error.
        for p in &self.workspace_exclusions {
            let reason = if p.is_empty() {
                Some("is empty, and an empty pattern excludes nothing")
            } else if p == "**/" {
                Some("is `**/` with no remainder, so it names no path component")
            } else if p.ends_with('/') {
                Some(
                    "ends in `/`; patterns are path-anchored, so the trailing slash makes it \
                     match nothing — write the directory name without it",
                )
            } else {
                None
            };
            if let Some(reason) = reason {
                return Err(field("workspace_exclusions", &format!("`{p}` {reason}")));
            }
        }
        // The seed reads an app-wide file, so what it may name is checked here
        // rather than trusted to the CLI that renders it: `from` may say `{home}` and
        // nothing else, and `to` is a plain relative name under the per-run harness home.
        if let Some(seed) = &self.home_seed {
            // A seed copies keys, or whole files, or both — and a seed that copies neither
            // leaves the harness unonboarded, which is what this section exists to prevent.
            if (seed.keys.is_empty() && seed.files.is_empty())
                || seed.keys.iter().any(|k| k.is_empty())
            {
                return Err(field(
                    "home_seed.keys",
                    "must name at least one non-empty key, or at least one whole file in \
                     `files`; a seed that copies nothing is a seed that leaves the harness \
                     unonboarded",
                ));
            }
            // `from`, `to` and `keys` are one clause and are declared together: `from` is
            // where the keys are read and `to` is the document they are merged into, and
            // neither means anything without them.
            if seed.keys.is_empty() != (seed.from.is_empty() && seed.to.is_empty()) {
                return Err(field(
                    "home_seed.keys",
                    "`keys`, `from` and `to` are the key-merge clause and are declared \
                     together or not at all: keys with no source have nowhere to come from, \
                     and a source with no keys copies nothing out of it",
                ));
            }
            if !seed.keys.is_empty() && seed.from.is_empty() {
                return Err(field(
                    "home_seed.from",
                    "must name at least one candidate under `{home}`",
                ));
            }
            for candidate in &seed.from {
                // The token check and the prefix check are separate sentences because they
                // refuse different mistakes: `{workspace}/x` names a legal token at an
                // illegal root, and `/etc/x` names no token at all.
                let mut tokens = BTreeSet::new();
                scan_braces(candidate, &mut tokens);
                if tokens.iter().any(|t| t != "home") {
                    return Err(field(
                        "home_seed.from",
                        &format!(
                            "`{candidate}` names a token other than {{home}}; the seed reads \
                             the participant's own configuration and nothing else"
                        ),
                    ));
                }
                if !candidate.starts_with("{home}/") {
                    return Err(field(
                        "home_seed.from",
                        &format!(
                            "`{candidate}` must begin `{{home}}/`, so a connector cannot \
                             point the read at a path of its own choosing"
                        ),
                    ));
                }
                if candidate.split('/').any(|seg| seg == "..") {
                    return Err(field(
                        "home_seed.from",
                        &format!("`{candidate}` walks out of {{home}} with `..`"),
                    ));
                }
            }
            let mut to_tokens = BTreeSet::new();
            scan_braces(&seed.to, &mut to_tokens);
            if !seed.keys.is_empty()
                && (seed.to.is_empty()
                    || seed.to.starts_with('/')
                    || seed.to.split('/').any(|seg| seg == "..")
                    || !to_tokens.is_empty())
            {
                return Err(field(
                    "home_seed.to",
                    &format!(
                        "`{}` must be a relative path under the per-run harness home, with \
                         no `..` segment and no template token",
                        seed.to
                    ),
                ));
            }
            for r in &seed.required {
                if !seed.keys.contains(r) {
                    return Err(field(
                        "home_seed.required",
                        &format!("`{r}` is required and is not one of home_seed.keys"),
                    ));
                }
            }
            // The two halves of the workspace-trust clause are useless apart: a field with
            // nothing to write under it writes an empty object, and keys with no field to
            // write them under have nowhere to go. Either both or neither.
            match (&seed.trust_field, seed.trust_keys.is_empty()) {
                (Some(f), false) if !f.is_empty() => {}
                (None, true) => {}
                _ => {
                    return Err(field(
                        "home_seed.trust_field",
                        "trust_field and trust_keys are declared together or not at all: a \
                         field with no keys writes an empty object, and keys with no field \
                         have nowhere to go",
                    ));
                }
            }
            if seed.trust_keys.iter().any(|k| k.is_empty()) {
                return Err(field("home_seed.trust_keys", "a key must not be empty"));
            }
            // The whole-file clause, checked the way the key clause above is:
            // `from_dir` may say `{home}` and nothing else, and a `files[]` entry is a
            // plain relative name on both sides of the copy.
            match (&seed.from_dir, seed.files.is_empty()) {
                (Some(_), false) | (None, true) => {}
                _ => {
                    return Err(field(
                        "home_seed.from_dir",
                        "from_dir and files are declared together or not at all: a directory \
                         with no files copies nothing, and files with no directory have \
                         nowhere to come from",
                    ));
                }
            }
            if let Some(dir) = &seed.from_dir {
                let mut tokens = BTreeSet::new();
                scan_braces(dir, &mut tokens);
                if tokens.iter().any(|t| t != "home")
                    || !dir.starts_with("{home}/")
                    || dir.split('/').any(|seg| seg == "..")
                {
                    return Err(field(
                        "home_seed.from_dir",
                        &format!(
                            "`{dir}` must begin `{{home}}/`, name no token but {{home}}, and \
                             not walk out with `..`: the seed reads the participant's own \
                             configuration and nothing else"
                        ),
                    ));
                }
            }
            for f in &seed.files {
                let mut tokens = BTreeSet::new();
                scan_braces(f, &mut tokens);
                if f.is_empty()
                    || f.starts_with('/')
                    || f.split('/').any(|seg| seg == "..")
                    || !tokens.is_empty()
                {
                    return Err(field(
                        "home_seed.files",
                        &format!(
                            "`{f}` must be a relative name under home_seed.from_dir, with no \
                             `..` segment and no template token"
                        ),
                    ));
                }
            }
        }
        for f in &self.foreign_scan {
            if f.harness.is_empty()
                || f.path_glob.is_empty()
                || f.directory_field.is_empty()
                || f.session_id_field.is_empty()
            {
                return Err(field(
                    "foreign_scan",
                    "R8: a scan must name the harness, the glob, the directory field and \
                     the session id field",
                ));
            }
            if let ForeignScanReader::SqliteSession {
                table,
                session_id_column,
                directory_column,
            } = &f.reader
            {
                for (name, value) in [
                    ("table", table),
                    ("session_id_column", session_id_column),
                    ("directory_column", directory_column),
                ] {
                    if !valid_sql_identifier(value) {
                        return Err(field(
                            "foreign_scan.reader",
                            &format!(
                                "sqlite_session {name} `{value}` must match \
                                 [A-Za-z_][A-Za-z0-9_]*"
                            ),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

fn valid_sql_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The snake_case spelling of a cursor type, as the manifest writes it.
fn cursor_type_name(k: CursorType) -> &'static str {
    match k {
        CursorType::Ordinal => "ordinal",
        CursorType::Rowid => "rowid",
        CursorType::SqlitePk => "sqlite_pk",
        CursorType::SessionSeq => "session_seq",
        CursorType::InodeOffset => "inode_offset",
    }
}

/// Refuse a list whose keys are not unique, naming the first repeated key.
fn unique<'a>(
    what: &str,
    keys: impl Iterator<Item = &'a str>,
    reason: &str,
) -> Result<(), ProtocolError> {
    let mut seen = BTreeSet::new();
    for k in keys {
        if !seen.insert(k) {
            return Err(ProtocolError::Field {
                field: what.to_string(),
                reason: format!("`{k}` is declared twice; {reason}"),
            });
        }
    }
    Ok(())
}

/// Collect every `{name}` in `s` into `out`. `{{` is a literal brace and is skipped, so
/// this is the same lexer `template::render` uses and the two can never disagree about
/// what counts as a variable.
fn scan_braces(s: &str, out: &mut BTreeSet<String>) {
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '{' {
            if i + 1 < b.len() && b[i + 1] == '{' {
                i += 2;
                continue;
            }
            if let Some(end) = b[i + 1..].iter().position(|c| *c == '}') {
                out.insert(b[i + 1..i + 1 + end].iter().collect::<String>());
                i += end + 2;
                continue;
            }
        }
        i += 1;
    }
}

/// The path-template token a `hook_install[].path` is implicitly anchored at, given its
/// `scope` — the same mapping the V1 renderer (removed by OS-31) used to pick which root to
/// join the path onto (`InstallScope::Workspace` -> `{workspace}`, `ArtifactRoot` ->
/// `{artifact_root}`, `HarnessHome` -> `{config_dir}`, per `PATH_TOKENS`'s doc comment:
/// "config_dir ... the per-run harness home the daemon created").
fn scope_token(scope: InstallScope) -> &'static str {
    match scope {
        InstallScope::Workspace => "workspace",
        InstallScope::ArtifactRoot => "artifact_root",
        InstallScope::HarnessHome => "config_dir",
    }
}

/// Is `allowlist_entry` the same file as `hook.path`, once the allowlist
/// entry's own scope token is accounted for?
///
/// `hook_install[].path` is always scope-relative; `config_allowlist[]` entries are full
/// path templates that name their own scope with a leading token — `{workspace}`,
/// `{home}` or `{config_dir}` (see `PATH_TOKENS`). Comparing the raw strings therefore
/// misses a real overlap like `{workspace}/.opencode/plugin/goalcup-tracker.ts` against
/// `.opencode/plugin/goalcup-tracker.ts` (scope `workspace`) — the same file, differing
/// only by that anchor.
///
/// The token stripped must be the ONE `hook.scope` maps to, not any of the three: `{home}`
/// names the participant's real home, which no `hook_install[]` scope ever resolves to
/// (`InstallScope` has no `Home` variant), so `{home}/.claude/settings.json` on
/// `config_allowlist` must never be read as "the same file" as a `scope = "workspace"`
/// hook whose relative path happens to also be `.claude/settings.json` — they are two
/// different files at two different roots that only coincide as strings once you strip
/// the wrong token. Stripping any-of-three, rather than the scope-matched one, is exactly
/// the false positive this function exists to not produce.
fn allowlist_entry_names_hook_install_path(allowlist_entry: &str, hook: &HookInstall) -> bool {
    let prefix = format!("{{{}}}/", scope_token(hook.scope));
    allowlist_entry
        .strip_prefix(prefix.as_str())
        .is_some_and(|rest| rest == hook.path)
}
