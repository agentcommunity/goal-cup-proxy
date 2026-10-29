//! The model service a harness's own config names (D62): `capture.model_host_from`.
//!
//! The participant picks the model service in the harness, and the harness saves its address
//! in its own config. `goalcup start` and each `goalcup continue` read that one value, before
//! anything is created, and the recording proxy intercepts its host for that run beside the
//! profile's `hosts`. Every other host stays an opaque tunnel, counted and never opened. There is
//! no provider list: whatever service the participant set up is the one recorded.
//!
//! The reader is small on purpose. It opens one file (no symlink, at most
//! [`MAX_CONFIG_BYTES`]) in the shared config home, finds one dotted key in it and keeps only
//! that value. Nothing else of the file is kept or said, and only the host and port of the value
//! are ever logged: a userinfo (`user:pass@`), the path and the query are dropped before any
//! line is written. YAML is read by the block-mapping lookup below (no YAML crate); JSON and
//! TOML by the parsers this binary already links.
//!
//! A value goalcup cannot record (missing, not a URL, an authority with a backslash, a space or a
//! control byte, plain `http`, loopback, an address literal, Goal Cup's own or an authentication
//! host, a path the recorder cannot follow) is refused with the reason and the way to change it
//! ([`crate::participant::cannot_record_model_service`]). A name that resolves only to this
//! computer is refused too, by [`refuse_if_on_this_computer`], at `goalcup prep`, `goalcup start`
//! and `goalcup continue`. `goalcup prep` makes the same checks and keeps the home not ready until
//! the harness names a service that can be recorded. Under `absent = "defaults"` a missing value
//! is no refusal: the run records the connector's own hosts, as a connector without the field
//! does. What is checked is the address the config names. A login the harness refreshes at another
//! host (an OAuth provider's token endpoint) is not checked here (D62).

use crate::CliError;
use crate::connector::HarnessConnector;
use crate::proxy::api::{CapturePath, CapturePolicy};
use crate::proxy::connect::ApprovedHost;
use goalcup_protocol::capture_policy::{
    is_capture_host, is_tracker_host, local_host_problem, mentions_goalcup, unclean_path,
};
use goalcup_protocol::harness::{
    CAPTURE_HOMES_DIR, MODEL_HOST_VIA_PART, ModelHostAbsent, ModelHostFrom,
};
use std::path::Path;

/// The largest config file the reader opens.
pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// The request paths, under the service's base path, that carry a known wire format, and the
/// decoder hint each is recorded with. The hint names a wire format, not a provider. A path
/// under the base with none of these is recorded with no hint (capture-only).
const WIRE_PATHS: &[(&str, &str)] = &[
    ("/chat/completions", "openai-chat-completions"),
    ("/messages", "anthropic-messages"),
    ("/responses", "openai-responses"),
];

/// The decoder hints a model host read from the harness's config is recorded with, for
/// `goalcup doctor`.
pub fn wire_hints() -> impl Iterator<Item = &'static str> {
    WIRE_PATHS.iter().map(|(_, hint)| *hint)
}

/// The model service a run records: the host and port of the config's URL, and its path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelHost {
    /// Lowercase DNS name, checked by [`is_capture_host`].
    pub host: String,
    pub port: u16,
    /// The URL's path, without its query or fragment; may be empty.
    pub base_path: String,
    /// Where it was read: `<key> in <file>`.
    pub from: String,
    /// The file value to edit when this service cannot be recorded here (the same edit a refused
    /// value gets, with its full path); `None` for a value that is not in a file.
    pub edit: Option<Box<EditSetting>>,
}

impl ModelHost {
    /// `host:port`, the only part of the value that is ever logged.
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// The run's detail line (`host.log`).
    pub fn detail(&self) -> String {
        format!("model service: {} ({})", self.authority(), self.from)
    }

    /// The recording proxy's host rule. Every exchange on the host is recorded, whatever its path;
    /// these rows only pick each exchange's decoder hint. Each [`WIRE_PATHS`] entry sits under the
    /// base path (and under `<base>/v1`, where an SDK adds the version itself) with its hint, and
    /// the rest of the base is a row with no hint.
    pub fn approved(&self) -> ApprovedHost {
        let base = self.base_path.trim_end_matches('/');
        let mut paths = Vec::new();
        for prefix in [base.to_string(), format!("{base}/v1")] {
            for (suffix, hint) in WIRE_PATHS {
                paths.push(CapturePath {
                    path: format!("{prefix}{suffix}"),
                    prefix: false,
                    protocol_hint: Some((*hint).to_string()),
                });
            }
        }
        // The rest of the base, with no hint: a URL with no path names the versioned API (`/v1/`).
        let rest = if base.is_empty() {
            "/v1/".to_string()
        } else {
            format!("{base}/")
        };
        paths.push(CapturePath {
            path: rest,
            prefix: true,
            protocol_hint: None,
        });
        ApprovedHost {
            host: self.host.clone(),
            policy: CapturePolicy { paths },
        }
    }
}

/// Why the config's value cannot be recorded, in words a participant reads: what the harness
/// is set to use (`scheme://host:port` or a short description, never the rest of the value) and
/// the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unrecordable {
    pub shown: String,
    pub reason: String,
    /// The setting a value read came from (`` `env.ANTHROPIC_BASE_URL` in settings.json ``),
    /// when the reason does not already say it.
    pub set_by: Option<String>,
    /// The config file is there but cannot be read: then the advice is about the file, not the
    /// service.
    pub file: Option<Box<UnreadableFile>>,
    /// The value is in the config file and cannot be recorded: where to edit it.
    pub edit: Option<Box<EditSetting>>,
}

/// A value in the config file that Goal Cup cannot record: the key to edit, the file's full path,
/// and what the run records without the value. `fallback` is the connector's own hosts when a
/// missing value means them (`absent = "defaults"`); otherwise it is `None`, deleting the value
/// refuses the run, and only editing it helps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSetting {
    pub key: String,
    pub path: std::path::PathBuf,
    pub fallback: Option<Vec<String>>,
}

/// A config file that is there but that Goal Cup cannot read: its path, why, and what fixes it.
/// Never anything of its contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableFile {
    pub path: std::path::PathBuf,
    pub problem: FileProblem,
}

/// Why a config file that is there cannot be read, and what the participant does about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileProblem {
    /// After "the file": `is a link`.
    pub reason: &'static str,
    /// A whole instruction: `Replace the link with a copy of the file`.
    pub fix: &'static str,
}

