import {
  buildVerifiedRoom,
  createRoomPacketFiles,
  createZip,
  roomToCsv,
  slugify,
} from "/room/create-logic.mjs";

const form = document.getElementById("builder-form");
const payoutList = document.getElementById("payout-list");
const payoutTemplate = document.getElementById("payout-template");
const builderMessage = document.getElementById("builder-message");
const roomPreview = document.getElementById("room-preview");
const copyShareButton = document.getElementById("copy-share-link");
const downloadPacketButton = document.getElementById("download-packet");
const downloadJsonButton = document.getElementById("download-json");
const downloadCsvButton = document.getElementById("download-csv");
const detailFields = [
  document.getElementById("room-title"),
  document.getElementById("room-purpose"),
  document.getElementById("privacy-boundary"),
];
const roomNetwork = document.getElementById("room-network");

let nextPayoutKey = 1;
let builtRoom = null;
let builtPayouts = [];
let wasmModulePromise;

function roomDetails() {
  return {
    title: detailFields[0].value,
    purpose: detailFields[1].value,
    privacyBoundary: detailFields[2].value,
    network: roomNetwork.value,
  };
}

function setBuilderMessage(text, state = "") {
  builderMessage.textContent = text;
  builderMessage.className = state ? `message ${state}` : "message";
}

function setPayoutStatus(editor, text, state = "") {
  const status = editor.querySelector(".payout-status");
  status.textContent = text;
  status.className = state ? `payout-status ${state}` : "payout-status";
}

