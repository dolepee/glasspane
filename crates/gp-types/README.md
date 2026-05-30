# gp-types

Receipt format types for [Glasspane](https://github.com/dolepee/glasspane) — per-output OCK based selective disclosure receipts for shielded Zcash payments.

This crate is the pure-Rust, no-network half of the Glasspane toolchain. It defines the v0 JSON envelope format, the shareable URL form (`https://host/r/<base64url-json>`), and optional ed25519 signing over the envelope. The actual cryptographic OCK derivation and Zcash transaction recovery live in the companion `gp-core` crate.

## Why use this directly

If you are building Zcash tooling and want to issue or verify Glasspane receipts as part of a larger flow (your own wallet, an accounting backend, a charity dashboard, an audit tool), this crate is the dependency you want. It pulls only `serde`, `base64`, `hex`, `chrono`, `thiserror`, and `ed25519-dalek`. No async runtime, no gRPC client, no protocol crates.

## Quickstart

```rust
use gp_types::{Network, Pool, Receipt};

let mut receipt = Receipt::new(
    Network::Mainnet,
    Pool::Orchard,
    /* tx_id    */ tx_id_bytes,
    /* output_index */ 0,
    /* ock      */ ock_bytes,
    /* label    */ "first glasspane receipt",
);

// Optional: sign the envelope with an ed25519 seed.
receipt.sign_ed25519(seed_32_bytes)?;

// Share as JSON, or as a single URL.
let url = receipt.to_url("https://glasspane.dev")?;

// On the verifier side:
let received = Receipt::from_url(&url)?;
assert!(received.verify_signature_if_present()?);
```

The full receipt schema, threat model, and protocol details live at <https://github.com/dolepee/glasspane>.

## License

MIT.
