//! TCP transport to the ZPL printer.
//! Rule #1: factory pattern + generic function + trait (`PrinterSender`).
//! Rule #2: pattern matching on payload / connect / write outcomes.
//! Rule #3: every timeout / default lives in [`crate::constants`].

use std::fmt;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::constants::*;
use crate::status::{StatusFactory, StatusProbeError, StatusProber};

/// Errors produced while pushing raw bytes to the printer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrinterError {
    EmptyPayload,
    TooLarge { got: usize, max: usize },
    Connect(String),
    Write(String),
    Timeout,
}

impl fmt::Display for PrinterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPayload => write!(f, "{MSG_EMPTY_BODY}"),
            Self::TooLarge { got, max } => write!(f, "{MSG_BODY_TOO_LARGE}: {got} > {max}"),
            Self::Connect(addr) => write!(f, "connect failed: {addr}"),
            Self::Write(addr) => write!(f, "write failed: {addr}"),
            Self::Timeout => write!(f, "printer io timeout"),
        }
    }
}

impl std::error::Error for PrinterError {}

/// Trait for anything that can push raw ZPL bytes to a printer.
pub trait PrinterSender {
    /// Send raw bytes; returns bytes written on success.
    fn send_raw<'a>(
        &'a self,
        payload: &'a [u8],
    ) -> impl std::future::Future<Output = Result<usize, PrinterError>> + Send + 'a;

    /// Human readable `host:port` target.
    fn target(&self) -> String;
}

/// Raw-TCP ZPL printer (port 9100 style).
#[derive(Debug, Clone)]
pub struct TcpZplPrinter {
    pub host: String,
    pub port: u16,
}

impl TcpZplPrinter {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    fn addr(&self) -> String {
        socket_addr_of(&self.host, self.port)
    }
}

impl PrinterSender for TcpZplPrinter {
    async fn send_raw(&self, payload: &[u8]) -> Result<usize, PrinterError> {
        // Validate first via pattern matching.
        match validate_payload(payload) {
            Err(e) => {
                tracing::warn!(event = LOG_TCP_ERR, target = self.addr(), error = %e, "validate failed");
                return Err(e);
            }
            Ok(_) => {}
        }

        let addr = self.addr();
        let connect_dur = Duration::from_millis(TCP_CONNECT_TIMEOUT_MS);
        let write_dur = Duration::from_millis(TCP_WRITE_TIMEOUT_MS);

        tracing::info!(
            event = LOG_TCP_CONNECT,
            target = addr,
            bytes = payload.len(),
            "dialing printer"
        );
        // Connect with timeout; pattern match every outcome.
        let mut stream = match timeout(connect_dur, TcpStream::connect(&addr)).await {
            Err(_) => {
                tracing::error!(event = LOG_TCP_ERR, target = addr, "connect timeout");
                return Err(PrinterError::Timeout);
            }
            Ok(Err(e)) => {
                tracing::error!(event = LOG_TCP_ERR, target = addr, error = %e, "connect failed");
                return Err(PrinterError::Connect(addr));
            }
            Ok(Ok(s)) => s,
        };

        // Write-all with timeout; pattern match every outcome.
        match timeout(write_dur, stream.write_all(payload)).await {
            Err(_) => {
                tracing::error!(event = LOG_TCP_ERR, target = addr, "write timeout");
                Err(PrinterError::Timeout)
            }
            Ok(Err(e)) => {
                tracing::error!(event = LOG_TCP_ERR, target = addr, error = %e, "write failed");
                Err(PrinterError::Write(addr))
            }
            Ok(Ok(_)) => match stream.flush().await {
                Err(e) => {
                    tracing::error!(event = LOG_TCP_ERR, target = addr, error = %e, "flush failed");
                    Err(PrinterError::Write(addr))
                }
                Ok(_) => {
                    tracing::info!(
                        event = LOG_TCP_SENT,
                        target = addr,
                        bytes = payload.len(),
                        "bytes pushed to printer"
                    );
                    Ok(payload.len())
                }
            },
        }
    }

    fn target(&self) -> String {
        self.addr()
    }
}

