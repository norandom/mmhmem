//! Release lookup, verified download and install (reliable-installers task
//! 5.4, `.kiro/specs/reliable-installers/design.md` §2 "Updates and status";
//! Requirements 2.3–2.7). A thin wrapper around `self_update`'s GitHub
//! backend.
//!
//! # Verification order
//!
//! [`verify_and_fetch`] and [`update_binary`] share one flow:
//!
//! 1. **Keys.** An empty key list is refused before any request
//!    ([`ReleaseError::NoTrustedKeys`]): `self_update` silently skips the
//!    signature check when it gets no keys (research.md §11).
//! 2. **Release.** The release is looked up by tag; it must list the
//!    target's archive, `SHA256SUMS` and `SHA256SUMS.sig`.
//! 3. **Checksum list.** `SHA256SUMS` and `SHA256SUMS.sig` are downloaded
//!    and the detached signature is verified with the same keys (context
//!    `SHA256SUMS`) before anything in the list is trusted; the archive's
//!    entry is read from the verified bytes.
//! 4. **Archive.** `self_update` downloads the archive and checks it against
//!    `SHA256SUMS` (`checksum_from_asset`), then its embedded zipsign
//!    signature (`verifying_keys`, context = the archive's file name). A
//!    pre-extraction gate then re-checks the archive against the entry from
//!    step 3, so a `SHA256SUMS` served differently to `self_update`'s own
//!    request cannot slip through.
//! 5. **Unpack and install.** Only then is the program unpacked (`.zip` for
//!    Windows, `.tar.gz` elsewhere) and moved to the install path. When that
//!    path is the running program, `self_update` replaces it in place
//!    (`self_replace`, which handles the Windows file-in-use case).
//!
//! A failure at any step returns before the install path is touched.
//!
//! # Blocking contract
//!
//! Every network function here is **blocking** (`self_update` uses
//! `reqwest::blocking`, which panics on a tokio runtime thread). They are safe
//! to call from plain sync code. When called from inside a tokio runtime they
//! run the work on a dedicated thread and block the caller until it is done;
//! async callers should still wrap the call in `tokio::task::spawn_blocking`
//! so the executor is not held.
//!
//! # Production vs. tests
//!
//! [`ReleaseSource::github`] is the only production source. The base URL
//! override ([`ReleaseSource::with_base_urls`]) exists for the offline tests
//! and the CI harness.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use self_update::backends::github::{Update, UpdateBuilder};

use crate::setup::record::{ReleaseTag, Sha256Hex};

#[path = "release_error.rs"]
mod error;
#[path = "release_latest.rs"]
mod latest;
#[path = "release_sums.rs"]
mod sums;
#[path = "release_target.rs"]
mod target;

pub use error::ReleaseError;
pub use latest::{LatestRelease, LookupVia, latest_release};
pub use target::Target;

/// The checksum list published with every release.
pub const SUMS_NAME: &str = "SHA256SUMS";
/// The detached signature of [`SUMS_NAME`].
pub const SUMS_SIG_NAME: &str = "SHA256SUMS.sig";
/// The GitHub owner of the project.
pub const GITHUB_OWNER: &str = "norandom";
/// The GitHub repository of the project.
pub const GITHUB_REPO: &str = "mmhmem";
/// Per-request timeout for lookups and downloads.
const TIMEOUT: Duration = Duration::from_secs(120);

/// Where releases come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseSource {
    owner: String,
    repo: String,
    api_base: Option<String>,
    web_base: Option<String>,
}

impl ReleaseSource {
    /// The project's GitHub releases (production).
    pub fn github() -> Self {
        Self {
            owner: GITHUB_OWNER.to_string(),
            repo: GITHUB_REPO.to_string(),
            api_base: None,
            web_base: None,
        }
    }

    /// A GitHub-compatible server at other base URLs. **Tests and the CI
    /// harness only**; `api` replaces `https://api.github.com`, `web`
    /// replaces `https://github.com` (used by the `releases/latest`
    /// redirect fallback).
    pub fn with_base_urls(owner: &str, repo: &str, api: &str, web: &str) -> Self {
        Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
            api_base: Some(api.trim_end_matches('/').to_string()),
            web_base: Some(web.trim_end_matches('/').to_string()),
        }
    }

    fn web_base(&self) -> &str {
        self.web_base.as_deref().unwrap_or("https://github.com")
    }
}

/// A verified program, unpacked into its own temporary folder (deleted when
/// this value is dropped).
#[derive(Debug)]
pub struct VerifiedArchive {
    /// The release tag.
    pub tag: ReleaseTag,
    /// The platform it is for.
    pub target: Target,
    /// The archive's published file name.
    pub archive_name: String,
    /// The archive's SHA-256, as listed in the verified `SHA256SUMS`.
    pub archive_sha256: Sha256Hex,
    /// The unpacked program.
    pub binary: PathBuf,
    /// The unpacked program's SHA-256.
    pub binary_sha256: Sha256Hex,
    _dir: tempfile::TempDir,
}

