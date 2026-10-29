//! The phases of a capture run, in the order they run: [`prepare`], [`claim_shared_home`],
//! [`create_and_gate`] (the abandon window), [`register_and_release`] and [`run_harness`]. The
//! fifth, `finish_run`, is in [`super::finish`].

use super::finish::{describe_exit, finish};
use super::forked::{
    AfterExit, ChatTerminal, Exited, Forked, Gate, Released, UploadStart, release_and_say,
};
use super::proxy::{Proxied, admitted, entry_is_sealed, raise_open_file_limit};
use super::refusals::{
    default_host_warning, holds_the_terminal, interrupted_before_launch, interrupted_refusal,
    offline_upstreams_for_test, reachable,
};
use super::server::{UploadHow, open_competition_run, remove_run_keeping_pending_credential};
use super::{CaptureRun, panic_while_quiet_for_test, pause_at};
use crate::CliError;
use crate::commands::say;
use crate::entries::{EntryRecord, remove_entry, write_entry};
use crate::home::{
    RunDir, SharedCaptureHome, WorkspaceAt, ensure_shared_home, lock_shared_home, note_home_holder,
    private_dir, shared_leaf,
};
use crate::launch::{gated_handle, spawn_gated};
use crate::preflight::Level;
use crate::proxy::connect::ProxyCredential;
use crate::proxy::preflight::Unreachable;
use crate::proxy::tls::UpstreamRoots;
use crate::retry::{retry_busy, retry_busy_entry, retry_busy_process};
use crate::run::reserve::{RunReserve, artifact_root, reserve_path};
use crate::run::{EntryRef, RunRecord};
use crate::upload::policy::{WrapUp, WrapUpSpec};
use goalcup_protocol::control::{
    CaptureFinishReport, CaptureGoalRequest, CaptureLaunchRequest, CaptureWorkspaceRequest,
    DaemonClient,
};
use goalcup_protocol::harness::ConfigHome;
use goalcup_protocol::process::{ProcessRegistration, termination_bound};
use goalcup_protocol::run_secret::RunSecret;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// What the gate is given for its two steps: the parked child's pid to arrive and, once it is
/// released, its acknowledgement that it is running. Separate from [`CAPTURE_STARTUP`] because it
/// bounds a different thing, even though both are five seconds.
const GATE_READY_WITHIN: Duration = Duration::from_secs(5);

/// Everything a capture run decides, and every refusal it makes, before anything is created.
pub(super) struct CapturePrep {
    /// The harness file this run resolved and hashed; the release refuses it if it changed.
    pub(super) executable: crate::launch::ResolvedExecutable,
    /// The harness home every run of this connector shares (D39); `None` when each run makes its
    /// own inside its run directory.
    pub(super) shared_home: Option<SharedCaptureHome>,
    /// The run `HOME` every run of this connector shares (D59, `run_home = "shared"`): the
    /// folder `<cfg>/capture-homes/<id>.run-home`, under the same lock. `None` when each run
    /// gets a `HOME` of its own.
    pub(super) shared_run_home: Option<SharedCaptureHome>,
    /// The lock that keeps a second run off that shared home: held from the claim, moved into
    /// the [`Forked`] run at the fork and let go at its seal (the upload that follows needs no
    /// home), and released by the OS if this process dies. A `goalcup continue` takes it before
    /// it closes anything and hands it over.
    pub(super) shared_home_lock: Option<std::fs::File>,
}

/// What a chat-mode run records as its goal. The v5 `Launch` record names the goal stream it
/// was launched with and the daemon records a launch only after a goal, so a run whose harness
/// was handed none records this fixed sentence instead: never sent to the harness, and never
/// anything the participant typed (that is in the harness's session files).
pub const CHAT_MODE_GOAL: &str = "chat mode (D57): the harness was given no goal; the \
     participant typed the prompt in the harness's own terminal, and it is recorded in the \
     harness's session files.";

/// Phase one: every refusal that comes before anything is created.
pub(super) fn prepare(
    cx: &CaptureRun<'_>,
    out: &mut dyn std::io::Write,
) -> Result<CapturePrep, CliError> {
    // The disk is judged before the server is asked for a run or anything is created. A
    // continued run keeps its reserve file, and `RunReserve::take` judges what is missing of it.
    if cx.continuing.is_none() {
        crate::run::reserve::check_room(&artifact_root(cx.cfg).join("reserve"), &cx.opts.disk)?;
    }
    let CaptureRun {
        env,
        opts,
        beside,
        connector: c,
        profile,
        model_host,
        ..
    } = *cx;
    // The rulebook the recording proxy's host rules and the launch come from, through the one door.
    let pin = crate::preflight::harness_pin_gate(c, opts.dev);
    match pin.level {
        Level::Fail => {
            return Err(CliError::Refused(format!(
                "`goalcup start` refuses this run:\n{}",
                pin.render().trim_end()
            )));
        }
        Level::Warn => say(out, pin.render().trim_end())?,
        _ => {}
    }
    holds_the_terminal(c.display_name(), "Nothing was created.")?;
    let executable = crate::launch::capture_executable(c, profile, env)?;
    // The provider hosts, and the model service's own port, answer a TCP connect before
    // anything is created (the recording proxy module's preflight; it names a firewall holding this
    // binary when they do not).
    if !offline_upstreams_for_test(env) {
        let refusal = |unreachable: Unreachable| {
            CliError::Refused(unreachable.refusal(cx.continuing.map(|c| c.record.run_id.as_str())))
        };
        // Which hosts are hard and which only warn is decided by `provider_hosts`: the model
        // service the settings name is the one host a run cannot go without, and the harness's
        // own hosts are one warning line, not a refusal, when a service is named.
        let split = crate::model_host::provider_hosts(
            model_host,
            profile.hosts.iter().map(|h| h.host.as_str()),
            profile.auth_hosts.iter().map(String::as_str),
            crate::model_host::Caller::Start,
        );
        match model_host {
            Some(model) => {
                // A model service whose name resolves only to this computer is refused before the
                // probe. This whole block is behind the test-only offline seam, so no integration
                // test reaches the check; its unit test is
                // `refuse_if_on_this_computer_uses_the_name_and_reason`.
                crate::model_host::refuse_if_on_this_computer(model).map_err(|refused| {
                    CliError::Refused(refused.refusal(c.display_name(), &c.loaded.manifest.id))
                })?;
                reachable(env, &endpoint_refs(&split.hard)).map_err(refusal)?;
                if let Err(missing) = reachable(env, &endpoint_refs(&split.warn)) {
                    say(out, &default_host_warning(&missing, model))?;
                }
            }
            None => reachable(env, &endpoint_refs(&split.hard)).map_err(refusal)?,
        }
    }
    // A per-run home is made inside the run directory below. A run directory inside a Git work
    // tree is refused before anything is created: it would hold what the
    // harness wrote, its own login included.
    let per_run = profile.home.config_home == ConfigHome::PerRun;
    if per_run {
        // A run on a server makes `<config>-runs` only once approved: until then it is judged by
        // the folder it will be made in.
        let whose = if beside == crate::home::run_folders(cx.cfg) {
            crate::per_run_home::RunFolder::ConfigRuns
        } else {
            crate::per_run_home::RunFolder::StartedFrom
        };
        let beside = beside
            .ancestors()
            .find(|folder| folder.exists())
            .unwrap_or(beside);
        crate::per_run_home::refuse_inside_git_work_tree(beside, whose)?;
        crate::per_run_home::refuse_synced_folder_under(beside, env.get("HOME").map(Path::new))?;
    }
    Ok(CapturePrep {
        executable,
        shared_home: None,
        shared_run_home: None,
        shared_home_lock: None,
    })
}

