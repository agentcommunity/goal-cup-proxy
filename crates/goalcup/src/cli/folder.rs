//! The folder checks of `goalcup start` (PRD-10 section 3 step 2, PRD-17 SC1): every problem in
//! one refusal (app. F 5), found by reading only, so a refused start creates nothing. A synced
//! folder and NFS or SMB are warnings; overlayfs, tmpfs, btrfs, ZFS and FUSE are accepted.

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

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
    // SC1: goalcup's folders and the project never hold each other or sit side by side, checked
    // on the spelled path (a `..` in it is taken as spelled) and through links.
    let own = [home.to_path_buf(), super::runs_dir(home)]
        .into_iter()
        .find(|own| tangled(project, own) || tangled(&real, &resolved(own)));
    let token = |t: String| t.split('\n').any(plain_token_line);
    let no_token = needs_token && !fs::read_to_string(project.join(".env.local")).is_ok_and(token);
    let prompt = prompt.and_then(|p| match fs::canonicalize(project.join(p)) {
        Ok(at) => at.starts_with(&real).then(|| text::prompt_inside(p)),
        Err(_) => Some(text::prompt_missing(p)),
    });
    let problems: Vec<String> = [
        (!extra.is_empty()).then(|| text::folder_extra(&extra)),
        (!open.is_empty()).then(|| text::env_not_ignored(&open)),
        git.map(text::inside_git),
        own.map(|own| text::beside_goalcup(&own)),
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
    // macOS names the type (not compiled in a box yet); a Linux magic number fits 32 bits.
    #[cfg(target_os = "macos")]
    let (magic, name) = (0, st.f_fstypename.map(|c| c as u8));
    #[cfg(not(target_os = "macos"))]
    let (magic, name) = (u64::from(st.f_type as u32), [0u8; 1]);
    let name = name.split(|&b| b == 0).next().unwrap_or_default();
    network_fs(magic, &String::from_utf8_lossy(name))
}

/// The key files (`.env`, `.env.*`) and every entry besides them, `.gitignore` and `.DS_Store`,
/// sorted. Only regular files are the kit; an entry that cannot be read is another entry.
fn entries(project: &Path) -> (Vec<String>, Vec<String>) {
    let (mut keys, mut extra) = (Vec::new(), Vec::new());
    for entry in fs::read_dir(project).into_iter().flatten() {
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

/// Exactly `GH_TOKEN=<value>`, the value visible ASCII with no quotes: the form the push block's
/// `sed -n s/^GH_TOKEN=//p` reads. The value is never kept or shown.
fn plain_token_line(line: &str) -> bool {
    let value = line.strip_prefix("GH_TOKEN=").unwrap_or_default();
    let plain = |b: u8| b.is_ascii_graphic() && !b"\"'".contains(&b);
    !value.is_empty() && value.bytes().all(plain)
}

/// One equals, is inside or holds the other, or both sit in one folder.
fn tangled(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a) || a.parent() == b.parent()
}

/// `path` with its nearest existing ancestor resolved (links followed) and the rest appended.
fn resolved(path: &Path) -> PathBuf {
    for at in path.ancestors() {
        if let Ok(real) = fs::canonicalize(at) {
            let rest = path.strip_prefix(at).unwrap_or(Path::new(""));
            return real.components().chain(rest.components()).collect();
        }
    }
    path.to_path_buf()
}

/// Whether the root `.gitignore` text ignores the root file `name`: the last match wins, `!`
/// re-includes, a trailing `/` matches folders only, a leading `/` or `**/` anchors, and any other
/// `/` names a path below the root. A class `[..]` or an escape never matches (kept short).
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
        if !pat.contains('/') && glob(pat.as_bytes(), name.as_bytes()) {
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
    let names = "Dropbox|OneDrive|Google Drive|iCloud|/Mobile Documents/|/Library/CloudStorage/";
    let all = real.to_string_lossy();
    names.split('|').any(|n| all.contains(n))
}
