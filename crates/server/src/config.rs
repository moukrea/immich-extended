//! Server configuration loaded from environment variables.

use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
};

use common::crypto::{MasterKey, MasterKeyError};
use thiserror::Error;

const DEFAULT_HTTP_BIND: &str = "0.0.0.0:8080";
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_DATA_DIR: &str = "./data";
const DEFAULT_DB_FILENAME: &str = "immich-extended.sqlite";
const DEFAULT_SESSION_COOKIE_NAME: &str = "__Host-iext_session";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid HTTP_BIND {value:?}: {source}")]
    InvalidHttpBind {
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },
    #[error("invalid DATA_DIR {value:?}: empty path")]
    InvalidDataDir { value: String },
    #[error("invalid DATABASE_URL: empty")]
    EmptyDatabaseUrl,
    #[error("master key: {0}")]
    MasterKey(#[from] MasterKeyError),
    #[error(
        "partial OIDC config: when OIDC_ISSUER_URL is set, OIDC_CLIENT_ID, \
         OIDC_CLIENT_SECRET, and OIDC_REDIRECT_URL must all be set too \
         (missing: {missing:?})"
    )]
    PartialOidcConfig { missing: Vec<&'static str> },
}

/// Cookie-session knobs derived from `SESSION_COOKIE_NAME` / `SESSION_COOKIE_SECURE`.
///
/// Defaults match production-over-TLS expectations: name `__Host-iext_session`
/// (browser enforces `Secure` + `Path=/` + no Domain) and `secure=true`. Local
/// plain-HTTP dev should set both: `SESSION_COOKIE_NAME=iext_session_dev` and
/// `SESSION_COOKIE_SECURE=false` — otherwise the cookie is silently stripped
/// by the browser before the next request comes in.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub cookie_name: String,
    pub cookie_secure: bool,
}

/// OIDC settings discovered from env when `OIDC_ISSUER_URL` is present. When
/// absent the whole block is `None` and the server skips OIDC route mounting
/// (single warn-level log at startup). PRD §8 allows local + OIDC to coexist;
/// the absence of OIDC must never break a local-only deployment.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub http_bind: SocketAddr,
    pub log_level: String,
    pub data_dir: PathBuf,
    pub database_url: String,
    pub session: SessionConfig,
    pub master_key: MasterKey,
    pub oidc: Option<OidcConfig>,
}

impl Config {
    /// Load the configuration from process environment, applying defaults
    /// for any unset variable. Returns an error if a value is present but
    /// fails to parse.
    ///
    /// `IMMICH_EXT_MASTER_KEY` is mandatory and has no default — per PRD §6
    /// the server refuses to boot if it's missing or malformed.
    pub fn from_env() -> Result<Self, ConfigError> {
        let http_bind_raw = env::var("HTTP_BIND").unwrap_or_else(|_| DEFAULT_HTTP_BIND.to_string());
        let http_bind = SocketAddr::from_str(&http_bind_raw).map_err(|source| {
            ConfigError::InvalidHttpBind {
                value: http_bind_raw.clone(),
                source,
            }
        })?;

        let log_level = env::var("LOG_LEVEL").unwrap_or_else(|_| DEFAULT_LOG_LEVEL.to_string());

        let data_dir_raw = env::var("DATA_DIR").unwrap_or_else(|_| DEFAULT_DATA_DIR.to_string());
        if data_dir_raw.is_empty() {
            return Err(ConfigError::InvalidDataDir {
                value: data_dir_raw,
            });
        }
        let data_dir = PathBuf::from(&data_dir_raw);

        let database_url = match env::var("DATABASE_URL") {
            Ok(v) => {
                if v.is_empty() {
                    return Err(ConfigError::EmptyDatabaseUrl);
                }
                v
            }
            Err(_) => default_database_url(&data_dir),
        };

        let session = SessionConfig::from_env();
        let master_key = MasterKey::from_env()?;
        let oidc = OidcConfig::from_env()?;

        Ok(Self {
            http_bind,
            log_level,
            data_dir,
            database_url,
            session,
            master_key,
            oidc,
        })
    }
}

