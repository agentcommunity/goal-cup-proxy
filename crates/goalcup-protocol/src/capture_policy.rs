//! Every reviewed trust table of the capture contract, in one file (harness contract v1, S2).
//!
//! A connector cannot widen policy: it can only be checked against it. So the rows below are
//! code, each is a trust decision about the participant's machine (D1, D36, D38, D41, D43),
//! and:
//!
//! * **every row needs a DECISIONS entry**; a new CA variable, credential name, interpreter
//!   exemption or startup-file class is a core PR, not a harness PR;
//! * **tightening a row that a shipped connector uses migrates that connector in the same
//!   PR**, and `tests/it/harness_conformance.rs` fails until it does;
//! * the lists that name *spellings* (interpreters, startup files, env names) slow an
//!   attacker down; they do not stop one. The control is the reviewer's re-derivation of the
//!   executable's `sha256` and the reviewer checklist (`docs/DECISIONS.md` D43, ruling 6).
//!
//! Recorded V1 manifests retain parser-only compatibility in `manifest.rs`; they do not own
//! capture policy. The harness manifest's loader and validator (schema 1) are in `harness.rs`.

use std::collections::BTreeSet;

// ---------------------------------------------------------------------------------------
// Bounds
// ---------------------------------------------------------------------------------------

pub const MAX_CAPTURE_ARGS: usize = 64;
pub const MAX_CAPTURE_ARG_BYTES: usize = 4096;
pub const MAX_CAPTURE_ENV: usize = 32;
pub const MAX_CAPTURE_ENV_KEY_BYTES: usize = 128;
pub const MAX_CAPTURE_ENV_VALUE_BYTES: usize = 1024;
pub const MAX_CAPTURE_HOSTS: usize = 16;
pub const MAX_CAPTURE_PATHS_PER_HOST: usize = 32;
pub const MAX_CAPTURE_PATH_BYTES: usize = 1024;
pub const MAX_CONTINUATION_BYTES: usize = 512;
pub const MAX_GOAL_FLAG_BYTES: usize = 32;
pub const MAX_VERSION_BYTES: usize = 32;

// ---------------------------------------------------------------------------------------
// CA trust, credentials, redaction mirror, tracker hosts
// ---------------------------------------------------------------------------------------

/// Whether a CA variable extends the harness's roots or replaces them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CaMode {
    /// Measured to add the proxy's CA to the harness's roots.
    Extend,
    /// Names a file that REPLACES the harness's provider-client roots with the proxy's CA.
    /// That fails closed (the harness then trusts only the proxy, which verifies the real
    /// upstream), but it is a different trust statement, so a manifest must declare it.
    Replace,
}

impl CaMode {
    /// The spellings `capture.ca.mode` accepts, in declaration order.
    pub const VALUES: &'static [&'static str] = &["extend", "replace"];

    /// The TOML spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            CaMode::Extend => "extend",
            CaMode::Replace => "replace",
        }
    }
}

/// The only CA variables a capture profile may name, with their mode (D41 as amended by D43).
///
/// Any connector may declare a replace-mode row. A new CA variable, in either mode, is a row
/// here plus a DECISIONS entry. `SSL_CERT_FILE`, `SSL_CERT_DIR`, `CURL_CA_BUNDLE` and
/// `REQUESTS_CA_BUNDLE` stay refused: they replace the roots of every tool child, not only the
/// harness's.
pub const CA_REGISTRY: &[(&str, CaMode)] = &[
    ("CODEX_CA_CERTIFICATE", CaMode::Extend),
    ("NODE_EXTRA_CA_CERTS", CaMode::Extend),
    ("HERMES_CA_BUNDLE", CaMode::Replace),
];

/// The registry mode of a CA variable, or `None` when it is not a registry row.
pub fn ca_registry_mode(name: &str) -> Option<CaMode> {
    CA_REGISTRY
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, mode)| *mode)
}

/// The provider-credential variables a capture profile may name. Closed on purpose (D43,
/// ruling 3): a new provider is a reviewed edit and a DECISIONS entry. Every value is held to
/// one grammar by the launcher (`child_secret_named`): 20 to 256 bytes of `[A-Za-z0-9_-]`.
/// `AMP_API_KEY` is Amp's access token (m2-amp-readiness; D49).
pub const CREDENTIAL_ENV: &[&str] = &["DEEPSEEK_API_KEY", "AMP_API_KEY"];

