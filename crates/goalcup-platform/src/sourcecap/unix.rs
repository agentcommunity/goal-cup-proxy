use super::{RefusalKind, SourceCapError, SourceFactsV1, SourceKind};
use crate::handle_budget::{NativeHandleBudgetV1, NativeHandlePermitV1};
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Identity {
    device: u64,
    inode: u64,
    #[cfg(target_os = "macos")]
    birth_seconds: i64,
    #[cfg(target_os = "macos")]
    birth_nanos: i64,
}

pub(super) fn journal_token(identity: &Identity) -> String {
    #[cfg(target_os = "macos")]
    {
        format!(
            "source-root-v1:unix:{}:{}:{}:{}",
            identity.device, identity.inode, identity.birth_seconds, identity.birth_nanos
        )
    }
    #[cfg(not(target_os = "macos"))]
    format!("source-root-v1:unix:{}:{}", identity.device, identity.inode)
}

pub(super) fn snapshot_facts(facts: &Facts) -> SourceFactsV1 {
    SourceFactsV1 {
        identity: journal_token(&facts.identity),
        kind: facts.kind,
        mode: Some(facts.mode & 0o7777),
        length: facts.length,
        posix_owner_executable: facts.posix_owner_executable,
        modified_seconds: facts.modified_seconds,
        modified_nanos: facts.modified_nanos,
        changed_seconds: facts.changed_seconds,
        changed_nanos: facts.changed_nanos,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MountIdentity {
    #[cfg(target_os = "linux")]
    mount_id: u64,
    #[cfg(target_os = "macos")]
    fsid: [u8; 8],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Facts {
    pub(super) kind: SourceKind,
    pub(super) length: u64,
    pub(super) posix_owner_executable: Option<bool>,
    identity: Identity,
    links: u64,
    mode: u32,
    modified_seconds: i64,
    modified_nanos: i64,
    changed_seconds: i64,
    changed_nanos: i64,
}

pub(super) struct DirHandle {
    fd: OwnedFd,
    identity: Identity,
    _permit: NativeHandlePermitV1,
}

pub(super) struct FileHandle {
    file: File,
    _permit: NativeHandlePermitV1,
}

pub(super) struct Entries {
    directory: *mut libc::DIR,
    _permit: NativeHandlePermitV1,
    complete: bool,
}

impl DirHandle {
    pub(super) fn open_selected(
        path: &Path,
        permit: NativeHandlePermitV1,
    ) -> Result<(Self, MountIdentity), SourceCapError> {
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            SourceCapError::refused("open selected source root", RefusalKind::UnsupportedRoot)
        })?;
        // SAFETY: the path is a live NUL-terminated CString for the call; O_NOFOLLOW and O_DIRECTORY refuse a symlinked or non-directory root.
        let raw = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        let fd = owned_fd(raw, "open selected source root")?;
        let facts = facts_from_fd(fd.as_raw_fd(), "inspect selected source root")?;
        if facts.kind != SourceKind::Directory {
            return Err(SourceCapError::refused(
                "open selected source root",
                RefusalKind::WrongKind,
            ));
        }
        let mount = mount_identity(fd.as_raw_fd(), "bind selected source root")?;
        Ok((
            Self {
                fd,
                identity: facts.identity,
                _permit: permit,
            },
            mount,
        ))
    }

    pub(super) fn identity(&self) -> &Identity {
        &self.identity
    }

    pub(super) fn facts(&self) -> Result<Facts, SourceCapError> {
        facts_from_fd(self.fd.as_raw_fd(), "inspect source directory")
    }

    pub(super) fn observe(
        &self,
        name: &OsStr,
        _budget: &NativeHandleBudgetV1,
    ) -> Result<Option<Facts>, SourceCapError> {
        let name = native_name(name, "observe source entry")?;
        let mut stat = MaybeUninit::<libc::stat>::uninit();
        // SAFETY: the retained parent descriptor and the NUL-terminated name are live; AT_SYMLINK_NOFOLLOW means a final symlink is described, not followed; the output is a writable MaybeUninit<stat>.
        let result = unsafe {
            libc::fstatat(
                self.fd.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result != 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::ENOENT) {
                Ok(None)
            } else {
                Err(SourceCapError::io("observe source entry", error))
            };
        }
        // SAFETY: the preceding system call reported success, so it fully initialised the output buffer.
        facts_from_stat(unsafe { stat.assume_init() }, "observe source entry").map(Some)
    }

    pub(super) fn entries(&self, permit: NativeHandlePermitV1) -> Result<Entries, SourceCapError> {
        // SAFETY: the descriptor is open and F_DUPFD_CLOEXEC takes an integer argument; it only creates a new close-on-exec descriptor.
        let raw = unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        let duplicate = owned_fd(raw, "duplicate source directory for enumeration")?;
        // SAFETY: `duplicate` is an open directory descriptor owned here; on success the stream takes it over, and the caller forgets the OwnedFd.
        let directory = unsafe { libc::fdopendir(duplicate.as_raw_fd()) };
        if directory.is_null() {
            return Err(SourceCapError::io(
                "open source enumeration",
                io::Error::last_os_error(),
            ));
        }
        std::mem::forget(duplicate);
        // SAFETY: the stream is non-null (checked above), still open and used only by this thread.
        unsafe { libc::rewinddir(directory) };
        Ok(Entries {
            directory,
            _permit: permit,
            complete: false,
        })
    }

    pub(super) fn open_dir(
        &self,
        name: &OsStr,
        root_mount: &MountIdentity,
        permit: NativeHandlePermitV1,
    ) -> Result<Self, SourceCapError> {
        let name = native_name(name, "open matching source directory")?;
        // SAFETY: the parent descriptor is open and the name is a live NUL-terminated CString for the call; the flags include O_NOFOLLOW so no final symlink is followed.
        let raw = unsafe {
            libc::openat(
                self.fd.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        let fd = owned_fd(raw, "open matching source directory")?;
        let facts = facts_from_fd(fd.as_raw_fd(), "inspect matching source directory")?;
        if facts.kind != SourceKind::Directory {
            return Err(SourceCapError::refused(
                "open matching source directory",
                RefusalKind::WrongKind,
            ));
        }
        validate_mount(
            fd.as_raw_fd(),
            root_mount,
            true,
            "open matching source directory",
        )?;
        Ok(Self {
            fd,
            identity: facts.identity,
            _permit: permit,
        })
    }

    pub(super) fn open_regular(
        &self,
        name: &OsStr,
        root_mount: &MountIdentity,
        permit: NativeHandlePermitV1,
    ) -> Result<FileHandle, SourceCapError> {
        let name = native_name(name, "open matching regular source")?;
        // SAFETY: the parent descriptor is open and the name is a live NUL-terminated CString for the call; the flags include O_NOFOLLOW so no final symlink is followed.
        let raw = unsafe {
            libc::openat(
                self.fd.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        let fd = owned_fd(raw, "open matching regular source")?;
        let facts = facts_from_fd(fd.as_raw_fd(), "inspect matching regular source")?;
        if facts.kind != SourceKind::Regular {
            let reason = if facts.kind == SourceKind::Special {
                RefusalKind::SpecialFile
            } else {
                RefusalKind::WrongKind
            };
            return Err(SourceCapError::refused(
                "open matching regular source",
                reason,
            ));
        }
        validate_mount(
            fd.as_raw_fd(),
            root_mount,
            false,
            "open matching regular source",
        )?;
        Ok(FileHandle {
            file: File::from(fd),
            _permit: permit,
        })
    }

    pub(super) fn read_symlink(
        &self,
        name: &OsStr,
        root_mount: &MountIdentity,
        expected: &Facts,
        destination: &mut [u8],
        permit: NativeHandlePermitV1,
    ) -> Result<usize, SourceCapError> {
        let name = native_name(name, "read matching symbolic link")?;
        #[cfg(target_os = "linux")]
        let flags = libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        #[cfg(target_os = "macos")]
        let flags = libc::O_RDONLY | libc::O_SYMLINK | libc::O_CLOEXEC;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (name, root_mount, expected, destination, permit);
            return Err(SourceCapError::refused(
                "read matching symbolic link",
                RefusalKind::UnsupportedSymlinkPayload,
            ));
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            // SAFETY: the parent descriptor is open and the name is a live NUL-terminated CString for the call; the flags include O_NOFOLLOW so no final symlink is followed.
            let raw = unsafe { libc::openat(self.fd.as_raw_fd(), name.as_ptr(), flags) };
            let fd = owned_fd(raw, "open symbolic link descriptor")?;
            let before = facts_from_fd(fd.as_raw_fd(), "inspect symbolic link descriptor")?;
            if &before != expected {
                return Err(SourceCapError::refused(
                    "read matching symbolic link",
                    RefusalKind::MetadataChanged,
                ));
            }
            validate_mount(
                fd.as_raw_fd(),
                root_mount,
                false,
                "read matching symbolic link",
            )?;
            let count = descriptor_readlink(fd.as_raw_fd(), destination)?;
            if count == destination.len() {
                return Err(SourceCapError::refused(
                    "read matching symbolic link",
                    RefusalKind::BufferLimit,
                ));
            }
            let after = facts_from_fd(fd.as_raw_fd(), "recheck symbolic link descriptor")?;
            if after != before {
                return Err(SourceCapError::refused(
                    "read matching symbolic link",
                    RefusalKind::MetadataChanged,
                ));
            }
            drop(fd);
            drop(permit);
            Ok(count)
        }
    }
}

impl FileHandle {
    pub(super) fn facts(&self) -> Result<Facts, SourceCapError> {
        facts_from_fd(self.file.as_raw_fd(), "inspect source stream")
    }
    pub(super) fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
        self.file.read(destination)
    }
    pub(super) fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

impl Entries {
    pub(super) fn next(
        &mut self,
        owner: &Identity,
        mount: &MountIdentity,
    ) -> Result<Option<OsString>, SourceCapError> {
        #[cfg(test)]
        if FORCE_ENUMERATION_ERROR.with(std::cell::Cell::take) {
            return Err(SourceCapError::io(
                "enumerate source directory",
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected native enumeration error",
                ),
            ));
        }
        if self.complete {
            return Ok(None);
        }
        loop {
            crate::errno::clear();
            // SAFETY: the stream is non-null, open and used only by this thread; a null return is distinguished from an error through errno.
            let entry = unsafe { libc::readdir(self.directory) };
            if entry.is_null() {
                let errno = crate::errno::get();
                if errno != 0 {
                    return Err(SourceCapError::io(
                        "enumerate source directory",
                        io::Error::from_raw_os_error(errno),
                    ));
                }
                self.complete = true;
                // SAFETY: the stream is non-null and still open; dirfd returns the descriptor it owns without transferring ownership.
                let fd = unsafe { libc::dirfd(self.directory) };
                let facts = facts_from_fd(fd, "complete source enumeration")?;
                if &facts.identity != owner {
                    return Err(SourceCapError::refused(
                        "complete source enumeration",
                        RefusalKind::IdentityChanged,
                    ));
                }
                validate_mount(fd, mount, false, "complete source enumeration")?;
                return Ok(None);
            }
            // SAFETY: `entry` is non-null from readdir; d_name is NUL terminated and stays valid until the next readdir/closedir, and the bytes are copied or inspected before then.
            let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if bytes != b"." && bytes != b".." {
                return Ok(Some(OsString::from_vec(bytes.to_vec())));
            }
        }
    }
}

impl Drop for Entries {
    fn drop(&mut self) {
        if !self.directory.is_null() {
            // SAFETY: the stream is non-null and still open; it is closed exactly once on this path and not used afterwards.
            unsafe {
                libc::closedir(self.directory);
            }
        }
    }
}

fn native_name(name: &OsStr, operation: &'static str) -> Result<CString, SourceCapError> {
    CString::new(name.as_bytes())
        .map_err(|_| SourceCapError::refused(operation, RefusalKind::IdentityChanged))
}

fn owned_fd(raw: RawFd, operation: &'static str) -> Result<OwnedFd, SourceCapError> {
    if raw < 0 {
        return Err(SourceCapError::io(operation, io::Error::last_os_error()));
    }
    // SAFETY: the descriptor was just returned by a successful system call (checked non-negative), nothing else owns it, and ownership moves into the wrapper.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn facts_from_fd(fd: RawFd, operation: &'static str) -> Result<Facts, SourceCapError> {
    let mut stat = MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `fd` is open and the output is a writable MaybeUninit<stat>; the result is checked before it is read.
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(SourceCapError::io(operation, io::Error::last_os_error()));
    }
    // SAFETY: the preceding system call reported success, so it fully initialised the output buffer.
    facts_from_stat(unsafe { stat.assume_init() }, operation)
}

fn facts_from_stat(stat: libc::stat, operation: &'static str) -> Result<Facts, SourceCapError> {
    let kind = match stat.st_mode & libc::S_IFMT {
        libc::S_IFDIR => SourceKind::Directory,
        libc::S_IFREG => SourceKind::Regular,
        libc::S_IFLNK => SourceKind::SymbolicLink,
        _ => SourceKind::Special,
    };
    let length = u64::try_from(stat.st_size)
        .map_err(|_| SourceCapError::refused(operation, RefusalKind::LengthLimit))?;
    let (modified_seconds, modified_nanos, changed_seconds, changed_nanos) = stat_times(&stat);
    // These three casts are redundant on Linux glibc (dev_t/mode_t are already u64/u32, and
    // nlink_t is u64 on x86_64) but real on macOS, where dev_t is i32 and nlink_t/mode_t are
    // u16 -- widening casts there, not no-ops. `u64::from(stat.st_dev)` does not compile on
    // macOS because dev_t is signed there, so the cast form is the only one that works on
    // both platforms. This file has no `#[cfg]` split (gated once in sourcecap.rs), so
    // the allow is local to each binding rather than duplicating the function per platform.
    #[allow(
        clippy::unnecessary_cast,
        reason = "dev_t is i32 on macOS; the cast widens there even though it is a no-op on Linux glibc"
    )]
    let device = stat.st_dev as u64;
    #[allow(
        clippy::unnecessary_cast,
        reason = "nlink_t is u16 on macOS (u32 on aarch64 Linux, u64 on x86_64 Linux); the cast widens on macOS and 32-bit-nlink Linux"
    )]
    let links = stat.st_nlink as u64;
    #[allow(
        clippy::unnecessary_cast,
        reason = "mode_t is u16 on macOS; the cast widens there even though it is a no-op on Linux glibc"
    )]
    let mode = stat.st_mode as u32;
    Ok(Facts {
        kind,
        length,
        posix_owner_executable: (kind == SourceKind::Regular)
            .then_some(stat.st_mode & libc::S_IXUSR != 0),
        identity: Identity {
            device,
            inode: stat.st_ino,
            #[cfg(target_os = "macos")]
            birth_seconds: stat.st_birthtime,
            #[cfg(target_os = "macos")]
            birth_nanos: stat.st_birthtime_nsec,
        },
        links,
        mode,
        modified_seconds,
        modified_nanos,
        changed_seconds,
        changed_nanos,
    })
}