impl Unrecordable {
    /// What to do about a value in the config file that cannot be recorded: "Edit or delete
    /// `<key>` in `<full path>`; without it Goal Cup records `<default hosts>`." When the value
    /// has no default to fall back on, only the edit. `None` for a value that is not in a file.
    pub fn edit_sentence(&self) -> Option<String> {
        let edit = self.edit.as_ref()?;
        let (key, path) = (&edit.key, edit.path.display());
        Some(match &edit.fallback {
            Some(hosts) if !hosts.is_empty() => format!(
                "Edit or delete {key} in {path}; without it Goal Cup records {}.",
                hosts.join(", ")
            ),
            _ => format!("Edit {key} in {path} to an https address Goal Cup can record."),
        })
    }

    /// "`<name>` is set to use `<shown>`, which Goal Cup cannot record: `<reason>` (set by
    /// `<key>` in `<file>`)." For a file that cannot be read: "Goal Cup cannot read `<path>`,
    /// where `<name>` keeps its model service setting: the file `<reason>`. `<fix>`."
    pub fn sentence(&self, name: &str) -> String {
        if let Some(file) = &self.file {
            return format!(
                "Goal Cup cannot read {}, where {name} keeps its model service setting: the \
                 file {}. {}.",
                file.path.display(),
                file.problem.reason,
                file.problem.fix
            );
        }
        let set_by = self
            .set_by
            .as_ref()
            .map(|s| format!(" (set by {s})"))
            .unwrap_or_default();
        format!(
            "{name} is set to use {}, which Goal Cup cannot record: {}{set_by}.",
            self.shown, self.reason
        )
    }

    /// The whole refusal `goalcup start` and `goalcup continue` give, for connector `id`.
    pub fn refusal(&self, name: &str, id: &str) -> String {
        if self.file.is_some() {
            return format!("{} Then run the command again.", self.sentence(name));
        }
        crate::participant::cannot_record_model_service(
            &self.sentence(name),
            id,
            self.edit_sentence().as_deref(),
        )
    }
}

/// The model service `connector`'s config names, for one segment, or `None` for a connector
/// without `capture.model_host_from` or one whose config names none under `absent =
/// "defaults"` (the run then records the connector's own hosts). Reads the file once; refuses a
/// value it cannot record with the reason and the way to change it
/// ([`crate::participant::cannot_record_model_service`]).
pub fn for_run(cfg: &Path, connector: &HarnessConnector) -> Result<Option<ModelHost>, CliError> {
    check(cfg, connector).map_err(|why| {
        CliError::Refused(why.refusal(connector.display_name(), &connector.loaded.manifest.id))
    })
}

/// [`for_run`] without the refusal around it, for `goalcup prep`.
pub fn check(cfg: &Path, connector: &HarnessConnector) -> Result<Option<ModelHost>, Unrecordable> {
    let manifest = &connector.loaded.manifest;
    let Some(from) = &manifest.capture.model_host_from else {
        return Ok(None);
    };
    let home = cfg.join(CAPTURE_HOMES_DIR).join(&manifest.id);
    let defaults: Vec<String> = manifest
        .capture
        .hosts
        .iter()
        .map(|h| h.host.clone())
        .collect();
    read(&home, from, &manifest.capture.auth_hosts, &defaults)
}

/// The command asking for provider hosts. It decides whether the harness's sign-in hosts are
/// probed, and the command the default-host warning tells the participant to run again. It is
/// not the clap `Command`: this is only the two callers that split hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Caller {
    /// `goalcup setup` (`goalcup prep`): probes the sign-in hosts too.
    Setup,
    /// `goalcup start`: never probes the sign-in hosts.
    Start,
}

impl Caller {
    /// The last clause of a default-host warning: what the participant runs next.
    fn rerun(self) -> &'static str {
        match self {
            Self::Setup => "run setup again",
            Self::Start => "start again",
        }
    }
}

/// The provider hosts one caller probes, split into the ones a missing answer refuses and the
/// ones whose missing answer is only a warning. Both lists keep the profile's order; a caller
/// that needs them sorted or without duplicates does that itself.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ProviderHosts {
    /// Must answer before the caller goes on.
    pub hard: Vec<(String, u16)>,
    /// Only a warning when they do not answer. Empty without a model service.
    pub warn: Vec<(String, u16)>,
}

/// The one place that decides which provider hosts a run needs, and which of them only warn.
/// `goalcup setup` and `goalcup start` both ask it; neither splits the hosts itself.
///
/// - The model service the settings name ([`ModelHost`]) is always hard: a run cannot go without
///   it, and it is the host the run records.
/// - With a model service, the harness's own hosts (its profile's `hosts`, and for setup its
///   sign-in hosts) only warn, because a run that records the service does not need them.
/// - Without a model service, the harness's first host is hard, and so are the sign-in hosts
///   for setup. The first host is the one the harness's default login uses (Codex's ChatGPT
///   sign-in is its first row). Its other hosts are possible paths only, such as an API key's
///   host, so they are not probed: a run that does not use them is never refused for them, and
///   their exchanges are recorded whatever a probe would say.
/// - Sign-in hosts (`capture.auth_hosts`) are asymmetric by caller. `goalcup setup` probes them,
///   as hard hosts without a model service and as warnings with one. `goalcup start` never
///   probes them, so a sign-in host that does not answer neither stops nor warns a start.
pub(crate) fn provider_hosts<'a>(
    model: Option<&ModelHost>,
    harness: impl IntoIterator<Item = &'a str>,
    sign_in: impl IntoIterator<Item = &'a str>,
    caller: Caller,
) -> ProviderHosts {
    let harness: Vec<(String, u16)> = harness
        .into_iter()
        .map(|host| (host.to_owned(), 443))
        .collect();
    // Only setup probes the sign-in hosts. Start leaves them out of both lists, so they never
    // refuse or warn a start.
    let sign_in: Vec<(String, u16)> = if caller == Caller::Setup {
        sign_in
            .into_iter()
            .map(|host| (host.to_owned(), 443))
            .collect()
    } else {
        Vec::new()
    };
    match model {
        Some(model) => ProviderHosts {
            hard: vec![(model.host.clone(), model.port)],
            warn: harness.into_iter().chain(sign_in).collect(),
        },
        None => ProviderHosts {
            hard: harness.into_iter().take(1).chain(sign_in).collect(),
            warn: Vec::new(),
        },
    }
}

/// The sentence for a default host that did not answer while the settings name a model service
/// instead. `missing` says which host and why (`host:port could not be reached`, optionally with
/// the reason); `service` is the model service the run records; `caller` picks the last clause.
/// Callers add their own prefix: setup prints it as a `[warn]` line, start as `warning:`.
pub(crate) fn default_host_sentence(missing: &str, service: &ModelHost, caller: Caller) -> String {
    format!(
        "{missing}; your settings use {}, so the run goes on. The run does not need that host. \
         To use it, check the network or a firewall such as Little Snitch, then {}",
        service.authority(),
        caller.rerun()
    )
}