/// The regex sources of the daemon's streaming recognizers (`redact/stream.rs`
/// `EXTRA_SHAPES`), in order. A `[redaction].extra_patterns` regex must equal one of them
/// exactly: the daemon's redactor refuses any other, and the protocol crate must refuse it
/// first so a fork PR sees the failure in CI. `tests/it/harness_schema.rs` ties this list to the
/// shipped manifests, which the daemon's own test ties to `EXTRA_SHAPES`.
pub const EXTRA_PATTERN_SOURCES: &[&str] = &[
    r#""rate_limits"\s*:\s*(\{(?:[^{}]|\{(?:[^{}]|\{[^{}]*\})*\})*\})"#,
    r#""plan_type"\s*:\s*"([^"\\]+)""#,
    r#""credits_remaining"\s*:\s*("[^"\\]*"|[^,}\s]+)"#,
    r#"x-anthropic-billing-[a-z0-9-]+:\s*[^\r\n"]+"#,
];

/// Hosts that belong to the tracker's own delivery path. A harness may name none of them, nor a
/// subdomain (the delivery-agnostic rule, F7). Empty until a project domain is chosen; a host
/// containing `goalcup` is refused anyway ([`mentions_goalcup`]), and the devserver binds
/// `127.0.0.1`, which [`is_capture_host`] already refuses.
pub const TRACKER_HOSTS: &[&str] = &[];

/// Whether `host` equals, or is a subdomain of, a [`TRACKER_HOSTS`] entry.
pub fn is_tracker_host(host: &str) -> bool {
    TRACKER_HOSTS
        .iter()
        .any(|t| host == *t || host.strip_suffix(*t).is_some_and(|p| p.ends_with('.')))
}

/// Whether `s` contains `goalcup`, case-insensitively: no argument, value or host of a harness
/// names the tracker (F7).
pub fn mentions_goalcup(s: &str) -> bool {
    s.to_ascii_lowercase().contains("goalcup")
}

// ---------------------------------------------------------------------------------------
// Environment names and values
// ---------------------------------------------------------------------------------------

/// Names a capture launch pins itself, over anything a profile's `env` says.
const PINNED_ENV: &[&str] = &["HOME", "PATH", "USER", "LOGNAME", "TMPDIR"];

/// Trust and TLS variables no capture profile may set that are NOT [`CA_REGISTRY`] rows: the four
/// roots that replace every tool child's, and the switch that turns verification off. Every
/// `CA_REGISTRY` name is refused too, derived from the registry itself (see `env_name_denied`), so
/// a new registry row cannot be settable through `capture.env` or `home.env`. A name is in one of
/// the two lists, never both.
pub const TRUST_ONLY: &[&str] = &[
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "CURL_CA_BUNDLE",
    "REQUESTS_CA_BUNDLE",
    "NODE_TLS_REJECT_UNAUTHORIZED",
];

/// Variables that load code or startup files into the harness or its shells, or that name a
/// command or load path its tool children run. The first twelve are the V1 list; the rest are
/// siblings with the same effect (R6 S9, I5c).
pub const INJECTION_ENV: &[&str] = &[
    "NODE_OPTIONS",
    "NODE_PATH",
    "BUN_OPTIONS",
    "BASH_ENV",
    "ENV",
    "ZDOTDIR",
    "PYTHONSTARTUP",
    "PYTHONPATH",
    "PYTHONHOME",
    "PYTHONUSERBASE",
    "PERL5OPT",
    "RUBYOPT",
    "GIT_SSH_COMMAND",
    "GIT_SSH",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
    "GIT_EXEC_PATH",
    "GIT_TEMPLATE_DIR",
    "GIT_PAGER",
    "PAGER",
    "EDITOR",
    "VISUAL",
    "JAVA_TOOL_OPTIONS",
    "_JAVA_OPTIONS",
    "JDK_JAVA_OPTIONS",
    "PERL5LIB",
    "PERLLIB",
    "RUBYLIB",
    "PROMPT_COMMAND",
    "PS4",
    // S2 review I-1: more names that make a tool run a command or load a file.
    "LESSOPEN",
    "LESSCLOSE",
    "BROWSER",
    "CLAUDE_CODE_SHELL_PREFIX",
    "CLAUDE_ENV_FILE",
    "PYTHONWARNINGS",
    "SHELLOPTS",
    "BASHOPTS",
    "PS1",
    "PS2",
];

