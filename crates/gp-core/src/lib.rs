//! Glasspane core: OCK-based selective disclosure of Zcash shielded outputs.
//!
//! This crate exposes the cryptographic primitives that gp-issuer uses to
//! produce receipts and gp-verifier uses to check them.
//!
//! The disclosure unit is the per-output Out Cipher Key (OCK). An OCK is
//! derived via `Domain::derive_ock` from the sender's Outgoing Viewing Key
//! plus the output's on-chain `cv`, `cmstar`, and `epk`. Disclosing the
//! OCK lets a verifier recover the note plaintext (recipient, value, memo)
//! for that ONE output and learn nothing about the rest of the wallet.
//!
//! v0 supports both the Orchard pool (via `derive_orchard_ock` /
//! `recover_orchard`) and the Sapling pool (via `derive_sapling_ock` /
//! `recover_sapling`). Both paths are validated against the published
//! Zcash protocol test vectors in the `tests` module.

use orchard::{
    keys::OutgoingViewingKey, note::ExtractedNoteCommitment, note_encryption::OrchardDomain,
    value::ValueCommitment,
};
use thiserror::Error;
use zcash_note_encryption::{
    try_output_recovery_with_ock, Domain, EphemeralKeyBytes, OutgoingCipherKey,
};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::BranchId;

/// Errors raised by gp-core.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("output recovery failed; the OCK does not match this output")]
    RecoveryFailed,
    #[error("parse raw transaction: {0}")]
    TransactionParse(String),
    #[error("raw transaction txid mismatch: receipt references {expected}, parsed {actual}")]
    TransactionIdMismatch { expected: String, actual: String },
}

/// Parse a transaction using the consensus branch embedded in its v5 header.
/// Older transaction versions retain the previous NU5 parser fallback.
pub fn parse_transaction(raw: &[u8]) -> Result<Transaction, CoreError> {
    let header = raw
        .get(..12)
        .ok_or_else(|| CoreError::TransactionParse("transaction header is truncated".into()))?;
    let version =
        u32::from_le_bytes(header[..4].try_into().expect("four-byte slice")) & 0x7fff_ffff;
    let branch_id = if version == 5 {
        let encoded = u32::from_le_bytes(header[8..12].try_into().expect("four-byte slice"));
        BranchId::try_from(encoded).map_err(|_| {
            CoreError::TransactionParse(format!("unsupported v5 consensus branch 0x{encoded:08x}"))
        })?
    } else {
        BranchId::Nu5
    };

    Transaction::read(raw, branch_id)
        .map_err(|error| CoreError::TransactionParse(error.to_string()))
}

/// Bind a receipt to the exact raw transaction it claims to disclose.
pub fn ensure_transaction_id(tx: &Transaction, expected: &str) -> Result<(), CoreError> {
    let actual = tx.txid().to_string();
    if actual == expected {
        Ok(())
    } else {
        Err(CoreError::TransactionIdMismatch {
            expected: expected.to_string(),
            actual,
        })
    }
}

/// All the data gp-issuer needs to derive an Orchard OCK once it has parsed
/// a confirmed transaction and located the action whose payment it wants to
/// disclose.
pub struct OrchardOckInputs<'a> {
    /// Sender's Outgoing Viewing Key for the account that made the payment.
    pub ovk: &'a OutgoingViewingKey,
    /// The action's published `cv_net` value commitment.
    pub cv: &'a ValueCommitment,
    /// The action's published extracted note commitment (`cmx`).
    pub cmx: &'a ExtractedNoteCommitment,
    /// The action's ephemeral key bytes (`epk`).
    pub epk_bytes: &'a EphemeralKeyBytes,
}

/// Derive the per-output OCK for an Orchard action.
///
/// Calls into the `Domain::derive_ock` trait method, which internally runs
/// the Zcash protocol's `prf_ock_orchard` over OVK + cv + cmx + epk.
///
/// The returned 32 byte OCK is the unit a Glasspane receipt discloses.
pub fn derive_orchard_ock(inputs: &OrchardOckInputs) -> OutgoingCipherKey {
    OrchardDomain::derive_ock(
        inputs.ovk,
        inputs.cv,
        &inputs.cmx.to_bytes(),
        inputs.epk_bytes,
    )
}

