//! The project folder of a new run on a server: the checks made before the approval (A1), and
//! the token the participant keeps in it.
//!
//! A new run works in the folder `goalcup start` is run from. Before the participant is asked to
//! approve anything, that folder must be: apart from goalcup's own folders (the daemon refuses a
//! workspace inside, around or beside them), outside any Git work tree (the agent runs `git init`
//! there), empty but for the starting kit (`.gitignore`, and `.env` / `.env.*` files), and every
//! one of those `.env` files covered by a `.gitignore` line, so the agent's first `git add .`
//! cannot publish the token. Each refusal says what to do, and that nothing was created.

use crate::CliError;
use goalcup_protocol::run_secret::RunSecret;
use std::path::{Path, PathBuf};

/// The variable the participant keeps their GitHub token in, in one of the folder's `.env` files.
pub const TOKEN_VARIABLE: &str = "GH_TOKEN";

/// The file the guide tells the participant to keep the token in.
pub const TOKEN_FILE: &str = ".env.local";

/// Every check of a new run's project folder, in the order a participant fixes them.
pub(super) fn refuse_unready_project(cfg: &Path, project: &Path) -> Result<(), CliError> {
    refuse_beside_goalcup(cfg, project)?;
    let real = std::fs::canonicalize(project).map_err(|source| CliError::Io {
        path: project.display().to_string(),
        source,
    })?;
    if let Some(outer) = real
        .parent()
        .and_then(goalcup_platform::clone::git_worktree_of)
    {
        return Err(CliError::Refused(
            crate::participant::project_inside_repository(project, &outer),
        ));
    }
    let names = kit_or_refuse(project)?;
    let ignore = std::fs::read_to_string(project.join(".gitignore")).unwrap_or_default();
    let uncovered: Vec<String> = names
        .iter()
        .filter(|name| is_key_file(name) && !ignored(&ignore, name))
        .cloned()
        .collect();
    if !uncovered.is_empty() {
        return Err(CliError::Refused(crate::participant::env_not_ignored(
            &uncovered,
        )));
    }
    refuse_unreadable_token_line(project, &names)
}

/// The push line the launch block prints reads the token with `sed -n s/^GH_TOKEN=//p`, so a key
/// file that names `GH_TOKEN` must do so in that form: exactly one `GH_TOKEN=<value>` line in all
/// the key files, with no `export`, no quotes, no spaces and no CR, and a value. Anything else
/// would fail the agent's first push, after the approval; it is refused here instead, naming the
/// file and the form, never the value. A folder with no `GH_TOKEN` line is not refused here.
fn refuse_unreadable_token_line(project: &Path, names: &[String]) -> Result<(), CliError> {
    let mut found: Vec<(&str, bool)> = Vec::new();
    for name in names.iter().filter(|name| is_key_file(name)) {
        let Ok(bytes) = std::fs::read(project.join(name)) else {
            continue;
        };
        for line in String::from_utf8_lossy(&bytes).split('\n') {
            if names_token(line) {
                found.push((name, plain_token_line(line)));
            }
        }
    }
    match found.as_slice() {
        [] | [(_, true)] => Ok(()),
        [(file, _), ..] => Err(CliError::Refused(crate::participant::token_line_form(file))),
    }
}

/// Whether `line` sets `GH_TOKEN` in any form a key file might use (`export`, spaces around `=`).
fn names_token(line: &str) -> bool {
    let line = line.trim();
    let line = line
        .strip_prefix("export")
        .filter(|rest| rest.starts_with(char::is_whitespace))
        .map_or(line, str::trim_start);
    line.strip_prefix(TOKEN_VARIABLE)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// Whether `line` is exactly `GH_TOKEN=<value>`, the value one or more visible ASCII characters
/// that are not quotes.
fn plain_token_line(line: &str) -> bool {
    line.strip_prefix("GH_TOKEN=").is_some_and(|value| {
        !value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\'')
    })
}

/// Whether `name` is one of the participant's key files: `.env`, or `.env.` and anything.
fn is_key_file(name: &str) -> bool {
    name == ".env" || name.starts_with(".env.")
}

/// The folder's entries, sorted, when it holds nothing but the starting kit (`.gitignore` and
/// key files, as files); else the refusal that names what else is there.
fn kit_or_refuse(project: &Path) -> Result<Vec<String>, CliError> {
    let entries = std::fs::read_dir(project).map_err(|source| CliError::Io {
        path: project.display().to_string(),
        source,
    })?;
    let mut kit = Vec::new();
    let mut extra = Vec::new();
    // An entry that cannot be read is not known to be the kit: refused like anything else.
    for entry in entries {
        let Ok(entry) = entry else {
            extra.push("an entry that could not be read".to_string());
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let file = entry.file_type().is_ok_and(|t| t.is_file());
        if file && (name == ".gitignore" || is_key_file(&name)) {
            kit.push(name);
        } else {
            extra.push(name);
        }
    }
    if !extra.is_empty() {
        extra.sort();
        return Err(CliError::Refused(crate::participant::project_not_empty(
            project, &extra,
        )));
    }
    kit.sort();
    Ok(kit)
}

/// The project folder must not be goalcup's configuration folder, its artifact root or its runs
/// folder, be inside one, hold one, or sit beside one: the daemon refuses such a workspace, and
/// that refusal would come after the approval.
fn refuse_beside_goalcup(cfg: &Path, project: &Path) -> Result<(), CliError> {
    let roots = [
        ("configuration folder", cfg.to_path_buf()),
        ("record folder", crate::run::reserve::artifact_root(cfg)),
        ("runs folder", crate::home::run_folders(cfg)),
    ];
    let here = resolved(project);
    for (what, root) in &roots {
        let there = resolved(root);
        let nested = here == there || here.starts_with(&there) || there.starts_with(&here);
        if nested || here.parent() == there.parent() {
            return Err(CliError::Refused(
                crate::participant::project_beside_goalcup(project, what, root),
            ));
        }
    }
    Ok(())
}

/// `path` with its nearest existing ancestor resolved (links followed) and the rest appended.
fn resolved(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut rest = Vec::new();
    let mut at = absolute.as_path();
    loop {
        if let Ok(real) = std::fs::canonicalize(at) {
            return rest.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                at = parent;
            }
            _ => return absolute,
        }
    }
}

