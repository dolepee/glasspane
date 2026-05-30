//! gp-verify: validate a Glasspane receipt against Zcash mainnet.
//!
//! Verifier flow:
//!   1. Parse the receipt JSON (gp-types::Receipt).
//!   2. Connect to lightwalletd via tonic.
//!   3. Fetch the transaction by tx_id with GetTransaction RPC.
//!   4. Parse the raw transaction bytes into a Zcash Transaction.
//!   5. Locate the Orchard action (or Sapling output) at the named index.
//!   6. Call gp_core::recover_orchard with the disclosed OCK.
//!   7. Display recipient + value + memo if recovery succeeds, FAIL otherwise.
//!
//! v0 implements Orchard. Sapling lands in v0.2.

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use gp_core::{ock_from_bytes, recover_orchard, OrchardDisclosure};
use gp_types::{Pool, Receipt};
use tonic::transport::Channel;
use zcash_client_backend::proto::service::{
    compact_tx_streamer_client::CompactTxStreamerClient, TxFilter,
};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BranchId, MainNetwork, NetworkUpgrade, Parameters};

#[derive(Parser, Debug)]
#[command(
    name = "gp-verify",
    version,
    about = "Verify a Glasspane receipt against Zcash mainnet"
)]
struct Args {
    /// Path to receipt JSON file. Default: read from stdin.
    receipt: Option<String>,

    /// lightwalletd endpoint URL. Default: zec.rocks (used by Zashi wallet).
    /// Pass your own if this default is unreachable from your network.
    #[arg(long, default_value = "https://zec.rocks:443")]
    lightwalletd: String,

    /// Validate only the receipt envelope. Skip chain verification.
    #[arg(long, default_value_t = false)]
    envelope_only: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // 1. Load the receipt.
    let raw = match args.receipt.as_deref() {
        Some(path) => std::fs::read_to_string(path).with_context(|| format!("read {path}"))?,
        None => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };
    let receipt: Receipt = serde_json::from_str(&raw).context("parse receipt JSON")?;
    receipt.validate().context("receipt envelope invalid")?;
    let tx_id_bytes = receipt.tx_id_bytes()?;
    let ock_bytes = receipt.ock_bytes()?;

    println!("RECEIPT  {}", &receipt.tx_id);
    println!("  pool        : {:?}", receipt.pool);
    println!("  network     : {:?}", receipt.network);
    println!("  output_index: {}", receipt.output_index);
    println!(
        "  label       : {}",
        if receipt.label.is_empty() {
            "(none)"
        } else {
            &receipt.label
        }
    );
    println!("  issued_at   : {}", receipt.issued_at);

    if args.envelope_only {
        println!();
        println!("ENVELOPE OK. Chain verification skipped (--envelope-only).");
        return Ok(());
    }

    // 2-4. Fetch + parse the transaction from lightwalletd.
    println!();
    println!("Fetching transaction from {} ...", args.lightwalletd);
    let raw_tx = fetch_tx(&args.lightwalletd, tx_id_bytes).await?;
    let tx = parse_tx(&raw_tx)?;
    println!("  Transaction fetched and parsed (consensus branch={:?}).", tx.consensus_branch_id());

    // 5-6. Recover the disclosed output.
    match receipt.pool {
        Pool::Orchard => verify_orchard(&tx, receipt.output_index, ock_bytes)?,
        Pool::Sapling => bail!(
            "Sapling pool verification is not yet implemented in v0.\n\
             Resend the payment via an Orchard-capable wallet, or wait for gp-core v0.2."
        ),
    }
    Ok(())
}

