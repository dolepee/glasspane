//! Browser-side Glasspane verification.
//!
//! The exported WASM function takes a Glasspane receipt plus raw Zcash
//! transaction hex and runs the same OCK recovery path used by the Rust CLI.

use gp_core::{
    ensure_transaction_id, ock_from_bytes, parse_transaction, recover_orchard, recover_sapling,
};
use gp_types::{Network, Pool, Receipt};
use serde::Serialize;
use wasm_bindgen::prelude::*;
use zcash_address::{
    unified::{self, Encoding, Receiver},
    ToAddress, ZcashAddress,
};
use zcash_note_encryption::OutgoingCipherKey;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::NetworkType;

#[derive(Debug, Serialize)]
struct BrowserVerifyResult {
    status: &'static str,
    verifier: &'static str,
    network: Network,
    pool: Pool,
    tx_id: String,
    parsed_tx_id: String,
    output_index: u32,
    label: String,
    signature: SignatureStatus,
    amount_zatoshis: u64,
    amount_zec: String,
    memo: String,
    recipient: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum SignatureStatus {
    Absent,
    Verified,
}

struct RecoveredOutput {
    recipient: String,
    value_zatoshis: u64,
    memo: String,
}

/// Verify a Glasspane receipt against raw Zcash transaction hex in-browser.
///
/// `receipt_input` accepts receipt JSON, a Glasspane `/r/<payload>` URL, or the
/// bare base64url payload from that URL. The function fails loudly if the
/// receipt envelope is invalid, the optional signature is invalid, the raw
/// transaction does not match the receipt txid, or the disclosed OCK cannot
/// recover the named output.
#[wasm_bindgen]
pub fn verify_receipt_with_raw_tx(
    receipt_input: &str,
    raw_tx_hex: &str,
) -> Result<String, JsValue> {
    verify_receipt(receipt_input, raw_tx_hex)
        .and_then(|result| serde_json::to_string(&result).map_err(|err| err.to_string()))
        .map_err(|err| JsValue::from_str(&err))
}

fn verify_receipt(receipt_input: &str, raw_tx_hex: &str) -> Result<BrowserVerifyResult, String> {
    let receipt = parse_receipt(receipt_input)?;
    let signature = match receipt.verify_signature_if_present() {
        Ok(true) => SignatureStatus::Verified,
        Ok(false) => SignatureStatus::Absent,
        Err(err) => return Err(format!("signature verification failed: {err}")),
    };

    let raw_tx = decode_raw_tx(raw_tx_hex)?;
    let tx = parse_transaction(&raw_tx).map_err(|err| err.to_string())?;
    let parsed_tx_id = tx.txid().to_string();
    ensure_transaction_id(&tx, &receipt.tx_id).map_err(|err| err.to_string())?;

    let ock = ock_from_bytes(receipt.ock_bytes().map_err(|err| err.to_string())?);
    let recovered = match receipt.pool {
        Pool::Orchard => recover_orchard_output(&tx, receipt.output_index, &ock, receipt.network)?,
        Pool::Sapling => recover_sapling_output(&tx, receipt.output_index, &ock, receipt.network)?,
    };

    Ok(BrowserVerifyResult {
        status: "verified",
        verifier: "glasspane-wasm",
        network: receipt.network,
        pool: receipt.pool,
        tx_id: receipt.tx_id,
        parsed_tx_id,
        output_index: receipt.output_index,
        label: receipt.label,
        signature,
        amount_zatoshis: recovered.value_zatoshis,
        amount_zec: zec_string(recovered.value_zatoshis),
        memo: recovered.memo,
        recipient: recovered.recipient,
    })
}

fn parse_receipt(input: &str) -> Result<Receipt, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("receipt input is empty".to_string());
    }

    if trimmed.starts_with('{') {
        let receipt: Receipt =
            serde_json::from_str(trimmed).map_err(|err| format!("parse receipt JSON: {err}"))?;
        receipt
            .validate()
            .map_err(|err| format!("receipt envelope invalid: {err}"))?;
        Ok(receipt)
    } else {
        Receipt::from_url(trimmed).map_err(|err| format!("parse receipt URL payload: {err}"))
    }
}

fn decode_raw_tx(raw_tx_hex: &str) -> Result<Vec<u8>, String> {
    let compact: String = raw_tx_hex.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return Err("raw transaction hex is empty".to_string());
    }
    hex::decode(&compact).map_err(|err| format!("decode raw transaction hex: {err}"))
}

