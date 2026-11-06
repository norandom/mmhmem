//! Release signing public keys built into the binary (reliable-installers task 5.2).
//!
//! Every release archive carries an embedded zipsign (Ed25519ph) signature and
//! `SHA256SUMS.sig` is a detached signature over `SHA256SUMS`, both made with
//! the same key. Setup trusts a release only if one of these keys verifies it.
//!
//! The list is EMPTY on purpose: the project owner generates the real key pair
//! and adds the public key here (see `docs/release-signing.md`). Never commit a
//! private key. To add or rotate a key, append the 32 public key bytes as
//! `0xNN` values (the publisher parses exactly this format and refuses to
//! publish with a signing key that is not listed):
//!
//! ```text
//! pub const RELEASE_PUBLIC_KEYS: &[[u8; 32]] = &[
//!     [0x00, 0x01, /* ... 32 bytes ... */ 0x1f],
//! ];
//! ```
//!
//! Task 5.4 MUST refuse to update when this list is empty: `self_update`
//! silently skips signature verification when given no keys (research 11).

/// Trusted release verifying keys, raw 32-byte Ed25519 public keys.
pub const RELEASE_PUBLIC_KEYS: &[[u8; 32]] = &[[
    0x8f, 0xdf, 0xc1, 0x60, 0xd3, 0x40, 0xd3, 0x4c, 0xe1, 0xa5, 0x5f, 0x43, 0x6c, 0x81, 0x55, 0x26,
    0x89, 0x97, 0x20, 0x49, 0x54, 0x34, 0xda, 0x1d, 0xed, 0xdc, 0xf0, 0xc8, 0xf0, 0xea, 0xad, 0x50,
]];

/// The ephemeral key of one installer end-to-end run (feature
/// `e2e-release-key`, task 8.2), as 64 hex characters given at build time.
/// A missing variable is a compile error (`env!`), a malformed one too
/// ([`decode_hex_32`] runs at compile time).
#[cfg(feature = "e2e-release-key")]
const E2E_KEY_HEX: &str = env!("MMHMEM_E2E_RELEASE_PUBLIC_KEY");

/// The extra key an `e2e-release-key` build trusts. Never in a release:
/// the release build has no such feature, and `publish-ci-release.ps1`
/// refuses any binary that carries [`E2E_BUILD_MARKER`].
#[cfg(feature = "e2e-release-key")]
pub const E2E_RELEASE_PUBLIC_KEY: [u8; 32] = decode_hex_32(E2E_KEY_HEX);

/// Bytes only an `e2e-release-key` build contains; the publisher searches
/// every binary for them and refuses to publish one that has them.
#[cfg(feature = "e2e-release-key")]
#[used]
pub static E2E_BUILD_MARKER: [u8; 28] = *b"mmhmem-e2e-release-key-build";

/// Every key setup trusts for updates: [`RELEASE_PUBLIC_KEYS`], plus the
/// installer end-to-end key in an `e2e-release-key` build. Everything that
/// checks a release signature reads its keys here.
pub fn trusted_release_keys() -> Vec<[u8; 32]> {
    #[allow(unused_mut)]
    let mut keys = RELEASE_PUBLIC_KEYS.to_vec();
    #[cfg(feature = "e2e-release-key")]
    {
        // Reading the marker keeps it in the binary through linker GC.
        std::hint::black_box(&E2E_BUILD_MARKER);
        keys.push(E2E_RELEASE_PUBLIC_KEY);
    }
    keys
}

/// 64 hex characters (either case) to 32 bytes. Panics (a compile error in
/// a const) on any other length or a non-hex character.
pub const fn decode_hex_32(hex: &str) -> [u8; 32] {
    let bytes = hex.as_bytes();
    assert!(
        bytes.len() == 64,
        "MMHMEM_E2E_RELEASE_PUBLIC_KEY must be exactly 64 hex characters"
    );
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (hex_digit(bytes[2 * i]) << 4) | hex_digit(bytes[2 * i + 1]);
        i += 1;
    }
    out
}

