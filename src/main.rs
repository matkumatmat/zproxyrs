//! zproxyrs: POST raw text -> TCP -> ZPL printer (+ ~HS verify + OpenAPI).
//! Wiring only; all logic lives in `lib.rs` modules.

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use zproxyrs::config::ConfigFactory;
use zproxyrs::constants::DEFAULT_RUST_LOG;
use zproxyrs::enums::{EnvKey, StrFactory};
use zproxyrs::routes::{AppState, RouterFactory};

#[tokio::main]
async fn main() {
    // Cheap dotenv: load `.env` if present, ignore errors (no extra dep).
    load_dotenv();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                std::env::var(StrFactory::of(EnvKey::RustLog))
                    .map(|_| {
                        tracing_subscriber::EnvFilter::new(
                            std::env::var(StrFactory::of(EnvKey::RustLog))
                                .unwrap_or_else(|_| DEFAULT_RUST_LOG.to_string()),
                        )
                    })
                    .unwrap_or_else(|_| DEFAULT_RUST_LOG.into())
            }),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    // Factory + trait (Rules #1): build config from environment loader.
    let cfg = ConfigFactory::from_env();
    let bind = format!("{}:{}", cfg.server_host, cfg.server_port);
    tracing::info!(
        printer = format!("{}:{}", cfg.printer_host, cfg.printer_port),
        bind,
        docs = "/docs",
        "starting zproxyrs"
    );

    let state = AppState::from_config(&cfg);
    // Factory (Rule #1): build router with Swagger docs.
    let app = RouterFactory::with_docs(state);

    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(%bind, error = %e, "bind failed");
            std::process::exit(1);
        }
    };

    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!(error = %e, "server error");
        std::process::exit(1);
    }
}

/// Minimal `.env` loader (KEY=VALUE, skips blanks/comments, no overwrite).
/// Keys are matched back to [`EnvKey`] enums (pattern matching, no literals).
fn load_dotenv() {
    let content = match std::fs::read_to_string(".env") {
        Ok(c) => c,
        Err(_) => return,
    };
    for line in content.lines() {
        let line = line.trim();
        match line {
            "" => continue,
            _ if line.starts_with('#') => continue,
            _ => match line.split_once('=') {
                Some((k, v)) => {
                    let (k, v) = (k.trim(), v.trim());
                    // Validate key via enum (unknown keys still loaded, but matched first).
                    let _known: Option<EnvKey> = EnvKey::from_key(k);
                    match (k.is_empty(), std::env::var_os(k)) {
                        (true, _) => continue,
                        (false, Some(_)) => continue, // real env wins
                        // SAFETY: single-threaded startup before server spawns.
                        (false, None) => unsafe { std::env::set_var(k, v) },
                    }
                }
                None => continue,
            },
        }
    }
}
