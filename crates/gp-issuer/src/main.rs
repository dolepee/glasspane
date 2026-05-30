//! gp-issue: produce a Glasspane receipt from a sent shielded payment.
//!
//! 24 hour gate scope: read the transaction from a local source or mainnet,
//! derive OCK from the sender's OVK + the output's on-chain (cv, cm_or_cmx, epk),
//! and write a v0 receipt JSON.
//!
//! In this stub binary the OCK is provided directly via --ock so the receipt
//! format end-to-end can be exercised before the cryptography path is fully wired.
//! The next step in the gate is to replace --ock with --ovk and have this binary
//! derive OCK by fetching the on-chain output via lightwalletd.

use anyhow::{bail, Context, Result};
use clap::Parser;
use gp_types::{Network, Pool, Receipt};

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

    /// Which shielded pool the disclosed output is in.
    #[arg(long, value_enum)]
    pool: PoolArg,

    /// 32 byte transaction id in hex (lowercase).
    #[arg(long)]
    tx_id: String,

    /// Index of the output (Sapling) or action (Orchard) inside the transaction.
    #[arg(long)]
    output_index: u32,

    /// Per-output Out Cipher Key (32 bytes, hex). For the 24 hour gate this is
    /// provided directly. In v0.2 it is derived from --ovk + on-chain output data.
    #[arg(long)]
    ock: String,

    /// Optional human-readable label, up to 120 chars.
    #[arg(long, default_value = "")]
    label: String,

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

fn main() -> Result<()> {
    let args = Args::parse();

    let tx_id = parse_hex_32(&args.tx_id).context("--tx-id must be 32 byte hex")?;
    let ock = parse_hex_32(&args.ock).context("--ock must be 32 byte hex")?;
    if args.label.len() > 120 {
        bail!("--label exceeds 120 characters");
    }

    let receipt = Receipt::new(
        args.network.into(),
        args.pool.into(),
        tx_id,
        args.output_index,
        ock,
        args.label,
    );
    receipt.validate()?;

    let json = serde_json::to_string_pretty(&receipt)?;
    match args.out {
        Some(path) => std::fs::write(&path, json).with_context(|| format!("write {path}"))?,
        None => println!("{json}"),
    }
    Ok(())
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
