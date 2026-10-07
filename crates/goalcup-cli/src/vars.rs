//! The eight template variables a launch recipe may name, and the three — and only three —
//! places they are built: workspace creation, normal resume, and read-only resume preview.
//!
//! The daemon owns the workspace: it creates the directory, renders and writes the
//! connector's `hook_install[]` files, records their expected hashes and reserves the
//! server port. What is left to the CLI is the half it genuinely owns — turning the
//! `WorkspaceInfo` that came back into the variables the *launch* recipe renders against.
//!
//! Every value here comes out of the daemon's own answer. Nothing reads an environment
//! variable, nothing rebuilds a path by joining a root to a filename it guessed, and
//! nothing falls back to a default when a field is missing. A participant who exports
//! `CLAUDE_CONFIG_DIR` cannot move where this run's harness home is, because the string
//! that lands in the launch environment is the one the daemon created the directory at.
//!
//! # Three builders, one list
//!
//! [`from_workspace`], [`from_resume`] and [`from_resume_preview`] fill the same eight
//! names. A normal resume that rendered even one differently would capture the second half
//! of an entry differently from the first; the read-only preview must render that exact same
//! relaunch line. The equality is asserted directly, name by name, in `tests/vars_launch.rs`.
//!
//! # Values are byte-exact, and quoting is the consumer's
//!
//! A value goes into the map exactly as the daemon spelled it and comes out of
//! [`render`] byte for byte: no trimming, no escaping, no quoting. A workspace path
//! containing a space renders with the space in it. Nothing here can know whether the
//! rendered string is about to become one `argv` element (the launcher, which passes
//! it to `Command::arg` and so needs no quoting at all), an environment value, or a line in
//! a file the daemon writes — and a module that guessed would either double-quote an argv
//! element or leave a shell-bound one unquoted. Quoting belongs to whoever hands the string
//! to something that parses it.
//!
//! [`render`]: goalcup_protocol::template::render
//!
//! # `run_id` is `entry_id`
//!
//! None of `WorkspaceInfo`, `ResumeInfo`, or `ResumePreviewInfo` carries a `run_id` field,
//! and the entry id is the identifier every other surface already speaks. All three builders
//! set the alias to the same string, so a manifest may spell it either way and mean one
//! thing. Rendering `{run_id}` against nothing would either fail the launch or — worse, had
//! the token been quietly dropped — start a tracked run whose own environment named an empty
//! run. If a run is ever given an identity distinct from an entry, it gets a field on all
//! three structs first and this alias goes away in the same change.
//!
//! # `entry_id`
//!
//! A hook-install **body** variable the daemon fills in its own pass. It is
//! supplied here too, from the daemon's answer, so a launch template may name it.
//!
//! # `server_port`
//!
//! Renders as the empty string when the port is `None`. No shipped connector names
//! `{server_port}`; the daemon reserves a port for any launch or resume template that does,
//! whatever the launch mode, and [`crate::launch::plan`] refuses an empty `{server_port}` in
//! any argv that names it rather than launching a harness on port `""`. On a resume the port
//! comes from the registry row the daemon keeps. A `None` reaching a recipe that names it is
//! a bug worth naming, and the refusal is for exactly that case.
//!
//! # No receiver variables
//!
//! `receiver_http` and `receiver_token` are not in this list, and `WorkspaceInfo`,
//! `ResumeInfo` and `ResumePreviewInfo` carry no receiver fields. A manifest that names one
//! fails to render rather than launching on a value nothing reads.

use crate::CliError;
use goalcup_protocol::control::{ResumeInfo, ResumePreviewInfo, WorkspaceInfo};
use goalcup_protocol::template::TemplateVars;

/// The eight variables every launch, resume, status-line and preflight-path template may
/// use. Built in exactly three places — workspace creation from [`WorkspaceInfo`], a normal
/// resume from [`ResumeInfo`], and a read-only resume preview from [`ResumePreviewInfo`] — so
/// a relaunch cannot silently differ from the original launch.
///
/// The same list `tests/shipped_connectors.rs` checks every shipped manifest's
/// `template_vars_used()` against, so a manifest can never name a placeholder no builder
/// fills. It is **not** `manifest::PATH_TOKENS`, which is the separate language the daemon
/// renders *path* templates in (`home`, `config_dir`, `slug`, `yyyy`, …); the two lists
/// overlap on three names and mean the same thing by each of them, and a manifest that
/// reaches a launch template for one of the path-only names is refused by [`render`] here
/// rather than launched on an empty value.
///
/// [`render`]: goalcup_protocol::template::render
pub const SUPPLIED: [&str; 8] = [
    "workspace",
    "artifact_root",
    "harness_home",
    "session_id",
    "entry_id",
    "run_id",
    "server_port",
    "status_line_path",
];

