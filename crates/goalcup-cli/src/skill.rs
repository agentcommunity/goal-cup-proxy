//! The goal-cup skill, put in a chat-mode harness's own skills folder before each launch (D57).
//!
//! The text is `skills/goal-cup/SKILL.md` at the repository root, embedded in the binary: one
//! file, no path looked up at run time, and so no way for a participant's install to hand the
//! harness a different one. It tells the agent what goalcup records, that the participant
//! may type only "keep going" after the first prompt, and how to ask the time left (D66).
//!
//! [`install`] replaces `<config_home>/<skills_dir>/goal-cup/` and nothing else in the harness's
//! home: the participant's own skills, settings and sessions there are not read or changed.

use crate::CliError;
use std::path::Path;

/// The skill's folder name under the harness's skills folder.
pub const SKILL_DIR_NAME: &str = "goal-cup";

/// The skill's one file, as shipped.
pub const SKILL_TEXT: &str = include_str!("../../../skills/goal-cup/SKILL.md");

/// Put the skill at `<config_home>/<skills_dir>/goal-cup/SKILL.md`, replacing whatever that one
/// folder held. The new folder is written beside it under a name of its own and renamed into
/// place, so the harness never sees half a skill. A link where the folder goes is removed, never
/// followed. `skills_dir` is the connector's, which the schema holds to a relative path below the
/// home.
pub fn install(config_home: &Path, skills_dir: &str) -> Result<(), CliError> {
    let skills = config_home.join(skills_dir);
    let io = |path: &Path| {
        let path = path.display().to_string();
        move |source| CliError::Io { path, source }
    };
    private_dirs(&skills).map_err(io(&skills))?;
    let target = skills.join(SKILL_DIR_NAME);
    let staging = skills.join(format!(".{SKILL_DIR_NAME}-{:016x}", rand::random::<u64>()));
    let staged = stage(&staging);
    let placed = staged.and_then(|()| {
        match std::fs::symlink_metadata(&target) {
            Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&target)?,
            Ok(_) => std::fs::remove_file(&target)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        std::fs::rename(&staging, &target)
    });
    if let Err(source) = placed {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(CliError::Io {
            path: target.display().to_string(),
            source,
        });
    }
    Ok(())
}

/// The new folder and its one file, both private to the participant.
fn stage(staging: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let mut dir = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        dir.mode(0o700);
    }
    dir.create(staging)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(staging.join("SKILL.md"))?;
    file.write_all(SKILL_TEXT.as_bytes())?;
    file.sync_all()
}

fn private_dirs(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The skill's fallback for an agent that cannot find `goalcup` names the variable the
    /// launch sets and the keys the time-left file holds: the text is the contract here.
    #[test]
    fn the_skill_names_the_time_left_file_the_launch_hands_the_agent() {
        use crate::run::time_left::{FILE_ENV, keep_file};
        assert!(SKILL_TEXT.contains(&format!("${FILE_ENV}")));
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("left");
        let left = crate::run::time_left::TimeLeft::counted(
            3600,
            goalcup_protocol::clock::continuous_ns(),
        );
        keep_file(&file, Some(&left), Some("2026-10-29T00:00:00Z"), 1, false).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        for line in text.lines() {
            let key = line.split('=').next().unwrap();
            assert!(SKILL_TEXT.contains(&format!("`{key}`")), "{key}");
        }
    }

    #[test]
    fn the_skill_replaces_its_own_folder_and_nothing_beside_it() {
        let home = tempfile::tempdir().unwrap();
        let skills = home.path().join("skills");
        std::fs::create_dir_all(skills.join("theirs")).unwrap();
        std::fs::write(skills.join("theirs/SKILL.md"), "theirs").unwrap();
        std::fs::create_dir_all(skills.join("goal-cup")).unwrap();
        std::fs::write(skills.join("goal-cup/old.md"), "old").unwrap();

        install(home.path(), "skills").unwrap();

        assert_eq!(
            std::fs::read_to_string(skills.join("goal-cup/SKILL.md")).unwrap(),
            SKILL_TEXT
        );
        assert!(!skills.join("goal-cup/old.md").exists());
        assert_eq!(
            std::fs::read_to_string(skills.join("theirs/SKILL.md")).unwrap(),
            "theirs"
        );
        let names: Vec<_> = std::fs::read_dir(&skills)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_where_the_skill_goes_is_replaced_not_followed() {
        let home = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(elsewhere.path().join("keep.md"), "keep").unwrap();
        std::fs::create_dir_all(home.path().join("skills")).unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), home.path().join("skills/goal-cup")).unwrap();

        install(home.path(), "skills").unwrap();

        assert!(home.path().join("skills/goal-cup/SKILL.md").is_file());
        assert!(
            !std::fs::symlink_metadata(home.path().join("skills/goal-cup"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(elsewhere.path().join("keep.md")).unwrap(),
            "keep"
        );
    }
}
