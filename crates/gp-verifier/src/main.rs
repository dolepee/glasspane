//! gp-verify: validate a Glasspane receipt against Zcash mainnet.
//!
//! 24 hour gate scope: parse the receipt, validate envelope, print the
//! disclosure intent. The chain-side verification (fetching the tx,
//! calling try_output_recovery_with_ock) lands in the next step of the
//! gate once we have a real receipt to validate.

use anyhow::{Context, Result};
use clap::Parser;
use gp_types::Receipt;

#[derive(Parser, Debug)]
#[command(
    name = "gp-verify",
    version,
    about = "Verify a Glasspane receipt against Zcash mainnet"
)]
struct Args {
    /// Path to receipt JSON file. Default: read from stdin.
    receipt: Option<String>,

    /// lightwalletd endpoint to verify against. Default: a public endpoint.
    #[arg(long, default_value = "https://mainnet.lightwalletd.com:9067")]
    lightwalletd: String,

    /// Skip the chain verification and only validate the envelope.
    /// Useful for the 24 hour gate before the chain path is wired.
    #[arg(long, default_value_t = false)]
    envelope_only: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

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

    println!("RECEIPT  {}", &receipt.tx_id);
    println!("  pool        : {:?}", receipt.pool);
    println!("  network     : {:?}", receipt.network);
    println!("  output_index: {}", receipt.output_index);
    println!("  label       : {}", if receipt.label.is_empty() { "(none)" } else { &receipt.label });
    println!("  issued_at   : {}", receipt.issued_at);
    println!("  ock         : {} (32 bytes, base64url)", &receipt.ock);

    if args.envelope_only {
        println!();
        println!("ENVELOPE OK. Chain verification skipped (--envelope-only).");
        return Ok(());
    }

    println!();
    println!("Chain verification not yet implemented in this binary.");
    println!("Next step in the 24 hour gate: implement try_output_recovery_with_ock against {}", args.lightwalletd);
    println!("Required steps:");
    println!("  1. Fetch tx {} from lightwalletd", &receipt.tx_id);
    println!("  2. Locate output at index {} in {:?} pool", receipt.output_index, receipt.pool);
    println!("  3. Call try_output_recovery_with_ock(receipt.ock, output, output.out_ciphertext())");
    println!("  4. If Some(plaintext), display recipient + value + memo. If None, FAIL.");
    std::process::exit(2);
}
