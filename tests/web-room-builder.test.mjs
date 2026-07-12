import assert from "node:assert/strict";
import test from "node:test";

import {
  buildRoomManifest,
  buildVerifiedRoom,
  createRoomPacketFiles,
  createZip,
  roomToCsv,
} from "../web/room/create-logic.mjs";

const details = {
  title: "Contributor Support",
  purpose: "Publish selected support transfers.",
  privacyBoundary: "Only selected outputs are disclosed.",
  network: "mainnet",
};

const payouts = [
  {
    id: "wallet-setup-support",
    label: "Wallet setup support transfer",
    role: "Community collaborator",
    receipt: {
      version: "0",
      network: "mainnet",
      tx_id: "a".repeat(64),
      ock: "disclosed-per-output-key",
    },
    rawTxHex: "05000080",
    verification: {
      status: "verified",
      pool: "orchard",
      tx_id: "a".repeat(64),
      output_index: 1,
      amount_zatoshis: 20_000,
      amount_zec: "0.00020000",
      memo: "wallet support",
      recipient: "u1recipient",
    },
  },
];

test("builds a private-safe verified room from browser verification results", () => {
  const report = buildVerifiedRoom(details, payouts, "2026-07-12T12:00:00.000Z");
  assert.equal(report.overall_pass, true);
  assert.equal(report.total_zatoshis, 20_000);
  assert.equal(report.total_zec, "0.00020000");
  assert.equal(report.results[0].label, "Wallet setup support transfer");
  assert.equal(report.results[0].recipient, "u1recipient");
  assert.equal(JSON.stringify(report).includes("disclosed-per-output-key"), false);
  assert.equal(JSON.stringify(report).includes("05000080"), false);
});

test("builds a gp-room compatible manifest with replay paths and expectations", () => {
  const manifest = buildRoomManifest(details, payouts);
  assert.equal(manifest.receipts[0].receipt_path, "receipts/wallet-setup-support.json");
  assert.equal(manifest.receipts[0].raw_tx_path, "raw/wallet-setup-support.hex");
  assert.equal(manifest.receipts[0].expected_zatoshis, 20_000);
  assert.deepEqual(manifest.expected.memo_labels, ["wallet support"]);
});

test("keeps OCK material in the replay packet and out of the verified board", () => {
  const report = buildVerifiedRoom(details, payouts, "2026-07-12T12:00:00.000Z");
  const files = createRoomPacketFiles(details, payouts, report);
  assert.match(files["contributor-support/receipts/wallet-setup-support.json"], /disclosed-per-output-key/);
  assert.doesNotMatch(files["contributor-support/verified-room.json"], /disclosed-per-output-key/);
  assert.equal(files["contributor-support/raw/wallet-setup-support.hex"], "05000080\n");

  const zip = createZip(files, new Date("2026-07-12T12:00:00Z"));
  assert.deepEqual([...zip.slice(0, 4)], [0x50, 0x4b, 0x03, 0x04]);
  const binaryText = new TextDecoder().decode(zip);
  assert.match(binaryText, /contributor-support\/room\.json/);
  assert.match(binaryText, /contributor-support\/verified-room\.json/);
});

test("exports accounting CSV with quoted memos", () => {
  const report = buildVerifiedRoom(details, payouts, "2026-07-12T12:00:00.000Z");
  report.results[0].memo = "wallet, setup";
  const csv = roomToCsv(report);
  assert.match(csv, /"wallet, setup"/);
  assert.match(csv, /0\.00020000/);
});

test("refuses to build a room with an unverified payout", () => {
  const unverified = [{ ...payouts[0], verification: null }];
  assert.throws(() => buildVerifiedRoom(details, unverified), /Verify Wallet setup support transfer/);
});