/// [`check`] on a given config home. With `via`, the name read there stands for the key's `*`
/// part (one indirection, never two). A missing file, key or `via` value is `Ok(None)` under
/// `absent = "defaults"` and refused otherwise; a value that is present is held to every
/// refusal whatever `absent` says. `defaults` are the connector's own hosts: what a refused value
/// leaves the run to record when the file's value is deleted.
pub fn read(
    home: &Path,
    from: &ModelHostFrom,
    auth_hosts: &[String],
    defaults: &[String],
) -> Result<Option<ModelHost>, Unrecordable> {
    let path = home.join(&from.file);
    let refuse = |shown: &str, reason: String| Unrecordable {
        shown: shown.to_string(),
        reason,
        set_by: None,
        file: None,
        edit: None,
    };
    // A refused value read from the file: where it is, and what the run records without it.
    let edit_at = |key: &str| {
        Box::new(EditSetting {
            key: key.to_string(),
            path: path.clone(),
            fallback: matches!(from.absent, ModelHostAbsent::Defaults).then(|| defaults.to_vec()),
        })
    };
    let missing = |key: &str| match from.absent {
        ModelHostAbsent::Defaults => Ok(None),
        _ => Err(refuse(
            "no model service",
            format!("`{key}` in {} is not set", from.file),
        )),
    };
    // A file that is there but cannot be read is refused whatever `absent` says: it may name a
    // service, which would then pass unrecorded. Only its path is said, never its contents.
    let unreadable = |problem: FileProblem| Unrecordable {
        file: Some(Box::new(UnreadableFile {
            path: path.clone(),
            problem,
        })),
        ..refuse(
            "an address Goal Cup cannot read",
            problem.reason.to_string(),
        )
    };
    // An empty value is no value, as the harness itself takes it.
    let value_at = |key: &str| {
        read_value(&path, &from.file, key)
            .map(|v| v.filter(|v| !v.trim().is_empty()))
            .map_err(unreadable)
    };
    let key = match &from.via {
        None => from.key.clone(),
        Some(via) => {
            let Some(name) = value_at(via)? else {
                return missing(via);
            };
            // Only a plain name, never shown: it becomes one part of the key, so it may not
            // hold a `.` that would reach into another table.
            let plain = (1..=64).contains(&name.len())
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
            if !plain {
                return Err(Unrecordable {
                    edit: Some(edit_at(via)),
                    ..refuse(
                        "an address Goal Cup cannot read",
                        format!(
                            "`{via}` in {} names a value that is not a provider name",
                            from.file
                        ),
                    )
                });
            }
            from.key
                .split('.')
                .map(|part| {
                    if part == MODEL_HOST_VIA_PART {
                        name.as_str()
                    } else {
                        part
                    }
                })
                .collect::<Vec<_>>()
                .join(".")
        }
    };
    let source = format!("`{key}` in {}", from.file);
    let Some(value) = value_at(&key)? else {
        return missing(&key);
    };
    let url = match parse_url(&value) {
        Ok(url) => url,
        Err(UrlProblem::NotHttp) => {
            return Err(Unrecordable {
                edit: Some(edit_at(&key)),
                ..refuse(
                    "an address Goal Cup cannot read",
                    format!("{source} is not an http or https URL"),
                )
            });
        }
        Err(UrlProblem::SplitAuthority) => {
            return Err(Unrecordable {
                set_by: Some(source),
                edit: Some(edit_at(&key)),
                ..refuse(
                    "an address Goal Cup cannot read",
                    AUTHORITY_REASON.to_string(),
                )
            });
        }
    };
    let shown = url.shown();
    let ip = url.host.parse::<std::net::IpAddr>().ok();
    let reason = if !url.https {
        Some("it is plain http, which goes around the recorder")
    } else if ip.is_some_and(|ip| ip.is_loopback())
        || (ip.is_none() && local_host_problem(&url.host).is_some())
    {
        Some(LOOPBACK_REASON)
    } else if ip.is_some() {
        Some("it is an IP address, and Goal Cup records a service by its DNS name")
    } else if !is_capture_host(&url.host) {
        Some("it is not a DNS name Goal Cup can record")
    } else if mentions_goalcup(&url.host) || is_tracker_host(&url.host) {
        // The event server (a subdomain of the project's domain) is Goal Cup's own host too.
        Some("it is Goal Cup's own host")
    } else if auth_hosts.contains(&url.host) {
        Some("it is a sign-in host, which is never recorded")
    } else if unclean_path(&url.path) {
        Some(UNCLEAN_PATH_REASON)
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(Unrecordable {
            set_by: Some(source),
            edit: Some(edit_at(&key)),
            ..refuse(&shown, reason.into())
        });
    }
    Ok(Some(ModelHost {
        host: url.host,
        port: url.port,
        base_path: url.path,
        from: format!("{key} in {}", from.file),
        edit: Some(edit_at(&key)),
    }))
}

/// Why a service on this computer or the local network is refused. The same words serve a
/// loopback address written in the config and a name that resolves only to this computer.
const LOOPBACK_REASON: &str =
    "it is on this computer or the local network, which is never recorded";

/// Why a model address whose path the recorder cannot follow is refused. The rule itself is
/// [`goalcup_protocol::capture_policy::unclean_path`], the one rule for every capture address.
const UNCLEAN_PATH_REASON: &str = "it names an address with an unusual path that the recorder \
     cannot follow safely. Write the address plainly, for example \
     https://ai-gateway.vercel.sh/v1";

/// Why a model address whose host part holds a backslash, a space or a control byte is refused:
/// a URL parser reads such an authority as another host, or refuses it, so the host checked would
/// not be the host reached.
const AUTHORITY_REASON: &str = "its host part has a backslash, a space or a control character, \
     which a URL parser reads as a different host or refuses";

/// The refusal for a model service whose name resolves only to this computer. It gets the same
/// words a loopback address in the config gets, and the same edit advice a refused value in the
/// config file gets, so the participant is told which file holds the value.
pub fn resolves_only_here(model: &ModelHost) -> Unrecordable {
    Unrecordable {
        shown: format!("https://{}:{}", model.host, model.port),
        reason: LOOPBACK_REASON.into(),
        set_by: Some(model.from.clone()),
        file: None,
        edit: model.edit.clone(),
    }
}

/// Refuses a model service whose name resolves only to this computer: the recorder would connect
/// to it here, unrecorded as a remote host. Every caller checks it at its own moment, `goalcup
/// prep`, `goalcup start` (before anything is created) and `goalcup continue` (before an earlier
/// segment is closed), and the answer is the name's point-in-time DNS result, so a later change of
/// DNS is not seen. Under the test-only offline seam, `start` does not reach it.
pub fn refuse_if_on_this_computer(model: &ModelHost) -> Result<(), Unrecordable> {
    refuse_if_on_this_computer_with(model, crate::proxy::preflight::resolves_only_to_loopback)
}