/// Name SUFFIXES that make a tool run a command: `GIT_PROXY_COMMAND`, `GIT_EDITOR`,
/// `MANPAGER`, `SUDO_ASKPASS`, `RUSTC_WRAPPER`, `CARGO_TARGET_<triple>_RUNNER`. The value rule
/// cannot stop these: a program resolved on `PATH` needs no `/`, `$`, backtick or `://`.
pub const COMMAND_SUFFIXES: &[(&str, &str)] = &[
    ("_COMMAND", "names a command a tool runs"),
    ("_CMD", "names a command a tool runs"),
    ("EDITOR", "names an editor a tool runs"),
    ("PAGER", "names a pager a tool runs"),
    (
        "ASKPASS",
        "names a program a tool runs to ask for a password",
    ),
    (
        "_WRAPPER",
        "names a wrapper a tool runs its commands through",
    ),
    ("_RUNNER", "names a runner a tool runs its programs through"),
];

/// Prefixes that inject code, export telemetry, reconfigure a tool child or relocate state.
/// `GIT_` is the whole git namespace (S2 review I-1): `GIT_CONFIG_*`, `GIT_SSH_COMMAND`,
/// `GIT_EXTERNAL_DIFF`, `GIT_EDITOR`, `GIT_PROXY_COMMAND` and the rest can each run a command,
/// and no harness needs one. Names compare upper-cased, so `npm_config_registry` is
/// `NPM_CONFIG_`.
pub const INJECTION_PREFIXES: &[(&str, &str)] = &[
    ("DYLD_", "injects code into the harness"),
    ("LD_", "injects code into the harness"),
    (
        "OTEL_",
        "exports telemetry somewhere the capture does not see",
    ),
    ("CLAUDE_CODE_USE_", "switches to another model provider"),
    (
        "GIT_",
        "configures git, whose environment can name a command, a pager, an editor or a config to run",
    ),
    (
        "NPM_CONFIG_",
        "reconfigures npm, which can redirect registries and run commands",
    ),
    (
        "PIP_",
        "reconfigures pip, which can redirect indexes and run commands",
    ),
    (
        "XDG_",
        "relocates the configuration and state of every tool child (XDG_* is not part of schema 1)",
    ),
    (
        "GOALCUP_",
        "is the tracker's own namespace, and a harness is delivery-agnostic",
    ),
];

/// Provider and cloud-credential namespaces: keys, endpoints, custom headers, provider
/// switches.
pub const PROVIDER_PREFIXES: &[(&str, &str)] = &[
    ("ANTHROPIC_", "configures the model provider"),
    ("OPENAI_", "configures the model provider"),
    ("AZURE_", "configures a model or cloud provider"),
    ("AWS_", "configures a model or cloud provider"),
    ("GOOGLE_", "configures a model or cloud provider"),
    ("GCLOUD_", "configures a model or cloud provider"),
    ("VERTEX_", "configures a model or cloud provider"),
    ("CLOUD_ML_", "configures a model or cloud provider"),
];

/// Namespaces reserved for one harness, which the capture launch configures itself. Not applied
/// to `home.env`, which is the harness's own home variable (`HERMES_HOME`).
pub const HARNESS_RESERVED_PREFIXES: &[(&str, &str)] = &[(
    "HERMES_",
    "is reserved for the Hermes harness, which the capture launch configures itself",
)];

/// Which field an environment name is being checked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnvContext {
    /// A key of `capture.env`: every row applies. `SHELL` passes the key check and is held to
    /// [`env_value_denied`].
    CaptureEnv,
    /// `capture.home.env`, the variable pointed at the config home. The substring rules, the
    /// pinned, trust and injection lists and the injection and provider prefixes apply;
    /// [`HARNESS_RESERVED_PREFIXES`] and the self-pin row do not (so `HERMES_HOME` passes), and
    /// `SHELL` is refused.
    HomeEnv,
}

