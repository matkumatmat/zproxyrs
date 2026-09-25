//! OpenAPI document (utoipa) — single source for `/api-docs/openapi.json` + `/docs`.
//! Schemas come from `routes` + `status` types via `ToSchema`.

use utoipa::OpenApi;

use crate::routes::{ErrorResponse, HealthResponse, PrintResponse, StatusResponse};
use crate::status::PrinterStatus;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "zpl-bridge",
        version = "0.1.0",
        description = "Agnostic ZPL bridge: POST raw text -> raw TCP :9100 -> Zebra printer, with ~HS status verification."
    ),
    paths(
        crate::routes::health_handler,
        crate::routes::status_handler,
        crate::routes::print_handler,
    ),
    components(
        schemas(
            PrintResponse,
            ErrorResponse,
            HealthResponse,
            StatusResponse,
            PrinterStatus,
        )
    ),
    tags(
        (name = "bridge", description = "Print + printer status")
    )
)]
pub struct ApiDoc;

/// Factory for the spec object (keeps OpenAPI construction in one place).
pub struct DocsFactory;

impl DocsFactory {
    pub fn spec() -> utoipa::openapi::OpenApi {
        ApiDoc::openapi()
    }

    pub fn json() -> String {
        match Self::spec().to_json() {
            Ok(j) => j,
            Err(_) => String::from("{}"),
        }
    }
}
