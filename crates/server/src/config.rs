//! Server configuration loaded from environment variables.

use std::{env, net::SocketAddr, path::PathBuf, str::FromStr};

use thiserror::Error;

const DEFAULT_HTTP_BIND: &str = "0.0.0.0:8080";
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_DATA_DIR: &str = "./data";

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
}

#[derive(Debug, Clone)]
pub struct Config {
    pub http_bind: SocketAddr,
    pub log_level: String,
    pub data_dir: PathBuf,
}

impl Config {
    /// Load the configuration from process environment, applying defaults
    /// for any unset variable. Returns an error if a value is present but
    /// fails to parse.
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

        Ok(Self {
            http_bind,
            log_level,
            data_dir,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
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

    #[test]
    fn defaults_when_env_unset() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(&["HTTP_BIND", "LOG_LEVEL", "DATA_DIR"]);

        let cfg = Config::from_env().unwrap();
        assert_eq!(cfg.http_bind.to_string(), "0.0.0.0:8080");
        assert_eq!(cfg.log_level, "info");
        assert_eq!(cfg.data_dir, PathBuf::from("./data"));
    }

    #[test]
    fn overrides_from_env() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(&["HTTP_BIND", "LOG_LEVEL", "DATA_DIR"]);
        env::set_var("HTTP_BIND", "127.0.0.1:9090");
        env::set_var("LOG_LEVEL", "debug");
        env::set_var("DATA_DIR", "/tmp/iet");

        let cfg = Config::from_env().unwrap();
        assert_eq!(cfg.http_bind.to_string(), "127.0.0.1:9090");
        assert_eq!(cfg.log_level, "debug");
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/iet"));
    }

    #[test]
    fn rejects_malformed_http_bind() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::new(&["HTTP_BIND", "LOG_LEVEL", "DATA_DIR"]);
        env::set_var("HTTP_BIND", "not-a-socket");

        let err = Config::from_env().unwrap_err();
        assert!(matches!(err, ConfigError::InvalidHttpBind { .. }));
    }
}
