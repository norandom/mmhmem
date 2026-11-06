//! The newest stable release (task 5.4).
//!
//! The GitHub releases API (`/repos/<owner>/<repo>/releases/latest`, through
//! `self_update`) is asked first. Only when that fails (for example the
//! unauthenticated API limit of 60 requests an hour is used up) the web
//! page `https://github.com/<owner>/<repo>/releases/latest` is requested
//! without following its redirect, and the tag is read from the `Location`
//! header (`.../releases/tag/<tag>`). That answer has no asset list, so the
//! file names a release must have are filled in from the tag. If both fail,
//! the API's error is returned. A tag that is not a stable
//! `vMAJOR.MINOR.PATCH` is refused either way (Requirement 2.1).

use std::time::Duration;

use self_update::reqwest::blocking::Client;
use self_update::reqwest::header::LOCATION;
use self_update::reqwest::redirect::Policy;

use super::{ReleaseError, ReleaseSource, SUMS_NAME, SUMS_SIG_NAME, TIMEOUT, Target};
use crate::setup::record::ReleaseTag;

/// How the newest release was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupVia {
    /// The GitHub releases API (asset list as published).
    Api,
    /// The `releases/latest` redirect (asset list derived from the tag).
    Redirect,
}

/// The newest stable release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestRelease {
    /// Its tag, e.g. `v0.5.0`.
    pub tag: ReleaseTag,
    /// Its version, e.g. `0.5.0`.
    pub version: String,
    /// The names of its files.
    pub assets: Vec<String>,
    /// How it was found.
    pub via: LookupVia,
}

/// The newest stable release of `source` (see the module docs). Blocking;
/// see the blocking contract in `release.rs`.
pub fn latest_release(
    source: &ReleaseSource,
    target: Target,
) -> Result<LatestRelease, ReleaseError> {
    latest_release_within(source, target, TIMEOUT)
}

/// [`latest_release`] with a per-request `timeout` (the background update
/// notice in `mmhmem stdio` uses a short one).
pub fn latest_release_within(
    source: &ReleaseSource,
    target: Target,
    timeout: Duration,
) -> Result<LatestRelease, ReleaseError> {
    super::off_runtime(|| match from_api(source, target, timeout) {
        Ok(latest) => Ok(latest),
        Err(api) => from_redirect(source, target, timeout).map_err(|_| api),
    })
}

fn from_api(
    source: &ReleaseSource,
    target: Target,
    timeout: Duration,
) -> Result<LatestRelease, ReleaseError> {
    let releases = super::configure(source, target)
        .timeout(timeout)
        .build()
        .and_then(|u| u.get_latest_release())
        .map_err(|e| super::lookup_error(&e))?;
    let release = releases.latest().ok_or_else(|| ReleaseError::Lookup {
        reason: "no release was found".to_string(),
    })?;
    let tag = stable_tag(&format!("v{}", release.version()))?;
    Ok(LatestRelease {
        version: version_of(&tag),
        tag,
        assets: release
            .assets()
            .iter()
            .map(|a| a.name().to_string())
            .collect(),
        via: LookupVia::Api,
    })
}

fn from_redirect(
    source: &ReleaseSource,
    target: Target,
    timeout: Duration,
) -> Result<LatestRelease, ReleaseError> {
    let failed = |reason: &str| ReleaseError::Lookup {
        reason: reason.to_string(),
    };
    let url = format!(
        "{}/{}/{}/releases/latest",
        source.web_base(),
        source.owner,
        source.repo
    );
    let client = Client::builder()
        .redirect(Policy::none())
        .timeout(timeout)
        .user_agent(concat!("mmhmem/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| failed("the connection could not be set up"))?;
    let response = client
        .get(url)
        .send()
        .map_err(|_| failed("the connection failed"))?;
    if !response.status().is_redirection() {
        return Err(failed("the release page did not name a release"));
    }
    let location = response
        .headers()
        .get(LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| failed("the release page did not name a release"))?;
    let text = location
        .trim_end_matches('/')
        .rsplit_once("/releases/tag/")
        .map(|(_, tag)| tag)
        .ok_or_else(|| failed("the release page did not name a release"))?;
    let tag = stable_tag(text)?;
    Ok(LatestRelease {
        version: version_of(&tag),
        assets: vec![
            target.archive_name(&tag),
            SUMS_NAME.to_string(),
            SUMS_SIG_NAME.to_string(),
        ],
        tag,
        via: LookupVia::Redirect,
    })
}

/// `text` as a tag if it is a stable `vMAJOR.MINOR.PATCH` (no leading
/// zeros, no pre-release or build suffix).
pub(super) fn stable_tag(text: &str) -> Result<ReleaseTag, ReleaseError> {
    let number = |p: &str| {
        !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && (p == "0" || !p.starts_with('0'))
    };
    let stable = text
        .strip_prefix('v')
        .map(|rest| rest.split('.').collect::<Vec<_>>())
        .is_some_and(|parts| parts.len() == 3 && parts.iter().all(|p| number(p)));
    let tag = ReleaseTag::try_from(text.to_string()).map_err(|_| ReleaseError::Lookup {
        reason: "the release tag is not a version number".to_string(),
    })?;
    if stable {
        Ok(tag)
    } else {
        Err(ReleaseError::NotStable {
            tag: tag.as_str().to_string(),
        })
    }
}

fn version_of(tag: &ReleaseTag) -> String {
    tag.as_str().trim_start_matches('v').to_string()
}
