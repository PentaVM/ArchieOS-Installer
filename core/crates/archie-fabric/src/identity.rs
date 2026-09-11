//! Node identity.
//!
//! Every ArchieOS node holds one Ed25519 keypair, generated on first boot and never
//! transmitted. The public half is published in every announcement; the private half
//! stays in `/var/lib/archie/identity/node.key` at mode 0600.
//!
//! # The property this buys
//!
//! [`archie_types::NodeId`] is `sha256(tag || public_key)`. Because the id is *derived*
//! rather than chosen, a receiver can verify that a message claiming to come from node
//! X was signed by the only key that could produce id X. There is no registry to
//! consult and no certificate authority required — and when PKI is introduced later, it
//! layers on top of this rather than replacing it.

use archie_types::NodeId;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("identity file error: {0}")]
    Io(#[from] std::io::Error),

    #[error("malformed key material: {0}")]
    MalformedKey(String),

    #[error("signature verification failed")]
    BadSignature,

    #[error("node id does not match the presented public key")]
    IdKeyMismatch,

    #[error("refusing to use an identity key with unsafe permissions: {0}")]
    UnsafePermissions(String),
}

pub type Result<T> = std::result::Result<T, IdentityError>;

/// This node's keypair. Holding one of these means holding the node's identity, so it
/// is deliberately not `Clone` — it should be created once and shared by reference.
pub struct NodeIdentity {
    signing_key: SigningKey,
    node_id: NodeId,
}

impl std::fmt::Debug for NodeIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never render the private key, not even in a debug log.
        f.debug_struct("NodeIdentity")
            .field("node_id", &self.node_id.to_string())
            .field("signing_key", &"<redacted>")
            .finish()
    }
}

impl NodeIdentity {
    /// Create a brand-new identity from the OS entropy source.
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        Self::from_signing_key(signing_key)
    }

    fn from_signing_key(signing_key: SigningKey) -> Self {
        let node_id = NodeId::from_public_key_bytes(&signing_key.verifying_key().to_bytes());
        Self {
            signing_key,
            node_id,
        }
    }

    /// Reconstruct from raw private key bytes.
    pub fn from_private_bytes(bytes: &[u8; 32]) -> Self {
        Self::from_signing_key(SigningKey::from_bytes(bytes))
    }

    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Hex public key, as published in announcements.
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key_bytes())
    }

    /// Sign a message. Returns the raw 64-byte signature.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.sign(message).to_bytes()
    }

    /// Load the identity from disk, generating and persisting one if absent.
    ///
    /// This is what `archied` calls during boot step 1. It is idempotent: a node keeps
    /// the same identity across reboots, which is what makes peer key pinning
    /// meaningful.
    pub fn load_or_create(key_path: impl AsRef<Path>) -> Result<Self> {
        let key_path = key_path.as_ref();
        if key_path.exists() {
            return Self::load(key_path);
        }

        if let Some(parent) = key_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let identity = Self::generate();
        write_private_key(key_path, &identity.signing_key.to_bytes())?;

        // The public half is written alongside purely for operator convenience —
        // nothing reads it back, so a missing or stale copy is not an error.
        let public_path = key_path.with_extension("pub");
        let _ = std::fs::write(&public_path, identity.public_key_hex());

        Ok(identity)
    }

    /// Load an existing identity, refusing a world-readable key file.
    pub fn load(key_path: impl AsRef<Path>) -> Result<Self> {
        let key_path = key_path.as_ref();
        check_permissions(key_path)?;

        let raw = std::fs::read_to_string(key_path)?;
        let decoded = hex::decode(raw.trim())
            .map_err(|e| IdentityError::MalformedKey(format!("key file is not hex: {e}")))?;
        let bytes: [u8; 32] = decoded.try_into().map_err(|_| {
            IdentityError::MalformedKey("private key must be exactly 32 bytes".into())
        })?;
        Ok(Self::from_private_bytes(&bytes))
    }
}

/// Verify that `signature` over `message` was produced by the holder of `public_key`,
/// and that `claimed_id` really is that key's derived id.
///
/// Both checks together are what make announcements unforgeable: the first proves
/// possession of the key, the second proves the key belongs to the claimed identity.
pub fn verify_signed_by(
    claimed_id: &NodeId,
    public_key: &[u8; 32],
    message: &[u8],
    signature: &[u8; 64],
) -> Result<()> {
    if !claimed_id.matches_public_key(public_key) {
        return Err(IdentityError::IdKeyMismatch);
    }
    let verifying_key = VerifyingKey::from_bytes(public_key)
        .map_err(|e| IdentityError::MalformedKey(e.to_string()))?;
    let signature = Signature::from_bytes(signature);
    verifying_key
        .verify(message, &signature)
        .map_err(|_| IdentityError::BadSignature)
}

/// Parse a hex public key into raw bytes.
pub fn public_key_from_hex(value: &str) -> Result<[u8; 32]> {
    let decoded =
        hex::decode(value).map_err(|e| IdentityError::MalformedKey(format!("not hex: {e}")))?;
    decoded
        .try_into()
        .map_err(|_| IdentityError::MalformedKey("public key must be 32 bytes".into()))
}

