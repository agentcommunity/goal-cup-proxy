use goalcup_platform::machine::{
    ac_from_pmset, container_from_systemd, facts, is_wsl_from, mountinfo_fstype,
    unsupported_fstype, unsupported_magic, url_allowed, windows_host_filesystem,
    windows_host_filesystem_with, windows_host_fstype, windows_host_magic, wsl_check_with,
    wsl_version_from,
};
use std::path::{Path, PathBuf};

#[test]
fn wsl_detection_reads_osrelease() {
    assert!(is_wsl_from("5.15.167.4-microsoft-standard-WSL2"));
    assert!(is_wsl_from("4.4.0-19041-Microsoft"));
    assert!(!is_wsl_from("6.8.0-45-generic"));
    assert!(!is_wsl_from(""));
}

#[test]
fn ac_power_parses_pmset() {
    assert_eq!(
        ac_from_pmset("Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t100%; charged"),
        Some(true)
    );
    assert_eq!(
        ac_from_pmset("Now drawing from 'Battery Power'\n -InternalBattery-0\t80%; discharging"),
        Some(false)
    );
    assert_eq!(ac_from_pmset(""), None);
    assert_eq!(ac_from_pmset("No batteries available"), None);
}

// Strings as the kernels print them.
const WSL2_RELEASE: &str = "5.15.167.4-microsoft-standard-WSL2\n";
const WSL2_VERSION: &str = "Linux version 5.15.167.4-microsoft-standard-WSL2 (root@f9c826d3017f) \
    (gcc (GCC) 11.2.0, GNU ld (GNU Binutils) 2.37) #1 SMP Thu Jan 11 04:09:03 UTC 2024\n";
const WSL2_EARLY_RELEASE: &str = "4.19.128-microsoft-standard\n";
const WSL1_RELEASE: &str = "4.4.0-19041-Microsoft\n";
const WSL1_VERSION: &str = "Linux version 4.4.0-19041-Microsoft (Microsoft@Microsoft.com) \
    (gcc version 5.4.0 (GCC) ) #1237-Microsoft Sat Sep 11 14:32:00 PST 2021\n";
const NATIVE_RELEASE: &str = "6.8.0-45-generic\n";
const NATIVE_VERSION: &str =
    "Linux version 6.8.0-45-generic (buildd@lcy02) (x86_64-linux-gnu-gcc-13) #45-Ubuntu SMP\n";

#[test]
fn wsl_version_tells_wsl2_from_wsl1() {
    assert_eq!(wsl_version_from(WSL2_RELEASE, WSL2_VERSION), Some(2));
    assert_eq!(wsl_version_from(WSL2_RELEASE, ""), Some(2));
    assert_eq!(wsl_version_from("", WSL2_VERSION), Some(2));
    // The first WSL2 kernels carried no "WSL2" suffix.
    assert_eq!(wsl_version_from(WSL2_EARLY_RELEASE, ""), Some(2));
    assert_eq!(wsl_version_from(WSL1_RELEASE, WSL1_VERSION), Some(1));
    assert_eq!(wsl_version_from(WSL1_RELEASE, ""), Some(1));
    assert_eq!(wsl_version_from("", WSL1_VERSION), Some(1));
    // A custom kernel built for WSL2 under another name is still not WSL1's 4.4.0.
    assert_eq!(wsl_version_from("6.6.36.3-Microsoft-custom", ""), Some(2));
    assert_eq!(wsl_version_from(NATIVE_RELEASE, NATIVE_VERSION), None);
    // A kernel built by someone at microsoft.com is not WSL: only the release is read.
    let builder = "Linux version 6.8.0-45-generic (builder@microsoft.com) (gcc 13.2) #45-Ubuntu\n";
    assert_eq!(wsl_version_from(NATIVE_RELEASE, builder), None);
    assert_eq!(wsl_version_from("", builder), None);
    assert_eq!(wsl_version_from("", "Microsoft"), None);
    assert!(!is_wsl_from("Linux version 6.8.0 (builder@microsoft.com)"));
    assert!(is_wsl_from("5.15.167.4-microsoft-standard-WSL2"));
    assert_eq!(wsl_version_from("", ""), None);
}

