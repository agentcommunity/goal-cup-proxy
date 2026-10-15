//! Handle-rooted access to the fixed files owned by one trace export.
//!
//! The public surface deliberately has no arbitrary child-name writer. A [`TraceDir`]
//! is an owning directory capability, and the only write operations name the chain's part
//! files, the acknowledgement stream, the four once-only documents, or the descriptor
//! replacement.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
#[path = "tracecap/unix.rs"]
mod imp;

const MAX_ENTRIES: usize = 4096;
const MAX_NAME_UNITS: usize = 1_048_576;
const MAX_CLEANUP_ENTRIES: usize = 65_536;
const MAX_CLEANUP_DEPTH: usize = 256;

/// A fixed, once-only document owned by a trace export.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedDocument {
    Receipt,
    Baseline,
    Seal,
    Manifest,
}

impl FixedDocument {
    fn name(self) -> &'static str {
        match self {
            Self::Receipt => "receipt.json",
            Self::Baseline => "baseline.json",
            Self::Seal => "seal.json",
            Self::Manifest => "manifest.toml",
        }
    }
}

/// The semantic reason a trace capability operation was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalKind {
    UnsupportedRoot,
    ReparsePoint,
    OfflineOrRecall,
    NotDirectory,
    NotRegularFile,
    NotDiskFile,
    MultipleLinks,
    IdentityChanged,
    EntryAlreadyExists,
    DirectoryEntryLimit,
    DirectoryNameLimit,
    NameTooLong,
}

/// A failure to acquire or use a handle-rooted trace object.
#[derive(Debug)]
pub enum TraceCapError {
    Refused {
        operation: &'static str,
        reason: RefusalKind,
    },
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Poisoned {
        stream: &'static str,
    },
}

impl TraceCapError {
    pub(crate) fn refused(operation: &'static str, reason: RefusalKind) -> Self {
        Self::Refused { operation, reason }
    }

    pub(crate) fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }
}

impl fmt::Display for TraceCapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused { operation, reason } => {
                write!(formatter, "{operation} refused: {reason:?}")
            }
            Self::Io { operation, source } => write!(formatter, "{operation} failed: {source}"),
            Self::Poisoned { stream } => write!(formatter, "{stream} writer is poisoned"),
        }
    }
}

impl std::error::Error for TraceCapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// An owning capability to one trace directory.
pub struct TraceDir {
    inner: imp::TraceDirHandle,
}

impl fmt::Debug for TraceDir {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("TraceDir").finish_non_exhaustive()
    }
}

impl TraceDir {
    /// Create missing path components, then acquire the final directory without
    /// following its final link/reparse point.
    pub fn create(root: &Path) -> Result<Self, TraceCapError> {
        std::fs::create_dir_all(root)
            .map_err(|source| TraceCapError::io("create trace root", source))?;
        Self::open(root)
    }

    /// Acquire an existing trace directory.
    pub fn open(root: &Path) -> Result<Self, TraceCapError> {
        imp::TraceDirHandle::open(root).map(|inner| Self { inner })
    }

    /// Descend one already-existing, validated directory component through this retained
    /// directory capability.  This is intentionally not a pathname join: callers that
    /// own an artifact tree must keep the root they acquired while crossing `artifacts`,
    /// an entry id, and finally `trace`.
    pub fn open_child_dir(&self, name: &str) -> Result<Self, TraceCapError> {
        validate_component(name)?;
        self.inner.open_child_dir(name).map(|inner| Self { inner })
    }

    /// Create one absent directory component beneath this retained capability and return
    /// its newly acquired no-follow handle.  Existing components are a refusal: callers
    /// that mean restart must use [`Self::open_child_dir`] explicitly.
    pub fn create_child_dir(&self, name: &str) -> Result<Self, TraceCapError> {
        validate_component(name)?;
        self.inner
            .create_child_dir(name)
            .map(|inner| Self { inner })
    }

    /// Open a retained child when it already exists, or create that one component and
    /// acquire it through the same parent capability.  A raced replacement is still opened
    /// with no-follow semantics; it can make construction fail but cannot redirect it.
    pub fn open_or_create_child_dir(&self, name: &str) -> Result<Self, TraceCapError> {
        validate_component(name)?;
        match self.create_child_dir(name) {
            Ok(child) => Ok(child),
            Err(TraceCapError::Refused {
                reason: RefusalKind::EntryAlreadyExists,
                ..
            }) => self.open_child_dir(name),
            Err(error) => Err(error),
        }
    }

