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
    /// fail to canonicalize are dropped (and logged) — deny-by-default.
    pub fn from_config(config: &Config) -> Self {
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
        Self { roots }
    }

    /// The canonicalized roots.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Canonicalize `raw` and require it to lie under an allowed root.
    pub fn confine(&self, raw: &str) -> Result<PathBuf, PathError> {
        let path = std::fs::canonicalize(raw).map_err(|_| PathError::NotFound(raw.to_string()))?;
        if self.roots.iter().any(|root| path.starts_with(root)) {
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
        if self.roots.iter().any(|root| full.starts_with(root)) {
            Ok(full)
        } else {
            Err(PathError::OutsideRoots(full))
        }
    }
}
