//! Connectors and host lists (PRD-11, seam S3).
//!
//! A connector is one small TOML file of data per harness; every harness goes through the same
//! code. `load` reads a built-in connector by id or a file (seam S16), `plan` turns it into the
//! launch plan the watchdog spawns from.

pub mod builtin;
pub(crate) mod hints;
pub mod hosts;
pub mod launch;
pub mod schema;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub use hosts::{Decision, Host, HostList, Kind, Skipped, Source};
pub use launch::{HomeLock, write_settings};
pub use schema::{Connector, FieldValue, MaskPattern, MaskRule, SettingRow};

/// What the plan reads besides the connector, gathered by PRD-10.
#[derive(Debug, Clone)]
pub struct Env {
    /// The participant's whole environment (owner question 2).
    pub vars: BTreeMap<String, String>,
    pub workspace: PathBuf,
    /// `goalcup continue`.
    pub resume: bool,
    /// `since` for native rows.
    pub run_start: SystemTime,
    /// `record-hosts.txt` lives here.
    pub goalcup_home: PathBuf,
    /// goalcup's own server, always `never`.
    pub server_host: String,
}

/// Where a connector came from: built into the binary, or a `--connector` file (S16).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectorSource {
    #[default]
    Builtin,
    File,
}

/// The connector named in the `start` event: `connector`, `connector_source`, `connector_sha256`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorRef {
    pub id: String,
    pub name: String,
    /// SHA-256 of the exact TOML bytes used, 64 hex.
    pub sha256: String,
    pub source: ConnectorSource,
}

/// Why a start cannot go ahead, shown by PRD-10 before any process starts. It names files, keys
/// and programs, never a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A `refuse_if_set` key holds a value: a setting that would route around the recorder.
    SettingSet { file: PathBuf, key: String },
    /// Another run holds this harness's isolated home.
    HomeLocked { home: PathBuf },
    /// `program --version` does not hold the connector's `version` prefix.
    VersionMismatch { program: String, want: String },
}

/// How a session file is read (PRD-04 section 2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Form {
    /// Read as it grows.
    #[default]
    Append,
    /// Read whole each time it changes and settles.
    Snapshot,
    /// A database, copied as a set during the run and at every seal.
    Final,
}

/// One session-file row for the native reader (PRD-04), templates resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeSource {
    pub name: String,
    /// Absolute, from `{home}` or `{workspace}`.
    pub root: PathBuf,
    /// `/`-separated, relative to `root`; `**` is a whole component.
    pub pattern: String,
    pub form: Form,
    /// The run start, the same for every row.
    pub since: SystemTime,
    pub warn_missing: bool,
}

/// The connector's session-file rows, in file order (the first matching row reads a file).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NativeSources(pub Vec<NativeSource>);

/// Everything the watchdog needs to start the harness (PRD-11 section 5).
#[derive(Debug)]
pub struct LaunchPlan {
    pub connector: ConnectorRef,
    pub harness_version: Option<String>,
    /// Resolved on the participant's `PATH`, else the bare name.
    pub program: PathBuf,
    /// `args`, or `args` and `resume`, templates filled.
    pub argv: Vec<String>,
    /// The complete harness environment, minus the port and the CA paths (`finish_env`).
    pub env: BTreeMap<String, String>,
    /// The isolated home, created unless `refusal` is a held lock.
    pub home: PathBuf,
    /// Held as long as the plan lives.
    pub lock: HomeLock,
    /// Run by the watchdog before spawn (S8). Each entry holds the arguments without the program:
    /// the watchdog runs `program` followed by them.
    pub before: Vec<Vec<String>>,
    /// Run by the watchdog after the harness exits, before the native finish. Arguments without
    /// the program, as in `before`.
    pub after: Vec<Vec<String>>,
    /// The connector's own CA variables; `finish_env` points each at the run CA.
    pub ca_env: Vec<String>,
    /// Shared, participant and config hosts.
    pub hosts: HostList,
    pub native: NativeSources,
    pub mask_rules: Vec<MaskRule>,
    /// Empty in the default build: no `secret_files` are read (owner question 3 has no yes).
    pub secret_values: Vec<(String, Vec<u8>)>,
    /// `local_model` notes, written once per entry after `start`.
    pub notes: Vec<serde_json::Value>,
    /// Hint and refusal files there but not read (linked, over 1 MiB, unreadable, unparsable):
    /// PRD-10 shows one start line for each (section 2) and logs it.
    pub unread: Vec<PathBuf>,
    /// Log lines: a hint skipped (file or variable, key, reason), a `never` host left out, a
    /// setting row not written. They name files, keys and hosts, never a value.
    pub log: Vec<String>,
    /// A `refuse_if_set` hit or a held lock: file and key, never the value.
    pub refusal: Option<Refusal>,
}

impl LaunchPlan {
    /// Adds the proxy and CA variables once the port and the two PEM files exist.
    pub fn finish_env(&mut self, port: u16, run_ca: &Path, bundle: &Path) {
        launch::finish_env(&mut self.env, &self.ca_env, port, run_ca, bundle);
    }
}