    /// Open (or exclusively create) a regular child while retaining this parent authority.
    pub(crate) fn open_or_create_regular(&self, name: &str) -> Result<File, TraceCapError> {
        validate_component(name)?;
        self.inner.open_or_create_regular(name)
    }

    /// Flush this retained directory's metadata after a child create or replacement.
    pub fn sync(&self) -> Result<(), TraceCapError> {
        self.inner.sync()
    }

    /// Enumerate direct child names through the retained directory capability. This never
    /// follows an entry and refuses non-UTF-8 names because immutable submission layout names
    /// are portable UTF-8 components.
    pub fn child_names(&self, limit: u64) -> Result<Vec<String>, TraceCapError> {
        let mut names = Vec::new();
        self.inner.for_each_name(|name| {
            let name = name.to_str().ok_or_else(|| {
                TraceCapError::refused("enumerate trace child names", RefusalKind::NameTooLong)
            })?;
            validate_component(name)?;
            if u64::try_from(names.len())
                .ok()
                .is_none_or(|count| count >= limit)
            {
                return Err(TraceCapError::refused(
                    "enumerate trace child names",
                    RefusalKind::DirectoryEntryLimit,
                ));
            }
            names.push(name.to_string());
            Ok(())
        })?;
        names.sort();
        Ok(names)
    }

    /// Read a regular child through the retained directory handle.
    pub fn read_regular(&self, name: &str) -> Result<Option<File>, TraceCapError> {
        validate_component(name)?;
        self.inner.read_regular(name)
    }

    /// A stable identity for the acquired directory capability.  It is intentionally
    /// opaque outside the capability layer: callers may bind a durable operation to the
    /// root they acquired, but cannot manufacture an identity from a path string.
    pub(crate) fn identity(&self) -> Result<String, TraceCapError> {
        self.inner.identity()
    }

    fn restrict(&self, mode: u32) -> Result<(), TraceCapError> {
        self.inner.restrict(mode)
    }

    fn remove_contents(&self) -> Result<(), TraceCapError> {
        let mut budget = CleanupBudget::default();
        self.inner.remove_contents(&mut budget, 0)
    }

    fn remove_child_dir_matching(&self, name: &str, child: &TraceDir) -> Result<(), TraceCapError> {
        validate_component(name)?;
        self.inner.remove_child_dir_matching(name, &child.inner)
    }

    pub fn sweep_descriptor_temps(&self) -> Result<(), TraceCapError> {
        let mut budget = EnumerationBudget::default();
        let mut owned = Vec::<OsString>::new();
        self.inner.for_each_name(|name| {
            budget.account(name)?;
            if is_descriptor_temp(name) {
                owned.push(name.to_os_string());
            }
            Ok(())
        })?;
        for name in owned {
            self.inner.remove_owned_temp(&name)?;
        }
        Ok(())
    }

    pub fn fixed_exists(&self, document: FixedDocument) -> Result<bool, TraceCapError> {
        self.inner.entry_exists(document.name())
    }

    pub fn read_fixed(
        &self,
        document: FixedDocument,
    ) -> Result<Option<TraceReadFile>, TraceCapError> {
        self.inner
            .open_optional_reader(document.name())
            .map(|reader| reader.map(|inner| TraceReadFile { inner }))
    }

    /// The chain's first part, `chain.jsonl`: [`Self::open_part_for_create`] at offset 0.
    pub fn open_chain_for_create(&self) -> Result<TraceAppendFile, TraceCapError> {
        self.open_part_for_create(0)
    }

    pub fn open_acks_for_create(&self) -> Result<TraceAppendFile, TraceCapError> {
        self.open_append("acks.jsonl".to_string(), "acks.jsonl", true)
    }

    /// The chain's first part, `chain.jsonl`: [`Self::open_part_for_resume`] at offset 0.
    pub fn open_chain_for_resume(&self) -> Result<TraceAppendFile, TraceCapError> {
        self.open_part_for_resume(0)
    }

