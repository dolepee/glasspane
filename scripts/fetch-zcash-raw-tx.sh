#!/usr/bin/env sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  echo "usage: $0 <txid> [out.hex]" >&2
  exit 64
fi

txid="$1"
out="${2:-}"
url="https://api.blockchair.com/zcash/raw/transaction/$txid"
body="$(curl -fsSL "$url")"
raw="$(printf '%s' "$body" | tr -d '\n' | sed -n 's/.*"raw_transaction":"\([0-9a-fA-F][0-9a-fA-F]*\)".*/\1/p')"

if [ -z "$raw" ]; then
  echo "raw_transaction not found in Blockchair response for $txid" >&2
  exit 1
fi

if [ -n "$out" ]; then
  printf '%s\n' "$raw" > "$out"
else
  printf '%s\n' "$raw"
fi
