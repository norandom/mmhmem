//! Test fixtures for `release.rs`: signed `.zip` / `.tar.gz` release
//! archives, a signed `SHA256SUMS`, and an `httpmock` server that plays the
//! GitHub releases API and asset downloads (as in `tests/setup_update_spike.rs`).

use std::io::{Cursor, Write};

use httpmock::prelude::*;
use httpmock::{Mock, MockServer};
use zipsign_api::Prehash;

use super::{ReleaseSource, SUMS_NAME, SUMS_SIG_NAME, Target};
use crate::setup::record::ReleaseTag;
use crate::setup::release_keys::test_keys::signing_key;

/// The trusted test key seed; others act as unknown keys.
pub(super) const TRUSTED: u8 = 1;
pub(super) const OWNER: &str = "o";
pub(super) const REPO: &str = "r";

pub(super) fn tag(text: &str) -> ReleaseTag {
    ReleaseTag::try_from(text.to_string()).unwrap()
}

pub(super) fn source(server: &MockServer) -> ReleaseSource {
    ReleaseSource::with_base_urls(OWNER, REPO, &server.base_url(), &server.base_url())
}

/// A zip with the single program entry.
pub(super) fn zip_archive(entry: &str, content: &[u8]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(entry, opts).unwrap();
        zip.write_all(content).unwrap();
        zip.finish().unwrap();
    }
    buf.into_inner()
}

/// A `.tar.gz` with the single program entry (ustar, mode 0755; gzip with
/// stored deflate blocks, so no compressor is needed).
pub(super) fn tar_gz_archive(entry: &str, content: &[u8]) -> Vec<u8> {
    let mut header = [0u8; 512];
    let mut field = |at: usize, bytes: &[u8]| header[at..at + bytes.len()].copy_from_slice(bytes);
    field(0, entry.as_bytes());
    field(100, b"0000755\0");
    field(108, b"0000000\0");
    field(116, b"0000000\0");
    field(124, format!("{:011o}\0", content.len()).as_bytes());
    field(136, b"00000000000\0");
    field(148, b"        ");
    field(156, b"0");
    field(257, b"ustar\x0000");
    let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
    header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    let mut tar = header.to_vec();
    tar.extend_from_slice(content);
    tar.resize(tar.len().div_ceil(512) * 512 + 1024, 0);
    gzip_stored(&tar)
}

fn gzip_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    let chunks: Vec<&[u8]> = data.chunks(0xffff).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        out.push(u8::from(i + 1 == chunks.len()));
        let len = u16::try_from(chunk.len()).unwrap();
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// The archive signed by key `seed` under its published `name` (context).
pub(super) fn sign_archive(target: Target, archive: &[u8], name: &str, seed: u8) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    let keys = [signing_key(seed)];
    let context = Some(name.as_bytes());
    match target {
        Target::WindowsX64 => {
            zipsign_api::sign::copy_and_sign_zip(
                &mut Cursor::new(archive),
                &mut out,
                &keys,
                context,
            )
            .unwrap();
        }
        _ => {
            zipsign_api::sign::copy_and_sign_tar(
                &mut Cursor::new(archive),
                &mut out,
                &keys,
                context,
            )
            .unwrap();
        }
    }
    out.into_inner()
}

/// `SHA256SUMS.sig` exactly as `zipsign sign separate` produces it.
pub(super) fn sign_sums(sums: &[u8], seed: u8) -> Vec<u8> {
    let mut prehash = Prehash::new();
    prehash.write_all(sums).unwrap();
    zipsign_api::sign::gather_signature_data(&[signing_key(seed)], &prehash, Some(b"SHA256SUMS"))
        .unwrap()
}

/// One release as the fake server publishes it.
pub(super) struct FakeRelease {
    pub tag: ReleaseTag,
    pub target: Target,
    pub archive: Vec<u8>,
    pub sums: Vec<u8>,
    pub sig: Vec<u8>,
    /// An asset that is listed but answers HTTP 404.
    pub not_found: Option<&'static str>,
    /// An asset left out of the release's asset list.
    pub unlisted: Option<&'static str>,
}

impl FakeRelease {
    /// A correct release: archive signed by the trusted key, `SHA256SUMS`
    /// listing it, `SHA256SUMS.sig` by the trusted key.
    pub(super) fn good(target: Target, content: &[u8]) -> Self {
        let tag = tag("v9.9.9");
        let name = target.archive_name(&tag);
        let raw = match target {
            Target::WindowsX64 => zip_archive(target.entry_name(), content),
            _ => tar_gz_archive(target.entry_name(), content),
        };
        let archive = sign_archive(target, &raw, &name, TRUSTED);
        let mut fake = Self {
            tag,
            target,
            archive,
            sums: Vec::new(),
            sig: Vec::new(),
            not_found: None,
            unlisted: None,
        };
        fake.resign_sums(&fake.archive.clone());
        fake
    }

    pub(super) fn archive_name(&self) -> String {
        self.target.archive_name(&self.tag)
    }

    /// Rewrite `SHA256SUMS` to list `bytes` for the archive, and sign it.
    pub(super) fn resign_sums(&mut self, bytes: &[u8]) {
        let hash = crate::storage::audit::sha256_hex(bytes);
        let sums = format!(
            "{hash}  {}\n{}  install.ps1\n",
            self.archive_name(),
            "0".repeat(64)
        );
        self.sums = sums.into_bytes();
        self.sig = sign_sums(&self.sums, TRUSTED);
    }

    fn release_json(&self, base: &str) -> serde_json::Value {
        let assets: Vec<serde_json::Value> = [
            (self.archive_name(), "archive"),
            (SUMS_NAME.to_string(), "sums"),
            (SUMS_SIG_NAME.to_string(), "sig"),
        ]
        .into_iter()
        .filter(|(name, _)| Some(name.as_str()) != self.unlisted)
        .map(|(name, id)| serde_json::json!({"name": name, "url": format!("{base}/assets/{id}")}))
        .collect();
        serde_json::json!({
            "tag_name": self.tag.as_str(), "name": self.tag.as_str(),
            "created_at": "2026-11-04T00:00:00Z", "body": "", "assets": assets
        })
    }

    /// Serve this release (by tag and as `latest`) on `server`.
    pub(super) fn serve<'a>(&self, server: &'a MockServer) -> Vec<Mock<'a>> {
        let release = self.release_json(&server.base_url());
        let mut mocks = Vec::new();
        for path in [
            format!("/repos/{OWNER}/{REPO}/releases/tags/{}", self.tag.as_str()),
            format!("/repos/{OWNER}/{REPO}/releases/latest"),
        ] {
            let body = release.clone();
            mocks.push(server.mock(|when, then| {
                when.method(GET).path(path);
                then.status(200).json_body(body);
            }));
        }
        for (id, name, bytes) in [
            ("archive", self.archive_name(), &self.archive),
            ("sums", SUMS_NAME.to_string(), &self.sums),
            ("sig", SUMS_SIG_NAME.to_string(), &self.sig),
        ] {
            let status = if Some(name.as_str()) == self.not_found {
                404
            } else {
                200
            };
            let body = bytes.clone();
            mocks.push(server.mock(|when, then| {
                when.method(GET).path(format!("/assets/{id}"));
                then.status(status).body(body);
            }));
        }
        mocks
    }
}

/// Total requests the server answered through `mocks`.
pub(super) fn calls(mocks: &[Mock<'_>]) -> usize {
    mocks.iter().map(Mock::calls).sum()
}