/// [`refuse_if_on_this_computer`] with the answer to the name lookup given, so a test can answer
/// it. `resolves` is asked the model's host and port, and nothing else.
pub(crate) fn refuse_if_on_this_computer_with(
    model: &ModelHost,
    resolves: impl Fn(&str, u16) -> bool,
) -> Result<(), Unrecordable> {
    if resolves(&model.host, model.port) {
        Err(resolves_only_here(model))
    } else {
        Ok(())
    }
}

/// The string at `key` in the file at `path` (named `name` in the manifest): `Ok(None)` when
/// the file is missing or the key is absent or not a string, `Err` with the reason when the file
/// is there but cannot be read ([`read_text`]).
fn read_value(path: &Path, name: &str, key: &str) -> Result<Option<String>, FileProblem> {
    Ok(read_text(path, name)?.and_then(|text| lookup(&text, name, key, false)))
}

/// The scalar at `key` in the file at `path`, as text: a string as it is, and a boolean or a
/// number of a JSON or TOML file as written (`true`, `3`). `None` when the file is missing or
/// cannot be read, or the key is absent. For `participant.no_prompts` (D67), which is only read,
/// never written.
pub fn read_setting(path: &Path, name: &str, key: &str) -> Option<String> {
    let text = read_text(path, name).ok()??;
    lookup(&text, name, key, true)
}

/// The text of the config file at `path`: `Ok(None)` when there is none or it holds only
/// white space, `Err` with the reason and the fix when it is there but is not a regular file of
/// at most [`MAX_CONFIG_BYTES`] (a link is never followed), cannot be read, or is not UTF-8
/// JSON or TOML its parser takes. A YAML file is read by [`yaml_lookup`], which takes what it
/// cannot follow as no value.
fn read_text(path: &Path, name: &str) -> Result<Option<String>, FileProblem> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    const UNREADABLE: FileProblem = FileProblem {
        reason: "cannot be read by your user",
        fix: "Make it readable by your user (chmod u+r)",
    };
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            return Err(FileProblem {
                reason: "is a link, and Goal Cup reads only a regular file",
                fix: "Replace the link with a copy of the file",
            });
        }
        Err(_) => return Err(UNREADABLE),
    };
    let meta = file.metadata().map_err(|_| UNREADABLE)?;
    if !meta.is_file() {
        return Err(FileProblem {
            reason: "is not a regular file",
            fix: "Replace it with a regular file",
        });
    }
    if meta.len() > MAX_CONFIG_BYTES {
        return Err(FileProblem {
            reason: "is over 1 MiB, the most Goal Cup reads",
            fix: "Make it smaller than 1 MiB",
        });
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UNREADABLE)?;
    let text = String::from_utf8(bytes).map_err(|_| FileProblem {
        reason: "is not UTF-8 text",
        fix: "Save it as UTF-8 text",
    })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_string();
    if text.trim().is_empty() {
        return Ok(None);
    }
    let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("json") if serde_json::from_str::<serde_json::Value>(&text).is_err() => {
            Err(FileProblem {
                reason: "is not valid JSON",
                fix: "Fix the JSON: Goal Cup reads plain JSON, without comments or trailing commas",
            })
        }
        Some("toml") if text.parse::<toml::Table>().is_err() => Err(FileProblem {
            reason: "is not valid TOML",
            fix: "Fix the TOML",
        }),
        _ => Ok(Some(text)),
    }
}

/// The model service line of the run's details (`host.log`) when the config names none and the
/// run records the connector's own hosts (`absent = "defaults"`), in the style of
/// [`ModelHost::detail`].
pub fn defaults_detail(from: &ModelHostFrom) -> String {
    let setting = match &from.via {
        Some(via) => format!("`{}` through `{via}`", from.key),
        None => format!("`{}`", from.key),
    };
    format!(
        "model service: the connector's default hosts ({setting} not set in {})",
        from.file
    )
}

/// The scalar at `key` in `text`, read as `name`'s extension says.
fn lookup(text: &str, name: &str, key: &str, any_scalar: bool) -> Option<String> {
    let parts: Vec<&str> = key.split('.').collect();
    let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("json") => json_lookup(text, &parts, any_scalar),
        Some("toml") => toml_lookup(text, &parts, any_scalar),
        Some("yaml" | "yml") => yaml_lookup(text, &parts),
        _ => None,
    }
}

fn json_lookup(text: &str, parts: &[&str], any_scalar: bool) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut at = &value;
    for part in parts {
        at = at.as_object()?.get(*part)?;
    }
    match at {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) if any_scalar => {
            Some(at.to_string())
        }
        _ => None,
    }
}

fn toml_lookup(text: &str, parts: &[&str], any_scalar: bool) -> Option<String> {
    let table: toml::Table = text.parse().ok()?;
    let (last, tables) = parts.split_last()?;
    let mut at = &table;
    for part in tables {
        at = at.get(*part)?.as_table()?;
    }
    match at.get(*last)? {
        toml::Value::String(s) => Some(s.clone()),
        toml::Value::Boolean(b) if any_scalar => Some(b.to_string()),
        toml::Value::Integer(i) if any_scalar => Some(i.to_string()),
        _ => None,
    }
}

/// The scalar at `parts` in a YAML document of block mappings (`key:` lines nested by
/// indentation), as a harness's `yaml.safe_dump` writes them. A flow collection, an anchor, a
/// tag or a block scalar at the key is `None`; so is a tab in the indentation.
fn yaml_lookup(text: &str, parts: &[&str]) -> Option<String> {
    let mut depth = 0;
    // The indentation of the last key matched (`None`: the document's top level), and of the
    // keys of the mapping searched now (set by its first line).
    let mut parent: Option<usize> = None;
    let mut level: Option<usize> = None;
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let body = line.trim_start_matches(' ');
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        if body.starts_with('\t') {
            return None;
        }
        let indent = line.len() - body.len();
        if indent == 0 && (body == "---" || body.starts_with("--- ") || body == "...") {
            if depth > 0 {
                return None;
            }
            continue;
        }
        if parent.is_some_and(|p| indent <= p) {
            // The mapping that should hold the next part has ended without it.
            return None;
        }
        let at = *level.get_or_insert(indent);
        if indent > at {
            // Inside the value of a key this lookup does not follow.
            continue;
        }
        if indent < at {
            return None;
        }
        let Some((key, rest)) = yaml_key(body) else {
            continue;
        };
        if key != parts[depth] {
            continue;
        }
        if depth + 1 == parts.len() {
            return yaml_scalar(rest);
        }
        let rest = rest.trim();
        if !rest.is_empty() && !rest.starts_with('#') {
            // A value on the same line: not a nested mapping.
            return None;
        }
        depth += 1;
        parent = Some(indent);
        level = None;
    }
    None
}

