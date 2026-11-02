//! Per-OS per-user data-directory resolution (local-mode Req 3.1; design.md
//! "Local dirs resolver").
//!
//! Pure functions over an injected env lookup (the config module's
//! established hermetic-test pattern): Linux `$XDG_DATA_HOME` else
//! `$HOME/.local/share`, Windows `%APPDATA%`, macOS `$HOME/Library/
//! Application Support` — each joined with the fixed `mmhmem` folder. A
//! missing base variable is a fail-fast config error naming the variable;
//! local mode never guesses a fallback location (Req 2.5 spirit).

use std::path::{Path, PathBuf};

use super::present;
use super::ConfigError;

/// Folder name under the per-user application-data location (Req 3.1).
pub const APP_DIR_NAME: &str = "mmhmem";
/// Audit file name inside the per-user data dir (local profile default,
/// Req 7.1: `<data dir>/audit.jsonl`).
pub const AUDIT_FILE_NAME: &str = "audit.jsonl";
/// Worker-config file name for the local profile (corpus-warmup-prune
/// Req 7): `~/.mmhmem.json` globally and `./.mmhmem.json` per project.
pub const CONFIG_FILE_NAME: &str = ".mmhmem.json";

/// Resolves the per-user data dir for the current operating system
/// (Req 3.1). The three platform resolvers are plain pure functions so
/// every branch is unit-testable on any host; the CI matrix runs the
/// suite on all three operating systems (Req 10.3).
pub fn local_data_dir(get: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, ConfigError> {
    #[cfg(windows)]
    {
        windows_data_dir(get)
    }
    #[cfg(target_os = "macos")]
    {
        macos_data_dir(get)
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        linux_data_dir(get)
    }
}

/// Linux: `$XDG_DATA_HOME` when set, else `$HOME/.local/share`
/// (XDG Base Directory Specification fallback).
pub fn linux_data_dir(get: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, ConfigError> {
    if let Some(xdg) = present(get, "XDG_DATA_HOME") {
        return Ok(PathBuf::from(xdg).join(APP_DIR_NAME));
    }
    match present(get, "HOME") {
        Some(home) => Ok(PathBuf::from(home).join(".local/share").join(APP_DIR_NAME)),
        None => Err(ConfigError::MissingUserDirEnv("HOME")),
    }
}

/// Windows: the standard per-user application-data variable `%APPDATA%`.
pub fn windows_data_dir(get: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, ConfigError> {
    match present(get, "APPDATA") {
        Some(appdata) => Ok(PathBuf::from(appdata).join(APP_DIR_NAME)),
        None => Err(ConfigError::MissingUserDirEnv("APPDATA")),
    }
}

/// macOS: the conventional `~/Library/Application Support`.
pub fn macos_data_dir(get: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, ConfigError> {
    match present(get, "HOME") {
        Some(home) => Ok(PathBuf::from(home)
            .join("Library/Application Support")
            .join(APP_DIR_NAME)),
        None => Err(ConfigError::MissingUserDirEnv("HOME")),
    }
}

/// Default local audit file (Req 7.1): `<data dir>/audit.jsonl`.
pub fn local_audit_file(data_dir: &Path) -> PathBuf {
    data_dir.join(AUDIT_FILE_NAME)
}

/// Global worker-config file (corpus-warmup-prune Req 7.1): the per-user
/// home directory joined with `.mmhmem.json`. `None` when the home
/// variable is unset — the file is optional, so a missing home means "no
/// global file", never a startup failure (Req 7.2: absent config must
/// behave exactly as before). Dispatches per OS like [`local_data_dir`]
/// so every branch is unit-testable on any host.
pub fn global_config_file(get: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        windows_global_config_file(get)
    }
    #[cfg(not(windows))]
    {
        unix_global_config_file(get)
    }
}

/// Windows: `%USERPROFILE%\.mmhmem.json`, falling back to `HOME` when only
/// the POSIX variable is present.
pub fn windows_global_config_file(get: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(home) = present(get, "USERPROFILE") {
        return Some(PathBuf::from(home).join(CONFIG_FILE_NAME));
    }
    present(get, "HOME").map(|home| PathBuf::from(home).join(CONFIG_FILE_NAME))
}

/// Linux/macOS: `$HOME/.mmhmem.json`.
pub fn unix_global_config_file(get: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    present(get, "HOME").map(|home| PathBuf::from(home).join(CONFIG_FILE_NAME))
}

/// Per-project worker-config file (Req 7.1): `./.mmhmem.json`, resolved
/// against the process working directory.
pub fn project_config_file() -> PathBuf {
    PathBuf::from(CONFIG_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn get(map: &HashMap<String, String>) -> impl Fn(&str) -> Option<String> + '_ {
        move |k| map.get(k).cloned()
    }

    fn expect_path(res: Result<PathBuf, ConfigError>) -> PathBuf {
        res.unwrap_or_else(|e| panic!("expected a path, got error: {e}"))
    }

    #[test]
    fn linux_xdg_data_home_wins_when_set() {
        let map = lookup(&[("XDG_DATA_HOME", "/xdg-data"), ("HOME", "/home/u")]);
        let dir = expect_path(linux_data_dir(&get(&map)));
        assert_eq!(dir, PathBuf::from("/xdg-data").join(APP_DIR_NAME));
    }

    #[test]
    fn linux_home_fallback_under_local_share() {
        let map = lookup(&[("HOME", "/home/u")]);
        let dir = expect_path(linux_data_dir(&get(&map)));
        assert_eq!(
            dir,
            PathBuf::from("/home/u/.local/share").join(APP_DIR_NAME)
        );
    }

    #[test]
    fn linux_missing_home_is_a_fail_fast_error_naming_home() {
        let map = lookup(&[]);
        let err = linux_data_dir(&get(&map)).unwrap_err();
        assert_eq!(err, ConfigError::MissingUserDirEnv("HOME"));
        assert!(
            err.to_string().contains("HOME"),
            "error must name HOME: {err}"
        );
    }

    #[test]
    fn windows_appdata_resolves() {
        let map = lookup(&[("APPDATA", r"C:\Users\u\AppData\Roaming")]);
        let dir = expect_path(windows_data_dir(&get(&map)));
        assert_eq!(
            dir,
            PathBuf::from(r"C:\Users\u\AppData\Roaming").join(APP_DIR_NAME)
        );
    }

    #[test]
    fn windows_missing_appdata_is_a_fail_fast_error_naming_appdata() {
        let map = lookup(&[]);
        let err = windows_data_dir(&get(&map)).unwrap_err();
        assert_eq!(err, ConfigError::MissingUserDirEnv("APPDATA"));
        assert!(
            err.to_string().contains("APPDATA"),
            "error must name APPDATA: {err}"
        );
    }

    #[test]
    fn macos_home_resolves_library_application_support() {
        let map = lookup(&[("HOME", "/Users/u")]);
        let dir = expect_path(macos_data_dir(&get(&map)));
        assert_eq!(
            dir,
            PathBuf::from("/Users/u/Library/Application Support").join(APP_DIR_NAME)
        );
    }

    #[test]
    fn macos_missing_home_is_a_fail_fast_error_naming_home() {
        let map = lookup(&[]);
        assert_eq!(
            macos_data_dir(&get(&map)).unwrap_err(),
            ConfigError::MissingUserDirEnv("HOME")
        );
    }

    #[test]
    fn blank_values_are_treated_as_missing() {
        let map = lookup(&[("XDG_DATA_HOME", "   "), ("HOME", "  ")]);
        assert_eq!(
            linux_data_dir(&get(&map)).unwrap_err(),
            ConfigError::MissingUserDirEnv("HOME")
        );
    }

    #[test]
    fn audit_file_joins_data_dir() {
        let dir = PathBuf::from("/tmp/data");
        assert_eq!(local_audit_file(&dir), dir.join(AUDIT_FILE_NAME));
    }

    #[test]
    fn dispatcher_selects_the_current_os_resolver() {
        let map = lookup(&[
            ("HOME", "/home/u"),
            ("APPDATA", r"C:\Roam"),
            ("XDG_DATA_HOME", "/xdg"),
        ]);
        let get = get(&map);
        let via_dispatch = expect_path(local_data_dir(&get));
        let direct = if cfg!(windows) {
            expect_path(windows_data_dir(&get))
        } else if cfg!(target_os = "macos") {
            expect_path(macos_data_dir(&get))
        } else {
            expect_path(linux_data_dir(&get))
        };
        assert_eq!(via_dispatch, direct);
    }

    #[test]
    fn windows_global_config_file_prefers_userprofile() {
        let map = lookup(&[
            ("USERPROFILE", r"C:\Users\u"),
            ("HOME", r"C:\Users\u\posix"),
        ]);
        let path = windows_global_config_file(&get(&map)).expect("home set");
        assert_eq!(path, PathBuf::from(r"C:\Users\u").join(CONFIG_FILE_NAME));
    }

    #[test]
    fn windows_global_config_file_falls_back_to_home() {
        let map = lookup(&[("HOME", r"C:\Users\u")]);
        let path = windows_global_config_file(&get(&map)).expect("home set");
        assert_eq!(path, PathBuf::from(r"C:\Users\u").join(CONFIG_FILE_NAME));
    }

    #[test]
    fn windows_global_config_file_missing_env_is_none_not_an_error() {
        // Req 7.2: the global config file is optional — a missing home
        // variable means "no global file", never a startup failure.
        let map = lookup(&[]);
        assert_eq!(windows_global_config_file(&get(&map)), None);
    }

    #[test]
    fn unix_global_config_file_joins_home() {
        let map = lookup(&[("HOME", "/home/u")]);
        let path = unix_global_config_file(&get(&map)).expect("home set");
        assert_eq!(path, PathBuf::from("/home/u").join(CONFIG_FILE_NAME));
    }

    #[test]
    fn unix_global_config_file_missing_home_is_none() {
        let map = lookup(&[]);
        assert_eq!(unix_global_config_file(&get(&map)), None);
    }

    #[test]
    fn project_config_file_is_dot_mmhmem_json() {
        assert_eq!(project_config_file(), PathBuf::from(".mmhmem.json"));
    }

    #[test]
    fn global_config_file_dispatcher_matches_current_os() {
        let map = lookup(&[("HOME", "/home/u"), ("USERPROFILE", r"C:\Users\u")]);
        let get = get(&map);
        let via_dispatch = global_config_file(&get).expect("home set");
        let direct = if cfg!(windows) {
            windows_global_config_file(&get).expect("home set")
        } else {
            unix_global_config_file(&get).expect("home set")
        };
        assert_eq!(via_dispatch, direct);
    }
}