function parseReceiptInput(input) {
  const trimmed = input.trim();
  if (!trimmed) throw new Error("Receipt input is empty.");
  const rawJson = trimmed.startsWith("{")
    ? trimmed
    : decodeBase64Url((trimmed.includes("/r/") ? trimmed.split("/r/").pop() : trimmed).split(/[?#]/)[0]);
  const receipt = JSON.parse(rawJson);
  if (!receipt || receipt.version !== "0") throw new Error("This is not a Glasspane v0 receipt.");
  if (!/^[0-9a-f]{64}$/i.test(receipt.tx_id || "")) {
    throw new Error("Receipt tx_id must be 64 hexadecimal characters.");
  }
  return receipt;
}

function decodeBase64Url(encoded) {
  let normalized = encoded.replace(/-/g, "+").replace(/_/g, "/");
  while (normalized.length % 4) normalized += "=";
  const binary = atob(normalized);
  const bytes = Uint8Array.from(binary, character => character.charCodeAt(0));
  return new TextDecoder("utf-8").decode(bytes);
}

async function loadWasmModule() {
  if (!wasmModulePromise) {
    wasmModulePromise = import("/wasm/gp_wasm.js").then(async module => {
      await module.default();
      return module;
    });
  }
  return wasmModulePromise;
}

function addPayout(seed = {}) {
  const fragment = payoutTemplate.content.cloneNode(true);
  const editor = fragment.querySelector(".payout-editor");
  const key = nextPayoutKey++;
  editor.dataset.key = String(key);
  editor.verification = null;
  editor.receiptObject = null;

  const fields = [
    ["label", editor.querySelector(".payout-label")],
    ["role", editor.querySelector(".payout-role")],
    ["receipt", editor.querySelector(".payout-receipt")],
    ["raw", editor.querySelector(".payout-raw")],
  ];
  const status = editor.querySelector(".payout-status");
  status.id = `payout-${key}-status`;
  fields.forEach(([name, field], index) => {
    field.id = `payout-${key}-${name}`;
    field.setAttribute("aria-describedby", status.id);
    editor.querySelectorAll("label")[index].htmlFor = field.id;
    field.addEventListener("input", () => invalidatePayout(editor));
  });

  editor.querySelector(".payout-label").value = seed.label || "";
  editor.querySelector(".payout-role").value = seed.role || "";
  editor.querySelector(".payout-receipt").value = seed.receiptText || "";
  editor.querySelector(".payout-raw").value = seed.rawTxHex || "";

  editor.querySelector(".payout-remove").addEventListener("click", () => {
    editor.remove();
    renumberPayouts();
    resetBuiltRoom();
    updateReadiness();
  });
  editor.querySelector(".payout-fetch").addEventListener("click", () => fetchRawTransaction(editor));
  editor.querySelector(".payout-verify").addEventListener("click", () => verifyPayout(editor));
  payoutList.appendChild(fragment);
  renumberPayouts();
  updateReadiness();
  return editor;
}

function invalidatePayout(editor) {
  if (!editor.verification) return;
  editor.verification = null;
  editor.receiptObject = null;
  editor.querySelector(".payout-result").hidden = true;
  editor.querySelector(".payout-heading").textContent = "Unverified receipt";
  setPayoutStatus(editor, "Changed - verify again", "warn");
  resetBuiltRoom();
  updateReadiness();
}

function renumberPayouts() {
  payoutEditors().forEach((editor, index) => {
    editor.querySelector(".payout-index").textContent = `Payout ${String(index + 1).padStart(2, "0")}`;
  });
}

function payoutEditors() {
  return [...payoutList.querySelectorAll(".payout-editor")];
}

async function fetchRawTransaction(editor) {
  const receiptField = editor.querySelector(".payout-receipt");
  const rawField = editor.querySelector(".payout-raw");
  clearInvalid(editor);
  invalidatePayout(editor);
  setPayoutStatus(editor, "Fetching public transaction...", "busy");
  try {
    const receipt = parseReceiptInput(receiptField.value);
    if (receipt.network !== "mainnet") {
      throw new Error("Automatic raw transaction fetch currently supports mainnet receipts only.");
    }
    const response = await fetch(`/api/raw-tx?network=mainnet&txid=${encodeURIComponent(receipt.tx_id)}`);
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.error || "Raw transaction fetch failed.");
    rawField.value = body.raw_tx_hex;
    setPayoutStatus(editor, `Fetched from ${body.source}. Ready to verify.`, "ok");
  } catch (error) {
    markInvalid(receiptField);
    setPayoutStatus(editor, error.message || String(error), "err");
  }
}

async function verifyPayout(editor) {
  const labelField = editor.querySelector(".payout-label");
  const roleField = editor.querySelector(".payout-role");
  const receiptField = editor.querySelector(".payout-receipt");
  const rawField = editor.querySelector(".payout-raw");
  clearInvalid(editor);

  const emptyFields = [labelField, roleField, receiptField, rawField].filter(field => !field.value.trim());
  if (emptyFields.length) {
    emptyFields.forEach(markInvalid);
    emptyFields[0].focus();
    setPayoutStatus(editor, "Complete all payout fields before verification.", "err");
    return;
  }

  setPayoutStatus(editor, "Running Glasspane WASM locally...", "busy");
  editor.querySelector(".payout-verify").setAttribute("aria-busy", "true");
  try {
    const receipt = parseReceiptInput(receiptField.value);
    if (receipt.network !== roomNetwork.value) {
      throw new Error(`Receipt uses ${receipt.network}, but this room uses ${roomNetwork.value}.`);
    }
    const module = await loadWasmModule();
    const rawResult = module.verify_receipt_with_raw_tx(receiptField.value, rawField.value);
    const result = JSON.parse(rawResult);
    editor.verification = result;
    editor.receiptObject = receipt;
    renderPayoutResult(editor, result);
    editor.querySelector(".payout-heading").textContent = labelField.value.trim();
    setPayoutStatus(editor, "Cryptographically verified in this browser.", "ok");
    resetBuiltRoom();
  } catch (error) {
    editor.verification = null;
    editor.receiptObject = null;
    markInvalid(receiptField);
    markInvalid(rawField);
    editor.querySelector(".payout-result").hidden = true;
    setPayoutStatus(editor, error.message || String(error), "err");
  } finally {
    editor.querySelector(".payout-verify").removeAttribute("aria-busy");
    updateReadiness();
  }
}

function renderPayoutResult(editor, result) {
  const output = editor.querySelector(".payout-result");
  output.replaceChildren(
    resultItem("Amount", `${result.amount_zec} ZEC`),
    resultItem("Memo", result.memo),
    resultItem("Pool", result.pool),
    resultItem("Recipient", result.recipient),
    resultItem("Tx", shortTx(result.tx_id)),
  );
  output.hidden = false;
}

function resultItem(label, value) {
  const item = document.createElement("div");
  const key = document.createElement("span");
  const output = document.createElement("strong");
  key.textContent = label;
  output.textContent = value;
  item.append(key, output);
  return item;
}

function currentPayouts() {
  const usedIds = new Set();
  return payoutEditors().map((editor, index) => {
    const label = editor.querySelector(".payout-label").value.trim();
    let id = slugify(label || `payout-${index + 1}`);
    let suffix = 2;
    while (usedIds.has(id)) id = `${slugify(label)}-${suffix++}`;
    usedIds.add(id);
    return {
      id,
      label,
      role: editor.querySelector(".payout-role").value.trim(),
      receipt: editor.receiptObject,
      rawTxHex: editor.querySelector(".payout-raw").value,
      verification: editor.verification,
    };
  });
}

function validateRoomDetails() {
  let valid = true;
  detailFields.forEach(field => {
    const fieldValid = Boolean(field.value.trim());
    field.toggleAttribute("aria-invalid", !fieldValid);
    if (!fieldValid) {
      const describedBy = new Set((field.getAttribute("aria-describedby") || "").split(/\s+/).filter(Boolean));
      describedBy.add("builder-message");
      field.setAttribute("aria-describedby", [...describedBy].join(" "));
    }
    if (!fieldValid) valid = false;
  });
  if (!valid) {
    detailFields.find(field => !field.value.trim())?.focus();
    throw new Error("Complete the room title, purpose, and privacy boundary.");
  }
}

function buildRoom() {
  validateRoomDetails();
  const payouts = currentPayouts();
  const room = buildVerifiedRoom(roomDetails(), payouts);
  builtRoom = room;
  builtPayouts = payouts;
  renderRoom(room);
  enableOutputs(true, true);
  document.getElementById("finish-state").textContent = "Ready";
  setBuilderMessage("Verified room built. Share the safe board link or download the full replay packet.", "ok");
  roomPreview.hidden = false;
  roomPreview.scrollIntoView({ behavior: reducedMotion() ? "auto" : "smooth", block: "start" });
  updateReadiness();
}

function renderRoom(room) {
  document.getElementById("preview-purpose").textContent = room.purpose;
  document.getElementById("preview-total").textContent = `${room.total_zec} ZEC`;
  document.getElementById("preview-summary").textContent = `${room.verified_count} verified payout${room.verified_count === 1 ? "" : "s"} on ${room.network}`;
  const body = document.getElementById("preview-body");
  body.replaceChildren();
  room.results.forEach(result => {
    const row = document.createElement("tr");
    const status = document.createElement("span");
    status.className = `status ${result.status}`;
    status.append(createDot(), result.status);
    const role = document.createElement("div");
    role.className = "premium-recipient-cell";
    const roleName = document.createElement("strong");
    const roleLabel = document.createElement("span");
    roleName.textContent = result.role;
    roleLabel.textContent = result.label;
    role.append(roleName, roleLabel);
    const tx = result.tx_url ? document.createElement("a") : document.createElement("span");
    tx.textContent = shortTx(result.tx_id);
    if (result.tx_url) {
      tx.href = result.tx_url;
      tx.target = "_blank";
      tx.rel = "noopener";
      tx.className = "tx-link";
      tx.setAttribute("aria-label", `Open transaction ${result.tx_id} in the Zcash explorer`);
    }
    row.append(
      tableCell("Status", status),
      tableCell("Recipient role", role),
      tableCell("Recovered memo", result.memo, "mono"),
      tableCell("Amount", `${result.amount_zec} ZEC`, "mono"),
      tableCell("Transaction", tx),
    );
    body.appendChild(row);
  });
}

function tableCell(label, content, className = "") {
  const cell = document.createElement("td");
  cell.dataset.label = label;
  if (className) cell.className = className;
  if (content instanceof Node) cell.appendChild(content);
  else cell.textContent = content;
  return cell;
}

function createDot() {
  const dot = document.createElement("span");
  dot.className = "gp-dot";
  dot.setAttribute("aria-hidden", "true");
  return dot;
}

function resetBuiltRoom() {
  builtRoom = null;
  builtPayouts = [];
  document.getElementById("finish-state").textContent = "Not built";
  enableOutputs(false, false);
}

function enableOutputs(enabled, packetEnabled) {
  copyShareButton.disabled = !enabled;
  downloadJsonButton.disabled = !enabled;
  downloadCsvButton.disabled = !enabled;
  downloadPacketButton.disabled = !packetEnabled;
}

function updateReadiness() {
  const detailsComplete = detailFields.every(field => field.value.trim());
  const payouts = payoutEditors();
  const verified = payouts.filter(editor => editor.verification?.status === "verified").length;
  let score = detailsComplete ? 35 : 0;
  if (payouts.length) score += 15;
  if (payouts.length && verified === payouts.length) score += 40;
  if (builtRoom) score += 10;

  document.getElementById("details-state").textContent = detailsComplete ? "Complete" : "Required";
  document.getElementById("payouts-state").textContent = `${verified} verified`;
  document.getElementById("readiness-score").textContent = `${score}%`;
  document.getElementById("readiness-bar").style.width = `${score}%`;
  document.getElementById("readiness-details").textContent = detailsComplete ? "complete" : "waiting";
  document.getElementById("readiness-payouts").textContent = String(payouts.length);
  document.getElementById("readiness-verified").textContent = String(verified);
}

function clearInvalid(root) {
  root.querySelectorAll('[aria-invalid="true"]').forEach(field => field.removeAttribute("aria-invalid"));
}

function markInvalid(field) {
  field.setAttribute("aria-invalid", "true");
}

function shortTx(txId) {
  return txId ? `${txId.slice(0, 6)}...${txId.slice(-4)}` : "tx";
}

function encodeBase64Url(text) {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

async function copyShareLink() {
  if (!builtRoom) return;
  const payload = encodeBase64Url(JSON.stringify(builtRoom));
  const url = `${window.location.origin}${window.location.pathname}#room=${payload}`;
  window.history.replaceState(null, "", `#room=${payload}`);
  await copyText(url);
  setBuilderMessage("Private-safe room link copied. It contains the verified board, not receipt OCKs.", "ok");
}

async function copyText(value) {
  if (navigator.clipboard && window.isSecureContext) {
    await navigator.clipboard.writeText(value);
    return;
  }
  const scratch = document.createElement("textarea");
  scratch.value = value;
  scratch.setAttribute("readonly", "");
  scratch.style.position = "fixed";
  scratch.style.left = "-9999px";
  document.body.appendChild(scratch);
  scratch.select();
  document.execCommand("copy");
  scratch.remove();
}

function downloadPacket() {
  if (!builtRoom || !builtPayouts.length) return;
  const files = createRoomPacketFiles(roomDetails(), builtPayouts, builtRoom);
  const zip = createZip(files);
  downloadBlob(`${slugify(builtRoom.title)}-proof-packet.zip`, zip, "application/zip");
  setBuilderMessage("Replayable proof packet prepared.", "ok");
}

function downloadText(filename, text, type) {
  downloadBlob(filename, new TextEncoder().encode(text), type);
}

function downloadBlob(filename, bytes, type) {
  const blob = new Blob([bytes], { type });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
}

async function loadSupportExample() {
  setBuilderMessage("Loading the confirmed support transfer...", "busy");
  try {
    const receiptResponse = await fetch("/room/support-receipt.json");
    if (!receiptResponse.ok) throw new Error("Could not load the support transfer receipt.");
    const receiptText = await receiptResponse.text();
    detailFields[0].value = "Glasspane Support Transfers";
    detailFields[1].value = "Selected wallet setup and coordination support transfers, verified without exposing the rest of the wallet.";
    detailFields[2].value = "Only the selected support transfer output is disclosed: recipient, amount, memo, and transaction reference. Wallet keys, balance, change outputs, and unrelated transactions remain private.";
    roomNetwork.value = "mainnet";
    payoutList.replaceChildren();
    const editor = addPayout({
      label: "Wallet setup support transfer",
      role: "Community collaborator",
      receiptText,
    });
    await fetchRawTransaction(editor);
    setBuilderMessage("Confirmed support transfer loaded. Verify it locally to add it to the board.", "ok");
    editor.querySelector(".payout-verify").focus();
    updateReadiness();
  } catch (error) {
    setBuilderMessage(error.message || String(error), "err");
  }
}

async function importVerifiedReport(file) {
  if (!file) return;
  try {
    const report = JSON.parse(await file.text());
    if (report?.version !== "0" || !Array.isArray(report.results)) {
      throw new Error("This is not a Glasspane verified-room.json report.");
    }
    builtRoom = report;
    builtPayouts = [];
    renderRoom(report);
    roomPreview.hidden = false;
    enableOutputs(true, false);
    document.getElementById("finish-state").textContent = "Imported";
    document.getElementById("import-file-name").textContent = file.name;
    setBuilderMessage("Verified report imported. Board sharing and accounting exports are ready.", "ok");
    roomPreview.scrollIntoView({ behavior: reducedMotion() ? "auto" : "smooth", block: "start" });
  } catch (error) {
    setBuilderMessage(error.message || String(error), "err");
  }
}

function loadSharedRoom() {
  const payload = new URLSearchParams(window.location.hash.slice(1)).get("room");
  if (!payload) return;
  try {
    const report = JSON.parse(decodeBase64Url(payload));
    if (report?.version !== "0" || !Array.isArray(report.results)) {
      throw new Error("Shared data is not a Glasspane room report.");
    }
    builtRoom = report;
    renderRoom(report);
    roomPreview.hidden = false;
    enableOutputs(true, false);
    document.getElementById("finish-state").textContent = "Shared report";
    setBuilderMessage("Private-safe verified room loaded from the share link.", "ok");
  } catch (error) {
    setBuilderMessage(error.message || String(error), "err");
  }
}

function reducedMotion() {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

form.addEventListener("submit", event => {
  event.preventDefault();
  try {
    buildRoom();
  } catch (error) {
    setBuilderMessage(error.message || String(error), "err");
  }
});

document.getElementById("add-payout").addEventListener("click", () => {
  const editor = addPayout();
  editor.querySelector(".payout-label").focus();
});
document.getElementById("load-support-example").addEventListener("click", loadSupportExample);
copyShareButton.addEventListener("click", () => copyShareLink().catch(error => setBuilderMessage(error.message, "err")));
downloadPacketButton.addEventListener("click", downloadPacket);
downloadJsonButton.addEventListener("click", () => {
  if (!builtRoom) return;
  downloadText("verified-room.json", `${JSON.stringify(builtRoom, null, 2)}\n`, "application/json");
});
downloadCsvButton.addEventListener("click", () => {
  if (!builtRoom) return;
  downloadText(`${slugify(builtRoom.title)}.csv`, roomToCsv(builtRoom), "text/csv;charset=utf-8");
});
document.getElementById("room-file").addEventListener("change", event => importVerifiedReport(event.target.files[0]));
detailFields.forEach(field => field.addEventListener("input", () => {
  field.removeAttribute("aria-invalid");
  const describedBy = (field.getAttribute("aria-describedby") || "")
    .split(/\s+/)
    .filter(id => id && id !== "builder-message");
  if (describedBy.length) field.setAttribute("aria-describedby", describedBy.join(" "));
  else field.removeAttribute("aria-describedby");
  resetBuiltRoom();
  updateReadiness();
}));
roomNetwork.addEventListener("change", () => {
  payoutEditors().forEach(invalidatePayout);
  resetBuiltRoom();
  updateReadiness();
});

addPayout();
loadSharedRoom();
updateReadiness();
