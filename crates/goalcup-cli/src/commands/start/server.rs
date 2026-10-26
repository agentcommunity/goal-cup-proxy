//! A run on the upload server: the prep checks, the participant's approval, and the run opened
//! (or its next segment asked for) on the server.

use super::{CaptureRun, Continuing, StartOptions};
use crate::CliError;
use crate::commands::say;
use crate::connector::HarnessConnector;
use crate::home::RunDir;
use crate::run::reserve::artifact_root;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The upload server this run opens with: `None` with no `goalcup login`. With a server, `goalcup
/// setup` must have run (`prep.json` is there; a `work_root` an older setup wrote in it is
/// ignored), and the project folder, the folder `goalcup start` is run from and the run's
/// workspace, must hold nothing but the participant's starting kit ([`refuse_unready_project`]).
/// Both are checked here, before the participant is asked for anything and before anything is
/// created. A real run's GitHub repository is asked for later,
/// once every other check has passed and before the approval ([`Serving::approved`]). A `--dev`
/// run is uploaded like any other, and the server checks its connector against no list. Since D70
/// the server refuses a run that sends `dev: true` in the competition or from a non-admin, but this
/// CLI does not send `dev` yet (D70 Task 4).
pub(super) fn uploading(
    cfg: &Path,
    env: &BTreeMap<String, String>,
    project: &Path,
    mode: crate::account::device::RunMode,
) -> Result<Option<Serving>, CliError> {
    use crate::account::device::RunMode;
    // A practice run or an organizer test run is a run on the server: without one there is
    // nothing to practise on, and a test run that never reaches the server is no test.
    let flag = match mode {
        RunMode::Practice => Some("--practice"),
        RunMode::Test => Some("--test"),
        RunMode::Competition => None,
    };
    let Some(server) = crate::server::read_server_config(cfg)? else {
        return match flag {
            Some(flag) => Err(CliError::Refused(format!(
                "`{flag}` needs `goalcup login` and `goalcup setup` first. Nothing was created."
            ))),
            None => Ok(None),
        };
    };
    // The one sentence `run goalcup setup first`.
    crate::prep_file::read_prep(cfg)?;
    refuse_unready_project(project)?;
    Ok(Some(Serving {
        url: server.url,
        cfg: std::fs::canonicalize(cfg).unwrap_or_else(|_| cfg.to_path_buf()),
        env: env.clone(),
        project: project.to_path_buf(),
    }))
}

/// Whether `name` is part of the participant's starting kit, which a new run's project folder may
/// hold: their `.gitignore` and their keys (`.env`, `.env.local`, ...), as files.
fn is_starting_kit(name: &str, entry: &std::fs::DirEntry) -> bool {
    (name == ".gitignore" || name == ".env" || name.starts_with(".env."))
        && entry.file_type().is_ok_and(|t| t.is_file())
}

/// A new run on a server works in its project folder, which must start empty but for the
/// starting kit ([`is_starting_kit`]): what the agent finds there is then its own work. Refused
/// before the approval, so a folder that is not ready costs no approval.
fn refuse_unready_project(project: &Path) -> Result<(), CliError> {
    let entries = std::fs::read_dir(project).map_err(|source| CliError::Io {
        path: project.display().to_string(),
        source,
    })?;
    // An entry that cannot be read, or whose type cannot be, is not known to be the kit: refused
    // like anything else in the folder.
    let mut extra: Vec<String> = entries
        .filter_map(|entry| match entry {
            Ok(entry) => {
                let name = entry.file_name().to_string_lossy().into_owned();
                (!is_starting_kit(&name, &entry)).then_some(name)
            }
            Err(_) => Some("an entry that could not be read".to_string()),
        })
        .collect();
    if extra.is_empty() {
        return Ok(());
    }
    extra.sort();
    Err(CliError::Refused(crate::participant::project_not_empty(
        project, &extra,
    )))
}

/// The `repo_url` a practice run is opened with. The server asks every run for a public GitHub
/// repository; a practice run has none (D69), so it names the organisers' public, empty practice
/// repository. Nothing is cloned from it or pushed to it.
pub const PRACTICE_REPO_URL: &str = "https://github.com/agentcommunity/goalcup-matrix-empty";

/// What `goalcup start` asks a real run's repository with ([`crate::commands::run_repo`]).
struct StartDoors<'a> {
    env: &'a BTreeMap<String, String>,
    open: super::OpenUrlFn,
}

