//! Safe file writes for the installer (task 2.1): atomic replace, backups and
//! owner-only files.
//!
//! Spec: `.kiro/specs/reliable-installers/design.md` §3 (backups folder) and
//! §4 "Plan, apply, rollback" (owner-only key file); Requirements 3.1, 3.2,
//! 3.3 and 9.1. Library choices are in `research.md` §6.
//!
//! - Every write goes to a temporary file in the target's own folder, is
//!   flushed to disk and then swapped in. An interrupted write leaves the old
//!   file intact.
//! - [`write_atomic`] keeps the replaced file's permissions (Requirement 3.1:
//!   only mmhmem's entry changes). Unix copies the mode onto the temporary
//!   file before writing; Windows swaps with `ReplaceFileW`, which keeps the
//!   replaced file's ACL, owner, attributes and creation time.
//! - Owner-only files are private from the first byte: Unix creates them with
//!   mode `0600`; Windows creates them through `CreateFileW` with the current
//!   user as owner and a protected DACL granting only that user access.
//! - Backups are byte-identical copies, always created owner-only because a
//!   config file may hold a secret.
//!
//! Nothing in this module logs or prints file contents.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tempfile::NamedTempFile;

/// How many name variants [`backup`] tries before giving up.
const MAX_BACKUP_VARIANTS: u32 = 1000;

/// Atomically replace `path` with `bytes`, keeping its permissions.
///
/// Missing parent folders are created. When `path` exists, its permissions
/// survive (Unix mode; Windows ACL, owner and attributes). A new file gets
/// the defaults: on Windows the folder's inherited ACL, on Unix mode `0600`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = stage(path, bytes, false)?;
    let swapped = imp::replace_preserving(temp.into_temp_path(), path);
    finish(swapped, path)
}

/// Atomically replace `path` with `bytes` in a file that only the current
/// user can access (for the worker API key, Requirement 9.1).
///
/// The temporary file is created private before any byte is written, so the
/// secret is never readable by others. Existing broader permissions on `path`
/// are deliberately not kept.
pub fn write_owner_only(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = stage(path, bytes, true)?;
    finish(temp.persist(path).map(drop).map_err(|err| err.error), path)
}

/// Copy `source` byte-for-byte into `backup_dir` and return the backup path.
///
/// Returns `Ok(None)` when `source` does not exist (nothing to back up).
/// The backup keeps the source file name; when that name is already taken in
/// `backup_dir`, a numbered prefix (`1-name`, `2-name`, …) is used instead, so
/// an earlier backup is never overwritten. The backup file is owner-only.
pub fn backup(source: &Path, backup_dir: &Path) -> Result<Option<PathBuf>> {
    let bytes = match fs_err::read(source) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context("could not read the file to back up"),
    };
    let name = source
        .file_name()
        .with_context(|| format!("cannot back up {}: no file name", source.display()))?;
    fs_err::create_dir_all(backup_dir).context("could not create the backup folder")?;
    let (target, mut file) = create_backup_file(backup_dir, &name.to_string_lossy())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .with_context(|| format!("could not write backup {}", target.display()))?;
    Ok(Some(target))
}

/// Create a new owner-only file at `path`; fails if `path` already exists.
///
/// Unix: `open(O_CREAT | O_EXCL, 0600)`. Windows: `CreateFileW(CREATE_NEW)`
/// with the current user as owner and a protected DACL granting
/// `FILE_ALL_ACCESS` to that user only.
pub fn create_owner_only(path: &Path) -> io::Result<File> {
    imp::create_owner_only(path)
}

