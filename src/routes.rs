//! HTTP layer: agnostic `POST /print` -> raw TCP -> ZPL printer + `~HS` verify.
//! Rule #1: factory (`RouterFactory`) + generic helpers + traits.
//! Rule #2: pattern matching via [`Route`]/[`JsonField`]/[`LogEvent`]/[`PrintOutcome`] enums.
//! Rule #6: users just POST a message; bridge forwards raw bytes and reports
//! printer truth (no more silent 200 — see `GET /status` + verified `POST`).

use axum::{
    Router,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, OpenApi, ToSchema};

use crate::config::AppConfig;
use crate::constants::*;
use crate::enums::{EnumStr, JsonField, LogEvent, PrintOutcome, Route, StrFactory};
use crate::printer::{PrinterError, PrinterFactory, PrinterKind, PrinterSender, send_payload};
use crate::status::{PrinterStatus, StatusProbeError, StatusProber};

// Re-export for docs macro path resolution.
pub use crate::status::PrinterStatus as PrinterStatusSchema;

/// Shared state cloned into every handler.
#[derive(Debug, Clone)]
pub struct AppState {
    pub printer_host: String,
    pub printer_port: u16,
}

impl AppState {
    pub fn from_config(cfg: &AppConfig) -> Self {
        Self {
            printer_host: cfg.printer_host.clone(),
            printer_port: cfg.printer_port,
        }
    }

    pub fn target(&self) -> String {
        match self.printer_host.trim().is_empty() {
            true => format!("{DEFAULT_ZPL_IP}:{}", self.printer_port),
            false => format!("{}:{}", self.printer_host, self.printer_port),
        }
    }
}

/// Factory that builds the axum router (Rule #1).
pub struct RouterFactory;

impl RouterFactory {
    pub fn create(state: AppState) -> Router {
        Router::new()
            .route(StrFactory::of(Route::Health), get(health_handler))
            .route(StrFactory::of(Route::Status), get(status_handler))
            .route(StrFactory::of(Route::Print), post(print_handler))
            .with_state(state)
    }

    /// Same routes + Swagger UI (`/docs`) + OpenAPI JSON.
    pub fn with_docs(state: AppState) -> Router {
        let (router, api) =
            utoipa_axum::router::OpenApiRouter::with_openapi(crate::docs::ApiDoc::openapi())
                .routes(utoipa_axum::routes!(health_handler))
                .routes(utoipa_axum::routes!(status_handler))
                .routes(utoipa_axum::routes!(print_handler))
                .split_for_parts();
        let _ = api;
        router.with_state(state).merge(
            utoipa_swagger_ui::SwaggerUi::new(StrFactory::of(Route::Docs)).url(
                StrFactory::of(Route::OpenApiJson),
                crate::docs::DocsFactory::spec(),
            ),
        )
    }
}

/// Query flags for `POST /print`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct VerifyQuery {
    /// `false` skips the post-print `~HS` probe (fast, legacy: 200 = bytes written).
    /// Default `true`: response includes parsed printer status + truthful code.
    #[param(required = false, example = true)]
    pub verify: Option<bool>,
}

/// Success body for `POST /print` (verified).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PrintResponse {
    #[schema(example = "sent")]
    pub status: String,
    #[schema(example = 40)]
    pub bytes: usize,
    #[schema(example = "192.168.19.5:9100")]
    pub printer: String,
    pub printer_status: Option<PrinterStatus>,
    pub warnings: Vec<String>,
}

/// Error body for `POST /print` / `GET /status`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ErrorResponse {
    #[schema(example = "printer_error")]
    pub status: String,
    #[schema(example = "connect failed: 192.168.19.5:9100")]
    pub error: String,
    pub printer: Option<String>,
    pub printer_status: Option<PrinterStatus>,
}

/// Health body for `GET /health`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    #[schema(example = "ok")]
    pub status: String,
    #[schema(example = "zpl-bridge")]
    pub service: String,
}