fn stat_times(stat: &libc::stat) -> (i64, i64, i64, i64) {
    (
        stat.st_mtime,
        stat.st_mtime_nsec,
        stat.st_ctime,
        stat.st_ctime_nsec,
    )
}

#[cfg(target_os = "linux")]
fn mount_identity(fd: RawFd, operation: &'static str) -> Result<MountIdentity, SourceCapError> {
    let mut statx = MaybeUninit::<libc::statx>::zeroed();
    let empty = b"\0";
    // SAFETY: `fd` is open and the empty path is NUL terminated; with AT_EMPTY_PATH the call describes `fd` itself and writes only the live statx output.
    let result = unsafe {
        libc::statx(
            fd,
            empty.as_ptr().cast(),
            libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_MNT_ID,
            statx.as_mut_ptr(),
        )
    };
    if result != 0 {
        return Err(SourceCapError::refused(
            operation,
            RefusalKind::UnsupportedMountBinding,
        ));
    }
    // SAFETY: the preceding system call reported success, so it fully initialised the output buffer.
    let statx = unsafe { statx.assume_init() };
    if statx.stx_mask & libc::STATX_MNT_ID == 0 {
        return Err(SourceCapError::refused(
            operation,
            RefusalKind::UnsupportedMountBinding,
        ));
    }
    Ok(MountIdentity {
        mount_id: statx.stx_mnt_id,
    })
}

