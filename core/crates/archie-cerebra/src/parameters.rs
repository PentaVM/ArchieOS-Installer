//! Encrypted local parameter checkpoints. No trainer examples are written here.
//! Files inherit the operator's directory ACL; keys remain caller-owned in memory.
use archie_memory::utility::ContextUtilityModel;
use archie_types::PrivacyGrade;
use archie_vault::{EntryClass, SealedEntry, Vault, VaultKey, VaultRef};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

const MAX_SEALED_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ParameterError {
    #[error("parameter checkpoint I/O failed")]
    Io(#[from] std::io::Error),
    #[error("parameter checkpoint is invalid, expired or cannot be decrypted")]
    Invalid,
}

/// Creates an encrypted checkpoint at a new path. Existing checkpoints are never
/// overwritten silently; the operator can remove old files after a successful save.
pub fn save_utility(
    path: &Path,
    key: &VaultKey,
    model: &ContextUtilityModel,
    now_ms: i64,
) -> Result<(), ParameterError> {
    let bytes = model.snapshot();
    ContextUtilityModel::restore(&bytes, now_ms).map_err(|_| ParameterError::Invalid)?;
    save(
        path,
        key,
        "vault:parameters/context-utility",
        &bytes,
        model.expires_at_ms(),
        now_ms,
    )
}

pub fn load_utility(
    path: &Path,
    key: &VaultKey,
    now_ms: i64,
) -> Result<ContextUtilityModel, ParameterError> {
    let bytes = load(path, key, "vault:parameters/context-utility", now_ms)?;
    ContextUtilityModel::restore(&bytes, now_ms).map_err(|_| ParameterError::Invalid)
}

fn save(
    path: &Path,
    key: &VaultKey,
    reference: &str,
    bytes: &[u8],
    expiry: i64,
    now_ms: i64,
) -> Result<(), ParameterError> {
    let mut vault = Vault::open(key).map_err(|_| ParameterError::Invalid)?;
    vault
        .store(
            "parameter-checkpoint",
            VaultRef::parse(reference).map_err(|_| ParameterError::Invalid)?,
            EntryClass::Preference,
            PrivacyGrade::Restricted,
            std::str::from_utf8(bytes).map_err(|_| ParameterError::Invalid)?,
            now_ms,
            Some(expiry),
        )
        .map_err(|_| ParameterError::Invalid)?;
    let sealed =
        serde_json::to_vec(&vault.sealed_entries()).map_err(|_| ParameterError::Invalid)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&sealed)?;
    file.sync_all()?;
    Ok(())
}

fn load(
    path: &Path,
    key: &VaultKey,
    reference: &str,
    now_ms: i64,
) -> Result<Vec<u8>, ParameterError> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_SEALED_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SEALED_BYTES {
        return Err(ParameterError::Invalid);
    }
    let sealed: Vec<SealedEntry> =
        serde_json::from_slice(&bytes).map_err(|_| ParameterError::Invalid)?;
    if sealed.len() != 1
        || sealed[0].grade != PrivacyGrade::Restricted
        || sealed[0].class != EntryClass::Preference
    {
        return Err(ParameterError::Invalid);
    }
    let mut vault = Vault::load(key, sealed).map_err(|_| ParameterError::Invalid)?;
    let disclosure = vault
        .disclose(
            "parameter-restore",
            &VaultRef::parse(reference).map_err(|_| ParameterError::Invalid)?,
            PrivacyGrade::Restricted,
            now_ms,
        )
        .map_err(|_| ParameterError::Invalid)?;
    Ok(disclosure
        .text()
        .ok_or(ParameterError::Invalid)?
        .as_bytes()
        .to_vec())
}
