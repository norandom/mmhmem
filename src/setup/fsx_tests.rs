//! Tests for `setup::fsx` (task 2.1): atomic replace, interrupted writes,
//! byte-identical backups and owner-only files read back per OS.

use super::*;

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn write_atomic_writes_exact_bytes_and_creates_parents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a").join("b").join("config.toml");
    let bytes = b"x = 1\r\n\0\xff tail";
    write_atomic(&path, bytes).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn write_atomic_replaces_existing_file_without_leftovers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path, b"old old old old").unwrap();
    write_atomic(&path, b"new").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"new");
    assert_eq!(names_in(dir.path()), vec!["settings.json"]);
}

#[test]
fn interrupted_write_before_rename_leaves_old_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, b"original").unwrap();
    for owner_only in [false, true] {
        let staged = stage(&path, b"half-written replacement", owner_only).unwrap();
        // Simulated crash: the temp file exists but is never renamed.
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        drop(staged);
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(names_in(dir.path()), vec!["config.toml"]);
    }
}

#[test]
fn failed_rename_leaves_target_and_names_path() {
    let dir = tempfile::tempdir().unwrap();
    // A non-empty folder at the target path makes the rename fail.
    let path = dir.path().join("blocked");
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("keep.txt"), b"keep").unwrap();
    let err = write_atomic(&path, b"new").unwrap_err();
    assert!(format!("{err:#}").contains("blocked"), "{err:#}");
    assert_eq!(std::fs::read(path.join("keep.txt")).unwrap(), b"keep");
    assert_eq!(names_in(dir.path()), vec!["blocked"]);
}

#[test]
fn backup_is_byte_identical_and_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("config.toml");
    let bytes: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
    std::fs::write(&source, &bytes).unwrap();
    let backups = dir.path().join("backups").join("2026-11-04T09-30-00Z");

    let first = backup(&source, &backups).unwrap().unwrap();
    assert_eq!(first, backups.join("config.toml"));
    assert_eq!(std::fs::read(&first).unwrap(), bytes);

    std::fs::write(&source, b"second").unwrap();
    let second = backup(&source, &backups).unwrap().unwrap();
    assert_eq!(second, backups.join("1-config.toml"));
    assert_eq!(std::fs::read(&second).unwrap(), b"second");
    assert_eq!(std::fs::read(&first).unwrap(), bytes);
}

#[test]
fn backup_of_missing_file_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let backups = dir.path().join("backups");
    let result = backup(&dir.path().join("absent.json"), &backups).unwrap();
    assert!(result.is_none());
    assert!(!backups.exists(), "no backup folder for nothing to back up");
}

#[test]
fn create_owner_only_refuses_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("key");
    std::fs::write(&path, b"existing").unwrap();
    let err = create_owner_only(&path).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&path).unwrap(), b"existing");
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn new_owner_only_file_is_0600_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("worker.key");
        let _file = create_owner_only(&path).unwrap();
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn owner_only_write_over_broader_file_is_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("worker.key");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_owner_only(&path, b"secret").unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(std::fs::read(&path).unwrap(), b"secret");
    }

    #[test]
    fn write_atomic_keeps_existing_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(mode(&path), 0o644);
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn backup_is_0600() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("config.toml");
        std::fs::write(&source, b"x").unwrap();
        let copy = backup(&source, &dir.path().join("b")).unwrap().unwrap();
        assert_eq!(mode(&copy), 0o600);
    }
}

#[cfg(windows)]
pub(crate) mod windows {
    use super::super::imp::{CurrentUser, sid_to_string};
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use std::process::Command;
    use std::ptr;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        DACL_SECURITY_INFORMATION, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;

    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const EVERYONE: &str = "S-1-1-0";

    #[derive(Debug, PartialEq)]
    struct Ace {
        sid: String,
        kind: u8,
        flags: u8,
        mask: u32,
    }

    #[derive(Debug, PartialEq)]
    struct Security {
        owner: String,
        protected: bool,
        aces: Vec<Ace>,
    }

    /// Frees the descriptor from GetSecurityInfo, also when an assert fails.
    struct Descriptor(PSECURITY_DESCRIPTOR);

    impl Drop for Descriptor {
        fn drop(&mut self) {
            // SAFETY: the descriptor came from GetSecurityInfo; freed once.
            unsafe { LocalFree(self.0) };
        }
    }

