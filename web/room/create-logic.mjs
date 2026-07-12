export function slugify(value) {
  return String(value || "glasspane-room")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/(^-|-$)/g, "") || "glasspane-room";
}

export function explorerUrl(network, txId) {
  if (network === "mainnet") {
    return `https://mainnet.zcashexplorer.app/transactions/${txId}`;
  }
  if (network === "testnet") {
    return `https://testnet.zcashexplorer.app/transactions/${txId}`;
  }
  return null;
}

export function roomToCsv(data) {
  const header = ["status", "recipient", "memo", "amount ZEC", "tx id", "verified-at"];
  const rows = data.results.map(item => [
    item.status || "",
    item.recipient || "",
    item.memo || item.error || "",
    item.amount_zec || "",
    item.tx_id || "",
    data.generated_at || "",
  ]);
  return [header, ...rows].map(row => row.map(csvEscape).join(",")).join("\n") + "\n";
}

export function buildVerifiedRoom(details, payouts, generatedAt = new Date().toISOString()) {
  validateDetails(details);
  if (!payouts.length) throw new Error("Add at least one payout receipt.");

  const unverified = payouts.find(payout => payout.verification?.status !== "verified");
  if (unverified) {
    throw new Error(`Verify ${unverified.label || "every payout"} before building the room.`);
  }

  const totalZatoshis = payouts.reduce(
    (total, payout) => total + Number(payout.verification.amount_zatoshis),
    0,
  );

  return {
    version: "0",
    title: details.title.trim(),
    purpose: details.purpose.trim(),
    privacy_boundary: details.privacyBoundary.trim(),
    network: details.network,
    generated_at: generatedAt,
    overall_pass: true,
    verified_count: payouts.length,
    rejected_count: 0,
    total_zatoshis: totalZatoshis,
    total_zec: zecString(totalZatoshis),
    expected_memo_labels: payouts.map(payout => payout.verification.memo),
    results: payouts.map(payout => {
      const result = payout.verification;
      return {
        id: payout.id,
        label: payout.label.trim(),
        role: payout.role.trim(),
        expected_outcome: "verified",
        status: "verified",
        expected_result_observed: true,
        tamper: false,
        tx_id: result.tx_id,
        tx_url: explorerUrl(details.network, result.tx_id),
        pool: result.pool,
        output_index: result.output_index,
        amount_zatoshis: result.amount_zatoshis,
        amount_zec: result.amount_zec,
        memo: result.memo,
        recipient: result.recipient,
        error: null,
      };
    }),
    failures: [],
  };
}

export function buildRoomManifest(details, payouts) {
  validateDetails(details);
  const totalZatoshis = payouts.reduce(
    (total, payout) => total + Number(payout.verification?.amount_zatoshis || 0),
    0,
  );

  return {
    version: "0",
    title: details.title.trim(),
    purpose: details.purpose.trim(),
    privacy_boundary: details.privacyBoundary.trim(),
    network: details.network,
    expected: {
      memo_labels: payouts.map(payout => payout.verification.memo),
      total_zatoshis: totalZatoshis,
    },
    receipts: payouts.map(payout => ({
      id: payout.id,
      label: payout.label.trim(),
      role: payout.role.trim(),
      receipt_path: `receipts/${payout.id}.json`,
      raw_tx_path: `raw/${payout.id}.hex`,
      expected_memo: payout.verification.memo,
      expected_zatoshis: payout.verification.amount_zatoshis,
      expected_outcome: "verified",
      tx_url: explorerUrl(details.network, payout.verification.tx_id),
    })),
  };
}

export function createRoomPacketFiles(details, payouts, verifiedRoom) {
  const root = slugify(details.title);
  const manifest = buildRoomManifest(details, payouts);
  const files = {
    [`${root}/room.json`]: prettyJson(manifest),
    [`${root}/verified-room.json`]: prettyJson(verifiedRoom),
    [`${root}/payouts.csv`]: roomToCsv(verifiedRoom),
  };

  for (const payout of payouts) {
    files[`${root}/receipts/${payout.id}.json`] = prettyJson(payout.receipt);
    files[`${root}/raw/${payout.id}.hex`] = `${payout.rawTxHex.trim()}\n`;
  }
  return files;
}

