//! The platforms that have a release (task 5.4; asset names as written by
//! `scripts/package-release-archives.ps1`). Anything else has no release
//! (Requirement 2.7).

use super::ReleaseError;
use crate::setup::record::ReleaseTag;

/// A platform with a published release archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// `x86_64-pc-windows-msvc` (`.zip` holding `mmhmem.exe`).
    WindowsX64,
    /// `x86_64-unknown-linux-musl` (`.tar.gz` holding `mmhmem`).
    LinuxX64,
    /// `x86_64-apple-darwin` (`.tar.gz` holding `mmhmem`).
    MacX64,
    /// `aarch64-apple-darwin` (`.tar.gz` holding `mmhmem`).
    MacArm,
}

impl Target {
    /// Every platform with a release.
    pub const ALL: [Self; 4] = [Self::WindowsX64, Self::LinuxX64, Self::MacX64, Self::MacArm];

    /// The platform this program runs on.
    pub fn current() -> Result<Self, ReleaseError> {
        Self::detect(std::env::consts::OS, std::env::consts::ARCH)
    }

    /// The platform for an operating system and processor as named by
    /// `std::env::consts::{OS, ARCH}`.
    pub fn detect(os: &str, arch: &str) -> Result<Self, ReleaseError> {
        match (os, arch) {
            ("windows", "x86_64") => Ok(Self::WindowsX64),
            ("linux", "x86_64") => Ok(Self::LinuxX64),
            ("macos", "x86_64") => Ok(Self::MacX64),
            ("macos", "aarch64") => Ok(Self::MacArm),
            _ => Err(ReleaseError::Unsupported {
                os: os.to_string(),
                arch: arch.to_string(),
            }),
        }
    }

    /// The platform for a Rust target triple.
    pub fn from_triple(triple: &str) -> Result<Self, ReleaseError> {
        Self::ALL
            .into_iter()
            .find(|t| t.triple() == triple)
            .ok_or_else(|| {
                let arch = triple.split('-').next().unwrap_or("");
                ReleaseError::Unsupported {
                    os: triple.to_string(),
                    arch: arch.to_string(),
                }
            })
    }

    /// The Rust target triple in the archive name.
    pub fn triple(self) -> &'static str {
        match self {
            Self::WindowsX64 => "x86_64-pc-windows-msvc",
            Self::LinuxX64 => "x86_64-unknown-linux-musl",
            Self::MacX64 => "x86_64-apple-darwin",
            Self::MacArm => "aarch64-apple-darwin",
        }
    }

    /// The archive's file name extension.
    pub fn extension(self) -> &'static str {
        match self {
            Self::WindowsX64 => "zip",
            Self::LinuxX64 | Self::MacX64 | Self::MacArm => "tar.gz",
        }
    }

    /// The single program entry inside the archive.
    pub fn entry_name(self) -> &'static str {
        match self {
            Self::WindowsX64 => "mmhmem.exe",
            Self::LinuxX64 | Self::MacX64 | Self::MacArm => "mmhmem",
        }
    }

    /// The published archive name, e.g.
    /// `mmhmem-v0.5.0-x86_64-pc-windows-msvc.zip`.
    pub fn archive_name(self, tag: &ReleaseTag) -> String {
        format!(
            "mmhmem-{}-{}.{}",
            tag.as_str(),
            self.triple(),
            self.extension()
        )
    }
}
