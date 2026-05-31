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
//! v0 implements both Orchard and Sapling pools.

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use gp_core::{
    ock_from_bytes, recover_orchard, recover_sapling, OrchardDisclosure, SaplingDisclosure,
};
use gp_types::{Network, Pool, Receipt};
use tonic::transport::Channel;
use zcash_address::{
    unified::{self, Encoding, Receiver},
    ToAddress, ZcashAddress,
};
use zcash_client_backend::proto::service::{
    compact_tx_streamer_client::CompactTxStreamerClient, TxFilter,
};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::NetworkType;
use zcash_protocol::consensus::{BranchId, MainNetwork, NetworkUpgrade, Parameters};

#[derive(Parser, Debug)]
#[command(
    name = "gp-verify",
    version,
    about = "Verify a Glasspane receipt against Zcash mainnet"
)]
struct Args {
    /// Source of the receipt to verify. Can be:
    ///   * a path to a receipt JSON file,
    ///   * a Glasspane URL of the form `https://<host>/r/<base64url-json>`,
    ///     or a bare `<base64url-json>` payload,
    ///   * omitted, in which case stdin is read.
    receipt: Option<String>,

    /// lightwalletd endpoint URL. Default: zec.rocks (used by Zashi wallet).
    /// Pass your own if this default is unreachable from your network.
    #[arg(long, default_value = "https://zec.rocks:443")]
    lightwalletd: String,

    /// Validate only the receipt envelope. Skip chain verification.
    #[arg(long, default_value_t = false)]
    envelope_only: bool,