impl crate::commands::run_repo::Doors for StartDoors<'_> {
    fn api(&self) -> Option<String> {
        #[cfg(feature = "test-support")]
        let api = self.env.get(super::TEST_GITHUB_API).cloned();
        #[cfg(not(feature = "test-support"))]
        let api = {
            let _ = self.env;
            Some(crate::account::github::GITHUB_API.to_string())
        };
        api
    }

    fn read_line(&self) -> Option<String> {
        use goalcup_platform::terminal::{is_foreground, is_terminal};
        if !(is_foreground(libc::STDIN_FILENO) && is_terminal(libc::STDIN_FILENO)) {
            return None;
        }
        crate::arming::read_stdin_line()
    }

    fn interrupted(&self) -> bool {
        crate::arming::interrupted()
    }

    fn open(&self, url: &str) -> bool {
        (self.open)(url)
    }
}

/// A run on a server that has passed every check that needs no approval.
pub(super) struct Serving {
    url: String,
    /// The configuration directory, absolute.
    cfg: PathBuf,
    /// The participant's environment (a test build's fake GitHub API is named there).
    env: BTreeMap<String, String>,
    /// The folder `goalcup start` was run from: the run's workspace.
    project: PathBuf,
}

impl Serving {
    /// For a real run, ask for its GitHub repository and check it ([`crate::commands::run_repo`]);
    /// then ask the participant to approve this run in the browser, and wait for them. Nothing has
    /// been created before this, and a refused repository, a denial, an expired code, a Ctrl-C or
    /// a server that cannot be reached leaves it so (the refusal says so).
    pub(super) fn approved(
        self,
        opts: &StartOptions,
        connector: &HarnessConnector,
        out: &mut dyn std::io::Write,
    ) -> Result<Uploading, CliError> {
        let repo = if opts.mode.is_practice() {
            None
        } else {
            let doors = StartDoors {
                env: &self.env,
                open: opts.open_url,
            };
            Some(crate::commands::run_repo::choose(
                &doors,
                &self.cfg,
                opts.repo.as_deref(),
                out,
            )?)
        };
        // Approving a competition run replaces the account's open one: said before the browser
        // opens, when this computer has such a run on this server.
        if opts.mode == crate::account::device::RunMode::Competition
            && let Some(started) = open_competition_run(&self.cfg, &self.url)
        {
            say(
                out,
                &crate::participant::replaces_open_run(&crate::participant::local_clock(started)),
            )?;
        }
        let anonymous = crate::upload::http::ServerClient::anonymous(&self.url);
        let approved = crate::account::device::approve(
            &anonymous,
            crate::account::device::Purpose::Run {
                mode: opts.mode,
                harness: connector.display_name(),
            },
            &connector.loaded.manifest.id,
            &opts.open_url,
            out,
        )?;
        let credential = match approved {
            crate::account::device::Approved::Run(credential) => {
                say(out, crate::participant::APPROVED)?;
                credential
            }
            crate::account::device::Approved::Login { .. } => {
                return Err(CliError::Internal(
                    "the server approved a login when a run was asked for. Nothing was \
                     created; run the command again, and report it if it keeps happening."
                        .into(),
                ));
            }
        };
        Ok(Uploading {
            server: crate::upload::http::ServerClient::with_credential(&self.url, &credential),
            how: UploadHow::Fresh(Box::new(NewRun {
                credential,
                repo,
                project: self.project,
            })),
        })
    }
}

