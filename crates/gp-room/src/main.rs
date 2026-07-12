//! gp-room: verify a Glasspane payout room from receipts + raw Zcash txs.
//!
//! A room is the product layer over Glasspane receipts: teams disclose only
//! selected shielded payouts, their memos, and totals while the rest of the
//! wallet remains opaque.

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use gp_core::{
    ensure_transaction_id, ock_from_bytes, parse_transaction, recover_orchard, recover_sapling,
};
use gp_types::{Network, Pool, Receipt};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use zcash_address::{
    unified::{self, Encoding, Receiver},
    ToAddress, ZcashAddress,
};
use zcash_note_encryption::OutgoingCipherKey;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::NetworkType;

#[derive(Parser, Debug)]
#[command(
    name = "gp-room",
    version,
    about = "Verify a Glasspane payout room from receipts and raw transactions"
)]
struct Args {
    /// Path to room.json.
    room: PathBuf,

    /// Where to write verified-room.json. If omitted, JSON is printed.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Optional CSV export path for accounting tools.
    #[arg(long)]
    csv: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let report = verify_room_file(&args.room)?;
    let json = serde_json::to_string_pretty(&report).context("serialize verified room")?;

    if let Some(out) = args.out {
        std::fs::write(&out, json).with_context(|| format!("write {}", out.display()))?;
    } else {
        println!("{json}");
    }

    if let Some(csv) = args.csv {
        std::fs::write(&csv, room_to_csv(&report))
            .with_context(|| format!("write {}", csv.display()))?;
    }