/// Other tools' home, config and cache variables, refused as `home.env` (S2 review I-3).
/// `home.env` points a variable at the per-run config home, so naming a tool's own relocator
/// (`GRADLE_USER_HOME`, `CARGO_HOME`, `IPYTHONDIR`, ...) would make that tool read the
/// connector's seeds as its configuration.
pub const TOOL_HOME_NAMES: &[&str] = &[
    "CARGO_HOME",
    "RUSTUP_HOME",
    "GEM_HOME",
    "GEM_PATH",
    "JAVA_HOME",
    "M2_HOME",
    "GRADLE_USER_HOME",
    "GRADLE_HOME",
    "IPYTHONDIR",
    "DOCKER_CONFIG",
    "GNUPGHOME",
    "KUBECONFIG",
    "GOPATH",
    "GOROOT",
    "GOMODCACHE",
    "GOENV",
    "GOCACHE",
    "NVM_DIR",
    "PYENV_ROOT",
    "ANDROID_HOME",
    "ANDROID_SDK_ROOT",
    "MPLCONFIGDIR",
    "HF_HOME",
    "TORCH_HOME",
    "COMPOSER_HOME",
    "NUGET_PACKAGES",
    "PYTHONPYCACHEPREFIX",
    "PYTHONUSERBASE",
];

/// Prefixes of other tools' namespaces, refused as `home.env` (S2 review I-3).
pub const TOOL_HOME_PREFIXES: &[&str] = &[
    "CONDA_",
    "MAMBA_",
    "NPM_",
    "YARN_",
    "PNPM_",
    "UV_",
    "POETRY_",
    "PYENV_",
    "JUPYTER_",
    "IPYTHON",
    "GRADLE_",
    "DOCKER_",
    "KUBE",
    "HOMEBREW_",
    "BUNDLE_",
    "GEM_",
    "CARGO_",
    "RUSTUP_",
    "JAVA_",
    "MAVEN_",
    "ANDROID_",
    "DOTNET_",
    "NUGET_",
    "COMPOSER_",
    "PYTHON",
    "GNUPG",
    "NVM_",
    "PIP_",
    "GIT_",
];

/// Why a (upper-cased) name is refused as `home.env` beyond the shared rules, or `None`.
///
/// Two layers. A deny list of other tools' relocators ([`TOOL_HOME_NAMES`],
/// [`TOOL_HOME_PREFIXES`]) gives the specific reason. Then a structural rule: a harness's own
/// home variable is ONE word, then `_HOME` or `_CONFIG_DIR` (`CODEX_HOME`, `HERMES_HOME`,
/// `CLAUDE_CONFIG_DIR`), `^[A-Z][A-Z0-9]*_(HOME|CONFIG_DIR)$`. The shape keeps a new harness
/// out of core PRs while keeping every multi-word tool relocator out of schema 1; a harness
/// whose home variable has another shape is a core PR with a DECISIONS entry.
fn home_env_problem(upper: &str) -> Option<&'static str> {
    if TOOL_HOME_NAMES.contains(&upper) || TOOL_HOME_PREFIXES.iter().any(|p| upper.starts_with(p)) {
        return Some(
            "is another tool's home, config or cache variable: pointing it at the config home              would make that tool read the connector's seeds as its own configuration",
        );
    }
    let word = upper
        .strip_suffix("_HOME")
        .or_else(|| upper.strip_suffix("_CONFIG_DIR"));
    let one_word = word.is_some_and(|w| {
        let mut b = w.bytes();
        b.next().is_some_and(|c| c.is_ascii_uppercase())
            && b.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    });
    if one_word {
        None
    } else {
        Some(
            "is not the shape of a harness's own home variable: one word, then `_HOME` or              `_CONFIG_DIR` (`^[A-Z][A-Z0-9]*_(HOME|CONFIG_DIR)$`)",
        )
    }
}

const CA_REASON_V2: &str =
    "changes certificate trust: a CA variable belongs in `capture.ca`, not `capture.env`";

