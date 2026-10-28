//! [`HarnessManifest::validate`] and the rules it runs, in their fixed order.

use super::parse::refuse;
use super::schema::{
    CHAT_MODE_ONLY, CONFIGURE_DENIED_KEYS, ConfigHome, HarnessManifest, LAUNCH_PREP_TOKENS,
    MAX_CONFIGURE_ARG_BYTES, MAX_CONFIGURE_ARGS, MAX_CONFIGURE_COMMANDS, MAX_MODEL_HOST_KEY_PARTS,
    MAX_NO_PROMPT_SETTINGS, MAX_NO_PROMPT_VALUES, MODEL_HOST_DENIED_WORDS,
    MODEL_HOST_FILE_EXTENSIONS, MODEL_HOST_VIA_PART, ModelHostAbsent, NativeForm, Origin,
    ParticipantSpec, PlatformKey, RunHome, Strategy, display_name_problem, first_screen_problem,
    no_prompt_value_problem, setup_section_problem,
};
use super::{MIGRATION_SENTENCE, SUPPORTED_SCHEMA_VERSIONS};
use crate::ProtocolError;
use crate::capture_policy::{
    EnvContext, MAX_CAPTURE_ARG_BYTES, MAX_CAPTURE_ARGS, MAX_CAPTURE_ENV,
    MAX_CAPTURE_ENV_KEY_BYTES, MAX_CAPTURE_ENV_VALUE_BYTES, MAX_CAPTURE_HOSTS,
    MAX_CAPTURE_PATHS_PER_HOST, MAX_CONTINUATION_BYTES, arg_path_denied, ca_env_problem,
    capture_arg_denied, capture_arg_denied_except, capture_path_problem, env_key_denied,
    env_value_denied, has_duplicate, is_capture_host, is_interpreter_program, is_tracker_host,
    local_host_problem, mentions_goalcup, startup_file_problem,
};
use crate::hints::valid_hint;

/// Template variables an `args` element may use.
const ARG_TOKENS: &[&str] = &["workspace"];

/// Template variables a `capture.env` value may use: the argument tokens, and the two homes the
/// launch makes for the run (`{config_home}`, the isolated harness home, and `{run_home}`, the
/// private `HOME`), so a harness with a second state variable can point it inside them. Each
/// renders to a private path Goal Cup Proxy makes; the value rule still refuses a value that starts
/// with `/` or `~` or holds `..`, so a value cannot leave them.
const ENV_TOKENS: &[&str] = &["workspace", "config_home", "run_home"];

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
    check_arg_admitting(field, origin, a, None)
}

