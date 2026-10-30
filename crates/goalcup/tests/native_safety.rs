//! PRD-04 acceptance `native_safety.rs`, the fsx cases (points 3 and 4): no symlink is followed,
//! a FIFO is never read and never hangs the reader, a replaced file is refused, and no
//! descriptor is kept. The walk-level cases join this file with `walk.rs`.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Read;
use std::os::unix::fs::symlink;
use std::sync::mpsc;
use std::time::Duration;

use goalcup::native::fsx::{Dir, Kind, file_stat};

fn n(s: &str) -> &OsStr {
    OsStr::new(s)
}

#[test]
fn a_symlink_is_never_followed() {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("real.jsonl"), b"secret-free\n").unwrap();
    fs::create_dir(t.path().join("real_dir")).unwrap();
    symlink(t.path().join("real.jsonl"), t.path().join("link.jsonl")).unwrap();
    symlink(t.path().join("real_dir"), t.path().join("link_dir")).unwrap();

    let root = Dir::open_root(t.path()).unwrap();
    let link = root.stat_at(n("link.jsonl")).unwrap();
    assert_eq!(link.kind, Kind::Symlink);
    assert!(root.open_file(n("link.jsonl"), link.fid()).is_err());
    let target = root.stat_at(n("real.jsonl")).unwrap();
    assert!(root.open_file(n("link.jsonl"), target.fid()).is_err());

    assert_eq!(root.stat_at(n("link_dir")).unwrap().kind, Kind::Symlink);
    assert!(root.open_child(n("link_dir")).is_err());
    assert!(root.open_child(n("real_dir")).is_ok());

    let linked_root = t.path().join("linked_root");
    symlink(t.path().join("real_dir"), &linked_root).unwrap();
    assert!(Dir::open_root(&linked_root).is_err());
}

#[test]
fn a_fifo_is_never_read_and_never_hangs() {
    let t = tempfile::tempdir().unwrap();
    let fifo = t.path().join("session.jsonl");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: `c` is a live NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);

    let root = Dir::open_root(t.path()).unwrap();
    let st = root.stat_at(n("session.jsonl")).unwrap();
    assert_eq!(st.kind, Kind::Other);

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let r = root.open_file(n("session.jsonl"), st.fid());
        let _ = tx.send(r.is_err());
    });
    let refused = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("opening a FIFO must not hang");
    assert!(refused);
}

#[test]
fn a_file_replaced_after_its_stat_is_refused() {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("s.jsonl"), b"one\n").unwrap();
    let root = Dir::open_root(t.path()).unwrap();
    let seen = root.stat_at(n("s.jsonl")).unwrap();

    fs::write(t.path().join("s.jsonl.tmp"), b"two\n").unwrap();
    fs::rename(t.path().join("s.jsonl.tmp"), t.path().join("s.jsonl")).unwrap();

    assert!(root.open_file(n("s.jsonl"), seen.fid()).is_err());
    let now = root.stat_at(n("s.jsonl")).unwrap();
    assert_ne!(now.fid(), seen.fid());
    assert!(root.open_file(n("s.jsonl"), now.fid()).is_ok());
}

#[test]
fn names_list_files_and_folders_and_a_regular_file_reads() {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("a.jsonl"), b"{\"x\":1}\n\xff\x00tail").unwrap();
    fs::create_dir(t.path().join("sub")).unwrap();
    fs::write(t.path().join("sub").join("b.jsonl"), b"b").unwrap();

    let root = Dir::open_root(t.path()).unwrap();
    let mut names = root.names().unwrap();
    names.sort();
    assert_eq!(names, [OsString::from("a.jsonl"), OsString::from("sub")]);
    // A second listing on the same folder starts from the top again.
    assert_eq!(root.names().unwrap().len(), 2);

    let st = root.stat_at(n("a.jsonl")).unwrap();
    assert_eq!(st.kind, Kind::File);
    assert_eq!(root.stat_at(n("sub")).unwrap().kind, Kind::Dir);
    let mut f = root.open_file(n("a.jsonl"), st.fid()).unwrap();
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, fs::read(t.path().join("a.jsonl")).unwrap());
    let again = file_stat(&f).unwrap();
    assert_eq!((again.fid(), again.size), (st.fid(), bytes.len() as u64));

    let sub = root.open_child(n("sub")).unwrap();
    assert_eq!(sub.stat().unwrap().dev, root.stat().unwrap().dev);
    assert_eq!(sub.names().unwrap(), [OsString::from("b.jsonl")]);
}

/// Descriptors this process holds on paths under `dir` (other tests may run in parallel).
#[cfg(target_os = "linux")]
fn fds_under(dir: &std::path::Path) -> usize {
    fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|e| fs::read_link(e.ok()?.path()).ok())
        .filter(|p| p.starts_with(dir))
        .count()
}

#[cfg(target_os = "linux")]
#[test]
fn no_descriptor_is_kept() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().canonicalize().unwrap();
    fs::create_dir(dir.join("d")).unwrap();
    fs::write(dir.join("d").join("f.jsonl"), b"x").unwrap();
    let before = fds_under(&dir);
    for _ in 0..50 {
        let root = Dir::open_root(&dir).unwrap();
        let d = root.open_child(n("d")).unwrap();
        for name in d.names().unwrap() {
            let st = d.stat_at(&name).unwrap();
            let f = d.open_file(&name, st.fid()).unwrap();
            file_stat(&f).unwrap();
        }
        let _ = d.open_file(n("f.jsonl"), root.stat().unwrap().fid());
        let _ = root.open_child(n("missing"));
    }
    assert_eq!(fds_under(&dir), before);
}