/// The harness home every run of a connector shares (D39), or its shared run `HOME` (D59), taken
/// for this run: its folder made (0700) if there is none, and the lock that keeps a second run
/// off it. A connector whose homes are made per run has none to take. This creates things on
/// disk, so a run on a server takes it only after the participant has approved the run.
pub(super) fn claim_shared_home(
    cx: &CaptureRun<'_>,
    prep: &mut CapturePrep,
) -> Result<(), CliError> {
    let CaptureRun {
        cfg,
        connector: c,
        profile,
        ..
    } = *cx;
    let Some((config_home, is_run_home)) = shared_leaf(cfg, profile, &c.loaded.manifest.id) else {
        return Ok(());
    };
    private_dir(config_home.parent().expect("a capture-home parent"), true)?;
    // `goalcup continue` took the lock before it closed anything, and hands it over.
    let lock = match prep.shared_home_lock.take() {
        Some(held) => held,
        None => {
            // A competition start replaces an open competition run of this account: the refusal
            // says so only then (a practice or test start never replaces one).
            let replaces = cx.opts.mode == crate::account::device::RunMode::Competition
                && cx
                    .upload
                    .is_some_and(|up| open_competition_run(cfg, up.server.base()).is_some());
            lock_shared_home(
                &config_home,
                &c.loaded.manifest.id,
                c.display_name(),
                replaces,
            )?
        }
    };
    let home = ensure_shared_home(&config_home)?;
    if is_run_home {
        prep.shared_run_home = Some(home);
    } else {
        prep.shared_home = Some(home);
    }
    prep.shared_home_lock = Some(lock);
    Ok(())
}

