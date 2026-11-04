//! Offline tests of the verified release download (task 5.4): an `httpmock`
//! server plays GitHub, archives and `SHA256SUMS` are signed with throwaway
//! test keys. Every refusal leaves the install path byte-identical.

use std::path::{Path, PathBuf};

use httpmock::prelude::*;

use super::fixtures::{
    FakeRelease, TRUSTED, calls, sign_archive, sign_sums, source, tag, zip_archive,
};
use super::*;
use crate::setup::release_keys::test_keys::public_key;

fn trusted() -> Vec<[u8; 32]> {
    vec![public_key(5), public_key(TRUSTED)]
}

/// An existing install holding `OLD`.
fn old_install(dir: &Path) -> PathBuf {
    let path = dir.join("mmhmem-installed");
    std::fs::write(&path, b"OLD").unwrap();
    path
}

/// Run `update_binary` against `fake`, expect a refusal, and check the
/// install path is unchanged.
fn refused(fake: &FakeRelease, keys: &[[u8; 32]]) -> ReleaseError {
    let server = MockServer::start();
    let _mocks = fake.serve(&server);
    let dir = tempfile::tempdir().unwrap();
    let path = old_install(dir.path());
    let err = update_binary(&source(&server), &fake.tag, fake.target, keys, &path)
        .expect_err("the update must be refused");
    assert_eq!(std::fs::read(&path).unwrap(), b"OLD", "{err}");
    assert!(err.to_string().ends_with("Nothing was changed."), "{err}");
    err
}

#[test]
fn verified_zip_is_unpacked_and_installed() {
    let fake = FakeRelease::good(Target::WindowsX64, b"NEW");
    let server = MockServer::start();
    let _mocks = fake.serve(&server);

    let fetched = verify_and_fetch(&source(&server), &fake.tag, fake.target, &trusted()).unwrap();
    assert_eq!(std::fs::read(&fetched.binary).unwrap(), b"NEW");
    assert_eq!(fetched.binary_sha256, Sha256Hex::of(b"NEW"));
    assert_eq!(fetched.archive_sha256, Sha256Hex::of(&fake.archive));
    assert_eq!(
        fetched.archive_name,
        "mmhmem-v9.9.9-x86_64-pc-windows-msvc.zip"
    );

    let dir = tempfile::tempdir().unwrap();
    let path = old_install(dir.path());
    let installed =
        update_binary(&source(&server), &fake.tag, fake.target, &trusted(), &path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"NEW");
    assert_eq!(installed.sha256, Sha256Hex::of(b"NEW"));
    assert_eq!(installed.path, path);
    assert_eq!(installed.tag.as_str(), "v9.9.9");
}

#[test]
fn verified_tar_gz_is_unpacked_for_a_unix_target() {
    let fake = FakeRelease::good(Target::LinuxX64, b"NEW-UNIX");
    let server = MockServer::start();
    let _mocks = fake.serve(&server);
    let fetched = verify_and_fetch(&source(&server), &fake.tag, fake.target, &trusted()).unwrap();
    assert_eq!(std::fs::read(&fetched.binary).unwrap(), b"NEW-UNIX");
    assert!(fetched.binary.ends_with("mmhmem"));
}

#[test]
fn works_when_called_inside_a_tokio_runtime() {
    let fake = FakeRelease::good(Target::WindowsX64, b"NEW");
    let server = MockServer::start();
    let _mocks = fake.serve(&server);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let fetched = runtime
        .block_on(async { verify_and_fetch(&source(&server), &fake.tag, fake.target, &trusted()) })
        .unwrap();
    assert_eq!(std::fs::read(&fetched.binary).unwrap(), b"NEW");
}

#[test]
fn empty_key_list_is_refused_before_any_request() {
    let fake = FakeRelease::good(Target::WindowsX64, b"NEW");
    let server = MockServer::start();
    let mocks = fake.serve(&server);
    let dir = tempfile::tempdir().unwrap();
    let path = old_install(dir.path());

    let err = update_binary(&source(&server), &fake.tag, fake.target, &[], &path).unwrap_err();
    assert_eq!(err, ReleaseError::NoTrustedKeys);
    let err = verify_and_fetch(&source(&server), &fake.tag, fake.target, &[]).unwrap_err();
    assert_eq!(err, ReleaseError::NoTrustedKeys);
    assert_eq!(calls(&mocks), 0, "nothing may be requested without keys");
    assert_eq!(std::fs::read(&path).unwrap(), b"OLD");
    assert!(err.to_string().contains("Nothing was downloaded"), "{err}");
}