export function createZip(files, timestamp = new Date()) {
  const encoder = new TextEncoder();
  const localParts = [];
  const centralParts = [];
  let offset = 0;
  const { dosDate, dosTime } = toDosTime(timestamp);

  for (const [name, content] of Object.entries(files)) {
    const nameBytes = encoder.encode(name);
    const data = typeof content === "string" ? encoder.encode(content) : content;
    const crc = crc32(data);

    const local = new Uint8Array(30 + nameBytes.length + data.length);
    const localView = new DataView(local.buffer);
    localView.setUint32(0, 0x04034b50, true);
    localView.setUint16(4, 20, true);
    localView.setUint16(6, 0x0800, true);
    localView.setUint16(8, 0, true);
    localView.setUint16(10, dosTime, true);
    localView.setUint16(12, dosDate, true);
    localView.setUint32(14, crc, true);
    localView.setUint32(18, data.length, true);
    localView.setUint32(22, data.length, true);
    localView.setUint16(26, nameBytes.length, true);
    localView.setUint16(28, 0, true);
    local.set(nameBytes, 30);
    local.set(data, 30 + nameBytes.length);
    localParts.push(local);

    const central = new Uint8Array(46 + nameBytes.length);
    const centralView = new DataView(central.buffer);
    centralView.setUint32(0, 0x02014b50, true);
    centralView.setUint16(4, 20, true);
    centralView.setUint16(6, 20, true);
    centralView.setUint16(8, 0x0800, true);
    centralView.setUint16(10, 0, true);
    centralView.setUint16(12, dosTime, true);
    centralView.setUint16(14, dosDate, true);
    centralView.setUint32(16, crc, true);
    centralView.setUint32(20, data.length, true);
    centralView.setUint32(24, data.length, true);
    centralView.setUint16(28, nameBytes.length, true);
    centralView.setUint16(30, 0, true);
    centralView.setUint16(32, 0, true);
    centralView.setUint16(34, 0, true);
    centralView.setUint16(36, 0, true);
    centralView.setUint32(38, 0, true);
    centralView.setUint32(42, offset, true);
    central.set(nameBytes, 46);
    centralParts.push(central);
    offset += local.length;
  }

  const centralSize = centralParts.reduce((total, part) => total + part.length, 0);
  const end = new Uint8Array(22);
  const endView = new DataView(end.buffer);
  endView.setUint32(0, 0x06054b50, true);
  endView.setUint16(4, 0, true);
  endView.setUint16(6, 0, true);
  endView.setUint16(8, centralParts.length, true);
  endView.setUint16(10, centralParts.length, true);
  endView.setUint32(12, centralSize, true);
  endView.setUint32(16, offset, true);
  endView.setUint16(20, 0, true);

  return concatBytes([...localParts, ...centralParts, end]);
}

function validateDetails(details) {
  if (!details.title?.trim()) throw new Error("Room title is required.");
  if (!details.purpose?.trim()) throw new Error("Room purpose is required.");
  if (!details.privacyBoundary?.trim()) throw new Error("Privacy boundary is required.");
  if (!["mainnet", "testnet", "regtest"].includes(details.network)) {
    throw new Error("Choose a supported Zcash network.");
  }
}

function prettyJson(value) {
  return `${JSON.stringify(value, null, 2)}\n`;
}

function zecString(zatoshis) {
  const whole = Math.floor(zatoshis / 100_000_000);
  const fraction = String(zatoshis % 100_000_000).padStart(8, "0");
  return `${whole}.${fraction}`;
}

function csvEscape(value) {
  const text = value == null ? "" : String(value);
  return /[",\n\r]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function concatBytes(parts) {
  const length = parts.reduce((total, part) => total + part.length, 0);
  const output = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    output.set(part, offset);
    offset += part.length;
  }
  return output;
}

function toDosTime(date) {
  const year = Math.max(1980, date.getFullYear());
  return {
    dosTime: (date.getHours() << 11) | (date.getMinutes() << 5) | Math.floor(date.getSeconds() / 2),
    dosDate: ((year - 1980) << 9) | ((date.getMonth() + 1) << 5) | date.getDate(),
  };
}
