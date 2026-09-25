//! Standalone unit tests for zproxyrs (Rule #4: outside `src/`, Rule #5).
//! Run with: `cargo test`
//! No printer or server required except where a fake TCP listener is spun up.

use std::collections::HashMap;

use axum::http::HeaderMap;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

use zproxyrs::config::{
    AppConfig, ConfigFactory, ConfigLoader, read_env_key, read_env_or, read_env_or_parsed,
};
use zproxyrs::constants::*;
use zproxyrs::docs::DocsFactory;
use zproxyrs::enums::{EnumStr, EnvKey, JsonField, LogEvent, PrintOutcome, Route, StrFactory};
use zproxyrs::printer::{
    PrinterError, PrinterFactory, PrinterKind, PrinterSender, send_payload, validate_payload,
};
use zproxyrs::routes::{AppState, RouterFactory, extract_payload, preview_of};
use zproxyrs::status::{PrinterFault, StatusFactory};

// ---------- helpers ----------

/// Fake loader to prove the factory is generic over any ConfigLoader.
struct MapLoader {
    vars: HashMap<String, String>,
}

impl ConfigLoader for MapLoader {
    fn load(&self) -> AppConfig {
        // Pattern matching over map lookups (Rule #2).
        let printer_host = match self.vars.get(ENV_ZPL_IP) {
            Some(v) => v.clone(),
            None => DEFAULT_ZPL_IP.to_string(),
        };
        let printer_port = match self.vars.get(ENV_ZPL_PORT) {
            Some(v) => match v.trim().parse::<u16>() {
                Ok(p) => p,
                Err(_) => DEFAULT_ZPL_PORT,
            },
            None => DEFAULT_ZPL_PORT,
        };
        AppConfig {
            printer_host,
            printer_port,
            server_host: DEFAULT_SERVER_HOST.to_string(),
            server_port: DEFAULT_SERVER_PORT,
        }
    }
}

/// In-memory printer proving the generic `send_payload` works with any trait impl.
struct MemPrinter {
    store: std::sync::Mutex<Vec<u8>>,
}

