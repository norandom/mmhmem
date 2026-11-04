//! The program's folder on the user's `PATH` (reliable-installers task 7.1,
//! `.kiro/specs/reliable-installers/design.md` §1 and §3; Requirements 7.1,
//! 7.2 and 5.1).
//!
//! **Windows:** the current user's `PATH` lives in
//! `HKCU\Environment\Path`. Setup adds the program's folder there as a
//! reviewed, recorded and reversible change ([`Target::PathEntry`]):
//! the folder is appended only when it is absent, the value keeps its type
//! (`REG_EXPAND_SZ` or `REG_SZ`) and every other byte, and running programs
//! are told with `WM_SETTINGCHANGE`. A rollback writes the value back
//! exactly as it was read.
//!
//! **Linux and macOS:** setup never edits shell start-up files. When
//! `~/.local/bin` is not on `PATH`, the results say how to add it
//! ([`path_hint`]) and the record says `path_entry.added = false`.
//!
//! The registry is reached only through [`UserPathStore`], so tests use
//! [`FakeUserPath`] and never touch the real registry.
//!
//! A planned `PATH` change carries the whole value as bytes
//! ([`PathValue::encode`]): one type byte, then the raw registry data
//! (UTF-16LE with its terminating NULs).
//!
//! [`Target::PathEntry`]: crate::setup::record::Target::PathEntry

use std::path::{Path, PathBuf};

/// How the registry value is typed (kept as it was found).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// `REG_EXPAND_SZ`: `%VARIABLES%` are expanded (the Windows default).
    Expand,
    /// `REG_SZ`: plain text.
    Plain,
}

/// The user's `PATH` value exactly as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathValue {
    /// The value type.
    pub kind: ValueKind,
    /// The raw data: UTF-16LE, usually ending in a NUL.
    pub data: Vec<u8>,
}

const EXPAND_BYTE: u8 = b'E';
const PLAIN_BYTE: u8 = b'S';

impl PathValue {
    /// The value as change bytes: one type byte, then the raw data.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![match self.kind {
            ValueKind::Expand => EXPAND_BYTE,
            ValueKind::Plain => PLAIN_BYTE,
        }];
        out.extend_from_slice(&self.data);
        out
    }

    /// The value from [`PathValue::encode`] bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let kind = match bytes.first() {
            Some(&EXPAND_BYTE) => ValueKind::Expand,
            Some(&PLAIN_BYTE) => ValueKind::Plain,
            _ => return Err("the PATH setting has an unknown format".to_owned()),
        };
        let data = bytes[1..].to_vec();
        if !data.len().is_multiple_of(2) {
            return Err("the PATH setting is not valid text".to_owned());
        }
        Ok(Self { kind, data })
    }

    /// A new value holding only `folder`.
    pub fn new_with(folder: &Path) -> Self {
        let mut units = utf16(&folder.to_string_lossy());
        units.push(0);
        Self {
            kind: ValueKind::Expand,
            data: to_bytes(&units),
        }
    }

    fn units(&self) -> Vec<u16> {
        self.data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect()
    }

    /// The folders in the value, in order (empty parts left out).
    pub fn entries(&self) -> Vec<PathBuf> {
        let units = self.units();
        let text = String::from_utf16_lossy(trim_nul(&units));
        text.split(';')
            .filter(|p| !p.trim().is_empty())
            .map(PathBuf::from)
            .collect()
    }

    /// Whether `folder` is one of the entries.
    pub fn contains(&self, folder: &Path) -> bool {
        self.entries().iter().any(|p| same_folder(p, folder))
    }

    /// The value with `folder` appended (unchanged when it is already
    /// there). Every other byte, the terminating NULs and the type stay.
    pub fn with_folder(&self, folder: &Path) -> Self {
        if self.contains(folder) {
            return self.clone();
        }
        let units = self.units();
        let text = trim_nul(&units);
        let tail = &units[text.len()..];
        let mut out = text.to_vec();
        if !text.is_empty() && text.last() != Some(&u16::from(b';')) {
            out.push(u16::from(b';'));
        }
        out.extend(utf16(&folder.to_string_lossy()));
        if tail.is_empty() {
            out.push(0);
        } else {
            out.extend_from_slice(tail);
        }
        Self {
            kind: self.kind,
            data: to_bytes(&out),
        }
    }
}

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn to_bytes(units: &[u16]) -> Vec<u8> {
    units.iter().flat_map(|u| u.to_le_bytes()).collect()
}

fn trim_nul(units: &[u16]) -> &[u16] {
    let end = units.iter().rposition(|u| *u != 0).map_or(0, |i| i + 1);
    &units[..end]
}

