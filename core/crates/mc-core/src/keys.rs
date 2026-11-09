//! Client keys and host-key trust for apps that manage their own SSH state.

use std::collections::HashMap;
use std::sync::Mutex;

pub use russh::keys::PrivateKey;
use russh::keys::ssh_key::{Algorithm, LineEnding};
use russh::keys::{HashAlg, PublicKey};

/// True for FIDO/U2F keys whose private part lives on a security key.
pub fn is_hardware_key(key: &PrivateKey) -> bool {
    matches!(
        key.algorithm(),
        Algorithm::SkEd25519 | Algorithm::SkEcdsaSha2NistP256
    )
}

use crate::error::{Error, Result};
use crate::transport::{HostKeyStatus, HostKeyVerifier};

/// A newly generated client key pair in OpenSSH formats.
#[derive(Clone)]
pub struct GeneratedKey {
    /// OpenSSH private key (unencrypted). The app must store it protected.
    pub private_openssh: String,
    /// `ssh-ed25519 AAAA… comment` line for `authorized_keys`.
    pub public_openssh: String,
    /// `SHA256:…`
    pub fingerprint: String,
}

/// Generate an Ed25519 key with the system's secure random source.
pub fn generate_ed25519(comment: &str) -> Result<GeneratedKey> {
    let mut rng = getrandom::rand_core::UnwrapErr(getrandom::SysRng);
    let mut key = PrivateKey::random(&mut rng, Algorithm::Ed25519)
        .map_err(|e| Error::Protocol(format!("key generation failed: {e}")))?;
    key.set_comment(comment);
    let private_openssh = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| Error::Protocol(format!("key encoding failed: {e}")))?
        .to_string();
    let public = key.public_key();
    Ok(GeneratedKey {
        private_openssh,
        public_openssh: public
            .to_openssh()
            .map_err(|e| Error::Protocol(format!("key encoding failed: {e}")))?,
        fingerprint: public.fingerprint(HashAlg::Sha256).to_string(),
    })
}

/// Outcome of importing a private key from text (file or clipboard).
#[derive(Clone)]
pub struct ImportedKey {
    /// Re-encoded as unencrypted OpenSSH; the app stores it protected.
    pub private_openssh: String,
    pub public_openssh: String,
    pub fingerprint: String,
    pub algorithm: String,
    pub comment: String,
    /// FIDO/U2F key (`sk-…`): the private part lives on the security key.
    pub hardware: bool,
}

// Debug without the private key, so it never ends up in a log.
impl std::fmt::Debug for GeneratedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneratedKey")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for ImportedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportedKey")
            .field("fingerprint", &self.fingerprint)
            .field("algorithm", &self.algorithm)
            .field("comment", &self.comment)
            .field("hardware", &self.hardware)
            .finish_non_exhaustive()
    }
}

/// Why an import failed, so the UI can ask for a passphrase.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("the key is protected by a passphrase")]
    PassphraseRequired,
    #[error("wrong passphrase")]
    WrongPassphrase,
    #[error("not a supported private key: {0}")]
    Invalid(String),
}

/// Import a private key in OpenSSH, PEM (PKCS#1/#8) or PuTTY format.
pub fn import_private_key(
    text: &str,
    passphrase: Option<&str>,
) -> Result<ImportedKey, ImportError> {
    let passphrase = passphrase.filter(|p| !p.is_empty());
    let key = match russh::keys::decode_secret_key(text.trim(), passphrase) {
        Ok(key) => key,
        Err(russh::keys::Error::KeyIsEncrypted) => {
            return Err(if passphrase.is_some() {
                ImportError::WrongPassphrase
            } else {
                ImportError::PassphraseRequired
            });
        }
        Err(e) if passphrase.is_some() && is_decryption_error(&e) => {
            return Err(ImportError::WrongPassphrase);
        }
        Err(e) => return Err(ImportError::Invalid(e.to_string())),
    };
    let public = key.public_key();
    let algorithm = key.algorithm();
    Ok(ImportedKey {
        private_openssh: key
            .to_openssh(LineEnding::LF)
            .map_err(|e| ImportError::Invalid(e.to_string()))?
            .to_string(),
        public_openssh: public
            .to_openssh()
            .map_err(|e| ImportError::Invalid(e.to_string()))?,
        fingerprint: public.fingerprint(HashAlg::Sha256).to_string(),
        algorithm: algorithm.as_str().to_owned(),
        comment: key.comment().as_str_lossy().to_owned(),
        hardware: is_hardware_key(&key),
    })
}