/// `key: rest` of one mapping line; `None` for a sequence item or a line with no key.
fn yaml_key(body: &str) -> Option<(String, &str)> {
    if body == "-" || body.starts_with("- ") || body.starts_with(['?', '[', '{']) {
        return None;
    }
    if let Some(quote) = body.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let close = body[1..].find(quote)? + 1;
        let rest = body[close + 1..].trim_start_matches(' ');
        let rest = rest.strip_prefix(':')?;
        return Some((body[1..close].to_string(), rest));
    }
    let bytes = body.as_bytes();
    let colon = (0..bytes.len())
        .find(|&i| bytes[i] == b':' && (i + 1 == bytes.len() || bytes[i + 1] == b' '))?;
    Some((body[..colon].trim_end().to_string(), &body[colon + 1..]))
}

/// A plain, single-quoted or double-quoted scalar on one line; `None` for anything else.
fn yaml_scalar(rest: &str) -> Option<String> {
    let s = rest.trim();
    if let Some(inner) = s.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => return Some(out),
                '\\' => out.push(chars.next()?),
                c => out.push(c),
            }
        }
        return None;
    }
    if let Some(inner) = s.strip_prefix('\'') {
        let mut out = String::new();
        let mut chars = inner.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    out.push('\'');
                    continue;
                }
                return Some(out);
            }
            out.push(c);
        }
        return None;
    }
    let plain = match s.find(" #") {
        Some(at) => s[..at].trim_end(),
        None => s,
    };
    if plain.is_empty()
        || plain.starts_with(['|', '>', '{', '[', '&', '*', '!', '%', '@', '`', '#'])
        || plain == "~"
        || plain.eq_ignore_ascii_case("null")
    {
        return None;
    }
    Some(plain.to_string())
}

/// What the reader keeps of a URL: never its userinfo or query.
struct Url {
    https: bool,
    host: String,
    port: u16,
    path: String,
}

impl Url {
    /// `scheme://host:port`, the only form a refusal shows.
    fn shown(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        if self.host.contains(':') {
            format!("{scheme}://[{}]:{}", self.host, self.port)
        } else {
            format!("{scheme}://{}:{}", self.host, self.port)
        }
    }
}

/// Why a value is not a URL the reader keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UrlProblem {
    /// Not an `http` or `https` URL with a host, a usable port and a short path.
    NotHttp,
    /// The host part holds a backslash, a space or a control byte ([`AUTHORITY_REASON`]).
    SplitAuthority,
}