/// The one body of the schema-1 environment-name rules.
fn env_name_denied(key: &str, ctx: EnvContext) -> Option<&'static str> {
    let upper = key.to_ascii_uppercase();
    if upper.ends_with("_BASE_URL") || upper.ends_with("_API_URL") || upper.contains("ENDPOINT") {
        return Some("redirects an endpoint (D1)");
    }
    if upper.ends_with("_PROXY") || upper == "PROXY" {
        return Some("is a proxy variable, which the capture launch pins itself");
    }
    if [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "CREDENTIAL",
        "API_KEY",
        "APIKEY",
        "AUTH",
    ]
    .iter()
    .any(|s| upper.contains(s))
        || upper.ends_with("_KEY")
    {
        return Some("would carry a credential, and a manifest never does");
    }
    if upper.contains("PROVIDER") {
        return Some("switches to another model provider");
    }
    // The exact names come before the prefixes, so a name that is both (`HERMES_CA_BUNDLE` is
    // a trust variable and in the `HERMES_` namespace) is told the more specific reason.
    let registry_row = ca_registry_mode(&upper).is_some();
    if registry_row || TRUST_ONLY.contains(&upper.as_str()) {
        return Some(if registry_row {
            CA_REASON_V2
        } else {
            "changes certificate trust, which no capture profile sets"
        });
    }
    if INJECTION_ENV.contains(&upper.as_str()) {
        return Some("injects code or startup files into the harness");
    }
    if let Some((_, why)) = COMMAND_SUFFIXES
        .iter()
        .find(|(suffix, _)| upper.ends_with(suffix))
    {
        return Some(why);
    }
    if ctx == EnvContext::HomeEnv
        && let Some(why) = home_env_problem(&upper)
    {
        return Some(why);
    }
    if ctx == EnvContext::HomeEnv && upper == "SHELL" {
        return Some("is the shell the harness runs commands with, not a config home");
    }
    if PINNED_ENV.contains(&upper.as_str()) {
        return Some("is pinned by the capture launch itself");
    }
    let prefixes = INJECTION_PREFIXES
        .iter()
        .chain(PROVIDER_PREFIXES.iter())
        .chain(
            HARNESS_RESERVED_PREFIXES
                .iter()
                .filter(|_| ctx == EnvContext::CaptureEnv),
        );
    for (prefix, why) in prefixes {
        if upper.starts_with(prefix) {
            return Some(why);
        }
    }
    // Any other `*_URL` names a server (Amp reads `AMP_URL`, m2-amp-readiness). The value rule
    // already refuses `://`; this refuses the name too. It comes last, so a name another rule
    // refuses (`AUTH_URL`, `GOALCUP_UPLOAD_URL`, `PIP_INDEX_URL`) keeps its more specific reason.
    if upper.ends_with("_URL") {
        return Some("redirects an endpoint (D1)");
    }
    None
}

/// Why an environment variable NAME is refused for `ctx`, or `None`. Names compare
/// upper-cased. The names a manifest pins in other fields (`home.env`, `ca.env`,
/// `credential.env`) are checked for distinctness by the validator, not here.
pub fn env_key_denied(key: &str, ctx: EnvContext) -> Option<&'static str> {
    env_name_denied(key, ctx)
}

/// The shells a `SHELL` value may name.
pub const SHELL_VALUES: &[&str] = &["/bin/zsh", "/bin/bash", "/bin/sh"];

/// Why an environment VALUE is refused, or `None` (I5b). `SHELL` is admitted only as one of
/// [`SHELL_VALUES`]. Any other key's value is refused when it starts with `/` or `~`, contains
/// `$`, a backtick or `://`, or holds a `..` path component: a value that is a path, a URL or
/// an expansion can point the harness at the participant's real files or at another endpoint.
///
/// Template tokens are checked before rendering, so `{workspace}` passes. Not relaxed for
/// `Origin::Dev`: the controlled workload and the kit need no absolute env value.
pub fn env_value_denied(key: &str, value: &str) -> Option<&'static str> {
    if key.eq_ignore_ascii_case("SHELL") {
        return if SHELL_VALUES.contains(&value) {
            None
        } else {
            Some("SHELL is admitted only as /bin/zsh, /bin/bash or /bin/sh")
        };
    }
    if value.starts_with('/') || value.starts_with('~') {
        return Some(
            "is a path; an env value that starts with `/` or `~` can point the harness at the participant's real files",
        );
    }
    if value.contains('$') || value.contains('`') {
        return Some("holds `$` or a backtick, a shell expansion");
    }
    if value.contains("://") {
        return Some("holds `://`, a URL; an endpoint is not a manifest value (D1)");
    }
    if value.split('/').any(|c| c == "..") {
        return Some("holds a `..` path component");
    }
    None
}

