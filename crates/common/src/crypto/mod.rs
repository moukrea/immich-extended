//! Symmetric encryption primitives.
//!
//! The current consumer is the per-user Immich API key, stored in
//! `immich_api_keys.ciphertext` and unsealed at engine-poll time. AES-256-GCM
//! is wrapped behind a typed [`master_key::MasterKey`] so callers cannot mix
//! up nonces or skip the integrity tag.

pub mod master_key;

pub use master_key::{MasterKey, MasterKeyError};