#[cfg(unix)]
pub(crate) fn write_private_key(path: &Path, bytes: &[u8; 32]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    // Create with 0600 from the outset. Writing then chmod-ing would leave a window in
    // which the key is readable.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(hex::encode(bytes).as_bytes())?;
    file.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn write_private_key(path: &Path, bytes: &[u8; 32]) -> Result<()> {
    // Windows is a development host only. ArchieOS itself is Linux, where the mode is
    // enforced above.
    std::fs::write(path, hex::encode(bytes))?;
    Ok(())
}

#[cfg(unix)]
pub(crate) fn check_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(IdentityError::UnsafePermissions(format!(
            "{} is mode {:o}; expected 0600",
            path.display(),
            mode
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn check_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("archie-identity-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn node_id_is_the_derivation_of_the_public_key() {
        let identity = NodeIdentity::generate();
        assert!(identity
            .node_id()
            .matches_public_key(&identity.public_key_bytes()));
    }

    #[test]
    fn distinct_identities_have_distinct_ids() {
        assert_ne!(
            NodeIdentity::generate().node_id(),
            NodeIdentity::generate().node_id()
        );
    }

    #[test]
    fn a_signature_verifies_against_its_own_identity() {
        let identity = NodeIdentity::generate();
        let message = b"archie announce";
        let signature = identity.sign(message);
        assert!(verify_signed_by(
            &identity.node_id(),
            &identity.public_key_bytes(),
            message,
            &signature
        )
        .is_ok());
    }

    #[test]
    fn a_tampered_message_fails_verification() {
        let identity = NodeIdentity::generate();
        let signature = identity.sign(b"original");
        let err = verify_signed_by(
            &identity.node_id(),
            &identity.public_key_bytes(),
            b"tampered",
            &signature,
        )
        .unwrap_err();
        assert!(matches!(err, IdentityError::BadSignature));
    }

    #[test]
    fn another_nodes_signature_is_rejected() {
        let alice = NodeIdentity::generate();
        let mallory = NodeIdentity::generate();
        let message = b"i am alice";
        let forged = mallory.sign(message);
        // Mallory signs a message but presents Alice's identity.
        let err = verify_signed_by(
            &alice.node_id(),
            &alice.public_key_bytes(),
            message,
            &forged,
        )
        .unwrap_err();
        assert!(matches!(err, IdentityError::BadSignature));
    }

    #[test]
    fn presenting_someone_elses_key_under_a_claimed_id_is_rejected() {
        // Mallory signs honestly with her own key but claims Alice's node id. The
        // id/key binding must catch this before the signature is even checked.
        let alice = NodeIdentity::generate();
        let mallory = NodeIdentity::generate();
        let message = b"i am alice";
        let signature = mallory.sign(message);
        let err = verify_signed_by(
            &alice.node_id(),
            &mallory.public_key_bytes(),
            message,
            &signature,
        )
        .unwrap_err();
        assert!(matches!(err, IdentityError::IdKeyMismatch));
    }

    #[test]
    fn identity_is_stable_across_reloads() {
        let dir = tempdir("stable");
        let key = dir.join("node.key");

        let first = NodeIdentity::load_or_create(&key).unwrap();
        let second = NodeIdentity::load_or_create(&key).unwrap();
        assert_eq!(first.node_id(), second.node_id());
        assert_eq!(first.public_key_hex(), second.public_key_hex());

        // A signature from the reloaded identity verifies against the original id.
        let signature = second.sign(b"m");
        assert!(verify_signed_by(
            &first.node_id(),
            &first.public_key_bytes(),
            b"m",
            &signature
        )
        .is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn creating_an_identity_also_writes_the_public_half() {
        let dir = tempdir("pubhalf");
        let key = dir.join("node.key");
        let identity = NodeIdentity::load_or_create(&key).unwrap();
        let published = std::fs::read_to_string(key.with_extension("pub")).unwrap();
        assert_eq!(published.trim(), identity.public_key_hex());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_world_readable_key_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir("perms");
        let key = dir.join("node.key");
        NodeIdentity::load_or_create(&key).unwrap();

        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = NodeIdentity::load(&key).unwrap_err();
        assert!(matches!(err, IdentityError::UnsafePermissions(_)));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_newly_created_key_is_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir("mode");
        let key = dir.join("node.key");
        NodeIdentity::load_or_create(&key).unwrap();
        let mode = std::fs::metadata(&key).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "identity key must never be group/world readable"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_corrupt_key_file_is_reported_not_silently_regenerated() {
        let dir = tempdir("corrupt");
        let key = dir.join("node.key");
        std::fs::write(&key, "this is not a key").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Permission validation deliberately runs before parsing. Give this
            // fixture valid secret-file permissions so it reaches the corruption
            // branch this test is meant to exercise.
            std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        // Silently generating a new identity would change the node id and break every
        // peer's pinning, so this must be a hard error.
        assert!(matches!(
            NodeIdentity::load_or_create(&key).unwrap_err(),
            IdentityError::MalformedKey(_)
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn debug_never_leaks_the_private_key() {
        let identity = NodeIdentity::generate();
        let rendered = format!("{identity:?}");
        assert!(rendered.contains("redacted"));
        assert!(rendered.contains(&identity.node_id().to_string()));
    }

    #[test]
    fn hex_public_key_round_trips() {
        let identity = NodeIdentity::generate();
        let parsed = public_key_from_hex(&identity.public_key_hex()).unwrap();
        assert_eq!(parsed, identity.public_key_bytes());
        assert!(public_key_from_hex("not hex").is_err());
        assert!(public_key_from_hex("aabb").is_err());
    }
}
