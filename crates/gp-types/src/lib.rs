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

    /// Encode this receipt as a shareable URL of the form
    /// `https://<host>/r/<base64url-json>`. Anyone with the URL can
    /// reconstruct the full Receipt via `Receipt::from_url`.
    ///
    /// This is the "click this link" disclosure form — equivalent to
    /// sharing the JSON file, more convenient for chat / email / QR.
    pub fn to_url(&self, host: &str) -> Result<String, ReceiptError> {
        let json = serde_json::to_vec(self).map_err(|_| ReceiptError::SerializationFailed)?;
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&json);
        let trimmed_host = host.trim_end_matches('/');
        Ok(format!("{trimmed_host}/r/{encoded}"))
    }

    /// Compute the message bytes the ed25519 signature commits to:
    /// `tx_id (32 bytes) || output_index_le (4 bytes) || ock (32 bytes) ||
    ///  label_len_le (4 bytes) || label_utf8`.
    /// This is deterministic and stable across serialisations.
    pub fn signing_message(&self) -> Result<Vec<u8>, ReceiptError> {
        let tx = self.tx_id_bytes()?;
        let ock = self.ock_bytes()?;
        let label = self.label.as_bytes();
        let mut buf = Vec::with_capacity(32 + 4 + 32 + 4 + label.len());
        buf.extend_from_slice(&tx);
        buf.extend_from_slice(&self.output_index.to_le_bytes());
        buf.extend_from_slice(&ock);
        buf.extend_from_slice(&(label.len() as u32).to_le_bytes());
        buf.extend_from_slice(label);
        Ok(buf)
    }

    /// Sign the receipt envelope with an ed25519 signing key seed (32 bytes).
    /// Adds a `signature` field that any verifier can check later via
    /// `verify_signature_if_present`.
    pub fn sign_ed25519(&mut self, signing_key_seed: [u8; 32]) -> Result<(), ReceiptError> {
        use ed25519_dalek::{Signer, SigningKey};
        let sk = SigningKey::from_bytes(&signing_key_seed);
        let pk = sk.verifying_key();
        let msg = self.signing_message()?;
        let sig = sk.sign(&msg);
        self.signature = Some(Signature {
            scheme: "ed25519".into(),
            public_key: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(pk.to_bytes()),
            sig: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig.to_bytes()),
        });
        Ok(())
    }

    /// If a signature is attached, verify it. If no signature is attached,
    /// returns Ok(false). Returns Ok(true) on a valid signature, Err on a
    /// signature that is present but does not verify.
    pub fn verify_signature_if_present(&self) -> Result<bool, ReceiptError> {
        use ed25519_dalek::{Signature as Ed25519Sig, Verifier, VerifyingKey};
        let Some(sig_env) = &self.signature else {
            return Ok(false);
        };
        if sig_env.scheme != "ed25519" {
            return Err(ReceiptError::UnsupportedSignatureScheme(
                sig_env.scheme.clone(),
            ));
        }
        let pk_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&sig_env.public_key)
            .map_err(|_| ReceiptError::InvalidSignature)?;
        if pk_bytes.len() != 32 {
            return Err(ReceiptError::InvalidSignature);
        }
        let mut pk_arr = [0u8; 32];
        pk_arr.copy_from_slice(&pk_bytes);
        let pk = VerifyingKey::from_bytes(&pk_arr).map_err(|_| ReceiptError::InvalidSignature)?;

        let sig_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&sig_env.sig)
            .map_err(|_| ReceiptError::InvalidSignature)?;
        if sig_bytes.len() != 64 {
            return Err(ReceiptError::InvalidSignature);
        }
        let mut sig_arr = [0u8; 64];
        sig_arr.copy_from_slice(&sig_bytes);
        let sig = Ed25519Sig::from_bytes(&sig_arr);

        let msg = self.signing_message()?;
        pk.verify(&msg, &sig)
            .map(|_| true)
            .map_err(|_| ReceiptError::InvalidSignature)
    }

    /// Decode a Glasspane URL produced by `to_url` back into a Receipt.
    /// Accepts either the full URL `https://host/r/<data>` or just the
    /// `<data>` segment. Always validates the resulting receipt envelope.
    pub fn from_url(s: &str) -> Result<Self, ReceiptError> {
        let trimmed = s.trim();
        // Locate the `/r/` segment to find the data payload.
        let data = match trimmed.rsplit_once("/r/") {
            Some((_, data)) => data,
            None => trimmed,
        };
        let data = data.split(['?', '#']).next().unwrap_or(data);
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(data.as_bytes())
            .map_err(|_| ReceiptError::InvalidUrl)?;
        let receipt: Receipt =
            serde_json::from_slice(&bytes).map_err(|_| ReceiptError::InvalidUrl)?;
        receipt.validate()?;
        Ok(receipt)
    }
}