    pub fn open_acks_for_resume(&self) -> Result<TraceAppendFile, TraceCapError> {
        self.open_append("acks.jsonl".to_string(), "acks.jsonl", true)
    }

    /// Open, creating it if absent, the chain part that starts at logical byte `start`
    /// ([`part_file_name`]). An existing file is opened, not refused: the caller checks that
    /// it is empty, exactly as for the single-file chain.
    pub fn open_part_for_create(&self, start: u64) -> Result<TraceAppendFile, TraceCapError> {
        self.open_append(part_file_name(start), "chain.jsonl", true)
    }

    /// Open the existing chain part that starts at logical byte `start`; a missing part is an
    /// error.
    pub fn open_part_for_resume(&self, start: u64) -> Result<TraceAppendFile, TraceCapError> {
        self.open_append(part_file_name(start), "chain.jsonl", false)
    }

    /// The start offsets of the chain parts in this directory, ascending. Only names
    /// [`parse_part_name`] accepts are parts; any other entry is ignored. The walk is bounded
    /// like every other enumeration of a trace directory.
    pub fn list_parts(&self) -> Result<Vec<u64>, TraceCapError> {
        let mut budget = EnumerationBudget::default();
        let mut starts = Vec::new();
        self.inner.for_each_name(|name| {
            budget.account(name)?;
            if let Some(start) = name.to_str().and_then(parse_part_name) {
                starts.push(start);
            }
            Ok(())
        })?;
        starts.sort_unstable();
        Ok(starts)
    }

    fn open_append(
        &self,
        name: String,
        stream: &'static str,
        create: bool,
    ) -> Result<TraceAppendFile, TraceCapError> {
        self.inner
            .open_append(&name, create)
            .map(|inner| TraceAppendFile {
                inner,
                name,
                stream,
                poisoned: false,
            })
    }

    pub fn create_fixed(&self, document: FixedDocument) -> Result<TraceFixedWriter, TraceCapError> {
        self.inner
            .create_fixed(document.name())
            .map(|inner| TraceFixedWriter { inner })
    }

    pub fn replace_descriptor(&self, bytes: &[u8]) -> Result<(), TraceCapError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temporary = format!(
            "trace.toml.{}.{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        self.inner.replace_descriptor(&temporary, bytes)
    }

    /// Make every byte appended through `writer` durable under its fixed name in this
    /// retained directory: the file's data and length, then the directory entry naming it.
    ///
    /// Durable means readable at the retained name after a crash, so the name must still
    /// identify the writer's own file when the flush completes; a replaced name is refused
    /// rather than reported as a durable stream nobody would open.
    ///
    /// Any failure permanently poisons `writer`. After a failed flush the kernel may already
    /// have dropped the unflushed pages, so a later flush could succeed without them; no
    /// later call may therefore report those bytes durable.
    pub fn sync_append(&self, writer: &mut TraceAppendFile) -> Result<(), TraceCapError> {
        if writer.poisoned {
            return Err(TraceCapError::Poisoned {
                stream: writer.stream,
            });
        }
        let result = (|| {
            writer.inner.validate()?;
            run_presync_hook()?;
            writer.inner.sync_all()?;
            writer.inner.validate()?;
            self.inner
                .check_append_binding(&writer.name, &writer.inner)?;
            self.inner.sync()
        })();
        if result.is_err() {
            writer.poisoned = true;
        }
        result
    }

    /// Open a read-only handle by the retained writer's own child name and require it
    /// to identify the same file. Any unsafe acquisition permanently poisons `writer`.
    pub fn open_matching_reader(
        &self,
        writer: &mut TraceAppendFile,
    ) -> Result<TraceReadFile, TraceCapError> {
        if writer.poisoned {
            return Err(TraceCapError::Poisoned {
                stream: writer.stream,
            });
        }
        match self.inner.open_matching_reader(&writer.name, &writer.inner) {
            Ok(inner) => Ok(TraceReadFile { inner }),
            Err(error) => {
                writer.poisoned = true;
                Err(error)
            }
        }
    }
}

#[derive(Default)]
struct CleanupBudget {
    entries: usize,
}