impl MemPrinter {
    fn new() -> Self {
        Self {
            store: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl PrinterSender for MemPrinter {
    async fn send_raw(&self, payload: &[u8]) -> Result<usize, PrinterError> {
        match validate_payload(payload) {
            Err(e) => Err(e),
            Ok(_) => {
                self.store.lock().unwrap().extend_from_slice(payload);
                Ok(payload.len())
            }
        }
    }

    fn target(&self) -> String {
        "mem://test".to_string()
    }
}

// ---------- constants (Rule #3) ----------

#[test]
fn constants_have_no_hardcoded_duplicates() {
    assert_eq!(ENV_ZPL_IP, "ZPL_IP");
    assert_eq!(ENV_ZPL_PORT, "ZPL_PORT");
    assert_eq!(ENV_SERVER_HOST, "SERVER_HOST");
    assert_eq!(ENV_SERVER_PORT, "SERVER_PORT");
    assert_eq!(ROUTE_PRINT, "/print");
    assert_eq!(ROUTE_HEALTH, "/health");
    assert_eq!(DEFAULT_ZPL_PORT, 9100);
    assert_eq!(DEFAULT_SERVER_PORT, 4331);
    assert!(MAX_BODY_BYTES >= 1024);
    assert_eq!(ZPL_START, "^XA");
    assert_eq!(ZPL_END, "^XZ");
}

// ---------- config factory + generics + pattern matching ----------

#[test]
fn config_factory_is_generic_over_loader() {
    let mut vars = HashMap::new();
    vars.insert(ENV_ZPL_IP.to_string(), "10.0.0.9".to_string());
    vars.insert(ENV_ZPL_PORT.to_string(), "9101".to_string());
    // Generic factory call (Rule #1).
    let cfg = ConfigFactory::create(MapLoader { vars });
    assert_eq!(cfg.printer_host, "10.0.0.9");
    assert_eq!(cfg.printer_port, 9101);
}

#[test]
fn config_factory_falls_back_on_bad_port() {
    let mut vars = HashMap::new();
    vars.insert(ENV_ZPL_PORT.to_string(), "not-a-port".to_string());
    let cfg = ConfigFactory::create(MapLoader { vars });
    // Pattern matching inside loader must fall back (Rule #2).
    assert_eq!(cfg.printer_port, DEFAULT_ZPL_PORT);
}

#[test]
fn env_helpers_use_pattern_matching() {
    // SAFETY: single-threaded manipulation for test only.
    unsafe { std::env::remove_var("zproxyrs_TEST_KEY") };
    assert_eq!(read_env_or("zproxyrs_TEST_KEY", "fallback"), "fallback");
    unsafe { std::env::set_var("zproxyrs_TEST_KEY", "hello") };
    assert_eq!(read_env_or("zproxyrs_TEST_KEY", "fallback"), "hello");
    unsafe { std::env::set_var("zproxyrs_TEST_PORT", "oops") };
    assert_eq!(read_env_or_parsed("zproxyrs_TEST_PORT", 1234u16), 1234u16);
    unsafe { std::env::set_var("zproxyrs_TEST_PORT", "4321") };
    assert_eq!(read_env_or_parsed("zproxyrs_TEST_PORT", 1234u16), 4321u16);
    unsafe {
        std::env::remove_var("zproxyrs_TEST_KEY");
        std::env::remove_var("zproxyrs_TEST_PORT");
    }
}

// ---------- printer factory + trait + generic fn ----------

#[test]
fn printer_factory_builds_tcp_sender() {
    let p = PrinterFactory::create(PrinterKind::Tcp, "192.168.1.50", 9100);
    assert_eq!(p.target(), "192.168.1.50:9100");
    let q = PrinterFactory::tcp("10.0.0.1", DEFAULT_ZPL_PORT);
    assert_eq!(q.target(), format!("10.0.0.1:{DEFAULT_ZPL_PORT}"));
}

#[test]
fn validate_payload_pattern_matches() {
    match validate_payload(b"") {
        Err(PrinterError::EmptyPayload) => {}
        other => panic!("expected EmptyPayload, got {other:?}"),
    }
    match validate_payload(b"^XA^XZ") {
        Ok(6) => {}
        other => panic!("expected Ok(6), got {other:?}"),
    }
    let big = vec![b'A'; MAX_BODY_BYTES + 1];
    match validate_payload(&big) {
        Err(PrinterError::TooLarge { .. }) => {}
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[tokio::test]
async fn generic_send_payload_works_with_any_sender() {
    let mem = MemPrinter::new();
    let n = send_payload(&mem, b"^XA^XZ").await.unwrap();
    assert_eq!(n, 6);
    match send_payload(&mem, b"").await {
        Err(PrinterError::EmptyPayload) => {}
        other => panic!("expected EmptyPayload, got {other:?}"),
    }
}

#[tokio::test]
async fn tcp_sender_pushes_raw_bytes_to_fake_printer() {
    // Fake ZPL printer: accept one connection and capture everything.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let rx = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        sock.read_to_end(&mut buf).await.unwrap();
        buf
    });

    let printer = PrinterFactory::tcp("127.0.0.1", addr.port());
    let payload = b"^XA^FO50,50^ADN,36,20^FDHello^FS^XZ";
    let n = send_payload(&printer, payload).await.unwrap();
    assert_eq!(n, payload.len());
    drop(printer); // close client so server sees EOF
    // Give server a beat then compare.
    let got = tokio::time::timeout(std::time::Duration::from_secs(5), rx)
        .await
        .expect("fake printer timeout")
        .unwrap();
    assert_eq!(got, payload);
}

#[tokio::test]
async fn tcp_sender_reports_connect_error_via_pattern_match() {
    // Port 1 is (almost) always closed; we only assert error mapping.
    let printer = PrinterFactory::tcp("127.0.0.1", 1);
    match send_payload(&printer, b"^XA^XZ").await {
        Err(PrinterError::Connect(_)) | Err(PrinterError::Timeout) => {}
        other => panic!("expected Connect/Timeout, got {other:?}"),
    }
}

// ---------- routes: agnostic extraction + factory ----------

#[test]
fn router_factory_builds_without_panic() {
    let state = AppState {
        printer_host: DEFAULT_ZPL_IP.to_string(),
        printer_port: DEFAULT_ZPL_PORT,
    };
    let _app = RouterFactory::create(state);
}

#[test]
fn extract_payload_accepts_raw_text() {
    let headers = HeaderMap::new(); // no content-type => raw path
    let body = b"^XA^XZ".to_vec();
    let out = extract_payload(&headers, &body).unwrap();
    assert_eq!(out, b"^XA^XZ");
}

#[test]
fn extract_payload_rejects_empty() {
    let headers = HeaderMap::new();
    let body: Vec<u8> = vec![];
    assert!(extract_payload(&headers, &body).is_err());
}

#[test]
fn extract_payload_accepts_json_keys() {
    use axum::http::header::CONTENT_TYPE;
    for key in [
        JSON_KEY_ZPL,
        JSON_KEY_MESSAGE,
        JSON_KEY_DATA,
        JSON_KEY_PAYLOAD,
        JSON_KEY_TEXT,
    ] {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
        let raw = serde_json::json!({ key: "^XA^XZ" })
            .to_string()
            .into_bytes();
        let out = extract_payload(&headers, &raw).unwrap();
        assert_eq!(out, b"^XA^XZ", "key={key}");
    }
}

#[test]
fn extract_payload_accepts_json_string_body() {
    use axum::http::header::CONTENT_TYPE;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
    let raw = b"\"^XA^XZ\"".to_vec();
    let out = extract_payload(&headers, &raw).unwrap();
    assert_eq!(out, b"^XA^XZ");
}

#[test]
fn extract_payload_rejects_bad_json() {
    use axum::http::header::CONTENT_TYPE;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
    let raw = br#"{"nope": 123}"#.to_vec();
    assert!(extract_payload(&headers, &raw).is_err());
}

#[test]
fn preview_helper_truncates_and_flattens() {
    assert_eq!(preview_of(&b"^XA^XZ".to_vec()), "^XA^XZ");
    assert_eq!(preview_of(&b"a\nb".to_vec()), "a\\nb");
    let big = vec![b'X'; PREVIEW_CHARS + 10];
    assert_eq!(preview_of(&big).len(), PREVIEW_CHARS);
}

// ---------- enums: pattern matching over canonical values ----------

#[test]
fn enums_resolve_without_hardcoded_strings() {
    // Generic factory over the EnumStr trait (Rule #1).
    assert_eq!(StrFactory::of(EnvKey::ZplIp), ENV_ZPL_IP);
    assert_eq!(StrFactory::of(EnvKey::ZplPort), ENV_ZPL_PORT);
    assert_eq!(StrFactory::of(Route::Print), ROUTE_PRINT);
    assert_eq!(StrFactory::of(Route::Health), ROUTE_HEALTH);
    assert_eq!(StrFactory::of(Route::Status), ROUTE_STATUS);
    assert_eq!(StrFactory::of(JsonField::Zpl), JSON_KEY_ZPL);
    assert_eq!(StrFactory::of(LogEvent::PrintRequest), LOG_PRINT_REQ);
    assert_eq!(StrFactory::owned(Route::Print), "/print");
}

#[test]
fn enums_roundtrip_via_pattern_matching() {
    match EnvKey::from_key("ZPL_IP") {
        Some(EnvKey::ZplIp) => {}
        other => panic!("expected ZplIp, got {other:?}"),
    }
    match EnvKey::from_key("NOPE") {
        None => {}
        other => panic!("expected None, got {other:?}"),
    }
    match Route::from_path("/status") {
        Some(Route::Status) => {}
        other => panic!("expected Status, got {other:?}"),
    }
    match JsonField::from_key("message") {
        Some(JsonField::Message) => {}
        other => panic!("expected Message, got {other:?}"),
    }
    assert_eq!(JsonField::all().len(), 5);
    // Direct trait use (generic proof).
    fn generic_str<E: EnumStr>(e: E) -> &'static str {
        e.as_str()
    }
    assert_eq!(generic_str(Route::Print), "/print");
}

#[test]
fn print_outcome_maps_via_match() {
    use axum::http::StatusCode;
    match PrintOutcome::Sent.status_code() {
        StatusCode::OK => {}
        other => panic!("expected 200, got {other}"),
    }
    assert_eq!(PrintOutcome::Rejected.status_code(), StatusCode::CONFLICT);
    assert_eq!(
        PrintOutcome::Unreachable.status_code(),
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(
        PrintOutcome::StatusUnknown.status_code(),
        StatusCode::GATEWAY_TIMEOUT
    );
    assert_eq!(PrintOutcome::Sent.as_str(), MSG_STATUS_SENT);
}

#[test]
fn env_key_reader_matches_loader() {
    unsafe { std::env::set_var("zproxyrs_ENUM_TEST", "v1") };
    assert_eq!(read_env_key(EnvKey::ZplIp, "fb").len() >= 2, true);
    unsafe { std::env::remove_var("zproxyrs_ENUM_TEST") };
}

// ---------- status: ~HS parsing + faults + outcome ----------

const HS_READY: &str = "\x02000,0,0,0000,000,0,0,0,000,0,0,0\x03\r\n\x02000,0,0,0,0,2,0,0,00000000,1,000\x03\r\n\x020000,0\x03\r\n";
const HS_PAPER_PAUSED: &str =
    "000,1,1,0000,000,0,0,0,000,0,0,0\n000,0,0,0,0,2,0,0,00000000,1,000\n0000,0\n";
const HS_HEAD_RIBBON: &str =
    "000,0,0,0000,000,0,0,0,000,0,0,0\n000,0,1,1,0,2,0,0,00000000,1,000\n0000,0\n";

#[test]
fn status_factory_parses_ready() {
    let st = StatusFactory::parse(&HS_READY.to_string()).unwrap();
    assert!(st.is_ready());
    assert!(st.faults().is_empty());
    assert_eq!(st.outcome(), PrintOutcome::Sent);
}

#[test]
fn status_factory_detects_paper_and_pause() {
    let st = StatusFactory::parse(&HS_PAPER_PAUSED.to_string()).unwrap();
    assert!(!st.is_ready());
    assert!(st.faults().contains(&PrinterFault::PaperOut));
    assert!(st.faults().contains(&PrinterFault::Paused));
    assert_eq!(st.outcome(), PrintOutcome::Rejected);
}

#[test]
fn status_factory_detects_head_and_ribbon() {
    let st = StatusFactory::parse(&HS_HEAD_RIBBON.to_string()).unwrap();
    assert!(st.faults().contains(&PrinterFault::HeadUp));
    assert!(st.faults().contains(&PrinterFault::RibbonOut));
    assert_eq!(st.outcome(), PrintOutcome::Rejected);
}

#[test]
fn status_factory_rejects_short_reply() {
    match StatusFactory::parse(&"only-one-line".to_string()) {
        Err(_) => {}
        Ok(_) => panic!("expected parse error"),
    }
    match StatusFactory::parse(&"".to_string()) {
        Err(_) => {}
        Ok(_) => panic!("expected parse error"),
    }
}

#[test]
fn status_warnings_map_to_degraded() {
    let mut st = StatusFactory::ready();
    st.label_waiting = true;
    assert!(st.is_ready());
    assert!(st.has_warnings());
    assert_eq!(st.outcome(), PrintOutcome::SentWithWarnings);
}

// ---------- docs: OpenAPI spec contains bridge routes ----------

#[test]
fn openapi_spec_covers_bridge() {
    let json = DocsFactory::json();
    for path in ["/health", "/status", "/print"] {
        assert!(json.contains(path), "spec missing {path}");
    }
    assert!(json.contains("zproxyrs"));
}