#[cfg(target_os = "macos")]
fn mount_identity(fd: RawFd, operation: &'static str) -> Result<MountIdentity, SourceCapError> {
    let mut statfs = MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `fd` is open and the output is a writable MaybeUninit<statfs>; the result is checked before it is read.
    if unsafe { libc::fstatfs(fd, statfs.as_mut_ptr()) } != 0 {
        return Err(SourceCapError::io(operation, io::Error::last_os_error()));
    }
    // SAFETY: the preceding system call reported success, so it fully initialised the output buffer.
    let statfs = unsafe { statfs.assume_init() };
    let mut fsid = [0u8; 8];
    // SAFETY: source is a live 8-byte-or-larger fsid_t field read as bytes; the destination holds `fsid.len()` bytes; the regions are distinct locals.
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&statfs.f_fsid as *const libc::fsid_t).cast::<u8>(),
            fsid.as_mut_ptr(),
            fsid.len(),
        );
    }
    Ok(MountIdentity { fsid })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn mount_identity(_fd: RawFd, operation: &'static str) -> Result<MountIdentity, SourceCapError> {
    Err(SourceCapError::refused(
        operation,
        RefusalKind::UnsupportedMountBinding,
    ))
}

fn validate_mount(
    fd: RawFd,
    expected: &MountIdentity,
    directory: bool,
    operation: &'static str,
) -> Result<(), SourceCapError> {
    #[cfg(test)]
    if FORCE_CROSS_MOUNT.with(std::cell::Cell::take) {
        return Err(SourceCapError::refused(operation, RefusalKind::CrossMount));
    }
    if &mount_identity(fd, operation)? != expected {
        return Err(SourceCapError::refused(operation, RefusalKind::CrossMount));
    }
    #[cfg(target_os = "macos")]
    if directory && directory_mount_status(fd, operation)? != 0 {
        return Err(SourceCapError::refused(operation, RefusalKind::CrossMount));
    }
    let _ = directory;
    Ok(())
}