/// [`check_arg`], with `admitted` the one deny-list word a `[launch_prep]` key argument may hold
/// (D72, [`capture_arg_denied_except`]).
fn check_arg_admitting(
    field: &str,
    origin: Origin,
    a: &str,
    admitted: Option<&str>,
) -> Result<(), ProtocolError> {
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
    let denied = match admitted {
        Some(word) => capture_arg_denied_except(a, word),
        None => capture_arg_denied(a),
    };
    if !substituted_path && let Some(why) = denied {
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

/// One argument of a `capture.configure` or `[launch_prep]` command, braces already handled:
/// the `args` rules, then no `.`- or `=`-separated part may be a [`CONFIGURE_DENIED_KEYS`] word
/// (compared whole, ignoring case and leading dashes, so `--model=x` and `provider=x` are
/// refused as `model.x` is): a configure command changes how the harness stores its session,
/// never how the agent behaves or what it signs in with (D61). `admitted` is the one word a
/// `[launch_prep]` key may hold (D72): a part that is that word, ignoring `-` and `_`, passes.
fn check_configure_arg(
    field: &str,
    origin: Origin,
    a: &str,
    admitted: Option<&str>,
) -> Result<(), ProtocolError> {
    check_arg_admitting(field, origin, a, admitted)?;
    let is_admitted = |part: &str| {
        admitted.is_some_and(|word| {
            part.chars()
                .filter(|c| *c != '-' && *c != '_')
                .collect::<String>()
                .eq_ignore_ascii_case(word)
        })
    };
    if let Some(word) = a
        .split(['.', '='])
        .map(|part| part.trim_start_matches('-'))
        .find(|part| {
            !is_admitted(part)
                && CONFIGURE_DENIED_KEYS
                    .iter()
                    .any(|denied| part.eq_ignore_ascii_case(denied))
        })
    {
        return Err(refuse(
            field,
            format!(
                "`{a}` names `{word}`. A configure command may change only how the harness \
                 stores its session. It may not set the model, the provider, an address, or a \
                 key, token or secret."
            ),
        ));
    }
    Ok(())
}

/// A list of harness commands under the `capture.configure` rules (D61), refused as `field`:
/// at most [`MAX_CONFIGURE_COMMANDS`], each of 1 to [`MAX_CONFIGURE_ARGS`] arguments of 1 to
/// [`MAX_CONFIGURE_ARG_BYTES`] bytes, no brace (so no template, `no_template` says why), and
/// each argument held to [`check_configure_arg`] with no word admitted.
fn check_templateless_commands(
    field: &str,
    commands: &[Vec<String>],
    origin: Origin,
    no_template: &str,
) -> Result<(), ProtocolError> {
    if commands.len() > MAX_CONFIGURE_COMMANDS {
        return Err(refuse(
            field,
            format!("holds more than {MAX_CONFIGURE_COMMANDS} commands"),
        ));
    }
    for command in commands {
        if command.is_empty() || command.len() > MAX_CONFIGURE_ARGS {
            return Err(refuse(
                field,
                format!("a command must hold 1 to {MAX_CONFIGURE_ARGS} arguments"),
            ));
        }
        for a in command {
            if a.is_empty() || a.len() > MAX_CONFIGURE_ARG_BYTES {
                return Err(refuse(
                    field,
                    format!("an argument must be 1 to {MAX_CONFIGURE_ARG_BYTES} bytes"),
                ));
            }
            if a.contains(['{', '}']) {
                return Err(refuse(field, format!("`{a}` holds a brace: {no_template}")));
            }
            check_configure_arg(field, origin, a, None)?;
        }
    }
    Ok(())
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

/// The rules of a value read from the harness's own config (`capture.model_host_from`, D62, and
/// `participant.no_prompts`, D67), refused under `<field>.file` or `<field>.key`: a YAML, JSON or
/// TOML file relative to the config home, a dotted key ([`config_key_problem`]), and neither
/// naming a key file or a key (`why_not_secret` says what is read instead).
fn config_value_problem(
    field: &str,
    file: &str,
    key: &str,
    why_not_secret: &str,
    via_part: bool,
) -> Result<(), ProtocolError> {
    let file_field = format!("{field}.file");
    if let Some(why) = relative_path_problem(file, 4) {
        return Err(refuse(&file_field, format!("`{file}` {why}")));
    }
    let name = file.rsplit('/').next().unwrap_or(file);
    let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    if !extension.is_some_and(|e| MODEL_HOST_FILE_EXTENSIONS.contains(&e.as_str())) {
        return Err(refuse(
            &file_field,
            format!(
                "`{file}` must end in one of .{}: the extension says how it is read",
                MODEL_HOST_FILE_EXTENSIONS.join(", .")
            ),
        ));
    }
    if let Some(word) = denied_word(name) {
        return Err(refuse(
            &file_field,
            format!("`{name}` names `{word}`: {why_not_secret}"),
        ));
    }
    config_key_problem(&format!("{field}.key"), key, why_not_secret, via_part)
}

/// A dotted key in a harness's config, refused under `field`: 1 to [`MAX_MODEL_HOST_KEY_PARTS`]
/// names, none a denied word. With `via_part` it holds exactly one part [`MODEL_HOST_VIA_PART`],
/// which the name `capture.model_host_from.via` reads stands for at run time.
fn config_key_problem(
    field: &str,
    key: &str,
    why_not_secret: &str,
    via_part: bool,
) -> Result<(), ProtocolError> {
    let parts: Vec<&str> = key.split('.').collect();
    let part_ok = |p: &&str| {
        (via_part && *p == MODEL_HOST_VIA_PART)
            || ((1..=64).contains(&p.len())
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
    };
    if parts.len() > MAX_MODEL_HOST_KEY_PARTS || !parts.iter().all(part_ok) {
        return Err(refuse(
            field,
            format!(
                "`{key}` must be 1 to {MAX_MODEL_HOST_KEY_PARTS} names of `[A-Za-z0-9_-]` \
                 (1 to 64 bytes) joined with `.`"
            ),
        ));
    }
    if via_part && parts.iter().filter(|p| **p == MODEL_HOST_VIA_PART).count() != 1 {
        return Err(refuse(
            field,
            format!(
                "`{key}` must hold exactly one `{MODEL_HOST_VIA_PART}` part, which the name \
                 `via` reads stands for"
            ),
        ));
    }
    if let Some(word) = denied_word(key) {
        return Err(refuse(
            field,
            format!("`{key}` names `{word}`: {why_not_secret}"),
        ));
    }
    Ok(())
}

/// The first `.`, `-` or `_` separated part of `text`, or `.` separated part, that is one of
/// [`MODEL_HOST_DENIED_WORDS`] (any case).
fn denied_word(text: &str) -> Option<String> {
    text.split(['.', '-', '_'])
        .chain(text.split('.'))
        .find(|w| {
            MODEL_HOST_DENIED_WORDS
                .iter()
                .any(|d| w.eq_ignore_ascii_case(d))
        })
        .map(str::to_string)
}

/// A single-component glob matcher: `*` any run (including none), `?` one character, anything
/// else itself. The one matcher of a `[[capture.native]]` pattern's components: the launcher's
/// precheck and the daemon's capture both call it. A `*` in the pattern is always a wildcard,
/// even against a name that holds a literal `*`.
pub fn glob_matches(name: &str, pattern: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let (mut i, mut j) = (0usize, 0usize);
    let (mut star, mut matched) = (None, 0usize);
    while i < name.len() {
        if j < pattern.len() && pattern[j] == '*' {
            star = Some(j);
            matched = i;
            j += 1;
        } else if j < pattern.len() && (pattern[j] == '?' || pattern[j] == name[i]) {
            i += 1;
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

/// The pattern components of a native `pattern`, or the reason it is not the
/// `NativeSourceSpec` grammar.
fn native_pattern_components(pattern: &str) -> Result<Vec<String>, String> {
    if pattern.is_empty() || pattern.len() > 1024 {
        return Err("is 1 to 1024 bytes".to_string());
    }
    if pattern.starts_with('/') {
        return Err("is relative to its root".to_string());
    }
    if pattern.contains("{id}") {
        return Err("names `{id}`, but `identity.source = \"none\"` has no session id".to_string());
    }
    if pattern.contains(['{', '}']) {
        return Err("may use no brace".to_string());
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
        if !part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'*' | b'?'))
        {
            return Err("has a component outside `[A-Za-z0-9._-]`, `*` and `?`".to_string());
        }
        out.push(part.to_string());
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
    /// identity, native, argv guard, termination, model host, hosts, the delivery-agnostic
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

    /// The anchor of the harness's section in `docs/HARNESS-SETUP.md`, when its manifest names
    /// one.
    pub fn setup_section(&self) -> Option<&str> {
        self.participant
            .as_ref()
            .and_then(|p| p.setup_section.as_deref())
    }

    /// `[participant]`: both strings reach a terminal, so they are held to plain text, and the
    /// setup section goes into a link, so it is held to an anchor's shape.
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
        if let Some(why) = p.setup_section.as_deref().and_then(setup_section_problem) {
            return Err(refuse("participant.setup_section", why));
        }
        self.check_time_left_tool(p)?;
        self.check_no_prompts(p)
    }

    /// `participant.time_left_tool` (fix plan v9): a config file and key held to the rules of a
    /// value read from the harness's config, in a home the participant keeps (a shared config
    /// home, or a shared run `HOME`). A harness whose homes are all made per run has nowhere to
    /// keep the lines.
    fn check_time_left_tool(&self, p: &ParticipantSpec) -> Result<(), ProtocolError> {
        const FIELD: &str = "participant.time_left_tool";
        let Some(tool) = &p.time_left_tool else {
            return Ok(());
        };
        let home = &self.capture.home;
        if home.config_home != ConfigHome::Shared && home.run_home != RunHome::Shared {
            return Err(refuse(
                FIELD,
                "requires `capture.home.config_home = \"shared\"` or `capture.home.run_home = \
                 \"shared\"`: the lines go in a home the participant keeps",
            ));
        }
        config_value_problem(
            FIELD,
            &tool.file,
            &tool.key,
            "the lines go in the file that lists the harness's MCP servers, never a key file",
            false,
        )
    }

    /// `participant.no_prompts` (D67): settings read from the shared config home like
    /// `capture.model_host_from`, each with 1 to [`MAX_NO_PROMPT_VALUES`] plain values. They
    /// are only read and recommended, never written.
    fn check_no_prompts(&self, p: &ParticipantSpec) -> Result<(), ProtocolError> {
        const FIELD: &str = "participant.no_prompts";
        if p.no_prompts.is_empty() {
            return Ok(());
        }
        if p.no_prompts.len() > MAX_NO_PROMPT_SETTINGS {
            return Err(refuse(
                FIELD,
                format!("holds more than {MAX_NO_PROMPT_SETTINGS} settings"),
            ));
        }
        if self.capture.home.config_home != ConfigHome::Shared {
            return Err(refuse(
                FIELD,
                "requires `capture.home.config_home = \"shared\"`: the settings are read from the \
                 home the participant set the harness up in",
            ));
        }
        for s in &p.no_prompts {
            config_value_problem(
                FIELD,
                &s.file,
                &s.key,
                "Goal Cup reads a permission setting from the harness's config, never a key file \
                 or a key",
                false,
            )?;
            if s.values.is_empty() || s.values.len() > MAX_NO_PROMPT_VALUES {
                return Err(refuse(
                    format!("{FIELD}.values"),
                    format!("`{}` must list 1 to {MAX_NO_PROMPT_VALUES} values", s.key),
                ));
            }
            if let Some(why) = s.values.iter().find_map(|v| no_prompt_value_problem(v)) {
                return Err(refuse(format!("{FIELD}.values"), why));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        if let Some(d) = p
            .no_prompts
            .iter()
            .find(|s| !seen.insert((s.file.as_str(), s.key.as_str())))
        {
            return Err(refuse(
                FIELD,
                format!("`{}` in `{}` is listed twice", d.key, d.file),
            ));
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
        // `from_toml_str` refuses a headless manifest before this; a manifest that reached here
        // another way is held to the same rule.
        if !self.capture.interactive {
            return Err(refuse("capture.interactive", CHAT_MODE_ONLY));
        }
        self.check_executable(origin)?;
        self.check_args(origin)?;
        self.check_configure(origin)?;
        self.check_launch_prep(origin)?;
        self.check_env()?;
        self.check_home()?;
        self.check_ca()?;
        self.check_native()?;
        self.check_argv_guard()?;
        self.check_termination()?;
        self.check_model_host_from()?;
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

    fn check_args(&self, origin: Origin) -> Result<(), ProtocolError> {
        let c = &self.capture;
        // A chat-mode harness may open its chat with no argument at all (D57).
        if c.args.len() > MAX_CAPTURE_ARGS {
            return Err(refuse(
                "capture.args",
                format!("must hold 0 to {MAX_CAPTURE_ARGS} arguments"),
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
        check_templateless_commands(
            "capture.configure",
            &self.capture.configure,
            origin,
            "a configure command takes no template (no goal, workspace or session), because it \
             runs before any run exists",
        )
    }

    /// `[launch_prep]` (D72): `ca_file` is one `.pem` file name, and each command follows the
    /// `capture.configure` rules, with one exception. A whole argument `{model_base_url}` or
    /// `{ca_file}` is admitted only right after its key: an argument with no `=` whose last
    /// `.`-separated part is exactly `base_url` or `ssl_ca_cert`. That key is then admitted
    /// although the deny lists refuse its word (`baseurl`, `cacert`): the only value such a key
    /// can get is goalcup's own. Any other brace is refused. `after_exit` follows the
    /// `capture.configure` rules exactly, with no template.
    fn check_launch_prep(&self, origin: Origin) -> Result<(), ProtocolError> {
        const FIELD: &str = "launch_prep.commands";
        let Some(prep) = &self.launch_prep else {
            return Ok(());
        };
        if let Some(name) = &prep.ca_file {
            let shaped = is_path_component(name)
                && name.len() > 4
                && name.to_ascii_lowercase().ends_with(".pem");
            if !shaped {
                return Err(refuse(
                    "launch_prep.ca_file",
                    format!(
                        "`{name}` must be one file name of `[A-Za-z0-9._-]` (at most 64 bytes) \
                         ending in `.pem`"
                    ),
                ));
            }
        }
        if prep.commands.len() > MAX_CONFIGURE_COMMANDS {
            return Err(refuse(
                FIELD,
                format!("holds more than {MAX_CONFIGURE_COMMANDS} commands"),
            ));
        }
        // The template `a` is, with its deny-list word and the last part its key must have.
        let token_of = |a: &str| {
            LAUNCH_PREP_TOKENS
                .iter()
                .find(|(token, _, _)| *token == a)
                .map(|(_, word, key)| (*word, *key))
        };
        // A key the template fills: no `=` (no value of its own), and its last `.`-separated
        // part is exactly the one the template names.
        let is_key_for = |a: &str, key: &str| !a.contains('=') && a.rsplit('.').next() == Some(key);
        for command in &prep.commands {
            if command.is_empty() || command.len() > MAX_CONFIGURE_ARGS {
                return Err(refuse(
                    FIELD,
                    format!("a command must hold 1 to {MAX_CONFIGURE_ARGS} arguments"),
                ));
            }
            for (i, a) in command.iter().enumerate() {
                if a.is_empty() || a.len() > MAX_CONFIGURE_ARG_BYTES {
                    return Err(refuse(
                        FIELD,
                        format!("an argument must be 1 to {MAX_CONFIGURE_ARG_BYTES} bytes"),
                    ));
                }
                if let Some((word, key)) = token_of(a) {
                    let after_key = i > 0 && is_key_for(&command[i - 1], key);
                    if !after_key {
                        return Err(refuse(
                            FIELD,
                            format!(
                                "`{a}` must come right after the key it fills, an argument \
                                 whose last `.` part is `{key}`, with no `=`"
                            ),
                        ));
                    }
                    let source = if word == "baseurl" {
                        self.capture
                            .model_host_from
                            .is_none()
                            .then_some("`capture.model_host_from`")
                    } else {
                        prep.ca_file.is_none().then_some("`launch_prep.ca_file`")
                    };
                    if let Some(needed) = source {
                        return Err(refuse(
                            FIELD,
                            format!("`{a}` needs {needed}, which gives it its value"),
                        ));
                    }
                    continue;
                }
                if a.contains(['{', '}']) {
                    return Err(refuse(
                        FIELD,
                        format!(
                            "`{a}` holds a brace: a launch prep command takes only the \
                             templates `{{model_base_url}}` and `{{ca_file}}`, each as a whole \
                             argument after its key"
                        ),
                    ));
                }
                // The key of a pair: the next argument is the template its word stands for.
                let admitted = command
                    .get(i + 1)
                    .and_then(|next| token_of(next))
                    .filter(|(_, key)| is_key_for(a, key))
                    .map(|(word, _)| word);
                check_configure_arg(FIELD, origin, a, admitted)?;
            }
        }
        // `after_exit` (0.7.1): the `capture.configure` rules exactly, with no template. It runs
        // after the harness is gone, where no value of the run is wanted.
        check_templateless_commands(
            "launch_prep.after_exit",
            &prep.after_exit,
            origin,
            "an after-exit command takes no template",
        )
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
                "must name 1 or 2 CA variables, or the harness could not trust the recording proxy",
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
        native_pattern_components(&n.pattern)
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

    /// `capture.model_host_from` (D62): a YAML, JSON or TOML file in the shared config home, and a
    /// dotted key. Neither may name a key file or a key: goalcup reads a service address only.
    fn check_model_host_from(&self) -> Result<(), ProtocolError> {
        let c = &self.capture;
        let Some(from) = &c.model_host_from else {
            return Ok(());
        };
        if c.home.config_home != ConfigHome::Shared {
            return Err(refuse(
                "capture.model_host_from",
                "requires `capture.home.config_home = \"shared\"`: the model service is read \
                 from the home the participant set the harness up in",
            ));
        }
        const WHY: &str = "Goal Cup Proxy reads the model service's address from the harness's \
                           config, never a key file or a key";
        // `via` names the table the key's `*` part stands for (`model_provider`), so the key
        // has that one part exactly when `via` is set.
        config_value_problem(
            "capture.model_host_from",
            &from.file,
            &from.key,
            WHY,
            from.via.is_some(),
        )?;
        match &from.via {
            Some(via) => config_key_problem("capture.model_host_from.via", via, WHY, false),
            None => Ok(()),
        }
    }

    fn check_hosts(&self, origin: Origin) -> Result<(), ProtocolError> {
        let c = &self.capture;
        // A profile that reads its model host from the harness's config (D62) may list none,
        // unless a missing value falls back to the hosts it lists.
        let least = usize::from(
            c.model_host_from
                .as_ref()
                .is_none_or(|from| from.absent == ModelHostAbsent::Defaults),
        );
        if c.hosts.len() < least || c.hosts.len() > MAX_CAPTURE_HOSTS {
            return Err(refuse(
                "capture.hosts",
                format!(
                    "must hold 1 to {MAX_CAPTURE_HOSTS} hosts, or none with \
                     `capture.model_host_from` and `absent = \"refuse\"`"
                ),
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

    /// The delivery-agnostic rule (F7): a harness never names Goal Cup Proxy, a server or an
    /// upload. Schema 1 has no field for one, and `deny_unknown_fields` keeps it that way.
    fn check_delivery_agnostic(&self) -> Result<(), ProtocolError> {
        let c = &self.capture;
        for (field, a) in self.every_arg() {
            if mentions_goalcup(a) {
                return Err(refuse(
                    field,
                    format!("`{a}` names Goal Cup Proxy: a harness is delivery-agnostic"),
                ));
            }
        }
        if let Some(name) = self.launch_prep.as_ref().and_then(|p| p.ca_file.as_ref())
            && mentions_goalcup(name)
        {
            return Err(refuse(
                "launch_prep.ca_file",
                format!("`{name}` names Goal Cup Proxy: a harness is delivery-agnostic"),
            ));
        }
        for (k, v) in &c.env {
            if k.to_ascii_uppercase().starts_with("GOALCUP_") || mentions_goalcup(v) {
                return Err(refuse(
                    "capture.env",
                    format!("`{k}` names Goal Cup Proxy: a harness is delivery-agnostic"),
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
                    format!(
                        "`{host}` is Goal Cup Proxy's own host: a harness is delivery-agnostic"
                    ),
                ));
            }
        }
        Ok(())
    }

    fn check_cross_field(&self) -> Result<(), ProtocolError> {
        let c = &self.capture;
        let per_run = c.home.config_home == ConfigHome::PerRun;
        let run_shared = c.home.run_home == RunHome::Shared;

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
        if root_shared && !native.since_run_start {
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
        let pattern = native_pattern_components(&native.pattern).expect("checked by native");
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
                             may not read a file Goal Cup Proxy seeds",
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
