//! AES-256-GCM "master key" — the single secret that wraps per-user Immich
//! API keys at rest. Loaded from `IMMICH_EXT_MASTER_KEY` at server startup; if
//! the env var is missing or malformed the server refuses to boot (PRD §6 —
//! loud failure beats silent insecure default).
//!
//! The newtype hides the raw 32 bytes from `Debug` so the key cannot leak via
//! a stray `{:?}` print. Cloning is cheap (32 bytes on the stack) so the type
//! can sit inside `AppState` and be passed around freely.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::rngs::OsRng;
use rand::RngCore;
use std::fmt;
use thiserror::Error;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const MASTER_KEY_ENV: &str = "IMMICH_EXT_MASTER_KEY";

#[derive(Debug, Error)]
pub enum MasterKeyError {
    #[error("environment variable {0} is not set")]
    EnvMissing(&'static str),
    #[error("environment variable {var} is not valid hex: {source}")]
    EnvNotHex {
        var: &'static str,
        #[source]
        source: hex::FromHexError,
    },
    #[error(
        "environment variable {var} must hex-decode to exactly {expected} bytes, got {actual}"
    )]
    WrongLength {
        var: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("invalid nonce length: expected {expected}, got {actual}")]
    NonceLength { expected: usize, actual: usize },
    #[error("AES-256-GCM operation failed (key/nonce/ciphertext mismatch)")]
    Crypto,
}

/// 32-byte symmetric key suitable for AES-256-GCM.
#[derive(Clone)]
pub struct MasterKey([u8; KEY_LEN]);

impl MasterKey {
    /// Load from `IMMICH_EXT_MASTER_KEY`. The value must be 64 lowercase- or
    /// uppercase-hex characters (32 bytes once decoded).
    pub fn from_env() -> Result<Self, MasterKeyError> {
        let raw = std::env::var(MASTER_KEY_ENV)
            .map_err(|_| MasterKeyError::EnvMissing(MASTER_KEY_ENV))?;
        Self::from_hex(&raw).map_err(|e| match e {
            MasterKeyError::EnvNotHex { source, .. } => MasterKeyError::EnvNotHex {
                var: MASTER_KEY_ENV,
                source,
            },
            MasterKeyError::WrongLength { actual, .. } => MasterKeyError::WrongLength {
                var: MASTER_KEY_ENV,
                expected: KEY_LEN,
                actual,
            },
            other => other,
        })
    }

    fn from_hex(raw: &str) -> Result<Self, MasterKeyError> {
        let bytes = hex::decode(raw.trim()).map_err(|source| MasterKeyError::EnvNotHex {
            var: MASTER_KEY_ENV,
            source,
        })?;
        if bytes.len() != KEY_LEN {
            return Err(MasterKeyError::WrongLength {
                var: MASTER_KEY_ENV,
                expected: KEY_LEN,
                actual: bytes.len(),
            });
        }
        let mut key = [0u8; KEY_LEN];
        key.copy_from_slice(&bytes);
        Ok(Self(key))
    }

    /// Direct constructor. Production code paths should always go through
    /// [`from_env`] so the "loud failure on missing/malformed key" property
    /// (PRD §6) holds. This entry point exists for integration tests that
    /// need to decrypt the row after a route stored it, which requires the
    /// same byte sequence on both sides of the round-trip.
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Encrypt `plaintext`, returning `(nonce_12, ciphertext_including_auth_tag)`.
    /// A fresh 12-byte nonce is drawn from `OsRng` on every call — nonce reuse
    /// is catastrophic for GCM, so we never let the caller pick one.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), MasterKeyError> {
        let cipher = Aes256Gcm::new_from_slice(&self.0).map_err(|_| MasterKeyError::Crypto)?;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|_| MasterKeyError::Crypto)?;
        Ok((nonce_bytes.to_vec(), ciphertext))
    }

    /// Decrypt `ciphertext` using `nonce`. GCM's integrity tag means a tampered
    /// ciphertext (or mismatched nonce/key) surfaces as `MasterKeyError::Crypto`.
    pub fn decrypt(&self, nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, MasterKeyError> {
        if nonce.len() != NONCE_LEN {
            return Err(MasterKeyError::NonceLength {
                expected: NONCE_LEN,
                actual: nonce.len(),
            });
        }
        let cipher = Aes256Gcm::new_from_slice(&self.0).map_err(|_| MasterKeyError::Crypto)?;
        let nonce = Nonce::from_slice(nonce);
        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| MasterKeyError::Crypto)
    }
}

impl fmt::Debug for MasterKey {
    /// Custom debug to keep the raw key bytes out of any `{:?}` / `tracing`
    /// output, which is the whole point of this newtype.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MasterKey").field("len", &KEY_LEN).finish()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard;