/// Write `bytes` to a durable temporary file next to `path`, not yet swapped.
fn stage(path: &Path, bytes: &[u8], owner_only: bool) -> Result<NamedTempFile> {
    let dir = parent_dir(path);
    fs_err::create_dir_all(dir).context("could not create the folder for the file")?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".mmhmem-").suffix(".tmp");
    let created = if owner_only {
        builder.make_in(dir, create_owner_only)
    } else {
        builder.tempfile_in(dir)
    };
    let mut temp = created
        .with_context(|| format!("could not create a temporary file in {}", dir.display()))?;
    if !owner_only {
        imp::copy_mode(path, temp.as_file())
            .with_context(|| format!("could not copy the permissions of {}", path.display()))?;
    }
    temp.write_all(bytes)
        .and_then(|()| temp.as_file().sync_all())
        .with_context(|| format!("could not write a temporary file for {}", path.display()))?;
    Ok(temp)
}

/// Report a swap result with the path, then make the swap durable.
fn finish(swapped: io::Result<()>, path: &Path) -> Result<()> {
    swapped.with_context(|| format!("could not replace {}", path.display()))?;
    imp::sync_dir(parent_dir(path));
    Ok(())
}

/// The folder that holds `path` (the current folder for a bare file name).
fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Create a fresh owner-only backup file named after `name` in `dir`.
fn create_backup_file(dir: &Path, name: &str) -> Result<(PathBuf, File)> {
    for n in 0..MAX_BACKUP_VARIANTS {
        let candidate = if n == 0 {
            dir.join(name)
        } else {
            dir.join(format!("{n}-{name}"))
        };
        match create_owner_only(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("could not create backup {}", candidate.display()));
            }
        }
    }
    anyhow::bail!("too many backups of {name} in {}", dir.display())
}

