//! The security-critical path rule shared by the `ShuntEngine` and the
//! read hook (Req 2.3, 3.3, 5.4; split from `shunt.rs` per the Req 13.3
//! 500-LOC cap).
//!
//! A security-critical file is always read directly, never summarised. The
//! rule looks only at the part of the path inside its project, so the
//! folders above it (a home folder named `lockhart`, a temp folder
//! `.tmpPamAMb`) never make every file in the project security-critical.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

/// Security-critical path pattern, case-insensitive (Req 3.3; design.md
/// ShuntEngine: auth|crypto|secret|token|rbac|jwt|tls|cert|mutex|lock|
/// concurrency|permission|sudo|pam).
static SECURITY_CRITICAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(auth|crypto|secret|token|rbac|jwt|tls|cert|mutex|lock|concurren|permission|sudo|pam)",
    )
    .expect("valid regex")
});

/// Security-critical check of a whole path.
pub fn is_security_critical_path(path: &Path) -> bool {
    SECURITY_CRITICAL_RE.is_match(&path.to_string_lossy())
}

/// Security-critical check of the part of `path` inside its project: the
/// most specific of `projects` that contains it. A path outside every
/// project is checked whole, which errs towards reading files directly.
pub fn is_security_critical_in(path: &Path, projects: &[PathBuf]) -> bool {
    let inside = projects
        .iter()
        .filter_map(|project| relative_to(path, project))
        .min_by_key(|rest| rest.components().count());
    is_security_critical_path(inside.as_deref().unwrap_or(path))
}

/// `path` below `base`, or `None` when it is not under it. Windows paths
/// compare without case, as the file system does.
fn relative_to(path: &Path, base: &Path) -> Option<PathBuf> {
    if base.as_os_str().is_empty() {
        return None;
    }
    if let Ok(rest) = path.strip_prefix(base) {
        return Some(rest.to_path_buf());
    }
    if cfg!(windows) {
        let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
        let (path, base) = (lower(path), lower(base));
        return path.strip_prefix(&base).ok().map(Path::to_path_buf);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{is_security_critical_in, is_security_critical_path};
    use std::path::{Path, PathBuf};

    #[test]
    fn free_fn_matches_security_paths_case_insensitively() {
        assert!(is_security_critical_path(Path::new("src/Auth/login.rs")));
        assert!(is_security_critical_path(Path::new("a/JWT_helper.rs")));
        assert!(!is_security_critical_path(Path::new("src/readme.rs")));
    }

    #[test]
    fn only_the_part_inside_the_project_counts() {
        let home = Path::new("/home/lockhart");
        let project = home.join("app");
        let projects = [project.clone()];
        let main = project.join("src/main.rs");
        assert!(!is_security_critical_in(&main, &projects));
        assert!(is_security_critical_in(
            &project.join("src/auth.rs"),
            &projects
        ));
        // Outside every project the whole path is checked.
        assert!(is_security_critical_in(&home.join("notes.md"), &projects));
        assert!(is_security_critical_in(&home.join("notes.md"), &[]));
        // An empty project (no harness cwd) checks the whole path.
        assert!(is_security_critical_in(&main, &[PathBuf::new()]));
    }

    #[test]
    fn the_most_specific_project_wins() {
        // A project inside another: the inner one decides, so the outer
        // project's `tokens` folder above it does not count.
        let outer = PathBuf::from("/work");
        let inner = outer.join("tokens").join("app");
        let projects = [outer, inner.clone()];
        assert!(!is_security_critical_in(&inner.join("lib.rs"), &projects));
    }

    #[cfg(windows)]
    #[test]
    fn windows_projects_match_without_case() {
        let projects = [PathBuf::from(r"C:\Users\Pam\Source\app")];
        let path = Path::new(r"c:\users\pam\source\APP\src\main.rs");
        assert!(!is_security_critical_in(path, &projects));
    }
}
