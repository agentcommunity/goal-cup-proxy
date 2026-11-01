//! The goal-cup skill (PRD-10 section 3 step 8): `skills/goal-cup/SKILL.md`, embedded, put in the
//! harness's own skills folder before each launch. It replaces `<skills>/goal-cup/` and nothing
//! else there. A skills folder outside the harness home installs nothing: goalcup adds nothing to
//! the project folder. A failure is one `goalcup.log` line, never a reason to stop (point 3).

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Component, Path, PathBuf};

use crate::cli::log::{Fact, line};
use crate::connector::launch::fill;

/// The skill's one file, as shipped.
pub const TEXT: &str = include_str!("../../../../skills/goal-cup/SKILL.md");

/// The folder name under the skills folder.
const NAME: &str = "goal-cup";

/// Puts the skill at `<skills_dir>/goal-cup/SKILL.md`. When nothing is installed a `goalcup.log`
/// line in `entry_dir` says why.
pub fn install(skills_dir: Option<&str>, home: &Path, workspace: &Path, entry_dir: &Path) {
    let Some(skills_dir) = skills_dir else {
        return;
    };
    let dir = PathBuf::from(fill(skills_dir, home, workspace));
    let inside = dir.starts_with(home) && !dir.components().any(|c| c == Component::ParentDir);
    if !inside {
        line(
            entry_dir,
            "skill_not_installed",
            &[("why", Fact::Word("outside_harness_home"))],
        );
        return;
    }
    if let Err(e) = place(&dir) {
        line(
            entry_dir,
            "skill_not_installed",
            &[("io", Fact::Io(e.kind()))],
        );
    }
}

/// Written beside the target under a name of its own, then renamed into place, so the harness
/// never sees half a skill. A link where the folder goes is removed, never followed.
fn place(skills: &Path) -> std::io::Result<()> {
    let private =
        |p: &Path, recursive: bool| DirBuilder::new().recursive(recursive).mode(0o700).create(p);
    private(skills, true)?;
    let staging = skills.join(format!(".{NAME}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    private(&staging, false)?;
    let target = skills.join(NAME);
    let done = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(staging.join("SKILL.md"))?;
        f.write_all(TEXT.as_bytes())?;
        f.sync_all()?;
        match fs::symlink_metadata(&target) {
            Ok(m) if m.is_dir() => fs::remove_dir_all(&target)?,
            Ok(_) => fs::remove_file(&target)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        fs::rename(&staging, &target)
    })();
    if done.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    done
}