#[test]
fn the_embedded_key_list_is_used_as_is() {
    // Empty until the owner adds the release key: then setup must refuse.
    let keys = crate::setup::release_keys::trusted_release_keys();
    if keys.is_empty() {
        let fake = FakeRelease::good(Target::WindowsX64, b"NEW");
        let err = refused(&fake, &keys);
        assert_eq!(err, ReleaseError::NoTrustedKeys);
    }
}

#[test]
fn unsigned_archive_is_refused() {
    let mut fake = FakeRelease::good(Target::WindowsX64, b"EVIL");
    fake.archive = zip_archive("mmhmem.exe", b"EVIL");
    fake.resign_sums(&fake.archive.clone());
    let err = refused(&fake, &trusted());
    assert!(
        matches!(err, ReleaseError::BadArchiveSignature { .. }),
        "{err}"
    );
}

#[test]
fn archive_signed_by_an_unknown_key_is_refused() {
    let mut fake = FakeRelease::good(Target::WindowsX64, b"EVIL");
    let name = fake.archive_name();
    fake.archive = sign_archive(fake.target, &zip_archive("mmhmem.exe", b"EVIL"), &name, 9);
    fake.resign_sums(&fake.archive.clone());
    let err = refused(&fake, &trusted());
    assert!(
        matches!(err, ReleaseError::BadArchiveSignature { .. }),
        "{err}"
    );
}

#[test]
fn archive_signed_under_another_name_is_refused() {
    let mut fake = FakeRelease::good(Target::WindowsX64, b"EVIL");
    let raw = zip_archive("mmhmem.exe", b"EVIL");
    fake.archive = sign_archive(
        fake.target,
        &raw,
        "mmhmem-v0.0.1-x86_64-pc-windows-msvc.zip",
        1,
    );
    fake.resign_sums(&fake.archive.clone());
    let err = refused(&fake, &trusted());
    assert!(
        matches!(err, ReleaseError::BadArchiveSignature { .. }),
        "{err}"
    );
}

#[test]
fn sums_signature_with_one_flipped_byte_is_refused() {
    let mut fake = FakeRelease::good(Target::WindowsX64, b"NEW");
    let last = fake.sig.len() - 1;
    fake.sig[last] ^= 0x01;
    assert_eq!(refused(&fake, &trusted()), ReleaseError::BadSumsSignature);
}

#[test]
fn sums_signed_by_an_unknown_key_or_changed_after_signing_is_refused() {
    let mut fake = FakeRelease::good(Target::WindowsX64, b"NEW");
    fake.sig = sign_sums(&fake.sums, 9);
    assert_eq!(refused(&fake, &trusted()), ReleaseError::BadSumsSignature);

    let mut fake = FakeRelease::good(Target::WindowsX64, b"NEW");
    fake.sums.extend_from_slice(b"\n");
    assert_eq!(refused(&fake, &trusted()), ReleaseError::BadSumsSignature);

    let mut fake = FakeRelease::good(Target::WindowsX64, b"NEW");
    fake.sig = b"not a signature".to_vec();
    assert_eq!(refused(&fake, &trusted()), ReleaseError::BadSumsSignature);
}

#[test]
fn archive_not_matching_sums_shows_expected_and_actual() {
    let mut fake = FakeRelease::good(Target::WindowsX64, b"NEW");
    fake.resign_sums(b"some other archive");
    let err = refused(&fake, &trusted());
    let expected = Sha256Hex::of(b"some other archive").to_string();
    let actual = Sha256Hex::of(&fake.archive).to_string();
    assert_eq!(
        err,
        ReleaseError::HashMismatch {
            name: fake.archive_name(),
            expected: expected.clone(),
            actual: actual.clone(),
        }
    );
    let text = err.to_string();
    assert!(text.contains(&expected) && text.contains(&actual), "{text}");
}

#[test]
fn sums_without_an_entry_for_the_archive_is_refused() {
    let mut fake = FakeRelease::good(Target::WindowsX64, b"NEW");
    fake.sums = format!("{}  other.zip\n", "a".repeat(64)).into_bytes();
    fake.sig = sign_sums(&fake.sums, TRUSTED);
    let err = refused(&fake, &trusted());
    assert!(matches!(err, ReleaseError::NoSumsEntry { .. }), "{err}");
}

