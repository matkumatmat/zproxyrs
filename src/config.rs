//! App + printer-target configuration.
//! Rule #1: factory pattern + generic function + trait.
//! Rule #2: pattern matching on every env lookup / parse (via [`EnvKey`] enums).

use std::str::FromStr;

use crate::constants::*;
use crate::enums::{EnumStr, EnvKey, StrFactory};

/// Runtime configuration for the bridge server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub printer_host: String,
    pub printer_port: u16,
    pub server_host: String,
    pub server_port: u16,
}

/// Trait for anything that can load an [`AppConfig`].
/// Lets us swap env / file / mock loaders without touching callers.
pub trait ConfigLoader {
    fn load(&self) -> AppConfig;
}

/// Loads configuration from process environment with CONST fallbacks.
#[derive(Debug, Default, Clone, Copy)]
pub struct EnvConfigLoader;

/// Factory that builds configs from any [`ConfigLoader`].
/// Generic over the loader so tests can inject fakes.
pub struct ConfigFactory;

impl ConfigFactory {
    /// Generic factory constructor (Rule #1).
    pub fn create<L: ConfigLoader>(loader: L) -> AppConfig {
        loader.load()
    }

    /// Convenience: build from the real environment.
    pub fn from_env() -> AppConfig {
        Self::create(EnvConfigLoader)
    }
}

impl ConfigLoader for EnvConfigLoader {
    fn load(&self) -> AppConfig {
        // Enum-driven lookups (no string literals at call sites).
        AppConfig {
            printer_host: read_env_key(EnvKey::ZplIp, DEFAULT_ZPL_IP),
            printer_port: read_env_key_parsed(EnvKey::ZplPort, DEFAULT_ZPL_PORT),
            server_host: read_env_key(EnvKey::ServerHost, DEFAULT_SERVER_HOST),
            server_port: read_env_key_parsed(EnvKey::ServerPort, DEFAULT_SERVER_PORT),
        }
    }
}

/// Enum-backed env reader (preferred): `match` on presence/emptiness.
pub fn read_env_key(key: EnvKey, fallback: &str) -> String {
    match std::env::var(StrFactory::of(key)) {
        Ok(v) if !v.trim().is_empty() => v,
        Ok(_) => fallback.to_string(),
        Err(_) => fallback.to_string(),
    }
}

/// Enum-backed env reader + parser (preferred).
pub fn read_env_key_parsed<T>(key: EnvKey, fallback: T) -> T
where
    T: FromStr,
{
    match std::env::var(StrFactory::of(key)) {
        Ok(raw) => match raw.trim().parse::<T>() {
            Ok(parsed) => parsed,
            Err(_) => fallback,
        },
        Err(_) => fallback,
    }
}

/// Legacy string-key reader (kept for compat; prefer [`read_env_key`]).
pub fn read_env_or(key: &str, fallback: &str) -> String {
    match EnvKey::from_key(key) {
        Some(e) => read_env_key(e, fallback),
        None => match std::env::var(key) {
            Ok(v) if !v.trim().is_empty() => v,
            Ok(_) => fallback.to_string(),
            Err(_) => fallback.to_string(),
        },
    }
}

/// Legacy string-key reader + parser (kept for compat).
pub fn read_env_or_parsed<T>(key: &str, fallback: T) -> T
where
    T: FromStr,
{
    match EnvKey::from_key(key) {
        Some(e) => read_env_key_parsed(e, fallback),
        None => match std::env::var(key) {
            Ok(raw) => match raw.trim().parse::<T>() {
                Ok(parsed) => parsed,
                Err(_) => fallback,
            },
            Err(_) => fallback,
        },
    }
}

/// Prove [`EnumStr`] is wired (generic over the trait).
pub fn env_key_str<E: EnumStr>(key: E) -> &'static str {
    key.as_str()
}

/// Build a `host:port` address via pattern matching (no hardcoding).
pub fn socket_addr_of(host: &str, port: u16) -> String {
    match host.trim().is_empty() {
        true => format!("{DEFAULT_ZPL_IP}:{port}"),
        false => format!("{host}:{port}"),
    }
}
