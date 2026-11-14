//! Worker settings from `.mmhmem.json` (corpus-warmup-prune Req 7.1-7.5;
//! design.md "config/mmhmem_file.rs").
//!
//! The local profile reads worker settings from two optional JSON files:
//! the per-project `./.mmhmem.json` and the global `~/.mmhmem.json`.
//! Every field is optional; missing files and absent settings are normal
//! (Req 7.2). A file that exists but is not valid JSON fails fast naming
//! the offending file (Req 7.3). Per-setting precedence — process
//! environment > project file > global file > built-in defaults (Req 7.1)
//! — is realized by composing a merged lookup closure over the unchanged
//! pure parser `Config::from_env_local` (research.md F2). The serve
//! profile never touches this module (Req 7.4). The worker API key from
//! any source never appears in logs or diagnostics (Req 7.5) — redaction
//! is source-agnostic in `Config::redacted_debug`. Two non-worker keys
//! are read too: `update_check` (`false` turns off the local-mode update
//! notice, see `services::update_check`; it is not part of `Config`) and
//! `shunt_content_cache_max_rows` (the content-cache row cap,
//! shunt-content-tool Req 4.3, surfaced through the shared parser).

use std::path::Path;

use serde::Deserialize;

use super::local_dirs;
use super::{
    ConfigError, ENV_SHUNT_CONTENT_CACHE_MAX_ROWS, ENV_WORKER_API_KEY, ENV_WORKER_API_URL,
    ENV_WORKER_MODEL,
};

/// Worker settings from `.mmhmem.json` — every field optional (Req 7.2).
/// Unknown JSON fields are ignored, so installers may extend the file
/// without breaking older binaries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct MmhmemFile {
    /// `worker_model` — the worker model id (Req 7.1).
    pub worker_model: Option<String>,
    /// `worker_api_url` — OpenAI-compatible chat-completions endpoint.
    pub worker_api_url: Option<String>,
    /// `worker_api_key` — worker bearer key (never logged, Req 7.5).
    pub worker_api_key: Option<String>,
    /// `update_check` — `false` turns off the session-start update notice
    /// and its once-a-day release lookup (local mode only).
    pub update_check: Option<bool>,
    /// `shunt_content_cache_max_rows` — content-cache row cap (u32 > 0,
    /// shunt-content-tool Req 4.3).
    pub shunt_content_cache_max_rows: Option<u32>,
}

impl MmhmemFile {
    /// Loads project-over-global from the two well-known file locations
    /// (local profile only, Req 7.1). Missing files → defaults (Req 7.2);
    /// a missing home variable means no global file, never a failure.
    pub fn load_default_locations() -> Result<Self, ConfigError> {
        let get = |key: &str| std::env::var(key).ok();
        let global = local_dirs::global_config_file(&get).unwrap_or_default();
        Self::load(&global, &local_dirs::project_config_file())
    }

    /// Loads project-over-global from explicit paths; per setting, the
    /// project file wins over the global file (Req 7.1).
    pub fn load(global: &Path, project: &Path) -> Result<Self, ConfigError> {
        let g = Self::read_one(global)?;
        let p = Self::read_one(project)?;
        Ok(Self {
            worker_model: p.worker_model.or(g.worker_model),
            worker_api_url: p.worker_api_url.or(g.worker_api_url),
            worker_api_key: p.worker_api_key.or(g.worker_api_key),
            update_check: p.update_check.or(g.update_check),
            shunt_content_cache_max_rows: p
                .shunt_content_cache_max_rows
                .or(g.shunt_content_cache_max_rows),
        })
    }

