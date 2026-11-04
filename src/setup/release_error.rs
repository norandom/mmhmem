//! Errors of the release download and check (`release.rs`, task 5.4).
//!
//! Every message is a plain sentence that says what failed and that nothing
//! was changed. Server response bodies are never part of a message: errors
//! from `self_update` are reduced to their kind, the HTTP status and the URL
//! the request went to.

use std::fmt;

use super::{SUMS_NAME, SUMS_SIG_NAME};

/// Why a release could not be looked up, checked or installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseError {
    /// No release is published for this operating system and processor.
    Unsupported {
        /// The operating system or target that has no release.
        os: String,
        /// The processor type.
        arch: String,
    },
    /// This build has no release signing key, so no download can be checked.
    NoTrustedKeys,
    /// The newest release is not a stable `vMAJOR.MINOR.PATCH` release.
    NotStable {
        /// The tag that was found (validated; never server text otherwise).
        tag: String,
    },
    /// The release could not be found or read.
    Lookup {
        /// What went wrong.
        reason: String,
    },
    /// The release does not list a file it must have.
    MissingAsset {
        /// The file name.
        name: String,
    },
    /// A file of the release could not be downloaded.
    Download {
        /// The file name.
        name: String,
        /// What went wrong.
        reason: String,
    },
    /// `SHA256SUMS.sig` does not verify `SHA256SUMS` with a trusted key.
    BadSumsSignature,
    /// `SHA256SUMS` has no usable entry for a file.
    NoSumsEntry {
        /// The file name.
        name: String,
    },
    /// A downloaded file does not match its `SHA256SUMS` entry.
    HashMismatch {
        /// The file name.
        name: String,
        /// The SHA-256 listed in `SHA256SUMS`.
        expected: String,
        /// The SHA-256 of the downloaded file.
        actual: String,
    },
    /// The archive's embedded signature is missing or not from a trusted key.
    BadArchiveSignature {
        /// The archive name.
        name: String,
        /// What the signature check reported.
        reason: String,
    },
    /// The verified program could not be unpacked or put in place.
    Install {
        /// What went wrong.
        reason: String,
    },
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const UNCHANGED: &str = "Nothing was changed.";
        match self {
            Self::Unsupported { os, arch } => write!(
                f,
                "There is no mmhmem release for this computer ({os}, {arch}). {UNCHANGED}"
            ),
            Self::NoTrustedKeys => write!(
                f,
                "This mmhmem program has no release signing key built in, so it cannot check \
                 a download. Nothing was downloaded. {UNCHANGED}"
            ),
            Self::NotStable { tag } => write!(
                f,
                "The newest release ({tag}) is not a stable release, so it is not installed. \
                 {UNCHANGED}"
            ),
            Self::Lookup { reason } => {
                write!(
                    f,
                    "Could not look up the mmhmem release: {reason}. {UNCHANGED}"
                )
            }
            Self::MissingAsset { name } => write!(
                f,
                "The release has no file named {name}, so it cannot be checked. {UNCHANGED}"
            ),
            Self::Download { name, reason } => {
                write!(f, "Could not download {name}: {reason}. {UNCHANGED}")
            }
            Self::BadSumsSignature => write!(
                f,
                "{SUMS_SIG_NAME} is not a valid signature of {SUMS_NAME} by the mmhmem release \
                 key, so the checksum list cannot be trusted. {UNCHANGED}"
            ),
            Self::NoSumsEntry { name } => write!(
                f,
                "{SUMS_NAME} has no usable checksum for {name}. {UNCHANGED}"
            ),
            Self::HashMismatch {
                name,
                expected,
                actual,
            } => write!(
                f,
                "The download {name} does not match {SUMS_NAME}: expected SHA-256 {expected}, \
                 got {actual}. {UNCHANGED}"
            ),
            Self::BadArchiveSignature { name, reason } => write!(
                f,
                "The download {name} is not signed by the mmhmem release key ({reason}). \
                 {UNCHANGED}"
            ),
            Self::Install { reason } => {
                write!(
                    f,
                    "Could not install the checked program: {reason}. {UNCHANGED}"
                )
            }
        }
    }
}

impl std::error::Error for ReleaseError {}

/// A short reason for a transport or HTTP failure, without any response body.
pub(super) fn transport_reason(e: &self_update::Error) -> String {
    use self_update::Error as E;
    match e {
        E::NotFound { .. } => "it was not found (HTTP 404)".to_string(),
        E::Unauthorized { status, .. } => format!("the server refused access (HTTP {status})"),
        E::RateLimited { .. } => "GitHub's request limit was reached; try again later".into(),
        E::HttpStatus { status, .. } => format!("the server answered HTTP {status}"),
        E::Transport(_) => "the connection failed".to_string(),
        E::Io(io) => format!("a file error happened ({})", io.kind()),
        E::InvalidResponse { .. } | E::Json(_) => "the answer was not readable".to_string(),
        E::NoReleaseFound { .. } => "no such release was found".to_string(),
        E::SemVer(_) => "the release tag is not a version number".to_string(),
        other => kind_of(other),
    }
}

/// The error kind only (the text before the first `:` of its message).
fn kind_of(e: &self_update::Error) -> String {
    let text = e.to_string();
    let kind = text.split(':').next().unwrap_or("").trim();
    format!("unexpected error ({kind})")
}

/// Map an error of `update_extended` to a plain [`ReleaseError`].
///
/// `gate_mismatch` is the `(expected, actual)` pair recorded by the
/// pre-extraction archive gate when it refused the archive.
pub(super) fn from_update(
    e: self_update::Error,
    archive: &str,
    gate_mismatch: Option<(String, String)>,
) -> ReleaseError {
    use self_update::Error as E;
    let name = archive.to_string();
    match e {
        E::ChecksumMismatch {
            expected, computed, ..
        } => ReleaseError::HashMismatch {
            name,
            expected,
            actual: computed,
        },
        E::ArchiveVerificationRejected { .. } => match gate_mismatch {
            Some((expected, actual)) => ReleaseError::HashMismatch {
                name,
                expected,
                actual,
            },
            None => ReleaseError::Install {
                reason: "the downloaded archive was not the expected file".to_string(),
            },
        },
        E::NoSignatures(_) => ReleaseError::BadArchiveSignature {
            name,
            reason: "it carries no signature".to_string(),
        },
        E::Signature(inner) => ReleaseError::BadArchiveSignature {
            name,
            reason: inner.to_string(),
        },
        E::ChecksumSourceInvalid { .. } => ReleaseError::NoSumsEntry { name },
        E::InstallPathNotWritable { path, .. } => ReleaseError::Install {
            reason: format!("{} cannot be written", path.display()),
        },
        other => ReleaseError::Download {
            name,
            reason: transport_reason(&other),
        },
    }
}
