//! Bounded, handle-rooted, no-follow source reads.

use crate::handle_budget::{HandleBudgetError, NativeHandleBudgetV1};
use std::cell::Cell;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

#[cfg(unix)]
#[path = "sourcecap/unix.rs"]
mod imp;

/// Largest buffer passed to a single native source read.
pub(crate) const MAX_SOURCE_IO_CHUNK_V1: usize = 1 << 20;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceIdentity(imp::Identity);

impl SourceIdentity {
    /// A versioned native identity encoding suitable for a durable source-root journal.
    /// This is deliberately distinct from `Debug`, whose fields are not a persistence contract.
    pub fn journal_token(&self) -> String {
        imp::journal_token(&self.0)
    }
}

/// Complete native metadata retained by the snapshot writer before and after each copy.
/// The identity string is versioned for persistence; all other fields are comparison facts,
/// not portable product metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFactsV1 {
    pub identity: String,
    pub kind: SourceKind,
    pub mode: Option<u32>,
    pub length: u64,
    pub posix_owner_executable: Option<bool>,
    pub modified_seconds: i64,
    pub modified_nanos: i64,
    pub changed_seconds: i64,
    pub changed_nanos: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceName(OsString);

impl SourceName {
    /// Construct one fixed UTF-8 child component.
    pub fn from_component(value: &str) -> Result<Self, SourceNameError> {
        if value.is_empty() {
            return Err(SourceNameError::Empty);
        }
        if value == "." {
            return Err(SourceNameError::Dot);
        }
        if value == ".." {
            return Err(SourceNameError::Parent);
        }
        if value.contains(['/', '\0']) {
            return Err(SourceNameError::SeparatorOrNul);
        }
        Ok(Self(OsString::from(value)))
    }

    pub fn as_os_str(&self) -> &OsStr {
        &self.0
    }

    fn from_native(value: OsString) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceNameError {
    Empty,
    Dot,
    Parent,
    SeparatorOrNul,
    NativeAlias,
}

impl fmt::Display for SourceNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "source name is empty",
            Self::Dot => "source name is dot",
            Self::Parent => "source name is parent",
            Self::SeparatorOrNul => "source name contains a separator or NUL",
            Self::NativeAlias => "source name has an unsupported native alias",
        })
    }
}
impl std::error::Error for SourceNameError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    Directory,
    Regular,
    SymbolicLink,
    Special,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceObservation {
    parent: imp::Identity,
    name: SourceName,
    facts: imp::Facts,
    mount: imp::MountIdentity,
}

impl SourceObservation {
    pub fn kind(&self) -> SourceKind {
        self.facts.kind
    }
    pub fn length(&self) -> u64 {
        self.facts.length
    }
    pub fn posix_owner_executable(&self) -> Option<bool> {
        self.facts.posix_owner_executable
    }

    /// The complete no-follow facts observed for this direct child.
    pub fn facts(&self) -> SourceFactsV1 {
        imp::snapshot_facts(&self.facts)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceLookup {
    Missing,
    Present(SourceObservation),
}

pub struct SourceDir {
    inner: imp::DirHandle,
    identity: SourceIdentity,
    root_mount: imp::MountIdentity,
    budget: NativeHandleBudgetV1,
    cursor_active: Cell<bool>,
}

impl fmt::Debug for SourceDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceDir").finish_non_exhaustive()
    }
}

impl SourceDir {
    pub fn open_selected(
        path: impl AsRef<Path>,
        budget: &NativeHandleBudgetV1,
    ) -> Result<Self, SourceCapError> {
        let permit = budget.reserve_scratch()?;
        let (inner, root_mount) = imp::DirHandle::open_selected(path.as_ref(), permit)?;
        let identity = SourceIdentity(inner.identity().clone());
        Ok(Self {
            inner,
            identity,
            root_mount,
            budget: budget.clone(),
            cursor_active: Cell::new(false),
        })
    }

    pub fn identity(&self) -> &SourceIdentity {
        &self.identity
    }

    /// Current facts for this retained directory capability. Comparing this value before and
    /// after a bounded enumeration detects namespace races that a stable inode alone misses.
    pub fn facts(&self) -> Result<SourceFactsV1, SourceCapError> {
        self.inner.facts().map(|facts| imp::snapshot_facts(&facts))
    }

    pub fn observe(&self, name: &SourceName) -> Result<SourceLookup, SourceCapError> {
        let Some(facts) = self.inner.observe(name.as_os_str(), &self.budget)? else {
            return Ok(SourceLookup::Missing);
        };
        Ok(SourceLookup::Present(SourceObservation {
            parent: self.inner.identity().clone(),
            name: name.clone(),
            facts,
            mount: self.root_mount.clone(),
        }))
    }