fn recover_orchard_output(
    tx: &Transaction,
    output_index: u32,
    ock: &OutgoingCipherKey,
    network: Network,
) -> Result<RecoveredOutput, String> {
    let bundle = tx
        .orchard_bundle()
        .ok_or_else(|| "transaction has no Orchard bundle".to_string())?;
    let actions: Vec<_> = bundle.actions().iter().collect();
    let action = actions
        .get(output_index as usize)
        .ok_or_else(|| format!("transaction has no Orchard action at index {output_index}"))?;
    let disc = recover_orchard(*action, ock).map_err(|_| {
        "OCK does not match this Orchard action; the receipt is wrong, tampered, or forged"
            .to_string()
    })?;
    Ok(RecoveredOutput {
        recipient: encode_ua(
            network,
            Pool::Orchard,
            disc.recipient.to_raw_address_bytes(),
        ),
        value_zatoshis: disc.value.inner(),
        memo: memo_to_display(&disc.memo).unwrap_or_else(|| "(non text memo)".to_string()),
    })
}

fn recover_sapling_output(
    tx: &Transaction,
    output_index: u32,
    ock: &OutgoingCipherKey,
    network: Network,
) -> Result<RecoveredOutput, String> {
    let bundle = tx
        .sapling_bundle()
        .ok_or_else(|| "transaction has no Sapling bundle".to_string())?;
    let output = bundle
        .shielded_outputs()
        .get(output_index as usize)
        .ok_or_else(|| format!("transaction has no Sapling output at index {output_index}"))?;
    let zip212 = sapling_crypto::note_encryption::Zip212Enforcement::On;
    let disc = recover_sapling(output, ock, zip212).map_err(|_| {
        "OCK does not match this Sapling output; the receipt is wrong, tampered, or forged"
            .to_string()
    })?;
    Ok(RecoveredOutput {
        recipient: encode_ua(network, Pool::Sapling, disc.recipient.to_bytes()),
        value_zatoshis: disc.value.inner(),
        memo: memo_to_display(&disc.memo).unwrap_or_else(|| "(non text memo)".to_string()),
    })
}

fn encode_ua(network: Network, pool: Pool, bytes: [u8; 43]) -> String {
    let net = match network {
        Network::Mainnet => NetworkType::Main,
        Network::Testnet => NetworkType::Test,
        Network::Regtest => NetworkType::Regtest,
    };
    let receiver = match pool {
        Pool::Orchard => Receiver::Orchard(bytes),
        Pool::Sapling => Receiver::Sapling(bytes),
    };
    match unified::Address::try_from_items(vec![receiver]) {
        Ok(ua) => ZcashAddress::from_unified(net, ua).encode(),
        Err(_) => format!("{pool:?}:{}", hex::encode(bytes)),
    }
}

fn memo_to_display(memo: &[u8; 512]) -> Option<String> {
    let trimmed: Vec<u8> = memo.iter().copied().take_while(|&b| b != 0).collect();
    if trimmed.is_empty() {
        return Some("(empty memo)".to_string());
    }
    String::from_utf8(trimmed).ok()
}

fn zec_string(zatoshis: u64) -> String {
    format!("{:.8}", zatoshis as f64 / 100_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_example_receipt_against_raw_tx() {
        let receipt = include_str!("../../../examples/mainnet-receipt.json");
        let raw_tx = include_str!("../../../examples/mainnet-tx.hex");
        let result = verify_receipt(receipt, raw_tx).expect("example receipt should verify");
        assert_eq!(result.status, "verified");
        assert_eq!(result.verifier, "glasspane-wasm");
        assert_eq!(result.pool, Pool::Orchard);
        assert_eq!(result.output_index, 1);
        assert_eq!(result.amount_zatoshis, 100_000);
        assert_eq!(result.amount_zec, "0.00100000");
        assert_eq!(result.memo, "glasspane first receipt");
        assert_eq!(result.tx_id, result.parsed_tx_id);
        assert!(result.recipient.starts_with("u1"));
    }

    #[test]
    fn rejects_tampered_receipt_loudly() {
        let receipt = include_str!(
            "../../../examples/rooms/zechub-demo/receipts/mainnet-receipt-tampered.json"
        );
        let raw_tx = include_str!("../../../examples/mainnet-tx.hex");
        let err = verify_receipt(receipt, raw_tx).expect_err("tampered OCK must fail");
        assert!(
            err.contains("OCK does not match this Orchard action"),
            "{err}"
        );
    }

    #[test]
    fn rejects_raw_tx_mismatch() {
        let receipt = include_str!("../../../examples/mainnet-receipt.json");
        let mut value: serde_json::Value = serde_json::from_str(receipt).unwrap();
        value["tx_id"] = serde_json::Value::String(
            "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        );
        let receipt = serde_json::to_string(&value).unwrap();
        let raw_tx = include_str!("../../../examples/mainnet-tx.hex");
        let err = verify_receipt(&receipt, raw_tx).expect_err("mismatched txid must fail");
        assert!(err.contains("raw transaction txid mismatch"), "{err}");
    }
}
