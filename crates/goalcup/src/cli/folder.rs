//! The folder checks of `goalcup start` (PRD-10 section 3 step 2, PRD-17 SC1): every problem in
//! one refusal (app. F 5), found by reading only, so a refused start creates nothing. A synced
//! folder and NFS or SMB are warnings; overlayfs, tmpfs, btrfs, ZFS and FUSE are accepted.

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use super::text;

/// The problems with `project` as a run's folder, one line each, or else the warnings. `home` is
/// `GOALCUP_HOME` from [`super::home`] and need not exist. `needs_token`: a run with `--repo`,
/// whose push block reads `GH_TOKEN` from `.env.local`. A relative `prompt` is taken in `project`.
pub fn check(
    project: &Path,
    home: &Path,
    needs_token: bool,
    prompt: Option<&Path>,
) -> Result<Vec<String>, Vec<String>> {
    let real = resolved(project);
    let (keys, extra) = entries(project);
    let ignore = fs::read_to_string(project.join(".gitignore")).unwrap_or_default();
    let open: Vec<String> = keys.into_iter().filter(|n| !ignored(&ignore, n)).collect();
    let git = real.ancestors().skip(1).find(|a| a.join(".git").exists());
    // SC1: goalcup's home (which holds `runs`) and the project never hold each other or sit side
    // by side, checked on the spelled path with `..` folded and through links.
    let own = tangled(project, &lexical(home)) || tangled(&real, &resolved(home));
    let no_token = needs_token && !fs::read_to_string(project.join(".env.local")).is_ok_and(token);
    let prompt = prompt.and_then(|p| match fs::canonicalize(project.join(p)) {
        Ok(at) => at.starts_with(&real).then(|| text::prompt_inside(p)),
        Err(_) => Some(text::prompt_missing(p)),
    });
    let problems: Vec<String> = [
        (!extra.is_empty()).then(|| text::folder_extra(&extra)),
        (!open.is_empty()).then(|| text::env_not_ignored(&open)),
        git.map(text::inside_git),
        own.then(|| text::beside_goalcup(home)),
        no_token.then(|| text::TOKEN_LINE.to_owned()),
        prompt,
    ]
    .into_iter()
    .flatten()
    .collect();
    if !problems.is_empty() {
        return Err(problems);
    }
    let synced = synced(&real).then(|| text::synced_folder(project));
    let network = [real.as_path(), home].into_iter().filter_map(|dir| {
        let at = dir.ancestors().find(|a| a.exists()).unwrap_or(dir);
        fs_kind(at).map(|kind| text::network_folder(dir, kind))
    });
    Ok(synced.into_iter().chain(network).collect())
}

/// NFS or SMB, by the Linux `statfs` magic or the macOS type name; any other file system is
/// accepted (overlayfs and tmpfs are what containers and sandboxes use).
pub fn network_fs(magic: u64, name: &str) -> Option<&'static str> {
    match (magic, name) {
        (0x6969, _) | (_, "nfs") => Some("NFS"),
        (0xFF53_4D42 | 0xFE53_4D42 | 0x517B, _) | (_, "smbfs") => Some("SMB"),
        _ => None,
    }
}

