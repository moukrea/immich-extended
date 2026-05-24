//! Argon2id password hashing and verification.
//!
//! Hashes are encoded as PHC strings (`$argon2id$v=19$m=...,t=...,p=...$salt$hash`)
//! which are self-describing — params, salt, and digest all live in one column,
//! so the schema stays a simple `password_hash TEXT`. Verification re-derives
//! params from the stored string, which lets us rotate Argon2 parameters in a
//! later iteration without touching existing rows.

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use password_hash::{Error as PhError, SaltString};
use rand::rngs::OsRng;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PasswordError {
    #[error("failed to hash password: {0}")]
    Hash(PhError),
    #[error("stored password hash is malformed: {0}")]
    MalformedHash(PhError),
}

/// Hash a plaintext password using Argon2id with default parameters and a
/// 16-byte random salt drawn from the OS CSPRNG.
///
/// The returned string is a self-describing PHC encoding suitable for storage
/// in the `local_credentials.password_hash` column.
pub fn hash_password(plain: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon = Argon2::default();
    let phc = argon
        .hash_password(plain.as_bytes(), &salt)
        .map_err(PasswordError::Hash)?;
    Ok(phc.to_string())
}

/// Verify a plaintext password against a stored PHC hash.
///
/// `Ok(true)`  → the password matches.
/// `Ok(false)` → the password does NOT match (caller maps to 401).
/// `Err(_)`    → the stored hash itself is malformed (caller maps to 500 —
///                this is a data-integrity bug, not a user-facing failure).
pub fn verify_password(plain: &str, encoded_hash: &str) -> Result<bool, PasswordError> {
    let parsed = PasswordHash::new(encoded_hash).map_err(PasswordError::MalformedHash)?;
    match Argon2::default().verify_password(plain.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(PhError::Password) => Ok(false),
        Err(other) => Err(PasswordError::MalformedHash(other)),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_succeeds() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash).unwrap());
    }

    #[test]
    fn hashing_same_plaintext_yields_distinct_hashes() {
        let a = hash_password("hunter2").unwrap();
        let b = hash_password("hunter2").unwrap();
        assert_ne!(a, b, "salt randomness should produce different hashes");
        // both still verify
        assert!(verify_password("hunter2", &a).unwrap());
        assert!(verify_password("hunter2", &b).unwrap());
    }

    #[test]
    fn wrong_password_returns_ok_false() {
        let hash = hash_password("hunter2").unwrap();
        let result = verify_password("hunter3", &hash).unwrap();
        assert!(!result, "wrong password must be Ok(false), never Err");
    }

    #[test]
    fn verifying_against_malformed_hash_errors() {
        let result = verify_password("anything", "not-a-phc-string");
        assert!(matches!(result, Err(PasswordError::MalformedHash(_))));
    }

    #[test]
    fn empty_password_round_trips() {
        let hash = hash_password("").unwrap();
        assert!(verify_password("", &hash).unwrap());
        assert!(!verify_password("nonempty", &hash).unwrap());
    }
}