    impl EnvGuard {
        fn new() -> Self {
            std::env::remove_var(MASTER_KEY_ENV);
            Self
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var(MASTER_KEY_ENV);
        }
    }

    fn deterministic_key() -> MasterKey {
        MasterKey::from_bytes([7u8; KEY_LEN])
    }

    #[test]
    fn encrypt_then_decrypt_round_trips() {
        let key = deterministic_key();
        let (nonce, ct) = key.encrypt(b"hello-immich-key").unwrap();
        let pt = key.decrypt(&nonce, &ct).unwrap();
        assert_eq!(pt, b"hello-immich-key");
    }

    #[test]
    fn each_encrypt_uses_a_fresh_nonce() {
        let key = deterministic_key();
        let (n1, _) = key.encrypt(b"same plaintext").unwrap();
        let (n2, _) = key.encrypt(b"same plaintext").unwrap();
        assert_ne!(n1, n2, "encrypt must draw a fresh nonce each call");
        assert_eq!(n1.len(), NONCE_LEN);
        assert_eq!(n2.len(), NONCE_LEN);
    }

    #[test]
    fn decrypt_with_wrong_nonce_fails() {
        let key = deterministic_key();
        let (_nonce, ct) = key.encrypt(b"secret").unwrap();
        let wrong_nonce = [0u8; NONCE_LEN];
        let err = key.decrypt(&wrong_nonce, &ct).unwrap_err();
        assert!(matches!(err, MasterKeyError::Crypto));
    }

    #[test]
    fn decrypt_with_wrong_nonce_length_is_typed() {
        let key = deterministic_key();
        let (_nonce, ct) = key.encrypt(b"secret").unwrap();
        let too_short = [0u8; NONCE_LEN - 1];
        let err = key.decrypt(&too_short, &ct).unwrap_err();
        assert!(
            matches!(err, MasterKeyError::NonceLength { expected, actual }
                if expected == NONCE_LEN && actual == NONCE_LEN - 1)
        );
    }

    #[test]
    fn decrypt_with_tampered_ciphertext_fails() {
        let key = deterministic_key();
        let (nonce, mut ct) = key.encrypt(b"secret").unwrap();
        // Flip a bit in the body — GCM's auth tag MUST catch it.
        ct[0] ^= 0x01;
        let err = key.decrypt(&nonce, &ct).unwrap_err();
        assert!(matches!(err, MasterKeyError::Crypto));
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let k1 = MasterKey::from_bytes([1u8; KEY_LEN]);
        let k2 = MasterKey::from_bytes([2u8; KEY_LEN]);
        let (nonce, ct) = k1.encrypt(b"secret").unwrap();
        let err = k2.decrypt(&nonce, &ct).unwrap_err();
        assert!(matches!(err, MasterKeyError::Crypto));
    }

    #[test]
    fn debug_does_not_leak_key_bytes() {
        let key = MasterKey::from_bytes([0xABu8; KEY_LEN]);
        let dbg = format!("{key:?}");
        assert!(
            !dbg.contains("AB"),
            "Debug must not include key bytes: {dbg}"
        );
        assert!(dbg.contains("len"));
    }

    #[test]
    fn from_env_accepts_64_hex_chars() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new();
        let hex_val = "0".repeat(64);
        std::env::set_var(MASTER_KEY_ENV, &hex_val);
        let key = MasterKey::from_env().unwrap();
        // Round-trip with the loaded key.
        let (nonce, ct) = key.encrypt(b"x").unwrap();
        assert_eq!(key.decrypt(&nonce, &ct).unwrap(), b"x");
    }

    #[test]
    fn from_env_rejects_missing() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new();
        let err = MasterKey::from_env().unwrap_err();
        assert!(matches!(err, MasterKeyError::EnvMissing(_)));
    }

    #[test]
    fn from_env_rejects_short_hex() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new();
        std::env::set_var(MASTER_KEY_ENV, "deadbeef");
        let err = MasterKey::from_env().unwrap_err();
        assert!(matches!(err, MasterKeyError::WrongLength { actual: 4, .. }));
    }

    #[test]
    fn from_env_rejects_non_hex() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new();
        std::env::set_var(MASTER_KEY_ENV, "zzz".repeat(22));
        let err = MasterKey::from_env().unwrap_err();
        assert!(matches!(err, MasterKeyError::EnvNotHex { .. }));
    }

    #[test]
    fn from_env_rejects_empty() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new();
        std::env::set_var(MASTER_KEY_ENV, "");
        let err = MasterKey::from_env().unwrap_err();
        assert!(matches!(err, MasterKeyError::WrongLength { actual: 0, .. }));
    }
}