/// Phase two, the abandon window: the run directory, the entry, the recording proxy, the recorded
/// goal and the parked child. Up to the fork a failure unmakes everything made here; the fork ends
/// the window.
pub(super) fn create_and_gate<'a>(
    cx: &CaptureRun<'a>,
    roots: UpstreamRoots,
    prep: &CapturePrep,
    out: &mut dyn std::io::Write,
) -> Result<(Forked<'a>, Gate), CliError> {
    let CaptureRun {
        client,
        upload,
        cfg,
        env,
        opts,
        beside,
        connector: c,
        profile,
        ..
    } = *cx;
    // Catch a replacement since prepare before writing even the temporary run directory.
    let shared_home = prep
        .shared_home
        .as_ref()
        .map(SharedCaptureHome::recheck)
        .transpose()?;
    let shared_run_home = prep
        .shared_run_home
        .as_ref()
        .map(SharedCaptureHome::recheck)
        .transpose()?;
    let run = RunDir::create(
        beside,
        profile.home.run_home,
        shared_run_home.as_deref(),
        env,
        match (cx.continuing, upload.map(|u| &u.how)) {
            // A continued run goes on in the folder it started in.
            (Some(continuing), _) => WorkspaceAt::Existing(&continuing.record.workspace),
            // A new run on a server works in the participant's project folder, as it is.
            (None, Some(UploadHow::Fresh(new))) => WorkspaceAt::Existing(&new.project),
            _ => WorkspaceAt::InRunDir,
        },
    )?;
    let config_home = match shared_home {
        Some(home) => home,
        // Seeded from the connector's own files, which the manifest gate has pinned.
        None => {
            match crate::per_run_home::create_per_run_home(&run.dir, &profile.home, &c.loaded.seeds)
            {
                Ok(home) => home,
                Err(e) => {
                    let _ = run.remove();
                    return Err(e);
                }
            }
        }
    };

    // The harness's own account of its version, asked once with nothing but `PATH` and a
    // throwaway `HOME` beside the run's own, removed afterwards: a probe may write state (Codex
    // creates `~/.codex/tmp/arg0`), and the run's `HOME` stays as the profile made it. Reported,
    // not proven, and `unreported` when it does not say (D53).
    let harness_version = crate::harness_version::reported_version_in_scratch_home(
        &prep.executable,
        &profile.executable.version_args,
        &run.dir.join("version-probe-home"),
    );

    // The last alias/target identity check is directly before the create request. Preserve the
    // exact D39 leaf spelling in the request, local entry and child environment.
    if let Some(home) = prep.shared_home.as_ref().or(prep.shared_run_home.as_ref()) {
        pause_at("capture-shared-home-pre-create");
        if let Err(error) = home.recheck() {
            let _ = run.remove();
            return Err(error);
        }
    }

    let session_id = fresh_uuid();
    // The run's start: what its record keeps, and what a native source that reads from the run's
    // start (D57) is read from. Sent for every entry, the same number for each.
    let started_at_unix = match cx.continuing {
        Some(continuing) => continuing.record.started_at_unix,
        None => crate::run::now_unix(),
    };
    // The model calls the run's earlier parts recorded, by the last part's `state.json`; none
    // for a run started by a build that did not count them (D73).
    let previous = cx.continuing.and_then(|continuing| {
        crate::run::read_state(&crate::run::run_dir(cfg, &continuing.record.run_id))
    });
    let model_calls_before = previous.as_ref().and_then(|state| state.model_calls);
    // The last part resumed and its harness failed at once (D73): this one opens a new
    // conversation rather than fail the same way again.
    let resume_failed = previous.as_ref().is_some_and(|state| state.resume_failed);
    // A continued run picks up the harness's last session only when the run has one; one whose
    // harness closed before the first message opens the normal way, and the launch block says so.
    // A store that cannot be read cannot tell: the harness is resumed, and the run's log says why.
    // A harness that may write no session file (a native row with `warn_missing = false`, D72)
    // is resumed when the run's earlier parts recorded model calls: there is a conversation to
    // pick up, which its own store keeps (D73).
    let mut session_note = None;
    let opens = match cx.continuing {
        None => crate::participant::Opens::First,
        Some(_) if resume_failed => {
            session_note = Some(
                "the last part asked the harness to pick up its last session and it exited at \
                 once, so this part opens a new one"
                    .to_string(),
            );
            crate::participant::Opens::NewSession
        }
        Some(_) => {
            match crate::launch::has_native_session(
                profile,
                &run.home,
                &config_home,
                started_at_unix,
            ) {
                Ok(true) => crate::participant::Opens::LastSession,
                Ok(false)
                    if resumes_without_a_file(
                        profile,
                        model_calls_before.unwrap_or(0),
                        &config_home,
                        started_at_unix,
                    ) =>
                {
                    session_note = Some(format!(
                        "no session file was found, and the run's earlier parts recorded \
                         {} model calls, so the harness is asked to pick up its last session",
                        model_calls_before.unwrap_or(0)
                    ));
                    crate::participant::Opens::LastSession
                }
                Ok(false) => crate::participant::Opens::NewSession,
                Err(why) => {
                    session_note = Some(format!(
                        "the harness's session store could not be read ({why}), so it is \
                         asked to pick up its last session"
                    ));
                    crate::participant::Opens::LastSession
                }
            }
        }
    };
    // With a server, the run is opened there before the daemon creates anything: the
    // server issues the entry id every record carries.
    let opened = match upload {
        Some(upload) => match upload.open(cx, &run, &session_id, &harness_version) {
            Ok(opened) => Some(opened),
            Err(error) => {
                let _ = run.remove();
                return Err(error);
            }
        },
        None => None,
    };
    // Give the run back to the server: this start failed before its launch.
    let give_back = |_cause: &CliError| {
        if let Some(give_back) = opened
            .as_ref()
            .and_then(|o| o.give_back.borrow_mut().take())
        {
            give_back.give();
        }
    };
    // The recording proxy's credential is made now, before the entry, so that the daemon is told
    // it: the harness holds it in `HTTPS_PROXY`, and the daemon redacts it by exact value from
    // everything it records (D60). It goes to the daemon in memory only.
    let credential = ProxyCredential::generate();
    let run_secrets = match RunSecret::new(credential.expose_secret().as_bytes()) {
        Some(secret) => with_project_token(secret, upload.is_some().then_some(&run.workspace)),
        None => {
            let error =
                CliError::Internal("the proxy credential is not one the daemon takes".to_string());
            let _ = run.remove();
            give_back(&error);
            return Err(error);
        }
    };
    let request = CaptureWorkspaceRequest {
        workspace: run.workspace.display().to_string(),
        profile_id: c.loaded.manifest.id.clone(),
        connector_dir_sha256: c.loaded.connector_dir_sha256.clone(),
        reserve_bytes: opts.disk.chain_quota,
        run_home: run.home.display().to_string(),
        config_home: config_home.display().to_string(),
        entry_id: opened.as_ref().map(|o| o.run.entry_id.clone()),
        native_since: Some(started_at_unix),
        run_secrets,
        // A continued run's clone must still be there when the daemon makes the workspace:
        // `goalcup continue` looked before it closed anything, and the daemon looks again.
        existing_clone: cx.continuing.is_some(),
    };
    let info = match retry_busy(|| client.create_capture_workspace(&request)) {
        Ok(info) => info,
        Err(e) => {
            let _ = run.remove();
            let error = e.into();
            give_back(&error);
            return Err(error);
        }
    };
    let template = EntryRecord {
        entry_id: info.entry_id.clone(),
        process_generation: info.process_generation,
        connector_id: c.loaded.manifest.id.clone(),
        run_public_key: info.run_public_key.clone(),
        workspace: run.workspace.display().to_string(),
        // A local capture has no artifact root or status page this machine is told about.
        artifact_root: String::new(),
        harness_home: config_home.display().to_string(),
        session_id: session_id.clone(),
        last_pid: None,
        // No daemon stamp comes back for a capture entry, so this machine's clock orders it.
        created_at: crate::launch::launched_at_now(),
        dev: opts.dev,
    };

    // This machine's record of the run: the server's run id for a run opened there, the entry's
    // own id for a local one. A continued run's is its record so far and the new segment.
    let new_entry = EntryRef {
        entry_id: info.entry_id.clone(),
        trace_dir: PathBuf::from(&info.trace_dir),
    };
    let record = match cx.continuing {
        Some(continuing) => {
            let mut record = continuing.record.clone();
            record.entries.push(new_entry);
            record
        }
        None => RunRecord {
            run_id: opened
                .as_ref()
                .map_or_else(|| info.entry_id.clone(), |o| o.run.run_id.clone()),
            server_url: upload.map(|u| u.server.base().to_string()),
            practice: opts.mode.is_practice(),
            test: opts.mode == crate::account::device::RunMode::Test,
            connector_id: c.loaded.manifest.id.clone(),
            workspace: run.workspace.clone(),
            started_at_unix,
            deadline_at: opened.as_ref().map(|o| o.run.deadline_at.clone()),
            entries: vec![new_entry],
        },
    };
    // What the participant puts at the start of their prompt, for a run with a repository: made
    // at the first launch, and read back from the run's file at a continue (D73).
    let prompt = match (cx.continuing, upload.map(|u| &u.how)) {
        (Some(continuing), _) => {
            crate::push_lines::read(&crate::push_lines::path(cfg, &continuing.record.run_id))
                .unwrap_or_default()
        }
        (None, Some(UploadHow::Fresh(new))) => prompt_for(new),
        _ => Vec::new(),
    };
    let mut prompt_file = (cx.continuing.is_some() && !prompt.is_empty())
        .then(|| crate::push_lines::path(cfg, &record.run_id));
    // The run's one reservation, taken here and released when the launch fails or the run ends.
    let mut reserve = None::<RunReserve>;
    // Whether the push block went to the clipboard (D73), copied just before the spawn.
    let mut on_clipboard = false;
    // The run's ids and folders, kept for its `host.log` and `goalcup status`, so the launch
    // block holds only what to do.
    let mut details: Vec<String> = session_note.into_iter().collect();
    // Up to the spawn, a failure unmakes everything above.
    let launched = write_entry(cfg, &template).and_then(|()| {
        // The record first: a reservation always has a record to say whose it is.
        crate::run::write(cfg, &record)?;
        // The push lines, kept in the run's directory so they are never lost (D73). A write
        // that fails costs only the path on the screen.
        if cx.continuing.is_none() && !prompt.is_empty() {
            let file = crate::push_lines::path(cfg, &record.run_id);
            match crate::push_lines::write(&file, &prompt) {
                Ok(()) => prompt_file = Some(file),
                Err(e) => details.push(format!("the push lines could not be kept: {e}")),
            }
        }
        #[cfg(unix)]
        if let Some(lock) = &prep.shared_home_lock {
            note_home_holder(lock, &record.run_id);
        }
        reserve = Some(RunReserve::take(
            &reserve_path(&artifact_root(cfg), &record.run_id),
            &opts.disk,
        )?);
        let bound = match (upload, opened.as_ref()) {
            (Some(upload), Some(opened)) => {
                let (bound, line) = upload.bind(cfg, opened, &info)?;
                details.push(line);
                Some(bound)
            }
            _ => None,
        };
        details.push(format!("entry {}", info.entry_id));
        match (cx.model_host, &profile.model_host_from) {
            (Some(model_host), _) => details.push(model_host.detail()),
            // `absent = "defaults"` and no setting: the connector's own hosts are recorded.
            (None, Some(from)) => details.push(crate::model_host::defaults_detail(from)),
            (None, None) => {}
        }
        details.push(format!("run directory: {}", run.dir.display()));
        raise_open_file_limit(out)?;
        let tunnels_file = crate::run::tunnels_path(cfg, &record.run_id, &info.entry_id);
        let proxied = Proxied::start(
            &info,
            profile,
            cx.model_host,
            &run,
            roots,
            credential,
            tunnels_file,
        )?;
        // A shared CA variable (`SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`) gets the system's roots
        // plus the recording proxy's CA, so tool children keep their real trust; a harness's own
        // variable gets the recording proxy's CA alone (the launch picks per name).
        let ca_bundle =
            crate::ca_bundle::bundle_for(profile, &run.ca, proxied.proxy.ca_certificate_der())?;
        admitted(client, &info)?;
        // The goal-cup skill, in the harness's own skills folder (D57), replacing only its own.
        if let Some(skills_dir) = &profile.home.skills_dir {
            crate::skill::install(&config_home, skills_dir)?;
        }
        // The settings the capture needs, put into the harness home again before each launch
        // (D61): the participant's own setup may have turned one off. Their output goes to the
        // run's `host.log`, never to the terminal or the trace. A run that fails here is
        // unmade, its log with it, so the refusal points at prep, which logs to its own file.
        // Then the connector's launch prep (D72), which logs a failed command and goes on.
        let launch_prep = c.loaded.manifest.launch_prep.is_some();
        if !profile.configure.is_empty() || launch_prep {
            let run_dir = crate::run::run_dir(cfg, &record.run_id);
            let (log, _) =
                crate::host_log::open_for_child(&run_dir).map_err(|source| CliError::Io {
                    path: run_dir
                        .join(crate::host_log::HOST_LOG_NAME)
                        .display()
                        .to_string(),
                    source,
                })?;
            let home_launch = crate::launch::PrepLaunch {
                executable: &prep.executable,
                run_home: &run.home,
                config_home: &config_home,
                parent_env: env,
            };
            crate::launch::configure_harness(
                c,
                &home_launch,
                &run.workspace,
                &log,
                &format!(
                    "Nothing was launched. Run `goalcup setup --harness {}` again; it runs the same \
                     command and keeps what it prints in {}.",
                    c.loaded.manifest.id,
                    cfg.join(crate::commands::prep::PREP_LOG_NAME).display()
                ),
            )?;
            if launch_prep {
                crate::launch::launch_prep(
                    c,
                    &home_launch,
                    &run.workspace,
                    &crate::launch::LaunchPrepValues {
                        ca_certificate_der: proxied.proxy.ca_certificate_der(),
                        model_host: cx.model_host,
                    },
                    &log,
                )?;
            }
        }
        let mut vars = goalcup_protocol::template::TemplateVars::new();
        vars.set("workspace", run.workspace.display().to_string());
        let build_launch = if opens == crate::participant::Opens::LastSession {
            crate::launch::build_resumed_launch
        } else {
            crate::launch::build_capture_launch
        };
        let plan = build_launch(
            c,
            &vars,
            &run.workspace,
            &crate::launch::CaptureLaunch {
                executable: &prep.executable,
                run_home: &run.home,
                config_home: &config_home,
                proxy_url: &proxied.proxy_url,
                ca_certificate: proxied.proxy.ca_certificate(),
                ca_bundle: ca_bundle.as_deref(),
                parent_env: env,
                time_left_file: Some(&crate::run::time_left::file_path(cfg, &record.run_id)),
            },
        )?;
        // Offered once. `busy` is the daemon's refusal before it reads a request and is
        // asked again; any other failure aborts the launch, because an answer that did not
        // arrive may have followed an accepted goal and a second would be a second
        // instruction.
        // The harness is given no goal, and the run records the fixed sentence that says so.
        let goal_request = CaptureGoalRequest {
            bytes: CHAT_MODE_GOAL.as_bytes().to_vec(),
        };
        let goal_stream = retry_busy(|| client.record_capture_goal(&info.entry_id, &goal_request))?;
        if crate::arming::interrupted() {
            return Err(interrupted_refusal());
        }
        // The only way a capture child is spawned (D40): forked and parked before `exec`,
        // and released only once its registration is durable.
        // On the clipboard too, when the block is shown and a clipboard tool is there (D73).
        // Before the spawn: on Linux `xclip` and `wl-copy` leave a selection server running,
        // and one started before the harness is never swept as the harness's (`subreaper.rs`).
        on_clipboard = crate::participant::shows_push_block(opens, model_calls_before)
            && !prompt.is_empty()
            && !crate::chat_without_terminal()
            && crate::push_lines::copy_to_clipboard(&crate::push_lines::text(&prompt), env);
        let gated = spawn_gated(&plan, GATE_READY_WITHIN)?;
        Ok((proxied, plan, goal_stream, gated, bound))
    });
    let (proxied, plan, goal_stream, gated, bound) = match launched {
        Ok(launched) => launched,
        Err(error) => {
            if let Some(held) = reserve.take() {
                // A continued run's reserve is the run's, and outlives this segment.
                if cx.continuing.is_none() {
                    release_and_say(held);
                }
            }
            let unmade = unmake(client, cfg, &info.entry_id, &run, &error, out);
            if !unmade {
                cx.uncleaned.set(true);
            }
            match cx.continuing {
                // The run stays: this machine's record of it goes back to what it was. The
                // server's new entry (it cannot be given back) is found again by the next
                // `goalcup continue`, which follows it.
                Some(continuing) => {
                    let _ = std::fs::remove_file(crate::upload::record_path(cfg, &info.entry_id));
                    let _ = std::fs::remove_file(crate::upload::receipt_path(cfg, &info.entry_id));
                    // Whether or not the daemon agreed to the abandon: this entry never ran, and
                    // a record that still names it would leave the next `continue` refused for
                    // good ("has no upload record").
                    let _ = crate::run::write(cfg, &continuing.record);
                }
                None => {
                    // The run is given back first, and its credential outlives the give-back
                    // while the server may still hold the run open: the next start's
                    // `give_back_pending` asks with it.
                    give_back(&error);
                    if unmade {
                        remove_run_keeping_pending_credential(cfg, &record.run_id);
                    }
                }
            }
            return Err(error);
        }
    };
    let forked = Forked {
        client,
        cfg,
        info,
        proxied: Some(proxied),
        template,
        upload: bound.map(|(record, key)| UploadStart {
            server: upload.map(|u| u.server.clone()),
            // The server's time left, counted from when it was asked (WP11).
            local_deadline: opened.as_ref().and_then(|o| {
                crate::upload::uploader::LocalDeadline::at_start(
                    &record,
                    o.run.seconds_left,
                    o.opened_at,
                    cx.continuing.is_some(),
                )
            }),
            record,
            key,
            policy: opts.upload_policy,
        }),
        give_back: opened
            .as_ref()
            .and_then(|o| o.give_back.borrow_mut().take()),
        record,
        reserve,
        watching: None,
        chat: None,
        home_lock: None,
        drained: Vec::new(),
        harness_name: c.display_name().to_string(),
        first_screen: c.first_screen().map(str::to_string),
        opens,
        prompt,
        prompt_file,
        on_clipboard,
        model_calls_before,
        details,
        after_exit: c
            .loaded
            .manifest
            .launch_prep
            .as_ref()
            .is_some_and(|p| !p.after_exit.is_empty())
            .then(|| AfterExit {
                executable: prep.executable.clone(),
                run_home: run.home.clone(),
                config_home: config_home.clone(),
                workspace: run.workspace.clone(),
            }),
    };
    let gate = Gate {
        plan,
        harness_version,
        goal_stream,
        gated,
    };
    Ok((forked, gate))
}

