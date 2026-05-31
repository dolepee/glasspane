//! gp-keygen: generate a Zcash mainnet test wallet for Glasspane.
//!
//! Produces a standard BIP39 24-word mnemonic and derives the ZIP-32
//! account-0 keys for Zcash mainnet. Prints:
//!   - the mnemonic (back this up; import into YWallet or Zashi to spend)
//!   - the Unified Address (u1...) for receiving
//!   - a transparent address (t1...) for swap services that only accept
//!     transparent payout addresses (e.g. some ChangeNOW routes)
//!   - the Orchard Outgoing Viewing Key (OVK) in hex, which gp-issue needs
//!
//! SECURITY: this is a hot test wallet generated on this machine. Only fund
//! it with the small amount needed for the Glasspane mainnet test. Do not
//! reuse this mnemonic for real holdings.
//!
//! The derivation (BIP39 seed -> ZIP-32 account 0 -> Orchard external scope)
//! matches what YWallet and Zashi do when you import the same mnemonic, so
//! the printed OVK matches the account you will actually spend from.

use anyhow::{Context, Result};
use bip0039::{Count, English, Mnemonic};
use clap::Parser;
use orchard::keys::Scope;
use zcash_keys::address::Address;
use zcash_keys::keys::{UnifiedAddressRequest, UnifiedSpendingKey};
use zcash_protocol::consensus::MainNetwork;
use zcash_transparent::keys::IncomingViewingKey;
use zip32::AccountId;

#[derive(Parser, Debug)]
#[command(
    name = "gp-keygen",
    version,
    about = "Generate a Zcash mainnet test wallet for Glasspane"
)]
struct Args {
    /// Restore from an existing 24-word mnemonic instead of generating a new
    /// one. Useful to re-derive the OVK + addresses for a wallet you already
    /// have. Quote the whole phrase.
    #[arg(long)]
    mnemonic: Option<String>,

    /// Write the secret material (mnemonic + OVK) to this file in addition to
    /// printing. The path should be OUTSIDE any git repo. Default: print only.
    #[arg(long)]
    out: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // 1. Get a mnemonic (generate fresh, or restore the one provided).
    let mnemonic: Mnemonic<English> = match args.mnemonic.as_deref() {
        Some(phrase) => Mnemonic::from_phrase(phrase.trim()).context("invalid 24-word mnemonic")?,
        None => Mnemonic::generate(Count::Words24),
    };
    let phrase = mnemonic.phrase().to_string();
    let seed = mnemonic.to_seed("");

    // 2. Derive account-0 unified spending key for Zcash mainnet.
    let account = AccountId::try_from(0u32).expect("0 is a valid account id");
    let usk = UnifiedSpendingKey::from_seed(&MainNetwork, &seed, account)
        .map_err(|e| anyhow::anyhow!("derive unified spending key from seed: {e:?}"))?;
    let ufvk = usk.to_unified_full_viewing_key();

    // 3. Orchard Outgoing Viewing Key (external scope) -> hex for gp-issue.
    let orchard_fvk = ufvk
        .orchard()
        .context("derived key has no Orchard component")?;
    let ovk = orchard_fvk.to_ovk(Scope::External);
    let ovk_hex = hex::encode(ovk.as_ref());

    // 4. Unified Address (all receiver types) for receiving funds.
    let (ua, _) = ufvk
        .default_address(UnifiedAddressRequest::ALLOW_ALL)
        .map_err(|e| anyhow::anyhow!("derive unified address: {e:?}"))?;
    let ua_str: String = ua.encode(&MainNetwork);

    // 5. Transparent t1 address for swap services that require it.
    let (taddr, _) = usk
        .transparent()
        .to_account_pubkey()
        .derive_external_ivk()
        .map_err(|e| anyhow::anyhow!("derive transparent external ivk: {e:?}"))?
        .default_address();
    let taddr_str = Address::from(taddr).encode(&MainNetwork);

    // Output.
    println!("=== Glasspane Zcash mainnet test wallet ===");
    println!();
    println!("MNEMONIC (back this up, import into YWallet or Zashi to spend):");
    println!("  {phrase}");
    println!();
    println!("RECEIVE — Unified Address (give to a swap that supports UAs):");
    println!("  {ua_str}");
    println!();
    println!("RECEIVE — transparent t1 address (give to ChangeNOW / swaps that need t-addr):");
    println!("  {taddr_str}");
    println!();
    println!("OVK (feed to gp-issue --ovk for the Orchard pool):");
    println!("  {ovk_hex}");
    println!();
    println!("Next:");
    println!("  1. Import the mnemonic into YWallet or Zashi (restore from seed).");
    println!("  2. Fund the t1 address from ChangeNOW (or the UA if the swap supports it).");
    println!("  3. In the wallet, shield the funds to Orchard, then send a small");
    println!("     shielded payment to your own UA (this is the payment you will disclose).");
    println!("  4. Run: gp-issue --pool orchard --tx-id <txid> --output-index 0 \\");
    println!("            --ovk {ovk_hex} --label \"first glasspane receipt\" --out receipt.json");
    println!("  5. Run: gp-verify receipt.json");
    println!();
    println!("SECURITY: hot test wallet. Fund only the small test amount. Do not reuse for real holdings.");

    if let Some(path) = args.out {
        let blob = serde_json::json!({
            "network": "mainnet",
            "account": 0,
            "mnemonic": phrase,
            "unified_address": ua_str,
            "transparent_address": taddr_str,
            "orchard_ovk_hex": ovk_hex,
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "note": "Hot test wallet for Glasspane. Keep this file OUT of any git repo."
        });
        std::fs::write(&path, serde_json::to_string_pretty(&blob)?)
            .with_context(|| format!("write secret material to {path}"))?;
        eprintln!("Wrote wallet material to {path} (keep it private, outside any repo).");
    }

    Ok(())
}