/// Compare folders the way `PATH` lookups do: trailing separators are
/// ignored, and ASCII case on Windows.
pub fn same_folder(a: &Path, b: &Path) -> bool {
    let trim = |p: &Path| p.to_string_lossy().trim_end_matches(['/', '\\']).to_owned();
    let (a, b) = (trim(a), trim(b));
    if cfg!(windows) {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

/// Whether `folder` is in `dirs`.
pub fn on_path(dirs: &[PathBuf], folder: &Path) -> bool {
    dirs.iter().any(|d| same_folder(d, folder))
}

/// Linux and macOS: the line telling the user how to put `folder` on
/// `PATH`, or `None` when it is already there.
pub fn path_hint(dirs: &[PathBuf], folder: &Path, home: &Path) -> Option<String> {
    if on_path(dirs, folder) {
        return None;
    }
    let shown = folder.strip_prefix(home).map_or_else(
        |_| folder.display().to_string(),
        |rel| format!("$HOME/{}", rel.to_string_lossy().replace('\\', "/")),
    );
    Some(crate::setup::ui::text::path_hint(&shown))
}

/// Reads and writes the current user's `PATH` value.
pub trait UserPathStore: Send + Sync {
    /// Whether this store can change the user's `PATH` (Windows only).
    fn supported(&self) -> bool;
    /// The value now (`None`: there is no user `PATH` value).
    fn read(&self) -> Result<Option<PathValue>, String>;
    /// Store exactly `value` (`None`: delete the value) and tell running
    /// programs.
    fn write(&self, value: Option<&PathValue>) -> Result<(), String>;
}

/// No user `PATH` store (Linux, macOS, and plans without a `PATH` change).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoUserPath;

const NOT_SUPPORTED: &str = "the PATH setting cannot be changed on this computer";

impl UserPathStore for NoUserPath {
    fn supported(&self) -> bool {
        false
    }

    fn read(&self) -> Result<Option<PathValue>, String> {
        Err(NOT_SUPPORTED.to_owned())
    }

    fn write(&self, _value: Option<&PathValue>) -> Result<(), String> {
        Err(NOT_SUPPORTED.to_owned())
    }
}

/// The real store of this process: the registry on Windows, none elsewhere.
pub fn process_store() -> std::sync::Arc<dyn UserPathStore> {
    #[cfg(windows)]
    {
        std::sync::Arc::new(registry::RegistryPath)
    }
    #[cfg(not(windows))]
    {
        std::sync::Arc::new(NoUserPath)
    }
}

#[cfg(windows)]
mod registry {
    //! `HKCU\Environment\Path` through `winreg`. Only `mmhmem setup` itself
    //! uses this; tests never do.

    use winreg::RegKey;
    use winreg::RegValue;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ};

    use super::{PathValue, UserPathStore, ValueKind};

    const KEY: &str = "Environment";
    const NAME: &str = "Path";

    pub(super) struct RegistryPath;

    fn reason(e: &std::io::Error) -> String {
        format!(
            "the PATH setting could not be read or changed ({})",
            e.kind()
        )
    }

    impl UserPathStore for RegistryPath {
        fn supported(&self) -> bool {
            true
        }

        fn read(&self) -> Result<Option<PathValue>, String> {
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let key = match hkcu.open_subkey_with_flags(KEY, KEY_READ) {
                Ok(key) => key,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(reason(&e)),
            };
            let value = match key.get_raw_value(NAME) {
                Ok(value) => value,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(reason(&e)),
            };
            let kind = match value.vtype {
                REG_EXPAND_SZ => ValueKind::Expand,
                REG_SZ => ValueKind::Plain,
                _ => return Err("the PATH setting has an unexpected type".to_owned()),
            };
            Ok(Some(PathValue {
                kind,
                data: value.bytes.into_owned(),
            }))
        }

        fn write(&self, value: Option<&PathValue>) -> Result<(), String> {
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let (key, _) = hkcu
                .create_subkey_with_flags(KEY, KEY_READ | KEY_WRITE)
                .map_err(|e| reason(&e))?;
            match value {
                Some(v) => {
                    let raw = RegValue {
                        bytes: std::borrow::Cow::Borrowed(&v.data),
                        vtype: match v.kind {
                            ValueKind::Expand => REG_EXPAND_SZ,
                            ValueKind::Plain => REG_SZ,
                        },
                    };
                    key.set_raw_value(NAME, &raw).map_err(|e| reason(&e))?;
                }
                None => match key.delete_value(NAME) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(reason(&e));
                    }
                    _ => {}
                },
            }
            broadcast();
            Ok(())
        }
    }

    /// Tell running programs (Explorer, new terminals) that the environment
    /// changed. Waits at most a second per window.
    fn broadcast() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
        };
        let area: Vec<u16> = "Environment".encode_utf16().chain([0]).collect();
        let mut result = 0usize;
        // SAFETY: `area` is a NUL-terminated UTF-16 string that outlives the
        // call; `result` is a valid out pointer.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                area.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                1000,
                &mut result,
            );
        }
    }
}

/// An in-memory user `PATH` for tests.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct FakeUserPath {
    value: std::sync::Mutex<Option<PathValue>>,
    /// Refuse every write (to test a failing `PATH` change).
    pub(crate) fail_writes: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
impl FakeUserPath {
    pub(crate) fn with(value: Option<PathValue>) -> Self {
        Self {
            value: std::sync::Mutex::new(value),
            fail_writes: Default::default(),
        }
    }

    pub(crate) fn get(&self) -> Option<PathValue> {
        self.value.lock().expect("path lock").clone()
    }
}

#[cfg(test)]
impl UserPathStore for FakeUserPath {
    fn supported(&self) -> bool {
        true
    }

    fn read(&self) -> Result<Option<PathValue>, String> {
        Ok(self.get())
    }

    fn write(&self, value: Option<&PathValue>) -> Result<(), String> {
        if self.fail_writes.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("the PATH setting could not be changed (test)".to_owned());
        }
        *self.value.lock().expect("path lock") = value.cloned();
        Ok(())
    }
}

#[cfg(test)]
#[path = "user_path_tests.rs"]
mod tests;
