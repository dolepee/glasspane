# Glasspane Receipt Format v0

A Glasspane receipt is a verifiable claim that a specific shielded Zcash payment was made. The receipt is portable: any party can verify it against Zcash mainnet without needing access to the sender's wallet.

## Cryptographic primitive

Glasspane uses the **per-output Out Cipher Key (OCK)** as the disclosure unit.

In Sapling and Orchard, every shielded output has an `out_ciphertext` that the sender themselves can decrypt using a per-output key called the OCK. The OCK is derived from the sender's Outgoing Viewing Key (OVK) plus the output's on-chain `cv`, `cm`, and `epk` per the Zcash protocol's `prf_ock` function. Importantly: **OCK is unique per output**. Knowing the OCK for one output does NOT let anyone decrypt any other output.

A Glasspane receipt discloses the OCK for one specific output. A verifier fetches that output from chain, runs `try_output_recovery_with_ock`, gets the note plaintext (recipient address, value, memo), and learns exactly that payment. The rest of the sender's wallet remains opaque.

## Format

JSON object with the following fields. Field names are lowercase, snake case. Binary fields are base64url encoded.

```json
{
  "version": "0",
  "network": "mainnet",
  "pool": "orchard",
  "tx_id": "<hex string, 32 bytes>",
  "output_index": <uint>,
  "ock": "<base64url, 32 bytes>",
  "label": "<utf-8 string, up to 120 chars>",
  "issued_at": "<RFC3339 timestamp>",
  "signature": {
    "scheme": "ed25519",
    "public_key": "<base64url, 32 bytes>",
    "sig": "<base64url, 64 bytes>"
  }
}
```

### Pool

`"orchard"` for Orchard outputs (default for new Zcash payments).
`"sapling"` for legacy Sapling outputs (supported for older wallets).

### Label

Free text the issuer can set, e.g. "donation to ZecHub" or "May rent receipt". Not validated by the verifier. Cosmetic.

### Signature

Optional in v0. Signs `tx_id || output_index || ock || label` with an ed25519 key the issuer chooses. Provides authorship attribution to the receipt envelope (the receipt was issued by someone holding that key). Does NOT prove anything about the underlying payment beyond what chain verification already provides.

## Verification procedure

Given a Glasspane receipt and Zcash mainnet access (via lightwalletd or a Zcash full node):

1. **Resolve the transaction.** Fetch the transaction with id `tx_id`. Fail if not found or unconfirmed.
2. **Locate the output.** Within the transaction, find the output at `output_index` in the named `pool` (Orchard action or Sapling output). Fail if absent or pool mismatch.
3. **Recover the note plaintext.** Call `try_output_recovery_with_ock(disclosed_ock, output, output.out_ciphertext())` for the named pool. Fail if recovery returns None (means the OCK does not match this output).
4. **Display.** Show the recovered note plaintext to the verifier:
   - Recipient diversified address (encoded as a UA where applicable)
   - Value in zatoshis (and converted to ZEC)
   - Memo bytes (rendered as UTF-8 text where possible)
5. **(Optional) Verify signature.** If `signature` is present, verify it over `tx_id || output_index || ock || label`.

If steps 1 through 3 pass: the receipt is valid. The verifier now knows the disclosed payment exists on Zcash mainnet exactly as the OCK recovery shows, and nothing else about the sender or the rest of their wallet.

## What the verifier learns

After successful verification:
- A payment of `value_zatoshis` was made to `recipient_address` in transaction `tx_id` at output index `output_index`.
- The memo attached to that output (cleartext after OCK decryption).
- The block in which the transaction was confirmed.

The verifier learns nothing else: not the sender's address, not other outputs in the same transaction (each has its own OCK), not the sender's wallet balance, not any other transaction.

## What the verifier does NOT learn

- The sender's UA, IVK, FVK, OVK, or any other wallet key material.
- Any other payments by the sender.
- Whether the sender controls other outputs in the same transaction (unless the sender also shares those OCKs).
- The sender's transaction history.

## Why OCK and not full viewing key

Full Viewing Key (FVK) and Incoming Viewing Key (IVK) are scoped to an entire account or address. Sharing them leaks ALL incoming or all incoming + outgoing for that key. They are not safe disclosure units for a per-payment receipt.

OCK is per-output. It is the smallest cryptographic disclosure unit that lets a verifier check a specific outgoing payment. This is the right granularity for Glasspane.

## Threat model boundary

A Glasspane receipt deliberately discloses one output. Anyone with the receipt can re-verify forever. Receipts are NOT revocable once shared. Receipts are forwardable: a recipient can pass the receipt to any third party who can also re-verify.

The sender should treat a receipt as: "any party with this file knows about this payment, permanently."

## Versioning

Field `version` is "0" during the hackathon. Breaking changes will bump the integer. Future versions may add:
- ZSA-aware decryption (asset id, asset value scaling)
- ZK proof of a property OVER the disclosed amount (e.g., "this payment was greater than 1 ZEC" without revealing the amount)
- Batched receipts (multiple outputs in one envelope, each with its own OCK)
- Time-locked verification windows (receipt is only valid between dates)
