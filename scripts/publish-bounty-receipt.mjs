#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const TXID_RE = /^[0-9a-f]{64}$/i;
const RAW_TX_RE = /^[0-9a-f]+$/i;
const ID_RE = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

function usage() {
  console.log(`Usage:
  node scripts/publish-bounty-receipt.mjs \\
    --receipt <receipt.json> \\
    --id <stable-id> \\
    --label <public-label> \\
    --role <public-role> \\
    --memo <exact-memo> \\
    --zatoshis <integer> \\
    [--raw-tx <transaction.hex>]

If --raw-tx is omitted, the script fetches public mainnet transaction hex from
the deployed Glasspane raw-transaction endpoint. Wallet secrets must never be
passed to this command.`);
}

function parseArgs(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    if (key === "--help" || key === "-h") {
      usage();
      process.exit(0);
    }
    if (!key.startsWith("--") || index + 1 >= argv.length) {
      throw new Error(`invalid argument: ${key}`);
    }
    values.set(key.slice(2), argv[index + 1]);
    index += 1;
  }
  return values;
}

function required(values, name) {
  const value = values.get(name)?.trim();
  if (!value) throw new Error(`--${name} is required`);
  return value;
}

function readJson(filePath, label) {
  try {
    return JSON.parse(readFileSync(filePath, "utf8"));
  } catch (error) {
    throw new Error(`${label} is not valid JSON: ${error.message}`);
  }
}

async function fetchRawTx(txId) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 15_000);
  const url = `https://glasspane-iota.vercel.app/api/raw-tx?txid=${txId}`;
  try {
    const response = await fetch(url, { signal: controller.signal });
    const body = await response.json();
    if (!response.ok) {
      throw new Error(body.error || `HTTP ${response.status}`);
    }
    return body.raw_tx_hex;
  } finally {
    clearTimeout(timeout);
  }
}

function removeIfPresent(filePath) {
  if (existsSync(filePath)) rmSync(filePath, { force: true });
}

