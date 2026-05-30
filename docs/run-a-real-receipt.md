# Run a real Glasspane receipt end to end

This walks through producing and verifying a real Glasspane receipt against
a confirmed Orchard-pool shielded payment on Zcash mainnet. About 5 minutes
and $0.005 of ZEC.

## Prerequisites

- A Zcash wallet that lets you export your **Orchard Outgoing Viewing Key
  (OVK)** as 32 bytes hex. Options:
  - **Zashi** (mobile): Settings → Advanced → Export viewing keys
  - **YWallet** (desktop / mobile): Settings → Export keys → "OVK only"
  - **Zingo** CLI: `zingo-cli viewing-keys` and pull the `ovk` field
  - **zcashd**: `z_exportviewingkey "your_orchard_address"` then parse the UFVK
- ~0.005 ZEC on mainnet (under $3 at $521/ZEC).
- A receiver UA you also control (so you can verify the disclosure makes sense).
- A working `gp-issue` and `gp-verify` binary. Build them once:
  ```bash
  git clone https://github.com/dolepee/glasspane.git
  cd glasspane
  cargo build --release
  ```

## Step 1: send a shielded payment

From your sender wallet, send a tiny shielded ZEC payment to a UA you also
own. **Make sure the output is an Orchard output**, not Sapling. Most modern
wallets default to Orchard when both sender and receiver have Orchard
receivers in their UAs. If unsure, send between two Zashi-generated UAs.

Attach a memo so you can verify the round trip end to end. Example: `"first
glasspane receipt"`.

Wait for confirmation. Note the **transaction id** the wallet shows you.

## Step 2: identify the output index

A Zcash shielded transaction can have multiple Orchard actions (outputs).
You need the index of the action that landed at YOUR receiver.

The easiest path: if the transaction has exactly one output to your address
(the typical case for small test sends), the index is **0**.

For multi-output transactions, you can list the actions via:

```bash
# (Future v0.2 command, not yet implemented in this CLI:
#   gp-inspect --tx-id <txid> --lightwalletd <url>
# For v0, count outputs manually in your wallet.)
```

For the initial mainnet test, send a transaction with exactly one shielded
output. The output index is 0.

## Step 3: issue the receipt

```bash
./target/release/gp-issue \
  --pool orchard \
  --tx-id <your-tx-id-in-hex> \
  --output-index 0 \
  --ovk <your-32-byte-ovk-in-hex> \
  --label "first mainnet glasspane receipt" \
  --out receipt.json \
  --lightwalletd https://zec.rocks:443
```

What this does:
1. Connects to lightwalletd at zec.rocks.
2. Calls `GetTransaction` to fetch your transaction.
3. Parses the transaction, locates the Orchard action at index 0.
4. Extracts the action's published `cv_net`, `cmx`, and `epk_bytes`.
5. Computes the per-output OCK via `prf_ock_orchard(ovk, cv, cmx, epk)`.
6. Writes a Glasspane receipt JSON to `receipt.json`.

The receipt is now a portable file. Anyone with this file can verify the
disclosed payment on mainnet. They learn nothing else about your wallet.

## Step 4: verify the receipt

Ideally from a different machine that has no wallet keys loaded (to prove
that's all the verifier needs):

```bash
./target/release/gp-verify receipt.json \
  --lightwalletd https://zec.rocks:443
```

Expected output:

```
RECEIPT  abc123...your tx id...
  pool        : Orchard
  network     : Mainnet
  output_index: 0
  label       : first mainnet glasspane receipt
  issued_at   : 2026-06-XX...

Fetching transaction from https://zec.rocks:443 ...
  Transaction fetched and parsed.

OUTPUT RECOVERED
  recipient   : orchard:<43 bytes hex of receiver address>
  value       : 500000 zatoshis (0.00500000 ZEC)
  memo        : first glasspane receipt

VERIFIED.
```

If the recipient hex matches your receiver UA, the value matches what you
sent, and the memo matches what you attached, **Glasspane works**.

## Things that should NOT work (and why)

These failures are the security property of the system. If any of them
produce a successful verification, that is a bug.

- Verifying the receipt with the OCK modified by one bit: must fail.
- Verifying the receipt against a different transaction id: must fail.
- Verifying the receipt against a different output index in the same
  transaction: must fail (each output has its own OCK).
- Constructing a fake receipt with a chosen recipient + amount: must fail,
  because the OCK that decrypts to those values cannot be computed without
  the sender's OVK.

## Troubleshooting

**`connect to lightwalletd at https://zec.rocks:443` fails**
Your network may block outbound gRPC on port 443 to that host. Try a
different endpoint:
- `https://mainnet.lightwalletd.com:9067`
- `https://lwd1.zecmate.com:9067`
- Run your own lightwalletd locally and point at `http://localhost:9067`.

**`transaction has no Orchard action at index N`**
The output you named doesn't exist or is in the Sapling pool. v0 supports
Orchard only. Check your wallet to see which pool it sent from.

**`OCK does not match this output`**
Either the OVK is wrong (check you exported the OVK for the SENDER's
account, not the receiver's), the output index is wrong, or the receipt
was forged. Try output index 1, 2, etc. if the tx has multiple outputs.

**Receipt verifies but recipient doesn't match what you sent**
Different output index. Try the others.

## What to share

The receipt JSON itself, once shared, is verifiable forever by anyone
who has it. Treat it as: "any party with this file knows about this
payment, permanently." See `spec/receipt.md` for the full threat model.

A typical share workflow:
1. You make a donation in shielded ZEC to a charity.
2. You generate the Glasspane receipt for that one payment.
3. You give the receipt to your accountant for tax records.
4. Your accountant can verify the donation exists on chain.
5. Your wallet, other donations, and any other payments stay private.
