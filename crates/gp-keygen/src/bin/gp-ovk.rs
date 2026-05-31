//! gp-ovk: extract the Orchard Outgoing Viewing Key from a wallet's exported
//! Unified Full Viewing Key (UFVK / `uview1...`).
//!
//! This is the derivation-agnostic way to get the OVK that `gp-issue` needs.
//! Instead of re-deriving keys from a seed (which can diverge between wallet
//! implementations), you export the UFVK from the exact wallet that holds and
//! spends the funds (zkool, YWallet, Zashi, zcashd) and run it through here.
//! The OVK that comes out provably belongs to that wallet's account.
//!
//! Usage:
//!   gp-ovk "uview1......"            # mainnet
//!   gp-ovk --network testnet "uview..."

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use orchard::keys::Scope;
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_protocol::consensus::{MainNetwork, TestNetwork};

#[derive(Parser, Debug)]
#[command(
    name = "gp-ovk",
    version,
    about = "Extract the Orchard OVK (for gp-issue) from a wallet's exported Unified Full Viewing Key"
)]
struct Args {
    /// The exported Unified Full Viewing Key string (begins with `uview1`).
    ufvk: String,

    /// Network the UFVK belongs to.
    #[arg(long, value_enum, default_value_t = Net::Mainnet)]
    network: Net,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum Net {
    Mainnet,
    Testnet,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let ufvk_str = args.ufvk.trim();

    let orchard_ovk_hex = match args.network {
        Net::Mainnet => {
            let ufvk = UnifiedFullViewingKey::decode(&MainNetwork, ufvk_str)
                .map_err(|e| anyhow!("decode UFVK (mainnet): {e}"))?;
            ovk_hex(&ufvk)?
        }
        Net::Testnet => {
            let ufvk = UnifiedFullViewingKey::decode(&TestNetwork, ufvk_str)
                .map_err(|e| anyhow!("decode UFVK (testnet): {e}"))?;
            ovk_hex(&ufvk)?
        }
    };

    println!("Orchard OVK (feed to gp-issue --ovk):");
    println!("  {orchard_ovk_hex}");
    Ok(())
}

fn ovk_hex(ufvk: &UnifiedFullViewingKey) -> Result<String> {
    let orchard_fvk = ufvk
        .orchard()
        .context("this UFVK has no Orchard component; cannot produce an Orchard OVK")?;
    let ovk = orchard_fvk.to_ovk(Scope::External);
    Ok(hex::encode(ovk.as_ref()))
}