/// The `(host, port)` pairs of a host list, borrowed, as `reachable` takes them.
fn endpoint_refs(list: &[(String, u16)]) -> Vec<(&str, u16)> {
    list.iter()
        .map(|(host, port)| (host.as_str(), *port))
        .collect()
}

/// Whether a continue resumes a harness that left no session file (D73): the connector says a
/// missing file is no problem, which holds only when it declares native rows and every one of
/// them has `warn_missing = false` (D72), the run's earlier parts recorded model calls, and the
/// harness home's conversation database was written since the run started. A connector with
/// any file-required row keeps the file rule.
fn resumes_without_a_file(
    profile: &goalcup_protocol::harness::CaptureSpec,
    calls: u64,
    config_home: &Path,
    since_unix: u64,
) -> bool {
    calls > 0
        && !profile.native.is_empty()
        && profile.native.iter().all(|native| !native.warn_missing)
        && session_db_written_since(config_home, since_unix)
}

/// The conversation database a harness with no session file keeps in its home (Hermes 0.21.1
/// and later: `state.db`, with its write-ahead log beside it), D73.
const SESSION_DB_FILES: &[&str] = &["state.db", "state.db-wal"];

/// Whether one of [`SESSION_DB_FILES`] in `config_home` was modified at or after `since_unix`
/// (whole seconds, as D57's native floor), never through a link.
fn session_db_written_since(config_home: &Path, since_unix: u64) -> bool {
    SESSION_DB_FILES.iter().any(|name| {
        std::fs::symlink_metadata(config_home.join(name))
            .ok()
            .filter(std::fs::Metadata::is_file)
            .and_then(|meta| meta.modified().ok())
            .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
            .is_some_and(|at| at.as_secs() >= since_unix)
    })
}

/// How soon after its release a resumed harness that exits non-zero counts as a failed resume
/// (D73): `hermes chat --continue` with no session to pick up exits 1 at once.
const RESUME_FAILS_WITHIN: Duration = Duration::from_secs(10);

/// Whether this part's exit was a failed resume: it was asked to pick up its last session and
/// ended non-zero within [`RESUME_FAILS_WITHIN`].
fn failed_resume(
    opens: crate::participant::Opens,
    status: &Result<std::process::ExitStatus, CliError>,
    ran: Duration,
) -> bool {
    opens == crate::participant::Opens::LastSession
        && status.as_ref().is_ok_and(|status| !status.success())
        && ran < RESUME_FAILS_WITHIN
}

/// The run's secrets for the redactor: the proxy credential, and for a run on a server the
/// GitHub token the participant keeps in the project folder (`GH_TOKEN` in a key file), which the
/// agent reads to push and might print. A token the redactor cannot take by value (too short,
/// or with a space) is left to its patterns.
fn with_project_token(credential: RunSecret, project: Option<&PathBuf>) -> Vec<RunSecret> {
    let mut secrets = vec![credential];
    if let Some((_, Some(token))) = project.and_then(|p| super::project::token(p))
        && secrets.len() < goalcup_protocol::run_secret::MAX_RUN_SECRETS
        && !secrets.contains(&token)
    {
        secrets.push(token);
    }
    secrets
}

/// A real run's prompt lines (E1): its repository, the key file that holds the token (the
/// guide's `.env.local` when none does yet) and the owner's private commit identity.
fn prompt_for(new: &super::server::NewRun) -> Vec<String> {
    let Some(repo) = &new.repo else {
        return Vec::new();
    };
    let file = super::project::token(&new.project)
        .map_or_else(|| super::project::TOKEN_FILE.to_string(), |(file, _)| file);
    crate::participant::prompt_block(
        &repo.repo_url,
        repo.owner(),
        &repo.commit_email(),
        &file,
        &new.project.join(&file),
    )
}

