//! Authentication primitives shared between the server binary and any future
//! workers/CLIs. Currently exposes password hashing; OIDC verification (M1-T5)
//! will land here too.

pub mod password;
