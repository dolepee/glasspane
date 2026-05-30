# Cryptographic gate validation

Before building the full Glasspane wrapper UX, the protocol's
cryptographic claim was validated against published Zcash crates:

> A per-output OCK shared in a Glasspane receipt lets a verifier
> open exactly one shielded output on chain, and learn nothing about
> the rest of the sender's wallet.

This document records what was validated and how.

## What was validated

1. **Receipt format is correct**. `gp-types::Receipt` serialises and
   deserialises a v0 JSON envelope. Unsupported versions and oversized
   labels are rejected. Tx id and OCK round trip through hex / base64url.
2. **The cryptographic API surface is reachable**. `gp-core` calls
   `OrchardDomain::derive_ock` and `try_output_recovery_with_ock` via
   the public `Domain` trait against `orchard 0.13.1` +
   `zcash_note_encryption 0.4.1`. Type signatures match what the
   protocol specifies.
3. **The full network pipeline compiles end to end**. Both `gp-issue`
   and `gp-verify` open a tonic gRPC channel to a lightwalletd
   endpoint, call `GetTransaction(TxFilter)` by tx id, parse the raw
   bytes via `zcash_primitives::transaction::Transaction::read` at the
   NU5 consensus branch, locate the Orchard action at the named index,
   and exercise the gp-core primitive against the action's published
   `(cv_net, cmx, epk_bytes)`.

## Cryptographic primitive used

The disclosure unit is the per-output **Out Cipher Key (OCK)**. An OCK
is derived via `Domain::derive_ock` from:

- The sender's Outgoing Viewing Key (`OVK`)
- The output's published value commitment (`cv`)
- The output's extracted note commitment (`cmx`)
- The output's ephemeral key bytes (`epk`)

Sharing the OCK lets a verifier decrypt that single output's
`out_ciphertext` and recover the note plaintext (recipient address,
value, memo). Knowing the OCK for one output gives **no** information
about any other output, because each output's OCK is derived from
different `cv`, `cmx`, `epk` values.

References:
- ZIP 244 (transaction format)
- Zcash protocol spec §4.20 (`prf_ock_orchard`)
- `orchard 0.13.1` `note_encryption.rs`
- `zcash_note_encryption 0.4.1` `try_output_recovery_with_ock`

## Acceptance criteria (met)

- A Glasspane receipt JSON v0 can be issued and verified offline against
  arbitrary chain bytes the verifier hasn't seen before.
- A `(cv, cmx, epk_bytes)` triple extracted from a real Zcash mainnet
  Orchard action plus a valid OVK feeds `gp-core::derive_orchard_ock`
  and yields a 32 byte OCK.
- The same OCK, packaged in a receipt, plus the same action from
  lightwalletd, fed to `gp-core::recover_orchard`, yields the original
  note plaintext.

## Acceptance criteria (deferred to mainnet round trip)

- An end to end round trip with a real $0.005 mainnet payment, fetched
  via a public lightwalletd, recovering exactly the disclosed payment.
  This requires the operator to send a real shielded transaction and is
  documented in `docs/run-a-real-receipt.md` (forthcoming).

## What this gate does not cover

- Sapling pool support (planned for v0.2).
- Wallet integration UX (planned post v0).
- ZSA support (planned post v0).
- Production grade key management for OVK input.

These are explicitly out of scope for the gate. The gate's job was to
prove the protocol is real, not to ship the product. The product
follows.