    if !report.overall_pass {
        bail!("room verification failed: {}", report.failures.join("; "));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
struct Room {
    version: String,
    title: String,
    purpose: String,
    privacy_boundary: String,
    network: Network,
    expected: RoomExpected,
    receipts: Vec<RoomReceipt>,
}

#[derive(Debug, Clone, Deserialize)]
struct RoomExpected {
    memo_labels: Vec<String>,
    #[serde(default)]
    min_total_zatoshis: Option<u64>,
    #[serde(default)]
    total_zatoshis: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct RoomReceipt {
    id: String,
    label: String,
    role: String,
    receipt_path: PathBuf,
    raw_tx_path: PathBuf,
    #[serde(default)]
    expected_memo: Option<String>,
    #[serde(default)]
    expected_min_zatoshis: Option<u64>,
    #[serde(default)]
    expected_zatoshis: Option<u64>,
    #[serde(default)]
    expected_outcome: ExpectedOutcome,
    #[serde(default)]
    tamper: bool,
    #[serde(default)]
    tx_url: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum ExpectedOutcome {
    #[default]
    Verified,
    Rejected,
}

#[derive(Debug, Serialize)]
struct VerifiedRoom {
    version: String,
    title: String,
    purpose: String,
    privacy_boundary: String,
    network: Network,
    generated_at: chrono::DateTime<chrono::Utc>,
    overall_pass: bool,
    verified_count: usize,
    rejected_count: usize,
    total_zatoshis: u64,
    total_zec: String,
    expected_memo_labels: Vec<String>,
    results: Vec<ReceiptReport>,
    failures: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ReceiptReport {
    id: String,
    label: String,
    role: String,
    expected_outcome: ExpectedOutcome,
    status: ReceiptStatus,
    expected_result_observed: bool,
    tamper: bool,
    tx_id: Option<String>,
    tx_url: Option<String>,
    pool: Option<Pool>,
    output_index: Option<u32>,
    amount_zatoshis: Option<u64>,
    amount_zec: Option<String>,
    memo: Option<String>,
    recipient: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ReceiptStatus {
    Verified,
    Rejected,
}

#[derive(Debug)]
struct RecoveredOutput {
    recipient: String,
    value_zatoshis: u64,
    memo: String,
}

fn verify_room_file(path: &Path) -> Result<VerifiedRoom> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let room: Room = serde_json::from_str(&raw).context("parse room JSON")?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    verify_room(&room, base_dir)
}

fn verify_room(room: &Room, base_dir: &Path) -> Result<VerifiedRoom> {
    if room.version != "0" {
        bail!("unsupported room version: {}", room.version);
    }

    let mut results = Vec::with_capacity(room.receipts.len());
    let mut failures = Vec::new();
    let mut total_zatoshis = 0u64;

    for entry in &room.receipts {
        let result = verify_entry(entry, room.network, base_dir);
        let report = match result {
            Ok((receipt, recovered)) => {
                let mut errors = Vec::new();
                if let Some(expected) = &entry.expected_memo {
                    if recovered.memo != *expected {
                        errors.push(format!(
                            "memo mismatch: expected {expected:?}, recovered {:?}",
                            recovered.memo
                        ));
                    }
                }
                if let Some(expected) = entry.expected_zatoshis {
                    if recovered.value_zatoshis != expected {
                        errors.push(format!(
                            "amount mismatch: expected {expected} zatoshis, recovered {}",
                            recovered.value_zatoshis
                        ));
                    }
                }
                if let Some(min) = entry.expected_min_zatoshis {
                    if recovered.value_zatoshis < min {
                        errors.push(format!(
                            "amount below minimum: expected at least {min} zatoshis, recovered {}",
                            recovered.value_zatoshis
                        ));
                    }
                }

                if errors.is_empty() {
                    total_zatoshis = total_zatoshis.saturating_add(recovered.value_zatoshis);
                    receipt_report(
                        entry,
                        &receipt,
                        ReceiptStatus::Verified,
                        Some(recovered),
                        None,
                    )
                } else {
                    receipt_report(
                        entry,
                        &receipt,
                        ReceiptStatus::Rejected,
                        None,
                        Some(errors.join("; ")),
                    )
                }
            }
            Err(err) => {
                let metadata = receipt_metadata(entry, base_dir);
                ReceiptReport {
                    id: entry.id.clone(),
                    label: entry.label.clone(),
                    role: entry.role.clone(),
                    expected_outcome: entry.expected_outcome,
                    status: ReceiptStatus::Rejected,
                    expected_result_observed: false,
                    tamper: entry.tamper,
                    tx_id: metadata.as_ref().map(|r| r.tx_id.clone()),
                    tx_url: entry
                        .tx_url
                        .clone()
                        .or_else(|| metadata.as_ref().and_then(explorer_url)),
                    pool: metadata.as_ref().map(|r| r.pool),
                    output_index: metadata.as_ref().map(|r| r.output_index),
                    amount_zatoshis: None,
                    amount_zec: None,
                    memo: None,
                    recipient: None,
                    error: Some(err.to_string()),
                }
            }
        };

        let expected_status = match entry.expected_outcome {
            ExpectedOutcome::Verified => ReceiptStatus::Verified,
            ExpectedOutcome::Rejected => ReceiptStatus::Rejected,
        };
        let observed = report.status == expected_status;
        let report = ReceiptReport {
            expected_result_observed: observed,
            ..report
        };

        if !observed {
            let msg = format!(
                "{} expected {:?} but got {:?}",
                entry.id, entry.expected_outcome, report.status
            );
            if let Some(error) = &report.error {
                failures.push(format!("{msg}: {error}"));
            } else {
                failures.push(msg);
            }
        }

        results.push(report);
    }

    for expected_memo in &room.expected.memo_labels {
        let observed = results.iter().any(|report| {
            report.status == ReceiptStatus::Verified
                && report.memo.as_deref() == Some(expected_memo.as_str())
        });
        if !observed {
            failures.push(format!(
                "expected memo was not recovered from a verified receipt: {expected_memo:?}"
            ));
        }
    }

    if let Some(min_total) = room.expected.min_total_zatoshis {
        if total_zatoshis < min_total {
            failures.push(format!(
                "room total below minimum: expected at least {min_total} zatoshis, recovered {total_zatoshis}"
            ));
        }
    }
    if let Some(expected_total) = room.expected.total_zatoshis {
        if total_zatoshis != expected_total {
            failures.push(format!(
                "room total mismatch: expected {expected_total} zatoshis, recovered {total_zatoshis}"
            ));
        }
    }

    let verified_count = results
        .iter()
        .filter(|r| r.status == ReceiptStatus::Verified)
        .count();
    let rejected_count = results
        .iter()
        .filter(|r| r.status == ReceiptStatus::Rejected)
        .count();

    Ok(VerifiedRoom {
        version: "0".to_string(),
        title: room.title.clone(),
        purpose: room.purpose.clone(),
        privacy_boundary: room.privacy_boundary.clone(),
        network: room.network,
        generated_at: chrono::Utc::now(),
        overall_pass: failures.is_empty(),
        verified_count,
        rejected_count,
        total_zatoshis,
        total_zec: zec_string(total_zatoshis),
        expected_memo_labels: room.expected.memo_labels.clone(),
        results,
        failures,
    })
}

fn receipt_report(
    entry: &RoomReceipt,
    receipt: &Receipt,
    status: ReceiptStatus,
    recovered: Option<RecoveredOutput>,
    error: Option<String>,
) -> ReceiptReport {
    ReceiptReport {
        id: entry.id.clone(),
        label: entry.label.clone(),
        role: entry.role.clone(),
        expected_outcome: entry.expected_outcome,
        status,
        expected_result_observed: false,
        tamper: entry.tamper,
        tx_id: Some(receipt.tx_id.clone()),
        tx_url: entry.tx_url.clone().or_else(|| explorer_url(receipt)),
        pool: Some(receipt.pool),
        output_index: Some(receipt.output_index),
        amount_zatoshis: recovered.as_ref().map(|r| r.value_zatoshis),
        amount_zec: recovered.as_ref().map(|r| zec_string(r.value_zatoshis)),
        memo: recovered.as_ref().map(|r| r.memo.clone()),
        recipient: recovered.map(|r| r.recipient),
        error,
    }
}

fn verify_entry(
    entry: &RoomReceipt,
    room_network: Network,
    base_dir: &Path,
) -> Result<(Receipt, RecoveredOutput)> {
    let receipt_path = resolve_room_path(base_dir, &entry.receipt_path);
    let receipt_raw = std::fs::read_to_string(&receipt_path)
        .with_context(|| format!("read receipt {}", receipt_path.display()))?;
    let receipt: Receipt = serde_json::from_str(&receipt_raw)
        .with_context(|| format!("parse receipt {}", receipt_path.display()))?;
    receipt.validate().context("receipt envelope invalid")?;
    if receipt.network != room_network {
        bail!(
            "receipt network {:?} does not match room network {:?}",
            receipt.network,
            room_network
        );
    }
    receipt
        .verify_signature_if_present()
        .context("receipt signature invalid")?;

    let raw_tx_path = resolve_room_path(base_dir, &entry.raw_tx_path);
    let raw_tx_hex = std::fs::read_to_string(&raw_tx_path)
        .with_context(|| format!("read raw tx {}", raw_tx_path.display()))?;
    let raw_tx = hex::decode(raw_tx_hex.trim()).context("decode raw tx hex")?;
    let tx = parse_transaction(&raw_tx)?;
    ensure_transaction_id(&tx, &receipt.tx_id)?;
    let ock = ock_from_bytes(receipt.ock_bytes()?);
    let recovered = match receipt.pool {
        Pool::Orchard => recover_orchard_output(&tx, receipt.output_index, &ock, receipt.network)?,
        Pool::Sapling => recover_sapling_output(&tx, receipt.output_index, &ock, receipt.network)?,
    };
    Ok((receipt, recovered))
}

fn receipt_metadata(entry: &RoomReceipt, base_dir: &Path) -> Option<Receipt> {
    let receipt_path = resolve_room_path(base_dir, &entry.receipt_path);
    let receipt_raw = std::fs::read_to_string(receipt_path).ok()?;
    let receipt: Receipt = serde_json::from_str(&receipt_raw).ok()?;
    receipt.validate().ok()?;
    Some(receipt)
}

fn resolve_room_path(base_dir: &Path, value: &Path) -> PathBuf {
    if value.is_absolute() {
        value.to_path_buf()
    } else {
        base_dir.join(value)
    }
}

fn recover_orchard_output(
    tx: &Transaction,
    output_index: u32,
    ock: &OutgoingCipherKey,
    network: Network,
) -> Result<RecoveredOutput> {
    let bundle = tx
        .orchard_bundle()
        .ok_or_else(|| anyhow!("transaction has no Orchard bundle"))?;
    let actions: Vec<_> = bundle.actions().iter().collect();
    let action = actions
        .get(output_index as usize)
        .ok_or_else(|| anyhow!("transaction has no Orchard action at index {output_index}"))?;
    let disc = recover_orchard(*action, ock).map_err(|_| {
        anyhow!("OCK does not match this Orchard action; the receipt is wrong, tampered, or forged")
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
) -> Result<RecoveredOutput> {
    let bundle = tx
        .sapling_bundle()
        .ok_or_else(|| anyhow!("transaction has no Sapling bundle"))?;
    let output = bundle
        .shielded_outputs()
        .get(output_index as usize)
        .ok_or_else(|| anyhow!("transaction has no Sapling output at index {output_index}"))?;
    let zip212 = sapling_crypto::note_encryption::Zip212Enforcement::On;
    let disc = recover_sapling(output, ock, zip212).map_err(|_| {
        anyhow!("OCK does not match this Sapling output; the receipt is wrong, tampered, or forged")
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
        Err(_) => format!("{:?}:{}", pool, hex::encode(bytes)),
    }
}

fn memo_to_display(memo: &[u8; 512]) -> Option<String> {
    let trimmed: Vec<u8> = memo.iter().copied().take_while(|&b| b != 0).collect();
    if trimmed.is_empty() {
        return Some("(empty memo)".to_string());
    }
    String::from_utf8(trimmed).ok()
}

fn explorer_url(receipt: &Receipt) -> Option<String> {
    match receipt.network {
        Network::Mainnet => Some(format!(
            "https://mainnet.zcashexplorer.app/transactions/{}",
            receipt.tx_id
        )),
        Network::Testnet => Some(format!(
            "https://testnet.zcashexplorer.app/transactions/{}",
            receipt.tx_id
        )),
        Network::Regtest => None,
    }
}

fn zec_string(zatoshis: u64) -> String {
    format!("{:.8}", zatoshis as f64 / 100_000_000.0)
}

fn room_to_csv(report: &VerifiedRoom) -> String {
    let mut out = String::from("status,recipient,memo,amount ZEC,tx id,verified-at\n");
    for row in &report.results {
        let fields = [
            format!("{:?}", row.status).to_lowercase(),
            row.recipient.clone().unwrap_or_default(),
            row.memo
                .clone()
                .or_else(|| row.error.clone())
                .unwrap_or_default(),
            row.amount_zec.clone().unwrap_or_default(),
            row.tx_id.clone().unwrap_or_default(),
            report.generated_at.to_rfc3339(),
        ];
        out.push_str(
            &fields
                .iter()
                .map(|value| csv_escape(value))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    out
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_room() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/rooms/zechub-demo/room.json")
    }

    #[test]
    fn example_room_verifies_and_records_expected_tamper() {
        let report = verify_room_file(&fixture_room()).expect("room should verify");
        assert!(report.overall_pass, "{:?}", report.failures);
        assert_eq!(report.verified_count, 1);
        assert_eq!(report.rejected_count, 1);
        assert_eq!(report.total_zatoshis, 100_000);
        assert!(report.results.iter().any(|r| {
            r.id == "tampered-ock"
                && r.status == ReceiptStatus::Rejected
                && r.expected_result_observed
        }));
    }

    #[test]
    fn unexpected_tamper_fails_loudly() {
        let room_path = fixture_room();
        let raw = std::fs::read_to_string(&room_path).expect("read fixture room");
        let mut room: Room = serde_json::from_str(&raw).expect("parse fixture room");
        let base_dir = room_path.parent().expect("fixture has a parent");
        let tampered = room
            .receipts
            .iter_mut()
            .find(|entry| entry.id == "tampered-ock")
            .expect("tampered entry exists");
        tampered.expected_outcome = ExpectedOutcome::Verified;
        let report = verify_room(&room, base_dir).expect("room report should still be produced");
        assert!(!report.overall_pass);
        assert!(report
            .failures
            .iter()
            .any(|f| f.contains("tampered-ock expected Verified but got Rejected")));
    }

    #[test]
    fn csv_export_contains_accounting_rows() {
        let report = verify_room_file(&fixture_room()).expect("room should verify");
        let csv = room_to_csv(&report);
        assert!(csv.starts_with("status,recipient,memo,amount ZEC,tx id,verified-at\n"));
        assert!(csv.contains("verified,u1"));
        assert!(csv.contains("glasspane first receipt,0.00100000,66167cd3020eb329"));
        assert!(csv.contains(
            "\"OCK does not match this Orchard action; the receipt is wrong, tampered, or forged\""
        ));
    }

    #[test]
    fn empty_room_is_an_honest_zero_value_report() {
        let room = Room {
            version: "0".to_string(),
            title: "Empty payout ledger".to_string(),
            purpose: "Publish receipts only after payouts happen.".to_string(),
            privacy_boundary: "No receipt outputs are disclosed yet.".to_string(),
            network: Network::Mainnet,
            expected: RoomExpected {
                memo_labels: vec![],
                min_total_zatoshis: None,
                total_zatoshis: Some(0),
            },
            receipts: vec![],
        };

        let report = verify_room(&room, Path::new(".")).expect("empty room should report");
        assert!(report.overall_pass, "{:?}", report.failures);
        assert_eq!(report.verified_count, 0);
        assert_eq!(report.rejected_count, 0);
        assert_eq!(report.total_zatoshis, 0);
        assert!(report.results.is_empty());
    }

    #[test]
    fn empty_room_fails_nonzero_expectations() {
        let room = Room {
            version: "0".to_string(),
            title: "Missing payout ledger".to_string(),
            purpose: "A payout is expected.".to_string(),
            privacy_boundary: "No receipt outputs are disclosed.".to_string(),
            network: Network::Mainnet,
            expected: RoomExpected {
                memo_labels: vec![],
                min_total_zatoshis: Some(1),
                total_zatoshis: None,
            },
            receipts: vec![],
        };

        let report = verify_room(&room, Path::new(".")).expect("room should report failure");
        assert!(!report.overall_pass);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("room total below minimum")));
    }

    #[test]
    fn empty_room_fails_expected_memo_claims() {
        let room = Room {
            version: "0".to_string(),
            title: "Missing payout ledger".to_string(),
            purpose: "A payout memo is expected.".to_string(),
            privacy_boundary: "No receipt outputs are disclosed.".to_string(),
            network: Network::Mainnet,
            expected: RoomExpected {
                memo_labels: vec!["missing payout".to_string()],
                min_total_zatoshis: None,
                total_zatoshis: None,
            },
            receipts: vec![],
        };

        let report = verify_room(&room, Path::new(".")).expect("room should report failure");
        assert!(!report.overall_pass);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.contains("expected memo was not recovered")));
    }
}
