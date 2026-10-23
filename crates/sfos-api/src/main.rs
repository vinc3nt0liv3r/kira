mod api;
mod query;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::routing::{get, post, put};
use axum::Router;
use chrono::Utc;
use sfos_store::Store;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing_subscriber::EnvFilter;

use crate::api::{AppState, evaluate_alert_rules, generate_and_send_report};

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
    let statement_timeout_ms: u64 = std::env::var("API_STATEMENT_TIMEOUT_MS")
        .ok()
        .map(|value| {
            value
                .parse()
                .with_context(|| format!("invalid API_STATEMENT_TIMEOUT_MS={value:?}"))
        })
        .transpose()?
        .unwrap_or(10_000);
    anyhow::ensure!(
        (1..=120_000).contains(&statement_timeout_ms),
        "API_STATEMENT_TIMEOUT_MS must be between 1 and 120000"
    );

    let database_password = std::env::var("DATABASE_PASSWORD").ok();
    let retention_admin_token = std::env::var("RETENTION_ADMIN_TOKEN")
        .ok()
        .filter(|token| !token.is_empty())
        .map(|token| {
            anyhow::ensure!(
                (32..=256).contains(&token.len()),
                "RETENTION_ADMIN_TOKEN must be between 32 and 256 characters"
            );
            Ok::<_, anyhow::Error>(std::sync::Arc::<str>::from(token))
        })
        .transpose()?;
    let postgres_options = format!("-c statement_timeout={statement_timeout_ms}");
    let store = Store::connect_with_password_and_options(
        &database_url,
        database_password.as_deref(),
        db_pool_size,
        Some(&postgres_options),
    )?;
    store.migrate().await.context("database migration failed")?;
    store
        .initialize_global_retention(raw_retention_hours)
        .await?;
    store.ensure_tenant(&tenant_slug, &tenant_name).await?;
    let query_concurrency: usize = std::env::var("API_MAX_CONCURRENT_QUERIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| db_pool_size.saturating_sub(1).clamp(1, 2));
    let rollups_ready = store.rollups_ready().await?;
    if !rollups_ready {
        tracing::warn!(
            "historical rollups are not ready; dashboard aggregate requests may scan retained raw events"
        );
    }

    if !web_dir.join("index.html").exists() {
        tracing::warn!(
            dir = %web_dir.display(),
            "dashboard assets not found; build the web app or set WEB_DIR"
        );
    }

    let smtp_host = std::env::var("SMTP_HOST").ok();
    let smtp_port = std::env::var("SMTP_PORT")
        .ok()
        .and_then(|v| v.parse().ok());
    let smtp_username = std::env::var("SMTP_USERNAME").ok();
    let smtp_password = std::env::var("SMTP_PASSWORD").ok();
    let smtp_from = std::env::var("SMTP_FROM").ok();

    let state = AppState {
        store,
        default_slug: Some(tenant_slug),
        retention_admin_token,
        query_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(query_concurrency)),
        smtp_host,
        smtp_port,
        smtp_username,
        smtp_password,
        smtp_from,
    };

    // Background scheduler: every minute, check due report schedules and evaluate alert rules.
    let scheduler_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let now = Utc::now();

            // Check due report schedules
            match scheduler_state.store.get_due_report_schedules(now).await {
                Ok(schedules) => {
                    for schedule in schedules {
                        let state = scheduler_state.clone();
                        let schedule_id = schedule.id;
                        tokio::spawn(async move {
                            if let Err(e) = generate_and_send_report(&state, &schedule, now).await {
                                tracing::error!(schedule_id = %schedule_id, %e, "report generation failed");
                            }
                        });
                    }
                }
                Err(e) => tracing::error!(%e, "failed to fetch due report schedules"),
            }

            // Evaluate alert rules
            match scheduler_state.store.list_tenants().await {
                Ok(tenants) => {
                    for tenant in tenants {
                        let state = scheduler_state.clone();
                        let tenant_id = tenant.id;
                        tokio::spawn(async move {
                            if let Err(e) = evaluate_alert_rules(&state, tenant_id, now).await {
                                tracing::error!(tenant_id = %tenant_id, %e, "alert evaluation failed");
                            }
                        });
                    }
                }
                Err(e) => tracing::error!(%e, "failed to list tenants for alert evaluation"),
            }
        }
    });

    let index = web_dir.join("index.html");
    let static_files = ServeDir::new(&web_dir).fallback(ServeFile::new(&index));
    let app = Router::new()
        .route("/healthz", get(api::healthz))
        .route("/api/v1/meta", get(api::meta))
        .route("/api/v1/overview", get(api::overview))
        .route("/api/v1/top", get(api::top))
        .route("/api/v1/events", get(api::events))
        .route(
            "/api/v1/events/export",
            get(api::events_export),
        )
        .route(
            "/api/v1/reports/schedules",
            get(api::list_report_schedules).post(api::create_report_schedule),
        )
        .route(
            "/api/v1/reports/schedules/:id",
            get(api::list_report_schedules)
                .put(api::update_report_schedule)
                .delete(api::delete_report_schedule),
        )
        .route(
            "/api/v1/reports/schedules/:id/results",
            get(api::list_report_results),
        )
        .route(
            "/api/v1/alerts/rules",
            get(api::list_alert_rules).post(api::create_alert_rule),
        )
        .route(
            "/api/v1/alerts/rules/:id",
            put(api::update_alert_rule).delete(api::delete_alert_rule),
        )
        .route(
            "/api/v1/alerts/instances",
            get(api::list_alert_instances),
        )
        .route(
            "/api/v1/alerts/instances/:id/acknowledge",
            post(api::acknowledge_alert_instance),
        )
        .route(
            "/api/v1/retention",
            get(api::retention_preview).put(api::update_retention),
        )
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
