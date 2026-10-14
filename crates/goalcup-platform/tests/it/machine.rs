use goalcup_platform::machine::{ac_from_pmset, facts, is_wsl_from, on_windows_drive_with};
use std::path::Path;

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

#[test]
fn only_mnt_letter_paths_are_windows_drives() {
    assert!(on_windows_drive_with(Path::new("/mnt/c/x"), true));
    assert!(on_windows_drive_with(Path::new("/mnt/d"), true));
    assert!(!on_windows_drive_with(Path::new("/mnt/data"), true));
    assert!(!on_windows_drive_with(Path::new("/mnt"), true));
    assert!(!on_windows_drive_with(Path::new("/home/c/x"), true));
    assert!(!on_windows_drive_with(Path::new("/mnt/c/x"), false));
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

/// Every JSON key and string value, lowercased.
fn strings(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::String(s) => out.push(s.to_lowercase()),
        serde_json::Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
        serde_json::Value::Object(o) => {
            for (k, x) in o {
                out.push(k.to_lowercase());
                strings(x, out);
            }
        }
        _ => {}
    }
}

#[test]
fn facts_name_no_host_or_user() {
    let facts = facts(&std::env::temp_dir());
    let json = serde_json::to_value(&facts).unwrap();
    let mut seen = Vec::new();
    strings(&json, &mut seen);
    assert!(!seen.is_empty(), "os and arch are always known");

    let host = host_name();
    let user = std::env::var("USER").ok();
    for secret in [host, user].into_iter().flatten() {
        let secret = secret.to_lowercase();
        // A short value would match unrelated words; whole-string equality stays honest.
        if secret.len() < 3 {
            continue;
        }
        let full = secret.clone();
        let short = full.split('.').next().unwrap_or(&full).to_string();
        assert!(
            !seen.iter().any(|s| *s == full || *s == short),
            "facts leak {secret:?}: {json}"
        );
        if secret.len() >= 6 {
            assert!(
                !seen.iter().any(|s| s.contains(&secret)),
                "facts embed {secret:?}: {json}"
            );
        }
    }
}

#[test]
fn facts_are_present_for_the_basics() {
    let f = facts(&std::env::temp_dir());
    assert_eq!(f.os.as_deref(), Some(std::env::consts::OS));
    assert_eq!(f.arch.as_deref(), Some(std::env::consts::ARCH));
    assert!(f.cpu_count.unwrap_or(0) >= 1);
    assert!(f.disk_total_bytes.unwrap_or(0) > 0);
}
