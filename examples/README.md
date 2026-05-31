# Mainnet proof

This directory contains a real, reproducible Glasspane disclosure for a payment that exists on **Zcash mainnet**.

- `mainnet-receipt.json` — a Glasspane v0 receipt for one Orchard output.
- `mainnet-tx.hex` — the raw bytes of the transaction it discloses (public chain data, bundled so verification works offline).

## Transaction

* Network: Zcash **mainnet**
* Tx id: `66167cd3020eb329446e86d80ccd0494baa3959bf9a0e586dbdccd204b6dcfd0`
* Block: 3,361,512

## Reproduce it yourself

From a fresh clone:

```bash
cargo run --release -p gp-verifier -- examples/mainnet-receipt.json --raw-tx-file examples/mainnet-tx.hex
```

Expected output:

```
OUTPUT RECOVERED
  recipient   : u1ug9ltzk9y74pgqtc7mtwnc8vsnlqlsalhamkfd4lyhegrfv6ff5znwr4syzylrpta36ehs3jjm3c4ssvpfar8zq2hamg35wwzqxqemeu
  value       : 100000 zatoshis (0.00100000 ZEC)
  memo        : glasspane first receipt

VERIFIED.
```

(To verify against the live chain instead of the bundled raw tx, drop the `--raw-tx-file` flag and the verifier fetches the transaction from lightwalletd.)

## What this receipt discloses, and what it does not

The receipt holds the per-output **OCK** for exactly one output of the transaction. Running it reveals:

* the recipient address,
* the value (0.001 ZEC),
* the memo (`glasspane first receipt`).

It reveals **nothing else**: not the sender's other outputs in the same transaction, not the sender's balance, not any other transaction the wallet has made, not the wallet's viewing key. The OCK for this output cannot open any other output. This is the whole point of Glasspane — prove one payment, keep the rest of the wallet private.

The transaction id and raw bytes are already public on the Zcash chain (every shielded transaction is a public object; its contents are encrypted). What Glasspane adds is the ability for the payer to selectively hand one counterparty the key to one specific output.