#[cfg(unix)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;

    use tempfile::TempPath;

    pub(super) fn create_owner_only(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }

    /// Give the temporary file the mode of an existing `target`.
    pub(super) fn copy_mode(target: &Path, temp: &File) -> io::Result<()> {
        match std::fs::metadata(target) {
            Ok(meta) => temp.set_permissions(meta.permissions()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Rename over `path`; the mode was already copied by [`copy_mode`].
    pub(super) fn replace_preserving(temp: TempPath, path: &Path) -> io::Result<()> {
        temp.persist(path).map_err(|err| err.error)
    }

    /// Make the rename durable; best effort, the rename already happened.
    pub(super) fn sync_dir(dir: &Path) {
        if let Ok(handle) = File::open(dir) {
            let _ = handle.sync_all();
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::{OsStr, c_void};
    use std::fs::File;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use std::path::Path;
    use std::ptr;

    use tempfile::TempPath;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_FILE_NOT_FOUND, GENERIC_READ, GENERIC_WRITE, HANDLE,
        INVALID_HANDLE_VALUE, LocalFree,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Memory returned by a Win32 call that must be released with `LocalFree`.
    struct LocalBox(*mut c_void);

    impl Drop for LocalBox {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the pointer came from a Win32 allocator documented to
                // be freed with LocalFree and is freed exactly once here.
                unsafe { LocalFree(self.0) };
            }
        }
    }

    /// A kernel handle closed on drop.
    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: the handle is valid and owned by this value only.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// The `TOKEN_USER` of the current process, with the user SID inside.
    pub(crate) struct CurrentUser {
        // u64 storage keeps the TOKEN_USER (which holds a pointer) aligned.
        buf: Vec<u64>,
    }

    impl CurrentUser {
        pub(crate) fn query() -> io::Result<Self> {
            let mut raw: HANDLE = ptr::null_mut();
            // SAFETY: GetCurrentProcess returns a pseudo handle; `raw` is a
            // valid out pointer.
            if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let token = OwnedHandle(raw);
            let mut len = 0u32;
            // SAFETY: a size query with a null buffer; it fails by design
            // and only writes the needed length into `len`.
            unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut len) };
            let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
            let size = u32::try_from(buf.len() * 8).map_err(io::Error::other)?;
            // SAFETY: `buf` is writable for `size` bytes and 8-byte aligned.
            let ok = unsafe {
                GetTokenInformation(token.0, TokenUser, buf.as_mut_ptr().cast(), size, &mut len)
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { buf })
        }

        /// The user SID; valid while `self` lives.
        pub(crate) fn sid(&self) -> PSID {
            // SAFETY: GetTokenInformation(TokenUser) filled `buf` with an
            // aligned TOKEN_USER whose SID points into the same buffer.
            unsafe { (*self.buf.as_ptr().cast::<TOKEN_USER>()).User.Sid }
        }

        /// The user SID in `S-1-5-…` form.
        pub(crate) fn sid_string(&self) -> io::Result<String> {
            // SAFETY: `sid()` points into `self.buf`, which outlives the call.
            unsafe { sid_to_string(self.sid()) }
        }
    }

    /// Format a SID as `S-1-5-…`.
    ///
    /// # Safety
    /// `sid` must point to a valid SID for the duration of the call.
    pub(crate) unsafe fn sid_to_string(sid: PSID) -> io::Result<String> {
        let mut wide = ptr::null_mut();
        // SAFETY: `sid` is valid (caller contract); `wide` receives a
        // LocalAlloc'd, NUL-terminated string freed by `LocalBox`.
        if unsafe { ConvertSidToStringSidW(sid, &mut wide) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let owned = LocalBox(wide.cast());
        let mut len = 0;
        // SAFETY: the string is NUL-terminated, so every read up to and
        // including the terminator is in bounds.
        while unsafe { *wide.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: `len` u16s before the terminator are initialized.
        let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(wide, len) });
        drop(owned);
        Ok(text)
    }

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain(Some(0)).collect()
    }

    pub(super) fn create_owner_only(path: &Path) -> io::Result<File> {
        let user = CurrentUser::query()?;
        // Owner = current user (an elevated session would otherwise make
        // Administrators the owner); protected DACL (P: no inherited ACEs)
        // with one ACE: file-all-access for the current user.
        let sid = user.sid_string()?;
        let sddl = wide(OsStr::new(&format!("O:{sid}D:P(A;;FA;;;{sid})")));
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated; `descriptor` receives a
        // LocalAlloc'd descriptor freed by `LocalBox`.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let _descriptor = LocalBox(descriptor);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).map_err(io::Error::other)?,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let name = wide(path.as_os_str());
        // Share delete so `tempfile` can rename and remove the open file.
        let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
        // SAFETY: `name` is NUL-terminated; `attributes` and the descriptor
        // it points to outlive the call.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                share,
                &attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `handle` is a valid, newly opened file handle that nothing
        // else owns; `File` takes ownership and closes it.
        Ok(unsafe { File::from_raw_handle(handle) })
    }

    /// The ACL is kept by [`replace_preserving`], not on the temporary file.
    pub(super) fn copy_mode(_target: &Path, _temp: &File) -> io::Result<()> {
        Ok(())
    }

    /// Swap `temp` in with `ReplaceFileW`, which keeps the replaced file's
    /// ACL, owner, attributes and creation time. When `path` does not exist
    /// yet, fall back to a plain rename (`MoveFileExW`). On failure the target
    /// is untouched and dropping `temp` removes the temporary file.
    pub(super) fn replace_preserving(temp: TempPath, path: &Path) -> io::Result<()> {
        let target = wide(path.as_os_str());
        let source = wide(temp.as_os_str());
        // SAFETY: both names are NUL-terminated; the optional pointers are
        // null as documented. `TempPath` holds only the name, so the
        // temporary file's handle is already closed.
        let ok = unsafe {
            ReplaceFileW(
                target.as_ptr(),
                source.as_ptr(),
                ptr::null(),
                REPLACEFILE_IGNORE_MERGE_ERRORS,
                ptr::null(),
                ptr::null(),
            )
        };
        if ok != 0 {
            // The temporary name no longer exists; nothing to clean up.
            drop(temp.keep());
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) {
            return temp.persist(path).map_err(|err| err.error);
        }
        Err(err)
    }

    /// Windows has no folder fsync; `MoveFileExW` already replaced the file.
    pub(super) fn sync_dir(_dir: &Path) {}
}

#[cfg(test)]
#[path = "fsx_tests.rs"]
pub(crate) mod tests;