/// Why a connector or a host list cannot load. Names the file and the key, never a value.
#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    #[error("no built-in connector `{id}`")]
    UnknownId { id: String },
    #[error("{file}: cannot read it ({kind})")]
    Read {
        file: String,
        kind: std::io::ErrorKind,
    },
    #[error("{file}: line {line}: {message}")]
    Parse {
        file: String,
        line: usize,
        message: String,
    },
    #[error("{file}: {key}: {reason}")]
    Invalid {
        file: String,
        key: String,
        reason: &'static str,
    },
    #[error("{file}: {key}: the connector sets a model address, which goalcup never does")]
    ModelAddress { file: String, key: String },
}

fn read(path: &Path) -> Result<Vec<u8>, ConnectorError> {
    std::fs::read(path).map_err(|e| ConnectorError::Read {
        file: path.display().to_string(),
        kind: e.kind(),
    })
}

/// A built-in connector by id, or a connector file (seam S16) when the argument holds a `/` or
/// ends `.toml`. The hosts come from `hosts_file`, else the built-in list.
pub fn load(id_or_path: &str, hosts_file: Option<&Path>) -> Result<Connector, ConnectorError> {
    let hosts = match hosts_file {
        Some(path) => HostList::from_file(&read(path)?, &path.display().to_string())?,
        None => HostList::from_file(builtin::HOSTS.as_bytes(), "connectors/model-hosts.toml")?,
    };
    let mut connector = if id_or_path.contains('/') || id_or_path.ends_with(".toml") {
        let path = Path::new(id_or_path);
        let mut c = schema::parse(&read(path)?, &path.display().to_string())?;
        c.source = ConnectorSource::File;
        c
    } else {
        let (id, text) = builtin::CONNECTORS
            .iter()
            .find(|(id, _)| *id == id_or_path)
            .ok_or_else(|| ConnectorError::UnknownId {
                id: id_or_path.to_string(),
            })?;
        schema::parse(text.as_bytes(), &format!("connectors/{id}.toml"))?
    };
    connector.hosts = hosts;
    Ok(connector)
}

/// The launch plan for `connector` in its isolated `home` (`$GOALCUP_HOME/homes/<id>`). It never
/// fails: what a participant can fix is a `refusal`.
pub fn plan(connector: &Connector, env: &Env, home: &Path) -> LaunchPlan {
    // A held lock means another run uses this home: the plan then creates, writes and runs
    // nothing there, so the running harness is not disturbed.
    let (lock, lock_refusal) = match HomeLock::take(home) {
        Ok(lock) => (lock, None),
        Err(refusal) => (HomeLock::default(), Some(refusal)),
    };
    let free = lock_refusal.is_none();
    let fill = |s: &str| launch::fill(s, home, &env.workspace);
    if free {
        launch::make_dir(home);
        for template in connector.home.values() {
            launch::make_dir(Path::new(&fill(template)));
        }
    }
    let mut argv: Vec<String> = connector.args.iter().map(|a| fill(a)).collect();
    if env.resume {
        argv.extend(connector.resume.iter().map(|a| fill(a)));
    }
    let harness_env = launch::build_env(connector, env, home);
    let program = launch::resolve_program(&connector.program, &env.vars);
    let harness_version = free
        .then(|| launch::harness_version(&program, &harness_env))
        .flatten();
    let version_refusal = match (&connector.version, &harness_version) {
        (Some(want), Some(got)) if !got.contains(want.as_str()) => Some(Refusal::VersionMismatch {
            program: connector.program.clone(),
            want: want.clone(),
        }),
        _ => None,
    };
    // `never` first, so `decide` is the one never test for participant and config hosts.
    let mut hosts = connector.hosts.clone();
    hosts.add_never(&env.server_host);
    let mut reader = hints::Reader {
        env,
        home,
        unread: Vec::new(),
        log: hosts.add_participant(&env.goalcup_home),
    };
    let (config, notes) = reader.collect(connector, &hosts);
    hosts.add_config(config);
    let refusal = lock_refusal
        .or(version_refusal)
        .or_else(|| reader.refusal(connector));
    let (unread, mut log) = (reader.unread, reader.log);

    let rows = connector.native.iter();
    let native = NativeSources(
        rows.map(|row| NativeSource {
            name: row.name.clone(),
            root: PathBuf::from(fill(&row.root)),
            pattern: row.pattern.clone(),
            form: row.form,
            since: env.run_start,
            warn_missing: row.warn_missing,
        })
        .collect(),
    );

    // A row not written is a log line, never a refusal (section 5).
    if free {
        let skipped = write_settings(home, &connector.update_off).into_iter();
        log.extend(skipped.map(|s| format!("update_off not written: {}", s.reason)));
    }

    LaunchPlan {
        connector: ConnectorRef {
            id: connector.id.clone(),
            name: connector.name.clone(),
            sha256: connector.sha256.clone(),
            source: connector.source,
        },
        harness_version,
        program,
        argv,
        env: harness_env,
        home: home.to_path_buf(),
        lock,
        before: connector.before.clone(),
        after: connector.after.clone(),
        ca_env: connector.ca_env.clone(),
        hosts,
        native,
        mask_rules: connector.mask.clone(),
        secret_values: Vec::new(),
        notes,
        unread,
        log,
        refusal,
    }
}
