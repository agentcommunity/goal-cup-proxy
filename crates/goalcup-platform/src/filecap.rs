//! Handle-rooted file access for untrusted relative descendant names.
//!
//! Windows is not yet supported (it is planned), so every operation here refuses
//! with `FileCapError::UnsupportedPlatform`. The type surface stays in place because
//! `readcap` and callers outside this crate (`goalcup-daemon`'s `cfg(windows)` recovery
//! stub) name it; there is no native NTFS handle-rooted implementation.

use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io;
use std::path::Path;

/// A reason that a descriptor-relative descendant name was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelativeNameError {
    /// The whole name was empty.
    Empty,
    /// The name began with `/`.
    Rooted,
    /// A repeated or trailing `/` produced an empty component.
    EmptyComponent,
    /// A component was `.`.
    DotComponent,
    /// A component was `..`.
    ParentComponent,
    /// A component contained a Windows path separator.
    Backslash,
    /// A component contained a drive or alternate-stream colon.
    Colon,
    /// A component contained a control character.
    ControlCharacter,
    /// A component contained a Win32-reserved character.
    ReservedCharacter,
    /// A component ended in a dot or space and would alias another Win32 name.
    TrailingDotOrSpace,
    /// A component used a reserved Windows device basename.
    ReservedDeviceName,
}

impl fmt::Display for RelativeNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            Self::Empty => "relative descendant is empty",
            Self::Rooted => "relative descendant is rooted",
            Self::EmptyComponent => "relative descendant has an empty component",
            Self::DotComponent => "relative descendant has a dot component",
            Self::ParentComponent => "relative descendant has a parent component",
            Self::Backslash => "relative descendant contains a backslash",
            Self::Colon => "relative descendant contains a colon",
            Self::ControlCharacter => "relative descendant contains a control character",
            Self::ReservedCharacter => "relative descendant contains a reserved character",
            Self::TrailingDotOrSpace => {
                "relative descendant has a component ending in a dot or space"
            }
            Self::ReservedDeviceName => {
                "relative descendant contains a reserved Windows device basename"
            }
        };
        formatter.write_str(reason)
    }
}

impl std::error::Error for RelativeNameError {}

/// A semantic reason that a selected root or descendant object was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalKind {
    /// The selected root used an unsupported network, device, or drive-relative form.
    UnsupportedRoot,
    /// The selected root is not on the supported filesystem family for this implementation.
    UnsupportedFileSystem,
    /// The acquired object is a reparse point.
    ReparsePoint,
    /// The acquired object is offline or would require recall.
    OfflineOrRecall,
    /// The acquired object is not a directory where one was required.
    NotDirectory,
    /// The acquired leaf is not an ordinary non-directory file.
    NotRegularFile,
    /// The acquired leaf has another hard-link name outside the selected tree.
    MultipleLinks,
    /// The acquired object is not backed by a disk-file handle.
    NotDiskFile,
    /// A validated component cannot be represented by the native API.
    NameTooLong,
}

/// A structured failure to acquire or traverse a contained file capability.
#[derive(Debug)]
pub enum FileCapError {
    /// Native handle-rooted traversal is unavailable on this build target.
    UnsupportedPlatform,
    /// An acquired object failed a security or type requirement.
    Refused {
        /// The operation whose object was refused.
        operation: &'static str,
        /// The semantic refusal reason.
        reason: RefusalKind,
    },
    /// A host API failed before a safe capability could be returned.
    Io {
        /// The host operation that failed.
        operation: &'static str,
        /// The original operating-system error.
        source: io::Error,
    },
}

impl FileCapError {
    pub(crate) fn refused(operation: &'static str, reason: RefusalKind) -> Self {
        Self::Refused { operation, reason }
    }

    pub(crate) fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    /// The underlying host error kind, when acquisition failed in the host API.
    pub fn io_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Io { source, .. } => Some(source.kind()),
            _ => None,
        }
    }
}

impl fmt::Display for FileCapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                formatter.write_str("native contained file access is unsupported")
            }
            Self::Refused { operation, reason } => {
                write!(formatter, "{operation} refused: {reason:?}")
            }
            Self::Io { operation, source } => write!(formatter, "{operation} failed: {source}"),
        }
    }
}

impl std::error::Error for FileCapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// An opaque, validated descriptor-relative descendant name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelativeDescendant {
    components: Vec<OsString>,
}