impl CleanupBudget {
    fn enter(&mut self, depth: usize) -> Result<(), TraceCapError> {
        if depth > MAX_CLEANUP_DEPTH {
            return Err(TraceCapError::refused(
                "remove artifact tree",
                RefusalKind::DirectoryEntryLimit,
            ));
        }
        Ok(())
    }

    fn account(&mut self) -> Result<(), TraceCapError> {
        self.entries = self.entries.saturating_add(1);
        if self.entries > MAX_CLEANUP_ENTRIES {
            return Err(TraceCapError::refused(
                "remove artifact tree",
                RefusalKind::DirectoryEntryLimit,
            ));
        }
        Ok(())
    }
}

/// The daemon-lifetime authority for one selected artifact root.
///
/// The configured path is used once, at acquisition. Registry, entry and cleanup work then
/// descends from this retained native directory handle, so replacing the configured name
/// cannot redirect a later operation.
pub struct ArtifactRoot {
    root: TraceDir,
    display: PathBuf,
}

impl fmt::Debug for ArtifactRoot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactRoot")
            .field("display", &self.display)
            .finish_non_exhaustive()
    }
}

impl ArtifactRoot {
    /// Acquire an existing selected artifact root without following its final link or
    /// reparse point.
    pub fn open(configured_root: &Path) -> Result<Self, TraceCapError> {
        Ok(Self {
            root: TraceDir::open(configured_root)?,
            display: configured_root.to_path_buf(),
        })
    }

    /// Open or create the fixed registry parent and leaf through this retained root.
    pub fn open_registry_file(&self) -> Result<File, TraceCapError> {
        self.ensure_current()?;
        let registry = self.root.open_or_create_child_dir("registry")?;
        registry.restrict(0o700)?;
        let file = registry.open_or_create_regular("registry.redb")?;
        restrict_regular(&file, 0o600)?;
        Ok(file)
    }

    pub fn create_entry(&self, entry_id: &str) -> Result<EntryArtifactRoot, TraceCapError> {
        validate_component(entry_id)?;
        self.ensure_current()?;
        let artifacts = self.root.open_or_create_child_dir("artifacts")?;
        let entry = artifacts.create_child_dir(entry_id)?;
        Ok(EntryArtifactRoot::from_acquired(
            artifacts,
            entry,
            entry_id,
            self.display.join("artifacts").join(entry_id),
        ))
    }

    pub fn open_entry(&self, entry_id: &str) -> Result<EntryArtifactRoot, TraceCapError> {
        validate_component(entry_id)?;
        self.ensure_current()?;
        let artifacts = self.root.open_child_dir("artifacts")?;
        let entry = artifacts.open_child_dir(entry_id)?;
        Ok(EntryArtifactRoot::from_acquired(
            artifacts,
            entry,
            entry_id,
            self.display.join("artifacts").join(entry_id),
        ))
    }

    pub fn display_path(&self) -> &Path {
        &self.display
    }

    fn ensure_current(&self) -> Result<(), TraceCapError> {
        let current = TraceDir::open(&self.display)?;
        if current.identity()? != self.root.identity()? {
            return Err(TraceCapError::refused(
                "validate selected artifact root",
                RefusalKind::IdentityChanged,
            ));
        }
        Ok(())
    }
}

#[cfg(unix)]
fn restrict_regular(file: &File, mode: u32) -> Result<(), TraceCapError> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(mode))
        .map_err(|source| TraceCapError::io("restrict registry file", source))
}

/// The retained authority for one configured `artifacts/<entry-id>` directory.
///
/// The display paths are diagnostics only. Every returned trace root is opened relative to
/// the native directory capability held here, so a later rename or reparse-point swap of an ancestor cannot redirect it.
pub struct EntryArtifactRoot {
    artifacts: TraceDir,
    entry: TraceDir,
    name: String,
    display: PathBuf,
}

impl fmt::Debug for EntryArtifactRoot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EntryArtifactRoot")
            .field("display", &self.display)
            .finish_non_exhaustive()
    }
}

impl EntryArtifactRoot {
    /// Create a fresh `configured_root/artifacts/entry_id` directory tree entirely through
    /// retained directory handles.  The final entry component is exclusive: retry/rebuild
    /// must call [`Self::open`] and never silently adopt fresh creation state.
    pub fn create(configured_root: &Path, entry_id: &str) -> Result<Self, TraceCapError> {
        ArtifactRoot::open(configured_root)?.create_entry(entry_id)
    }