/// When the newest competition run this computer has open on `server` started (unix seconds), if
/// it has one: a run opened there, not practice or test, and not over by its state's count.
pub(super) fn open_competition_run(cfg: &Path, server: &str) -> Option<u64> {
    let now = crate::run::now_unix();
    let same = |url: &str| url.trim_end_matches('/') == server.trim_end_matches('/');
    crate::run::all(cfg)
        .into_iter()
        .filter(|run| {
            !run.practice
                && !run.test
                && run.deadline_at.is_some()
                && run.server_url.as_deref().is_some_and(same)
                && !crate::run::is_over(
                    run,
                    crate::run::read_state(&crate::run::run_dir(cfg, &run.run_id)).as_ref(),
                    now,
                )
        })
        .map(|run| run.started_at_unix)
        .max()
}

/// A run's upload server, for `goalcup start` and `goalcup continue`: the client that carries
/// this run's credential, and what is special to the way the run is opened.
pub(super) struct Uploading {
    pub(super) server: crate::upload::http::ServerClient,
    pub(super) how: UploadHow,
}

pub(super) enum UploadHow {
    /// A new run: approved, opened.
    Fresh(Box<NewRun>),
    /// A new segment of a run that is open.
    Continue {
        run_id: String,
        /// The entry the server is told this segment follows.
        previous_entry_id: String,
        /// The entries this machine has: a latest entry of the server's that is none of them is
        /// a segment a `goalcup continue` opened and did not launch.
        known_entries: Vec<String>,
    },
}

/// What a new run is opened with.
pub(super) struct NewRun {
    credential: crate::account::credential::RunCredential,
    /// A real run's repository; `None` for a practice run, which has none.
    repo: Option<crate::commands::run_repo::RunRepo>,
    /// The run's workspace: the participant's project folder, used as it is. Goal Cup makes no
    /// clone and runs no git there; the agent sets git up and pushes.
    pub(super) project: PathBuf,
}

/// The run as the server opened it, what it was asked, and the give-back until the launch.
pub(super) struct OpenedRun {
    pub(super) run: crate::upload::api::OpenRun,
    pub(super) request: crate::upload::api::OpenRunRequest,
    /// When the server answered, on a clock that runs through sleep: the run's own count of
    /// its length starts here.
    pub(super) opened_at: goalcup_protocol::clock::ContinuousOrigin,
    pub(super) give_back: std::cell::RefCell<Option<crate::upload::GiveBack>>,
    /// The segment's number, for a continued run.
    pub(super) segment: Option<u64>,
}

impl Uploading {
    /// The upload side of a continued run: its saved credential, and the segment it follows.
    pub(super) fn continuing(continuing: &Continuing) -> Result<Uploading, CliError> {
        let record = &continuing.record;
        let Some(url) = &record.server_url else {
            return Err(CliError::Internal(
                "a run with no upload server was asked to continue".to_string(),
            ));
        };
        Ok(Uploading {
            server: crate::upload::http::ServerClient::with_credential(url, &continuing.credential),
            how: UploadHow::Continue {
                run_id: record.run_id.clone(),
                previous_entry_id: continuing.previous_entry_id.clone(),
                known_entries: record.entries.iter().map(|e| e.entry_id.clone()).collect(),
            },
        })
    }

