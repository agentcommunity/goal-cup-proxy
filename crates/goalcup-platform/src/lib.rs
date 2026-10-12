//! Platform-specific paths for /goal Cup binaries.
//!
//! This crate's native capability layers (`filecap`, `sourcecap`, `tracecap`) are
//! implemented for Unix only. Windows is not yet supported; it is planned (D42). A non-Unix
//! build fails at compile time below rather than shipping a silently unsupported binary, and
//! the workspace carries no non-Unix stub code. Linux is best-effort until it is tested.

#[cfg(not(unix))]
compile_error!(
    "goalcup-platform builds on unix targets only; Windows is not yet supported (it is planned)"
);

/// Contained file access rooted in an owned directory capability.
pub mod filecap;
pub mod handle_budget;
pub mod readcap;
pub mod sourcecap;
pub mod tracecap;

use std::ffi::OsString;
use std::path::PathBuf;

/// The path convention used by the current operating system.
///
/// This is public so resolver tests can exercise every policy branch on one host. It proves
/// path selection only; it does not claim the host's native filesystem or security behaviour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    Windows,
    Macos,
    Unix,
}

/// Resolve the tracker's configuration directory without creating it.
///
/// A non-empty `GOALCUP_CONFIG_DIR` always wins. The supplied environment values are kept as
/// `OsString`s, so an override is returned without a UTF-8 conversion or canonicalisation.
pub fn config_dir_with(
    platform: Platform,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let var = |name: &str| lookup(name).filter(|value| !value.is_empty());
    if let Some(dir) = var("GOALCUP_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    match platform {
        Platform::Windows => match var("LOCALAPPDATA") {
            Some(local) => Some(PathBuf::from(local).join("goalcup")),
            None => var("USERPROFILE").map(|profile| {
                PathBuf::from(profile)
                    .join("AppData")
                    .join("Local")
                    .join("goalcup")
            }),
        },
        Platform::Macos => var("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library/Application Support")
                .join("goalcup")
        }),
        Platform::Unix => match var("XDG_CONFIG_HOME") {
            Some(xdg) => Some(PathBuf::from(xdg).join("goalcup")),
            None => var("HOME").map(|home| PathBuf::from(home).join(".config").join("goalcup")),
        },
    }
}

/// Resolve the current platform's configuration directory without creating it.
pub fn config_dir() -> Option<PathBuf> {
    config_dir_with(current_platform(), |name| std::env::var_os(name))
}

fn current_platform() -> Platform {
    #[cfg(target_os = "macos")]
    {
        Platform::Macos
    }
    #[cfg(not(target_os = "macos"))]
    {
        Platform::Unix
    }
}

#[cfg(test)]
mod tests {
    use super::{Platform, config_dir_with};
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let table: Vec<(String, OsString)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), OsString::from(*value)))
            .collect();
        move |name| {
            table
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn an_override_wins_on_every_platform_and_keeps_unicode_spaces_and_a_long_root() {
        let override_dir = format!("/private/Goal Cup 🏆/{}", "long-root-".repeat(24));
        for platform in [Platform::Windows, Platform::Macos, Platform::Unix] {
            assert_eq!(
                config_dir_with(
                    platform,
                    env(&[
                        ("GOALCUP_CONFIG_DIR", &override_dir),
                        ("LOCALAPPDATA", "C:\\Users\\P\\AppData\\Local"),
                        ("USERPROFILE", "C:\\Users\\P"),
                        ("HOME", "/home/p"),
                        ("XDG_CONFIG_HOME", "/home/p/xdg"),
                    ]),
                ),
                Some(PathBuf::from(&override_dir)),
                "the supplied override is returned verbatim for {platform:?}"
            );
        }
    }

    #[test]
    fn macos_uses_home_and_ignores_xdg() {
        assert_eq!(
            config_dir_with(
                Platform::Macos,
                env(&[("HOME", "/Users/Goal Cup"), ("XDG_CONFIG_HOME", "/tmp/xdg")]),
            ),
            Some(PathBuf::from(
                "/Users/Goal Cup/Library/Application Support/goalcup"
            ))
        );
    }

    #[test]
    fn unix_uses_xdg_without_requiring_home() {
        assert_eq!(
            config_dir_with(
                Platform::Unix,
                env(&[("XDG_CONFIG_HOME", "/var/lib/goal cup config")]),
            ),
            Some(PathBuf::from("/var/lib/goal cup config/goalcup"))
        );
    }

    #[test]
    fn unix_falls_back_to_dot_config_when_xdg_is_empty() {
        assert_eq!(
            config_dir_with(
                Platform::Unix,
                env(&[("HOME", "/home/p"), ("XDG_CONFIG_HOME", "")]),
            ),
            Some(PathBuf::from("/home/p/.config/goalcup"))
        );
    }

    #[test]
    fn windows_uses_local_app_data_and_ignores_home_and_xdg() {
        let local = r"C:\Users\Goal Cup\AppData\Local";
        assert_eq!(
            config_dir_with(
                Platform::Windows,
                env(&[
                    ("LOCALAPPDATA", local),
                    ("USERPROFILE", r"C:\Users\fallback"),
                    ("HOME", "/home/not-windows"),
                    ("XDG_CONFIG_HOME", "/tmp/not-windows"),
                ]),
            ),
            Some(PathBuf::from(local).join("goalcup"))
        );
    }

    #[test]
    fn windows_uses_local_app_data_without_any_other_root() {
        let local = r"C:\Users\Goal Cup\AppData\Local";
        assert_eq!(
            config_dir_with(Platform::Windows, env(&[("LOCALAPPDATA", local)])),
            Some(PathBuf::from(local).join("goalcup"))
        );
    }

    #[test]
    fn windows_falls_back_to_user_profile_when_local_app_data_is_empty() {
        let profile = r"C:\Users\Goal Cup";
        assert_eq!(
            config_dir_with(
                Platform::Windows,
                env(&[("LOCALAPPDATA", ""), ("USERPROFILE", profile)]),
            ),
            Some(
                PathBuf::from(profile)
                    .join("AppData")
                    .join("Local")
                    .join("goalcup")
            )
        );
    }

    #[test]
    fn missing_or_empty_inputs_do_not_resolve_a_directory() {
        for platform in [Platform::Windows, Platform::Macos, Platform::Unix] {
            assert_eq!(
                config_dir_with(
                    platform,
                    env(&[
                        ("GOALCUP_CONFIG_DIR", ""),
                        ("LOCALAPPDATA", ""),
                        ("USERPROFILE", ""),
                        ("HOME", ""),
                        ("XDG_CONFIG_HOME", ""),
                    ]),
                ),
                None,
                "empty inputs are absent for {platform:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_override_keeps_non_utf8_path_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let override_dir = OsString::from_vec(b"/tmp/goalcup-\xff".to_vec());
        assert_eq!(
            config_dir_with(Platform::Unix, |name| {
                (name == "GOALCUP_CONFIG_DIR").then(|| override_dir.clone())
            }),
            Some(PathBuf::from(override_dir))
        );
    }
}