    /// Acquire `configured_root/artifacts/entry_id` component-by-component, without
    /// following any final or intermediate symlink/reparse point.
    pub fn open(configured_root: &Path, entry_id: &str) -> Result<Self, TraceCapError> {
        validate_component(entry_id)?;
        let root = TraceDir::open(configured_root)?;
        let artifacts = root.open_child_dir("artifacts")?;
        let entry = artifacts.open_child_dir(entry_id)?;
        Ok(Self::from_acquired(
            artifacts,
            entry,
            entry_id,
            configured_root.join("artifacts").join(entry_id),
        ))
    }

    fn from_acquired(
        artifacts: TraceDir,
        entry: TraceDir,
        entry_id: &str,
        display: PathBuf,
    ) -> Self {
        Self {
            artifacts,
            entry,
            name: entry_id.to_string(),
            display,
        }
    }

    /// Create a new final `trace` child.  It is exclusive by design: fresh construction
    /// must never adopt an empty directory left by another attempted run.
    pub fn create_trace(&self) -> Result<TraceDir, TraceCapError> {
        self.entry.create_child_dir("trace")
    }

    /// Open the existing final `trace` child for restart/recovery.
    pub fn open_trace(&self) -> Result<TraceDir, TraceCapError> {
        self.entry.open_child_dir("trace")
    }

    /// Recursively remove this exact acquired entry without following child links or a
    /// replacement installed at its former name. A failure is authoritative: callers must
    /// retain their entry record and report that cleanup did not complete.
    pub fn remove_recursively(&self) -> Result<(), TraceCapError> {
        self.entry.remove_contents()?;
        self.artifacts
            .remove_child_dir_matching(&self.name, &self.entry)
    }

    pub fn display_path(&self) -> &Path {
        &self.display
    }
}

fn validate_component(name: &str) -> Result<(), TraceCapError> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return Err(TraceCapError::refused(
            "validate contained artifact component",
            RefusalKind::NameTooLong,
        ));
    }
    Ok(())
}

/// The name of the chain part that starts at logical byte `start`: `chain.jsonl` for the
/// first part, `chain.<start, 20 digits>.jsonl` after it, so names sort in chain order.
pub fn part_file_name(start: u64) -> String {
    if start == 0 {
        "chain.jsonl".to_string()
    } else {
        format!("chain.{start:020}.jsonl")
    }
}

/// The start offset a chain part's name carries, or `None` for any name
/// [`part_file_name`] does not produce: exactly `chain.jsonl` (0), or `chain.` then 20
/// decimal digits naming a value above 0, then `.jsonl`.
pub fn parse_part_name(name: &str) -> Option<u64> {
    if name == "chain.jsonl" {
        return Some(0);
    }
    let digits = name.strip_prefix("chain.")?.strip_suffix(".jsonl")?;
    if digits.len() != 20 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u64>().ok().filter(|start| *start > 0)
}

/// An append stream whose checks and only write path remain inside this crate.
pub struct TraceAppendFile {
    inner: imp::AppendFileHandle,
    name: String,
    stream: &'static str,
    poisoned: bool,
}

impl fmt::Debug for TraceAppendFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TraceAppendFile")
            .field("stream", &self.stream)
            .field("poisoned", &self.poisoned)
            .finish_non_exhaustive()
    }
}