#[cfg(target_os = "macos")]
fn directory_mount_status(fd: RawFd, operation: &'static str) -> Result<u32, SourceCapError> {
    #[repr(C)]
    struct Buffer {
        length: u32,
        status: u32,
    }
    let mut attributes = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: 0,
        volattr: 0,
        dirattr: 0x0000_0004,
        fileattr: 0,
        forkattr: 0,
    };
    let mut buffer = Buffer {
        length: 0,
        status: 0,
    };
    // SAFETY: `fd` is open; the attrlist and the buffer are live, correctly sized for the stated length, and exclusively borrowed for the call.
    let result = unsafe {
        libc::fgetattrlist(
            fd,
            (&mut attributes as *mut libc::attrlist).cast(),
            (&mut buffer as *mut Buffer).cast(),
            std::mem::size_of::<Buffer>(),
            0,
        )
    };
    if result != 0 {
        return Err(SourceCapError::refused(
            operation,
            RefusalKind::UnsupportedMountBinding,
        ));
    }
    Ok(buffer.status & 0x0000_0003)
}

#[cfg(target_os = "linux")]
fn descriptor_readlink(fd: RawFd, destination: &mut [u8]) -> Result<usize, SourceCapError> {
    let empty = b"\0";
    // SAFETY: `fd` is open, the empty path is NUL terminated, and the destination is writable for exactly its length.
    let result = unsafe {
        libc::syscall(
            libc::SYS_readlinkat,
            fd,
            empty.as_ptr().cast::<libc::c_char>(),
            destination.as_mut_ptr().cast::<libc::c_char>(),
            destination.len(),
        )
    };
    if result < 0 {
        let error = io::Error::last_os_error();
        if matches!(
            error.raw_os_error(),
            Some(libc::ENOSYS | libc::EINVAL | libc::ENOENT)
        ) {
            return Err(SourceCapError::refused(
                "read matching symbolic link",
                RefusalKind::UnsupportedSymlinkPayload,
            ));
        }
        return Err(SourceCapError::io("read matching symbolic link", error));
    }
    usize::try_from(result).map_err(|_| {
        SourceCapError::refused("read matching symbolic link", RefusalKind::BufferLimit)
    })
}