/// Why an `args` element is refused as a path, or `None` (I5e, hardened by S2 review I-2).
/// An absolute or home-relative path, or an escape from the workspace, in argv can point the
/// harness at the participant's real config. Refused:
/// * a `..` path component anywhere (an element is split on every character that is not a
///   letter, digit, `.`, `_` or `-`, so `--cd={workspace}/../x`, `--opt=..` and `x/..` all
///   hold one);
/// * `/` or `~` at the start, or right after any of `=` `"` `'` `:` `,` `[` `(` or a space
///   (`--cfg=/x`, `model_instructions_file="/x"`, `file:///x`, `[/x]`);
/// * a short option with its value attached (`-C/x`, `-C~`, `-Ix/y`): a single dash, one to
///   four letters or digits, then `/` or `~`;
/// * `$` followed by a name or `{` (`$HOME`, `${HOME}`, `a$TMPDIR`): an expansion the harness
///   or a shell may perform.
///
/// Admitted for `Origin::Dev` only (N1): the controlled workload and the kit substitute
/// absolute paths for `@WORKLOAD@`, `@CONFIG@` and `stand-in.py`. A `{workspace}` token on its
/// own or followed by a relative path is fine; it cannot be combined with `..`.
pub fn arg_path_denied(arg: &str) -> Option<&'static str> {
    const WHY: &str = "is, or carries, an absolute path (or a home-relative or parent-relative one, or an expansion); a path in argv can point the harness at the participant's real config";
    let bytes = arg.as_bytes();
    let is_path_start = |i: usize| matches!(bytes[i], b'/' | b'~');
    if bytes.is_empty() {
        return None;
    }
    if is_path_start(0) {
        return Some(WHY);
    }
    for i in 1..bytes.len() {
        if is_path_start(i)
            && matches!(
                bytes[i - 1],
                b'=' | b'"' | b'\'' | b':' | b',' | b'[' | b'(' | b' ' | b'@'
            )
        {
            return Some(WHY);
        }
    }
    // `$NAME` and `${NAME}`.
    if bytes
        .windows(2)
        .any(|w| w[0] == b'$' && (w[1].is_ascii_alphabetic() || matches!(w[1], b'_' | b'{')))
    {
        return Some(WHY);
    }
    // `-X/...`: a single-dash cluster with a path attached.
    if bytes[0] == b'-' && bytes.get(1).is_some_and(|b| *b != b'-') {
        let cluster = bytes[1..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric())
            .count();
        if (1..=4).contains(&cluster)
            && bytes
                .get(1 + cluster)
                .is_some_and(|b| matches!(b, b'/' | b'~'))
        {
            return Some(WHY);
        }
    }
    // A `..` component anywhere.
    let normalized: String = arg
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '/'
            }
        })
        .collect();
    if normalized.split('/').any(|c| c == "..") {
        return Some(WHY);
    }
    None
}

// ---------------------------------------------------------------------------------------
// The executable and the config home
// ---------------------------------------------------------------------------------------