fn fs_kind(path: &Path) -> Option<&'static str> {
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `c` is NUL-terminated and `st` is a writable statfs for the call.
    if unsafe { libc::statfs(c.as_ptr(), st.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statfs returned 0, so it filled the struct.
    let st = unsafe { st.assume_init() };
    // A Linux magic number fits 32 bits; macOS names the type (not compiled in a box yet).
    #[cfg(not(target_os = "macos"))]
    {
        network_fs(u64::from(st.f_type as u32), "")
    }
    #[cfg(target_os = "macos")]
    {
        let name = st.f_fstypename.map(|c| c as u8);
        let name = name.split(|&b| b == 0).next().unwrap_or_default();
        network_fs(0, &String::from_utf8_lossy(name))
    }
}

/// The key files (`.env`, `.env.*`) and every entry besides them, `.gitignore` and `.DS_Store`,
/// sorted. Only regular files are the kit; an entry that cannot be read is another entry, and a
/// folder that cannot be listed holds entries that could not be listed (never passes as empty).
fn entries(project: &Path) -> (Vec<String>, Vec<String>) {
    let (mut keys, mut extra) = (Vec::new(), Vec::new());
    let Ok(list) = fs::read_dir(project) else {
        return (keys, vec![text::UNLISTED_FOLDER.to_owned()]);
    };
    for entry in list {
        let Ok(entry) = entry else {
            extra.push(text::UNREADABLE_ENTRY.to_owned());
            continue;
        };
        let file = entry.file_type().is_ok_and(|t| t.is_file());
        let name = entry.file_name().to_string_lossy().into_owned();
        match (file, name.as_str()) {
            (true, ".gitignore" | ".DS_Store") => {}
            (true, n) if n == ".env" || n.starts_with(".env.") => keys.push(name),
            _ => extra.push(name),
        }
    }
    extra.sort();
    (keys, extra)
}

/// Exactly one line `GH_TOKEN=<value>`, the value visible ASCII with no quotes (so no CR): the
/// push block's `sed -n s/^GH_TOKEN=//p` then prints one plain value. It is never kept or shown.
fn token(env_local: String) -> bool {
    let mut values = env_local
        .split('\n')
        .filter_map(|l| l.strip_prefix("GH_TOKEN="));
    let plain = |b: u8| b.is_ascii_graphic() && !b"\"'".contains(&b);
    let one = values
        .next()
        .filter(|v| !v.is_empty() && v.bytes().all(plain));
    one.is_some() && values.next().is_none()
}

/// One equals, is inside or holds the other, or both sit in one folder.
fn tangled(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a) || a.parent() == b.parent()
}

/// `path` with its nearest existing ancestor resolved (links followed) and the rest appended,
/// `..` folded: the rest does not exist, so goalcup would make it as plain folders.
fn resolved(path: &Path) -> PathBuf {
    for at in path.ancestors() {
        if let Ok(real) = fs::canonicalize(at) {
            return lexical(&real.join(path.strip_prefix(at).unwrap_or(Path::new(""))));
        }
    }
    lexical(path)
}

/// `path` with each `..` taking off the part before it, as written (no link is followed).
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            part => out.push(part),
        }
    }
    out
}

/// Whether the root `.gitignore` text ignores the root file `name`: the last match wins, `!`
/// re-includes, a trailing `/` matches folders only, a leading `/` or `**/` anchors, and any other
/// `/` names a path below the root. Kept short, so it errs to "not ignored" (a refusal): a class
/// `[..]` or a `\` after the first byte is not read, so such a line never ignores, and such a `!`
/// line always re-includes.
fn ignored(ignore: &str, name: &str) -> bool {
    let mut verdict = false;
    for line in ignore.lines().map(str::trim_end) {
        if line.is_empty() || line.starts_with('#') || line.ends_with('/') {
            continue;
        }
        let (negated, pat) = match line.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, line.strip_prefix('\\').unwrap_or(line)),
        };
        let pat = pat.strip_prefix("**/").unwrap_or(pat);
        let pat = pat.strip_prefix('/').unwrap_or(pat);
        let unread = negated && pat.contains(['[', '\\']);
        if !pat.contains('/') && (unread || glob(pat.as_bytes(), name.as_bytes())) {
            verdict = !negated;
        }
    }
    verdict
}

/// Git's wildcard match of one path part: `*` any run, `?` one byte, every other byte itself.
fn glob(pat: &[u8], text: &[u8]) -> bool {
    match pat.first() {
        None => text.is_empty(),
        Some(b'*') => (0..=text.len()).any(|skip| glob(&pat[1..], &text[skip..])),
        Some(b'?') => !text.is_empty() && glob(&pat[1..], &text[1..]),
        Some(b'[') => false,
        Some(p) => text.first() == Some(p) && glob(&pat[1..], &text[1..]),
    }
}

/// A folder a sync client uploads, by name on the resolved path (a warning, so loose is fine).
fn synced(real: &Path) -> bool {
    let all = real.to_string_lossy();
    let names = [
        "Dropbox",
        "OneDrive",
        "Google Drive",
        "iCloud",
        "/Mobile Documents/",
        "/Library/CloudStorage/",
    ];
    names.iter().any(|n| all.contains(n))
}
