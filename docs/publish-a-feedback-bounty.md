# Publish a support or bounty receipt

The Support Transfers room grows only after real mainnet transfers verify. The
publish command copies only the selected receipt and public raw transaction into
the room, verifies the complete room with `gp-room`, and updates the web report
and CSV.

## Before paying

Ask the recipient for explicit consent to publish this selected payout. A
Glasspane receipt intentionally reveals the recovered recipient address,
amount, memo, and transaction id. It does not reveal the sender's seed,
spending key, viewing keys, balance, or other transactions.

For a public bounty, ask the recipient for a fresh Unified Address beginning
with `u` and use an Orchard-capable wallet. Do not publish an unrelated private
payment just to populate the room.

## 1. Send the payout

Send a shielded mainnet payment with a short, unique memo, for example:

```text
glasspane feedback 01
```

Wait for confirmation and record the transaction id and exact zatoshi amount.
One ZEC is 100,000,000 zatoshis.

## 2. Create the selective receipt on the wallet machine

Export the sender account's Unified Full Viewing Key (`uview1...`) from the
wallet. Never export or share the seed or spending key. In PowerShell, keep the
viewing material out of command history by reading it into temporary variables:

```powershell
$ufvk = Read-Host "Sender UFVK (uview1...)"
$ovk = cargo run --quiet --release -p gp-keygen --bin gp-ovk -- $ufvk |
  Select-Object -Last 1 |
  ForEach-Object { $_.Trim() }

$txid = "YOUR_CONFIRMED_TX_ID"
$raw = Invoke-RestMethod "https://glasspane-iota.vercel.app/api/raw-tx?txid=$txid"
$raw.raw_tx_hex | Set-Content -NoNewline bounty-tx.hex

cargo run --release -p gp-issuer -- `
  --pool orchard `
  --tx-id $txid `
  --output-index 0 `
  --ovk $ovk `
  --label "Glasspane feedback bounty 01" `
  --raw-tx-file bounty-tx.hex `
  --out bounty-receipt.json

Remove-Variable ufvk, ovk
```

If output index `0` does not recover the intended payout, do not publish.
Identify the correct Orchard action and recreate the receipt with that index.

## 3. Verify and publish in one command

Run this from the Glasspane repository. Replace the public labels and exact
amount with the confirmed payout details:

```powershell
node .\scripts\publish-bounty-receipt.mjs `
  --receipt .\bounty-receipt.json `
  --id feedback-01 `
  --label "Feedback bounty 01" `
  --role "Community reviewer" `
  --memo "glasspane feedback 01" `
  --zatoshis 100000
```

The command fetches the public raw transaction when `--raw-tx` is omitted. It
does not change the room unless the receipt, memo, amount, transaction, and
complete room all verify. Commit and push the resulting public room files only
after reviewing the diff for accidental secrets.