/// Programs that interpret their arguments as code, a script or a command: refused as
/// `executable.program` for `Origin::Pinned` (I5d). The check is on the spelling
/// ([`is_interpreter_program`]); the harness's own entry point, even a launcher script found
/// on `PATH`, is pinned by `sha256` like any other program. `codex`, `claude` and `hermes` are
/// not on the list. Kit cases and the controlled workload load as `Origin::Dev` and run
/// `python3`.
pub const INTERPRETER_PROGRAMS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "dash",
    "ksh",
    "fish",
    "csh",
    "tcsh",
    "env",
    "nohup",
    "sudo",
    "doas",
    "xargs",
    "open",
    "osascript",
    "python",
    "python2",
    "python3",
    "pypy",
    "pypy3",
    "node",
    "nodejs",
    "deno",
    "bun",
    "ruby",
    "perl",
    "php",
    "lua",
    "npx",
    "pnpx",
    "bunx",
    "uv",
    "uvx",
    "pipx",
    // S2 review M-3: launchers that run another program by name.
    "arch",
    "nice",
    "time",
    "timeout",
    "caffeinate",
    "script",
    "stdbuf",
    "busybox",
    "awk",
    "gawk",
    "tclsh",
    "expect",
    "pwsh",
    "java",
    "npm",
    "pnpm",
    "yarn",
    "git",
    "find",
    "make",
    "pythonw",
];

/// Whether `program` is an interpreter by name (N3). Compared case-insensitively, because APFS
/// resolves `Python3`; and a listed name followed by a version (`python3.13`, `python3.13t`,
/// `perl5.34`, `ruby3.3`, `node22`) is the same program. A listed name followed by anything
/// else is a different name (`python-lsp`, `shell-gpt`).
pub fn is_interpreter_program(program: &str) -> bool {
    let lower = program.to_ascii_lowercase();
    INTERPRETER_PROGRAMS.iter().any(|base| {
        lower == *base
            || lower.strip_prefix(base).is_some_and(|rest| {
                rest.starts_with(|c: char| c.is_ascii_digit())
                    && rest.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+')
                    })
            })
    })
}

/// Basenames a harness may never seed or create as a directory: files a shell, git, a package
/// manager or a language runtime reads at startup and runs. Compared case-insensitively
/// (APFS) against every component of a seed destination and of `dirs` (I5a).
pub const STARTUP_BASENAMES: &[&str] = &[
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".zlogout",
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".bash_logout",
    ".profile",
    ".kshrc",
    ".inputrc",
    ".envrc",
    ".gitconfig",
    ".gitattributes",
    ".npmrc",
    ".yarnrc",
    ".yarnrc.yml",
    "bunfig.toml",
    ".netrc",
    ".curlrc",
    ".wgetrc",
    ".pythonrc",
    ".pythonrc.py",
    "sitecustomize.py",
    "usercustomize.py",
    "pip.conf",
    ".pypirc",
    ".gemrc",
];

/// A file with one of these suffixes runs code when the interpreter starts (Python `*.pth`).
pub const STARTUP_SUFFIXES: &[&str] = &[".pth"];

/// Directory components that hold credentials or code the tools load, wherever they sit in a
/// seed destination or a `dirs` entry.
pub const STARTUP_COMPONENTS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".git",
    ".aws",
    ".docker",
    ".kube",
    "site-packages",
];

/// Why a relative path (a seed destination or a `dirs` entry) is refused as a startup file, or
/// `None` (I5a). Case-insensitive. The path `.config/git/...` (git's global config) is refused
/// as two consecutive components. Hermes's seeds (`config.yaml`, `auth.json`, `.update_check`)
/// and `dirs = ["home"]` pass.
pub fn startup_file_problem(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    let components: Vec<&str> = lower.split('/').collect();
    for c in &components {
        if STARTUP_BASENAMES.contains(c) {
            return Some("is a startup file a shell, git, a package manager or a runtime runs");
        }
        if STARTUP_SUFFIXES.iter().any(|s| c.ends_with(s)) {
            return Some("is a file the interpreter runs at startup (a `.pth` file)");
        }
        if STARTUP_COMPONENTS.contains(c) {
            return Some("is, or sits in, a directory of credentials or tool-loaded code");
        }
    }
    if components
        .windows(2)
        .any(|w| w[0] == ".config" && w[1] == "git")
    {
        return Some("is git's global configuration (`.config/git`)");
    }
    None
}

/// The first name that appears twice in `names`, for the distinctness checks.
pub(crate) fn has_duplicate<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut seen = BTreeSet::new();
    names.into_iter().find(|n| !seen.insert(*n))
}

// ---------------------------------------------------------------------------------------
// Arguments and hosts (moved unchanged from the V1 manifest)
// ---------------------------------------------------------------------------------------