#[cfg(target_os = "macos")]
fn descriptor_readlink(fd: RawFd, destination: &mut [u8]) -> Result<usize, SourceCapError> {
    type Freadlink =
        unsafe extern "C" fn(libc::c_int, *mut libc::c_char, libc::size_t) -> libc::ssize_t;
    // SAFETY: RTLD_DEFAULT with a static NUL-terminated symbol name; dlsym only reads the name and returns a pointer or null.
    let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"freadlink".as_ptr()) };
    if symbol.is_null() {
        return Err(SourceCapError::refused(
            "read matching symbolic link",
            RefusalKind::UnsupportedSymlinkPayload,
        ));
    }
    // SAFETY: `symbol` is the non-null address of the system `freadlink` function resolved by dlsym, whose C signature is exactly `Freadlink`.
    let function: Freadlink = unsafe { std::mem::transmute(symbol) };
    // SAFETY: `function` is the checked freadlink pointer; `fd` is open and the destination slice is writable for its stated length.
    let result = unsafe { function(fd, destination.as_mut_ptr().cast(), destination.len()) };
    if result < 0 {
        return Err(SourceCapError::io(
            "read matching symbolic link",
            io::Error::last_os_error(),
        ));
    }
    usize::try_from(result).map_err(|_| {
        SourceCapError::refused("read matching symbolic link", RefusalKind::BufferLimit)
    })
}

#[cfg(test)]
thread_local! {
    static FORCE_ENUMERATION_ERROR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FORCE_CROSS_MOUNT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(super) fn force_enumeration_error_once() {
    FORCE_ENUMERATION_ERROR.set(true);
}

#[cfg(test)]
pub(super) fn force_cross_mount_once() {
    FORCE_CROSS_MOUNT.set(true);
}
