const TXID_RE = /^[0-9a-f]{64}$/i;
const RAW_TX_RE = /^[0-9a-f]+$/i;

export default async function handler(request, response) {
  if (request.method !== "GET") {
    response.setHeader("Allow", "GET");
    return response.status(405).json({ error: "method not allowed" });
  }

  const txid = String(request.query.txid || "").trim().toLowerCase();
  const network = String(request.query.network || "mainnet").trim().toLowerCase();

  response.setHeader("Cache-Control", "public, max-age=60, s-maxage=3600");

  if (network !== "mainnet") {
    return response.status(400).json({ error: "raw tx fetch currently supports mainnet only" });
  }
  if (!TXID_RE.test(txid)) {
    return response.status(400).json({ error: "txid must be 64 hex characters" });
  }

  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 10_000);

  try {
    const upstream = await fetch(`https://api.blockchair.com/zcash/raw/transaction/${txid}`, {
      headers: { "accept": "application/json" },
      signal: controller.signal,
    });
    if (!upstream.ok) {
      return response.status(502).json({ error: `Blockchair returned ${upstream.status}` });
    }

    const body = await upstream.json();
    const rawTx = body?.data?.[txid]?.raw_transaction;
    if (typeof rawTx !== "string" || !RAW_TX_RE.test(rawTx)) {
      return response.status(404).json({ error: "raw transaction not found for txid" });
    }

    return response.status(200).json({
      network,
      tx_id: txid,
      raw_tx_hex: rawTx,
      source: "blockchair",
    });
  } catch (error) {
    const message = error?.name === "AbortError" ? "raw tx fetch timed out" : "raw tx fetch failed";
    return response.status(502).json({ error: message });
  } finally {
    clearTimeout(timeout);
  }
}