/// Phase three, from the fork: take the parked child's handle, register it durably, and only
/// then release it. Every way out is [`Forked::unlaunched`]: the child never ran, or it is
/// registered and the seal answers for it.
pub(super) fn register_and_release<'a>(
    mut forked: Forked<'a>,
    gate: Gate,
    out: &mut dyn std::io::Write,
) -> Result<Released<'a>, CliError> {
    pause_at("capture-gate-forked");
    let Gate {
        plan,
        harness_version,
        goal_stream,
        gated,
    } = gate;
    let handed = match gated_handle(&plan, &gated, &crate::launch::launched_at_now()) {
        Ok(handed) => handed,
        Err(e) => {
            gated.abort();
            return Err(forked.unlaunched(e, out));
        }
    };
    let registration = ProcessRegistration {
        generation: forked.info.process_generation,
        handle: handed.clone(),
    };
    let (client, entry_id) = (forked.client, &forked.info.entry_id);
    if let Err(error) = retry_busy_process(|| client.register_process(entry_id, &registration)) {
        // Definite or ambiguous, the child is never released: it ends at its gate without
        // having run a byte of the harness, and the seal reads a registration that names a
        // process that never exec'd.
        gated.abort();
        return Err(forked.unlaunched(CliError::Control(error), out));
    }
    pause_at("capture-gate-registered");
    // The last look at the flag. A Ctrl-C during the spawn or the registration above reached
    // only this process and the parked child, which rides it out; released now, the harness
    // would run the whole goal the participant asked to stop.
    if crate::arming::interrupted() {
        gated.abort();
        return Err(forked.unlaunched(interrupted_before_launch(), out));
    }
    // The harness takes the terminal at its release: the launch line is the last thing this
    // command prints there, and from here the terminal's signals are absorbed and every message
    // goes to `host.log` until the harness exits.
    {
        // Moved to the background, or its terminal gone, while the run was approved and opened:
        // the harness is never released onto a terminal it cannot read.
        if let Err(e) = holds_the_terminal(
            &forked.harness_name,
            "Nothing was launched, and what was made is sealed without a launch.",
        ) {
            gated.abort();
            return Err(forked.unlaunched(e, out));
        }
        let run_dir = crate::run::run_dir(forked.cfg, &forked.record.run_id);
        // What the participant does now, and when the run ends: the last thing said here
        // before the harness owns the terminal, ending with the ready mark.
        let ends = forked.record.deadline_at.as_deref().map(|deadline_at| {
            let count = forked.time_left();
            crate::participant::end_words(
                deadline_at,
                crate::participant::seconds_left(
                    count.as_ref(),
                    Some(deadline_at),
                    crate::run::now_unix(),
                ),
            )
        });
        let block = crate::participant::launch_block(&crate::participant::Launch {
            harness: &forked.harness_name,
            mode: forked
                .record
                .server_url
                .is_some()
                .then(|| forked.record.mode()),
            opens: forked.opens,
            ends,
            first_screen: forked.first_screen.as_deref(),
            prompt: &forked.prompt,
            prompt_file: forked.prompt_file.as_deref(),
            on_clipboard: forked.on_clipboard,
            calls_before: forked.model_calls_before,
        });
        let _ = block
            .iter()
            .try_for_each(|line| say(out, line))
            .and_then(|()| {
                out.flush().map_err(|source| CliError::Io {
                    path: "stdout".to_string(),
                    source,
                })
            });
        // A test build with no terminal takes none, and says here what the log would hold.
        if crate::chat_without_terminal() {
            for line in forked.details.drain(..) {
                let _ = say(out, &line);
            }
        } else {
            match ChatTerminal::take(&run_dir) {
                Ok(terminal) => {
                    forked.chat = Some(terminal);
                    // The run's ids and folders, kept out of the launch block, go to its log.
                    for line in forked.details.drain(..) {
                        crate::host_log::say_raw(&line);
                    }
                    panic_while_quiet_for_test();
                }
                Err(e) => {
                    gated.abort();
                    return Err(forked.unlaunched(e, out));
                }
            }
        }
    }
    let spawned = match gated.release() {
        Ok(spawned) => spawned,
        // No harness ran, so no `Launch` is written, and the entry is sealed; the terminal is
        // this command's again, to say so.
        Err(e) => {
            forked.chat = None;
            return Err(forked.unlaunched(e, out));
        }
    };
    // The harness owns every Ctrl-C from here (it shares the terminal), and the launch record
    // and the seal below must not give up on their first `busy` because of one.
    crate::arming::close_window();
    // Observed on this process's monotonic clock, the one the recording proxy's first producer
    // stamps its frames with under its own ID.
    let mut spawned_at = crate::proxy::metadata::capture_time();
    spawned_at.clock_id = forked.info.producer_id.clone();
    let child = spawned.child;
    if let Err(e) = write_entry(forked.cfg, &forked.record(Some(handed.pid))) {
        let _ = say(
            out,
            &format!("  (this machine's own note of the harness's pid could not be updated: {e})"),
        );
    }
    Ok(Released {
        forked,
        handed,
        child,
        spawned_at,
        goal_stream,
        harness_version,
    })
}