#[test]
fn windows_host_filesystems_are_named_by_type_and_by_magic() {
    for name in ["9p", "drvfs", "virtiofs", "cifs", "smb3", "ntfs3", "9P"] {
        assert!(windows_host_fstype(name), "{name}");
    }
    for name in [
        "ext4", "xfs", "btrfs", "overlay", "tmpfs", "zfs", "fuse", "vfat", "",
    ] {
        assert!(!windows_host_fstype(name), "{name}");
    }
    assert_eq!(windows_host_magic(0x0102_1997), Some("9p"));
    assert_eq!(windows_host_magic(0xFF53_4D42), Some("cifs"));
    // ext4, xfs, tmpfs, overlayfs, fuse.
    for magic in [0xEF53, 0x5846_5342, 0x0102_1994, 0x794C_7630, 0x6573_5546] {
        assert_eq!(windows_host_magic(magic), None, "{magic:#x}");
    }
}

const MOUNTINFO: &str = "\
22 28 0:21 / /sys rw,nosuid,nodev,noexec,relatime shared:7 - sysfs sysfs rw
30 1 8:16 / / rw,relatime shared:1 - ext4 /dev/sdb rw,discard,errors=remount-ro,data=ordered
31 30 0:27 / /mnt/wsl rw,relatime shared:2 - tmpfs tmpfs rw
40 30 0:38 / /mnt/c rw,noatime,dirsync shared:3 - 9p C:\\134 rw,dirsync,aname=drvfs;path=C:\\;uid=1000
41 30 0:39 / /mnt/d rw,noatime shared:4 - drvfs D: rw,uid=1000
42 30 0:40 / /mnt/e rw,noatime shared:5 - virtiofs E:\\134 rw
43 30 0:41 / /mnt/my\\040drive rw,noatime shared:6 - 9p F:\\134 rw,aname=drvfs
44 40 0:42 / /mnt/c/linuxdir rw,relatime shared:8 - ext4 /dev/sdc rw
";

#[test]
fn mountinfo_names_the_filesystem_of_the_longest_mount_point() {
    let at = |p: &str| mountinfo_fstype(MOUNTINFO, Path::new(p));
    assert_eq!(at("/home/me/work").as_deref(), Some("ext4"));
    assert_eq!(at("/").as_deref(), Some("ext4"));
    assert_eq!(at("/mnt/c").as_deref(), Some("9p"));
    assert_eq!(at("/mnt/c/Users/me/work").as_deref(), Some("9p"));
    assert_eq!(at("/mnt/d/x").as_deref(), Some("drvfs"));
    assert_eq!(at("/mnt/e/x").as_deref(), Some("virtiofs"));
    // A mount point is a whole path component, not a string prefix.
    assert_eq!(at("/mnt/cache/x").as_deref(), Some("ext4"));
    assert_eq!(at("/mnt/wsl/x").as_deref(), Some("tmpfs"));
    // A Linux mount inside a Windows drive is Linux; an escaped space is read back.
    assert_eq!(at("/mnt/c/linuxdir/work").as_deref(), Some("ext4"));
    assert_eq!(at("/mnt/my drive/x").as_deref(), Some("9p"));
    assert_eq!(mountinfo_fstype("", Path::new("/x")), None);
    assert_eq!(mountinfo_fstype("garbage line\n", Path::new("/x")), None);
}

#[test]
fn the_wsl_checks_refuse_wsl1_and_every_folder_on_a_windows_host_filesystem() {
    let work = Path::new("/mnt/c/Users/me/work");
    let run = Path::new("/home/me/runs");
    let artifacts = Path::new("/mnt/d/artifacts");
    let config = Path::new("/home/me/.config/goalcup");
    let folders = [
        ("work folder", work),
        ("run folder", run),
        ("artifact root", artifacts),
        ("config folder", config),
    ];
    let types = |p: &Path| mountinfo_fstype(MOUNTINFO, p);

    // Not WSL: nothing is looked at, even a folder on /mnt/c.
    assert_eq!(wsl_check_with(None, &folders, &types), Ok(()));
    // WSL1 is refused, with the way out, whatever the folders are.
    let one = wsl_check_with(Some(1), &[("run folder", run)], &types).unwrap_err();
    assert!(
        one.starts_with("WSL1 cannot run Goal Cup") && one.contains("wsl --set-version <distro> 2"),
        "{one}"
    );
    // WSL2: each Windows-host folder is named with its type; the Linux ones are not.
    let two = wsl_check_with(Some(2), &folders, &types).unwrap_err();
    assert!(
        two.contains("work folder /mnt/c/Users/me/work") && two.contains("(9p)"),
        "{two}"
    );
    assert!(
        two.contains("artifact root /mnt/d/artifacts") && two.contains("(drvfs)"),
        "{two}"
    );
    assert!(
        !two.contains("run folder") && !two.contains("config folder"),
        "{two}"
    );
    assert_eq!(
        wsl_check_with(
            Some(2),
            &[("run folder", run), ("config folder", config)],
            &types
        ),
        Ok(())
    );
    // A folder whose filesystem cannot be told is not refused.
    assert_eq!(wsl_check_with(Some(2), &folders, &|_: &Path| None), Ok(()));
}