/// Body for `GET /status` (pure `~HS` probe, no printing).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StatusResponse {
    #[schema(example = "192.168.19.5:9100")]
    pub printer: String,
    #[schema(example = true)]
    pub reachable: bool,
    #[schema(example = true)]
    pub ready: bool,
    pub faults: Vec<String>,
    pub status: Option<PrinterStatus>,
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "bridge",
    responses((status = 200, description = "Bridge is up", body = HealthResponse))
)]
pub async fn health_handler() -> impl IntoResponse {
    tracing::info!(
        event = StrFactory::of(LogEvent::HealthCheck),
        "health_check"
    );
    Json(HealthResponse {
        status: MSG_HEALTH_OK.to_string(),
        service: MSG_SERVICE_NAME.to_string(),
    })
}

#[utoipa::path(
    get,
    path = "/status",
    tag = "bridge",
    responses(
        (status = 200, description = "Probe answered (inspect ready/faults)", body = StatusResponse),
        (status = 502, description = "Printer unreachable / probe failed", body = ErrorResponse),
        (status = 504, description = "Probe timed out (critical fault silences ~HS)", body = ErrorResponse),
    )
)]
pub async fn status_handler(State(state): State<AppState>) -> impl IntoResponse {
    let printer = PrinterFactory::create(
        PrinterKind::Tcp,
        state.printer_host.clone(),
        state.printer_port,
    );
    let target = printer.target();
    tracing::info!(
        event = StrFactory::of(LogEvent::StatusQuery),
        printer = target,
        "status probe"
    );

    match printer.query_status().await {
        Ok(st) => {
            let faults: Vec<String> = st.faults().iter().map(|f| f.as_str().to_string()).collect();
            tracing::info!(
                event = StrFactory::of(LogEvent::StatusOk),
                printer = target,
                ready = st.is_ready(),
                faults = ?faults,
                "status ok"
            );
            (
                StatusCode::OK,
                Json(serde_json::json!(StatusResponse {
                    printer: target,
                    reachable: true,
                    ready: st.is_ready(),
                    faults,
                    status: Some(st),
                })),
            )
                .into_response()
        }
        Err(e) => {
            let (code, msg) = map_probe_error(&e);
            tracing::error!(
                event = StrFactory::of(LogEvent::StatusError),
                printer = target,
                error = msg,
                "status failed"
            );
            (
                code,
                Json(ErrorResponse {
                    status: MSG_PRINTER_ERROR.to_string(),
                    error: msg,
                    printer: Some(target),
                    printer_status: None,
                }),
            )
                .into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/print",
    tag = "bridge",
    params(VerifyQuery),
    request_body(
        content = String,
        description = "Raw ZPL as text/plain (or JSON envelope: zpl|message|data|payload|text)",
        content_type = "text/plain",
    ),
    responses(
        (status = 200, description = "Bytes written + printer reports ready (or warnings listed)", body = PrintResponse),
        (status = 400, description = "Empty body / bad JSON envelope", body = ErrorResponse),
        (status = 409, description = "Sent but printer reports fault (paper/ribbon/head/paused)", body = ErrorResponse),
        (status = 413, description = "Body exceeds 1 MiB", body = ErrorResponse),
        (status = 502, description = "Printer unreachable / write failed", body = ErrorResponse),
        (status = 504, description = "~HS silence (critical fault) or io timeout", body = ErrorResponse),
    )
)]
pub async fn print_handler(
    State(state): State<AppState>,
    Query(q): Query<VerifyQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let verify = match q.verify {
        Some(false) => false,
        _ => true,
    };
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let target = state.target();

    tracing::info!(
        event = StrFactory::of(LogEvent::PrintRequest),
        content_type = content_type,
        body_len = body.len(),
        printer = target,
        verify = verify,
        preview = preview_of(&body),
        "incoming print"
    );

    // Generic extraction helper (Rule #1) + pattern matching (Rule #2).
    let payload: Vec<u8> = match extract_payload(&headers, &body) {
        Ok(v) => v,
        Err((code, msg)) => {
            tracing::warn!(
                event = StrFactory::of(LogEvent::ExtractFailed),
                content_type = content_type,
                body_len = body.len(),
                error = msg,
                "extract failed"
            );
            return (
                code,
                Json(ErrorResponse {
                    status: MSG_PRINTER_ERROR.to_string(),
                    error: msg,
                    printer: Some(target),
                    printer_status: None,
                }),
            )
                .into_response();
        }
    };

    let printer = PrinterFactory::create(
        PrinterKind::Tcp,
        state.printer_host.clone(),
        state.printer_port,
    );
    let target = printer.target();

    // 1) Push bytes (fire-and-forget channel).
    let bytes = match send_payload(&printer, &payload).await {
        Ok(n) => n,
        Err(e) => {
            let (code, msg) = map_printer_error(&e);
            let outcome = match &e {
                PrinterError::Timeout => PrintOutcome::StatusUnknown,
                _ => PrintOutcome::Unreachable,
            };
            tracing::error!(
                event = StrFactory::of(LogEvent::PrintFailed),
                bytes = payload.len(),
                printer = target,
                outcome = outcome.as_str(),
                error = msg,
                "write failed"
            );
            return (
                code,
                Json(ErrorResponse {
                    status: outcome.as_str().to_string(),
                    error: msg,
                    printer: Some(target),
                    printer_status: None,
                }),
            )
                .into_response();
        }
    };

    // 2) Fast path: caller opted out of verification.
    if !verify {
        tracing::info!(
            event = StrFactory::of(LogEvent::PrintOk),
            bytes = bytes,
            printer = target,
            verified = false,
            "print sent (unverified)"
        );
        return (
            StatusCode::OK,
            Json(serde_json::json!(PrintResponse {
                status: PrintOutcome::Sent.as_str().to_string(),
                bytes,
                printer: target,
                printer_status: None,
                warnings: vec![],
            })),
        )
            .into_response();
    }

    // 3) Verified path: `~HS` probe tells the truth.
    match printer.query_status().await {
        Ok(st) => {
            let outcome = st.outcome();
            let faults: Vec<String> = st.faults().iter().map(|f| f.as_str().to_string()).collect();
            match outcome {
                PrintOutcome::Sent => {
                    tracing::info!(
                        event = StrFactory::of(LogEvent::PrintOk),
                        bytes = bytes,
                        printer = target,
                        preview = preview_of(&payload),
                        "print sent + ready"
                    );
                    (
                        outcome.status_code(),
                        Json(serde_json::json!(PrintResponse {
                            status: outcome.as_str().to_string(),
                            bytes,
                            printer: target,
                            printer_status: Some(st),
                            warnings: vec![],
                        })),
                    )
                        .into_response()
                }
                PrintOutcome::SentWithWarnings => {
                    tracing::info!(
                        event = StrFactory::of(LogEvent::PrintOk),
                        bytes = bytes,
                        printer = target,
                        warnings = ?faults,
                        "print sent with warnings"
                    );
                    (
                        outcome.status_code(),
                        Json(serde_json::json!(PrintResponse {
                            status: outcome.as_str().to_string(),
                            bytes,
                            printer: target,
                            printer_status: Some(st),
                            warnings: faults,
                        })),
                    )
                        .into_response()
                }
                PrintOutcome::Rejected
                | PrintOutcome::Unreachable
                | PrintOutcome::StatusUnknown => {
                    tracing::error!(
                        event = StrFactory::of(LogEvent::PrintFailed),
                        bytes = bytes,
                        printer = target,
                        outcome = outcome.as_str(),
                        faults = ?faults,
                        "printer reports fault"
                    );
                    (
                        outcome.status_code(),
                        Json(ErrorResponse {
                            status: outcome.as_str().to_string(),
                            error: format!("printer reports fault: {}", faults.join(", ")),
                            printer: Some(target),
                            printer_status: Some(st),
                        }),
                    )
                        .into_response()
                }
            }
        }
        Err(e) => {
            // ~HS silence after a successful write == critical fault per Zebra
            // docs (ribbon-out / over-temp / rewinder-full suppress reply).
            let (code, msg) = map_probe_error(&e);
            tracing::error!(
                event = StrFactory::of(LogEvent::PrintFailed),
                bytes = bytes,
                printer = target,
                error = msg,
                "status probe failed after write"
            );
            (
                code,
                Json(ErrorResponse {
                    status: PrintOutcome::StatusUnknown.as_str().to_string(),
                    error: format!("bytes written ({bytes}) but {msg}"),
                    printer: Some(target),
                    printer_status: None,
                }),
            )
                .into_response()
        }
    }
}

/// Generic payload extractor: works for any byte buffer (Rule #1: generic fn).
/// Pattern matches on content-type, JSON shape, and emptiness (Rule #2).
pub fn extract_payload<T: AsRef<[u8]>>(
    headers: &HeaderMap,
    body: &T,
) -> Result<Vec<u8>, (StatusCode, String)> {
    let raw = body.as_ref();

    match raw {
        [] => Err((StatusCode::BAD_REQUEST, MSG_EMPTY_BODY.to_string())),
        _ if raw.len() > MAX_BODY_BYTES => Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("{MSG_BODY_TOO_LARGE}: {} > {MAX_BODY_BYTES}", raw.len()),
        )),
        _ => {
            let content_type = headers
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_lowercase();

            match content_type.contains("json") {
                true => match parse_json_payload(raw) {
                    Some(bytes) => match bytes.is_empty() {
                        true => Err((StatusCode::BAD_REQUEST, MSG_EMPTY_BODY.to_string())),
                        false => Ok(bytes),
                    },
                    None => Err((
                        StatusCode::BAD_REQUEST,
                        format!(
                            "invalid json: expected one of {{{}, {}, {}, {}, {}}} as string",
                            JsonField::Zpl.as_str(),
                            JsonField::Message.as_str(),
                            JsonField::Data.as_str(),
                            JsonField::Payload.as_str(),
                            JsonField::Text.as_str()
                        ),
                    )),
                },
                false => Ok(raw.to_vec()),
            }
        }
    }
}

