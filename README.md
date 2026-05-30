# Glasspane

> Prove you made a specific Zcash payment, to a specific party, without revealing the rest of your wallet.

[![CI](https://github.com/dolepee/glasspane/actions/workflows/ci.yml/badge.svg)](https://github.com/dolepee/glasspane/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Web verifier](https://img.shields.io/badge/web-verifier-blue)](https://dolepee.github.io/glasspane/)

Glasspane is a portable receipt format and toolchain for **per-payment selective disclosure** on Zcash. A sender who made a shielded payment can issue a Glasspane receipt that lets any third party verify exactly that one transaction against Zcash mainnet, while learning nothing else about the sender's wallet.

The cryptography is the per-output **Out Cipher Key (OCK)** that already exists inside the Zcash protocol. Sharing the OCK for one output is the smallest, cleanest disclosure unit Zcash provides. We've packaged it.

* Web verifier (paste a receipt, see the envelope, get the CLI command): **https://dolepee.github.io/glasspane/**
* End-to-end tutorial: [`docs/run-a-real-receipt.md`](docs/run-a-real-receipt.md)
* Receipt format spec: [`spec/receipt.md`](spec/receipt.md)
* Engineering validation report: [`spec/24h-gate.md`](spec/24h-gate.md)

## Why this needs Zcash specifically

Bitcoin has no encrypted memos and no shielded receivers, so any "receipt" leaks the entire payment graph. Monero has no clean per-output selective disclosure primitive. Ethereum needs ZK add-ons to do any of this. Zcash's Sapling and Orchard pools already publish a per-output `out_ciphertext` that is encrypted under a per-output OCK derived from the sender's Outgoing Viewing Key. The protocol-level primitive for "show one party one specific payment without showing them the rest of your account" is **already in the chain**. It just hasn't had end-user tooling until now.

## Use cases

* Charity donor receipts. Prove the $10 donation without doxxing your wallet.
* Tax reporting. Hand your accountant a verifiable proof of a specific payment without giving them your viewing key.
* Investigative journalism expense reports. Verifiable on chain to an editor; opaque to anyone else.
* Freelance payment proofs to a regulator or counterparty.
* Self-sovereign records: own the cryptographic evidence of your past transactions without being doxxable.

## What the receipt actually discloses

For one shielded payment, the receipt contains:

* Transaction id (32 bytes, hex)
* Output / action index inside that transaction
* Pool: `orchard` or `sapling`
* The per-output **OCK** (32 bytes, base64url)
* Optional 120-char label + issued-at timestamp
* Optional ed25519 signature over the envelope (`tx_id || output_index || ock || label`)

A verifier with this receipt can fetch the named transaction from any Zcash full node or lightwalletd, run `try_output_recovery_with_ock(ock, action_or_output)` on the indexed output, and recover the note plaintext: recipient address, value, and 512-byte memo. **They learn nothing else.** The OCK for the disclosed output does not unlock any other output, in this transaction or anywhere else.

The full threat-model boundary (what receipts do and do not prove) is in [`spec/receipt.md`](spec/receipt.md).

## Status

Production-shape v0 across two pools.

| Capability | Status |
|---|---|
| Receipt format v0 (Orchard + Sapling) | shipped |
| `gp-issue`: derive OCK from on-chain action + sender's OVK | shipped |
| `gp-verify`: fetch tx via lightwalletd + recover via OCK | shipped |
| Receipt URL form (`https://host/r/<base64url>`) | shipped |
| Bech32m UA encoding for recovered recipients (`u1...`) | shipped |
| Optional ed25519 receipt signing | shipped |
| Static web verifier (envelope decode + tx explorer link) | shipped, [live](https://dolepee.github.io/glasspane/) |
| Protocol-correctness tests against published Zcash test vectors | Orchard TV0 + TV1, Sapling TV0 |
| Input-sensitivity tests (bit flips must change the OCK) | shipped |
| CI (cargo fmt + clippy `-D warnings` + tests on every push) | shipped |
| WASM in-browser cryptographic recovery | roadmap |
| ZSA-aware receipts | roadmap |

15 tests passing across 4 crates.

## Quickstart

### 1. Build the CLI

```bash
git clone https://github.com/dolepee/glasspane.git
cd glasspane
cargo build --release
```

Two binaries land in `target/release/`: `gp-issue` and `gp-verify`.

### 2. Issue a receipt for a shielded payment you sent

You need: the transaction id (from your wallet), the output index for the action that landed at your receiver (`0` for single-output sends), and your Outgoing Viewing Key as 32 bytes hex. See [`docs/run-a-real-receipt.md`](docs/run-a-real-receipt.md) for how to extract the OVK from Zashi, YWallet, Zingo, or zcashd.

```bash
./target/release/gp-issue \
  --pool orchard \
  --tx-id <your-tx-id> \
  --output-index 0 \
  --ovk <your-32-byte-ovk> \
  --label "first glasspane receipt" \
  --out receipt.json \
  --url
```

Add `--sign-with-key <32-hex>` to sign the envelope.

### 3. Verify the receipt

```bash
./target/release/gp-verify receipt.json
```

`gp-verify` also accepts a Glasspane URL (`https://host/r/<base64url>`), a bare base64url payload, or stdin. On success you see the recovered recipient UA, the value in zatoshis and ZEC, the memo, and (if signed) `signature : ed25519 OK`. On a wrong OCK, output index, or tampered receipt, you see a clear failure.

### 4. Or share the URL form

Hand the contents of `receipt.json` to anyone with `gp-verify` installed, or share the URL emitted on stderr by `gp-issue --url`. Anyone with the URL can paste it into the **[web verifier](https://dolepee.github.io/glasspane/)** to see the envelope, or into `gp-verify` to do the full cryptographic recovery.

## How the protocol works

1. Each Zcash shielded output publishes a value commitment `cv`, an extracted note commitment `cmstar` (`cmx` for Orchard, `cmu` for Sapling), and an ephemeral key `epk`. It also publishes two ciphertexts: `enc_ciphertext` (encrypted to the recipient under their Incoming Viewing Key) and `out_ciphertext` (encrypted to the sender under the **OCK**).
2. The OCK is `prf_ock(ovk, cv, cmstar_bytes, epk)`. It is **unique per output** because `cv`, `cmstar`, `epk` are unique per output. Knowing the OCK for output A reveals nothing about the OCK for output B.
3. A Glasspane receipt discloses one OCK. A verifier runs `try_output_recovery_with_ock(domain, ock, output, out_ciphertext)` to decrypt the note plaintext for exactly that output. The other outputs in the same transaction stay opaque under their own (unshared) OCKs.

This is exactly the protocol-level mechanism the Zcash spec describes (`§4.20 prf_ock_orchard`, `§4.19 prf_ock_sapling`). Our test suite verifies bit-exact agreement against the published Zcash protocol test vectors.

## Architecture

```
glasspane/
├── crates/
│   ├── gp-types/     # Receipt format, URL encoding, ed25519 signing
│   ├── gp-core/      # OCK derive + recover for Orchard + Sapling
│   ├── gp-issuer/    # gp-issue binary
│   └── gp-verifier/  # gp-verify binary
├── spec/
│   ├── receipt.md    # Protocol spec for the v0 receipt format
│   └── 24h-gate.md   # Engineering validation report
├── docs/
│   └── run-a-real-receipt.md   # End-to-end mainnet tutorial
├── web/
│   └── index.html    # Static web verifier (GitHub Pages)
└── .github/workflows/
    ├── ci.yml        # fmt + clippy + tests on every push
    └── pages.yml     # Deploys web/ to github.io
```

`gp-types` and `gp-core` are pure libraries with no network dependencies. They can be embedded in other Rust tooling without pulling tonic, lightwalletd, or async runtimes.

## Tests + protocol correctness

```bash
cargo test --workspace
```

15 tests across:

* Receipt format (envelope validate, version reject, label length, JSON round trip, URL round trip, bare-payload URL parse, garbage URL reject, ed25519 signature round trip, ed25519 tampering reject).
* OCK derivation (byte helper round trip, Orchard test vectors 0 and 1 bit-exact, Sapling test vector 0 bit-exact, input sensitivity: any bit flip in OVK / epk / cmx must change the OCK).
* API surface (Orchard `derive_ock` and `try_output_recovery_with_ock` reachable through the published `Domain` trait at expected signatures).

`cargo clippy --workspace --all-targets -- -D warnings` is clean. CI runs all three checks (fmt, clippy, tests) on every push to `main`.

## Cryptographic primitives + versions

| Component | Crate | Version | Role |
|---|---|---|---|
| Orchard note encryption + OCK | `orchard` | 0.13.1 | per-output `prf_ock_orchard`, `Domain::derive_ock` |
| Sapling note encryption + OCK | `sapling-crypto` | 0.7 | per-output `prf_ock`, `try_sapling_output_recovery_with_ock` |
| Shared ZNE machinery | `zcash_note_encryption` | 0.4 | `try_output_recovery_with_ock` |
| Transaction parsing | `zcash_primitives` | 0.27 | `Transaction::read` |
| lightwalletd gRPC client | `zcash_client_backend` | 0.22 | `CompactTxStreamerClient::get_transaction` |
| Unified Address encoding | `zcash_address` | 0.11 | recipient display in `u1...` form |
| Receipt signing | `ed25519-dalek` | 2 | optional envelope signature |

All versions are pinned to released-on-crates.io releases for reproducibility.

## Roadmap (post v0)

* WASM-compile `gp-core` and ship the full cryptographic recovery in the browser, so the [web verifier](https://dolepee.github.io/glasspane/) does the whole thing without a separate CLI step.
* ZSA-aware receipts (when Zcash Shielded Assets ship a stable application surface).
* ZK proof of a property *over* the disclosed value ("this payment was greater than 1 ZEC") without revealing the amount.
* Batched receipts (multiple outputs in one envelope, each with its own OCK).

## References

* ZIP-244 (transaction format), ZIP-316 (Unified Addresses)
* Zcash Protocol Specification §4.19 (`prf_ock_sapling`), §4.20 (`prf_ock_orchard`)
* `zcash-test-vectors/orchard_note_encryption.py` — protocol-level test vectors
* `zcash-test-vectors/sapling_note_encryption.py` — protocol-level test vectors

## License

MIT. See [LICENSE](LICENSE).

Submission for **ZecHub Hackathon 3.0** (May 25 – July 15, 2026).