// ------------------------------------------------------------------ `.gitignore`

/// Whether the `.gitignore` text `ignore` ignores the file `name` at the folder's root, by Git's
/// rules for a root file: the last pattern that matches decides (`!` re-includes), a pattern
/// ending in `/` names folders only, a leading `/` or `**/` anchors at the root, and a pattern
/// with any other `/` names a path below the root, never a root file.
fn ignored(ignore: &str, name: &str) -> bool {
    let mut verdict = false;
    for line in ignore.lines() {
        let line = line.trim_end_matches('\r');
        let line = trim_unescaped_spaces(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (negated, pattern) = match line.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, line.strip_prefix('\\').unwrap_or(line)),
        };
        if pattern.ends_with('/') {
            continue;
        }
        let pattern = pattern
            .strip_prefix("**/")
            .or_else(|| pattern.strip_prefix('/'))
            .unwrap_or(pattern);
        if pattern.contains('/') {
            continue;
        }
        if glob(pattern.as_bytes(), name.as_bytes()) {
            verdict = !negated;
        }
    }
    verdict
}

/// `line` without trailing spaces, except one escaped with a backslash.
fn trim_unescaped_spaces(line: &str) -> &str {
    let mut end = line.len();
    while end > 0 && line.as_bytes()[end - 1] == b' ' {
        if end >= 2 && line.as_bytes()[end - 2] == b'\\' {
            break;
        }
        end -= 1;
    }
    &line[..end]
}

/// Git's wildcard match of one path component: `*` any run, `?` one character, `[...]` a class
/// (`!` or `^` negates it, `a-z` ranges), `\` escapes the next character.
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.first() {
        None => text.is_empty(),
        Some(b'*') => {
            let rest = &pattern[1..];
            (0..=text.len()).any(|skip| glob(rest, &text[skip..]))
        }
        Some(b'?') => !text.is_empty() && glob(&pattern[1..], &text[1..]),
        Some(b'[') => match (class(&pattern[1..]), text.first()) {
            (Some((matches, used)), Some(&c)) => {
                matches(c) && glob(&pattern[1 + used..], &text[1..])
            }
            (None, Some(&c)) => c == b'[' && glob(&pattern[1..], &text[1..]),
            (_, None) => false,
        },
        Some(b'\\') if pattern.len() > 1 => {
            text.first() == Some(&pattern[1]) && glob(&pattern[2..], &text[1..])
        }
        Some(&p) => text.first() == Some(&p) && glob(&pattern[1..], &text[1..]),
    }
}

/// A character class after its `[`: whether a character is in it, and how many bytes of the
/// pattern it took (its `]` included). `None` when it has no closing `]`.
fn class(pattern: &[u8]) -> Option<(impl Fn(u8) -> bool, usize)> {
    let (negated, start) = match pattern.first() {
        Some(b'!' | b'^') => (true, 1),
        _ => (false, 0),
    };
    let mut ranges = Vec::new();
    let mut i = start;
    loop {
        let &c = pattern.get(i)?;
        if c == b']' && i > start {
            break;
        }
        let low = if c == b'\\' {
            i += 1;
            *pattern.get(i)?
        } else {
            c
        };
        if pattern.get(i + 1) == Some(&b'-') && pattern.get(i + 2).is_some_and(|&e| e != b']') {
            ranges.push((low, pattern[i + 2]));
            i += 3;
        } else {
            ranges.push((low, low));
            i += 1;
        }
    }
    let matches = move |c: u8| ranges.iter().any(|&(lo, hi)| lo <= c && c <= hi) != negated;
    Some((matches, i + 1))
}

// ------------------------------------------------------------------ the token

/// Where the participant keeps their GitHub token in `project`: the key file holding a
/// `GH_TOKEN=` line (`.env.local` first, then the others by name), and the value when it is one
/// the recorder can redact by value ([`RunSecret`]). `None` when no key file names it.
pub(crate) fn token(project: &Path) -> Option<(String, Option<RunSecret>)> {
    let mut names: Vec<String> = std::fs::read_dir(project)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| is_key_file(name))
        .collect();
    names.sort_by_key(|name| (name != TOKEN_FILE, name.clone()));
    names.into_iter().find_map(|name| {
        let text = std::fs::read_to_string(project.join(&name)).ok()?;
        let value = token_in(&text)?;
        Some((name, RunSecret::new(value.as_bytes())))
    })
}

/// The value of the last `GH_TOKEN=` line of a key file (an `export ` before it is allowed, and
/// one pair of quotes around the value is taken off).
fn token_in(text: &str) -> Option<String> {
    text.lines().rev().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let value = line.strip_prefix(TOKEN_VARIABLE)?.trim_start();
        let value = value.strip_prefix('=')?.trim();
        let unquoted = ['"', '\'']
            .iter()
            .find_map(|q| value.strip_prefix(*q)?.strip_suffix(*q))
            .unwrap_or(value);
        (!unquoted.is_empty()).then(|| unquoted.to_string())
    })
}