/// An `http` or `https` URL with a host.
fn parse_url(value: &str) -> Result<Url, UrlProblem> {
    let value = value.trim();
    let (https, rest) = match value.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => (true, rest),
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => (false, rest),
        _ => return Err(UrlProblem::NotHttp),
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    // A WHATWG parser ends the authority at a backslash (it reads it as `/`) and removes or refuses
    // a control byte or a space, so an authority holding one would be sent to another host than the
    // one checked here. Refused whole, the userinfo included.
    if authority
        .bytes()
        .any(|b| b <= b' ' || b == 0x7f || b == b'\\')
    {
        return Err(UrlProblem::SplitAuthority);
    }
    let path_end = tail.find(['?', '#']).unwrap_or(tail.len());
    let path = &tail[..path_end];
    if path.bytes().any(|b| b <= b' ' || b == 0x7f) || path.len() > 512 {
        return Err(UrlProblem::NotHttp);
    }
    // A userinfo is dropped here, before anything can show it.
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = if let Some(literal) = hostport.strip_prefix('[') {
        let (literal, after) = literal.split_once(']').ok_or(UrlProblem::NotHttp)?;
        let port = match after {
            "" => None,
            p => Some(p.strip_prefix(':').ok_or(UrlProblem::NotHttp)?),
        };
        (literal.to_ascii_lowercase(), port)
    } else {
        match hostport.rsplit_once(':') {
            Some((host, port)) => (host.to_ascii_lowercase(), Some(port)),
            None => (hostport.to_ascii_lowercase(), None),
        }
    };
    let host = host.strip_suffix('.').unwrap_or(&host).to_string();
    if host.is_empty() || host.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(UrlProblem::NotHttp);
    }
    let port = match port {
        None => {
            if https {
                443
            } else {
                80
            }
        }
        Some(p) if !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()) => {
            match p.parse::<u16>() {
                Ok(0) | Err(_) => return Err(UrlProblem::NotHttp),
                Ok(port) => port,
            }
        }
        Some(_) => return Err(UrlProblem::NotHttp),
    };
    Ok(Url {
        https,
        host,
        port,
        path: path.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `from` (a `capture.model_host_from` table, TOML) read against a home holding `name`
    /// with `text`.
    fn read_in(from: &str, name: &str, text: &str) -> Result<Option<ModelHost>, Unrecordable> {
        let from: ModelHostFrom = toml::from_str(from).unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(name), text).unwrap();
        read(
            home.path(),
            &from,
            &["auth.example.test".to_string()],
            &["api.default.example".to_string()],
        )
    }

    const CODEX: &str = r#"
file = "config.toml"
key = "model_providers.*.base_url"
via = "model_provider"
absent = "defaults"
"#;

    #[test]
    fn an_indirection_reads_the_named_table() {
        let config = "model_provider = \"vercel\"\n\
                      [model_providers.vercel]\n\
                      base_url = \"https://ai-gateway.vercel.sh/codex/v1\"\n\
                      wire_api = \"responses\"\n";
        let m = read_in(CODEX, "config.toml", config).unwrap().unwrap();
        assert_eq!(m.host, "ai-gateway.vercel.sh");
        assert_eq!(m.base_path, "/codex/v1");
        assert!(
            m.from.contains("model_providers.vercel.base_url"),
            "{}",
            m.from
        );
        // Codex's `wire_api = "responses"` posts to `<base>/responses`, which carries its hint.
        let rule = m.approved();
        assert_eq!(
            crate::proxy::metadata::select(&rule.policy, "/codex/v1/responses")
                .and_then(|p| p.protocol_hint.clone())
                .as_deref(),
            Some("openai-responses")
        );
    }

    #[test]
    fn a_missing_value_falls_back_only_when_asked() {
        const CLAUDE: &str = r#"file = "settings.json"
key = "env.ANTHROPIC_BASE_URL""#;
        let defaults = format!("{CLAUDE}\nabsent = \"defaults\"");
        assert_eq!(read_in(&defaults, "settings.json", "{}"), Ok(None));
        let missing = read_in(&defaults, "other.json", "{}");
        assert_eq!(missing, Ok(None), "no settings.json at all");
        assert_eq!(
            read_in(&defaults, "settings.json", " \n"),
            Ok(None),
            "an empty file"
        );
        let refused = read_in(CLAUDE, "settings.json", "{}").unwrap_err();
        assert!(refused.reason.contains("is not set"), "{refused:?}");
    }

    #[test]
    fn a_file_that_is_there_but_unreadable_is_refused_even_with_defaults() {
        let from = r#"file = "settings.json"
key = "env.ANTHROPIC_BASE_URL"
absent = "defaults""#;
        let broken = r#"{"env":{"ANTHROPIC_BASE_URL":"https://gw.example/sk-secret"#;
        let refused = read_in(from, "settings.json", broken).unwrap_err();
        let said = refused.sentence("Claude Code");
        assert!(said.contains("/settings.json, where Claude Code"), "{said}");
        assert!(said.contains("is not valid JSON"), "{said}");
        assert!(!said.contains("gw.example"), "{said}");
    }

    #[test]
    fn a_builtin_provider_without_a_table_falls_back() {
        for config in ["model_provider = \"openai\"\n", "model = \"gpt-5\"\n"] {
            assert_eq!(read_in(CODEX, "config.toml", config), Ok(None), "{config}");
        }
    }

    #[test]
    fn a_present_unrecordable_value_is_refused_even_with_defaults() {
        let from = r#"file = "settings.json"
key = "env.ANTHROPIC_BASE_URL"
absent = "defaults""#;
        for (url, reason) in [
            (
                "http://localhost:4000",
                "it is plain http, which goes around the recorder",
            ),
            (
                "https://auth.example.test/v1",
                "it is a sign-in host, which is never recorded",
            ),
            (
                "https://203.0.113.7/v1",
                "it is an IP address, and Goal Cup records a service by its DNS name",
            ),
            (
                "https://gateway/v1",
                "it is on this computer or the local network, which is never recorded",
            ),
        ] {
            let settings = format!(r#"{{"env":{{"ANTHROPIC_BASE_URL":"{url}"}}}}"#);
            let refused = read_in(from, "settings.json", &settings).unwrap_err();
            assert_eq!(refused.reason, reason);
        }
    }

    #[test]
    fn a_via_value_that_is_not_a_name_is_refused() {
        for name in ["../x", "a.b"] {
            let config = format!(
                "model_provider = \"{name}\"\n[model_providers.x]\nbase_url = \"https://x.example\"\n"
            );
            let refused = read_in(CODEX, "config.toml", &config).unwrap_err();
            assert!(
                refused.reason.contains("is not a provider name"),
                "{refused:?}"
            );
            assert!(!refused.reason.contains(name), "the value is never shown");
        }
    }

    #[test]
    fn the_yaml_reader_finds_one_nested_scalar_and_nothing_else() {
        let doc = "\
# a harness config
model:
  default: some-model
  provider: some-provider
  base_url: https://api.models.example.test/v1   # set by setup
  nested:
    base_url: https://wrong.example
other:
  base_url: https://other.example
sessions:
  write_json_snapshots: true
";
        assert_eq!(
            yaml_lookup(doc, &["model", "base_url"]).as_deref(),
            Some("https://api.models.example.test/v1")
        );
        assert_eq!(
            yaml_lookup(doc, &["model", "nested", "base_url"]).as_deref(),
            Some("https://wrong.example")
        );
        assert_eq!(yaml_lookup(doc, &["model", "api_key"]), None);
        assert_eq!(yaml_lookup(doc, &["model"]), None, "a mapping is no scalar");
        assert_eq!(
            yaml_lookup(
                "model:\n  base_url: \"https://q.example/v1\"\n",
                &["model", "base_url"]
            )
            .as_deref(),
            Some("https://q.example/v1")
        );
        assert_eq!(
            yaml_lookup(
                "model:\n  'base_url': 'https://it''s.example'\n",
                &["model", "base_url"]
            )
            .as_deref(),
            Some("https://it's.example")
        );
        assert_eq!(
            yaml_lookup("model: gpt-5\nbase_url: x\n", &["model", "base_url"]),
            None,
            "a string model holds no key"
        );
        assert_eq!(
            yaml_lookup(
                "model:\nbase_url: https://top.example\n",
                &["model", "base_url"]
            ),
            None,
            "a key of the top level is not the model's"
        );
        assert_eq!(
            yaml_lookup(
                "model: {base_url: https://flow.example}\n",
                &["model", "base_url"]
            ),
            None
        );
        assert_eq!(
            yaml_lookup(
                "model:\n  base_url: |\n    https://block.example\n",
                &["model", "base_url"]
            ),
            None
        );
    }

    #[test]
    fn json_and_toml_are_read_by_their_parsers() {
        assert_eq!(
            json_lookup(
                r#"{"model":{"base_url":"https://j.example/v1"}}"#,
                &["model", "base_url"],
                false
            )
            .as_deref(),
            Some("https://j.example/v1")
        );
        assert_eq!(
            json_lookup(r#"{"model":{"base_url":3}}"#, &["model", "base_url"], false),
            None
        );
        assert_eq!(
            toml_lookup(
                "[model]\nbase_url = \"https://t.example\"\n",
                &["model", "base_url"],
                false
            )
            .as_deref(),
            Some("https://t.example")
        );
        assert_eq!(
            toml_lookup("base_url = \"https://t.example\"\n", &["base_url"], false).as_deref(),
            Some("https://t.example")
        );
    }

    #[test]
    fn a_setting_reads_booleans_and_numbers_as_text_and_never_a_table() {
        let json = r#"{"skip":true,"n":3,"permissions":{"defaultMode":"bypassPermissions"}}"#;
        assert_eq!(json_lookup(json, &["skip"], true).as_deref(), Some("true"));
        assert_eq!(json_lookup(json, &["n"], true).as_deref(), Some("3"));
        assert_eq!(json_lookup(json, &["skip"], false), None);
        assert_eq!(
            json_lookup(json, &["permissions", "defaultMode"], true).as_deref(),
            Some("bypassPermissions")
        );
        assert_eq!(json_lookup(json, &["permissions"], true), None);
        let toml = "approval_policy = \"never\"\nflag = false\n[t]\nx = 1\n";
        assert_eq!(
            toml_lookup(toml, &["approval_policy"], true).as_deref(),
            Some("never")
        );
        assert_eq!(toml_lookup(toml, &["flag"], true).as_deref(), Some("false"));
        assert_eq!(toml_lookup(toml, &["t"], true), None);
    }

    #[test]
    fn a_url_keeps_its_host_port_and_path_and_drops_its_userinfo() {
        let url = parse_url("https://user:hunter2@Models.Example.TEST.:8443/v1/?x=1#f").unwrap();
        assert!(url.https);
        assert_eq!(url.host, "models.example.test");
        assert_eq!(url.port, 8443);
        assert_eq!(url.path, "/v1/");
        assert_eq!(url.shown(), "https://models.example.test:8443");
        assert!(!url.shown().contains("hunter2"));
        assert_eq!(parse_url("https://openrouter.ai/api/v1").unwrap().port, 443);
        assert_eq!(
            parse_url("http://localhost:11434/v1").unwrap().shown(),
            "http://localhost:11434"
        );
        assert_eq!(
            parse_url("http://[::1]:8080").unwrap().shown(),
            "http://[::1]:8080"
        );
        for bad in [
            "api.models.example.test",
            "ftp://x.example",
            "https://",
            "https://x:0",
            "https://x:99999",
            "https://x:a",
        ] {
            assert!(parse_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_host_rule_hints_known_wire_paths_and_records_the_rest_of_the_base() {
        let host = ModelHost {
            host: "models.example.test".into(),
            port: 443,
            base_path: "/v1/".into(),
            from: "model.base_url in config.yaml".into(),
            edit: None,
        };
        let rule = host.approved();
        assert_eq!(rule.host, "models.example.test");
        let hint = |path: &str| {
            crate::proxy::metadata::select(&rule.policy, path).map(|p| p.protocol_hint.clone())
        };
        assert_eq!(
            hint("/v1/chat/completions"),
            Some(Some("openai-chat-completions".into()))
        );
        assert_eq!(
            hint("/v1/messages"),
            Some(Some("anthropic-messages".into()))
        );
        assert_eq!(hint("/v1/models"), Some(None));
        assert_eq!(hint("/v2/chat/completions"), None);
        // A URL with no path names the versioned API; the rows only pick hints, and the proxy
        // records every exchange on the host whether a row names it or not.
        let bare = ModelHost {
            base_path: String::new(),
            ..host.clone()
        }
        .approved();
        let bare_hint =
            |path: &str| crate::proxy::metadata::select(&bare.policy, path).map(|p| p.prefix);
        assert_eq!(bare_hint("/v1/messages"), Some(false));
        assert_eq!(bare_hint("/v1/models"), Some(true));
        assert_eq!(bare_hint("/healthz"), None);
        assert!(!bare.policy.paths.iter().any(|p| p.path == "/"));
        assert_eq!(
            host.detail(),
            "model service: models.example.test:443 (model.base_url in config.yaml)"
        );
    }

    /// The hints the host rule hands out meet the decoders: for every base a harness can name
    /// (a gateway prefix, the `/v1` the SDKs add, or none), each path the rule hints is selected
    /// with that hint, and a stream carrying it is accepted by a registered decoder. The decoders
    /// take their streams by hint here. This test does not reach the per-record gate: the
    /// Anthropic decoder also reads only an endpoint ending in `/messages` (its `ENDPOINT_SUFFIX`),
    /// and only a POST with `application/json` (its `select_gate_tests` module pins that gate).
    #[test]
    fn every_hinted_path_is_read_by_its_decoder() {
        use goalcup_protocol::capture::{CaptureSource, StreamKey};
        use goalcup_verifier::capture_analysis::api::StreamOpen;
        use goalcup_verifier::capture_analysis::registry::DecoderRegistry;

        let registry = DecoderRegistry::builtin();
        let key = StreamKey {
            producer: "proxy".into(),
            stream: 1,
        };
        // Per base, not a total: a new wire hint or base adds rows without editing this test.
        for base in ["/claude-code", "/codex/v1", "/v1", ""] {
            let host = ModelHost {
                host: "models.example.test".into(),
                port: 443,
                base_path: base.into(),
                from: "model.base_url in config.yaml".into(),
                edit: None,
            };
            let rule = host.approved();
            let mut hinted_here = 0;
            for row in rule.policy.paths.iter().filter(|row| !row.prefix) {
                let Some(hint) = row.protocol_hint.as_deref() else {
                    continue;
                };
                hinted_here += 1;
                let picked = crate::proxy::metadata::select(&rule.policy, &row.path)
                    .unwrap_or_else(|| panic!("{base:?}: {} is selected", row.path));
                assert_eq!(picked.protocol_hint.as_deref(), Some(hint), "{}", row.path);
                let open = StreamOpen::new(
                    &key,
                    CaptureSource::WireRequest,
                    Some(hint),
                    Some("application/json"),
                );
                let readers = registry.accepting(&open);
                assert!(
                    !readers.is_empty(),
                    "{base:?}: no decoder accepts {hint} for {}",
                    row.path
                );
                if hint == "anthropic-messages" {
                    assert!(row.path.ends_with("/messages"), "{}", row.path);
                }
            }
            assert!(hinted_here > 0, "{base:?}: no path is hinted");
        }
    }

    /// `url` as the base URL under a neutral settings key, read the way a run reads it.
    fn read_model_url(url: &str) -> Result<Option<ModelHost>, Unrecordable> {
        let from = r#"file = "config.json"
key = "env.MODEL_BASE_URL"
absent = "defaults""#;
        let config = serde_json::json!({ "env": { "MODEL_BASE_URL": url } }).to_string();
        read_in(from, "config.json", &config)
    }

    #[test]
    fn an_unclean_base_path_is_refused() {
        for url in [
            "https://gateway.example.test/x/../y",
            "https://g.example.com/a/%2e%2e/v1",
            "https://g.example.com/a/%2E/v1",
            "https://g.example.com/a\\b",
            "https://g.example.com/caf\u{e9}/v1",
        ] {
            let refused = read_model_url(url).unwrap_err();
            assert!(
                refused.reason.contains("unusual path"),
                "{url}: {refused:?}"
            );
            assert!(
                refused.reason.contains("https://ai-gateway.vercel.sh/v1"),
                "the refusal says what to type: {refused:?}"
            );
        }
        for url in [
            "https://gateway.example.test/y",
            "https://gateway.example.test/v1/y",
            "https://gateway.example.test",
        ] {
            assert!(matches!(read_model_url(url), Ok(Some(_))), "{url}");
        }
    }

    /// A backslash, a space or a control byte in the authority is refused under its own reason: a
    /// WHATWG parser would send the harness to another host than the one checked here. A plain
    /// userinfo is still read.
    #[test]
    fn an_authority_that_splits_differently_is_refused() {
        for url in [
            "https://127.0.0.1\\@gateway.example.test/v1",
            "https://gateway.example.test\\@127.0.0.1/v1",
            "https://gateway.example.test\\v1",
            "https://gateway\texample.test/v1",
            "https://gateway example.test/v1",
        ] {
            let refused = read_model_url(url).unwrap_err();
            assert!(
                refused.reason.starts_with("its host part has a backslash"),
                "{url}: {refused:?}"
            );
            assert_eq!(
                refused.set_by.as_deref(),
                Some("`env.MODEL_BASE_URL` in config.json")
            );
            assert!(
                !refused.sentence("Harness").contains("is not an http"),
                "{url}"
            );
        }
        assert!(matches!(
            read_model_url("https://key@gateway.example.test/v1"),
            Ok(Some(_))
        ));
    }

    #[test]
    fn the_event_server_is_never_a_model_host() {
        for url in [
            "https://ingest.agentcommunity.org/v1",
            "https://agentcommunity.org/v1",
        ] {
            let refused = read_model_url(url).unwrap_err();
            assert_eq!(refused.reason, "it is Goal Cup's own host", "{url}");
        }
        assert!(matches!(
            read_model_url("https://gateway.example.test/v1"),
            Ok(Some(_))
        ));
    }

    /// The check is by name: a name that resolves only to this computer is refused with the
    /// loopback sentence, naming the host and port it checked, and one that resolves elsewhere is
    /// not. The lookup is injected here; the start-up ordering is not tested by this.
    #[test]
    fn refuse_if_on_this_computer_uses_the_name_and_reason() {
        let model = ModelHost {
            host: "gateway.example.test".into(),
            port: 443,
            base_path: String::new(),
            from: "env.MODEL_BASE_URL in config.json".into(),
            edit: None,
        };
        let asked = std::cell::RefCell::new(Vec::new());
        let refused = refuse_if_on_this_computer_with(&model, |host, port| {
            asked.borrow_mut().push((host.to_string(), port));
            true
        })
        .unwrap_err();
        assert_eq!(
            *asked.borrow(),
            vec![("gateway.example.test".to_string(), 443)],
            "the name is the one checked"
        );
        assert_eq!(refused.shown, "https://gateway.example.test:443");
        assert_eq!(refused.reason, LOOPBACK_REASON);
        assert_eq!(
            refused.set_by.as_deref(),
            Some("env.MODEL_BASE_URL in config.json")
        );
        assert!(
            refuse_if_on_this_computer_with(&model, |_, _| false).is_ok(),
            "a name that resolves elsewhere is not refused here"
        );
    }

    /// A name that resolves only to this computer and comes from the config file is refused with
    /// the file's full path and the key to edit, the same advice a refused value in the file
    /// gets, with the connector's own hosts as the fallback. Without that advice the participant
    /// is sent to setup and never told which file holds the value.
    #[test]
    fn a_name_on_this_computer_from_a_file_says_which_file_to_edit() {
        let model = ModelHost {
            host: "gateway.example.test".into(),
            port: 443,
            base_path: "/v1".into(),
            from: "env.MODEL_BASE_URL in config.json".into(),
            edit: Some(Box::new(EditSetting {
                key: "env.MODEL_BASE_URL".into(),
                path: "/home/user/config/config.json".into(),
                fallback: Some(vec!["api.models.example.test".into()]),
            })),
        };
        let refused = refuse_if_on_this_computer_with(&model, |_, _| true).unwrap_err();
        let text = refused.refusal("Harness", "harness");
        assert!(
            text.contains(
                "Edit or delete env.MODEL_BASE_URL in /home/user/config/config.json; \
                 without it Goal Cup records api.models.example.test."
            ),
            "{text}"
        );
        assert!(!text.contains("goalcup setup"), "{text}");

        let no_fallback = ModelHost {
            edit: Some(Box::new(EditSetting {
                key: "env.MODEL_BASE_URL".into(),
                path: "/home/user/config/config.json".into(),
                fallback: None,
            })),
            ..model
        };
        let refused = refuse_if_on_this_computer_with(&no_fallback, |_, _| true).unwrap_err();
        assert!(
            refused.refusal("Harness", "harness").contains(
                "Edit env.MODEL_BASE_URL in /home/user/config/config.json to an https address \
                 Goal Cup can record."
            ),
            "{no_fallback:?}"
        );
    }

    fn named(host: &str, port: u16) -> (String, u16) {
        (host.to_string(), port)
    }

    /// Without a service the harness's first host is hard and its other hosts are not probed.
    /// With one, the service is the only hard host and the harness's own hosts only warn.
    /// Sign-in hosts are setup's alone.
    #[test]
    fn start_never_probes_sign_in_hosts_and_setup_does() {
        let model = ModelHost {
            host: "gateway.example.test".to_string(),
            port: 443,
            base_path: String::new(),
            from: String::new(),
            edit: None,
        };
        let harness = ["api.default.example"];
        let sign_in = ["auth.example.test"];

        let setup_bare = provider_hosts(None, harness, sign_in, Caller::Setup);
        assert_eq!(
            setup_bare,
            ProviderHosts {
                hard: vec![
                    named("api.default.example", 443),
                    named("auth.example.test", 443)
                ],
                warn: Vec::new(),
            }
        );
        let start_bare = provider_hosts(None, harness, sign_in, Caller::Start);
        assert_eq!(
            start_bare,
            ProviderHosts {
                hard: vec![named("api.default.example", 443)],
                warn: Vec::new(),
            }
        );
        let setup_named = provider_hosts(Some(&model), harness, sign_in, Caller::Setup);
        assert_eq!(
            setup_named,
            ProviderHosts {
                hard: vec![named("gateway.example.test", 443)],
                warn: vec![
                    named("api.default.example", 443),
                    named("auth.example.test", 443)
                ],
            }
        );
        let start_named = provider_hosts(Some(&model), harness, sign_in, Caller::Start);
        assert_eq!(
            start_named,
            ProviderHosts {
                hard: vec![named("gateway.example.test", 443)],
                warn: vec![named("api.default.example", 443)],
            }
        );
    }

    /// Codex's two shipped rows, ChatGPT's first: without a service only the first is hard, so an
    /// API-key host that does not answer neither refuses nor warns a run, for either caller. The
    /// sign-in hosts keep their setup-only probe.
    #[test]
    fn a_second_default_host_is_never_probed_without_a_service() {
        let harness = ["chatgpt.com", "api.openai.com"];
        let sign_in = ["auth.example.test"];
        assert_eq!(
            provider_hosts(None, harness, sign_in, Caller::Start),
            ProviderHosts {
                hard: vec![named("chatgpt.com", 443)],
                warn: Vec::new(),
            }
        );
        assert_eq!(
            provider_hosts(None, harness, sign_in, Caller::Setup),
            ProviderHosts {
                hard: vec![named("chatgpt.com", 443), named("auth.example.test", 443)],
                warn: Vec::new(),
            }
        );
    }

    /// Both callers' warnings come from one sentence; only the last clause names the command the
    /// participant runs again.
    #[test]
    fn the_default_host_warning_names_the_command_that_is_run_again() {
        let model = ModelHost {
            host: "gateway.example.test".to_string(),
            port: 443,
            base_path: String::new(),
            from: String::new(),
            edit: None,
        };
        let missing = "api.default.example:443 could not be reached";
        let setup = default_host_sentence(missing, &model, Caller::Setup);
        let start = default_host_sentence(missing, &model, Caller::Start);
        assert!(setup.starts_with(missing) && start.starts_with(missing));
        assert!(setup.contains("your settings use gateway.example.test:443, so the run goes on"));
        assert!(setup.ends_with("then run setup again"), "{setup}");
        assert!(start.ends_with("then start again"), "{start}");
    }
}