    pub fn entries(&self) -> Result<SourceEntries<'_>, SourceCapError> {
        if self.cursor_active.replace(true) {
            return Err(SourceCapError::refused(
                "open source enumeration",
                RefusalKind::CursorActive,
            ));
        }
        let result = self
            .budget
            .reserve_scratch()
            .map_err(SourceCapError::from)
            .and_then(|permit| self.inner.entries(permit));
        match result {
            Ok(inner) => Ok(SourceEntries {
                owner: self,
                inner: Some(inner),
                poisoned: false,
            }),
            Err(error) => {
                self.cursor_active.set(false);
                Err(error)
            }
        }
    }

    pub fn open_dir_matching(
        &self,
        name: &SourceName,
        expected: &SourceObservation,
    ) -> Result<Self, SourceCapError> {
        self.check_binding(
            name,
            expected,
            SourceKind::Directory,
            "open matching source directory",
        )?;
        let permit = self.budget.reserve_scratch()?;
        let inner = self
            .inner
            .open_dir(name.as_os_str(), &self.root_mount, permit)?;
        let facts = inner.facts()?;
        if facts != expected.facts {
            return Err(SourceCapError::refused(
                "open matching source directory",
                RefusalKind::MetadataChanged,
            ));
        }
        let identity = SourceIdentity(inner.identity().clone());
        Ok(Self {
            inner,
            identity,
            root_mount: self.root_mount.clone(),
            budget: self.budget.clone(),
            cursor_active: Cell::new(false),
        })
    }

    pub fn open_regular_matching(
        &self,
        name: &SourceName,
        expected: &SourceObservation,
        max_len: u64,
    ) -> Result<SourceFile, SourceCapError> {
        self.check_binding(
            name,
            expected,
            SourceKind::Regular,
            "open matching regular source",
        )?;
        if expected.facts.length > max_len {
            return Err(SourceCapError::refused(
                "open matching regular source",
                RefusalKind::LengthLimit,
            ));
        }
        let permit = self.budget.reserve_scratch()?;
        let inner = self
            .inner
            .open_regular(name.as_os_str(), &self.root_mount, permit)?;
        if inner.facts()? != expected.facts {
            return Err(SourceCapError::refused(
                "open matching regular source",
                RefusalKind::MetadataChanged,
            ));
        }
        Ok(SourceFile {
            inner,
            opening: expected.clone(),
            position: 0,
            poisoned: false,
        })
    }

    pub fn read_symlink_into_matching(
        &self,
        name: &SourceName,
        expected: &SourceObservation,
        destination: &mut [u8],
    ) -> Result<usize, SourceCapError> {
        self.check_binding(
            name,
            expected,
            SourceKind::SymbolicLink,
            "read matching symbolic link",
        )?;
        if destination.is_empty() {
            return Err(SourceCapError::refused(
                "read matching symbolic link",
                RefusalKind::BufferLimit,
            ));
        }
        let permit = self.budget.reserve_scratch()?;
        self.inner.read_symlink(
            name.as_os_str(),
            &self.root_mount,
            &expected.facts,
            destination,
            permit,
        )
    }

    fn check_binding(
        &self,
        name: &SourceName,
        expected: &SourceObservation,
        kind: SourceKind,
        operation: &'static str,
    ) -> Result<(), SourceCapError> {
        if expected.parent != *self.inner.identity()
            || expected.name != *name
            || expected.mount != self.root_mount
        {
            return Err(SourceCapError::refused(
                operation,
                RefusalKind::IdentityChanged,
            ));
        }
        if expected.facts.kind != kind {
            return Err(SourceCapError::refused(operation, RefusalKind::WrongKind));
        }
        Ok(())
    }
}

pub struct SourceEntries<'a> {
    owner: &'a SourceDir,
    inner: Option<imp::Entries>,
    poisoned: bool,
}

impl SourceEntries<'_> {
    pub fn next_entry(&mut self) -> Result<Option<SourceName>, SourceCapError> {
        if self.poisoned {
            return Err(SourceCapError::refused(
                "enumerate source directory",
                RefusalKind::PoisonedCursor,
            ));
        }
        let result = self
            .inner
            .as_mut()
            .expect("cursor owns enumeration")
            .next(self.owner.inner.identity(), &self.owner.root_mount);
        match result {
            Ok(Some(name)) => Ok(Some(SourceName::from_native(name))),
            Ok(None) => Ok(None),
            Err(error) => {
                self.poisoned = true;
                Err(error)
            }
        }
    }
}

impl Drop for SourceEntries<'_> {
    fn drop(&mut self) {
        self.inner.take();
        self.owner.cursor_active.set(false);
    }
}

pub struct SourceFile {
    inner: imp::FileHandle,
    opening: SourceObservation,
    position: u64,
    poisoned: bool,
}

impl SourceFile {
    pub fn observation(&self) -> &SourceObservation {
        &self.opening
    }
    pub fn finish(mut self) -> Result<SourceObservation, SourceCapError> {
        if self.poisoned {
            return Err(SourceCapError::refused(
                "finish source read",
                RefusalKind::PoisonedStream,
            ));
        }
        if self.inner.facts()? != self.opening.facts {
            self.poisoned = true;
            return Err(SourceCapError::refused(
                "finish source read",
                RefusalKind::MetadataChanged,
            ));
        }
        Ok(self.opening.clone())
    }
}