/// A verified program put in place by [`update_binary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledBinary {
    /// The release tag.
    pub tag: ReleaseTag,
    /// The archive's SHA-256, as listed in the verified `SHA256SUMS`.
    pub archive_sha256: Sha256Hex,
    /// Where the program now is.
    pub path: PathBuf,
    /// Its SHA-256, re-hashed after the install (to store in the record).
    pub sha256: Sha256Hex,
}

/// Download, verify (see the module docs for the order) and unpack the
/// release `tag` for `target` into a temporary folder. Nothing outside that
/// folder is changed.
pub fn verify_and_fetch(
    source: &ReleaseSource,
    tag: &ReleaseTag,
    target: Target,
    keys: &[[u8; 32]],
) -> Result<VerifiedArchive, ReleaseError> {
    let keys = checked_keys(keys)?;
    let dir = tempfile::tempdir().map_err(install_io)?;
    let binary = dir.path().join(target.entry_name());
    let done = off_runtime(|| fetch_into(source, tag, target, keys, &binary))?;
    Ok(VerifiedArchive {
        tag: tag.clone(),
        target,
        archive_name: target.archive_name(tag),
        archive_sha256: done.archive_sha256,
        binary,
        binary_sha256: done.binary_sha256,
        _dir: dir,
    })
}

/// Download, verify and install the release `tag` for `target` at
/// `install_path`. Pass the running program's path
/// (`std::env::current_exe()`) to update it in place; any other path is
/// replaced by a rename (tests). On any failure `install_path` is unchanged.
pub fn update_binary(
    source: &ReleaseSource,
    tag: &ReleaseTag,
    target: Target,
    keys: &[[u8; 32]],
    install_path: &Path,
) -> Result<InstalledBinary, ReleaseError> {
    let keys = checked_keys(keys)?;
    let done = off_runtime(|| fetch_into(source, tag, target, keys, install_path))?;
    Ok(InstalledBinary {
        tag: tag.clone(),
        archive_sha256: done.archive_sha256,
        path: install_path.to_path_buf(),
        sha256: done.binary_sha256,
    })
}

/// Whether the installed program still matches the record (Requirement 2.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryStatus {
    /// Its SHA-256 equals the recorded one.
    Match,
    /// It was changed after the install.
    Mismatch {
        /// The recorded SHA-256.
        expected: Sha256Hex,
        /// The SHA-256 of the file now.
        actual: Sha256Hex,
    },
    /// There is no file at the recorded path.
    Missing,
}

/// Re-hash the program at `installed` and compare it with `recorded`.
/// Errors other than "not found" (for example no permission) are returned.
pub fn status_hash(installed: &Path, recorded: &Sha256Hex) -> std::io::Result<BinaryStatus> {
    let bytes = match fs_err::read(installed) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BinaryStatus::Missing),
        Err(e) => return Err(e),
    };
    let actual = Sha256Hex::of(&bytes);
    Ok(if &actual == recorded {
        BinaryStatus::Match
    } else {
        BinaryStatus::Mismatch {
            expected: recorded.clone(),
            actual,
        }
    })
}

/// Refuse an empty key list: `self_update` would silently skip the
/// signature check (research.md §11).
fn checked_keys(keys: &[[u8; 32]]) -> Result<&[[u8; 32]], ReleaseError> {
    if keys.is_empty() {
        Err(ReleaseError::NoTrustedKeys)
    } else {
        Ok(keys)
    }
}

/// Run blocking network work off any tokio runtime thread (see the module
/// docs). Without a runtime the closure runs on the calling thread.
fn off_runtime<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    if tokio::runtime::Handle::try_current().is_err() {
        return work();
    }
    std::thread::scope(|scope| match scope.spawn(work).join() {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    })
}

/// The base `self_update` configuration shared by the lookup and the
/// download. Nothing here downloads: only [`fetch_into`] calls
/// `update_extended`, and only after [`checked_keys`].
fn configure(source: &ReleaseSource, target: Target) -> UpdateBuilder {
    let mut builder = Update::configure();
    builder
        .repo_owner(&source.owner)
        .repo_name(&source.repo)
        .bin_name("mmhmem")
        .bin_path_in_archive(target.entry_name())
        .target(target.triple())
        .current_version(env!("CARGO_PKG_VERSION"))
        .timeout(TIMEOUT)
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(false);
    if let Some(api) = &source.api_base {
        builder.api_base_url(api);
    }
    builder
}

fn lookup_error(e: &self_update::Error) -> ReleaseError {
    ReleaseError::Lookup {
        reason: error::transport_reason(e),
    }
}