#[test]
fn the_filesystems_a_run_cannot_keep_records_on_are_named_by_type_and_by_magic() {
    for (name, said) in [
        ("btrfs", "btrfs"),
        ("zfs", "ZFS"),
        ("nfs4", "NFS"),
        ("tmpfs", "tmpfs"),
        ("overlay", "overlayfs"),
        ("fuse.sshfs", "FUSE"),
        ("macfuse", "FUSE"),
    ] {
        assert_eq!(unsupported_fstype(name), Some(said), "{name}");
    }
    for name in ["ext4", "xfs", "apfs", "9p", ""] {
        assert_eq!(unsupported_fstype(name), None, "{name}");
    }
    assert_eq!(unsupported_magic(0x9123_683E), Some("btrfs"));
    assert_eq!(unsupported_magic(0x794C_7630), Some("overlayfs"));
    // ext4 and XFS.
    for magic in [0xEF53, 0x5846_5342] {
        assert_eq!(unsupported_magic(magic), None, "{magic:#x}");
    }
}

#[test]
fn a_folder_on_this_machines_filesystem_is_not_a_windows_host_one() {
    let tmp = std::env::temp_dir();
    assert_eq!(windows_host_filesystem(&tmp), None);
    // A folder not made yet is judged by the nearest one that exists.
    assert_eq!(windows_host_filesystem(&tmp.join("not/made/yet")), None);
    #[cfg(target_os = "linux")]
    {
        // The real mountinfo is readable and names the filesystem holding the temp folder.
        let text = std::fs::read_to_string("/proc/self/mountinfo").unwrap();
        let real = std::fs::canonicalize(&tmp).unwrap();
        let name = mountinfo_fstype(&text, &real).expect("a mount covers the temp folder");
        assert!(!windows_host_fstype(&name), "{name}");
    }
}

fn host_name() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its stated length.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return None;
    }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    Some(String::from_utf8_lossy(&buf[..end]).into_owned())
}

/// Whether `needle` appears in `hay` bounded by non-alphanumeric characters (or the ends).
fn contains_word(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(i, m)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + m.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Every JSON string value (not keys), lowercased.
fn values(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::String(s) => out.push(s.to_lowercase()),
        serde_json::Value::Array(a) => a.iter().for_each(|x| values(x, out)),
        serde_json::Value::Object(o) => o.values().for_each(|x| values(x, out)),
        _ => {}
    }
}

#[test]
fn facts_name_no_host_or_user() {
    let json = serde_json::to_value(facts(&std::env::temp_dir())).unwrap();
    let mut vals = Vec::new();
    values(&json, &mut vals);

    assert!(
        !vals.iter().any(|v| v.contains('/')),
        "a fact holds a path: {json}"
    );
    if let Some(home) = std::env::var("HOME").ok().filter(|h| h.len() >= 3) {
        let home = home.to_lowercase();
        assert!(
            !vals.iter().any(|v| v.contains(&home)),
            "home leaked: {json}"
        );
    }
    // `os` and `os_version` legitimately hold distribution names such as "ubuntu 24.04", which a
    // host or user may share, so only the other fields are checked for identity.
    let mut other = Vec::new();
    if let Some(map) = json.as_object() {
        for (key, v) in map {
            if key != "os" && key != "os_version" {
                values(v, &mut other);
            }
        }
    }
    let host = host_name().map(|h| h.split('.').next().unwrap_or("").to_lowercase());
    let user = std::env::var("USER").ok().map(|u| u.to_lowercase());
    for secret in [host, user].into_iter().flatten().filter(|s| s.len() >= 3) {
        for v in other.iter().filter(|v| v.len() >= 4) {
            assert!(!contains_word(v, &secret), "{secret:?} named in {v:?}");
        }
    }
}

