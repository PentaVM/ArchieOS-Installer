//! Who vouches for a catalog.
//!
//! A catalog tells the agent what to download and run, so it is only read once a trusted
//! key has signed exactly its bytes. The signature is checked before the bytes are parsed:
//! an unverified catalog is not input.

use crate::catalog::{Catalog, CatalogError, MAX_CATALOG_BYTES};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

/// Signatures are over this tag followed by the catalog, so a signature made for anything
/// else with the same key never verifies as a catalog.
const DOMAIN: &[u8] = b"archie-toolpack/1\0";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    #[error("line {0} of the trusted keys is not an Ed25519 public key in 64 hex characters")]
    Key(usize),
    #[error("there are no trusted keys, so no catalog can be verified")]
    NoKeys,
    #[error("the signature is not 128 hex characters")]
    Malformed,
    #[error("no trusted key signed this catalog")]
    Untrusted,
    #[error("the catalog is larger than {MAX_CATALOG_BYTES} bytes")]
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenError {
    #[error(transparent)]
    Signature(#[from] SignatureError),
    #[error("the catalog is not UTF-8")]
    Encoding,
    #[error(transparent)]
    Catalog(#[from] CatalogError),
}

#[derive(Debug, Clone)]
pub struct TrustedKeys {
    keys: Vec<VerifyingKey>,
}

impl TrustedKeys {
    /// One key per line, as hex. Blank lines and lines starting with `#` are ignored.
    pub fn parse(text: &str) -> Result<Self, SignatureError> {
        let mut keys = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bytes: [u8; 32] = hex::decode(line)
                .ok()
                .and_then(|bytes| bytes.try_into().ok())
                .ok_or(SignatureError::Key(number + 1))?;
            keys.push(
                VerifyingKey::from_bytes(&bytes).map_err(|_| SignatureError::Key(number + 1))?,
            );
        }
        Ok(Self { keys })
    }

    pub fn from_keys(keys: Vec<VerifyingKey>) -> Self {
        Self { keys }
    }

    pub fn verify(&self, catalog: &[u8], signature_hex: &str) -> Result<(), SignatureError> {
        if self.keys.is_empty() {
            return Err(SignatureError::NoKeys);
        }
        if catalog.len() > MAX_CATALOG_BYTES {
            return Err(SignatureError::TooLarge);
        }
        let bytes: [u8; 64] = hex::decode(signature_hex.trim())
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(SignatureError::Malformed)?;
        let signature = Signature::from_bytes(&bytes);
        let message = signing_message(catalog);
        if self
            .keys
            .iter()
            .any(|key| key.verify_strict(&message, &signature).is_ok())
        {
            Ok(())
        } else {
            Err(SignatureError::Untrusted)
        }
    }
}

fn signing_message(catalog: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(DOMAIN.len() + catalog.len());
    message.extend_from_slice(DOMAIN);
    message.extend_from_slice(catalog);
    message
}

/// Signs a catalog, for release tooling. The result is the signature file's contents.
pub fn sign(catalog: &[u8], key: &SigningKey) -> String {
    hex::encode(key.sign(&signing_message(catalog)).to_bytes())
}

/// Verifies, and only then parses.
pub fn open_catalog(
    catalog: &[u8],
    signature_hex: &str,
    keys: &TrustedKeys,
) -> Result<Catalog, OpenError> {
    keys.verify(catalog, signature_hex)?;
    let text = std::str::from_utf8(catalog).map_err(|_| OpenError::Encoding)?;
    Ok(Catalog::parse(text)?)
}
