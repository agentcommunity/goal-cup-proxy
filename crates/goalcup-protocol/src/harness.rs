//! The harness manifest, `connectors/<id>/harness.toml`, schema 1 (harness contract v1, S2;
//! `docs/DECISIONS.md` D42, D43).
//!
//! A harness is data: this file is its whole capture recipe, and its `[capture]` table is the
//! **profile** ([`CaptureSpec`]). The module defines the types, the loader that reads one
//! connector directory, and [`HarnessManifest::validate`], which refuses every manifest the
//! refusal table names. The trust tables it checks against are in [`crate::capture_policy`]; a
//! manifest can only be checked against them, never widen them.
//!
//! "Schema 1" is this file's format. A connector carries no `manifest.toml` (D48), and the
//! loader refuses one. Every caller loads a connector through [`load_dir`], the one loader.
//!
//! # Parsing
//!
//! 1. The text is parsed as a generic TOML table and `schema_version` is read first
//!    (`SUPPORTED_SCHEMA_VERSIONS`). The executable pin's retired keys (`sha256`, `version`) are
//!    refused here with their own sentence (D53), before the strict deserialization would call
//!    them a newer schema's, and so is a `[capture.credential]` table (D61). So is every trace of
//!    headless mode ([`CHAT_MODE_ONLY`]): `interactive` other than `true`, `goal_flag`, and the
//!    identity fields beyond `source = "none"`, in the base recipe or a platform override.
//! 2. It is then deserialized with `deny_unknown_fields` everywhere. An unknown key, and an
//!    unknown value of a closed-set field, are refused with one sentence ([`NEWER_SENTENCE`]):
//!    serde's own "unknown variant" text never reaches the user.
//! 3. [`HarnessManifest::validate`] runs the checks in a fixed order, so the first refusal
//!    is deterministic: executable, args, env, home, CA,
//!    native, argv guard, termination, model host, hosts, the delivery-agnostic rule (F7), the
//!    cross-field rules, and then every rule again on the recipe each `[capture.platform.<key>]`
//!    resolves to (an override may replace only env entries).
//!
//! [`Origin`] is the one observable predicate that relaxes anything: `Dev` (an unpinned
//! directory under `GOALCUP_CONNECTORS_DIR` loaded with `--dev`, for kit cases and the
//! controlled workload) skips the interpreter refusal, the absolute-path `args` refusal and the
//! loopback-host refusal (the kit's stand-ins are `*.localhost`). Env values, seeds, the deny
//! lists and every cross-field rule stay strict for both.

mod check;
mod load;
mod parse;
mod schema;

pub use check::{MAX_NATIVE_ROWS, glob_matches};
pub use load::{
    LoadedConnector, LoadedSeed, discover_connector_dirs, load_dir, load_dir_resolving_origin,
    load_discovered_dir,
};
pub use parse::CREDENTIAL_REFUSAL;
pub use schema::{
    ArgvGuard, CAPTURE_HOMES_DIR, CHAT_MODE_ONLY, CONFIGURE_DENIED_KEYS, CaSpec, CaptureSpec,
    ConfigHome, ExecutableSpec, ExtraPattern, FieldValue, HarnessManifest, HomeSpec, HostSpec,
    IdentitySource, IdentitySpec, LAUNCH_PREP_CA_FILE, LAUNCH_PREP_MODEL_BASE_URL,
    LAUNCH_PREP_TOKENS, LaunchPrepSpec, MAX_CONFIGURE_ARG_BYTES, MAX_CONFIGURE_ARGS,
    MAX_CONFIGURE_COMMANDS, MAX_DISPLAY_NAME_CHARS, MAX_FIRST_SCREEN_CHARS,
    MAX_MODEL_HOST_KEY_PARTS, MAX_NO_PROMPT_SETTINGS, MAX_NO_PROMPT_VALUE_CHARS,
    MAX_NO_PROMPT_VALUES, MAX_REDACTION_NAME_BYTES, MAX_SETUP_SECTION_CHARS,
    MODEL_HOST_FILE_EXTENSIONS, MODEL_HOST_VIA_PART, ModelHostAbsent, ModelHostFrom, NativeForm,
    NativeSpec, NoPromptSetting, Origin, ParticipantSpec, PassEnv, PlatformKey, PlatformOverride,
    RedactionRule, RedactionSpec, RunHome, SeedFile, SeedMode, Strategy, TerminationSpec,
    TimeLeftTool, display_name_problem, first_screen_problem, provider_hosts,
    setup_section_problem,
};

/// The schema versions this build reads. Reader policy: these only.
pub(crate) const SUPPORTED_SCHEMA_VERSIONS: &[u32] = &[1];

/// The tail of every "this is not schema 1" refusal, for keys and for values alike.
pub const NEWER_SENTENCE: &str = "not part of connector schema 1: a field from a newer schema needs a newer goalcup, and a misspelling is refused rather than ignored.";

/// The tail of the `schema_version` refusals.
const MIGRATION_SENTENCE: &str = "A newer schema needs a newer goalcup; an older one is migrated by the PR that retired it (docs/CONTRACTS.md#migrating-from-manifesttoml).";

/// The most a seed file may weigh, in bytes.
pub const MAX_SEED_BYTES: u64 = 64 * 1024;
