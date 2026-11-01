//! Workspace path confinement (design.md "Security Considerations" + the
//! amended rule: EVERY agent-supplied path — read target, write target, write
//! reference — is canonicalized and must lie under `AGENT_MEM_ALLOWED_ROOTS`).
//!
//! Shared by the shunt engine and the boilerplate writer so the rule lives in
//! exactly one place.

use std::path::PathBuf;

use crate::config::Config;

/// Canonicalized allowed roots.
#[derive(Debug, Clone)]
pub struct AllowedRoots {
    roots: Vec<PathBuf>,
    /// True only when the profile configured NO roots at all (the local
    /// default): every path is allowed (Req 3.3). Distinct from `roots`
    /// becoming empty after failed canonicalization, which stays
    /// deny-by-default — serve mode never sees an empty configured set
    /// (its config validation requires ≥1 root).
    unconfined: bool,
}

/// Confinement failure.
#[derive(Debug)]
pub enum PathError {
    /// Path does not exist (canonicalize failed).
    NotFound(String),
    /// Canonicalized path lies outside all allowed roots.
    OutsideRoots(PathBuf),
}

impl AllowedRoots {
    /// From config; roots are canonicalized once at construction. Roots that
    /// fail to canonicalize are dropped (and logged) — deny-by-default. An
    /// empty configured set means unconfined (local mode, Req 3.3).
    pub fn from_config(config: &Config) -> Self {
        let unconfined = config.allowed_roots.is_empty();
        let roots = config
            .allowed_roots
            .iter()
            .filter_map(|r| match std::fs::canonicalize(r) {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::warn!("allowed root {} not canonicalizable: {e}", r.display());
                    None
                }
            })
            .collect();
        Self { roots, unconfined }
    }

    /// The canonicalized roots.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// True when `path` lies under an allowed root — or always, when the
    /// profile configured no roots (unconfined local mode, Req 3.3).
    fn allowed(&self, path: &std::path::Path) -> bool {
        self.unconfined || self.roots.iter().any(|root| path.starts_with(root))
    }

    /// Canonicalize `raw` and require it to lie under an allowed root.
    pub fn confine(&self, raw: &str) -> Result<PathBuf, PathError> {
        let path = std::fs::canonicalize(raw).map_err(|_| PathError::NotFound(raw.to_string()))?;
        if self.allowed(&path) {
            Ok(path)
        } else {
            Err(PathError::OutsideRoots(path))
        }
    }

    /// Like [`Self::confine`], but for a file that does NOT exist yet (a
    /// write target): canonicalize the nearest existing ancestor and
    /// re-join the non-existent tail, then enforce the root check on the
    /// result. This is what stops `..` traversal into outside roots.
    pub fn confine_new_file(&self, raw: &str) -> Result<PathBuf, PathError> {
        let p = PathBuf::from(raw);
        let mut existing = p.clone();
        let mut tail: Vec<std::ffi::OsString> = Vec::new();
        while !existing.exists() {
            match (existing.file_name(), existing.parent()) {
                (Some(name), Some(parent)) => {
                    tail.push(name.to_os_string());
                    existing = parent.to_path_buf();
                }
                _ => return Err(PathError::NotFound(raw.to_string())),
            }
        }
        let canon =
            std::fs::canonicalize(&existing).map_err(|_| PathError::NotFound(raw.to_string()))?;
        let mut full = canon;
        for comp in tail.iter().rev() {
            full.push(comp);
        }
        if self.allowed(&full) {
            Ok(full)
        } else {
            Err(PathError::OutsideRoots(full))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ConfigError};
    use std::collections::HashMap;

    /// Local-profile env with per-user vars so config parsing succeeds with
    /// zero mandatory variables (Req 2.5).
    fn local_env() -> HashMap<String, String> {
        [
            ("XDG_DATA_HOME", "/xdg-data"),
            ("HOME", "/home/u"),
            ("APPDATA", r"C:\Users\u\AppData\Roaming"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    fn local_config() -> Config {
        let map = local_env();
        Config::from_env_local(&|k| map.get(k).cloned()).unwrap()
    }

    fn serve_config_with_roots(roots: &str) -> Config {
        let map: HashMap<String, String> =
            [("AGENT_MEM_TOKEN", "t"), ("AGENT_MEM_ALLOWED_ROOTS", roots)]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        Config::from_env(&|k| map.get(k).cloned()).unwrap()
    }

    fn manifest_file() -> String {
        format!("{}/src/lib.rs", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn no_configured_roots_means_unconfined() {
        // Req 3.3: unset roots in local mode allow reads.
        let roots = AllowedRoots::from_config(&local_config());
        let confined = roots.confine(&manifest_file());
        assert!(
            confined.is_ok(),
            "unconfined must allow any path: {confined:?}"
        );
    }

    #[test]
    fn unconfined_allows_new_files_outside_any_root() {
        let roots = AllowedRoots::from_config(&local_config());
        let dir = tempfile::tempdir().unwrap();
        let fresh = dir.path().join("brand-new").join("file.txt");
        assert!(roots.confine_new_file(&fresh.to_string_lossy()).is_ok());
    }

    #[test]
    fn configured_roots_still_enforce() {
        // Serve-mode behavior: roots set → inside allowed, outside denied.
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let roots = AllowedRoots::from_config(&serve_config_with_roots(manifest_dir));
        assert!(roots.confine(&manifest_file()).is_ok());
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("outside.txt");
        std::fs::write(&outside_file, "x").unwrap();
        assert!(matches!(
            roots.confine(&outside_file.to_string_lossy()),
            Err(PathError::OutsideRoots(_))
        ));
    }

    #[test]
    fn all_roots_dropped_by_canonicalization_still_denies() {
        // The fail-safe corner: roots were CONFIGURED (not empty) but none
        // canonicalize — deny-by-default must hold (Req 9/12.5, no
        // fail-open regression through the unconfined branch).
        let roots = AllowedRoots::from_config(&serve_config_with_roots(
            "/nonexistent-root-for-mmhmem-test",
        ));
        assert!(matches!(
            roots.confine(&manifest_file()),
            Err(PathError::OutsideRoots(_))
        ));
    }

    #[test]
    fn local_config_has_no_required_vars_and_serve_still_does() {
        // Guard: the local profile really loads with zero AGENT_MEM_* vars,
        // and the serve profile still refuses a missing token (Req 9.2).
        let _cfg = local_config();
        let empty: HashMap<String, String> = HashMap::new();
        assert_eq!(
            Config::from_env(&|k| empty.get(k).cloned()).unwrap_err(),
            ConfigError::MissingToken
        );
    }
}