/// Try known JSON keys in order; pattern match each lookup (enum-driven).
fn parse_json_payload(raw: &[u8]) -> Option<Vec<u8>> {
    let v: serde_json::Value = serde_json::from_slice(raw).ok()?;

    // Direct string body: `"^XA...^XZ"`.
    match &v {
        serde_json::Value::String(s) => return Some(s.as_bytes().to_vec()),
        _ => {}
    }

    for field in JsonField::all() {
        match v.get(field.as_str()) {
            Some(serde_json::Value::String(s)) => return Some(s.as_bytes().to_vec()),
            _ => continue,
        }
    }
    None
}

/// Map printer errors to HTTP codes via pattern matching.
fn map_printer_error(e: &PrinterError) -> (StatusCode, String) {
    match e {
        PrinterError::EmptyPayload => (StatusCode::BAD_REQUEST, e.to_string()),
        PrinterError::TooLarge { .. } => (StatusCode::PAYLOAD_TOO_LARGE, e.to_string()),
        PrinterError::Timeout => (StatusCode::GATEWAY_TIMEOUT, e.to_string()),
        PrinterError::Connect(_) | PrinterError::Write(_) => {
            (StatusCode::BAD_GATEWAY, e.to_string())
        }
    }
}

/// Map `~HS` probe errors: silence => 504 (critical fault per Zebra docs).
fn map_probe_error(e: &StatusProbeError) -> (StatusCode, String) {
    match e {
        StatusProbeError::Timeout => (
            StatusCode::GATEWAY_TIMEOUT,
            "printer silent on ~HS (critical fault: ribbon-out / over-temp / rewinder-full?)"
                .to_string(),
        ),
        StatusProbeError::Connect(_) => (StatusCode::BAD_GATEWAY, e.to_string()),
        StatusProbeError::Write(_) => (StatusCode::BAD_GATEWAY, e.to_string()),
        StatusProbeError::Read(_) => (StatusCode::BAD_GATEWAY, e.to_string()),
        StatusProbeError::Parse(_) => (StatusCode::BAD_GATEWAY, e.to_string()),
    }
}

/// Generic preview helper: first N chars, lossy, single-line (Rule #1).
pub fn preview_of<T: AsRef<[u8]>>(buf: &T) -> String {
    let raw = buf.as_ref();
    let end = raw.len().min(PREVIEW_CHARS);
    match raw.get(..end) {
        Some(slice) => String::from_utf8_lossy(slice).replace('\n', "\\n"),
        None => String::new(),
    }
}