impl OidcConfig {
    /// Read OIDC env vars. Returns `Ok(None)` if `OIDC_ISSUER_URL` is absent
    /// (OIDC disabled). Returns `Err(PartialOidcConfig)` if the issuer URL is
    /// set but any of the other three variables is missing — partial config
    /// almost always indicates a deployment mistake, and silently disabling
    /// OIDC in that case would be misleading.
    pub fn from_env() -> Result<Option<Self>, ConfigError> {
        let issuer_url = match env::var("OIDC_ISSUER_URL").ok().filter(|s| !s.is_empty()) {
            Some(v) => v,
            None => return Ok(None),
        };

        let client_id = env::var("OIDC_CLIENT_ID").ok().filter(|s| !s.is_empty());
        let client_secret = env::var("OIDC_CLIENT_SECRET")
            .ok()
            .filter(|s| !s.is_empty());
        let redirect_url = env::var("OIDC_REDIRECT_URL").ok().filter(|s| !s.is_empty());

        let mut missing = Vec::new();
        if client_id.is_none() {
            missing.push("OIDC_CLIENT_ID");
        }
        if client_secret.is_none() {
            missing.push("OIDC_CLIENT_SECRET");
        }
        if redirect_url.is_none() {
            missing.push("OIDC_REDIRECT_URL");
        }
        if !missing.is_empty() {
            return Err(ConfigError::PartialOidcConfig { missing });
        }

        Ok(Some(Self {
            issuer_url,
            // unwrap-safe: missing list is empty so all three Options are Some.
            client_id: client_id.unwrap_or_default(),
            client_secret: client_secret.unwrap_or_default(),
            redirect_url: redirect_url.unwrap_or_default(),
        }))
    }
}

impl SessionConfig {
    /// Load session cookie configuration from env. Defaults to production-safe
    /// values; tests and local plain-HTTP dev override via env.
    pub fn from_env() -> Self {
        let cookie_name = env::var("SESSION_COOKIE_NAME")
            .unwrap_or_else(|_| DEFAULT_SESSION_COOKIE_NAME.to_string());
        let cookie_secure = env::var("SESSION_COOKIE_SECURE")
            .ok()
            .map(|s| !matches!(s.to_ascii_lowercase().as_str(), "false" | "0" | "no"))
            .unwrap_or(true);
        Self {
            cookie_name,
            cookie_secure,
        }
    }
}

