//! `SHA256SUMS` and its detached `SHA256SUMS.sig` (task 5.4; format in
//! `docs/release-signing.md`): download, signature check with the trusted
//! keys (zipsign "separate" block, context `SHA256SUMS`) and entry lookup.

use std::time::Duration;

use self_update::http_client::header::{ACCEPT, HeaderMap, HeaderValue};
use zipsign_api::Prehash;
use zipsign_api::verify::{collect_keys, find_match, read_signatures};

use super::error::transport_reason;
use super::{ReleaseError, SUMS_NAME};
use crate::setup::record::Sha256Hex;

/// Largest `SHA256SUMS` or `SHA256SUMS.sig` accepted (both are a few
/// hundred bytes in practice).
const MAX_SMALL_ASSET: u64 = 1024 * 1024;

/// Download a small release file (`SHA256SUMS`, `SHA256SUMS.sig`) to memory.
pub(super) fn download_small(
    url: &str,
    name: &str,
    timeout: Duration,
) -> Result<Vec<u8>, ReleaseError> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/octet-stream"));
    let mut download = self_update::Download::from_url(url);
    download
        .timeout(timeout)
        .max_download_size(MAX_SMALL_ASSET)
        .show_download_progress(false)
        .replace_headers(headers);
    let mut body = Vec::new();
    download
        .download_to(&mut body)
        .map_err(|e| ReleaseError::Download {
            name: name.to_string(),
            reason: transport_reason(&e),
        })?;
    Ok(body)
}

/// Check that `sig` is a signature of `sums` by one of `keys` (context
/// `SHA256SUMS`). `keys` must not be empty (checked by the caller).
pub(super) fn verify_sums_signature(
    keys: &[[u8; 32]],
    sums: &[u8],
    sig: &[u8],
) -> Result<(), ReleaseError> {
    let keys =
        collect_keys(keys.iter().map(|k| Ok(*k))).map_err(|_| ReleaseError::NoTrustedKeys)?;
    let signatures = read_signatures(&mut &sig[..]).map_err(|_| ReleaseError::BadSumsSignature)?;
    let prehash = Prehash::calculate(&mut &sums[..]).map_err(|_| ReleaseError::BadSumsSignature)?;
    find_match(&keys, &signatures, &prehash, Some(SUMS_NAME.as_bytes()))
        .map(|_| ())
        .map_err(|_| ReleaseError::BadSumsSignature)
}

/// The SHA-256 that `sums` lists for `name`: lines of `<hash>  <name>` (a
/// `*` before the name, binary mode, is accepted). A missing, malformed or
/// repeated entry is refused.
pub(super) fn sums_entry(sums: &[u8], name: &str) -> Result<Sha256Hex, ReleaseError> {
    let none = || ReleaseError::NoSumsEntry {
        name: name.to_string(),
    };
    let text = std::str::from_utf8(sums).map_err(|_| none())?;
    let mut found = None;
    for line in text.lines() {
        let Some((hash, file)) = line.trim_end().split_once(char::is_whitespace) else {
            continue;
        };
        let file = file.trim_start();
        if file.strip_prefix('*').unwrap_or(file) != name {
            continue;
        }
        if found.is_some() {
            return Err(none());
        }
        found = Some(Sha256Hex::try_from(hash.to_ascii_lowercase()).map_err(|_| none())?);
    }
    found.ok_or_else(none)
}
