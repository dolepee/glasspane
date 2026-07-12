# Glasspane Room Format v0

A Glasspane room is a public payout proof board. It groups selected Glasspane
receipts, raw Zcash transactions, expected memo labels, and aggregate checks so
a third party can verify the disclosed payouts without receiving a viewing key
or seeing the rest of the wallet.

Rooms are intentionally file-based for v0:

```text
room.json + receipt JSON files + raw transaction hex files -> gp-room -> verified-room.json
```

The raw transaction hex can come from any Zcash node, lightwalletd-backed tool,
or block explorer raw-transaction endpoint. The room never contains wallet
secrets, seeds, spending keys, outgoing viewing keys, or unified full viewing
keys.

An empty room is valid when its expected total is zero or omitted and its
expected memo list is empty. This lets a team publish an honest ledger before
its first payout. A zero-receipt room with a non-zero minimum, non-zero exact
total, or expected memo fails verification.

## `room.json`

```json
{
  "version": "0",
  "title": "ZecHub demo payout room",
  "purpose": "Selective disclosure for a small Zcash payout run.",
  "privacy_boundary": "Only the listed receipts are disclosed. The rest of the wallet remains opaque.",
  "network": "mainnet",
  "expected": {
    "memo_labels": ["designer paid", "translator paid"],
    "min_total_zatoshis": 100000,
    "total_zatoshis": 100000
  },
  "receipts": [
    {
      "id": "designer-paid",
      "label": "Designer payout",
      "role": "Designer",
      "receipt_path": "receipts/designer-paid.json",
      "raw_tx_path": "raw/designer-paid.hex",
      "expected_memo": "designer paid",
      "expected_zatoshis": 100000,
      "expected_outcome": "verified",
      "tx_url": "https://mainnet.zcashexplorer.app/transactions/..."
    },
    {
      "id": "tampered-ock",
      "label": "Tampered copy",
      "role": "Tamper check",
      "receipt_path": "receipts/designer-paid-tampered.json",
      "raw_tx_path": "raw/designer-paid.hex",
      "expected_outcome": "rejected",
      "tamper": true
    }
  ]
}
```

## Fields

| Field | Meaning |
|---|---|
| `version` | Room schema version. v0 is the only accepted value. |
| `title` | Human-readable board title. |
| `purpose` | Why this room exists. |
| `privacy_boundary` | Plain-language disclosure boundary shown to reviewers. |
| `network` | `mainnet`, `testnet`, or `regtest`. Every receipt must match it. |
| `expected.memo_labels` | Memos the room is expected to prove. |
| `expected.min_total_zatoshis` | Optional lower bound for the recovered verified total. |
| `expected.total_zatoshis` | Optional exact recovered verified total. |
| `receipts[].id` | Stable machine-readable receipt id. |
| `receipts[].label` | UI label for the payout. |
| `receipts[].role` | Human-readable recipient/contributor role. |
| `receipts[].receipt_path` | Path to a Glasspane receipt JSON, relative to `room.json` unless absolute. |
| `receipts[].raw_tx_path` | Path to raw transaction hex, relative to `room.json` unless absolute. |
| `receipts[].expected_memo` | Optional exact recovered memo check. |
| `receipts[].expected_min_zatoshis` | Optional per-receipt lower bound. |
| `receipts[].expected_zatoshis` | Optional exact recovered amount. |
| `receipts[].expected_outcome` | `verified` by default, or `rejected` for a deliberate tamper case. |
| `receipts[].tamper` | Marks an intentional red-case receipt for the board. |
| `receipts[].tx_url` | Optional explorer URL. If absent, `gp-room` derives one for mainnet/testnet. |

## Verification Semantics

`gp-room` recovers each receipt through the same OCK path as `gp-verify`:

1. Validate the receipt envelope.
2. Verify an attached ed25519 signature if present.
3. Parse the raw Zcash transaction.
4. Locate the Orchard action or Sapling output.
5. Recover recipient, amount, and memo with the disclosed OCK.
6. Check expected memo/amount/total constraints.

A marked tamper receipt is expected to produce a red `rejected` result. Any
unexpected rejection, unexpected verification of a tamper case, memo mismatch,
amount mismatch, or aggregate mismatch makes the room fail loudly with a
non-zero exit code.

## Output

`verified-room.json` contains the public board data:

- room title, purpose, privacy boundary, and network
- `overall_pass`
- verified and rejected counts
- aggregate verified total
- per-receipt status, amount, memo, tx id, tx URL, and error if rejected

The output is safe to publish. It contains only the selected disclosures.
