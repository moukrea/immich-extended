//! Shared types, utilities, and database access for immich-extended.

pub mod auth;
pub mod db;
pub mod users;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Returns the crate version at compile time.
pub fn version() -> &'static str {
    VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_cargo_pkg_version() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
        assert!(!version().is_empty());
    }
}