impl RelativeDescendant {
    /// Validate a portable `/`-separated descendant name.
    pub fn parse(value: &str) -> Result<Self, RelativeNameError> {
        if value.is_empty() {
            return Err(RelativeNameError::Empty);
        }
        if value.starts_with('/') {
            return Err(RelativeNameError::Rooted);
        }

        let mut components = Vec::new();
        for component in value.split('/') {
            validate_component(component)?;
            components.push(OsString::from(component));
        }
        Ok(Self { components })
    }
}

/// An owning handle to a selected directory root.
pub struct PinnedDir {
    _private: (),
}

impl fmt::Debug for PinnedDir {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("PinnedDir").finish_non_exhaustive()
    }
}

impl PinnedDir {
    /// Open one selected root without following a final reparse point.
    ///
    /// Windows is not yet supported (it is planned): every acquisition refuses.
    pub fn open_selected(root: impl AsRef<Path>) -> Result<Self, FileCapError> {
        let _ = root;
        Err(FileCapError::UnsupportedPlatform)
    }

    /// Acquire an ordinary disk-backed file beneath this pinned root.
    ///
    /// Windows is not yet supported (it is planned): every acquisition refuses.
    pub fn read_regular(&self, relative: &RelativeDescendant) -> Result<File, FileCapError> {
        let _ = relative;
        Err(FileCapError::UnsupportedPlatform)
    }

    /// Acquire an ordinary disk-backed file beneath this pinned root, creating only its
    /// final component when absent.
    ///
    /// Windows is not yet supported (it is planned): every acquisition refuses.
    pub fn open_or_create_regular(
        &self,
        relative: &RelativeDescendant,
    ) -> Result<File, FileCapError> {
        let _ = relative;
        Err(FileCapError::UnsupportedPlatform)
    }
}

fn validate_component(component: &str) -> Result<(), RelativeNameError> {
    if component.is_empty() {
        return Err(RelativeNameError::EmptyComponent);
    }
    if component == "." {
        return Err(RelativeNameError::DotComponent);
    }
    if component == ".." {
        return Err(RelativeNameError::ParentComponent);
    }
    if component.ends_with(['.', ' ']) {
        return Err(RelativeNameError::TrailingDotOrSpace);
    }
    for character in component.chars() {
        if character.is_control() {
            return Err(RelativeNameError::ControlCharacter);
        }
        match character {
            '\\' => return Err(RelativeNameError::Backslash),
            ':' => return Err(RelativeNameError::Colon),
            '<' | '>' | '"' | '|' | '?' | '*' => {
                return Err(RelativeNameError::ReservedCharacter);
            }
            _ => {}
        }
    }
    if is_reserved_device_name(component) {
        return Err(RelativeNameError::ReservedDeviceName);
    }
    Ok(())
}

fn is_reserved_device_name(component: &str) -> bool {
    let basename = component.split('.').next().unwrap_or(component);
    let uppercase = basename.to_ascii_uppercase();
    if matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    ["COM", "LPT"].into_iter().any(|prefix| {
        uppercase.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::RelativeDescendant;

    #[test]
    fn relative_descendants_accept_portable_names_unicode_spaces_and_long_trees() {
        let long_component = "x".repeat(240);
        let cases = [
            "trace.toml".to_owned(),
            "nested/evidence.json".to_owned(),
            "Unicode 🏆/終了.json".to_owned(),
            format!("long/{long_component}/leaf"),
            "COM0/LPT10/ordinary.txt".to_owned(),
        ];

        for value in cases {
            RelativeDescendant::parse(&value)
                .unwrap_or_else(|error| panic!("expected {value:?} to be accepted: {error}"));
        }
    }

    #[test]
    fn relative_descendants_reject_ambiguous_or_unsafe_spellings() {
        let cases = [
            "",
            "/absolute",
            "child/",
            "a//b",
            ".",
            "..",
            "a/./b",
            "a/../b",
            r"a\b",
            r"C:\rooted",
            r"C:drive-relative",
            r"\\server\share\file",
            r"\\?\C:\device",
            r"\\.\PIPE\named",
            "file:stream",
            "nul\0byte",
            "control\u{001f}byte",
            "delete\u{007f}byte",
            "c1\u{0085}byte",
            "less<than",
            "greater>than",
            "double\"quote",
            "vertical|bar",
            "question?mark",
            "asterisk*",
            "trailing.",
            "trailing ",
            "parent/trailing./leaf",
            "CON",
            "con.txt",
            "PRN",
            "AUX.log",
            "NUL",
            "COM1",
            "com9.port",
            "LPT1",
            "lpt9.txt",
            "COM\u{00b9}",
            "LPT\u{00b3}.txt",
        ];

        for value in cases {
            assert!(
                RelativeDescendant::parse(value).is_err(),
                "expected {value:?} to be refused"
            );
        }
    }
}