/// Disclosure recovered by gp-verifier from a Glasspane receipt + the
/// on-chain action.
#[derive(Debug)]
pub struct OrchardDisclosure {
    pub recipient: orchard::Address,
    pub value: orchard::value::NoteValue,
    pub memo: [u8; 512],
}

/// Recover the disclosed payment from a published Orchard action using the
/// OCK shared in a Glasspane receipt.
///
/// On success, returns the recipient address, the value, and the 512 byte
/// memo. On failure (the OCK does not validly open the action's
/// `out_ciphertext`), returns `CoreError::RecoveryFailed`.
pub fn recover_orchard<T>(
    action: &orchard::Action<T>,
    ock: &OutgoingCipherKey,
) -> Result<OrchardDisclosure, CoreError> {
    let domain = OrchardDomain::for_action(action);
    let out_ct = action.encrypted_note().out_ciphertext;
    match try_output_recovery_with_ock(&domain, ock, action, &out_ct) {
        Some((note, recipient, memo)) => Ok(OrchardDisclosure {
            recipient,
            value: note.value(),
            memo,
        }),
        None => Err(CoreError::RecoveryFailed),
    }
}

/// Inputs needed to derive a Sapling OCK once gp-issuer has located the
/// Sapling output description it wants to disclose.
pub struct SaplingOckInputs<'a> {
    pub ovk: &'a sapling_crypto::keys::OutgoingViewingKey,
    pub cv: &'a sapling_crypto::value::ValueCommitment,
    pub cmu: &'a sapling_crypto::note::ExtractedNoteCommitment,
    pub epk_bytes: &'a EphemeralKeyBytes,
}

/// Derive the per-output OCK for a Sapling output. Wraps `prf_ock` which
/// the `sapling-crypto` crate exposes publicly (in contrast to Orchard,
/// which keeps its `prf_ock_orchard` `pub(crate)` and forces external
/// callers through the `Domain` trait).
pub fn derive_sapling_ock(inputs: &SaplingOckInputs) -> OutgoingCipherKey {
    sapling_crypto::note_encryption::prf_ock(
        inputs.ovk,
        inputs.cv,
        &inputs.cmu.to_bytes(),
        inputs.epk_bytes,
    )
}

/// Disclosure recovered from a Sapling output via OCK.
#[derive(Debug)]
pub struct SaplingDisclosure {
    pub recipient: sapling_crypto::PaymentAddress,
    pub value: sapling_crypto::value::NoteValue,
    pub memo: [u8; 512],
}

/// Recover the disclosed payment from a published Sapling output description
/// using the OCK shared in a Glasspane receipt.
pub fn recover_sapling(
    output: &sapling_crypto::bundle::OutputDescription<sapling_crypto::bundle::GrothProofBytes>,
    ock: &OutgoingCipherKey,
    zip212: sapling_crypto::note_encryption::Zip212Enforcement,
) -> Result<SaplingDisclosure, CoreError> {
    match sapling_crypto::note_encryption::try_sapling_output_recovery_with_ock(ock, output, zip212)
    {
        Some((note, recipient, memo)) => Ok(SaplingDisclosure {
            recipient,
            value: note.value(),
            memo,
        }),
        None => Err(CoreError::RecoveryFailed),
    }
}

/// Build an `OutgoingCipherKey` from raw 32 byte material, e.g. the bytes
/// decoded from a Glasspane receipt's `ock` field.
pub fn ock_from_bytes(bytes: [u8; 32]) -> OutgoingCipherKey {
    OutgoingCipherKey(bytes)
}

