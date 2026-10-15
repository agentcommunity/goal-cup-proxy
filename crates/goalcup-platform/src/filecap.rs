//! The refusal and error types for `readcap`'s handle-rooted file access. `tracecap` and
//! `sourcecap` define their own error types.

use std::fmt;
use std::io;

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
}

impl fmt::Display for FileCapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
