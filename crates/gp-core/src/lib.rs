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
//! v0 targets the Orchard pool. The Sapling shape is mechanically the same
//! and lands in v0.2.

use orchard::{
    keys::OutgoingViewingKey,
    note::ExtractedNoteCommitment,
    note_encryption::OrchardDomain,
    value::ValueCommitment,
};
use thiserror::Error;
use zcash_note_encryption::{
    try_output_recovery_with_ock, Domain, EphemeralKeyBytes, OutgoingCipherKey,
};

/// Errors raised by gp-core.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("output recovery failed; the OCK does not match this output")]
    RecoveryFailed,
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
        let _: fn(&orchard::Action<()>, &OutgoingCipherKey) -> Result<OrchardDisclosure, CoreError> =
            recover_orchard::<()>;
    }
}