impl TraceAppendFile {
    /// The file's byte length: a fallible handle query, not a collection size, so there is no
    /// `is_empty` to pair it with.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&mut self) -> Result<u64, TraceCapError> {
        if self.poisoned {
            return Err(TraceCapError::Poisoned {
                stream: self.stream,
            });
        }
        let result = self.inner.validate().and_then(|()| self.inner.len());
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    pub fn append_checked(&mut self, bytes: &[u8]) -> Result<(), TraceCapError> {
        if self.poisoned {
            return Err(TraceCapError::Poisoned {
                stream: self.stream,
            });
        }
        let result = (|| {
            self.inner.validate()?;
            self.inner.write_all(bytes)?;
            run_postwrite_hook();
            self.inner.validate()
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
}

impl Read for TraceAppendFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl Seek for TraceAppendFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}

/// A consuming writer for one absent-only fixed document.
pub struct TraceFixedWriter {
    inner: imp::FixedWriterHandle,
}

impl TraceFixedWriter {
    pub fn write_all_sync_checked(mut self, bytes: &[u8]) -> Result<(), TraceCapError> {
        self.inner.validate()?;
        self.inner.write_all(bytes)?;
        self.inner.validate()?;
        self.inner.sync_all()
    }
}

/// An opaque read-only trace file.
pub struct TraceReadFile {
    inner: File,
}

impl Read for TraceReadFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl Seek for TraceReadFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}

#[derive(Default)]
struct EnumerationBudget {
    entries: usize,
    name_units: usize,
}

impl EnumerationBudget {
    fn account(&mut self, name: &OsStr) -> Result<(), TraceCapError> {
        self.entries = self.entries.checked_add(1).ok_or_else(|| {
            TraceCapError::refused(
                "enumerate trace directory",
                RefusalKind::DirectoryEntryLimit,
            )
        })?;
        if self.entries > MAX_ENTRIES {
            return Err(TraceCapError::refused(
                "enumerate trace directory",
                RefusalKind::DirectoryEntryLimit,
            ));
        }
        self.name_units = self
            .name_units
            .checked_add(raw_name_units(name))
            .ok_or_else(|| {
                TraceCapError::refused("enumerate trace directory", RefusalKind::DirectoryNameLimit)
            })?;
        if self.name_units > MAX_NAME_UNITS {
            return Err(TraceCapError::refused(
                "enumerate trace directory",
                RefusalKind::DirectoryNameLimit,
            ));
        }
        Ok(())
    }
}

#[cfg(unix)]
fn raw_name_units(name: &OsStr) -> usize {
    use std::os::unix::ffi::OsStrExt;
    name.as_bytes().len()
}

fn is_descriptor_temp(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(middle) = name
        .strip_prefix("trace.toml.")
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    let mut parts = middle.split('.');
    let (Some(pid), Some(counter), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    canonical_decimal::<u32>(pid) && canonical_decimal::<u64>(counter)
}

fn canonical_decimal<T>(value: &str) -> bool
where
    T: std::str::FromStr + fmt::Display,
{
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value
            .parse::<T>()
            .is_ok_and(|parsed| parsed.to_string() == value)
}

#[cfg(test)]
thread_local! {
    static POSTWRITE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn run_postwrite_hook() {
    POSTWRITE_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_postwrite_hook() {}

// Fault seam at the real flush boundary: a unit test may make the next flush fail exactly
// where the operating system's would, before any byte is flushed.
#[cfg(test)]
thread_local! {
    static PRESYNC_FAILURE: std::cell::RefCell<Option<io::Error>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn run_presync_hook() -> Result<(), TraceCapError> {
    PRESYNC_FAILURE.with(|failure| match failure.borrow_mut().take() {
        Some(error) => Err(TraceCapError::io("sync trace append stream", error)),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn run_presync_hook() -> Result<(), TraceCapError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::path::{Path, PathBuf};

    #[cfg(unix)]
    struct Fixture(PathBuf);

    #[cfg(unix)]
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::current_dir()
                .unwrap()
                .join("target/tracecap-unit-tests")
                .join(format!(
                    "common-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    #[cfg(unix)]
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn descriptor_temp_grammar_is_canonical_and_bounded() {
        for name in [
            "trace.toml.0.0.tmp",
            "trace.toml.4294967295.18446744073709551615.tmp",
        ] {
            assert!(is_descriptor_temp(OsStr::new(name)), "{name}");
        }
        for name in [
            "foo.tmp",
            "trace.toml..0.tmp",
            "trace.toml.00.0.tmp",
            "trace.toml.+1.0.tmp",
            "trace.toml.1.-1.tmp",
            "trace.toml.4294967296.0.tmp",
            "trace.toml.1.18446744073709551616.tmp",
            "trace.toml.1.0.extra.tmp",
        ] {
            assert!(!is_descriptor_temp(OsStr::new(name)), "{name}");
        }
    }

    #[test]
    fn enumeration_budget_counts_unmatched_names_before_deletion() {
        let mut budget = EnumerationBudget::default();
        for _ in 0..MAX_ENTRIES {
            budget.account(OsStr::new("unrelated")).unwrap();
        }
        assert!(budget.account(OsStr::new("also-unrelated")).is_err());

        let mut budget = EnumerationBudget {
            name_units: MAX_NAME_UNITS,
            ..EnumerationBudget::default()
        };
        assert!(budget.account(OsStr::new("x")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_postwrite_link_check_poisons_the_stream() {
        let tmp = Fixture::new();
        let root = TraceDir::create(tmp.path()).unwrap();
        let chain_path = tmp.path().join("chain.jsonl");
        let alias = tmp.path().join("alias");
        let mut chain = root.open_chain_for_create().unwrap();
        POSTWRITE_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new({
                let chain_path = chain_path.clone();
                let alias = alias.clone();
                move || std::fs::hard_link(chain_path, alias).unwrap()
            }));
        });
        assert!(chain.append_checked(b"first").is_err());
        std::fs::remove_file(alias).unwrap();
        assert!(matches!(
            chain.append_checked(b"second"),
            Err(TraceCapError::Poisoned { .. })
        ));
        assert_eq!(std::fs::read(chain_path).unwrap(), b"first");
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_flush_poisons_the_stream_for_every_later_append_and_flush() {
        let tmp = Fixture::new();
        let root = TraceDir::create(tmp.path()).unwrap();
        let mut chain = root.open_chain_for_create().unwrap();
        chain.append_checked(b"first\n").unwrap();
        root.sync_append(&mut chain).unwrap();
        chain.append_checked(b"second\n").unwrap();

        PRESYNC_FAILURE.with(|failure| {
            *failure.borrow_mut() = Some(io::Error::from_raw_os_error(libc::EIO));
        });
        assert!(matches!(
            root.sync_append(&mut chain),
            Err(TraceCapError::Io { source, .. }) if source.raw_os_error() == Some(libc::EIO)
        ));
        // A retried flush must not report bytes the failed one may have dropped.
        assert!(matches!(
            root.sync_append(&mut chain),
            Err(TraceCapError::Poisoned { .. })
        ));
        assert!(matches!(
            chain.append_checked(b"third\n"),
            Err(TraceCapError::Poisoned { .. })
        ));
        assert_eq!(
            std::fs::read(tmp.path().join("chain.jsonl")).unwrap(),
            b"first\nsecond\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_flush_refuses_a_name_that_no_longer_identifies_the_stream() {
        let tmp = Fixture::new();
        let root = TraceDir::create(tmp.path()).unwrap();
        let mut acks = root.open_acks_for_create().unwrap();
        acks.append_checked(b"one\n").unwrap();
        std::fs::rename(tmp.path().join("acks.jsonl"), tmp.path().join("moved")).unwrap();
        std::fs::write(tmp.path().join("acks.jsonl"), b"").unwrap();

        assert!(matches!(
            root.sync_append(&mut acks),
            Err(TraceCapError::Refused {
                reason: RefusalKind::IdentityChanged,
                ..
            })
        ));
        std::fs::remove_file(tmp.path().join("acks.jsonl")).unwrap();
        std::fs::rename(tmp.path().join("moved"), tmp.path().join("acks.jsonl")).unwrap();
        assert!(matches!(
            root.sync_append(&mut acks),
            Err(TraceCapError::Poisoned { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_flush_refuses_a_second_link_to_the_stream() {
        let tmp = Fixture::new();
        let root = TraceDir::create(tmp.path()).unwrap();
        let mut chain = root.open_chain_for_create().unwrap();
        chain.append_checked(b"one\n").unwrap();
        std::fs::hard_link(tmp.path().join("chain.jsonl"), tmp.path().join("alias")).unwrap();
        assert!(matches!(
            root.sync_append(&mut chain),
            Err(TraceCapError::Refused {
                reason: RefusalKind::MultipleLinks,
                ..
            })
        ));
        std::fs::remove_file(tmp.path().join("alias")).unwrap();
        assert!(matches!(
            root.sync_append(&mut chain),
            Err(TraceCapError::Poisoned { .. })
        ));
    }
}
