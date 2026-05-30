# 24 hour cryptographic gate

Single objective: prove that a Glasspane receipt can be generated and verified end to end on Zcash mainnet using existing librustzcash crates.

If this passes by hour 24 (worst case 48), Glasspane is real and we build the wrapper UX. If not, pivot to SIWZ.

## The crypto path is now identified

Sender side:
1. Sender knows their OVK (Outgoing Viewing Key) for the account that made the payment.
2. For the specific output to disclose, the sender computes `OCK = prf_ock(ovk, cv, cm_u_or_full, epk)` using the output's on-chain `cv`, `cm`, and `epk`. This recreates the per-output cipher key.
3. Sender packages `tx_id`, `output_index`, `pool`, and the 32 byte `OCK` into a receipt JSON.

Verifier side:
1. Verifier fetches the transaction via lightwalletd.
2. Verifier extracts the named output from the named pool.
3. Verifier calls `try_output_recovery_with_ock(disclosed_ock, output_data, out_ciphertext)`.
4. If recovery returns `Some(plaintext)`, the receipt is valid. Display the plaintext.

Crates needed:
- `sapling-crypto` for Sapling outputs (`try_sapling_output_recovery_with_ock`)
- `orchard` for Orchard outputs (`try_output_recovery_with_ock` via `OrchardDomain`)
- `zcash_note_encryption` shared trait
- `zcash_client_backend` for lightwalletd block / tx fetching
- `zcash_primitives` for transaction parsing

## Acceptance criteria

By the end of hour 24:

1. A real shielded payment exists on Zcash mainnet, sent from a wallet the operator controls. Tx hash recorded.
2. A small Rust binary, given the sender's OVK plus the tx hash and output index, computes the OCK and outputs a Glasspane receipt JSON file.
3. A second small Rust binary, given ONLY the receipt JSON file and a lightwalletd endpoint, verifies the receipt and prints the recovered note plaintext (recipient address, value, memo).
4. The verifier binary has NO wallet keys loaded at runtime. The OCK in the receipt is the only secret material it sees.
5. Repeat verification with the same receipt produces identical output deterministically.

## Tooling shortlist (try in this order)

1. **librustzcash + sapling-crypto + orchard crates**: the most direct path. Recompute OCK from OVK + chain data; verify via `try_output_recovery_with_ock`. Pure Rust, no custom protocol work.
2. **zcash-devtool** if it exposes an OCK helper: long shot but worth a `grep ock` in the binary.
3. **Custom send via `zcash_client_backend`**: as last resort, build the payment ourselves so we have the OVK and output material directly in hand.

## What we expect to learn

- The OCK derivation is documented in the Zcash protocol spec. The hardest part is wiring `prf_ock` correctly for each pool and parsing the right on-chain values.
- Mapping a chain transaction to per-output `(cv, cm, epk)` requires the Sapling output description or Orchard action data. Both are exposed in `zcash_primitives::transaction`.
- The verifier needs to access the on-chain output's `out_ciphertext` and `cv` cleanly. lightwalletd's `GetTransaction` response should suffice.

## Decision tree at hour 12

- Recompute OCK from a known OVK + a known mainnet tx succeeds: continue, target full receipt generation by hour 18.
- Cannot recompute OCK because of pool parsing or `prf_ock` mismatch: pause, switch to dumping plaintext via `try_output_recovery_with_ovk` directly (still works, just leaks more — share the OVK only for the demo, document it as v0.1 not v0).
- Cannot do either: prepare SIWZ fallback in parallel, decide at hour 18.

## Decision tree at hour 24

- All five acceptance criteria met: lock Glasspane v0 spec, begin wrapper build.
- Receipt generation works but verifier is slow / brittle: ship with caveats, fix in week 2.
- One pool works but not the other (Orchard works, Sapling doesn't or vice versa): pick the working pool, document scope.
- Neither pool works deterministically: pivot to SIWZ at hour 24. No further extension.

## What this gate does NOT validate

- Wallet UX for the sender side (deferred).
- Verifier web UI (deferred).
- Real-world user testing (deferred).

The gate is cryptography only.

## What the operator needs ready

- A Zcash mainnet wallet they control (Zashi, YWallet, Zingo, or zcashd-based).
- The OVK or full spending key for that account.
- ~0.005 ZEC to send a real shielded payment (under $3).
- A working Rust toolchain.
- Access to a public lightwalletd endpoint (e.g. zec.rocks, or run a small one locally).
