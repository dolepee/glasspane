//! gp-issue: produce a Glasspane receipt from a sent shielded payment.
//!
//! Issuer flow:
//!   1. Operator names the transaction (--tx-id) and the output in it
//!      (--output-index --pool orchard) plus their Outgoing Viewing Key
//!      (--ovk, 32 byte hex) for the account that made the payment.
//!   2. Fetch the transaction via lightwalletd's GetTransaction RPC.
//!   3. Parse the raw bytes into a Zcash Transaction.
//!   4. Locate the Orchard action at output_index. Read cv_net, cmx,
//!      epk_bytes from the action's on-chain data.
//!   5. Call gp_core::derive_orchard_ock(ovk, cv, cmx, epk).
//!   6. Serialise a Glasspane receipt JSON with the derived OCK and
//!      write to stdout (or --out FILE).
//!
//! v0 implements Orchard. Sapling lands in v0.2.

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use gp_core::{derive_orchard_ock, ock_to_bytes, OrchardOckInputs};
use gp_types::{Network, Pool, Receipt};
use orchard::keys::OutgoingViewingKey;
use tonic::transport::Channel;
use zcash_client_backend::proto::service::{
    compact_tx_streamer_client::CompactTxStreamerClient, TxFilter,
};
use zcash_note_encryption::EphemeralKeyBytes;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BranchId, MainNetwork, NetworkUpgrade, Parameters};

#[derive(Parser, Debug)]
#[command(
    name = "gp-issue",
    version,
    about = "Produce a Glasspane receipt for a shielded Zcash payment"
)]
struct Args {
    /// Zcash network the transaction lives on.
    #[arg(long, value_enum, default_value_t = NetworkArg::Mainnet)]
    network: NetworkArg,

    /// Which shielded pool the disclosed output lives in.
    #[arg(long, value_enum)]
    pool: PoolArg,

    /// 32 byte transaction id in hex (lowercase, display byte order).
    #[arg(long)]
    tx_id: String,

    /// Index of the action (Orchard) or output (Sapling) inside the
    /// transaction.
    #[arg(long)]
    output_index: u32,

    /// Sender's Outgoing Viewing Key for the account that made the
    /// payment, as 32 hex bytes. The OCK is derived from this plus the
    /// on-chain action's (cv, cmx, epk).
    #[arg(long)]
    ovk: String,

    /// Optional human-readable label, up to 120 chars.
    #[arg(long, default_value = "")]
    label: String,

    /// lightwalletd endpoint URL. Default: zec.rocks.
    #[arg(long, default_value = "https://zec.rocks:443")]
    lightwalletd: String,

    /// Write the receipt JSON to this path. Default: stdout.
    #[arg(long)]
    out: Option<String>,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum NetworkArg {
    Mainnet,
    Testnet,
    Regtest,
}
impl From<NetworkArg> for Network {
    fn from(n: NetworkArg) -> Self {
        match n {
            NetworkArg::Mainnet => Network::Mainnet,
            NetworkArg::Testnet => Network::Testnet,
            NetworkArg::Regtest => Network::Regtest,
        }
    }
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum PoolArg {
    Sapling,
    Orchard,
}
impl From<PoolArg> for Pool {
    fn from(p: PoolArg) -> Self {
        match p {
            PoolArg::Sapling => Pool::Sapling,
            PoolArg::Orchard => Pool::Orchard,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if matches!(args.pool, PoolArg::Sapling) {
        bail!(
            "Sapling pool is not yet implemented in v0. Resend the payment via an\n\
             Orchard capable wallet, or wait for gp-core v0.2."
        );
    }

    let tx_id = parse_hex_32(&args.tx_id).context("--tx-id must be 32 byte hex")?;
    let ovk_bytes = parse_hex_32(&args.ovk).context("--ovk must be 32 byte hex")?;
    let ovk = OutgoingViewingKey::from(ovk_bytes);
    if args.label.len() > 120 {
        bail!("--label exceeds 120 characters");
    }

    // Fetch + parse the transaction.
    eprintln!("Fetching transaction from {} ...", args.lightwalletd);
    let raw_tx = fetch_tx(&args.lightwalletd, tx_id).await?;
    let tx = parse_tx(&raw_tx)?;
    eprintln!("  Transaction fetched and parsed.");

    // Locate the named Orchard action and derive OCK.
    let ock_bytes = derive_orchard_ock_from_chain(&tx, args.output_index, &ovk)?;

    // Build the receipt.
    let receipt = Receipt::new(
        args.network.into(),
        args.pool.into(),
        tx_id,
        args.output_index,
        ock_bytes,
        args.label,
    );
    receipt.validate()?;

    let json = serde_json::to_string_pretty(&receipt)?;
    match args.out {
        Some(path) => {
            std::fs::write(&path, &json).with_context(|| format!("write {path}"))?;
            eprintln!("Receipt written to {path}");
        }
        None => println!("{json}"),
    }
    Ok(())
}

async fn fetch_tx(endpoint: &str, tx_id_bytes: [u8; 32]) -> Result<Vec<u8>> {
    // lightwalletd's TxFilter.hash expects internal byte order
    // (reverse of the display order returned by explorers).
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

fn parse_tx(raw: &[u8]) -> Result<Transaction> {
    let nu5_height = MainNetwork
        .activation_height(NetworkUpgrade::Nu5)
        .ok_or_else(|| anyhow!("MainNetwork is missing the NU5 activation height"))?;
    let branch_id = BranchId::for_height(&MainNetwork, nu5_height);
    Transaction::read(raw, branch_id).context("parse raw transaction")
}

/// Locate the Orchard action at `output_index` in `tx`, extract its
/// (cv, cmx, epk), and derive the per-output OCK using `ovk`.
fn derive_orchard_ock_from_chain(
    tx: &Transaction,
    output_index: u32,
    ovk: &OutgoingViewingKey,
) -> Result<[u8; 32]> {
    let bundle = tx
        .orchard_bundle()
        .ok_or_else(|| anyhow!("transaction has no Orchard bundle"))?;
    let actions: Vec<_> = bundle.actions().iter().collect();
    let action = actions
        .get(output_index as usize)
        .ok_or_else(|| anyhow!("transaction has no Orchard action at index {output_index}"))?;

    let cv = action.cv_net();
    let cmx = action.cmx();
    let epk_bytes = EphemeralKeyBytes(action.encrypted_note().epk_bytes);

    let ock = derive_orchard_ock(&OrchardOckInputs {
        ovk,
        cv,
        cmx,
        epk_bytes: &epk_bytes,
    });

    Ok(ock_to_bytes(&ock))
}

fn parse_hex_32(s: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(s.trim()).context("not valid hex")?;
    if bytes.len() != 32 {
        bail!("expected 32 bytes, got {}", bytes.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}