impl Read for SourceFile {
    fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
        if self.poisoned {
            return Err(io::Error::other("source stream is poisoned"));
        }
        let remaining = self.opening.facts.length.saturating_sub(self.position);
        let allowed = destination
            .len()
            .min(MAX_SOURCE_IO_CHUNK_V1)
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        if allowed == 0 {
            return Ok(0);
        }
        match self.inner.read(&mut destination[..allowed]) {
            Ok(count) => {
                self.position = self.position.saturating_add(count as u64);
                Ok(count)
            }
            Err(error) => {
                self.poisoned = true;
                Err(error)
            }
        }
    }
}

impl Seek for SourceFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        if self.poisoned {
            return Err(io::Error::other("source stream is poisoned"));
        }
        let candidate = match position {
            SeekFrom::Start(value) => i128::from(value),
            SeekFrom::End(value) => i128::from(self.opening.facts.length) + i128::from(value),
            SeekFrom::Current(value) => i128::from(self.position) + i128::from(value),
        };
        if candidate < 0 || candidate > i128::from(self.opening.facts.length) {
            self.poisoned = true;
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source seek outside opening length",
            ));
        }
        match self.inner.seek(SeekFrom::Start(candidate as u64)) {
            Ok(actual) if actual == candidate as u64 => {
                self.position = actual;
                Ok(actual)
            }
            Ok(_) => {
                self.poisoned = true;
                Err(io::Error::other("source seek returned a different offset"))
            }
            Err(error) => {
                self.poisoned = true;
                Err(error)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalKind {
    UnsupportedRoot,
    UnsupportedFileSystem,
    UnsupportedMountBinding,
    UnsupportedSymlinkPayload,
    CrossMount,
    WrongKind,
    SpecialFile,
    ReparsePoint,
    OfflineOrRecall,
    IdentityChanged,
    MetadataChanged,
    LengthLimit,
    BufferLimit,
    CursorActive,
    PoisonedCursor,
    PoisonedStream,
}

#[derive(Debug)]
pub enum SourceCapError {
    Budget(HandleBudgetError),
    Refused {
        operation: &'static str,
        reason: RefusalKind,
    },
    Io {
        operation: &'static str,
        source: io::Error,
    },
}

impl SourceCapError {
    pub(crate) fn refused(operation: &'static str, reason: RefusalKind) -> Self {
        Self::Refused { operation, reason }
    }
    pub(crate) fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }
}
impl From<HandleBudgetError> for SourceCapError {
    fn from(value: HandleBudgetError) -> Self {
        Self::Budget(value)
    }
}
impl fmt::Display for SourceCapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Budget(error) => error.fmt(f),
            Self::Refused { operation, reason } => write!(f, "{operation} refused: {reason:?}"),
            Self::Io { operation, source } => write!(f, "{operation} failed: {source}"),
        }
    }
}
impl std::error::Error for SourceCapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Budget(e) => Some(e),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "goalcup-sourcecap-unit-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn observed(root: &SourceDir, name: &SourceName) -> SourceObservation {
        match root.observe(name).unwrap() {
            SourceLookup::Present(value) => value,
            SourceLookup::Missing => panic!(),
        }
    }

    fn assert_cross_mount(error: SourceCapError) {
        assert!(matches!(
            error,
            SourceCapError::Refused {
                reason: RefusalKind::CrossMount,
                ..
            }
        ));
    }

    #[test]
    fn native_enumeration_error_poisons_cursor() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("entry"), b"x").unwrap();
        let root = SourceDir::open_selected(&fixture.0, &NativeHandleBudgetV1::new()).unwrap();
        let mut entries = root.entries().unwrap();
        imp::force_enumeration_error_once();
        assert!(matches!(
            entries.next_entry(),
            Err(SourceCapError::Io { .. })
        ));
        assert!(matches!(
            entries.next_entry(),
            Err(SourceCapError::Refused {
                reason: RefusalKind::PoisonedCursor,
                ..
            })
        ));
    }

    #[test]
    fn mount_mismatch_refuses_directory_regular_and_symlink_acquisition() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join("dir")).unwrap();
        fs::write(fixture.0.join("file"), b"x").unwrap();
        symlink("target", fixture.0.join("link")).unwrap();
        let root = SourceDir::open_selected(&fixture.0, &NativeHandleBudgetV1::new()).unwrap();

        let dir = SourceName::from_component("dir").unwrap();
        imp::force_cross_mount_once();
        match root.open_dir_matching(&dir, &observed(&root, &dir)) {
            Err(error) => assert_cross_mount(error),
            Ok(_) => panic!("cross-mount directory was accepted"),
        }

        let file = SourceName::from_component("file").unwrap();
        imp::force_cross_mount_once();
        match root.open_regular_matching(&file, &observed(&root, &file), 1) {
            Err(error) => assert_cross_mount(error),
            Ok(_) => panic!("cross-mount regular file was accepted"),
        }

        let link = SourceName::from_component("link").unwrap();
        imp::force_cross_mount_once();
        let mut destination = [0u8; 64];
        assert_cross_mount(
            root.read_symlink_into_matching(&link, &observed(&root, &link), &mut destination)
                .unwrap_err(),
        );
    }
}
