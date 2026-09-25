//! Canonical enums (pattern-matching first, CONST-backed, no literals).
//! Rule #1: factory + generic fn + trait. Rule #2: `match` everywhere.
//! Numeric/default CONSTs stay in [`crate::constants`]; these enums map to them.

use crate::constants::*;

/// Trait for every string-backed enum: one canonical `&'static str`.
pub trait EnumStr {
    fn as_str(&self) -> &'static str;
}

/// Global factory over ANY [`EnumStr`] (Rule #1: generic + factory).
pub struct StrFactory;

impl StrFactory {
    pub fn of<E: EnumStr>(e: E) -> &'static str {
        e.as_str()
    }

    pub fn owned<E: EnumStr>(e: E) -> String {
        e.as_str().to_string()
    }
}

// ---- env keys ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvKey {
    ZplIp,
    ZplPort,
    ServerHost,
    ServerPort,
    RustLog,
}

impl EnumStr for EnvKey {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ZplIp => ENV_ZPL_IP,
            Self::ZplPort => ENV_ZPL_PORT,
            Self::ServerHost => ENV_SERVER_HOST,
            Self::ServerPort => ENV_SERVER_PORT,
            Self::RustLog => ENV_RUST_LOG,
        }
    }
}

impl EnvKey {
    /// Parse `KEY=...` left-hand side back into an enum (pattern matching).
    pub fn from_key(key: &str) -> Option<Self> {
        match key.trim() {
            k if k == ENV_ZPL_IP => Some(Self::ZplIp),
            k if k == ENV_ZPL_PORT => Some(Self::ZplPort),
            k if k == ENV_SERVER_HOST => Some(Self::ServerHost),
            k if k == ENV_SERVER_PORT => Some(Self::ServerPort),
            k if k == ENV_RUST_LOG => Some(Self::RustLog),
            _ => None,
        }
    }
}

// ---- routes ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Route {
    Print,
    Health,
    Status,
    Docs,
    OpenApiJson,
}

impl EnumStr for Route {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Print => ROUTE_PRINT,
            Self::Health => ROUTE_HEALTH,
            Self::Status => ROUTE_STATUS,
            Self::Docs => ROUTE_DOCS,
            Self::OpenApiJson => ROUTE_OPENAPI_JSON,
        }
    }
}

impl Route {
    pub fn from_path(path: &str) -> Option<Self> {
        match path {
            p if p == ROUTE_PRINT => Some(Self::Print),
            p if p == ROUTE_HEALTH => Some(Self::Health),
            p if p == ROUTE_STATUS => Some(Self::Status),
            p if p == ROUTE_DOCS => Some(Self::Docs),
            p if p == ROUTE_OPENAPI_JSON => Some(Self::OpenApiJson),
            _ => None,
        }
    }
}

// ---- agnostic JSON input fields ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JsonField {
    Zpl,
    Message,
    Data,
    Payload,
    Text,
}

impl EnumStr for JsonField {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Zpl => JSON_KEY_ZPL,
            Self::Message => JSON_KEY_MESSAGE,
            Self::Data => JSON_KEY_DATA,
            Self::Payload => JSON_KEY_PAYLOAD,
            Self::Text => JSON_KEY_TEXT,
        }
    }
}

impl JsonField {
    pub fn all() -> [Self; 5] {
        [
            Self::Zpl,
            Self::Message,
            Self::Data,
            Self::Payload,
            Self::Text,
        ]
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            k if k == JSON_KEY_ZPL => Some(Self::Zpl),
            k if k == JSON_KEY_MESSAGE => Some(Self::Message),
            k if k == JSON_KEY_DATA => Some(Self::Data),
            k if k == JSON_KEY_PAYLOAD => Some(Self::Payload),
            k if k == JSON_KEY_TEXT => Some(Self::Text),
            _ => None,
        }
    }
}

// ---- log events ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogEvent {
    PrintRequest,
    PrintOk,
    PrintFailed,
    ExtractFailed,
    HealthCheck,
    TcpConnect,
    TcpSent,
    TcpError,
    StatusQuery,
    StatusOk,
    StatusError,
}

impl EnumStr for LogEvent {
    fn as_str(&self) -> &'static str {
        match self {
            Self::PrintRequest => LOG_PRINT_REQ,
            Self::PrintOk => LOG_PRINT_OK,
            Self::PrintFailed => LOG_PRINT_ERR,
            Self::ExtractFailed => LOG_EXTRACT_ERR,
            Self::HealthCheck => LOG_HEALTH,
            Self::TcpConnect => LOG_TCP_CONNECT,
            Self::TcpSent => LOG_TCP_SENT,
            Self::TcpError => LOG_TCP_ERR,
            Self::StatusQuery => LOG_STATUS_QUERY,
            Self::StatusOk => LOG_STATUS_OK,
            Self::StatusError => LOG_STATUS_ERR,
        }
    }
}

// ---- print outcome (HTTP mapping input) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintOutcome {
    /// Bytes on the wire + printer reports ready.
    Sent,
    /// Bytes on the wire but printer reports warnings (buffer full, label waiting).
    SentWithWarnings,
    /// Refused / hardware fault: caller must act (paper, ribbon, head, paused).
    Rejected,
    /// Bridge could not reach the printer at all.
    Unreachable,
    /// Status probe timed out (critical fault per Zebra docs silences ~HS).
    StatusUnknown,
}

impl PrintOutcome {
    /// Map outcome -> HTTP code via pattern matching (single source of truth).
    pub fn status_code(&self) -> axum::http::StatusCode {
        match self {
            Self::Sent => axum::http::StatusCode::OK,
            Self::SentWithWarnings => axum::http::StatusCode::OK,
            Self::Rejected => axum::http::StatusCode::CONFLICT,
            Self::Unreachable => axum::http::StatusCode::BAD_GATEWAY,
            Self::StatusUnknown => axum::http::StatusCode::GATEWAY_TIMEOUT,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sent => MSG_STATUS_SENT,
            Self::SentWithWarnings => MSG_STATUS_DEGRADED,
            Self::Rejected => MSG_STATUS_REJECTED,
            Self::Unreachable => MSG_PRINTER_ERROR,
            Self::StatusUnknown => MSG_STATUS_UNKNOWN,
        }
    }
}