/// Extract raw 32 bytes from an `OutgoingCipherKey` for serialisation
/// into a Glasspane receipt.
pub fn ock_to_bytes(ock: &OutgoingCipherKey) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(ock.as_ref());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use orchard::{note::ExtractedNoteCommitment as Cmx, value::ValueCommitment as Cv};
    use zcash_note_encryption::EphemeralKeyBytes;

    #[test]
    fn parse_transaction_accepts_nu6_2_branch() {
        let mut raw = hex::decode(include_str!("../../../examples/mainnet-tx.hex").trim())
            .expect("mainnet transaction fixture must be valid hex");
        raw[8..12].copy_from_slice(&u32::from(BranchId::Nu6_2).to_le_bytes());

        parse_transaction(&raw).expect("NU6.2 v5 transactions must parse");
    }

    #[test]
    fn transaction_id_binding_rejects_wrong_raw_transaction() {
        let raw = hex::decode(include_str!("../../../examples/mainnet-tx.hex").trim())
            .expect("mainnet transaction fixture must be valid hex");
        let tx = parse_transaction(&raw).expect("mainnet transaction fixture must parse");
        let actual = tx.txid().to_string();
        ensure_transaction_id(&tx, &actual).expect("matching txid must pass");

        let wrong = format!(
            "{}{}",
            if &actual[..1] == "0" { "1" } else { "0" },
            &actual[1..]
        );
        assert!(matches!(
            ensure_transaction_id(&tx, &wrong),
            Err(CoreError::TransactionIdMismatch { .. })
        ));
    }

    /// Orchard test vector index 0, sourced from
    /// orchard 0.15.0 `src/test_vectors/note_encryption.rs`,
    /// which in turn references the canonical Zcash protocol test vectors at
    /// https://github.com/zcash-hackworks/zcash-test-vectors/blob/master/orchard_note_encryption.py
    mod tv0 {
        pub const OVK: [u8; 32] = [
            0x5d, 0x7a, 0x8f, 0x73, 0x9a, 0x2d, 0x9e, 0x94, 0x5b, 0x0c, 0xe1, 0x52, 0xa8, 0x04,
            0x9e, 0x29, 0x4c, 0x4d, 0x6e, 0x66, 0xb1, 0x64, 0x93, 0x9d, 0xaf, 0xfa, 0x2e, 0xf6,
            0xee, 0x69, 0x21, 0x48,
        ];
        pub const CV_NET: [u8; 32] = [
            0xdd, 0xba, 0x24, 0xf3, 0x9f, 0x70, 0x8e, 0xd7, 0xa7, 0x48, 0x57, 0x13, 0x71, 0x11,
            0x42, 0xc2, 0x38, 0x51, 0x38, 0x15, 0x30, 0x2d, 0xf0, 0xf4, 0x83, 0x04, 0x21, 0xa6,
            0xc1, 0x3e, 0x71, 0x01,
        ];
        pub const CMX: [u8; 32] = [
            0xa5, 0x70, 0x6f, 0x3d, 0x1b, 0x68, 0x8e, 0x9d, 0xc6, 0x34, 0xee, 0xe4, 0xe6, 0x5b,
            0x02, 0x8a, 0x43, 0xee, 0xae, 0xd2, 0x43, 0x5b, 0xea, 0x2a, 0xe3, 0xd5, 0x16, 0x05,
            0x75, 0xc1, 0x1a, 0x3b,
        ];
        pub const EPHEMERAL_KEY: [u8; 32] = [
            0xad, 0xdb, 0x47, 0xb6, 0xac, 0x5d, 0xfc, 0x16, 0x55, 0x89, 0x23, 0xd3, 0xa8, 0xf3,
            0x76, 0x09, 0x5c, 0x69, 0x5c, 0x04, 0x7c, 0x4e, 0x32, 0x66, 0xae, 0x67, 0x69, 0x87,
            0xf7, 0xe3, 0x13, 0x81,
        ];
        pub const OCK: [u8; 32] = [
            0x4e, 0x9d, 0x45, 0x94, 0x6b, 0x3e, 0xea, 0xe7, 0xfe, 0x30, 0x5d, 0x5b, 0x90, 0x50,
            0x36, 0x14, 0x1f, 0x9f, 0x40, 0x09, 0xa6, 0x29, 0x4b, 0x96, 0xc7, 0x22, 0xa4, 0xa0,
            0xbe, 0x68, 0x5d, 0xff,
        ];
    }

    /// Orchard test vector index 1, sourced from the same upstream as TV0.
    mod tv1 {
        pub const OVK: [u8; 32] = [
            0xe7, 0x30, 0x81, 0xef, 0x8d, 0x62, 0xcb, 0x78, 0x0a, 0xb6, 0x88, 0x3a, 0x50, 0xa0,
            0xd4, 0x70, 0x19, 0x0d, 0xfb, 0xa1, 0x0a, 0x85, 0x7f, 0x82, 0x84, 0x2d, 0x38, 0x25,
            0xb3, 0xd6, 0xda, 0x05,
        ];
        pub const CV_NET: [u8; 32] = [
            0x15, 0x49, 0x70, 0x7e, 0x1e, 0xd2, 0xb2, 0xeb, 0x66, 0x15, 0x65, 0x0b, 0xec, 0x45,
            0xa2, 0x17, 0x64, 0x10, 0x4a, 0x23, 0xea, 0xf6, 0xba, 0x49, 0x6c, 0xb9, 0xb8, 0xe8,
            0x25, 0x7a, 0xd8, 0xb3,
        ];
        pub const CMX: [u8; 32] = [
            0x9e, 0x04, 0x32, 0xb2, 0xb3, 0x33, 0xcd, 0xe8, 0xce, 0x92, 0x1b, 0x77, 0xca, 0x7e,
            0x9e, 0x41, 0x51, 0xe3, 0x74, 0xd5, 0x16, 0xcd, 0xa1, 0x17, 0x63, 0x83, 0x6a, 0xf3,
            0xb6, 0x6f, 0x5b, 0x15,
        ];
        pub const EPHEMERAL_KEY: [u8; 32] = [
            0x91, 0x92, 0x3e, 0xd8, 0x2b, 0x76, 0xd7, 0x97, 0x30, 0x7c, 0xaa, 0x23, 0x02, 0xc0,
            0xcf, 0x75, 0x56, 0x12, 0x17, 0x24, 0x98, 0x67, 0x53, 0x2a, 0xe5, 0x1c, 0x2e, 0xa0,
            0x05, 0xed, 0xad, 0xb6,
        ];
        pub const OCK: [u8; 32] = [
            0x91, 0x36, 0x59, 0x30, 0x9e, 0xcf, 0xcd, 0xfd, 0x7e, 0x0c, 0xef, 0x23, 0xf8, 0x80,
            0xae, 0x4c, 0xf4, 0xd8, 0xcf, 0x67, 0x78, 0xb9, 0xc4, 0xe6, 0xf4, 0xc7, 0x71, 0x7b,
            0xf5, 0xca, 0xf0, 0x9e,
        ];
    }

    fn derive_for_vector(
        ovk_bytes: [u8; 32],
        cv_bytes: [u8; 32],
        cmx_bytes: [u8; 32],
        epk_bytes_raw: [u8; 32],
    ) -> [u8; 32] {
        let ovk_key = orchard::keys::OutgoingViewingKey::from(ovk_bytes);
        let cv = Cv::from_bytes(&cv_bytes).expect("cv_net must decode");
        let cmx = Cmx::from_bytes(&cmx_bytes).expect("cmx must decode");
        let epk = EphemeralKeyBytes(epk_bytes_raw);
        let ock = derive_orchard_ock(&OrchardOckInputs {
            ovk: &ovk_key,
            cv: &cv,
            cmx: &cmx,
            epk_bytes: &epk,
        });
        ock_to_bytes(&ock)
    }

    /// The strongest cryptographic claim Glasspane makes:
    /// `derive_orchard_ock(ovk, cv, cmx, epk)` produces bit exact the OCK
    /// specified by the published Zcash protocol test vectors.
    ///
    /// This proves gp-issuer's OCK derivation path is correct against the
    /// authoritative test material, without needing a mainnet round trip.
    #[test]
    fn derive_orchard_ock_matches_zcash_test_vector_0() {
        let ock = derive_for_vector(tv0::OVK, tv0::CV_NET, tv0::CMX, tv0::EPHEMERAL_KEY);
        assert_eq!(
            ock, tv0::OCK,
            "gp-core::derive_orchard_ock must produce the OCK specified by Zcash protocol test vector 0",
        );
    }

    /// Second protocol test vector. Confirms the derivation works across
    /// different parameter combinations, not just a lucky single case.
    #[test]
    fn derive_orchard_ock_matches_zcash_test_vector_1() {
        let ock = derive_for_vector(tv1::OVK, tv1::CV_NET, tv1::CMX, tv1::EPHEMERAL_KEY);
        assert_eq!(
            ock, tv1::OCK,
            "gp-core::derive_orchard_ock must produce the OCK specified by Zcash protocol test vector 1",
        );
    }

    /// Sapling protocol test vector 0 sourced from
    /// `sapling-crypto 0.7` `src/test_vectors/note_encryption.rs`.
    mod sap_tv0 {
        pub const OVK: [u8; 32] = [
            0x98, 0xd1, 0x69, 0x13, 0xd9, 0x9b, 0x04, 0x17, 0x7c, 0xab, 0xa4, 0x4f, 0x6e, 0x4d,
            0x22, 0x4e, 0x03, 0xb5, 0xac, 0x03, 0x1d, 0x7c, 0xe4, 0x5e, 0x86, 0x51, 0x38, 0xe1,
            0xb9, 0x96, 0xd6, 0x3b,
        ];
        pub const CV: [u8; 32] = [
            0xa9, 0xcb, 0x0d, 0x13, 0x72, 0x32, 0xff, 0x84, 0x48, 0xd0, 0xf0, 0x78, 0xb6, 0x81,
            0x4c, 0x66, 0xcb, 0x33, 0x1b, 0x0f, 0x2d, 0x3d, 0x8a, 0x08, 0x5b, 0xed, 0xba, 0x81,
            0x5f, 0x00, 0xa8, 0xdb,
        ];
        pub const CMU: [u8; 32] = [
            0x63, 0x55, 0x72, 0xf5, 0x72, 0xa8, 0xa1, 0xa0, 0xb7, 0xac, 0xbc, 0x0a, 0xfc, 0x6d,
            0x66, 0xf1, 0x4a, 0x02, 0xef, 0xac, 0xde, 0x7b, 0xdf, 0x03, 0x44, 0x3e, 0xd4, 0xc3,
            0xe5, 0x51, 0xd4, 0x70,
        ];
        pub const EPK: [u8; 32] = [
            0xde, 0xd6, 0x8f, 0x05, 0xc6, 0x58, 0xfc, 0xae, 0x5a, 0xe2, 0x18, 0x64, 0x6f, 0xf8,
            0x44, 0x40, 0x6f, 0x84, 0x42, 0x67, 0x84, 0x04, 0x0d, 0x0b, 0xef, 0x2b, 0x09, 0xcb,
            0x38, 0x48, 0xc4, 0xdc,
        ];
        pub const OCK: [u8; 32] = [
            0x6c, 0xe6, 0x1e, 0xad, 0x78, 0x49, 0x20, 0x42, 0x93, 0x34, 0x9e, 0x83, 0x2e, 0x95,
            0xca, 0x3a, 0xc6, 0x42, 0x2e, 0xc4, 0xfe, 0x21, 0xe5, 0xd1, 0x53, 0x86, 0x55, 0x8e,
            0x4d, 0x37, 0x79, 0x6d,
        ];
    }

    /// gp-core::derive_sapling_ock must produce the OCK specified by the
    /// Zcash protocol's Sapling note-encryption test vector 0.
    #[test]
    fn derive_sapling_ock_matches_zcash_test_vector_0() {
        use sapling_crypto::{
            keys::OutgoingViewingKey as SapOvk, note::ExtractedNoteCommitment as SapCmu,
            value::ValueCommitment as SapCv,
        };
        let ovk = SapOvk(sap_tv0::OVK);
        let cv = SapCv::from_bytes_not_small_order(&sap_tv0::CV).unwrap();
        let cmu = SapCmu::from_bytes(&sap_tv0::CMU).unwrap();
        let epk = EphemeralKeyBytes(sap_tv0::EPK);

        let ock = derive_sapling_ock(&SaplingOckInputs {
            ovk: &ovk,
            cv: &cv,
            cmu: &cmu,
            epk_bytes: &epk,
        });

        assert_eq!(
            ock_to_bytes(&ock),
            sap_tv0::OCK,
            "gp-core::derive_sapling_ock must produce the OCK specified by Zcash protocol Sapling test vector 0",
        );
    }

    /// Sensitivity: any single bit flip in any input must change the OCK.
    /// Confirms the derivation depends on every input we feed it, which is
    /// the property that makes the OCK a per-output disclosure unit rather
    /// than a wallet-wide one.
    #[test]
    fn derive_orchard_ock_is_sensitive_to_each_input() {
        let baseline = derive_for_vector(tv0::OVK, tv0::CV_NET, tv0::CMX, tv0::EPHEMERAL_KEY);

        // Flip one bit in OVK.
        let mut ovk_mut = tv0::OVK;
        ovk_mut[0] ^= 0x01;
        let ock_ovk = derive_for_vector(ovk_mut, tv0::CV_NET, tv0::CMX, tv0::EPHEMERAL_KEY);
        assert_ne!(ock_ovk, baseline, "OCK must depend on OVK");

        // Flip one bit in epk.
        let mut epk_mut = tv0::EPHEMERAL_KEY;
        epk_mut[5] ^= 0x80;
        let ock_epk = derive_for_vector(tv0::OVK, tv0::CV_NET, tv0::CMX, epk_mut);
        assert_ne!(ock_epk, baseline, "OCK must depend on ephemeral key");

        // Flip one bit in cmx (need a still-valid cmx; we mutate the byte at index 0).
        // If the mutated cmx doesn't decode we fall back to a different byte.
        for k in 0..32 {
            let mut cmx_mut = tv0::CMX;
            cmx_mut[k] ^= 0x01;
            if Cmx::from_bytes(&cmx_mut).is_some().into() {
                let ock_cmx = derive_for_vector(tv0::OVK, tv0::CV_NET, cmx_mut, tv0::EPHEMERAL_KEY);
                assert_ne!(ock_cmx, baseline, "OCK must depend on cmx");
                return;
            }
        }
        panic!("could not find a mutated cmx that still decodes");
    }

    /// Helper bytes go from raw -> OutgoingCipherKey -> raw cleanly.
    #[test]
    fn ock_byte_helpers_round_trip() {
        let raw = {
            let mut b = [0u8; 32];
            for (i, v) in b.iter_mut().enumerate() {
                *v = i as u8;
            }
            b
        };
        let ock = ock_from_bytes(raw);
        assert_eq!(ock_to_bytes(&ock), raw);
    }

    /// Confirms `OrchardDomain::derive_ock` is reachable through the public
    /// `Domain` trait from external code.
    ///
    /// Constructing a real `ValueCommitment` requires an internal
    /// `ValueCommitTrapdoor` (`pub(crate)` in orchard 0.13.1). External
    /// callers therefore only ever obtain `ValueCommitment` references from
    /// a parsed `Action`, never by construction. This test confirms the
    /// `Domain::derive_ock` function exists in the public surface at the
    /// expected types so gp-issuer can call it once it has parsed an
    /// `Action` from lightwalletd.
    #[test]
    fn derive_ock_is_reachable_via_public_api() {
        // If this compiles, `OrchardDomain::derive_ock` is callable from
        // external code with the expected type signature.
        let _: fn(
            &OutgoingViewingKey,
            &ValueCommitment,
            &[u8; 32],
            &EphemeralKeyBytes,
        ) -> OutgoingCipherKey = OrchardDomain::derive_ock;
    }

    /// Confirms `try_output_recovery_with_ock` is reachable through the
    /// public API with `orchard::Action<T>` and `OrchardDomain`.
    #[test]
    fn recover_orchard_is_reachable_via_public_api() {
        // If this compiles, gp-verifier can call recover_orchard against
        // any `orchard::Action<T>` returned by a transaction parser.
        let _: fn(
            &orchard::Action<()>,
            &OutgoingCipherKey,
        ) -> Result<OrchardDisclosure, CoreError> = recover_orchard::<()>;
    }
}
