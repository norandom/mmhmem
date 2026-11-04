//! reliable-installers task 1.4: prove the adopted `self_update` setup works
//! before any installer code depends on it. A local mock of the GitHub
//! releases API serves a zipsign-signed archive plus `SHA256SUMS`; the update
//! must install the signed archive and refuse an unsigned or mismatched one,
//! leaving the installed file untouched.

use std::io::{Cursor, Write};
use std::path::Path;

use httpmock::prelude::*;
use zipsign_api::SigningKey;

const TARGET: &str = "x86_64-pc-windows-msvc";
const TAG: &str = "v9.9.9";

/// Fixed test key: deterministic, never used outside this test.
fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn asset_name() -> String {
    format!("mmhmem-{TAG}-{TARGET}.zip")
}

/// A zip holding the binary the update installs.
fn archive(content: &[u8]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(format!("mmhmem{}", std::env::consts::EXE_SUFFIX), opts)
            .unwrap();
        zip.write_all(content).unwrap();
        zip.finish().unwrap();
    }
    buf.into_inner()
}

/// The archive signed the way the release pipeline will sign it: the
/// signature context is the published file name.
fn signed(zip: &[u8]) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    zipsign_api::sign::copy_and_sign_zip(
        &mut Cursor::new(zip),
        &mut out,
        &[signing_key()],
        Some(asset_name().as_bytes()),
    )
    .unwrap();
    out.into_inner()
}

fn sums_for(bytes: &[u8]) -> String {
    format!(
        "{}  {}\n",
        mmhmem::storage::audit::sha256_hex(bytes),
        asset_name()
    )
}

/// Mock release: `latest` metadata, the archive and `SHA256SUMS`.
fn serve(server: &MockServer, archive: Vec<u8>, sums: String) {
    let base = server.base_url();
    let release = serde_json::json!({
        "tag_name": TAG, "name": TAG, "created_at": "2026-11-04T00:00:00Z", "body": "",
        "assets": [
            {"name": asset_name(), "url": format!("{base}/assets/1")},
            {"name": "SHA256SUMS", "url": format!("{base}/assets/2")}
        ]
    });
    server.mock(|when, then| {
        when.method(GET).path("/repos/o/r/releases/latest");
        then.status(200).json_body(release.clone());
    });
    server.mock(|when, then| {
        when.method(GET).path("/repos/o/r/releases");
        then.status(200).json_body(serde_json::json!([release]));
    });
    server.mock(|when, then| {
        when.method(GET).path("/assets/1");
        then.status(200).body(archive);
    });
    server.mock(|when, then| {
        when.method(GET).path("/assets/2");
        then.status(200).body(sums);
    });
}

fn update(
    server: &MockServer,
    install: &Path,
) -> self_update::errors::Result<self_update::ReleaseStatus> {
    self_update::backends::github::Update::configure()
        .api_base_url(server.base_url())
        .repo_owner("o")
        .repo_name("r")
        .bin_name("mmhmem")
        .target(TARGET)
        .current_version("0.0.1")
        .bin_install_path(install)
        .verifying_keys([signing_key().verifying_key().to_bytes()])
        .checksum_from_asset("SHA256SUMS")
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(false)
        .build()?
        .update_extended()
}

fn installed_file(dir: &Path) -> std::path::PathBuf {
    let path = dir.join(format!("mmhmem{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&path, b"OLD").unwrap();
    path
}

#[test]
fn signed_archive_with_matching_sums_is_installed() {
    let server = MockServer::start();
    let bytes = signed(&archive(b"NEW"));
    serve(&server, bytes.clone(), sums_for(&bytes));
    let dir = tempfile::tempdir().unwrap();
    let path = installed_file(dir.path());

    let status = update(&server, &path).expect("signed update must succeed");
    assert!(status.is_updated(), "expected an update, got {status:?}");
    assert_eq!(std::fs::read(&path).unwrap(), b"NEW");
}

#[test]
fn unsigned_archive_is_refused_and_nothing_changes() {
    let server = MockServer::start();
    let bytes = archive(b"EVIL");
    serve(&server, bytes.clone(), sums_for(&bytes));
    let dir = tempfile::tempdir().unwrap();
    let path = installed_file(dir.path());

    let err = update(&server, &path).expect_err("unsigned archive must be refused");
    assert!(
        err.to_string().contains("could not read signatures"),
        "{err}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"OLD", "{err}");
}

#[test]
fn signed_archive_with_wrong_sums_is_refused() {
    let server = MockServer::start();
    let bytes = signed(&archive(b"NEW"));
    serve(&server, bytes, sums_for(b"something else"));
    let dir = tempfile::tempdir().unwrap();
    let path = installed_file(dir.path());

    let err = update(&server, &path).expect_err("checksum mismatch must be refused");
    assert!(err.to_string().contains("checksum mismatch"), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"OLD", "{err}");
}

#[test]
fn archive_signed_by_another_key_is_refused() {
    let server = MockServer::start();
    let mut out = Cursor::new(Vec::new());
    zipsign_api::sign::copy_and_sign_zip(
        &mut Cursor::new(archive(b"EVIL")),
        &mut out,
        &[SigningKey::from_bytes(&[9u8; 32])],
        Some(asset_name().as_bytes()),
    )
    .unwrap();
    let bytes = out.into_inner();
    serve(&server, bytes.clone(), sums_for(&bytes));
    let dir = tempfile::tempdir().unwrap();
    let path = installed_file(dir.path());

    let err = update(&server, &path).expect_err("foreign signature must be refused");
    assert!(err.to_string().contains("no matching key"), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"OLD", "{err}");
}