/// Why a capture argument is refused, or `None`. The capture-specific deny list (D39):
/// model endpoint, provider switch and credential, proxy and trust overrides, telemetry
/// export, code injection, indirection through a settings file or profile, remote
/// execution, and the switches that turn the native session store off.
///
/// Whole flags match exactly or as `flag=value`, case-insensitively. Substrings match after
/// lower-casing and dropping every `-` and `_`, so `--base-url`, `openai_base_url=`,
/// `baseUrl` in a JSON value and `ANTHROPIC_BASE_URL` are one rule.
pub fn capture_arg_denied(arg: &str) -> Option<&'static str> {
    const WHOLE: &[(&str, &str)] = &[
        ("--remote", "runs the session somewhere else"),
        ("remote", "runs the session somewhere else"),
        ("remote-control", "hands the session to a remote controller"),
        ("cloud", "runs the session somewhere else"),
        ("--oss", "switches to another model provider"),
        ("--local-provider", "switches to another model provider"),
        ("--ephemeral", "turns the native session store off"),
        (
            "--no-session-persistence",
            "turns the native session store off",
        ),
        (
            "--settings",
            "loads settings from outside the profile, which can set any of this list",
        ),
        (
            "--profile",
            "loads a configuration profile from outside the capture profile",
        ),
        ("--plugin-dir", "loads plugin code into the harness"),
    ];
    const SUBSTRING: &[(&str, &str)] = &[
        ("baseurl", "redirects the model endpoint (D1)"),
        ("apiurl", "redirects the model endpoint (D1)"),
        ("endpoint", "redirects an endpoint (D1)"),
        ("modelprovider", "switches to another model provider"),
        ("bedrock", "switches to another model provider"),
        ("vertex", "switches to another model provider"),
        ("foundry", "switches to another model provider"),
        (
            "profile=",
            "selects a configuration profile from outside the capture profile",
        ),
        ("apikey", "carries a provider credential"),
        ("authtoken", "carries a provider credential"),
        ("bearer", "carries a provider credential"),
        ("credential", "carries a provider credential"),
        ("proxy", "sets a proxy the capture launch does not own"),
        ("cacert", "changes which certificates the harness trusts"),
        ("cabundle", "changes which certificates the harness trusts"),
        ("certfile", "changes which certificates the harness trusts"),
        ("rejectunauthorized", "turns certificate verification off"),
        (
            "otel",
            "exports telemetry somewhere the capture does not see",
        ),
        ("dyld", "injects code into the harness"),
        ("ldpreload", "injects code into the harness"),
        ("nodeoptions", "injects code into the harness"),
        ("bunoptions", "injects code into the harness"),
    ];
    let lower = arg.to_ascii_lowercase();
    for (flag, why) in WHOLE {
        if lower == *flag || lower.strip_prefix(flag).is_some_and(|r| r.starts_with('=')) {
            return Some(why);
        }
    }
    let normal: String = lower.chars().filter(|c| *c != '-' && *c != '_').collect();
    SUBSTRING
        .iter()
        .find(|(needle, _)| normal.contains(needle))
        .map(|(_, why)| *why)
}

/// A configured host: lowercase ASCII letters, digits and hyphens in dot-separated labels
/// of 1–63 bytes, not starting or ending with a hyphen, at most 253 bytes, with no scheme,
/// port, userinfo, wildcard or trailing dot, and not an IPv4 literal. The same rule the
/// proxy's `goalcup_cli::proxy::tls::is_config_host` applies to what it is handed.
pub fn is_capture_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        && host.parse::<std::net::Ipv4Addr>().is_err()
}

/// Why a capture path is refused, or `None`.
pub fn capture_path_problem(path: &str) -> Option<&'static str> {
    if !path.starts_with('/') {
        return Some("must start with `/`");
    }
    if path.len() > MAX_CAPTURE_PATH_BYTES {
        return Some("is longer than the bound");
    }
    if path.contains(['?', '#']) {
        return Some("must be a path without query or fragment");
    }
    if path.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Some("contains whitespace or a control character");
    }
    if path.split('/').any(|s| s == "." || s == "..") {
        return Some("contains a dot segment");
    }
    None
}