/// The launch variables, from what `POST /v0/workspaces` answered.
pub fn from_workspace(info: &WorkspaceInfo) -> TemplateVars {
    build(Fields {
        workspace: &info.workspace,
        artifact_root: &info.artifact_root,
        harness_home: &info.harness_home,
        session_id: &info.session_id,
        entry_id: &info.entry_id,
        server_port: info.server_port,
        status_line_path: &info.status_line_path,
    })
}

/// The same eight, from what a resume answered.
///
/// `entry_id` is a parameter rather than read off `info.entry_id` because the id the caller
/// asked to resume is the one the rest of the command is already working in terms of —
/// `goalcup resume <entry_id>` resolved it, cached under it and will register a process
/// against it. `ResumeInfo` echoes the same string back; taking the caller's means a daemon
/// that echoed a different one cannot quietly move this launch's `GOALCUP_RUN` to another
/// entry.
///
/// The echo is checked rather than assumed. Preferring the caller's id keeps the *variables*
/// pointing at the entry the participant asked for, but the rest of `info` — the workspace,
/// the artifact root, the harness home — is the daemon's answer, and if
/// those belong to a different entry than the id says, then a resume of `A` would launch
/// into `B`'s directories while telling the harness it is `A`. Two ids that disagree is that
/// state, so it is a refusal (rung 1) naming both rather than a silent preference for one.
pub fn from_resume(info: &ResumeInfo, entry_id: &str) -> Result<TemplateVars, CliError> {
    from_resume_fields(
        &info.entry_id,
        entry_id,
        Fields {
            workspace: &info.workspace,
            artifact_root: &info.artifact_root,
            harness_home: &info.harness_home,
            session_id: &info.session_id,
            entry_id,
            server_port: info.server_port,
            status_line_path: &info.status_line_path,
        },
    )
}

/// The same launch variables, from the side-effect-free resume preview answer.
pub fn from_resume_preview(
    info: &ResumePreviewInfo,
    entry_id: &str,
) -> Result<TemplateVars, CliError> {
    from_resume_fields(
        &info.entry_id,
        entry_id,
        Fields {
            workspace: &info.workspace,
            artifact_root: &info.artifact_root,
            harness_home: &info.harness_home,
            session_id: &info.session_id,
            entry_id,
            server_port: info.server_port,
            status_line_path: &info.status_line_path,
        },
    )
}

/// Check the echoed entry id before either normal or read-only resume answer becomes launch
/// variables.
fn from_resume_fields(
    answered_entry_id: &str,
    entry_id: &str,
    fields: Fields<'_>,
) -> Result<TemplateVars, CliError> {
    if answered_entry_id != entry_id {
        return Err(CliError::Refused(format!(
            "resume of entry `{entry_id}` was answered for entry `{answered_entry_id}`; the \
             workspace and home in that answer belong to a different entry, so nothing is \
             launched"
        )));
    }
    Ok(build(fields))
}

/// What all three builders have in common, named once.
///
/// It exists so the three are one function with field-readers in front of it rather than
/// three `TemplateVars::set` sequences that agree today. A name added to [`SUPPLIED`] and
/// not to `build` is a compile error on the array's length; a name set on one side and not
/// the other is not expressible at all.
struct Fields<'a> {
    workspace: &'a str,
    artifact_root: &'a str,
    harness_home: &'a str,
    session_id: &'a str,
    entry_id: &'a str,
    server_port: Option<u16>,
    status_line_path: &'a str,
}

fn build(f: Fields<'_>) -> TemplateVars {
    let mut v = TemplateVars::new();
    v.set("workspace", f.workspace);
    v.set("artifact_root", f.artifact_root);
    v.set("harness_home", f.harness_home);
    v.set("session_id", f.session_id);
    v.set("entry_id", f.entry_id);
    // The alias. Same string, all three builders — see the module header.
    v.set("run_id", f.entry_id);
    // Written as a match rather than as an `unwrap_or_default`, because the `None` arm is
    // deliberate and not a fallback: an unreserved port renders empty here and
    // `launch::plan` refuses an argv that names `{server_port}` and carries one, rather than
    // this module inventing a number.
    v.set(
        "server_port",
        match f.server_port {
            Some(port) => port.to_string(),
            None => String::new(),
        },
    );
    v.set("status_line_path", f.status_line_path);
    // The daemon's, filled in its own hook-install pass. Supplied so a launch template
    // naming it renders rather than failing the launch over a variable that is not this
    // side's to fill.
    v
}