/// Phase four: record the launch and wait for the harness. A seal that raced in, or a launch
/// the daemon found sealed, ends the child (D40); any other failure to record leaves the trace
/// incomplete and never stops the run.
pub(super) fn run_harness<'a>(
    cx: &CaptureRun<'_>,
    released: Released<'a>,
    sleep_lock: Result<(), String>,
    out: &mut dyn std::io::Write,
) -> Result<Exited<'a>, CliError> {
    let Released {
        mut forked,
        handed,
        mut child,
        spawned_at,
        goal_stream,
        harness_version,
    } = released;
    // Linux: what the harness leaves behind is this process's own child (a subreaper, `main.rs`),
    // and is swept between the root's exit and the seal. Armed first, while the root is certainly
    // unreaped.
    let sweep = crate::subreaper::Sweep::for_root(handed.pid);
    // How long the harness ran, for a failed resume (D73).
    let released_at = std::time::Instant::now();
    let (client, entry_id) = (forked.client, forked.info.entry_id.clone());
    // A seal that raced in between the registration and the release cannot have ended a
    // harness that was still parked, so one that is sealed now is a harness running on a
    // sealed entry: this command ends the child it just released (D40).
    match entry_is_sealed(client, &forked.info) {
        Ok(true) => return Err(forked.sealed_launch(child, &handed, out)),
        Ok(false) => {}
        Err(e) => {
            let _ = say(
                out,
                &format!(
                    "  (the entry's status could not be read after the harness was released: {e})"
                ),
            );
        }
    }
    pause_at("capture-gate-released");
    // Chat mode names no session (identity `none`): the native store is read without a scope.
    let launch = CaptureLaunchRequest {
        goal_stream,
        spawned_at,
        harness_version,
        continuation: cx.profile.continuation.clone(),
    };
    // A launch the daemon did not record leaves the trace incomplete; it never stops the run,
    // unless the entry is sealed: then the harness has no entry to run on (D40). A `Conflict`
    // on an open entry is a capture-side fault, and it never ends the child.
    if let Err(e) = retry_busy(|| client.record_capture_launch(&entry_id, &launch)) {
        if matches!(entry_is_sealed(client, &forked.info), Ok(true)) {
            return Err(forked.sealed_launch(child, &handed, out));
        }
        let _ = say(
            out,
            &format!(
                "  (the launch could not be recorded, so the trace will read incomplete: {e})"
            ),
        );
    }
    // Launched: the run on the server is this participant's now, and is not given back.
    if let Some(give_back) = forked.give_back.take() {
        give_back.launched();
    }
    // The first chunk goes now, beside the run; nothing the uploader does waits the harness.
    // The run's upload record, kept here so the workspace's commit can be noted the moment the
    // run stops.
    let noted = forked.upload.as_ref().map(|start| start.record.clone());
    // How long the wrap-up after the stop lasts: the upload policy's for a run on a server, and
    // the local default for one without.
    let wrap_spec = WrapUpSpec::for_run(forked.upload.as_ref().map(|start| &start.policy), cx.env);
    let uploads_dir = forked.cfg.join(crate::upload::UPLOADS_DIR_NAME);
    let note_repo_head = || {
        if let Some(record) = &noted {
            crate::upload::uploader::note_repo_head(&uploads_dir, record);
        }
    };
    // The run's state file, which the watch tick keeps and this function changes at each
    // transition of the agent; and what the threads of the run tell each other.
    let mut launched = crate::run::RunState::launched(&entry_id, sleep_lock);
    // What `goalcup status` may advise of `goalcup continue` for this run: only a run on a server
    // can be restarted by it.
    launched.continuable = forked.record.server_url.is_some();
    // What is left of the run, by the server's answer: the uploader keeps it from here (WP11).
    launched.time_left = forked.time_left();
    // The run's model calls so far, which the tick adds this part's to (D73).
    launched.model_calls = Some(forked.model_calls_before.unwrap_or(0));
    let keeper = crate::run::StateKeeper::begin(
        &crate::run::run_dir(forked.cfg, &forked.record.run_id),
        launched,
    );
    // Only a run with a recording proxy can tell that no model call was recorded.
    let watches_calls = forked.proxied.is_some();
    let vitals = match forked.proxied.as_ref() {
        Some(proxied) => crate::run::RunSignals::counting(proxied.proxy.model_requests())
            .recording_from(proxied.proxy.recorded_requests())
            .generations_from(proxied.proxy.generation_requests())
            .client_tls_from(proxied.proxy.client_tls_failures())
            .recorder_from(
                proxied.proxy.recorder_restarts(),
                proxied.proxy.recorder_lost(),
            ),
        None => crate::run::RunSignals::default(),
    }
    .model_calls_before(forked.model_calls_before.unwrap_or(0));
    // A run on a server reports its disk and goalcup's own processes in its heartbeats.
    let vitals = match forked.upload.as_ref() {
        Some(start) => vitals.probing(crate::vitals::Probe::new(
            &crate::run::run_dir(forked.cfg, &forked.record.run_id),
            Path::new(&start.record.trace_dir),
        )),
        None => vitals,
    };

    // The run's one stop channel: the uploader's deadline or heartbeat stop sends on it, and the
    // wait for the harness receives from it. A full disk never stops the run (D65).
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<String>();
    // The agent's pause on a nearly full disk (D65): the watch tick pauses and resumes it, and
    // what is frozen is continued before the harness is ended, once it has exited, and on every
    // way out of this function.
    let pause = crate::run::pause::AgentPause::of_harness(&handed);
    let _continue_on_exit = pause.end_on_drop();
    let uploading = forked.upload.take().and_then(|start| {
        let server = start.server?;
        let spool = forked.cfg.join(crate::upload::UPLOADS_DIR_NAME);
        match crate::upload::uploader::begin(
            server,
            start.record,
            start.key,
            start.policy,
            spool,
            start.local_deadline,
            stop_tx,
            vitals.clone(),
        ) {
            Ok(running) => Some(running),
            Err(e) => {
                let _ = say(out, &format!("  (the upload could not start: {e})"));
                None
            }
        }
    });
    // At the run's deadline the uploader says stop: this command finishes the run there and
    // then, as it would once the harness exits (the recording proxy stopped, then the seal), except
    // that the daemon's seal ends the harness, given its grace. That ends the wait below. What the
    // finish said is printed once the wait is over.
    type Early = (Result<CaptureFinishReport, CliError>, Vec<u8>);
    let early = std::sync::Mutex::new(None::<Early>);
    // The wrap-up of a run stopped at its deadline starts there, not when the harness is gone.
    let stopped_wrap = std::sync::Mutex::new(None::<WrapUp>);
    let proxy_slot = std::sync::Mutex::new(forked.proxied.take());
    let (client, cfg) = (forked.client, forked.cfg);
    let grace = handed.grace_seconds;
    // The watch tick runs on a thread of its own from here to just before the reservation is
    // released (it keeps the reservation sized through the upload's wrap-up too).
    forked.watching = forked.reserve.take().map(|reserve| {
        crate::run::watch::watch_beside(crate::run::watch::Watch {
            reserve,
            run: forked.record.clone(),
            // Every entry this process uploads: this segment's, and the earlier segments being
            // closed beside it, each with its own acknowledged offset.
            uploads: uploading
                .iter()
                .map(|running| running.last_ok())
                .chain(forked.drained.iter().map(|drain| drain.last_ok.clone()))
                .collect(),
            acked: uploading
                .iter()
                .map(|running| (running.entry_id().to_string(), running.acked_offset()))
                .chain(
                    forked
                        .drained
                        .iter()
                        .map(|drain| (drain.entry_id.clone(), drain.acked_offset.clone())),
                )
                .collect(),
            policy: cx.opts.disk,
            beat: Some(crate::run::watch::Beat {
                keeper: keeper.clone(),
                vitals: vitals.clone(),
                ask: {
                    let (cfg, entry_id) = (cfg.to_path_buf(), entry_id.clone());
                    let key = forked.info.run_public_key.clone();
                    Arc::new(move || ask_recording(&cfg, &entry_id, &key))
                },
                time_left_file: Some(crate::run::time_left::file_path(cfg, &forked.record.run_id)),
                // A connector whose harness may write no session file says so (D72): then
                // a missing one is not a problem worth a line.
                native: cx
                    .connector
                    .loaded
                    .manifest
                    .warns_missing_native()
                    .then(|| crate::run::watch::NativeWatch {
                        silence: cx.opts.native_silence.for_run(forked.record.practice),
                        harness: forked.harness_name.clone(),
                        guide: crate::commands::prep::setup_guide_link(cx.connector),
                    }),
                no_calls: watches_calls.then(|| crate::run::watch::NoCallsWatch {
                    after: crate::run::no_model_calls_after(forked.record.practice),
                    harness: forked.harness_name.clone(),
                }),
            }),
            pause: Some(pause.clone()),
        })
    });
    // A harness that suspends itself is continued at once: suspension is not supported during a
    // run, and a stopped harness would hold the participant's terminal frozen. The one stop that
    // is kept is goalcup's own pause on a nearly full disk (D65).
    let mut continued = |signal: i32| {
        if pause.is_paused() {
            return false;
        }
        crate::host_log::say(&format!(
            "the harness stopped itself (signal {signal}); a run cannot be suspended, so it \
             was continued at once"
        ));
        true
    };
    let on_stopped: Option<&mut dyn FnMut(i32) -> bool> = Some(&mut continued);
    let status = crate::upload::wait_stopping(
        &mut child,
        stop_rx,
        |reason| {
            // A paused agent is continued before the seal ends it (a stopped process does not
            // act on SIGTERM), and is never paused again (D65).
            pause.end();
            let Some(proxied) = proxy_slot.lock().unwrap_or_else(|e| e.into_inner()).take() else {
                return;
            };
            // The commit the workspace has now, before the harness's grace lets it change.
            note_repo_head();
            // The one bound of the wrap-up, for the seal below and the upload's finish later.
            let wrap_up = wrap_spec.begin();
            *stopped_wrap.lock().unwrap_or_else(|e| e.into_inner()) = Some(wrap_up);
            keeper.update(|state| {
                state.stopped = Some(reason.to_string());
                state.disk.disk_paused = false;
            });
            let label = if reason == crate::upload::uploader::REPLACED_REASON {
                "stopped"
            } else {
                "deadline"
            };
            let line = format!("{label}: {reason}. The agent is stopped and the run sealed now.");
            // At once, on stderr: the seal below may be asked again for minutes, and the stdout report
            // that follows it is printed only once it returns. A chat-mode run's goes to its log.
            crate::host_log::say(&line);
            let mut said = format!("{line}\n").into_bytes();

            let deadline = std::time::Instant::now() + crate::finish_budget();
            // A seal that did not take (the daemon busy or restarting) is asked again through the
            // whole wrap-up: the harness must not run on past its deadline, and only the daemon's
            // seal ends it.
            let finished = finish(
                client,
                &entry_id,
                proxied,
                deadline,
                wrap_up,
                termination_bound(grace),
                true,
                None,
                Some(cfg),
                &mut said,
            );
            *early.lock().unwrap_or_else(|e| e.into_inner()) = Some((finished, said));
        },
        on_stopped,
    )
    .map_err(|source| CliError::Io {
        path: handed.program.clone(),
        source,
    });
    forked.proxied = proxy_slot.into_inner().unwrap_or_else(|e| e.into_inner());
    // What the harness left frozen is continued before the sweep below signals it (D65).
    pause.end();
    // How long the harness ran, taken before the after-exit commands below can add their time.
    let ran = released_at.elapsed();
    // The harness has exited: the terminal is this command's again, if it is still there. First,
    // so a harness that crashed in raw mode does not leave the terminal raw, with Ctrl-C absorbed,
    // while the after-exit commands below run.
    if let Some(chat) = forked.chat.as_mut() {
        chat.harness_exited();
    }
    // Before the after-exit commands too, so `goalcup status` says the harness has exited while
    // they run.
    keeper.update(|state| {
        state.disk.disk_paused = false;
        // The count at the harness's exit, which the tick's last look may be behind (D73).
        state.model_calls = Some(vitals.model_calls());
        // A resume that failed at once makes the next continue open a new conversation.
        state.resume_failed = failed_resume(forked.opens, &status, ran);
        state.harness = crate::run::HarnessState::Exited {
            status: match &status {
                Ok(status) => describe_exit(status),
                Err(error) => error.to_string(),
            },
        };
    });
    // The connector's after-exit commands (D72, 0.7.1), on every way the harness ended, and
    // before the recording proxy is stopped and drained (`finish_run`): a process the harness
    // left running (Hermes's gateway) would hold the proxy's connections open until its own
    // idle exit. At a deadline the proxy was stopped above, within its short bound, and the
    // commands still end what the harness left. When one of them failed, what the harness left
    // may still hold those connections, and `finish_run` stops the proxy within its short bound
    // instead of waiting for them.
    let helper_left = match forked.after_exit.take() {
        Some(after) => !run_after_exit(cx, &forked, &after, forked.proxied.is_some()),
        None => false,
    };
    // The technical lines that follow (the stop, the seal, the upload) go under one heading; what
    // the participant does next is said after them, last (`finish_run`).
    let _ = say(out, "");
    let _ = say(
        out,
        &crate::participant::details_heading(forked.chat.as_ref().map(|chat| chat.quiet.path())),
    );
    // A run that ended on its own stopped just now.
    let wrap_up = stopped_wrap
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or_else(|| wrap_spec.begin());
    // A run that ended on its own: its commit as it ended.
    if forked.proxied.is_some() {
        note_repo_head();
    }
    let finished_early =
        early
            .into_inner()
            .unwrap_or_else(|e| e.into_inner())
            .map(|(finished, said)| {
                let _ = out.write_all(&said);
                finished
            });
    // What was swept is said on stdout; what may still be running goes to stderr now, and makes
    // the command exit 1 after the seal (`finish_run`), so a script that reads only the exit
    // code still learns of it. The trace is unchanged either way.
    let swept = sweep.map(|sweep| sweep.run(Duration::from_secs(handed.grace_seconds)));
    if let Some(swept) = &swept {
        if let Some(line) = swept.swept_line() {
            let _ = say(out, &line);
        }
        for line in swept.warnings() {
            crate::host_log::say_raw(&line);
        }
    }
    Ok(Exited {
        forked,
        status,
        finished_early,
        helper_left,
        uploading,
        wrap_up,
        left_running: swept.and_then(|swept| swept.refusal(&entry_id)),
        keeper,
    })
}