#[test]
fn missing_assets_are_refused() {
    for name in [
        "SHA256SUMS",
        "SHA256SUMS.sig",
        "mmhmem-v9.9.9-x86_64-pc-windows-msvc.zip",
    ] {
        let mut fake = FakeRelease::good(Target::WindowsX64, b"NEW");
        fake.not_found = Some(name);
        let err = refused(&fake, &trusted());
        assert!(
            matches!(&err, ReleaseError::Download { name: n, .. } if n == name),
            "{name}: {err}"
        );
        assert!(err.to_string().contains("HTTP 404"), "{err}");

        let mut fake = FakeRelease::good(Target::WindowsX64, b"NEW");
        fake.unlisted = Some(name);
        let err = refused(&fake, &trusted());
        assert_eq!(err, ReleaseError::MissingAsset { name: name.into() });
    }
}

#[test]
fn unknown_release_tag_is_a_lookup_error() {
    let fake = FakeRelease::good(Target::WindowsX64, b"NEW");
    let server = MockServer::start();
    let _mocks = fake.serve(&server);
    let dir = tempfile::tempdir().unwrap();
    let path = old_install(dir.path());
    let err = update_binary(
        &source(&server),
        &tag("v1.0.0"),
        fake.target,
        &trusted(),
        &path,
    )
    .unwrap_err();
    assert!(matches!(err, ReleaseError::Lookup { .. }), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"OLD");
}

#[test]
fn unsupported_platforms_have_no_release() {
    for (os, arch) in [
        ("linux", "aarch64"),
        ("windows", "aarch64"),
        ("freebsd", "x86_64"),
    ] {
        let err = Target::detect(os, arch).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "There is no mmhmem release for this computer ({os}, {arch}). Nothing was changed."
            )
        );
    }
    assert!(Target::from_triple("riscv64gc-unknown-linux-gnu").is_err());
    for target in Target::ALL {
        assert_eq!(Target::from_triple(target.triple()), Ok(target));
    }
    assert!(Target::current().is_ok(), "the test host has a release");
}

#[test]
fn archive_names_match_the_packaging_script() {
    let t = tag("v1.2.3");
    let names: Vec<String> = Target::ALL.iter().map(|x| x.archive_name(&t)).collect();
    assert_eq!(
        names,
        [
            "mmhmem-v1.2.3-x86_64-pc-windows-msvc.zip",
            "mmhmem-v1.2.3-x86_64-unknown-linux-musl.tar.gz",
            "mmhmem-v1.2.3-x86_64-apple-darwin.tar.gz",
            "mmhmem-v1.2.3-aarch64-apple-darwin.tar.gz",
        ]
    );
}

#[test]
fn status_hash_reports_match_mismatch_and_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mmhmem.exe");
    std::fs::write(&path, b"NEW").unwrap();
    let recorded = Sha256Hex::of(b"NEW");
    assert_eq!(status_hash(&path, &recorded).unwrap(), BinaryStatus::Match);

    std::fs::write(&path, b"CHANGED").unwrap();
    assert_eq!(
        status_hash(&path, &recorded).unwrap(),
        BinaryStatus::Mismatch {
            expected: recorded.clone(),
            actual: Sha256Hex::of(b"CHANGED"),
        }
    );
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        status_hash(&path, &recorded).unwrap(),
        BinaryStatus::Missing
    );
}

#[test]
fn latest_release_comes_from_the_api() {
    let fake = FakeRelease::good(Target::WindowsX64, b"NEW");
    let server = MockServer::start();
    let _mocks = fake.serve(&server);
    let latest = latest_release(&source(&server), fake.target).unwrap();
    assert_eq!(latest.tag.as_str(), "v9.9.9");
    assert_eq!(latest.version, "9.9.9");
    assert_eq!(latest.via, LookupVia::Api);
    assert!(latest.assets.contains(&"SHA256SUMS.sig".to_string()));
    assert!(latest.assets.contains(&fake.archive_name()));
}

