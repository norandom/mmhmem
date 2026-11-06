//! Whether another process runs a given program (reliable-installers
//! Requirement 7.3: a removal must tell the truth about the program).
//!
//! Windows cannot delete a program file while any process runs it. When the
//! removal is started from the installed program itself, the delete is
//! deferred until setup exits (`self_replace`, see `plan_apply.rs`); that
//! deferred delete only works when no *other* process (an AI tool's
//! `mmhmem stdio`, a second window) still runs the program, and it cannot
//! report a failure. So the removal asks first: [`used_by_other_process`]
//! lists the running processes and compares their program paths with the
//! installed one, leaving out this process. Processes this user may not
//! inspect are not seen.
//!
//! On Unix a running program's file can be deleted at once, so the answer
//! is always `false` there.

use std::path::Path;

/// True when a process other than this one runs the program at `program`.
pub fn used_by_other_process(program: &Path) -> bool {
    imp::used_by_other_process(program)
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    pub(super) fn used_by_other_process(_program: &Path) -> bool {
        false
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };

    /// Closes a handle when dropped.
    struct Owned(HANDLE);

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful Win32 call and is
            // closed exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }

    pub(super) fn used_by_other_process(program: &Path) -> bool {
        let Ok(wanted) = std::fs::canonicalize(program) else {
            return false;
        };
        let Some(name) = wanted
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
        else {
            return false;
        };
        // SAFETY: plain call; the result is checked before use.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return false;
        }
        let snapshot = Owned(snapshot);
        // SAFETY: plain call.
        let me = unsafe { GetCurrentProcessId() };
        // SAFETY: an all-zero PROCESSENTRY32W is valid; dwSize is set below.
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        // SAFETY: `snapshot` is a valid snapshot handle, `entry` a valid out
        // pointer with its size set.
        let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
        while more {
            let pid = entry.th32ProcessID;
            if pid != me
                && exe_name(&entry.szExeFile) == name
                && image_path(pid).is_some_and(|p| same(&p, &wanted))
            {
                return true;
            }
            // SAFETY: as for Process32FirstW.
            more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
        }
        false
    }

    /// The lower-case file name in a NUL-terminated UTF-16 buffer.
    fn exe_name(raw: &[u16]) -> String {
        let len = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
        String::from_utf16_lossy(&raw[..len]).to_lowercase()
    }

    /// The full program path of process `pid`, if this user may ask.
    fn image_path(pid: u32) -> Option<PathBuf> {
        // SAFETY: plain call; a null handle means no access.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let handle = Owned(handle);
        let mut buf = vec![0u16; 32 * 1024];
        let mut len = buf.len() as u32;
        // SAFETY: `buf` holds `len` u16s; `len` is a valid in/out pointer.
        let ok = unsafe {
            QueryFullProcessImageNameW(handle.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
        };
        (ok != 0).then(|| PathBuf::from(OsString::from_wide(&buf[..len as usize])))
    }

    /// Same file, compared through the canonical path (case-insensitive,
    /// like the file system).
    fn same(running: &Path, wanted: &Path) -> bool {
        std::fs::canonicalize(running).is_ok_and(|p| {
            p.to_string_lossy().to_lowercase() == wanted.to_string_lossy().to_lowercase()
        })
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::used_by_other_process;
    use std::process::{Command, Stdio};

    /// A copy of `cmd.exe` named `mmhmem.exe` waits on its open stdin; while
    /// it runs, its file is in use by another process, and after it ends it
    /// is not. This test process itself never counts.
    #[test]
    fn a_running_copy_is_seen_and_this_process_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let copy = dir.path().join("mmhmem.exe");
        let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        std::fs::copy(format!(r"{system}\System32\cmd.exe"), &copy).expect("copy cmd.exe");
        assert!(!used_by_other_process(&copy));
        let mut child = Command::new(&copy)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the copy");
        let seen = (0..50).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            used_by_other_process(&copy)
        });
        child.kill().expect("kill");
        child.wait().expect("wait");
        assert!(seen, "the running copy was not seen");
        assert!(!used_by_other_process(&copy));
        let me = std::env::current_exe().expect("current exe");
        assert!(!used_by_other_process(&me));
    }
}