    /// Read the file's owner and DACL back from the file system.
    fn read_security(path: &Path) -> Security {
        let file = File::open(path).unwrap();
        let (mut owner, mut dacl): (PSID, *mut ACL) = (ptr::null_mut(), ptr::null_mut());
        let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: valid file handle; out pointers are valid; `owner` and
        // `dacl` point into `sd`, which `Descriptor` frees after use.
        let status = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut sd,
            )
        };
        assert_eq!(status, ERROR_SUCCESS);
        let sd = Descriptor(sd);
        assert!(!dacl.is_null(), "file has a NULL DACL (everyone allowed)");
        let (mut control, mut revision) = (0u16, 0u32);
        // SAFETY: `sd.0` is a valid descriptor from GetSecurityInfo.
        let ok = unsafe { GetSecurityDescriptorControl(sd.0, &mut control, &mut revision) };
        assert_ne!(ok, 0);
        let mut info = ACL_SIZE_INFORMATION {
            AceCount: 0,
            AclBytesInUse: 0,
            AclBytesFree: 0,
        };
        let size = size_of::<ACL_SIZE_INFORMATION>() as u32;
        // SAFETY: `dacl` is valid; `info` is writable for `size` bytes.
        let ok = unsafe {
            GetAclInformation(
                dacl,
                ptr::from_mut(&mut info).cast(),
                size,
                AclSizeInformation,
            )
        };
        assert_ne!(ok, 0);
        Security {
            // SAFETY: `owner` points into the live descriptor `sd`.
            owner: unsafe { sid_to_string(owner) }.unwrap(),
            protected: control & SE_DACL_PROTECTED != 0,
            aces: (0..info.AceCount).map(|i| read_ace(dacl, i)).collect(),
        }
    }

    /// One allow or deny ACE (both share the `ACCESS_ALLOWED_ACE` layout).
    fn read_ace(dacl: *const ACL, index: u32) -> Ace {
        let mut raw = ptr::null_mut();
        // SAFETY: `dacl` is valid and `index < AceCount`.
        assert_ne!(unsafe { GetAce(dacl, index, &mut raw) }, 0);
        // SAFETY: GetAce returned a pointer to an ACE inside the DACL; the
        // type is checked before the mask and SID are used.
        let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
        assert!(
            ace.Header.AceType <= 1,
            "unexpected ACE type {}",
            ace.Header.AceType
        );
        let sid = ptr::from_ref(&ace.SidStart).cast_mut().cast();
        Ace {
            // SAFETY: the SID starts at `SidStart` inside the live DACL.
            sid: unsafe { sid_to_string(sid) }.unwrap(),
            kind: ace.Header.AceType,
            flags: ace.Header.AceFlags,
            mask: ace.Mask,
        }
    }

    /// Assert: owner is the current user; protected DACL with exactly one
    /// non-inherited allow ACE, full access, for the current user.
    pub(crate) fn assert_current_user_only(path: &Path) {
        let user = CurrentUser::query().unwrap().sid_string().unwrap();
        let security = read_security(path);
        assert!(security.protected, "DACL must be protected: {security:?}");
        assert_eq!(security.owner, user, "owner must be the current user");
        let only = Ace {
            sid: user,
            kind: ACCESS_ALLOWED_ACE_TYPE,
            flags: 0,
            mask: FILE_ALL_ACCESS,
        };
        assert_eq!(security.aces, vec![only]);
    }

    #[test]
    fn new_owner_only_file_is_current_user_only_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("worker.key");
        let _file = create_owner_only(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        assert_current_user_only(&path);
    }

    /// Negative control: the read-back check rejects an ordinary file.
    #[test]
    #[should_panic(expected = "DACL must be protected")]
    fn acl_check_rejects_plain_inherited_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.txt");
        std::fs::write(&path, b"x").unwrap();
        assert_current_user_only(&path);
    }

    #[test]
    fn owner_only_write_over_inherited_file_is_current_user_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("worker.key");
        std::fs::write(&path, b"old").unwrap();
        write_owner_only(&path, b"secret").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"secret");
        assert_current_user_only(&path);
    }

    #[test]
    fn write_atomic_keeps_extra_explicit_ace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"old").unwrap();
        let status = Command::new("icacls")
            .arg(&path)
            .args(["/grant", &format!("*{EVERYONE}:(R)")])
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "icacls failed");
        let before = read_security(&path);
        assert!(
            before
                .aces
                .iter()
                .any(|a| a.sid == EVERYONE && a.flags == 0)
        );

        write_atomic(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(read_security(&path), before);
        assert_eq!(names_in(dir.path()), vec!["settings.json"]);
    }

    #[test]
    fn backup_is_current_user_only() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("config.toml");
        std::fs::write(&source, b"x").unwrap();
        let copy = backup(&source, &dir.path().join("b")).unwrap().unwrap();
        assert_current_user_only(&copy);
    }
}