fn is_decryption_error(e: &russh::keys::Error) -> bool {
    matches!(
        e,
        russh::keys::Error::KeyIsCorrupt
            | russh::keys::Error::Unpad(_)
            | russh::keys::Error::SshKey(_)
    )
}

/// Encrypt an (unencrypted) OpenSSH private key with `passphrase` for export: the
/// standard OpenSSH format (bcrypt-pbkdf, aes256-ctr) that `ssh` and our import read.
pub fn encrypt_private_key(openssh: &str, passphrase: &str) -> Result<String> {
    if passphrase.is_empty() {
        return Err(Error::Protocol("passphrase must not be empty".into()));
    }
    let mut rng = getrandom::rand_core::UnwrapErr(getrandom::SysRng);
    Ok(parse_private_key(openssh)?
        .encrypt(&mut rng, passphrase)
        .map_err(|e| Error::Protocol(format!("key encryption failed: {e}")))?
        .to_openssh(LineEnding::LF)
        .map_err(|e| Error::Protocol(format!("key encoding failed: {e}")))?
        .to_string())
}

/// Parse an OpenSSH private key (unencrypted).
pub fn parse_private_key(openssh: &str) -> Result<PrivateKey> {
    PrivateKey::from_openssh(openssh)
        .map_err(|e| Error::Connect(format!("invalid private key: {e}")))
}

/// Parse an OpenSSH public key line (`ssh-ed25519 AAAA… [comment]`).
pub fn parse_public_key(openssh: &str) -> Result<PublicKey> {
    PublicKey::from_openssh(openssh.trim())
        .map_err(|e| Error::Protocol(format!("invalid public key: {e}")))
}

/// Host keys the user has trusted, keyed by `host:port`. The app persists the
/// entries and loads them back with [`TrustStore::trust`].
#[derive(Default)]
pub struct TrustStore {
    keys: Mutex<HashMap<String, Vec<PublicKey>>>,
}

fn host_id(host: &str, port: u16) -> String {
    format!("{}:{port}", host.to_ascii_lowercase())
}

impl TrustStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn trust(&self, host: &str, port: u16, key: PublicKey) {
        let mut keys = self.keys.lock().expect("trust store lock");
        let entry = keys.entry(host_id(host, port)).or_default();
        if !entry.contains(&key) {
            entry.push(key);
        }
    }

    /// Forget all keys of a host, e.g. after the user accepted a changed key.
    pub fn forget(&self, host: &str, port: u16) {
        self.keys
            .lock()
            .expect("trust store lock")
            .remove(&host_id(host, port));
    }
}

