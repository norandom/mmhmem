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
pub const RELEASE_PUBLIC_KEYS: &[[u8; 32]] = &[
    // Intentionally empty until the owner generates the release key.
];

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
    fn embedded_list_is_well_formed() {
        // Empty until the owner adds the real key; every entry must be a valid point.
        for key in RELEASE_PUBLIC_KEYS {
            zipsign_api::VerifyingKey::from_bytes(key).expect("valid Ed25519 public key");
        }
    }
}