fn default_database_url(data_dir: &Path) -> String {
    let db_path = data_dir.join(DEFAULT_DB_FILENAME);
    format!("sqlite://{}?mode=rwc", db_path.display())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // env mutations are process-global; serialize the tests in this module.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        keys: Vec<&'static str>,
    }

    impl EnvGuard {
        fn new(keys: &[&'static str]) -> Self {
            for k in keys {
                env::remove_var(k);
            }
            Self {
                keys: keys.to_vec(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for k in &self.keys {
                env::remove_var(k);
            }
        }
    }

    const ALL_KEYS: &[&str] = &[
        "HTTP_BIND",
        "LOG_LEVEL",
        "DATA_DIR",
        "DATABASE_URL",
        "SESSION_COOKIE_NAME",
        "SESSION_COOKIE_SECURE",
        "IMMICH_EXT_MASTER_KEY",
        "OIDC_ISSUER_URL",
        "OIDC_CLIENT_ID",
        "OIDC_CLIENT_SECRET",
        "OIDC_REDIRECT_URL",
    ];

    /// 64 hex chars = 32 bytes — the format `MasterKey::from_env` accepts.
    const TEST_MASTER_KEY: &str =
        "0000000000000000000000000000000000000000000000000000000000000000";

    fn with_master_key() {
        env::set_var("IMMICH_EXT_MASTER_KEY", TEST_MASTER_KEY);
    }

    #[test]
    fn defaults_when_env_unset() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(ALL_KEYS);
        with_master_key();

        let cfg = Config::from_env().unwrap();
        assert_eq!(cfg.http_bind.to_string(), "0.0.0.0:8080");
        assert_eq!(cfg.log_level, "info");
        assert_eq!(cfg.data_dir, PathBuf::from("./data"));
        assert_eq!(
            cfg.database_url,
            "sqlite://./data/immich-extended.sqlite?mode=rwc"
        );
        assert_eq!(cfg.session.cookie_name, "__Host-iext_session");
        assert!(cfg.session.cookie_secure);
    }

    #[test]
    fn session_overrides_from_env() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(ALL_KEYS);
        with_master_key();
        env::set_var("SESSION_COOKIE_NAME", "iext_session_dev");
        env::set_var("SESSION_COOKIE_SECURE", "false");

        let cfg = Config::from_env().unwrap();
        assert_eq!(cfg.session.cookie_name, "iext_session_dev");
        assert!(!cfg.session.cookie_secure);
    }

    #[test]
    fn overrides_from_env() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(ALL_KEYS);
        with_master_key();
        env::set_var("HTTP_BIND", "127.0.0.1:9090");
        env::set_var("LOG_LEVEL", "debug");
        env::set_var("DATA_DIR", "/tmp/iet");
        env::set_var("DATABASE_URL", "sqlite::memory:");

        let cfg = Config::from_env().unwrap();
        assert_eq!(cfg.http_bind.to_string(), "127.0.0.1:9090");
        assert_eq!(cfg.log_level, "debug");
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/iet"));
        assert_eq!(cfg.database_url, "sqlite::memory:");
    }

    #[test]
    fn database_url_default_follows_data_dir() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(ALL_KEYS);
        with_master_key();
        env::set_var("DATA_DIR", "/var/lib/iet");

        let cfg = Config::from_env().unwrap();
        assert_eq!(
            cfg.database_url,
            "sqlite:///var/lib/iet/immich-extended.sqlite?mode=rwc"
        );
    }

    #[test]
    fn rejects_malformed_http_bind() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(ALL_KEYS);
        with_master_key();
        env::set_var("HTTP_BIND", "not-a-socket");

        let err = Config::from_env().unwrap_err();
        assert!(matches!(err, ConfigError::InvalidHttpBind { .. }));
    }

    #[test]
    fn rejects_empty_database_url() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(ALL_KEYS);
        with_master_key();
        env::set_var("DATABASE_URL", "");

        let err = Config::from_env().unwrap_err();
        assert!(matches!(err, ConfigError::EmptyDatabaseUrl));
    }

    #[test]
    fn rejects_missing_master_key() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(ALL_KEYS);
        // Deliberately do NOT call with_master_key — the env var is unset.

        let err = Config::from_env().unwrap_err();
        assert!(matches!(err, ConfigError::MasterKey(_)));
    }

    #[test]
    fn oidc_disabled_when_issuer_unset() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(ALL_KEYS);
        with_master_key();

        let cfg = Config::from_env().unwrap();
        assert!(cfg.oidc.is_none(), "OIDC must be disabled with no issuer");
    }

    #[test]
    fn oidc_enabled_with_full_config() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(ALL_KEYS);
        with_master_key();
        env::set_var("OIDC_ISSUER_URL", "https://issuer.example.test");
        env::set_var("OIDC_CLIENT_ID", "ie-client");
        env::set_var("OIDC_CLIENT_SECRET", "shh");
        env::set_var(
            "OIDC_REDIRECT_URL",
            "https://ext.example.test/api/v1/auth/oidc/callback",
        );

        let cfg = Config::from_env().unwrap();
        let oidc = cfg.oidc.expect("OIDC must be enabled with full config");
        assert_eq!(oidc.issuer_url, "https://issuer.example.test");
        assert_eq!(oidc.client_id, "ie-client");
        assert_eq!(oidc.client_secret, "shh");
        assert_eq!(
            oidc.redirect_url,
            "https://ext.example.test/api/v1/auth/oidc/callback"
        );
    }

    #[test]
    fn oidc_partial_config_is_error() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(ALL_KEYS);
        with_master_key();
        env::set_var("OIDC_ISSUER_URL", "https://issuer.example.test");
        env::set_var("OIDC_CLIENT_ID", "ie-client");
        // Deliberately omit OIDC_CLIENT_SECRET and OIDC_REDIRECT_URL.

        let err = Config::from_env().unwrap_err();
        match err {
            ConfigError::PartialOidcConfig { missing } => {
                assert!(missing.contains(&"OIDC_CLIENT_SECRET"));
                assert!(missing.contains(&"OIDC_REDIRECT_URL"));
                assert!(!missing.contains(&"OIDC_CLIENT_ID"));
            }
            other => panic!("expected PartialOidcConfig, got {other:?}"),
        }
    }

    #[test]
    fn oidc_empty_string_treated_as_unset() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(ALL_KEYS);
        with_master_key();
        env::set_var("OIDC_ISSUER_URL", "");

        let cfg = Config::from_env().unwrap();
        assert!(
            cfg.oidc.is_none(),
            "empty OIDC_ISSUER_URL must disable OIDC, not error"
        );
    }
}
