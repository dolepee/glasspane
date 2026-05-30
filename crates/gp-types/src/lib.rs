//! Glasspane receipt format types.
//!
//! A Glasspane receipt v0 is a JSON document that lets a verifier check
//! a single shielded Zcash payment against mainnet, using the per-output
//! OCK (Out Cipher Key) as the disclosure primitive.
//!
//! See `spec/receipt.md` for the protocol-level description.

use base64::Engine;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Schema version of the Glasspane receipt format.
pub const RECEIPT_VERSION: &str = "0";

/// Which shielded pool the disclosed output lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pool {
    Sapling,
    Orchard,
}

/// Which Zcash network the receipt references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Mainnet,
    Testnet,
    Regtest,
}

/// Optional signature envelope. Signs `tx_id || output_index || ock || label`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    pub scheme: String,
    /// base64url(32 bytes ed25519 public key)
    pub public_key: String,
    /// base64url(64 bytes ed25519 signature)
    pub sig: String,
}

/// A Glasspane receipt v0.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub version: String,
    pub network: Network,
    pub pool: Pool,
    /// 32 byte transaction id, lowercase hex.
    pub tx_id: String,
    /// Index of the output (Sapling) or action (Orchard) inside the transaction.
    pub output_index: u32,
    /// base64url(32 bytes OCK)
    pub ock: String,
    /// Optional human-readable label, up to 120 chars.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// RFC3339 timestamp the receipt was issued.
    pub issued_at: chrono::DateTime<chrono::Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

impl Receipt {
    /// Construct a new unsigned v0 receipt.
    pub fn new(
        network: Network,
        pool: Pool,
        tx_id: [u8; 32],
        output_index: u32,
        ock: [u8; 32],
        label: impl Into<String>,
    ) -> Self {
        Self {
            version: RECEIPT_VERSION.to_string(),
            network,
            pool,
            tx_id: hex::encode(tx_id),
            output_index,
            ock: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(ock),
            label: label.into(),
            issued_at: chrono::Utc::now(),
            signature: None,
        }
    }

    /// Decode the OCK back into raw bytes.
    pub fn ock_bytes(&self) -> Result<[u8; 32], ReceiptError> {
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&self.ock)
            .map_err(|_| ReceiptError::InvalidOck)?;
        if raw.len() != 32 {
            return Err(ReceiptError::InvalidOck);
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&raw);
        Ok(out)
    }

    /// Decode the tx id back into raw bytes.
    pub fn tx_id_bytes(&self) -> Result<[u8; 32], ReceiptError> {
        let raw = hex::decode(&self.tx_id).map_err(|_| ReceiptError::InvalidTxId)?;
        if raw.len() != 32 {
            return Err(ReceiptError::InvalidTxId);
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&raw);
        Ok(out)
    }

    /// Validate basic envelope fields.
    pub fn validate(&self) -> Result<(), ReceiptError> {
        if self.version != RECEIPT_VERSION {
            return Err(ReceiptError::UnsupportedVersion(self.version.clone()));
        }
        if self.label.len() > 120 {
            return Err(ReceiptError::LabelTooLong);
        }
        let _ = self.tx_id_bytes()?;
        let _ = self.ock_bytes()?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ReceiptError {
    #[error("unsupported receipt version: {0}")]
    UnsupportedVersion(String),
    #[error("invalid tx_id (must be 32 byte hex)")]
    InvalidTxId,
    #[error("invalid ock (must be base64url of 32 bytes)")]
    InvalidOck,
    #[error("label exceeds 120 characters")]
    LabelTooLong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_orchard_receipt() {
        let mut tx_id = [0u8; 32];
        for (i, b) in tx_id.iter_mut().enumerate() {
            *b = i as u8;
        }
        let mut ock = [0u8; 32];
        for (i, b) in ock.iter_mut().enumerate() {
            *b = (255 - i) as u8;
        }
        let r = Receipt::new(Network::Mainnet, Pool::Orchard, tx_id, 0, ock, "test");
        r.validate().expect("valid");

        let s = serde_json::to_string(&r).unwrap();
        let back: Receipt = serde_json::from_str(&s).unwrap();
        back.validate().expect("round trip valid");
        assert_eq!(back.tx_id_bytes().unwrap(), tx_id);
        assert_eq!(back.ock_bytes().unwrap(), ock);
        assert_eq!(back.pool, Pool::Orchard);
    }

    #[test]
    fn rejects_bad_version() {
        let mut r = Receipt::new(Network::Mainnet, Pool::Sapling, [0u8; 32], 0, [0u8; 32], "");
        r.version = "999".to_string();
        assert!(r.validate().is_err());
    }

    #[test]
    fn rejects_long_label() {
        let r = Receipt::new(
            Network::Mainnet,
            Pool::Sapling,
            [0u8; 32],
            0,
            [0u8; 32],
            "x".repeat(121),
        );
        assert!(matches!(r.validate(), Err(ReceiptError::LabelTooLong)));
    }
}