/// Fetch a raw mainnet transaction by txid via lightwalletd's GetTransaction
/// gRPC. Returns the raw transaction bytes ready for `Transaction::read`.
async fn fetch_tx(endpoint: &str, tx_id_bytes: [u8; 32]) -> Result<Vec<u8>> {
    // lightwalletd reports tx hashes in display (display-byte) order, but
    // the protocol's TxFilter.hash field expects the internal byte order
    // (the same as on-chain tx commitments). We reverse here.
    let mut hash = tx_id_bytes;
    hash.reverse();

    let channel = Channel::from_shared(endpoint.to_string())
        .with_context(|| format!("invalid lightwalletd endpoint URL {endpoint}"))?
        .connect()
        .await
        .with_context(|| format!("connect to lightwalletd at {endpoint}"))?;
    let mut client = CompactTxStreamerClient::new(channel);
    let req = TxFilter {
        block: None,
        index: 0,
        hash: hash.to_vec(),
    };
    let resp = client.get_transaction(req).await.context("GetTransaction RPC")?;
    let raw = resp.into_inner();
    if raw.data.is_empty() {
        bail!("lightwalletd returned an empty transaction for the given txid");
    }
    Ok(raw.data)
}

/// Parse raw tx bytes into a Zcash Transaction object. We have to tell the
/// parser the consensus branch in order to dispatch the right component
/// decoders; we use NU5 which is what current mainnet Orchard outputs live
/// under.
fn parse_tx(raw: &[u8]) -> Result<Transaction> {
    // NU5 activated on Zcash mainnet at the Orchard upgrade. Later upgrades
    // (NU6, etc.) maintain compatibility for tx parsing purposes here.
    let nu5_height = MainNetwork
        .activation_height(NetworkUpgrade::Nu5)
        .ok_or_else(|| anyhow!("MainNetwork is missing the NU5 activation height"))?;
    let branch_id = BranchId::for_height(&MainNetwork, nu5_height);
    Transaction::read(raw, branch_id).context("parse raw transaction")
}

/// Run the Orchard recovery path against the action at `output_index`.
fn verify_orchard(tx: &Transaction, output_index: u32, ock: [u8; 32]) -> Result<()> {
    let bundle = tx
        .orchard_bundle()
        .ok_or_else(|| anyhow!("transaction has no Orchard bundle"))?;
    let actions: Vec<_> = bundle.actions().iter().collect();
    let action = actions
        .get(output_index as usize)
        .ok_or_else(|| anyhow!("transaction has no Orchard action at index {output_index}"))?;
    let ock_key = ock_from_bytes(ock);

    match recover_orchard(*action, &ock_key) {
        Ok(disc) => {
            display(&disc);
            Ok(())
        }
        Err(_) => bail!(
            "OCK does not match this output. The receipt's `ock` field is wrong, the\n\
             `output_index` is wrong, or the receipt was forged."
        ),
    }
}

fn display(disc: &OrchardDisclosure) {
    println!();
    println!("OUTPUT RECOVERED");
    println!("  recipient   : {}", encode_recipient(&disc.recipient));
    println!(
        "  value       : {} zatoshis ({:.8} ZEC)",
        disc.value.inner(),
        disc.value.inner() as f64 / 1e8
    );
    let memo_text = match memo_to_display(&disc.memo) {
        Some(s) => s,
        None => "(non text memo)".to_string(),
    };
    println!("  memo        : {memo_text}");
    println!();
    println!("VERIFIED.");
}

/// Encode an Orchard recipient address. For v0 we display the 43 byte raw
/// address form as hex. Full UA encoding (with checksum + Bech32m) lands
/// in v0.2 when we wire `zcash_address`.
fn encode_recipient(addr: &orchard::Address) -> String {
    let bytes = addr.to_raw_address_bytes();
    format!("orchard:{}", hex::encode(bytes))
}

/// Try to render a 512-byte memo as a UTF-8 string, trimming trailing zeros.
fn memo_to_display(memo: &[u8; 512]) -> Option<String> {
    let trimmed: Vec<u8> = memo.iter().copied().take_while(|&b| b != 0).collect();
    if trimmed.is_empty() {
        return Some("(empty memo)".to_string());
    }
    String::from_utf8(trimmed).ok()
}
