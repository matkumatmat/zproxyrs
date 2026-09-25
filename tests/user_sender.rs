//! Rust user-sender test (Rule #5): act as an agnostic user that only
//! knows `POST /print` with a raw text body — no printer knowledge.
//! Spins up a fake ZPL printer (print capture + `~HS` ready reply) +
//! the real bridge, then POSTs via raw TCP (stdlib-style HTTP, no client
//! crate), asserting end-to-end delivery + verified status.
//!
//! Run with: `cargo test --test user_sender`

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use zproxyrs::constants::*;
use zproxyrs::routes::{AppState, RouterFactory};

/// Ready-state `~HS` reply (all flags clear, tear-off mode).
pub const FAKE_HS_READY: &str = concat!(
    "\x02000,0,0,0000,000,0,0,0,000,0,0,0\x03\r\n",
    "\x02000,0,0,0,0,2,0,0,00000000,1,000\x03\r\n",
    "\x020000,0\x03\r\n",
);

/// Read exactly one HTTP response head + body (simple, test-only parser).
async fn raw_http_post(
    host: &str,
    port: u16,
    path: &str,
    content_type: &str,
    body: &[u8],
) -> (u16, Vec<u8>) {
    let addr = format!("{host}:{port}");
    let mut sock = TcpStream::connect(&addr).await.expect("bridge connect");
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    sock.write_all(req.as_bytes()).await.unwrap();
    sock.write_all(body).await.unwrap();
    let mut raw = Vec::new();
    sock.read_to_end(&mut raw).await.unwrap();

    // Split head/body, parse `HTTP/1.1 XXX ...`.
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("http response");
    let head = String::from_utf8_lossy(&raw[..sep]).to_string();
    let status: u16 = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, raw[sep + 4..].to_vec())
}

/// Fake printer: conn1 captures print bytes (EOF), conn2 answers `~HS`.
/// Returns captured print payload.
async fn fake_printer_two_phase(listener: TcpListener) -> Vec<u8> {
    // Phase 1: print bytes.
    let (mut print_sock, _) = listener.accept().await.unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), print_sock.read_to_end(&mut buf)).await;

    // Phase 2: `~HS` probe — read command, reply ready, close.
    let (mut hs_sock, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
        .await
        .expect("hs accept timeout")
        .unwrap();
    let mut cmd = vec![0u8; 64];
    let _ = tokio::time::timeout(Duration::from_secs(5), hs_sock.read(&mut cmd)).await;
    hs_sock.write_all(FAKE_HS_READY.as_bytes()).await.unwrap();
    hs_sock.shutdown().await.ok();
    buf
}

async fn spawn_bridge(printer_port: u16) -> (u16, tokio::task::JoinHandle<()>) {
    let bridge_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bridge_port = bridge_listener.local_addr().unwrap().port();
    let state = AppState {
        printer_host: "127.0.0.1".to_string(),
        printer_port,
    };
    let app = RouterFactory::create(state);
    let server = axum::serve(bridge_listener, app).with_graceful_shutdown(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
    });
    let h = tokio::spawn(async move { server.await.unwrap() });
    tokio::time::sleep(Duration::from_millis(200)).await;
    (bridge_port, h)
}

#[tokio::test]
async fn rust_user_can_post_raw_text_and_printer_receives_it() {
    let printer_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let printer_port = printer_listener.local_addr().unwrap().port();
    let printer_rx = tokio::spawn(async move { fake_printer_two_phase(printer_listener).await });

    let (bridge_port, server_h) = spawn_bridge(printer_port).await;

    // Agnostic user POST: raw ZPL as text/plain (knows nothing about TCP:9100).
    let zpl = b"^XA^FO50,50^ADN,36,20^FDHello-from-Rust^FS^XZ";
    let (status, body) =
        raw_http_post("127.0.0.1", bridge_port, ROUTE_PRINT, "text/plain", zpl).await;
    let text = String::from_utf8_lossy(&body).to_string();
    assert_eq!(status, 200, "body={text}");
    assert!(text.contains(MSG_STATUS_SENT), "body={text}");
    assert!(
        text.contains("printer_status"),
        "verified response must embed status, body={text}"
    );

    tokio::time::sleep(Duration::from_millis(300)).await;
    server_h.abort();
    let got = tokio::time::timeout(Duration::from_secs(5), printer_rx)
        .await
        .expect("printer timeout")
        .unwrap();
    assert_eq!(got, zpl);
}

#[tokio::test]
async fn rust_user_can_post_json_envelope() {
    let printer_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let printer_port = printer_listener.local_addr().unwrap().port();
    let printer_rx = tokio::spawn(async move { fake_printer_two_phase(printer_listener).await });

    let (bridge_port, server_h) = spawn_bridge(printer_port).await;

    // JSON envelope variant — still agnostic, bridge unwraps `zpl` key.
    let envelope = br#"{"zpl": "^XA^FO10,10^ADN,18,10^FDJSON^FS^XZ"}"#;
    let (status, body) = raw_http_post(
        "127.0.0.1",
        bridge_port,
        ROUTE_PRINT,
        "application/json",
        envelope,
    )
    .await;
    let text = String::from_utf8_lossy(&body).to_string();
    assert_eq!(status, 200, "body={text}");

    tokio::time::sleep(Duration::from_millis(300)).await;
    server_h.abort();
    let got = tokio::time::timeout(Duration::from_secs(5), printer_rx)
        .await
        .expect("printer timeout")
        .unwrap();
    assert_eq!(got, b"^XA^FO10,10^ADN,18,10^FDJSON^FS^XZ");
}

#[tokio::test]
async fn rust_user_can_skip_verify_for_fast_path() {
    // Single-phase fake: only the print connection, no ~HS expected.
    let printer_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let printer_port = printer_listener.local_addr().unwrap().port();
    let printer_rx = tokio::spawn(async move {
        let (mut sock, _) = printer_listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(10), sock.read_to_end(&mut buf)).await;
        buf
    });

    let (bridge_port, server_h) = spawn_bridge(printer_port).await;

    let zpl = b"^XA^FO1,1^ADN,18,10^FDFAST^FS^XZ";
    let path = format!("{ROUTE_PRINT}?verify=false");
    let (status, body) = raw_http_post("127.0.0.1", bridge_port, &path, "text/plain", zpl).await;
    let text = String::from_utf8_lossy(&body).to_string();
    assert_eq!(status, 200, "body={text}");
    assert!(text.contains(MSG_STATUS_SENT), "body={text}");

    tokio::time::sleep(Duration::from_millis(300)).await;
    server_h.abort();
    let got = tokio::time::timeout(Duration::from_secs(5), printer_rx)
        .await
        .expect("printer timeout")
        .unwrap();
    assert_eq!(got, zpl);
}