impl HostKeyVerifier for TrustStore {
    fn verify(&self, host: &str, port: u16, key: &PublicKey) -> HostKeyStatus {
        let keys = self.keys.lock().expect("trust store lock");
        match keys.get(&host_id(host, port)) {
            Some(known) if known.contains(key) => HostKeyStatus::Trusted,
            // A known host presenting any other key is a change, also with
            // another algorithm: that must not look like first contact.
            Some(known) if !known.is_empty() => HostKeyStatus::Changed,
            _ => HostKeyStatus::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_round_trips() {
        let key = generate_ed25519("test@mc").unwrap();
        assert!(key.public_openssh.starts_with("ssh-ed25519 "));
        assert!(key.public_openssh.ends_with(" test@mc"));
        assert!(key.fingerprint.starts_with("SHA256:"));
        let parsed = parse_private_key(&key.private_openssh).unwrap();
        assert_eq!(
            parsed.public_key().to_openssh().unwrap(),
            key.public_openssh
        );
    }

    #[test]
    fn exported_key_needs_the_passphrase() {
        let generated = generate_ed25519("export@mc").unwrap();
        let exported = encrypt_private_key(&generated.private_openssh, "lange-passphrase").unwrap();
        assert!(exported.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert!(matches!(
            import_private_key(&exported, None),
            Err(ImportError::PassphraseRequired)
        ));
        let back = import_private_key(&exported, Some("lange-passphrase")).unwrap();
        assert_eq!(back.fingerprint, generated.fingerprint);
        assert_eq!(back.comment, "export@mc");
        assert!(encrypt_private_key(&generated.private_openssh, "").is_err());
    }

    #[test]
    fn import_handles_plain_and_encrypted_keys() {
        let generated = generate_ed25519("import@mc").unwrap();
        let plain = import_private_key(&generated.private_openssh, None).unwrap();
        assert_eq!(plain.fingerprint, generated.fingerprint);
        assert_eq!(plain.algorithm, "ssh-ed25519");
        assert_eq!(plain.comment, "import@mc");
        assert!(!plain.hardware);

        let mut rng = getrandom::rand_core::UnwrapErr(getrandom::SysRng);
        let encrypted = parse_private_key(&generated.private_openssh)
            .unwrap()
            .encrypt(&mut rng, "geheim")
            .unwrap()
            .to_openssh(LineEnding::LF)
            .unwrap()
            .to_string();
        assert!(matches!(
            import_private_key(&encrypted, None),
            Err(ImportError::PassphraseRequired)
        ));
        assert!(matches!(
            import_private_key(&encrypted, Some("falsch")),
            Err(ImportError::WrongPassphrase)
        ));
        let decrypted = import_private_key(&encrypted, Some("geheim")).unwrap();
        assert_eq!(decrypted.fingerprint, generated.fingerprint);
        assert!(!decrypted.private_openssh.contains("aes256"));
        assert!(matches!(
            import_private_key("hello", None),
            Err(ImportError::Invalid(_))
        ));
    }

    #[test]
    fn trust_store_detects_unknown_trusted_and_changed() {
        let a = parse_public_key(&generate_ed25519("a").unwrap().public_openssh).unwrap();
        let b = parse_public_key(&generate_ed25519("b").unwrap().public_openssh).unwrap();
        let store = TrustStore::new();
        assert_eq!(store.verify("Host", 22, &a), HostKeyStatus::Unknown);
        store.trust("host", 22, a.clone());
        assert_eq!(store.verify("HOST", 22, &a), HostKeyStatus::Trusted);
        assert_eq!(store.verify("host", 22, &b), HostKeyStatus::Changed);
        assert_eq!(store.verify("host", 2222, &a), HostKeyStatus::Unknown);
    }

    #[test]
    fn other_algorithm_from_trusted_host_is_a_change() {
        let ed25519 = parse_public_key(&generate_ed25519("a").unwrap().public_openssh).unwrap();
        let ecdsa = parse_public_key(concat!(
            "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBIAipjHDNEGxCV/",
            "cBB3H5IOEVWW1s8tgoKnrSvzipRtJN2hH7sdvnKcT2tviQhvwUjOzExG/tQhth6m+ejqf5NE="
        ))
        .unwrap();
        let store = TrustStore::new();
        store.trust("host", 22, ed25519);
        assert_eq!(store.verify("host", 22, &ecdsa), HostKeyStatus::Changed);
    }

    #[test]
    fn debug_hides_private_keys() {
        let key = generate_ed25519("a").unwrap();
        assert!(!format!("{key:?}").contains("PRIVATE"));
        let imported = import_private_key(&key.private_openssh, None).unwrap();
        assert!(!format!("{imported:?}").contains("PRIVATE"));
    }
}
