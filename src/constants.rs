//! Shared constants. Rule #3: use CONST instead of hardcoding values.

// ---- env keys ----
pub const ENV_ZPL_IP: &str = "ZPL_IP";
pub const ENV_ZPL_PORT: &str = "ZPL_PORT";
pub const ENV_SERVER_HOST: &str = "SERVER_HOST";
pub const ENV_SERVER_PORT: &str = "SERVER_PORT";
pub const ENV_RUST_LOG: &str = "RUST_LOG";

// ---- defaults ----
pub const DEFAULT_ZPL_IP: &str = "127.0.0.1";
pub const DEFAULT_ZPL_PORT: u16 = 9100;
pub const DEFAULT_SERVER_HOST: &str = "127.0.0.1";
pub const DEFAULT_SERVER_PORT: u16 = 4331;
pub const DEFAULT_RUST_LOG: &str = "info";

// ---- routes ----
pub const ROUTE_PRINT: &str = "/print";
pub const ROUTE_HEALTH: &str = "/health";
pub const ROUTE_STATUS: &str = "/status";
pub const ROUTE_DOCS: &str = "/docs";
pub const ROUTE_OPENAPI_JSON: &str = "/docs/openapi.json";

// ---- limits / timeouts ----
pub const MAX_BODY_BYTES: usize = 1_048_576; // 1 MiB raw ZPL cap
pub const TCP_CONNECT_TIMEOUT_MS: u64 = 5_000;
pub const TCP_WRITE_TIMEOUT_MS: u64 = 10_000;

// ---- JSON payload keys (agnostic user input) ----
pub const JSON_KEY_ZPL: &str = "zpl";
pub const JSON_KEY_MESSAGE: &str = "message";
pub const JSON_KEY_DATA: &str = "data";
pub const JSON_KEY_PAYLOAD: &str = "payload";
pub const JSON_KEY_TEXT: &str = "text";

// ---- response / log messages ----
pub const MSG_STATUS_OK: &str = "ok";
pub const MSG_STATUS_SENT: &str = "sent";
pub const MSG_STATUS_DEGRADED: &str = "sent_with_warnings";
pub const MSG_STATUS_REJECTED: &str = "printer_error";
pub const MSG_STATUS_UNKNOWN: &str = "printer_status_unknown";
pub const MSG_HEALTH_OK: &str = "ok";
pub const MSG_SERVICE_NAME: &str = "zproxyrs";
pub const MSG_EMPTY_BODY: &str = "empty body: nothing to print";
pub const MSG_BODY_TOO_LARGE: &str = "body too large";
pub const MSG_PRINTER_ERROR: &str = "printer error";

// ---- log events (Rule #3: no hardcoded event strings) ----
pub const LOG_PRINT_REQ: &str = "print_request";
pub const LOG_PRINT_OK: &str = "print_sent";
pub const LOG_PRINT_ERR: &str = "print_failed";
pub const LOG_EXTRACT_ERR: &str = "extract_failed";
pub const LOG_HEALTH: &str = "health_check";
pub const LOG_TCP_CONNECT: &str = "tcp_connect";
pub const LOG_TCP_SENT: &str = "tcp_sent";
pub const LOG_TCP_ERR: &str = "tcp_error";
pub const LOG_STATUS_QUERY: &str = "status_query";
pub const LOG_STATUS_OK: &str = "status_ok";
pub const LOG_STATUS_ERR: &str = "status_error";
pub const PREVIEW_CHARS: usize = 120;

// ---- printer status probe ----
pub const STATUS_QUERY_CMD: &str = "~HS";
pub const STATUS_READ_TIMEOUT_MS: u64 = 4_000;
pub const STATUS_PORT_DEFAULT: u16 = 9100;

// ---- ZPL framing markers (used for validation hints) ----
pub const ZPL_START: &str = "^XA";
pub const ZPL_END: &str = "^XZ";