#[test]
fn latest_release_falls_back_to_the_redirect_when_the_api_fails() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/repos/o/r/releases/latest");
        then.status(403).header("x-ratelimit-remaining", "0");
    });
    server.mock(|when, then| {
        when.method(GET).path("/o/r/releases/latest");
        then.status(302).header(
            "location",
            format!("{}/o/r/releases/tag/v1.4.0", server.base_url()),
        );
    });
    let latest = latest_release(&source(&server), Target::LinuxX64).unwrap();
    assert_eq!(latest.tag.as_str(), "v1.4.0");
    assert_eq!(latest.via, LookupVia::Redirect);
    assert_eq!(
        latest.assets,
        [
            "mmhmem-v1.4.0-x86_64-unknown-linux-musl.tar.gz",
            "SHA256SUMS",
            "SHA256SUMS.sig",
        ]
    );
}

#[test]
fn latest_release_errors_are_plain_when_both_lookups_fail() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.any_request();
        then.status(500).body("<html>secret server page</html>");
    });
    let err = latest_release(&source(&server), Target::LinuxX64).unwrap_err();
    assert!(matches!(err, ReleaseError::Lookup { .. }), "{err}");
    assert!(!err.to_string().contains("secret"), "{err}");
}

#[test]
fn only_stable_tags_count_as_releases() {
    assert!(latest::stable_tag("v1.2.3").is_ok());
    assert!(latest::stable_tag("v0.10.0").is_ok());
    for bad in ["v1.2.3-rc.1", "v1.2", "1.2.3", "v01.2.3", "v1.2.3+b"] {
        assert!(latest::stable_tag(bad).is_err(), "{bad}");
    }
}

#[test]
fn sums_entries_are_parsed_strictly() {
    let a = "a".repeat(64);
    let b = "B".repeat(64);
    let text = format!("{a}  x.zip\n{b} *y.tar.gz\n");
    assert_eq!(
        sums::sums_entry(text.as_bytes(), "x.zip").unwrap().as_str(),
        a
    );
    assert_eq!(
        sums::sums_entry(text.as_bytes(), "y.tar.gz")
            .unwrap()
            .as_str(),
        "b".repeat(64)
    );
    assert!(sums::sums_entry(text.as_bytes(), "z.zip").is_err());
    let twice = format!("{a}  x.zip\n{a}  x.zip\n");
    assert!(sums::sums_entry(twice.as_bytes(), "x.zip").is_err());
    assert!(sums::sums_entry(b"abc  x.zip\n", "x.zip").is_err());
}

#[test]
fn archive_gate_refuses_a_file_not_matching_the_verified_sums() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.zip");
    std::fs::write(&path, b"bytes").unwrap();
    assert!(
        ArchiveGate::new("a.zip", Sha256Hex::of(b"bytes"))
            .check(&path)
            .is_ok()
    );
    assert!(
        ArchiveGate::new("b.zip", Sha256Hex::of(b"bytes"))
            .check(&path)
            .is_err()
    );
    let gate = ArchiveGate::new("a.zip", Sha256Hex::of(b"other"));
    assert!(gate.check(&path).is_err());
    let (expected, actual) = gate.mismatch().unwrap();
    assert_eq!(expected, Sha256Hex::of(b"other").to_string());
    assert_eq!(actual, Sha256Hex::of(b"bytes").to_string());
}

#[test]
fn a_local_release_folder_is_served_to_the_same_checks() {
    let fake = FakeRelease::good(Target::WindowsX64, b"NEW");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(fake.archive_name()), &fake.archive).unwrap();
    std::fs::write(dir.path().join(SUMS_NAME), &fake.sums).unwrap();
    std::fs::write(dir.path().join(SUMS_SIG_NAME), &fake.sig).unwrap();
    let local = LocalRelease::serve(dir.path()).unwrap();
    let latest = latest_release(local.source(), Target::WindowsX64).unwrap();
    assert_eq!(latest.tag, fake.tag);
    let path = old_install(dir.path());
    let done = update_binary(local.source(), &fake.tag, fake.target, &trusted(), &path).unwrap();
    assert_eq!(std::fs::read(&done.path).unwrap(), b"NEW");
    let err = update_binary(
        local.source(),
        &tag("v1.0.0"),
        fake.target,
        &trusted(),
        &path,
    )
    .expect_err("an unknown tag is a lookup error");
    assert!(matches!(err, ReleaseError::Lookup { .. }), "{err}");
    assert!(LocalRelease::serve(&dir.path().join("missing")).is_err());
}