    /// Reads one optional file; missing → defaults, unreadable or invalid
    /// JSON → fail fast naming the file (Req 7.3). An empty path means
    /// "no location" (unset home variable) and reads as absent (Req 7.2).
    /// A UTF-8 BOM prefix is tolerated (hand-edited Windows files).
    fn read_one(path: &Path) -> Result<Self, ConfigError> {
        if path.as_os_str().is_empty() {
            return Ok(Self::default());
        }
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
                serde_json::from_str(text)
                    .map_err(|_| ConfigError::InvalidMmhmemFile(path.to_path_buf()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(_) => Err(ConfigError::InvalidMmhmemFile(path.to_path_buf())),
        }
    }

    /// File-sourced value for a file-backed env key (None for every other
    /// key — the file carries worker, update-check and cache-cap settings
    /// only).
    fn setting(&self, key: &str) -> Option<String> {
        if key == ENV_WORKER_MODEL {
            self.worker_model.clone()
        } else if key == ENV_WORKER_API_URL {
            self.worker_api_url.clone()
        } else if key == ENV_WORKER_API_KEY {
            self.worker_api_key.clone()
        } else if key == ENV_SHUNT_CONTENT_CACHE_MAX_ROWS {
            self.shunt_content_cache_max_rows.map(|n| n.to_string())
        } else {
            None
        }
    }

    /// Merged per-setting lookup for the local profile (Req 7.1): process
    /// environment first, then project file, then global file (the two
    /// files arrive pre-merged in `self`). Composed over the unchanged
    /// pure parser, so the serve profile and existing tests are untouched.
    pub fn merged_lookup<'a>(
        get: &'a dyn Fn(&str) -> Option<String>,
        file: &'a Self,
    ) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| get(key).or_else(|| file.setting(key))
    }

    /// Updates (or creates) `path` with `worker_model = model`, preserving any
    /// existing keys and setting `worker_api_url` to the default when absent.
    pub fn save_worker_model(path: &Path, model: &str) -> std::io::Result<()> {
        let mut obj = match std::fs::read_to_string(path) {
            Ok(text) => {
                let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(text)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::Map::new(),
            Err(e) => return Err(e),
        };
        obj.insert(
            "worker_model".to_owned(),
            serde_json::Value::String(model.to_owned()),
        );
        if !obj.contains_key("worker_api_url") {
            obj.insert(
                "worker_api_url".to_owned(),
                serde_json::Value::String(super::DEFAULT_WORKER_API_URL.to_owned()),
            );
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut json = serde_json::to_string_pretty(&serde_json::Value::Object(obj))
            .map_err(std::io::Error::other)?;
        json.push('\n');
        std::fs::write(path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Config, ENV_WORKER_API_URL, ENV_WORKER_MODEL};
    use super::*;
    use std::collections::HashMap;
    use std::fs;

    fn lookup(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn get(map: &HashMap<String, String>) -> impl Fn(&str) -> Option<String> + '_ {
        move |k| map.get(k).cloned()
    }

    /// The per-user location env base (same shape as profile.rs's helper)
    /// so `from_env_local` can resolve its default data dir.
    fn local_user_env(overrides: &[(&str, &str)]) -> HashMap<String, String> {
        let mut map = lookup(&[
            ("XDG_DATA_HOME", "/xdg-data"),
            ("HOME", "/home/u"),
            ("APPDATA", r"C:\Users\u\AppData\Roaming"),
        ]);
        for (k, v) in overrides {
            map.insert(k.to_string(), v.to_string());
        }
        map
    }

    /// Writes `json` to `dir/name` and returns the path.
    fn write_file(dir: &std::path::Path, name: &str, json: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, json).expect("write config file");
        path
    }

    #[test]
    fn load_absent_files_give_all_none() {
        // Req 7.2: no files anywhere → unchanged defaults.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = MmhmemFile::load(&dir.path().join("global"), &dir.path().join("project"))
            .expect("absent files are normal");
        assert_eq!(file, MmhmemFile::default());
    }

    #[test]
    fn load_merges_project_over_global_per_setting() {
        // Req 7.1: project wins per setting, global fills the gaps.
        let dir = tempfile::tempdir().expect("tempdir");
        let global = write_file(
            dir.path(),
            "global.json",
            r#"{"worker_model":"g-model","worker_api_key":"g-key"}"#,
        );
        let project = write_file(
            dir.path(),
            "project.json",
            r#"{"worker_model":"p-model","worker_api_url":"p-url"}"#,
        );
        let file = MmhmemFile::load(&global, &project).expect("valid files");
        assert_eq!(file.worker_model.as_deref(), Some("p-model"));
        assert_eq!(file.worker_api_url.as_deref(), Some("p-url"));
        assert_eq!(file.worker_api_key.as_deref(), Some("g-key"));
    }

    #[test]
    fn load_missing_project_inherits_global() {
        let dir = tempfile::tempdir().expect("tempdir");
        let global = write_file(dir.path(), "global.json", r#"{"worker_model":"g-model"}"#);
        let file = MmhmemFile::load(&global, &dir.path().join("missing-project")).expect("ok");
        assert_eq!(file.worker_model.as_deref(), Some("g-model"));
        assert_eq!(file.worker_api_key, None);
    }

    #[test]
    fn load_missing_global_uses_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = write_file(dir.path(), "project.json", r#"{"worker_api_key":"p-key"}"#);
        let file = MmhmemFile::load(&dir.path().join("missing-global"), &project).expect("ok");
        assert_eq!(file.worker_api_key.as_deref(), Some("p-key"));
    }

    #[test]
    fn load_invalid_global_json_fails_fast_naming_the_file() {
        // Req 7.3: invalid JSON → error naming the offending file.
        let dir = tempfile::tempdir().expect("tempdir");
        let global = write_file(dir.path(), "global.json", "not json at all");
        let err = MmhmemFile::load(&global, &dir.path().join("missing-project"))
            .expect_err("invalid JSON must fail fast");
        assert_eq!(err, ConfigError::InvalidMmhmemFile(global.clone()));
        assert!(
            err.to_string().contains("global.json"),
            "error must name the file: {err}"
        );
    }

    #[test]
    fn load_invalid_project_json_fails_fast_naming_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = write_file(dir.path(), "project.json", "{not closed");
        let err = MmhmemFile::load(&dir.path().join("missing-global"), &project)
            .expect_err("invalid JSON must fail fast");
        assert_eq!(err, ConfigError::InvalidMmhmemFile(project.clone()));
        assert!(
            err.to_string().contains("project.json"),
            "error must name the file: {err}"
        );
    }

    #[test]
    fn load_empty_objects_are_all_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let global = write_file(dir.path(), "global.json", "{}");
        let project = write_file(dir.path(), "project.json", "{}");
        let file = MmhmemFile::load(&global, &project).expect("empty objects are valid");
        assert_eq!(file, MmhmemFile::default());
    }

    #[test]
    fn load_ignores_unknown_json_fields() {
        // Forward compatibility: installers may write extra fields.
        let dir = tempfile::tempdir().expect("tempdir");
        let project = write_file(
            dir.path(),
            "project.json",
            r#"{"worker_model":"m","future_setting":42}"#,
        );
        let file = MmhmemFile::load(&dir.path().join("missing-global"), &project)
            .expect("unknown fields are ignored");
        assert_eq!(file.worker_model.as_deref(), Some("m"));
    }

    #[test]
    fn load_tolerates_a_utf8_bom_prefix() {
        // Hand-edited Windows files may carry a BOM; the content is still
        // valid JSON, so Req 7.3's fail-fast must not trigger on it.
        let dir = tempfile::tempdir().expect("tempdir");
        let global = write_file(
            dir.path(),
            "global.json",
            "\u{feff}{\"worker_model\":\"m\"}",
        );
        let file = MmhmemFile::load(&global, &dir.path().join("missing-project"))
            .expect("BOM-prefixed JSON is valid");
        assert_eq!(file.worker_model.as_deref(), Some("m"));
    }

    #[test]
    fn merged_lookup_env_wins_over_file() {
        // Req 7.1 precedence, first link: environment wins.
        let file = MmhmemFile {
            worker_model: Some("file-model".into()),
            ..MmhmemFile::default()
        };
        let map = lookup(&[(ENV_WORKER_MODEL, "env-model")]);
        let g = get(&map);
        let merged = MmhmemFile::merged_lookup(&g, &file);
        assert_eq!(merged(ENV_WORKER_MODEL).as_deref(), Some("env-model"));
    }

    #[test]
    fn merged_lookup_file_used_when_env_absent() {
        let file = MmhmemFile {
            worker_model: Some("file-model".into()),
            ..MmhmemFile::default()
        };
        let map = lookup(&[]);
        let g = get(&map);
        let merged = MmhmemFile::merged_lookup(&g, &file);
        assert_eq!(merged(ENV_WORKER_MODEL).as_deref(), Some("file-model"));
    }

    #[test]
    fn merged_lookup_none_when_both_absent() {
        let file = MmhmemFile::default();
        let map = lookup(&[]);
        let g = get(&map);
        let merged = MmhmemFile::merged_lookup(&g, &file);
        assert_eq!(merged(ENV_WORKER_MODEL), None);
    }

    #[test]
    fn merged_lookup_non_worker_keys_stay_env_only() {
        // The file carries worker settings only — never any other setting.
        let file = MmhmemFile {
            worker_model: Some("file-model".into()),
            ..MmhmemFile::default()
        };
        let map = lookup(&[("AGENT_MEM_DATA_DIR", "/from-env")]);
        let g = get(&map);
        let merged = MmhmemFile::merged_lookup(&g, &file);
        assert_eq!(merged("AGENT_MEM_DATA_DIR").as_deref(), Some("/from-env"));
        assert_eq!(
            merged("AGENT_MEM_TOKEN"),
            None,
            "file must not supply tokens"
        );
    }

    #[test]
    fn merged_precedence_reaches_config_end_to_end() {
        // Req 7.1 full matrix in one config: env model, project url,
        // global key — each setting from a different level.
        let dir = tempfile::tempdir().expect("tempdir");
        let global = write_file(dir.path(), "global.json", r#"{"worker_api_key":"g-key"}"#);
        let project = write_file(dir.path(), "project.json", r#"{"worker_api_url":"p-url"}"#);
        let file = MmhmemFile::load(&global, &project).expect("valid files");
        let map = local_user_env(&[(ENV_WORKER_MODEL, "env-model")]);
        let cfg = Config::from_env_local(&MmhmemFile::merged_lookup(&get(&map), &file))
            .expect("merged lookup composes with the pure parser");
        assert_eq!(cfg.worker_model.as_deref(), Some("env-model"));
        assert_eq!(cfg.worker_api_url, "p-url");
        assert_eq!(cfg.worker_api_key.as_deref(), Some("g-key"));
    }

    #[test]
    fn file_sourced_worker_key_is_redacted_in_debug() {
        // Req 7.5: redaction is source-agnostic — a key that came from a
        // file must never appear in diagnostics either.
        let dir = tempfile::tempdir().expect("tempdir");
        let global = write_file(
            dir.path(),
            "global.json",
            r#"{"worker_api_key":"file-secret-key"}"#,
        );
        let file =
            MmhmemFile::load(&global, &dir.path().join("missing-project")).expect("valid file");
        let map = local_user_env(&[]);
        let cfg =
            Config::from_env_local(&MmhmemFile::merged_lookup(&get(&map), &file)).expect("ok");
        let dbg = cfg.redacted_debug();
        assert!(
            !dbg.contains("file-secret-key"),
            "file-sourced key leaked into redacted_debug: {dbg}"
        );
        assert!(
            dbg.contains("worker_api_key"),
            "field label expected: {dbg}"
        );
    }

    #[test]
    fn empty_url_file_value_is_not_blank_defaulted() {
        // A file-sourced URL must beat the built-in default like any
        // other level (Req 7.1), and a blank file value is ignored by
        // the same `present()` discipline as blank env values.
        let file = MmhmemFile {
            worker_api_url: Some("p-url".into()),
            ..MmhmemFile::default()
        };
        let map = local_user_env(&[]);
        let cfg =
            Config::from_env_local(&MmhmemFile::merged_lookup(&get(&map), &file)).expect("ok");
        assert_eq!(cfg.worker_api_url, "p-url");
        let _ = ENV_WORKER_API_URL; // referenced for the doc link
    }

    #[test]
    fn update_check_key_merges_project_over_global() {
        let dir = tempfile::tempdir().expect("tempdir");
        let global = write_file(dir.path(), "g.json", r#"{"update_check":false}"#);
        let file = MmhmemFile::load(&global, &dir.path().join("none")).expect("ok");
        assert_eq!(file.update_check, Some(false));
        let project = write_file(dir.path(), "p.json", r#"{"update_check":true}"#);
        let file = MmhmemFile::load(&global, &project).expect("ok");
        assert_eq!(file.update_check, Some(true));
        let empty = write_file(dir.path(), "e.json", "{}");
        assert_eq!(MmhmemFile::load(&empty, &empty).unwrap().update_check, None);
    }

    #[test]
    fn shunt_content_cache_max_rows_merges_env_project_global_default() {
        // shunt-content-tool Req 4.3: env > project > global > default.
        let dir = tempfile::tempdir().expect("tempdir");
        let global = write_file(
            dir.path(),
            "g.json",
            r#"{"shunt_content_cache_max_rows":100}"#,
        );
        let project = write_file(
            dir.path(),
            "p.json",
            r#"{"shunt_content_cache_max_rows":200}"#,
        );
        let file = MmhmemFile::load(&global, &project).expect("ok");
        let map = local_user_env(&[]);
        let cfg =
            Config::from_env_local(&MmhmemFile::merged_lookup(&get(&map), &file)).expect("ok");
        assert_eq!(cfg.shunt_content_cache_max_rows, 200, "project over global");

        let file = MmhmemFile::load(&global, &dir.path().join("missing")).expect("ok");
        let cfg =
            Config::from_env_local(&MmhmemFile::merged_lookup(&get(&map), &file)).expect("ok");
        assert_eq!(cfg.shunt_content_cache_max_rows, 100, "global fills");

        let map = local_user_env(&[(ENV_SHUNT_CONTENT_CACHE_MAX_ROWS, "300")]);
        let cfg =
            Config::from_env_local(&MmhmemFile::merged_lookup(&get(&map), &file)).expect("ok");
        assert_eq!(cfg.shunt_content_cache_max_rows, 300, "env wins over file");
    }

    #[test]
    fn shunt_content_cache_max_rows_zero_in_file_fails_fast() {
        // A zero cap from any source is rejected by the shared parser with
        // the key-naming error, never silently coerced.
        let file = MmhmemFile {
            shunt_content_cache_max_rows: Some(0),
            ..MmhmemFile::default()
        };
        let map = local_user_env(&[]);
        let err = Config::from_env_local(&MmhmemFile::merged_lookup(&get(&map), &file))
            .expect_err("zero cap must fail fast");
        assert!(matches!(
            err,
            ConfigError::InvalidShuntContentCacheMaxRows(_)
        ));
        assert!(
            err.to_string().contains("SHUNT_CONTENT_CACHE_MAX_ROWS"),
            "error must name the key: {err}"
        );
    }
}