impl StatusProber for TcpZplPrinter {
    /// Send `~HS` on a fresh connection and read the 3-line reply.
    /// NOTE: Zebra suppresses the reply on critical faults (ribbon out,
    /// head over-temp, rewinder full) — that surfaces as `Timeout`, which
    /// callers map to 504 / hardware-fault, not success.
    async fn query_status(&self) -> Result<crate::status::PrinterStatus, StatusProbeError> {
        let addr = self.addr();
        let connect_dur = Duration::from_millis(TCP_CONNECT_TIMEOUT_MS);
        let read_dur = Duration::from_millis(STATUS_READ_TIMEOUT_MS);

        tracing::info!(
            event = LOG_STATUS_QUERY,
            target = addr,
            "probing printer status"
        );
        let mut stream = match timeout(connect_dur, TcpStream::connect(&addr)).await {
            Err(_) => return Err(StatusProbeError::Timeout),
            Ok(Err(e)) => {
                tracing::error!(event = LOG_TCP_ERR, target = addr, error = %e, "status connect failed");
                return Err(StatusProbeError::Connect(addr));
            }
            Ok(Ok(s)) => s,
        };

        let cmd = format!("{STATUS_QUERY_CMD}\r\n");
        match timeout(read_dur, stream.write_all(cmd.as_bytes())).await {
            Err(_) => return Err(StatusProbeError::Timeout),
            Ok(Err(e)) => {
                tracing::error!(event = LOG_TCP_ERR, target = addr, error = %e, "status write failed");
                return Err(StatusProbeError::Write(addr));
            }
            Ok(Ok(_)) => {}
        }

        // Read until 3 non-empty lines or timeout. `~HS` has no unique
        // terminator: 3 lines each STX..ETX CRLF.
        let mut buf = Vec::with_capacity(512);
        let mut tmp = [0u8; 512];
        let read_fut = async {
            loop {
                match stream.read(&mut tmp).await {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        let text = String::from_utf8_lossy(&buf);
                        let lines = text
                            .lines()
                            .map(str::trim)
                            .filter(|l| !l.is_empty())
                            .count();
                        match lines >= 3 {
                            true => break,
                            false => continue,
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok::<_, std::io::Error>(())
        };
        match timeout(read_dur, read_fut).await {
            Err(_) => {
                tracing::error!(event = LOG_STATUS_ERR, target = addr, "status read timeout");
                return Err(StatusProbeError::Timeout);
            }
            Ok(Err(e)) => {
                tracing::error!(event = LOG_STATUS_ERR, target = addr, error = %e, "status read failed");
                return Err(StatusProbeError::Read(addr));
            }
            Ok(Ok(_)) => {}
        }

        let text = String::from_utf8_lossy(&buf).to_string();
        match StatusFactory::parse(&text) {
            Ok(st) => {
                tracing::info!(
                    event = LOG_STATUS_OK,
                    target = addr,
                    paper_out = st.paper_out,
                    paused = st.paused,
                    head_up = st.head_up,
                    ribbon_out = st.ribbon_out,
                    "status parsed"
                );
                Ok(st)
            }
            Err(e) => {
                tracing::error!(event = LOG_STATUS_ERR, target = addr, error = %e, preview = %text.chars().take(120).collect::<String>(), "status parse failed");
                Err(StatusProbeError::Parse(e))
            }
        }
    }
}

/// Which printer transport to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrinterKind {
    Tcp,
}

/// Factory for printer senders (Rule #1: factory pattern).
pub struct PrinterFactory;

impl PrinterFactory {
    /// Generic factory: build any `PrinterKind` into a concrete sender.
    /// Currently only TCP exists; the `match` keeps the door open for USB/etc.
    pub fn create(kind: PrinterKind, host: impl Into<String>, port: u16) -> TcpZplPrinter {
        match kind {
            PrinterKind::Tcp => TcpZplPrinter::new(host, port),
        }
    }

    /// Shortcut for the standard TCP ZPL printer.
    pub fn tcp(host: impl Into<String>, port: u16) -> TcpZplPrinter {
        Self::create(PrinterKind::Tcp, host, port)
    }
}

/// Generic send helper usable with ANY [`PrinterSender`] (Rule #1: generic fn).
/// Pattern matches on the result so callers get a uniform path.
pub async fn send_payload<P: PrinterSender>(
    printer: &P,
    payload: &[u8],
) -> Result<usize, PrinterError> {
    match printer.send_raw(payload).await {
        Ok(n) => Ok(n),
        Err(e) => Err(e),
    }
}

/// Validate payload size via pattern matching (Rule #2).
pub fn validate_payload(payload: &[u8]) -> Result<usize, PrinterError> {
    match payload {
        [] => Err(PrinterError::EmptyPayload),
        bytes if bytes.len() > MAX_BODY_BYTES => Err(PrinterError::TooLarge {
            got: bytes.len(),
            max: MAX_BODY_BYTES,
        }),
        bytes => Ok(bytes.len()),
    }
}

/// Resolve printer address from host/port with pattern matching.
pub fn socket_addr_of(host: &str, port: u16) -> String {
    match host.trim().is_empty() {
        true => format!("{DEFAULT_ZPL_IP}:{port}"),
        false => format!("{}:{}", host.trim(), port),
    }
}