#[test]
fn systemd_container_marker_ignores_wsl() {
    assert!(container_from_systemd("docker\n"));
    assert!(container_from_systemd("podman"));
    assert!(!container_from_systemd("wsl\n"));
    assert!(!container_from_systemd("WSL"));
    assert!(!container_from_systemd(""));
    assert!(!container_from_systemd("\n"));
}

#[test]
fn contains_word_respects_boundaries() {
    assert!(contains_word("macos 15.1 bob", "bob"));
    assert!(contains_word("bob", "bob"));
    assert!(!contains_word("bobcat 1", "bob"));
    assert!(!contains_word("rootfs", "root"));
}

#[test]
fn facts_are_present_for_the_basics() {
    let f = facts(&std::env::temp_dir());
    assert_eq!(f.os.as_deref(), Some(std::env::consts::OS));
    assert_eq!(f.arch.as_deref(), Some(std::env::consts::ARCH));
    assert!(f.cpu_count.unwrap_or(0) >= 1);
    assert!(f.disk_total_bytes.unwrap_or(0) > 0);
}

#[test]
fn only_https_urls_reach_the_opener() {
    assert!(url_allowed("https://example.com/path?a=1&b=2"));
    assert!(!url_allowed("http://example.com"));
    assert!(!url_allowed("http://127.0.0.1:8080/"));
    assert!(!url_allowed("--help"));
    assert!(!url_allowed("-https://x"));
    assert!(!url_allowed("file:///etc/passwd"));
    assert!(!url_allowed("https://"));
    assert!(!url_allowed("https://a b"));
    assert!(!url_allowed("https://a\nb"));
    assert!(!url_allowed(""));
}

/// One mount-table line for a mount of `fstype` at `point`.
fn mount_line(point: &Path, fstype: &str) -> String {
    format!("40 30 0:38 / {} rw - {fstype} src rw\n", point.display())
}

#[test]
fn a_dangling_link_is_followed_to_the_mount_its_target_would_be_on() {
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let top = std::env::temp_dir().join(format!("goalcup-wsl-links-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&top);
    std::fs::create_dir_all(&top).unwrap();
    let scratch = Scratch(top);
    let top = std::fs::canonicalize(&scratch.0).unwrap();
    let real = top.join("drive");
    std::fs::create_dir(&real).unwrap();
    let elsewhere = top.join("home");
    std::fs::create_dir(&elsewhere).unwrap();
    // `link` points at a child of the real folder that does not exist (yet).
    let link = elsewhere.join("link");
    std::os::unix::fs::symlink(real.join("missing"), &link).unwrap();
    // A link to a link, relative, to the same place.
    let relative = elsewhere.join("relative");
    std::os::unix::fs::symlink("link", &relative).unwrap();

    let table = format!(
        "{}{}",
        mount_line(Path::new("/"), "ext4"),
        mount_line(&real, "9p")
    );
    let on = |p: &Path| windows_host_filesystem_with(p, &table);
    assert_eq!(on(&link.join("work/clone")).as_deref(), Some("9p"));
    assert_eq!(on(&relative.join("work")).as_deref(), Some("9p"));
    assert_eq!(on(&real.join("not/made")).as_deref(), Some("9p"));
    // The folder holding the link is not the drive.
    assert_eq!(on(&elsewhere.join("plain/work")), None);
    // A link in a loop ends in the nearest real folder rather than looping.
    let one = elsewhere.join("one");
    let two = elsewhere.join("two");
    std::os::unix::fs::symlink(&two, &one).unwrap();
    std::os::unix::fs::symlink(&one, &two).unwrap();
    assert_eq!(on(&one.join("x")), None);
}

#[test]
fn a_relative_path_is_taken_from_the_current_directory() {
    let here = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
    let table = mount_line(&here, "drvfs");
    assert_eq!(
        windows_host_filesystem_with(Path::new("not/made/yet"), &table).as_deref(),
        Some("drvfs")
    );
    let elsewhere = mount_line(Path::new("/definitely/not/here"), "drvfs");
    assert_eq!(
        windows_host_filesystem_with(Path::new("not/made/yet"), &elsewhere),
        None
    );
}
