# Glasspane Rooms

> A private payout board for Zcash teams.

[![CI](https://github.com/dolepee/glasspane/actions/workflows/ci.yml/badge.svg)](https://github.com/dolepee/glasspane/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rooms board](https://img.shields.io/badge/rooms-board-blue)](https://glasspane-iota.vercel.app/room/zechub-demo)

Glasspane Rooms lets a Zcash team prove selected shielded payouts, memos,
and totals without handing over a viewing key or exposing the rest of the
treasury.

The public board is simple: green rows are selected payouts recovered from
Zcash mainnet, the red row is a deliberately tampered receipt rejected by the
verifier, and the wallet remainder stays opaque.

* Website: **https://glasspane-iota.vercel.app/**
* Rooms board: **https://glasspane-iota.vercel.app/room/zechub-demo**
* Self-serve renderer: **https://glasspane-iota.vercel.app/room/create**
* Room schema: [`spec/room.md`](spec/room.md)
* Receipt format spec: [`spec/receipt.md`](spec/receipt.md)
* Mainnet receipt tutorial: [`docs/run-a-real-receipt.md`](docs/run-a-real-receipt.md)

Submission for **ZecHub Hackathon 3.0**, Accounting track.

## What it proves

For each selected payout, a Glasspane receipt discloses one per-output
**Out Cipher Key (OCK)**. The verifier uses that OCK against the named Zcash
transaction output and recovers:

* recipient address
* amount
* encrypted memo
* aggregate room total

It does **not** disclose the sender's seed, spending key, outgoing viewing key,
unified full viewing key, other transactions, or wallet balance.

This is different from a viewing key. A viewing key is useful, but it can be
wallet-wide surveillance. Glasspane proves only the payouts the team chooses.

## Try the room

The included demo room is built from a real mainnet Orchard payment plus a
deliberately tampered copy of the receipt.

```bash
cargo run -p gp-room -- examples/rooms/zechub-demo/room.json --out verified-room.json
```

Expected result:

* `mainnet-first-receipt`: verified, 0.00100000 ZEC, memo `glasspane first receipt`
* `tampered-ock`: rejected, because the OCK no longer opens the output
* `overall_pass`: true, because the tamper failure is expected by the room

## Verified on Zcash mainnet

Glasspane has been demonstrated end to end on **mainnet**. A real shielded
payment was made, and the verifier recovered it from the live chain using only
the per-output OCK:

* Tx: [`66167cd3020eb329446e86d80ccd0494baa3959bf9a0e586dbdccd204b6dcfd0`](https://mainnet.zcashexplorer.app/transactions/66167cd3020eb329446e86d80ccd0494baa3959bf9a0e586dbdccd204b6dcfd0) (block 3,361,512)
* Recovered: **0.001 ZEC** to an Orchard address, memo `glasspane first receipt`
* The rest of the wallet stayed opaque.

Reproduce it from a fresh clone in one command (offline, using the bundled raw tx):

```bash
cargo run --release -p gp-verifier -- examples/mainnet-receipt.json --raw-tx-file examples/mainnet-tx.hex
```

Details and the exact receipt are in [`examples/`](examples/).

## Status

Production-shape v0 across the receipt primitive and room verifier.

| Capability | Status |
|---|---|
| Receipt format v0 (Orchard + Sapling) | shipped |
| `gp-issue`: derive OCK from on-chain action + sender's OVK | shipped |
| `gp-verify`: fetch tx via lightwalletd + recover via OCK | shipped |
| `gp-room`: verify a payout room from receipts + raw tx files | shipped |
| Room schema and example mainnet room | shipped |
| Live Rooms board `/room/zechub-demo` | shipped |
| Self-serve room renderer `/room/create` | shipped |
| CSV export for accounting tools | shipped in room UI and `gp-room --csv` |
| Embeddable verified-payout badge | shipped on room pages |
| Receipt URL form (`https://host/r/<base64url>`) | shipped |
| Bech32m UA encoding for recovered recipients (`u1...`) | shipped |
| Optional ed25519 receipt signing | shipped |
| Static web verifier (envelope decode + tx explorer link) | shipped, [live](https://glasspane-iota.vercel.app) |
| Protocol-correctness tests against published Zcash test vectors | Orchard TV0 + TV1, Sapling TV0 |
| Input-sensitivity tests (bit flips must change the OCK) | shipped |
| CI (cargo fmt + clippy `-D warnings` + tests on every push) | shipped |
| WASM in-browser cryptographic recovery | shipped with raw tx fetch + paste fallback on `/room/create` |
| ZSA-aware receipts | roadmap |

21 tests passing across 6 crates.

## Quickstart

### 1. Build the tools

```bash
git clone https://github.com/dolepee/glasspane.git
cd glasspane
cargo build --release
```

The main binaries are:

* `gp-issue`: issue one receipt
* `gp-verify`: verify one receipt
* `gp-room`: verify a room of receipts and produce `verified-room.json`

### 2. Verify the demo room

```bash
./target/release/gp-room examples/rooms/zechub-demo/room.json --out verified-room.json
```

For accounting tools, export the same verified report as CSV:

```bash
./target/release/gp-room examples/rooms/zechub-demo/room.json --csv payouts.csv --out verified-room.json
```

Open the live board at
[`/room/zechub-demo`](https://glasspane-iota.vercel.app/room/zechub-demo), or
paste a `verified-room.json` into
[`/room/create`](https://glasspane-iota.vercel.app/room/create).

### 3. Issue a receipt for a shielded payment you sent

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

### 4. Verify one receipt

```bash
./target/release/gp-verify receipt.json
```

`gp-verify` also accepts a Glasspane URL (`https://host/r/<base64url>`), a bare base64url payload, or stdin. On success you see the recovered recipient UA, the value in zatoshis and ZEC, the memo, and (if signed) `signature : ed25519 OK`. On a wrong OCK, output index, or tampered receipt, you see a clear failure.

### 5. Verify one receipt in the browser

Open [`/room/create`](https://glasspane-iota.vercel.app/room/create), load or paste a receipt JSON or `/r/` URL, then click **Fetch raw tx** or paste raw transaction hex manually. The same-origin API fetches public raw tx hex from Blockchair; the browser still runs `gp-wasm` locally and rejects txid mismatches before recovery.

To rebuild the committed browser bundle:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.122
CC_wasm32_unknown_unknown=/path/to/clang cargo build -p gp-wasm --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir web/wasm target/wasm32-unknown-unknown/release/gp_wasm.wasm
```

### 6. Or share the URL form

Hand the contents of `receipt.json` to anyone with `gp-verify` installed, or share the URL emitted on stderr by `gp-issue --url`. Anyone with the URL can paste it into the browser verifier on `/room/create`, or into `gp-verify` to do the full cryptographic recovery from a raw or fetched transaction.

## Why this needs Zcash specifically

Bitcoin has no encrypted memos and no shielded receivers, so any "receipt" leaks the entire payment graph. Monero has no clean per-output selective disclosure primitive. Ethereum needs ZK add-ons to do any of this. Zcash's Sapling and Orchard pools already publish a per-output `out_ciphertext` that is encrypted under a per-output OCK derived from the sender's Outgoing Viewing Key. The protocol-level primitive for "show one party one specific payment without showing them the rest of your account" is **already in the chain**. Glasspane packages it for teams.

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
│   ├── gp-verifier/  # gp-verify binary
│   ├── gp-room/      # gp-room payout room verifier
│   ├── gp-wasm/      # browser receipt + raw tx verifier
│   └── gp-keygen/    # test wallet + OVK helper tooling
├── examples/
│   └── rooms/
│       └── zechub-demo/       # demo room + verified-room.json
├── spec/
│   ├── receipt.md    # Protocol spec for the v0 receipt format
│   ├── room.md       # Room schema
│   └── 24h-gate.md   # Engineering validation report
├── docs/
│   └── run-a-real-receipt.md   # End-to-end mainnet tutorial
├── web/
│   ├── index.html             # Dashboard workspace
│   ├── room/
│   │   ├── zechub-demo.html   # Rooms board
│   │   ├── zechub-demo.json   # Public room report
│   │   └── create.html        # Self-serve room renderer
│   └── wasm/                  # Browser verifier bundle generated from gp-wasm
└── .github/workflows/
    └── ci.yml        # fmt + clippy + tests on every push
```

The web app (`web/`) deploys to Vercel automatically on every push to `main` (Vercel's GitHub integration, project root set to `web/`).

`gp-types` and `gp-core` are pure libraries with no network dependencies. They can be embedded in other Rust tooling without pulling tonic, lightwalletd, or async runtimes.

## Tests + protocol correctness

```bash
cargo test --workspace
```

21 tests across:

* Receipt format (envelope validate, version reject, label length, JSON round trip, URL round trip, bare-payload URL parse, garbage URL reject, ed25519 signature round trip, ed25519 tampering reject).
* OCK derivation (byte helper round trip, Orchard test vectors 0 and 1 bit-exact, Sapling test vector 0 bit-exact, input sensitivity: any bit flip in OVK / epk / cmx must change the OCK).
* Room verification and export (example room verifies with expected tamper rejection; unexpected tamper fails the room; CSV contains accounting rows).
* Browser WASM verification (example receipt recovers from raw tx; tampered OCK fails loudly; raw txid mismatch fails before recovery).
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

* Multiple raw transaction sources for the browser fetch path, so Blockchair is not the only upstream.
* ZSA-aware receipts (when Zcash Shielded Assets ship a stable application surface).
* Batched receipts (multiple outputs in one envelope, each with its own OCK).

## References

* ZIP-244 (transaction format), ZIP-316 (Unified Addresses)
* Zcash Protocol Specification §4.19 (`prf_ock_sapling`), §4.20 (`prf_ock_orchard`)
* `zcash-test-vectors/orchard_note_encryption.py` — protocol-level test vectors
* `zcash-test-vectors/sapling_note_encryption.py` — protocol-level test vectors

## License

MIT. See [LICENSE](LICENSE).

Submission for **ZecHub Hackathon 3.0** (May 25 – July 15, 2026).