const fn hex_digit(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => panic!("MMHMEM_E2E_RELEASE_PUBLIC_KEY must contain only hex characters"),
    }
}

#[cfg(test)]
pub(crate) mod test_keys {
    use zipsign_api::SigningKey;

    /// Deterministic throwaway key for tests only; never a release key.
    pub(crate) fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    pub(crate) fn public_key(seed: u8) -> [u8; 32] {
        signing_key(seed).verifying_key().to_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::RELEASE_PUBLIC_KEYS;
    use super::test_keys::{public_key, signing_key};
    use zipsign_api::Prehash;
    use zipsign_api::verify::{collect_keys, find_match, read_signatures};

    /// Detached signature exactly as `zipsign sign separate` produces it.
    fn sign_sums(seed: u8, sums: &[u8]) -> Vec<u8> {
        let mut prehash = Prehash::new();
        std::io::Write::write_all(&mut prehash, sums).unwrap();
        zipsign_api::sign::gather_signature_data(
            &[signing_key(seed)],
            &prehash,
            Some(b"SHA256SUMS"),
        )
        .unwrap()
    }

    fn verifies(keys: &[[u8; 32]], sig: &[u8], sums: &[u8]) -> bool {
        let keys = collect_keys(keys.iter().map(|k| Ok(*k))).unwrap();
        let signatures = read_signatures(&mut &sig[..]).unwrap();
        let prehash = Prehash::calculate(&mut &sums[..]).unwrap();
        find_match(&keys, &signatures, &prehash, Some(b"SHA256SUMS")).is_ok()
    }

    #[test]
    fn signature_verifies_only_against_a_list_containing_its_key() {
        let sums = b"abc  mmhmem-v1.0.0-x86_64-pc-windows-msvc.zip\n";
        let sig = sign_sums(1, sums);
        assert!(verifies(&[public_key(2), public_key(1)], &sig, sums));
        assert!(!verifies(&[public_key(2), public_key(3)], &sig, sums));
        let mut changed = sums.to_vec();
        changed[0] ^= 1;
        assert!(!verifies(&[public_key(1)], &sig, &changed));
    }

    #[test]
    fn decodes_exactly_64_hex_characters() {
        let hex = "00017f80ffABcdEF0123456789abcdef00000000000000000000000000000a0B";
        let key = super::decode_hex_32(hex);
        assert_eq!(key[..5], [0x00, 0x01, 0x7f, 0x80, 0xff]);
        assert_eq!(key[5..8], [0xab, 0xcd, 0xef]);
        assert_eq!(key[30..], [0x0a, 0x0b]);
        let pk = public_key(7);
        let text: String = pk.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(super::decode_hex_32(&text), pk);
    }

    #[test]
    #[should_panic(expected = "exactly 64 hex characters")]
    fn rejects_a_short_key() {
        super::decode_hex_32("abcd");
    }

    #[test]
    #[should_panic(expected = "only hex characters")]
    fn rejects_a_non_hex_key() {
        super::decode_hex_32(&"g".repeat(64));
    }

    #[cfg(not(feature = "e2e-release-key"))]
    #[test]
    fn a_normal_build_trusts_only_the_release_keys() {
        assert_eq!(super::trusted_release_keys(), RELEASE_PUBLIC_KEYS.to_vec());
    }

    #[cfg(feature = "e2e-release-key")]
    #[test]
    fn an_e2e_build_also_trusts_the_build_time_key() {
        let keys = super::trusted_release_keys();
        assert_eq!(keys.len(), RELEASE_PUBLIC_KEYS.len() + 1);
        assert_eq!(keys.last(), Some(&super::E2E_RELEASE_PUBLIC_KEY));
        assert_eq!(&super::E2E_BUILD_MARKER, b"mmhmem-e2e-release-key-build");
    }

    #[test]
    fn embedded_list_is_well_formed() {
        // Empty until the owner adds the real key; every entry must be a valid point.
        for key in RELEASE_PUBLIC_KEYS {
            zipsign_api::VerifyingKey::from_bytes(key).expect("valid Ed25519 public key");
        }
    }
}