    /// Offline mode: read the raw transaction hex from this file instead of
    /// fetching it from lightwalletd. Useful where outbound gRPC is blocked.
    /// Get the hex from any block explorer's raw-transaction endpoint.
    #[arg(long)]
    raw_tx_file: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // 1. Load the receipt. Accept URL, file path, or stdin.
    let receipt: Receipt = match args.receipt.as_deref() {
        Some(arg) if looks_like_url(arg) => Receipt::from_url(arg).context("parse receipt URL")?,
        Some(path) => {
            let raw = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
            let r: Receipt = serde_json::from_str(&raw).context("parse receipt JSON")?;
            r.validate().context("receipt envelope invalid")?;
            r
        }
        None => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            // stdin may carry either JSON or a URL.
            let trimmed = buf.trim();
            if looks_like_url(trimmed) {
                Receipt::from_url(trimmed).context("parse receipt URL from stdin")?
            } else {
                let r: Receipt =
                    serde_json::from_str(&buf).context("parse receipt JSON from stdin")?;
                r.validate().context("receipt envelope invalid")?;
                r
            }
        }
    };
    set_display_network(receipt.network);
    let tx_id_bytes = receipt.tx_id_bytes()?;
    let ock_bytes = receipt.ock_bytes()?;

    // Auto-verify any ed25519 signature attached to the envelope. We do this
    // before chain verification because a bad signature is informative on its
    // own: it means someone tampered with the receipt after issuance.
    match receipt.verify_signature_if_present() {
        Ok(true) => println!("  signature   : ed25519 OK"),
        Ok(false) => {} // no signature attached
        Err(e) => bail!("signature verification failed: {e}"),
    }

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

    // 2-4. Obtain + parse the transaction, either from a local raw-hex file
    // (offline mode) or from lightwalletd.
    println!();
    let raw_tx = match args.raw_tx_file.as_deref() {
        Some(path) => {
            println!("Reading raw transaction from {path} (offline mode) ...");
            let hex_str = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
            hex::decode(hex_str.trim()).context("decode raw tx hex")?
        }
        None => {
            println!("Fetching transaction from {} ...", args.lightwalletd);
            fetch_tx(&args.lightwalletd, tx_id_bytes).await?
        }
    };
    let tx = parse_tx(&raw_tx)?;
    println!(
        "  Transaction parsed (consensus branch={:?}).",
        tx.consensus_branch_id()
    );

    // 5-6. Recover the disclosed output.
    match receipt.pool {
        Pool::Orchard => verify_orchard(&tx, receipt.output_index, ock_bytes)?,
        Pool::Sapling => verify_sapling(&tx, receipt.output_index, ock_bytes)?,
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
    let resp = client
        .get_transaction(req)
        .await
        .context("GetTransaction RPC")?;
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

/// Run the Sapling recovery path against the output at `output_index`.
fn verify_sapling(tx: &Transaction, output_index: u32, ock: [u8; 32]) -> Result<()> {
    let bundle = tx
        .sapling_bundle()
        .ok_or_else(|| anyhow!("transaction has no Sapling bundle"))?;
    let outputs = bundle.shielded_outputs();
    let output = outputs
        .get(output_index as usize)
        .ok_or_else(|| anyhow!("transaction has no Sapling output at index {output_index}"))?;
    let ock_key = ock_from_bytes(ock);
    // Current mainnet (post-Canopy) enforces ZIP-212 unconditionally.
    let zip212 = sapling_crypto::note_encryption::Zip212Enforcement::On;

    match recover_sapling(output, &ock_key, zip212) {
        Ok(disc) => {
            display_sapling(&disc);
            Ok(())
        }
        Err(_) => bail!(
            "OCK does not match this Sapling output. The receipt's `ock` field is wrong,\n\
             the `output_index` is wrong, or the receipt was forged."
        ),
    }
}

fn display_sapling(disc: &SaplingDisclosure) {
    println!();
    println!("OUTPUT RECOVERED (Sapling)");
    let ua = encode_ua(
        receipt_network_for_display(),
        Pool::Sapling,
        disc.recipient.to_bytes(),
    );
    println!("  recipient   : {ua}");
    println!(
        "  value       : {} zatoshis ({:.8} ZEC)",
        disc.value.inner(),
        disc.value.inner() as f64 / 1e8
    );
    let memo_text = memo_to_display(&disc.memo).unwrap_or_else(|| "(non text memo)".to_string());
    println!("  memo        : {memo_text}");
    println!();
    println!("VERIFIED.");
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

/// Encode an Orchard recipient as a Unified Address (Bech32m, `u1...`).
fn encode_recipient(addr: &orchard::Address) -> String {
    let bytes = addr.to_raw_address_bytes();
    encode_ua(receipt_network_for_display(), Pool::Orchard, bytes)
}

// The display path doesn't have direct access to the receipt's network field
// because the helpers operate on disclosures. We thread it via this
// thread-local set at the top of `main`. Default is mainnet.
thread_local! {
    static DISPLAY_NETWORK: std::cell::Cell<Network> = const { std::cell::Cell::new(Network::Mainnet) };
}
fn set_display_network(n: Network) {
    DISPLAY_NETWORK.with(|c| c.set(n));
}
fn receipt_network_for_display() -> Network {
    DISPLAY_NETWORK.with(|c| c.get())
}

/// Wrap a raw 43-byte Orchard or Sapling receiver in a single-receiver
/// Unified Address and encode as Bech32m. Falls back to a raw hex display
/// if UA construction fails for any reason.
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

/// Heuristic: does this look like a Glasspane URL rather than a file path
/// or JSON blob? We treat "contains `/r/`" or "starts with http(s)://" as
/// URL.
fn looks_like_url(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("http://") || s.starts_with("https://") || s.contains("/r/")
}

/// Try to render a 512-byte memo as a UTF-8 string, trimming trailing zeros.
fn memo_to_display(memo: &[u8; 512]) -> Option<String> {
    let trimmed: Vec<u8> = memo.iter().copied().take_while(|&b| b != 0).collect();
    if trimmed.is_empty() {
        return Some("(empty memo)".to_string());
    }
    String::from_utf8(trimmed).ok()
}
