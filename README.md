# Glasspane

> Prove you made a specific Zcash payment to a specific party, without revealing the rest of your wallet.

Selective disclosure receipts for shielded Zcash payments. After making a shielded payment on Zcash mainnet, the sender can generate a Glasspane receipt that lets a designated verifier confirm exactly that one transaction on chain. No other wallet content is exposed.

## Why

Privacy and accountability are usually mutually exclusive. Zcash's protocol uniquely supports both via viewing keys and note plaintexts, but no end-user tooling makes per-payment selective disclosure usable. Glasspane fills that gap.

Use cases:
- Charity donor receipts (prove you donated to a specific charity without doxxing your wallet)
- Tax reporting (give an accountant proof of payments without full account access)
- Investigative journalism expense proofs
- Freelance payment receipts
- Regulator-friendly compliance trails

## What is in a Glasspane receipt

For one shielded payment, the sender shares:

- Transaction id on Zcash mainnet
- Output index inside that transaction
- The note plaintext for that output: recipient diversified address, value, rseed, memo
- A short label and an issued-at timestamp

The verifier runs the receipt against Zcash mainnet (via lightwalletd or any Zcash node) and confirms:

1. The transaction exists and is confirmed
2. The output at that index has a note commitment matching the disclosed plaintext
3. The plaintext decrypts cleanly under the disclosed ephemeral material
4. The recipient address, value, and memo match the receipt

Anything else about the sender's wallet remains opaque to the verifier.

## What it does NOT prove

- That the sender's full wallet contains anything specific.
- That the sender has not made other payments.
- That the recipient has not received other payments.
- Identity of the sender.

The receipt is scoped to ONE payment. Read the threat model in `THREAT.md` for the full boundary.

## Status

v0 protocol is implemented end to end for both **Orchard** and **Sapling** pools. The cryptographic gate passes against the published Zcash protocol test vectors for both pools (`cargo test --workspace`).

What works today:
- `gp-issue` derives the OCK for a specific shielded output by fetching the transaction from lightwalletd and computing `prf_ock` over the on-chain `(cv, cmstar, epk)`.
- `gp-verify` accepts a receipt as a file path, a `https://host/r/<base64url>` URL, a bare base64url payload, or stdin. It fetches the named transaction, runs `try_output_recovery_with_ock` against the disclosed OCK, and prints the recovered note plaintext.
- Receipt format spec is in `spec/receipt.md`. Engineering gate report is in `spec/24h-gate.md`. End to end mainnet tutorial is in `docs/run-a-real-receipt.md`.

## Build

(forthcoming)

## License

MIT (see LICENSE).
