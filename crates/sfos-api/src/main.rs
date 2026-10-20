mod api;
mod query;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::routing::get;
use axum::Router;
use sfos_store::Store;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing_subscriber::EnvFilter;

use crate::api::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL is required (e.g. postgres://user:pass@host/db)")?;
    let addr: SocketAddr = std::env::var("API_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()
        .context("invalid API_ADDR")?;
    let web_dir = PathBuf::from(std::env::var("WEB_DIR").unwrap_or_else(|_| "web/dist".into()));
    let tenant_slug = std::env::var("TENANT_SLUG").unwrap_or_else(|_| "default".into());
    let tenant_name = std::env::var("TENANT_NAME").unwrap_or_else(|_| "Default site".into());
    let db_pool_size: usize = std::env::var("DB_POOL_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let raw_retention_hours: i64 = std::env::var("RETENTION_HOURS")
        .ok()
        .map(|value| {
            value
                .parse()
                .with_context(|| format!("invalid RETENTION_HOURS={value:?}"))
        })
        .transpose()?
        .unwrap_or(6)
        .clamp(1, 168);

    let database_password = std::env::var("DATABASE_PASSWORD").ok();
    let store =
        Store::connect_with_password(&database_url, database_password.as_deref(), db_pool_size)?;
    store.migrate().await.context("database migration failed")?;
    store.ensure_tenant(&tenant_slug, &tenant_name).await?;

    if !web_dir.join("index.html").exists() {
        tracing::warn!(
            dir = %web_dir.display(),
            "dashboard assets not found; build the web app or set WEB_DIR"
        );
    }

    let state = AppState {
        store,
        default_slug: Some(tenant_slug),
        raw_retention_hours,
    };

    let index = web_dir.join("index.html");
    let static_files = ServeDir::new(&web_dir).fallback(ServeFile::new(&index));
    let app = Router::new()
        .route("/healthz", get(api::healthz))
        .route("/api/v1/meta", get(api::meta))
        .route("/api/v1/overview", get(api::overview))
        .route("/api/v1/top", get(api::top))
        .route("/api/v1/events", get(api::events))
        .fallback_service(static_files)
        .layer(CompressionLayer::new())
        .with_state(state);

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("cannot bind API on {addr}"))?;
    tracing::info!(%addr, web = %web_dir.display(), "dashboard API listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("api server failed")?;
    Ok(())
}