/// Run the connector's `[launch_prep] after_exit` commands (D72, 0.7.1) in the harness home, with
/// their output and any failure in the run's `host.log`. Nothing here stops the run. Answers
/// whether every command ran and ended cleanly; when one did not and `proxy_running` (the
/// recording proxy still runs, so `finish_run` will stop it), the log also says the proxy is
/// stopped within its short bound.
fn run_after_exit(
    cx: &CaptureRun<'_>,
    forked: &Forked<'_>,
    after: &AfterExit,
    proxy_running: bool,
) -> bool {
    use std::io::Write as _;
    let run_dir = crate::run::run_dir(forked.cfg, &forked.record.run_id);
    let mut log = match crate::host_log::open_for_child(&run_dir) {
        Ok((log, _)) => log,
        Err(e) => {
            crate::host_log::say(&format!(
                "the harness's after-exit commands were not run: its log could not be opened ({e})"
            ));
            return false;
        }
    };
    let ended = crate::launch::after_exit(
        cx.connector,
        &crate::launch::PrepLaunch {
            executable: &after.executable,
            run_home: &after.run_home,
            config_home: &after.config_home,
            parent_env: cx.env,
        },
        &after.workspace,
        &log,
    );
    if !ended && proxy_running {
        let _ = writeln!(
            log,
            "goalcup: an after-exit command did not end cleanly, so the recording proxy is \
             stopped within {} seconds, its open connections cut, instead of waiting for them",
            super::finish::RUNNING_HARNESS_PROXY_DRAIN.as_secs()
        );
    }
    ended
}

/// The cleanup every abandon takes: ask the daemon, and remove the local record only where it
/// agreed. `sentence` is what the participant is told when both went through. Answers
/// whether the daemon abandoned the entry.
fn abandon_because(
    client: &(dyn DaemonClient + Sync),
    cfg: &Path,
    entry_id: &str,
    reason: &str,
    sentence: &str,
    out: &mut dyn std::io::Write,
) -> bool {
    // A Ctrl-C may be what started this abandon; it has been acted on, and must not cut the
    // abandon's own busy retries short.
    crate::arming::close_window();
    let abandoned = retry_busy_entry(|| client.abandon(entry_id, reason));
    let confirmed = abandoned.is_ok();
    let removed = match &abandoned {
        Ok(()) => remove_entry(cfg, entry_id),
        // Not attempted rather than attempted and failed, which is why this is `Ok`: the
        // sentence below names the abandon that was refused, and a second complaint about a
        // file nobody tried to delete would be noise on top of it.
        Err(_) => Ok(()),
    };
    // A write that failed while reporting a failure is not worth a second failure on top of
    // the first — the same call `main.rs` makes when it cannot print the error itself.
    let _ = match (&abandoned, &removed) {
        (Ok(()), Ok(())) => say(out, sentence),
        _ => {
            let mut said = format!("Part {entry_id} could not be cleaned up:");
            if let Err(e) = abandoned {
                said.push_str(&format!("\n  goalcup's recorder refused to drop it: {e}"));
                said.push_str(
                    "\n  this machine's note of the part has been kept, because the recorder \
                     may still be holding it.",
                );
            }
            if let Err(e) = removed {
                said.push_str(&format!("\n  the local record is still there: {e}"));
            }
            said.push_str("\n  `goalcup entries` shows what is left.");
            say(out, &said)
        }
    };
    confirmed
}

/// How the daemon says the recording is, read through a client made from `control.json` each
/// time, so a restarted daemon is followed. `None` when it cannot say, or when what answered is
/// not the daemon that opened the entry (see [`recording_of`]).
fn ask_recording(
    cfg: &Path,
    entry_id: &str,
    run_public_key: &str,
) -> Option<crate::run::watch::Heard> {
    let client = crate::daemon::HttpDaemonClient::from_config_dir(cfg).ok()?;
    recording_of(&client, entry_id, run_public_key)
}

/// The recording, and the files the native source has admitted, as the entry's own signed
/// status says them. The status route is unauthenticated, so a document counts only when it is
/// signed by the run key the entry's creation named and is about this entry: the heartbeat tells
/// the server this word, and a forged "ok" must not.
fn recording_of(
    client: &dyn DaemonClient,
    entry_id: &str,
    run_public_key: &str,
) -> Option<crate::run::watch::Heard> {
    let status = retry_busy(|| client.status(entry_id)).ok()?;
    if status.verify_with(run_public_key).is_err() || status.payload.entry_id != entry_id {
        return None;
    }
    Some(crate::run::watch::Heard {
        answered: true,
        recording: crate::run::Recording::of_capture(status.payload.capture?.capture),
        native_files: status.payload.native_files_admitted,
    })
}