export function supportRoomPaths(repoRoot) {
  const roomDir = path.join(repoRoot, "examples", "rooms", "glasspane-bounties");
  return {
    roomDir,
    roomPath: path.join(roomDir, "room.json"),
    verifiedPath: path.join(roomDir, "verified-room.json"),
    csvPath: path.join(roomDir, "payouts.csv"),
    webReportPath: path.join(repoRoot, "web", "room", "glasspane-support.json"),
  };
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const receiptSource = path.resolve(required(args, "receipt"));
  const id = required(args, "id");
  const label = required(args, "label");
  const role = required(args, "role");
  const memo = required(args, "memo");
  const zatoshisText = required(args, "zatoshis");

  if (!ID_RE.test(id)) {
    throw new Error("--id must contain lowercase letters, numbers, and single hyphens only");
  }
  if (label.length > 80 || /[\r\n]/.test(label)) {
    throw new Error("--label must be one line and at most 80 characters");
  }
  if (role.length > 80 || /[\r\n]/.test(role)) {
    throw new Error("--role must be one line and at most 80 characters");
  }
  if (Buffer.byteLength(memo, "utf8") > 512) {
    throw new Error("--memo must fit in the 512-byte Zcash memo field");
  }
  const zatoshis = Number(zatoshisText);
  if (!Number.isSafeInteger(zatoshis) || zatoshis <= 0) {
    throw new Error("--zatoshis must be a positive integer");
  }

  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const repoRoot = path.resolve(scriptDir, "..");
  const { roomDir, roomPath, verifiedPath, csvPath, webReportPath } =
    supportRoomPaths(repoRoot);
  const receiptDestination = path.join(roomDir, "receipts", `${id}.json`);
  const rawDestination = path.join(roomDir, "raw", `${id}.hex`);
  const candidatePath = path.join(roomDir, `.room-${process.pid}.tmp.json`);
  const candidateReportPath = path.join(roomDir, `.verified-${process.pid}.tmp.json`);
  const candidateCsvPath = path.join(roomDir, `.payouts-${process.pid}.tmp.csv`);

  if (!existsSync(receiptSource)) throw new Error(`receipt not found: ${receiptSource}`);
  if (existsSync(receiptDestination) || existsSync(rawDestination)) {
    throw new Error(`receipt id already has stored files: ${id}`);
  }

  const receipt = readJson(receiptSource, "receipt");
  const txId = String(receipt.tx_id || "").toLowerCase();
  if (!TXID_RE.test(txId)) throw new Error("receipt tx_id must be 64 hex characters");
  if (receipt.network !== "mainnet") throw new Error("the support room accepts mainnet receipts only");

  const room = readJson(roomPath, "support room");
  if (room.receipts.some((entry) => entry.id === id)) {
    throw new Error(`receipt id already exists in room.json: ${id}`);
  }

  let rawTx;
  const rawSource = args.get("raw-tx");
  if (rawSource) {
    rawTx = readFileSync(path.resolve(rawSource), "utf8").trim();
  } else {
    console.log(`Fetching public raw transaction ${txId}...`);
    rawTx = await fetchRawTx(txId);
  }
  if (!RAW_TX_RE.test(rawTx) || rawTx.length % 2 !== 0) {
    throw new Error("raw transaction must be even-length hexadecimal");
  }

  const entry = {
    id,
    label,
    role,
    receipt_path: `receipts/${id}.json`,
    raw_tx_path: `raw/${id}.hex`,
    expected_memo: memo,
    expected_zatoshis: zatoshis,
    expected_outcome: "verified",
    tx_url: `https://mainnet.zcashexplorer.app/transactions/${txId}`,
  };
  const candidate = structuredClone(room);
  candidate.receipts.push(entry);
  candidate.expected.memo_labels = candidate.receipts
    .map((item) => item.expected_memo)
    .filter(Boolean);
  candidate.expected.total_zatoshis = candidate.receipts.reduce((total, item) => {
    if (!Number.isSafeInteger(item.expected_zatoshis)) {
      throw new Error(`receipt ${item.id} needs expected_zatoshis before publishing`);
    }
    return total + item.expected_zatoshis;
  }, 0);

  mkdirSync(path.dirname(receiptDestination), { recursive: true });
  mkdirSync(path.dirname(rawDestination), { recursive: true });
  writeFileSync(receiptDestination, `${JSON.stringify(receipt, null, 2)}\n`);
  writeFileSync(rawDestination, `${rawTx.toLowerCase()}\n`);
  writeFileSync(candidatePath, `${JSON.stringify(candidate, null, 2)}\n`);

  try {
    const result = spawnSync(
      "cargo",
      [
        "run",
        "--locked",
        "-p",
        "gp-room",
        "--",
        candidatePath,
        "--out",
        candidateReportPath,
        "--csv",
        candidateCsvPath,
      ],
      { cwd: repoRoot, stdio: "inherit" },
    );
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error("gp-room rejected the candidate payout");

    copyFileSync(candidatePath, roomPath);
    copyFileSync(candidateReportPath, verifiedPath);
    copyFileSync(candidateReportPath, webReportPath);
    copyFileSync(candidateCsvPath, csvPath);
  } catch (error) {
    removeIfPresent(receiptDestination);
    removeIfPresent(rawDestination);
    throw error;
  } finally {
    removeIfPresent(candidatePath);
    removeIfPresent(candidateReportPath);
    removeIfPresent(candidateCsvPath);
  }

  console.log(`Published ${id}: ${zatoshis} zatoshis`);
  console.log(`Room report: ${path.relative(repoRoot, verifiedPath)}`);
  console.log(`Web report:  ${path.relative(repoRoot, webReportPath)}`);
}

const invokedDirectly = process.argv[1]
  && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (invokedDirectly) {
  main().catch((error) => {
    console.error(`publish failed: ${error.message}`);
    process.exitCode = 1;
  });
}