fn install_io(e: std::io::Error) -> ReleaseError {
    ReleaseError::Install {
        reason: format!("a file error happened ({})", e.kind()),
    }
}

/// What [`fetch_into`] verified and installed.
struct Fetched {
    archive_sha256: Sha256Hex,
    binary_sha256: Sha256Hex,
}

/// The download URLs of the checksum list and its signature. The release
/// must also list `archive` (downloaded later by `self_update`).
struct AssetUrls {
    sums: String,
    sig: String,
}

fn asset_urls(release: &self_update::Release, archive: &str) -> Result<AssetUrls, ReleaseError> {
    let url = |name: &str| {
        release
            .assets()
            .iter()
            .find(|a| a.name() == name)
            .map(|a| a.download_url().to_string())
            .ok_or_else(|| ReleaseError::MissingAsset {
                name: name.to_string(),
            })
    };
    url(archive)?;
    Ok(AssetUrls {
        sums: url(SUMS_NAME)?,
        sig: url(SUMS_SIG_NAME)?,
    })
}

/// Steps 2–5 of the module docs, installing to `install_path`.
fn fetch_into(
    source: &ReleaseSource,
    tag: &ReleaseTag,
    target: Target,
    keys: &[[u8; 32]],
    install_path: &Path,
) -> Result<Fetched, ReleaseError> {
    // Checked again here, next to the only `update_extended` call.
    let keys = checked_keys(keys)?;
    let archive = target.archive_name(tag);
    let release = configure(source, target)
        .build()
        .and_then(|u| u.get_release_version(tag.as_str()))
        .map_err(|e| lookup_error(&e))?;
    let urls = asset_urls(&release, &archive)?;
    let sums_bytes = sums::download_small(&urls.sums, SUMS_NAME, TIMEOUT)?;
    let sig_bytes = sums::download_small(&urls.sig, SUMS_SIG_NAME, TIMEOUT)?;
    sums::verify_sums_signature(keys, &sums_bytes, &sig_bytes)?;
    let expected = sums::sums_entry(&sums_bytes, &archive)?;

    let gate = ArchiveGate::new(&archive, expected.clone());
    let wanted = archive.clone();
    let mut builder = configure(source, target);
    builder
        .release_tag(tag.as_str())
        .bin_install_path(install_path)
        .asset_matcher(move |assets| assets.iter().find(|a| a.name() == wanted).cloned())
        .verifying_keys(keys.to_vec())
        .checksum_from_asset(SUMS_NAME)
        .verify_archive(gate.callback());
    let status = builder
        .build()
        .and_then(|u| u.update_extended())
        .map_err(|e| error::from_update(e, &archive, gate.mismatch()))?;
    if !status.is_updated() {
        return Err(ReleaseError::Install {
            reason: "the release was not installed".to_string(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::Permissions::from_mode(0o755);
        fs_err::set_permissions(install_path, mode).map_err(install_io)?;
    }
    let binary = fs_err::read(install_path).map_err(install_io)?;
    Ok(Fetched {
        archive_sha256: expected,
        binary_sha256: Sha256Hex::of(&binary),
    })
}

/// The pre-extraction gate (step 4): the archive must have the expected
/// file name and the SHA-256 listed in the *verified* `SHA256SUMS`.
#[derive(Clone)]
struct ArchiveGate {
    name: String,
    expected: Sha256Hex,
    mismatch: Arc<Mutex<Option<(String, String)>>>,
}

impl ArchiveGate {
    fn new(name: &str, expected: Sha256Hex) -> Self {
        Self {
            name: name.to_string(),
            expected,
            mismatch: Arc::new(Mutex::new(None)),
        }
    }

    fn callback(&self) -> impl Fn(&Path) -> self_update::Result<()> + Send + Sync + 'static {
        let gate = self.clone();
        move |path| gate.check(path)
    }

    fn check(&self, path: &Path) -> self_update::Result<()> {
        let reject = self_update::Error::archive_verification_rejected;
        if path.file_name().and_then(|n| n.to_str()) != Some(self.name.as_str()) {
            return Err(reject("unexpected archive name"));
        }
        let actual = Sha256Hex::of(&std::fs::read(path)?);
        if actual == self.expected {
            return Ok(());
        }
        if let Ok(mut slot) = self.mismatch.lock() {
            *slot = Some((self.expected.to_string(), actual.to_string()));
        }
        Err(reject("archive does not match the verified SHA256SUMS"))
    }

    fn mismatch(&self) -> Option<(String, String)> {
        self.mismatch.lock().ok().and_then(|slot| slot.clone())
    }
}

#[cfg(test)]
#[path = "release_fixtures.rs"]
mod fixtures;
#[cfg(test)]
#[path = "release_tests.rs"]
mod tests;