    /// Open the run on the server, before the daemon creates anything; for a continued run, ask
    /// the server for its next segment.
    pub(super) fn open(
        &self,
        cx: &CaptureRun<'_>,
        run: &RunDir,
        session_id: &str,
        harness_version: &str,
    ) -> Result<OpenedRun, CliError> {
        let c = cx.connector;
        let mut request = crate::upload::api::OpenRunRequest {
            connector_id: c.loaded.manifest.id.clone(),
            manifest_sha256: c.loaded.profile_sha256(),
            connector_dir_sha256: c.loaded.connector_dir_sha256.clone(),
            workspace: run.workspace.display().to_string(),
            session_id: session_id.to_string(),
            harness_version: harness_version.to_string(),
            proxy_version: crate::upload::api::ProxyVersion::current(),
            repo_url: String::new(),
            machine: None,
        };
        let new = match &self.how {
            UploadHow::Fresh(new) => new,
            UploadHow::Continue {
                run_id,
                previous_entry_id,
                known_entries,
            } => {
                return self.open_segment(
                    cx.cfg,
                    &request,
                    run_id,
                    previous_entry_id,
                    known_entries,
                    cx.over,
                );
            }
        };
        request.repo_url = new
            .repo
            .as_ref()
            .map_or(PRACTICE_REPO_URL, |repo| repo.repo_url.as_str())
            .to_string();
        request.machine = Some(goalcup_platform::machine::facts(&artifact_root(cx.cfg)));
        crate::upload::give_back_pending(self.server.base(), cx.cfg);
        // Taken before the first ask: a repeat after a lost reply answers the first
        // `official_start`, and the run's own clock must never start later than the server's.
        let opened_at = goalcup_protocol::clock::ContinuousOrigin::now();
        let run = crate::upload::open_run(&self.server, &request)?;
        // The run is bound to the credential now, and the credential is kept in the run's own
        // directory, where `goalcup upload` and `continue` find it from the entry. Both the
        // note of the open and the credential are written before anything else can fail.
        let kept = crate::upload::GiveBack::opened(&self.server, cx.cfg, &run).and_then(|give| {
            crate::account::credential::store(
                &crate::run::run_dir(cx.cfg, &run.run_id),
                &new.credential,
            )
            .map(|()| give)
        });
        match kept {
            Ok(give_back) => {
                // Only now is the repository used: the next real run suggests a new name. A hint
                // that cannot be written stops nothing.
                if let Some(repo) = &new.repo
                    && let Err(error) =
                        crate::commands::run_repo::keep_for_next_run(cx.cfg, &repo.repo_url)
                {
                    crate::host_log::say(&format!(
                        "the repository of run {} was not kept for the next run's suggestion: \
                         {error}",
                        run.run_id
                    ));
                }
                Ok(OpenedRun {
                    run,
                    request,
                    opened_at,
                    give_back: std::cell::RefCell::new(Some(give_back)),
                    segment: None,
                })
            }
            Err(error) => {
                // The note of the open and the credential are deleted only once the server has
                // taken the run back; while it is away they are the only way to give it back.
                if !crate::upload::undo_open(&self.server, cx.cfg, &run.run_id) {
                    remove_run_keeping_pending_credential(cx.cfg, &run.run_id);
                    let kept = crate::upload::pending_path(cx.cfg).exists()
                        && crate::account::credential::credential_path(&crate::run::run_dir(
                            cx.cfg,
                            &run.run_id,
                        ))
                        .exists();
                    crate::host_log::say(&if kept {
                        format!(
                            "run {} could not be given back to the server just now; the next \
                             goalcup start gives it back",
                            run.run_id
                        )
                    } else {
                        format!(
                            "run {} could not be given back to the server, and this machine \
                             cannot ask again; the server ends it at its deadline or when your \
                             account starts another",
                            run.run_id
                        )
                    });
                }
                Err(error)
            }
        }
    }