/// A fresh version-4 UUID, the form a pre-assigned harness session id takes.
fn fresh_uuid() -> String {
    let bits = (rand::random::<u128>() & !(0xf << 76) & !(0x3 << 62)) | (0x4 << 76) | (0x2 << 62);
    let hex = format!("{bits:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// Abandon an entry nothing was launched into, and remove what this command made for it: the
/// daemon removes its chain, reservation and still-empty workspace, and the run directory
/// goes once the daemon has agreed.
fn unmake(
    client: &(dyn DaemonClient + Sync),
    cfg: &Path,
    entry_id: &str,
    run: &RunDir,
    cause: &CliError,
    out: &mut dyn std::io::Write,
) -> bool {
    let reason = format!("`goalcup start` failed before the launch: {cause}");
    let sentence = format!(
        "Part {entry_id} is abandoned: nothing was launched, and its record and this \
         machine's note of it are gone."
    );
    let abandoned = abandon_because(client, cfg, entry_id, &reason, &sentence, out);
    if abandoned && let Err(e) = run.remove() {
        let _ = say(
            out,
            &format!(
                "  its run directory {} could not be removed: {e}",
                run.dir.display()
            ),
        );
    }
    abandoned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::MockDaemonClient;

    /// A continue resumes Hermes, whose native row says a missing file is no problem, once the
    /// run has recorded model calls and its home's `state.db` was written since the run started;
    /// a connector that always writes its file keeps the file rule (D73).
    #[test]
    fn a_harness_with_no_session_file_is_resumed_only_after_model_calls() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
        let hermes = crate::connector::load_harness(&dir, "hermes-cli", false).unwrap();
        let codex = crate::connector::load_harness(&dir, "codex-cli", false).unwrap();
        let hermes = &hermes.loaded.manifest.capture;
        let codex = &codex.loaded.manifest.capture;
        let home = tempfile::tempdir().unwrap();
        let now = crate::run::now_unix();
        assert!(
            !resumes_without_a_file(hermes, 3, home.path(), now - 5),
            "no state.db"
        );
        std::fs::write(home.path().join("state.db"), b"").unwrap();
        assert!(resumes_without_a_file(hermes, 3, home.path(), now - 5));
        assert!(
            !resumes_without_a_file(hermes, 3, home.path(), now + 60),
            "written before"
        );
        assert!(!resumes_without_a_file(hermes, 0, home.path(), now - 5));
        assert!(!resumes_without_a_file(codex, 3, home.path(), now - 5));
    }

    /// Claude's `subagent-meta` and `tool-results` rows say a missing file is no problem, but its
    /// main session row does not, so a Claude run keeps the file rule even when a `state.db`
    /// was written since the run started.
    #[test]
    fn a_connector_with_one_file_required_row_keeps_the_file_rule() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
        let claude = crate::connector::load_harness(&dir, "claude-code-cli", false).unwrap();
        let claude = &claude.loaded.manifest.capture;
        assert!(claude.native.iter().any(|native| !native.warn_missing));
        assert!(claude.native.iter().any(|native| native.warn_missing));
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("state.db"), b"").unwrap();
        let now = crate::run::now_unix();
        assert!(!resumes_without_a_file(claude, 3, home.path(), now - 5));
    }

    /// A resumed harness that exits non-zero at once is a failed resume, and the next continue
    /// opens a new conversation; a slow or clean exit, or a part that did not resume, is not
    /// (D73).
    #[test]
    fn a_resume_that_fails_at_once_is_recorded() {
        use crate::participant::Opens;
        use std::os::unix::process::ExitStatusExt;
        let failed: Result<std::process::ExitStatus, CliError> =
            Ok(std::process::ExitStatus::from_raw(1 << 8));
        let clean: Result<std::process::ExitStatus, CliError> =
            Ok(std::process::ExitStatus::from_raw(0));
        let soon = Duration::from_secs(2);
        assert!(failed_resume(Opens::LastSession, &failed, soon));
        assert!(!failed_resume(Opens::LastSession, &clean, soon));
        assert!(!failed_resume(
            Opens::LastSession,
            &failed,
            Duration::from_secs(600)
        ));
        assert!(!failed_resume(Opens::NewSession, &failed, soon));
    }

    /// The count of model calls survives in `state.json` across parts, and a state an older
    /// build wrote reads as none (D73).
    #[test]
    fn the_runs_model_calls_are_counted_across_parts() {
        let counter = Arc::new(std::sync::atomic::AtomicU64::new(2));
        let signals = crate::run::RunSignals::default()
            .generations_from(counter)
            .model_calls_before(5);
        assert_eq!(signals.model_calls(), 7);
        let dir = tempfile::tempdir().unwrap();
        let mut state = crate::run::RunState::launched("e1", Ok(()));
        state.model_calls = Some(7);
        crate::run::StateKeeper::begin(dir.path(), state);
        let read = crate::run::read_state(dir.path()).unwrap();
        assert_eq!(read.model_calls, Some(7));
        let file = dir.path().join(crate::run::STATE_FILE_NAME);
        let mut old: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        old.as_object_mut().unwrap().remove("model_calls");
        std::fs::write(&file, serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(
            crate::run::read_state(dir.path()).unwrap().model_calls,
            None
        );
    }

    /// The interrupt flag is process-global; the tests that move it take turns.
    static FLAG: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The recording the heartbeat reports is the entry's own signed status: a document signed by
    /// another key, about another entry, or edited after it was signed says nothing.
    #[test]
    fn the_recording_is_read_from_the_verified_status_only() {
        use goalcup_protocol::capture::*;
        let tmp = tempfile::tempdir().unwrap();
        let daemon = MockDaemonClient::armed_ok(tmp.path(), "fake");
        daemon.set_capture_status(
            M2Status {
                run: RunState::Running,
                capture: CaptureState::Incomplete,
                delivery: DeliveryState::LocalOnly,
                analysis: AnalysisState::NotRun,
                anchor: AnchorState::LocalOnly,
                storage: CaptureStorage {
                    quota_bytes: 1,
                    reserved_bytes: 1,
                    used_bytes: 0,
                    durable: None,
                    accounting_available: true,
                },
                gap_count: 1,
                assistance_observed: false,
            },
            "gaps",
            1,
            0,
        );
        let info = daemon.info();
        let key = info.run_public_key.clone();
        let entry = info.entry_id.clone();
        assert_eq!(
            recording_of(&daemon, &entry, &key).map(|heard| heard.recording),
            Some(crate::run::Recording::Gaps),
            "signed by the entry's key, about the entry"
        );
        // Another run key: a process that bound the control port first, with a key of its own.
        let other = "00".repeat(32);
        assert_eq!(recording_of(&daemon, &entry, &other), None);
        // A good signature about another entry.
        assert_eq!(recording_of(&daemon, "entry-other", &key), None);
        // Edited after it was signed.
        daemon.tamper_status_payload();
        assert_eq!(recording_of(&daemon, &entry, &key), None);
    }

    /// #7: a Ctrl-C started this abandon; the daemon's ordinary `busy` must be ridden out, not
    /// surfaced at the first try, or the entry stays held with its reservation.
    #[test]
    fn an_abandon_after_ctrl_c_rides_out_a_busy() {
        let _flag = FLAG.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let daemon = MockDaemonClient::armed_ok(tmp.path(), "fake");
        daemon.busy_once("abandon");
        crate::arming::clear_interrupt();
        crate::arming::interrupt_for_test();

        let mut out = Vec::new();
        let abandoned = abandon_because(&daemon, tmp.path(), "entry-1", "ctrl-c", "gone", &mut out);

        crate::arming::clear_interrupt();
        assert!(abandoned, "{}", String::from_utf8_lossy(&out));
        assert_eq!(daemon.abandoned().len(), 1);
    }

    /// #7: once the window is closed (the harness released), a Ctrl-C meant for the harness
    /// is not latched, so the launch record and the seal keep their retries.
    #[test]
    fn a_ctrl_c_after_the_release_is_not_latched() {
        let _flag = FLAG.lock().unwrap_or_else(|e| e.into_inner());
        crate::arming::clear_interrupt();
        crate::arming::close_window();
        crate::arming::interrupt_for_test();
        let latched = crate::arming::interrupted();
        crate::arming::clear_interrupt();
        assert!(!latched);
        crate::arming::interrupt_for_test();
        let reopened = crate::arming::interrupted();
        crate::arming::clear_interrupt();
        assert!(reopened, "the next start's window is open again");
    }
}