#[derive(Debug, Error)]
pub enum ReceiptError {
    #[error("unsupported receipt version: {0}")]
    UnsupportedVersion(String),
    #[error("invalid url (must contain /r/<base64url-json>)")]
    InvalidUrl,
    #[error("serialization failed")]
    SerializationFailed,
    #[error("invalid tx_id (must be 32 byte hex)")]
    InvalidTxId,
    #[error("invalid ock (must be base64url of 32 bytes)")]
    InvalidOck,
    #[error("label exceeds 120 characters")]
    LabelTooLong,
    #[error("unsupported signature scheme: {0}")]
    UnsupportedSignatureScheme(String),
    #[error("signature does not verify")]
    InvalidSignature,
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

    #[test]
    fn url_round_trip() {
        let mut tx_id = [0u8; 32];
        for (i, b) in tx_id.iter_mut().enumerate() {
            *b = (i * 7) as u8;
        }
        let mut ock = [0u8; 32];
        for (i, b) in ock.iter_mut().enumerate() {
            *b = (i * 13) as u8;
        }
        let r = Receipt::new(Network::Mainnet, Pool::Orchard, tx_id, 2, ock, "url demo");

        let url = r.to_url("https://glasspane.zec").unwrap();
        assert!(url.starts_with("https://glasspane.zec/r/"));

        let back = Receipt::from_url(&url).unwrap();
        assert_eq!(back.tx_id_bytes().unwrap(), tx_id);
        assert_eq!(back.ock_bytes().unwrap(), ock);
        assert_eq!(back.output_index, 2);
        assert_eq!(back.label, "url demo");
        assert_eq!(back.pool, Pool::Orchard);
    }

    #[test]
    fn from_url_accepts_bare_payload() {
        let r = Receipt::new(Network::Mainnet, Pool::Orchard, [1u8; 32], 0, [2u8; 32], "");
        let url = r.to_url("https://glasspane.zec").unwrap();
        let bare = url.rsplit_once("/r/").unwrap().1;
        let back = Receipt::from_url(bare).unwrap();
        assert_eq!(back.output_index, 0);
    }

    #[test]
    fn from_url_rejects_garbage() {
        assert!(Receipt::from_url("https://glasspane.zec/r/!!!notbase64!!!").is_err());
        assert!(Receipt::from_url("not a url at all").is_err());
    }

    #[test]
    fn ed25519_signature_round_trip() {
        // Deterministic signing key.
        let seed = [0x11u8; 32];
        let mut r = Receipt::new(
            Network::Mainnet,
            Pool::Orchard,
            [0x22u8; 32],
            0,
            [0x33u8; 32],
            "signed receipt",
        );
        assert!(!r.verify_signature_if_present().unwrap());
        r.sign_ed25519(seed).unwrap();
        assert!(r.signature.is_some());
        assert!(r.verify_signature_if_present().unwrap());
    }

    #[test]
    fn ed25519_signature_rejects_tampering() {
        let seed = [0x55u8; 32];
        let mut r = Receipt::new(
            Network::Mainnet,
            Pool::Orchard,
            [0x66u8; 32],
            0,
            [0x77u8; 32],
            "original",
        );
        r.sign_ed25519(seed).unwrap();
        // Mutate the label after signing. The signature should no longer verify.
        r.label = "tampered".into();
        assert!(matches!(
            r.verify_signature_if_present(),
            Err(ReceiptError::InvalidSignature)
        ));
    }
}
