//! `goalcup start` (PRD-10 section 3). Built so far: the refusals that come before anything is
//! read or created, and the folder checks of step 2. The steps that follow come in a later slice.

use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use super::{fail, folder, home, say, text};

/// `file_flags`: `--connector` or `--hosts` was given; `trial`: a practice or test run; `repo`:
/// `--repo` was given, so the push block needs `GH_TOKEN`.
pub fn run(trial: bool, file_flags: bool, repo: bool, prompt: Option<&Path>) -> ExitCode {
    if file_flags && !trial {
        return fail(text::FILE_FLAGS_COMPETE);
    }
    // Section 3's order: the connector (step 1) first, then the folder checks (step 2).
    let home = match home() {
        Ok(home) => home,
        Err(e) => return fail(e),
    };
    let Ok(project) = std::env::current_dir() else {
        return fail(text::PROJECT_GONE);
    };
    match folder::check(&project, &home, repo, prompt) {
        Err(problems) => {
            say(text::FOLDER_REFUSED);
            let mut err = std::io::stderr().lock();
            for p in problems {
                let _ = writeln!(err, "  {p}");
            }
            let _ = writeln!(err, "{}", text::NOTHING_CREATED);
            return ExitCode::from(1);
        }
        Ok(warnings) => {
            for w in warnings {
                say(&w);
            }
        }
    }
    say(&text::not_built("start"));
    ExitCode::from(3)
}