    /// `POST /v1/runs/{id}/continue`, as the answer of an open. A run's latest entry on the
    /// server that this machine does not know is a segment an earlier `goalcup continue` opened
    /// and did not launch (the daemon or the terminal failed after the server had answered):
    /// this segment follows that one, which stays on the server with no chain, once.
    fn open_segment(
        &self,
        cfg: &Path,
        carried: &crate::upload::api::OpenRunRequest,
        run_id: &str,
        previous_entry_id: &str,
        known_entries: &[String],
        run_over: &std::cell::Cell<bool>,
    ) -> Result<OpenedRun, CliError> {
        use crate::upload::Continued;
        let over = || {
            run_over.set(true);
            // The server's word, kept: `goalcup status` and the reserve's release follow it now.
            crate::run::note_over(cfg, run_id);
            CliError::Refused(crate::participant::run_over(run_id))
        };
        let opened_at = goalcup_protocol::clock::ContinuousOrigin::now();
        let mut previous = previous_entry_id.to_string();
        let mut followed = false;
        let segment = loop {
            let request = crate::upload::api::ContinueRequest {
                previous_entry_id: previous.clone(),
                connector_id: carried.connector_id.clone(),
                manifest_sha256: carried.manifest_sha256.clone(),
                connector_dir_sha256: carried.connector_dir_sha256.clone(),
                workspace: carried.workspace.clone(),
                session_id: carried.session_id.clone(),
                harness_version: carried.harness_version.clone(),
                proxy_version: carried.proxy_version.clone(),
            };
            match crate::upload::continue_run(&self.server, run_id, &request)? {
                Continued::Segment(segment) => break segment,
                Continued::Over => return Err(over()),
                Continued::Stale => {
                    let latest: crate::upload::api::RunPosition = self
                        .server
                        .json("GET", &format!("/v1/runs/{run_id}"), None::<&()>)
                        .map_err(|error| {
                            CliError::Refused(format!(
                                "the server did not say where run {run_id} is: {error}. \
                                 Nothing was started."
                            ))
                        })?;
                    if latest.stop {
                        return Err(over());
                    }
                    if followed || known_entries.contains(&latest.entry_id) {
                        return Err(CliError::Refused(crate::participant::continued_elsewhere(
                            run_id,
                            &latest.entry_id,
                            &previous,
                        )));
                    }
                    followed = true;
                    previous = latest.entry_id;
                }
            }
        };
        Ok(OpenedRun {
            run: crate::upload::api::OpenRun {
                run_id: run_id.to_string(),
                entry_id: segment.entry_id,
                official_start: segment.official_start,
                deadline_at: segment.deadline_at,
                seconds_left: segment.seconds_left,
            },
            request: carried.clone(),
            opened_at,
            // A segment cannot be given back: the run is the participant's, and stays open.
            give_back: std::cell::RefCell::new(None),
            segment: Some(segment.segment),
        })
    }

    /// The daemon made the entry under the server's id: fetch the start receipt over its
    /// baseline and keep it. The line that says what the server stamped is returned, for the
    /// caller to place: in `host.log`.
    pub(super) fn bind(
        &self,
        cfg: &Path,
        opened: &OpenedRun,
        info: &goalcup_protocol::control::CaptureWorkspaceInfo,
    ) -> Result<((crate::upload::UploadRecord, String), String), CliError> {
        if info.entry_id != opened.run.entry_id {
            return Err(CliError::Internal(format!(
                "the daemon created entry {} for the server's entry {}",
                info.entry_id, opened.run.entry_id
            )));
        }
        let receipt = crate::upload::fetch_receipt(
            &self.server,
            &opened.run,
            &opened.request,
            &info.chain_head,
        )?;
        let record = crate::upload::UploadRecord {
            run_id: opened.run.run_id.clone(),
            entry_id: opened.run.entry_id.clone(),
            server_url: self.server.base().to_string(),
            trace_dir: info.trace_dir.clone(),
            official_start: opened.run.official_start.clone(),
            deadline_at: opened.run.deadline_at.clone(),
            workspace: opened.request.workspace.clone(),
        };
        crate::upload::keep_run(cfg, &record, &receipt)?;
        let line = match opened.segment {
            Some(segment) => format!(
                "run {} continued on {}: part {segment}, deadline {}",
                record.run_id, record.server_url, record.deadline_at
            ),
            None => format!(
                "run {} opened on {}: official start {}, deadline {}",
                record.run_id, record.server_url, record.official_start, record.deadline_at
            ),
        };
        Ok(((record, receipt.public_key), line))
    }
}

/// Remove this machine's directory for a run that never launched. When the note of the open
/// still names the run (the server was away when it was given back), the credential stays: it is
/// the only thing that can give that run back later.
pub(super) fn remove_run_keeping_pending_credential(cfg: &Path, run_id: &str) {
    let pending = std::fs::read(crate::upload::pending_path(cfg))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<crate::upload::PendingOpen>(&bytes).ok())
        .is_some_and(|note| note.run_id == run_id);
    if !pending {
        crate::run::remove(cfg, run_id);
        return;
    }
    let dir = crate::run::run_dir(cfg, run_id);
    let keep = crate::account::credential::CREDENTIAL_FILE_NAME;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.file_name() != keep {
                let path = entry.path();
                let _ = if path.is_dir() {
                    std::fs::remove_dir_all(path)
                } else {
                    std::fs::remove_file(path)
                };
            }
        }
    }
}
