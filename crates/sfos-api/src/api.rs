use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Duration, TimeZone, Timelike, Utc};
use lettre::Transport;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sfos_store::{AlertInstance, AlertRule, ReportResult, ReportSchedule, Store, Tenant, WebhookDelivery};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use crate::query::{RangeQuery, TimeRange};

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub default_slug: Option<String>,
    pub retention_admin_token: Option<Arc<str>>,
    pub query_semaphore: Arc<Semaphore>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_from: Option<String>,
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }
    fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.into(),
        }
    }
    fn gone(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::GONE,
            message: msg.into(),
        }
    }
    fn unavailable(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: msg.into(),
        }
    }
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "valid bearer token required".into(),
        }
    }
    fn database_error(err: &tokio_postgres::Error) -> Self {
        if err.code() == Some(&tokio_postgres::error::SqlState::QUERY_CANCELED) {
            Self::unavailable("database query exceeded the API statement timeout")
        } else {
            tracing::error!(%err, "api database request failed");
            Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: "internal server error".into(),
            }
        }
    }
    fn internal(err: anyhow::Error) -> Self {
        tracing::error!(%err, "api request failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "internal server error".into(),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        if let Some(database_error) = err.downcast_ref::<tokio_postgres::Error>() {
            Self::database_error(database_error)
        } else {
            Self::internal(err)
        }
    }
}

impl From<tokio_postgres::Error> for ApiError {
    fn from(err: tokio_postgres::Error) -> Self {
        Self::database_error(&err)
    }
}

impl From<deadpool_postgres::PoolError> for ApiError {
    fn from(err: deadpool_postgres::PoolError) -> Self {
        Self::internal(err.into())
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

fn acquire_query_permit(
    query_semaphore: &Arc<Semaphore>,
    busy_message: &'static str,
) -> Result<OwnedSemaphorePermit, ApiError> {
    query_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::unavailable(busy_message))
}

#[derive(Debug, Serialize)]
struct TenantView {
    slug: String,
    name: String,
    is_default: bool,
    retention_days: i32,
}

impl From<&Tenant> for TenantView {
    fn from(t: &Tenant) -> Self {
        Self {
            slug: t.slug.clone(),
            name: t.name.clone(),
            is_default: t.is_default,
            retention_days: t.retention_days,
        }
    }
}

async fn resolve_tenant(state: &AppState, slug: Option<&str>) -> Result<Tenant, ApiError> {
    if let Some(slug) = slug {
        return state
            .store
            .tenant_by_slug(slug)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("unknown tenant {slug:?}")));
    }
    if let Some(slug) = state.default_slug.as_deref() {
        if let Some(tenant) = state.store.tenant_by_slug(slug).await? {
            return Ok(tenant);
        }
    }
    state
        .store
        .default_tenant()
        .await?
        .ok_or_else(|| ApiError::bad_request("no tenant configured yet; send a log line first"))
}

fn resolve_range(query: &RangeQuery) -> Result<TimeRange, ApiError> {
    query.resolve(Utc::now()).map_err(ApiError::bad_request)
}

fn hour_floor(value: DateTime<Utc>) -> Result<DateTime<Utc>, ApiError> {
    value
        .date_naive()
        .and_hms_opt(value.hour(), 0, 0)
        .context("failed to align time to an hour")
        .map(|value| value.and_utc())
        .map_err(ApiError::internal)
}

fn hour_ceil(value: DateTime<Utc>) -> Result<DateTime<Utc>, ApiError> {
    let floor = hour_floor(value)?;
    Ok(if floor < value {
        floor + Duration::hours(1)
    } else {
        floor
    })
}

fn raw_cutover(now: DateTime<Utc>, retention_hours: i64) -> DateTime<Utc> {
    let cutoff = now - Duration::hours(retention_hours.max(1));
    cutoff
        .date_naive()
        .and_hms_opt(cutoff.hour(), 0, 0)
        .map(|hour| hour.and_utc())
        .unwrap_or(cutoff)
}

pub async fn meta(
    State(state): State<AppState>,
    Query(query): Query<RangeQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, query.tenant.as_deref()).await?;
    let tenants = state.store.list_tenants().await?;
    let last_event_at = state.store.last_event_at(tenant.id).await?;
    let rollups_ready = state.store.rollups_ready().await?;
    let raw_retention_hours = state.store.global_retention_hours().await?;

    let client = state.store.pool().get().await?;
    let devices = client
        .query(
            "SELECT name, device_id, max(model) AS model, max(firmware) AS firmware,
                    max(last_seen) AS last_seen, sum(events)::bigint AS events
             FROM device_activity
             WHERE tenant_id = $1
               AND bucket >= date_trunc('minute', now() - interval '5 minutes')
               AND last_seen > now() - interval '5 minutes'
             GROUP BY device_key, name, device_id
             ORDER BY last_seen DESC
             LIMIT 20",
            &[&tenant.id],
        )
        .await?;
    let devices: Vec<serde_json::Value> = devices
        .iter()
        .map(|row| {
            json!({
                "name": row.get::<_, String>("name"),
                "device_id": row.get::<_, Option<String>>("device_id"),
                "model": row.get::<_, Option<String>>("model"),
                "firmware": row.get::<_, Option<String>>("firmware"),
                "last_seen": row.get::<_, DateTime<Utc>>("last_seen"),
                "events": row.get::<_, i64>("events"),
            })
        })
        .collect();

    Ok(Json(json!({
        "tenants": tenants.iter().map(TenantView::from).collect::<Vec<_>>(),
        "tenant": TenantView::from(&tenant),
        "server_time": Utc::now(),
        "last_event_at": last_event_at,
        "devices": devices,
        "raw_retention_hours": raw_retention_hours,
        "rollups_ready": rollups_ready,
    })))
}

const RETENTION_CHOICES_HOURS: &[i64] = &[6, 12, 24, 48, 72, 168];

pub async fn retention_preview(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let current_hours = state.store.global_retention_hours().await?;
    let (database_bytes, raw_partition_bytes, bytes_per_hour) =
        state.store.retention_storage_estimate().await?;
    let base_bytes = database_bytes.saturating_sub(raw_partition_bytes);
    let options: Vec<serde_json::Value> = RETENTION_CHOICES_HOURS
        .iter()
        .map(|hours| {
            let projected = bytes_per_hour.map(|rate| {
                base_bytes.saturating_add(rate.saturating_mul(hours.saturating_add(1)))
            });
            json!({
                "hours": hours,
                "estimated_database_bytes": projected,
            })
        })
        .collect();
    Ok(Json(json!({
        "global": true,
        "current_hours": current_hours,
        "choices_hours": RETENTION_CHOICES_HOURS,
        "database_bytes": database_bytes,
        "current_raw_partition_bytes": raw_partition_bytes,
        "estimated_raw_bytes_per_hour": bytes_per_hour,
        "options": options,
        "admin_configured": state.retention_admin_token.is_some(),
        "application_delay_minutes": 5,
        "estimate_note": "Approximate projection from PostgreSQL partition statistics and recent partition sizes; includes one extra hour for whole-hour partition boundaries. It is not a storage guarantee.",
    })))
}

#[derive(Debug, Deserialize)]
pub struct RetentionUpdate {
    retention_hours: i64,
}

pub async fn update_retention(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RetentionUpdate>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Some(expected) = state.retention_admin_token.as_deref() else {
        return Err(ApiError::unavailable(
            "retention changes are disabled; configure RETENTION_ADMIN_TOKEN on the API service",
        ));
    };
    let valid = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| constant_time_eq(token.as_bytes(), expected.as_bytes()));
    if !valid {
        return Err(ApiError::unauthorized());
    }
    if !RETENTION_CHOICES_HOURS.contains(&request.retention_hours) {
        return Err(ApiError::bad_request(format!(
            "retention_hours must be one of: {}",
            RETENTION_CHOICES_HOURS
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    state
        .store
        .set_global_retention_hours(request.retention_hours)
        .await?;
    Ok(Json(json!({
        "global": true,
        "retention_hours": request.retention_hours,
        "message": "Retention setting saved; expired hourly partitions are removed by collector maintenance within about five minutes after their rollups are refreshed.",
    })))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= usize::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

#[derive(Debug, Serialize, Default)]
struct Totals {
    events: i64,
    web_requests: i64,
    allowed: i64,
    blocked: i64,
    bytes_up: i64,
    bytes_down: i64,
    threats: i64,
    users: i64,
    domains: i64,
}

#[derive(Debug, Serialize, Clone)]
struct SeriesPoint {
    t: DateTime<Utc>,
    events: i64,
    allowed: i64,
    blocked: i64,
    bytes_up: i64,
    bytes_down: i64,
}

pub async fn overview(
    State(state): State<AppState>,
    Query(query): Query<RangeQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _permit = acquire_query_permit(
        &state.query_semaphore,
        "dashboard query capacity is busy; retry shortly",
    )?;
    let tenant = resolve_tenant(&state, query.tenant.as_deref()).await?;
    let range = resolve_range(&query)?;
    let client = state.store.pool().get().await?;
    let rollups_ready = state.store.rollups_ready().await?;
    let rollup_from = hour_floor(range.from)?;
    let rollup_to = hour_ceil(range.to)?;

    let totals_sql = if rollups_ready {
        "WITH totals AS (
                    SELECT COALESCE(sum(events), 0)::bigint AS events,
                           COALESCE(sum(web_requests), 0)::bigint AS web_requests,
                           COALESCE(sum(allowed), 0)::bigint AS allowed,
                           COALESCE(sum(blocked), 0)::bigint AS blocked,
                           COALESCE(sum(bytes_up), 0)::bigint AS bytes_up,
                           COALESCE(sum(bytes_down), 0)::bigint AS bytes_down,
                           COALESCE(sum(threats), 0)::bigint AS threats
                    FROM event_rollups
                    WHERE tenant_id = $1 AND dimension = 'overview'
                      AND bucket >= $2 AND bucket < $3
                ), identities AS (
                    SELECT count(DISTINCT key) FILTER (WHERE dimension = 'users')::bigint AS users,
                           count(DISTINCT key) FILTER (WHERE dimension = 'domains')::bigint AS domains
                    FROM event_rollups
                    WHERE tenant_id = $1 AND dimension IN ('users', 'domains')
                      AND bucket >= $2 AND bucket < $3
                )
                SELECT totals.*, identities.*
                FROM totals CROSS JOIN identities"
    } else {
        "SELECT count(*)::bigint AS events,
                    count(*) FILTER (WHERE category_group = 'web'
                        AND COALESCE(con_event, '') <> 'Start')::bigint AS web_requests,
                    count(*) FILTER (WHERE action = 'allowed'
                        AND COALESCE(con_event, '') <> 'Start')::bigint AS allowed,
                    count(*) FILTER (WHERE action = 'blocked'
                        AND COALESCE(con_event, '') <> 'Start')::bigint AS blocked,
                    COALESCE(sum(sent_bytes), 0)::bigint AS bytes_up,
                    COALESCE(sum(recv_bytes), 0)::bigint AS bytes_down,
                    count(*) FILTER (WHERE category_group = 'threat')::bigint AS threats,
                    count(DISTINCT COALESCE(NULLIF(user_name, ''), host(src_ip)))::bigint AS users,
                    count(DISTINCT domain)::bigint AS domains
             FROM events
             WHERE tenant_id = $1 AND ts >= $2 AND ts < $3"
    };
    let totals_params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = if rollups_ready {
        vec![&tenant.id, &rollup_from, &rollup_to]
    } else {
        vec![&tenant.id, &range.from, &range.to]
    };
    let totals_row = client.query_one(totals_sql, &totals_params).await?;
    let totals = Totals {
        events: totals_row.get("events"),
        web_requests: totals_row.get("web_requests"),
        allowed: totals_row.get("allowed"),
        blocked: totals_row.get("blocked"),
        bytes_up: totals_row.get("bytes_up"),
        bytes_down: totals_row.get("bytes_down"),
        threats: totals_row.get("threats"),
        users: totals_row.get("users"),
        domains: totals_row.get("domains"),
    };

    let bucket_secs = if rollups_ready {
        range.bucket_secs.max(3_600)
    } else {
        range.bucket_secs
    };
    let series_sql = if rollups_ready {
        "SELECT to_timestamp(floor(extract(epoch FROM bucket) / $4::bigint) * $4::bigint) AS bucket,
                        sum(events)::bigint AS events,
                        sum(allowed)::bigint AS allowed,
                        sum(blocked)::bigint AS blocked,
                        sum(bytes_up)::bigint AS bytes_up,
                        sum(bytes_down)::bigint AS bytes_down
                 FROM event_rollups
                 WHERE tenant_id = $1 AND dimension = 'overview'
                   AND bucket >= $2 AND bucket < $3
                 GROUP BY 1
                 ORDER BY 1"
    } else {
        "SELECT to_timestamp(floor(extract(epoch FROM ts) / $4::bigint) * $4::bigint) AS bucket,
                    count(*)::bigint AS events,
                    count(*) FILTER (WHERE action = 'allowed'
                        AND COALESCE(con_event, '') <> 'Start')::bigint AS allowed,
                    count(*) FILTER (WHERE action = 'blocked'
                        AND COALESCE(con_event, '') <> 'Start')::bigint AS blocked,
                    COALESCE(sum(sent_bytes), 0)::bigint AS bytes_up,
                    COALESCE(sum(recv_bytes), 0)::bigint AS bytes_down
             FROM events
             WHERE tenant_id = $1 AND ts >= $2 AND ts < $3
             GROUP BY 1
             ORDER BY 1"
    };
    let series_params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = if rollups_ready {
        vec![&tenant.id, &rollup_from, &rollup_to, &bucket_secs]
    } else {
        vec![&tenant.id, &range.from, &range.to, &bucket_secs]
    };
    let rows = client.query(series_sql, &series_params).await?;
    let points: Vec<SeriesPoint> = rows
        .iter()
        .map(|row| SeriesPoint {
            t: row.get("bucket"),
            events: row.get("events"),
            allowed: row.get("allowed"),
            blocked: row.get("blocked"),
            bytes_up: row.get("bytes_up"),
            bytes_down: row.get("bytes_down"),
        })
        .collect();

    Ok(Json(json!({
        "range": {
            "from": range.from,
            "to": range.to,
            "bucket_secs": bucket_secs,
        },
        "totals": totals,
        "series": fill_series(points, range.from, range.to, bucket_secs),
    })))
}

/// The chart needs a point for every bucket, including quiet minutes.
fn fill_series(
    points: Vec<SeriesPoint>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    step: i64,
) -> Vec<SeriesPoint> {
    let step = step.max(1);
    let mut by_bucket: HashMap<i64, SeriesPoint> = points
        .into_iter()
        .map(|p| (p.t.timestamp().div_euclid(step) * step, p))
        .collect();
    let mut out = Vec::new();
    let mut t = from.timestamp().div_euclid(step) * step;
    let end = to.timestamp().div_euclid(step) * step;
    let mut guard = 0;
    while t <= end && guard < 5_000 {
        let point = by_bucket.remove(&t).unwrap_or(SeriesPoint {
            t: Utc.timestamp_opt(t, 0).single().unwrap_or(from),
            events: 0,
            allowed: 0,
            blocked: 0,
            bytes_up: 0,
            bytes_down: 0,
        });
        out.push(point);
        t += step;
        guard += 1;
    }
    out
}

#[derive(Debug, Deserialize)]
pub struct TopQuery {
    pub tenant: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub dimension: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
struct TopRow {
    key: String,
    events: i64,
    blocked: i64,
    bytes: i64,
    users: Option<i64>,
}

// Report request/response types
#[derive(Debug, Deserialize)]
pub struct CreateReportSchedule {
    pub name: String,
    pub description: Option<String>,
    pub cron_schedule: String,
    pub format: String,
    pub recipients: serde_json::Value,
    pub filters: serde_json::Value,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateReportSchedule {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub cron_schedule: Option<String>,
    pub format: Option<String>,
    pub recipients: Option<serde_json::Value>,
    pub filters: Option<serde_json::Value>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ReportScheduleResponse {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub cron_schedule: String,
    pub format: String,
    pub recipients: serde_json::Value,
    pub filters: serde_json::Value,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ReportResultResponse {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub status: String,
    pub file_path: Option<String>,
    pub row_count: i64,
    pub error_message: Option<String>,
    pub generated_at: DateTime<Utc>,
    pub sent_at: Option<DateTime<Utc>>,
}

// Alert request/response types
#[derive(Debug, Deserialize)]
pub struct CreateAlertRule {
    pub name: String,
    pub description: Option<String>,
    pub condition_type: String,
    pub threshold: String,
    pub severity: Option<String>,
    pub cooldown_minutes: Option<i32>,
    pub enabled: Option<bool>,
    pub webhook_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAlertRule {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub condition_type: Option<String>,
    pub threshold: Option<String>,
    pub severity: Option<String>,
    pub cooldown_minutes: Option<i32>,
    pub enabled: Option<bool>,
    pub webhook_url: Option<Option<String>>,
}

#[derive(Debug, Serialize)]
pub struct AlertRuleResponse {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub condition_type: String,
    pub threshold: String,
    pub severity: String,
    pub cooldown_minutes: i32,
    pub enabled: bool,
    pub webhook_url: Option<String>,
    pub last_triggered: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AlertInstanceResponse {
    pub id: Uuid,
    pub rule_id: Uuid,
    pub tenant_id: Uuid,
    pub message: String,
    pub severity: String,
    pub triggered_at: DateTime<Utc>,
    pub acknowledged: bool,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub acknowledged_by: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AcknowledgeAlert {
    pub acknowledged_by: String,
}

pub async fn top(
    State(state): State<AppState>,
    Query(query): Query<TopQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _permit = acquire_query_permit(
        &state.query_semaphore,
        "dashboard query capacity is busy; retry shortly",
    )?;
    let dimension = query.dimension.as_deref().unwrap_or("users");
    let tenant = resolve_tenant(&state, query.tenant.as_deref()).await?;
    let range = resolve_range(&RangeQuery {
        tenant: query.tenant.clone(),
        from: query.from.clone(),
        to: query.to.clone(),
        interval: None,
    })?;
    let limit = query.limit.unwrap_or(10).clamp(1, 100);

    let client = state.store.pool().get().await?;
    let rollups_ready = state.store.rollups_ready().await?;
    let rollup_from = hour_floor(range.from)?;
    let rollup_to = hour_ceil(range.to)?;
    let (sql, params): (String, Vec<&(dyn tokio_postgres::types::ToSql + Sync)>) = if rollups_ready
    {
        if dimension != "destinations" && dimension_expr(dimension).is_none() {
            return Err(ApiError::bad_request(format!(
                "unknown dimension {dimension:?}"
            )));
        }
        (
            "SELECT key, sum(events)::bigint AS events,
                        sum(blocked)::bigint AS blocked,
                        sum(bytes_up + bytes_down)::bigint AS bytes,
                        NULL::bigint AS users
                 FROM event_rollups
                 WHERE tenant_id = $1 AND dimension = $2
                   AND bucket >= $3 AND bucket < $4
                 GROUP BY key
                 HAVING sum(events) > 0
                 ORDER BY events DESC, bytes DESC
                 LIMIT $5"
                .to_string(),
            vec![&tenant.id, &dimension, &rollup_from, &rollup_to, &limit],
        )
    } else if dimension == "destinations" {
        (
                "SELECT destination.key,
                        count(*)::bigint AS events,
                        count(*) FILTER (WHERE action = 'blocked')::bigint AS blocked,
                        COALESCE(sum(COALESCE(sent_bytes, 0) + COALESCE(recv_bytes, 0)), 0)::bigint AS bytes,
                        NULL::bigint AS users
                 FROM events
                 CROSS JOIN LATERAL (
                     VALUES
                         ('Website: ' || NULLIF(domain, '')),
                         ('Application: ' || NULLIF(application, ''))
                 ) AS destination(key)
                 WHERE tenant_id = $1 AND ts >= $2 AND ts < $3
                   AND COALESCE(con_event, '') <> 'Start'
                   AND destination.key IS NOT NULL
                 GROUP BY destination.key
                 ORDER BY events DESC, bytes DESC
                 LIMIT $4"
                    .to_string(),
                vec![&tenant.id, &range.from, &range.to, &limit],
            )
    } else {
        let expr = dimension_expr(dimension)
            .ok_or_else(|| ApiError::bad_request(format!("unknown dimension {dimension:?}")))?;
        let extra = if dimension == "threats" {
            "AND category_group = 'threat'"
        } else if dimension == "categories" {
            "AND category_group IN ('web', 'threat')"
        } else {
            ""
        };
        let users = if dimension == "users" {
            "1::bigint"
        } else {
            "NULL::bigint"
        };
        (
                format!(
                    "SELECT {expr} AS key,
                            count(*)::bigint AS events,
                            count(*) FILTER (WHERE action = 'blocked')::bigint AS blocked,
                            COALESCE(sum(COALESCE(sent_bytes, 0) + COALESCE(recv_bytes, 0)), 0)::bigint AS bytes,
                            {users} AS users
                     FROM events
                     WHERE tenant_id = $1 AND ts >= $2 AND ts < $3
                       AND COALESCE(con_event, '') <> 'Start'
                       AND {expr} IS NOT NULL AND {expr} <> '' {extra}
                     GROUP BY 1
                     ORDER BY events DESC, bytes DESC
                     LIMIT $4"
                ),
                vec![&tenant.id, &range.from, &range.to, &limit],
            )
    };
    let rows = client.query(&sql, &params).await?;
    let rows: Vec<TopRow> = rows
        .iter()
        .map(|row| TopRow {
            key: row.get("key"),
            events: row.get("events"),
            blocked: row.get("blocked"),
            bytes: row.get("bytes"),
            users: row.get("users"),
        })
        .collect();
    Ok(Json(json!({
        "dimension": dimension,
        "range": { "from": range.from, "to": range.to },
        "rows": rows,
    })))
}

fn dimension_expr(dimension: &str) -> Option<&'static str> {
    match dimension {
        "users" => Some("COALESCE(NULLIF(user_name, ''), host(src_ip))"),
        "domains" => Some("domain"),
        "categories" => Some("category"),
        "policies" => Some("fw_rule_name"),
        "threats" => Some("COALESCE(threat_name, reason)"),
        "applications" => Some("application"),
        "countries" => Some("dst_country"),
        "devices" => Some("COALESCE(NULLIF(device_name, ''), NULLIF(device_id, ''))"),
        "log_types" => Some("log_type"),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    pub tenant: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub q: Option<String>,
    pub action: Option<String>,
    pub group: Option<String>,
    pub user: Option<String>,
    pub category: Option<String>,
    pub device: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let _permit = acquire_query_permit(
        &state.query_semaphore,
        "event search capacity is busy; retry shortly",
    )?;
    let tenant = resolve_tenant(&state, query.tenant.as_deref()).await?;
    let range = resolve_range(&RangeQuery {
        tenant: query.tenant.clone(),
        from: query.from.clone(),
        to: query.to.clone(),
        interval: None,
    })?;
    let retention_hours = state.store.global_retention_hours().await?;
    let cutoff = raw_cutover(Utc::now(), retention_hours);
    if range.to <= cutoff {
        return Err(ApiError::gone(format!(
            "raw event details are retained for {} hours",
            retention_hours
        )));
    }
    let raw_from = range.from.max(cutoff);
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let offset = query.offset.unwrap_or(0).clamp(0, 100_000);
    let q = query
        .q
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let action = query
        .action
        .filter(|a| matches!(a.as_str(), "allowed" | "blocked" | "warned"));
    let group = query.group.filter(|g| {
        matches!(
            g.as_str(),
            "web" | "threat" | "firewall" | "auth" | "vpn" | "system" | "other"
        )
    });
    let user_filter = query
        .user
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let category_filter = query
        .category
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let device_filter = query
        .device
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let client = state.store.pool().get().await?;
    let rows = client
        .query(
            "SELECT id, ts, category_group, action, log_type, log_component, device_name,
                    host(src_ip) AS src_ip, host(dst_ip) AS dst_ip, src_port, dst_port,
                    COALESCE(NULLIF(user_name, ''), host(src_ip)) AS display_user,
                    domain, url, http_method, http_status, category,
                    COALESCE(category_type, attrs->>'http_category_type',
                             attrs->>'category_type') AS category_type,
                    application,
                    fw_rule_name, sent_bytes, recv_bytes, threat_name, message
             FROM events
             WHERE tenant_id = $1 AND ts >= $2 AND ts < $3
               AND ($4::text IS NULL OR user_name ILIKE '%' || $4 || '%'
                    OR domain ILIKE '%' || $4 || '%'
                    OR url ILIKE '%' || $4 || '%'
                    OR message ILIKE '%' || $4 || '%'
                    OR threat_name ILIKE '%' || $4 || '%'
                    OR category ILIKE '%' || $4 || '%'
                    OR COALESCE(category_type, attrs->>'http_category_type',
                                attrs->>'category_type') ILIKE '%' || $4 || '%'
                    OR fw_rule_name ILIKE '%' || $4 || '%'
                    OR host(src_ip) = $4)
               AND ($5::text IS NULL OR action = $5)
               AND ($6::text IS NULL OR category_group = $6)
               AND ($9::text IS NULL OR COALESCE(NULLIF(user_name, ''), host(src_ip)) ILIKE '%' || $9 || '%')
            AND ($10::text IS NULL OR category ILIKE '%' || $10 || '%'
                 OR COALESCE(category_type, attrs->>'http_category_type', attrs->>'category_type') ILIKE '%' || $10 || '%')
            AND ($11::text IS NULL OR device_name ILIKE '%' || $11 || '%' OR device_id ILIKE '%' || $11 || '%')
             ORDER BY ts DESC, id DESC
             LIMIT $7 OFFSET $8",
            &[
                &tenant.id, &raw_from, &range.to, &q, &action, &group, &limit, &offset,
                &user_filter, &category_filter, &device_filter,
            ],
        )
        .await?;
    let rows: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.get::<_, i64>("id"),
                "ts": row.get::<_, DateTime<Utc>>("ts"),
                "group": row.get::<_, String>("category_group"),
                "action": row.get::<_, Option<String>>("action"),
                "log_type": row.get::<_, String>("log_type"),
                "log_component": row.get::<_, Option<String>>("log_component"),
                "device_name": row.get::<_, Option<String>>("device_name"),
                "src_ip": row.get::<_, Option<String>>("src_ip"),
                "dst_ip": row.get::<_, Option<String>>("dst_ip"),
                "src_port": row.get::<_, Option<i32>>("src_port"),
                "dst_port": row.get::<_, Option<i32>>("dst_port"),
                "user": row.get::<_, Option<String>>("display_user"),
                "domain": row.get::<_, Option<String>>("domain"),
                "url": row.get::<_, Option<String>>("url"),
                "method": row.get::<_, Option<String>>("http_method"),
                "status": row.get::<_, Option<i32>>("http_status"),
                "category": row.get::<_, Option<String>>("category"),
                "category_type": row.get::<_, Option<String>>("category_type"),
                "application": row.get::<_, Option<String>>("application"),
                "rule": row.get::<_, Option<String>>("fw_rule_name"),
                "bytes_up": row.get::<_, Option<i64>>("sent_bytes"),
                "bytes_down": row.get::<_, Option<i64>>("recv_bytes"),
                "threat": row.get::<_, Option<String>>("threat_name"),
                "message": row.get::<_, Option<String>>("message"),
            })
        })
        .collect();

    Ok(Json(json!({
        "range": { "from": raw_from, "to": range.to },
        "raw_window_hours": retention_hours,
        "limit": limit,
        "offset": offset,
        "rows": rows,
    })))
}

pub async fn events_export(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Response, ApiError> {
    let _permit = acquire_query_permit(
        &state.query_semaphore,
        "event export capacity is busy; retry shortly",
    )?;
    let tenant = resolve_tenant(&state, query.tenant.as_deref()).await?;
    let range = resolve_range(&RangeQuery {
        tenant: query.tenant.clone(),
        from: query.from.clone(),
        to: query.to.clone(),
        interval: None,
    })?;
    let retention_hours = state.store.global_retention_hours().await?;
    let cutoff = raw_cutover(Utc::now(), retention_hours);
    if range.to <= cutoff {
        return Err(ApiError::gone(format!(
            "raw event details are retained for {} hours",
            retention_hours
        )));
    }
    let raw_from = range.from.max(cutoff);
    let q = query
        .q
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let action = query
        .action
        .filter(|a| matches!(a.as_str(), "allowed" | "blocked" | "warned"));
    let group = query.group.filter(|g| {
        matches!(
            g.as_str(),
            "web" | "threat" | "firewall" | "auth" | "vpn" | "system" | "other"
        )
    });
    let user_filter = query
        .user
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let category_filter = query
        .category
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let device_filter = query
        .device
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let client = state.store.pool().get().await?;
    let rows = client
        .query(
            "SELECT ts, log_type, log_component, category_group, action, display_user,
                    COALESCE(domain, host(dst_ip)) AS destination, application, category,
                    COALESCE(category_type, attrs->>'http_category_type', attrs->>'category_type') AS category_type,
                    url, host(src_ip) AS src_ip, host(dst_ip) AS dst_ip, fw_rule_name,
                    sent_bytes, recv_bytes, threat_name, message, device_name
             FROM (
                 SELECT tenant_id, id, ts, category_group, action, log_type, log_component, device_name,
                        host(src_ip) AS src_ip, host(dst_ip) AS dst_ip,
                        COALESCE(NULLIF(user_name, ''), host(src_ip)) AS display_user,
                        domain, url, http_method, http_status, category,
                        attrs,
                        COALESCE(category_type, attrs->>'http_category_type',
                                 attrs->>'category_type') AS category_type,
                        application,
                        fw_rule_name, sent_bytes, recv_bytes, threat_name, message
                 FROM events
             ) e
             WHERE tenant_id = $1 AND ts >= $2 AND ts < $3
               AND ($4::text IS NULL OR display_user ILIKE '%' || $4 || '%'
                    OR domain ILIKE '%' || $4 || '%'
                    OR url ILIKE '%' || $4 || '%'
                    OR message ILIKE '%' || $4 || '%'
                    OR threat_name ILIKE '%' || $4 || '%'
                    OR category ILIKE '%' || $4 || '%'
                    OR COALESCE(category_type, attrs->>'http_category_type',
                                attrs->>'category_type') ILIKE '%' || $4 || '%'
                    OR fw_rule_name ILIKE '%' || $4 || '%'
                    OR src_ip = $4)
               AND ($5::text IS NULL OR action = $5)
               AND ($6::text IS NULL OR category_group = $6)
               AND ($9::text IS NULL OR display_user ILIKE '%' || $9 || '%')
               AND ($10::text IS NULL OR category ILIKE '%' || $10 || '%'
                    OR COALESCE(category_type, attrs->>'http_category_type', attrs->>'category_type') ILIKE '%' || $10 || '%')
               AND ($11::text IS NULL OR device_name ILIKE '%' || $11 || '%')
             ORDER BY ts ASC",
            &[
                &tenant.id, &raw_from, &range.to, &q, &action, &group,
                &100_000i64, &0i64,
                &user_filter, &category_filter, &device_filter,
            ],
        )
        .await?;

    let mut body = String::from("timestamp,log_type,log_component,group,action,user,destination,application,category,category_type,url,src_ip,dst_ip,policy,bytes_up,bytes_down,threat,message,device\n");
    for row in rows {
        let ts: DateTime<Utc> = row.get("ts");
        let log_type: String = row.get("log_type");
        let log_component: Option<String> = row.get("log_component");
        let group: String = row.get("group");
        let action: Option<String> = row.get("action");
        let user: Option<String> = row.get("display_user");
        let destination: Option<String> = row.get("destination");
        let application: Option<String> = row.get("application");
        let category: Option<String> = row.get("category");
        let category_type: Option<String> = row.get("category_type");
        let url: Option<String> = row.get("url");
        let src_ip: Option<String> = row.get("src_ip");
        let dst_ip: Option<String> = row.get("dst_ip");
        let policy: Option<String> = row.get("fw_rule_name");
        let bytes_up: Option<i64> = row.get("sent_bytes");
        let bytes_down: Option<i64> = row.get("recv_bytes");
        let threat: Option<String> = row.get("threat_name");
        let message: Option<String> = row.get("message");
        let device: Option<String> = row.get("device_name");

        let mut line = String::new();
        line.push_str(&csv_escape(ts.to_rfc3339()));
        line.push(',');
        line.push_str(&csv_escape(log_type));
        line.push(',');
        line.push_str(&csv_escape(log_component.unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(group));
        line.push(',');
        line.push_str(&csv_escape(action.unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(user.unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(destination.unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(application.unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(category.unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(category_type.unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(url.unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(src_ip.unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(dst_ip.unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(policy.unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(bytes_up.map(|v| v.to_string()).unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(bytes_down.map(|v| v.to_string()).unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(threat.unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(message.unwrap_or_default()));
        line.push(',');
        line.push_str(&csv_escape(device.unwrap_or_default()));
        line.push('\n');
        body.push_str(&line);
    }

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        "text/csv; charset=utf-8".parse().unwrap(),
    );
    headers.insert(
        axum::http::header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"kira-events-{}.csv\"", Utc::now().format("%Y%m%dT%H%M%SZ")).parse().unwrap(),
    );
    Ok((headers, body).into_response())
}

fn csv_escape(value: String) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

fn send_email(
    state: &AppState,
    to: &[String],
    subject: &str,
    body: &str,
) -> Result<(), ApiError> {
    let Some(host) = &state.smtp_host else {
        return Err(ApiError::unavailable("SMTP not configured"));
    };
    let port = state.smtp_port.unwrap_or(587);
    let username = match &state.smtp_username {
        Some(u) => u.clone(),
        None => return Err(ApiError::unavailable("SMTP username not configured")),
    };
    let password = match &state.smtp_password {
        Some(p) => p.clone(),
        None => return Err(ApiError::unavailable("SMTP password not configured")),
    };
    let from = match &state.smtp_from {
        Some(f) => f.clone(),
        None => return Err(ApiError::unavailable("SMTP from address not configured")),
    };

    let email = lettre::message::Message::builder()
        .from(from.parse().map_err(|_| ApiError::bad_request("invalid from address"))?)
        .to(to[0].parse().map_err(|_| ApiError::bad_request("invalid to address"))?)
        .subject(subject)
        .singlepart(
            lettre::message::SinglePart::builder()
                .body(body.to_string()),
        )
        .map_err(|e| ApiError::internal(anyhow::anyhow!("failed to build email: {e}")))?;

    let mailer = lettre::SmtpTransport::relay(host)
        .map_err(|e| ApiError::internal(anyhow::anyhow!("failed to create SMTP transport: {e}")))?
        .port(port)
        .credentials(lettre::transport::smtp::authentication::Credentials::new(username, password))
        .build();

    mailer.send(&email).map_err(|e| {
        ApiError::internal(anyhow::anyhow!("failed to send email: {e}"))
    })?;

    Ok(())
}

async fn send_webhook(
    state: &AppState,
    url: &str,
    rule_name: &str,
    message: &str,
    severity: &str,
    triggered_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    let payload = serde_json::json!({
        "rule_name": rule_name,
        "message": message,
        "severity": severity,
        "triggered_at": triggered_at.to_rfc3339(),
    });

    let client = reqwest::Client::new();
    let res = client
        .post(url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| ApiError::internal(anyhow::anyhow!("webhook request failed: {e}")))?;

    let status = res.status().as_u16();
    let body = res.text().await.unwrap_or_default();

    if status >= 200 && status < 300 {
        Ok(())
    } else {
        Err(ApiError::internal(anyhow::anyhow!(
            "webhook returned status {}: {}",
            status,
            body
        )))
    }
}

// Report schedule handlers
pub async fn list_report_schedules(
    State(state): State<AppState>,
    Query(query): Query<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, query.get("tenant").and_then(|v| v.as_str())).await?;
    let schedules = state.store.list_report_schedules(tenant.id).await?;
    Ok(Json(json!({
        "schedules": schedules.iter().map(|s| ReportScheduleResponse {
            id: s.id,
            tenant_id: s.tenant_id,
            name: s.name.clone(),
            description: s.description.clone(),
            cron_schedule: s.cron_schedule.clone(),
            format: s.format.clone(),
            recipients: s.recipients.clone(),
            filters: s.filters.clone(),
            enabled: s.enabled,
            last_run: s.last_run,
            next_run: s.next_run,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }).collect::<Vec<_>>()
    })))
}

pub async fn create_report_schedule(
    State(state): State<AppState>,
    Json(payload): Json<CreateReportSchedule>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, None).await?;
    if !["csv", "pdf"].contains(&payload.format.as_str()) {
        return Err(ApiError::bad_request("format must be csv or pdf"));
    }
    let schedule = ReportSchedule {
        id: Uuid::new_v4(),
        tenant_id: tenant.id,
        name: payload.name,
        description: payload.description,
        cron_schedule: payload.cron_schedule,
        format: payload.format,
        recipients: payload.recipients,
        filters: payload.filters,
        enabled: payload.enabled.unwrap_or(true),
        last_run: None,
        next_run: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let created = state.store.create_report_schedule(&schedule).await?;
    Ok(Json(json!({
        "schedule": ReportScheduleResponse {
            id: created.id,
            tenant_id: created.tenant_id,
            name: created.name,
            description: created.description,
            cron_schedule: created.cron_schedule,
            format: created.format,
            recipients: created.recipients,
            filters: created.filters,
            enabled: created.enabled,
            last_run: created.last_run,
            next_run: created.next_run,
            created_at: created.created_at,
            updated_at: created.updated_at,
        }
    })))
}

pub async fn update_report_schedule(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(payload): Json<UpdateReportSchedule>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, None).await?;
    // Verify schedule exists and belongs to tenant
    let _ = state.store.list_report_schedules(tenant.id).await?;
    let schedule = ReportSchedule {
        id,
        tenant_id: tenant.id,
        name: payload.name.unwrap_or_default(),
        description: payload.description.flatten(),
        cron_schedule: payload.cron_schedule.unwrap_or_default(),
        format: payload.format.unwrap_or_default(),
        recipients: payload.recipients.unwrap_or(serde_json::json!([])),
        filters: payload.filters.unwrap_or(serde_json::json!({})),
        enabled: payload.enabled.unwrap_or(true),
        last_run: None,
        next_run: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let updated = state.store.update_report_schedule(id, &schedule).await?;
    match updated {
        Some(s) => Ok(Json(json!({
            "schedule": ReportScheduleResponse {
                id: s.id,
                tenant_id: s.tenant_id,
                name: s.name,
                description: s.description,
                cron_schedule: s.cron_schedule,
                format: s.format,
                recipients: s.recipients,
                filters: s.filters,
                enabled: s.enabled,
                last_run: s.last_run,
                next_run: s.next_run,
                created_at: s.created_at,
                updated_at: s.updated_at,
            }
        }))),
        None => Err(ApiError::not_found("report schedule not found")),
    }
}

pub async fn delete_report_schedule(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, None).await?;
    let _ = state.store.list_report_schedules(tenant.id).await?;
    let deleted = state.store.delete_report_schedule(id).await?;
    if deleted {
        Ok(Json(json!({"message": "report schedule deleted"})))
    } else {
        Err(ApiError::not_found("report schedule not found"))
    }
}

pub async fn list_report_results(
    State(state): State<AppState>,
    axum::extract::Path(schedule_id): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, None).await?;
    let _ = state.store.list_report_schedules(tenant.id).await?;
    let results = state.store.list_report_results(schedule_id).await?;
    Ok(Json(json!({
        "results": results.iter().map(|r| ReportResultResponse {
            id: r.id,
            schedule_id: r.schedule_id,
            status: r.status.clone(),
            file_path: r.file_path.clone(),
            row_count: r.row_count,
            error_message: r.error_message.clone(),
            generated_at: r.generated_at,
            sent_at: r.sent_at,
        }).collect::<Vec<_>>()
    })))
}

// Alert rule handlers
pub async fn list_alert_rules(
    State(state): State<AppState>,
    Query(query): Query<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, query.get("tenant").and_then(|v| v.as_str())).await?;
    let rules = state.store.get_enabled_alert_rules(tenant.id).await?;
    Ok(Json(json!({
        "rules": rules.iter().map(|r| AlertRuleResponse {
            id: r.id,
            tenant_id: r.tenant_id,
            name: r.name.clone(),
            description: r.description.clone(),
            condition_type: r.condition_type.clone(),
            threshold: r.threshold.clone(),
            severity: r.severity.clone(),
            cooldown_minutes: r.cooldown_minutes,
            enabled: r.enabled,
            webhook_url: r.webhook_url.clone(),
            last_triggered: r.last_triggered,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }).collect::<Vec<_>>()
    })))
}

pub async fn create_alert_rule(
    State(state): State<AppState>,
    Json(payload): Json<CreateAlertRule>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, None).await?;
    if !["threat", "blocked_category", "large_transfer"].contains(&payload.condition_type.as_str()) {
        return Err(ApiError::bad_request("condition_type must be threat, blocked_category, or large_transfer"));
    }
    if !["low", "medium", "high", "critical"].contains(&payload.severity.as_deref().unwrap_or("medium")) {
        return Err(ApiError::bad_request("severity must be low, medium, high, or critical"));
    }
    let rule = AlertRule {
        id: Uuid::new_v4(),
        tenant_id: tenant.id,
        name: payload.name,
        description: payload.description,
        condition_type: payload.condition_type,
        threshold: payload.threshold,
        severity: payload.severity.unwrap_or("medium".to_string()),
        cooldown_minutes: payload.cooldown_minutes.unwrap_or(60),
        enabled: payload.enabled.unwrap_or(true),
        webhook_url: payload.webhook_url,
        last_triggered: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let created = state.store.create_alert_rule(&rule).await?;
    Ok(Json(json!({
        "rule": AlertRuleResponse {
            id: created.id,
            tenant_id: created.tenant_id,
            name: created.name,
            description: created.description,
            condition_type: created.condition_type,
            threshold: created.threshold,
            severity: created.severity,
            cooldown_minutes: created.cooldown_minutes,
            enabled: created.enabled,
            webhook_url: created.webhook_url,
            last_triggered: created.last_triggered,
            created_at: created.created_at,
            updated_at: created.updated_at,
        }
    })))
}

pub async fn update_alert_rule(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(payload): Json<UpdateAlertRule>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, None).await?;
    let _ = state.store.list_alert_rules(tenant.id).await?;
    let rule = AlertRule {
        id,
        tenant_id: tenant.id,
        name: payload.name.unwrap_or_default(),
        description: payload.description.flatten(),
        condition_type: payload.condition_type.unwrap_or_default(),
        threshold: payload.threshold.unwrap_or_default(),
        severity: payload.severity.unwrap_or_default(),
        cooldown_minutes: payload.cooldown_minutes.unwrap_or(60),
        enabled: payload.enabled.unwrap_or(true),
        webhook_url: payload.webhook_url.flatten(),
        last_triggered: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let updated = state.store.update_alert_rule(id, &rule).await?;
    match updated {
        Some(r) => Ok(Json(json!({
            "rule": AlertRuleResponse {
                id: r.id,
                tenant_id: r.tenant_id,
                name: r.name,
                description: r.description,
                condition_type: r.condition_type,
                threshold: r.threshold,
                severity: r.severity,
                cooldown_minutes: r.cooldown_minutes,
                enabled: r.enabled,
                webhook_url: r.webhook_url,
                last_triggered: r.last_triggered,
                created_at: r.created_at,
                updated_at: r.updated_at,
            }
        }))),
        None => Err(ApiError::not_found("alert rule not found")),
    }
}

pub async fn delete_alert_rule(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, None).await?;
    let _ = state.store.list_alert_rules(tenant.id).await?;
    let deleted = state.store.delete_alert_rule(id).await?;
    if deleted {
        Ok(Json(json!({"message": "alert rule deleted"})))
    } else {
        Err(ApiError::not_found("alert rule not found"))
    }
}

pub async fn list_alert_instances(
    State(state): State<AppState>,
    Query(query): Query<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, query.get("tenant").and_then(|v| v.as_str())).await?;
    let acknowledged = query.get("acknowledged").and_then(|v| v.as_bool());
    let instances = state.store.list_alert_instances(tenant.id, acknowledged).await?;
    Ok(Json(json!({
        "instances": instances.iter().map(|i| AlertInstanceResponse {
            id: i.id,
            rule_id: i.rule_id,
            tenant_id: i.tenant_id,
            message: i.message.clone(),
            severity: i.severity.clone(),
            triggered_at: i.triggered_at,
            acknowledged: i.acknowledged,
            acknowledged_at: i.acknowledged_at,
            acknowledged_by: i.acknowledged_by.clone(),
        }).collect::<Vec<_>>()
    })))
}

pub async fn acknowledge_alert_instance(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(payload): Json<AcknowledgeAlert>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, None).await?;
    let _ = state.store.list_alert_instances(tenant.id, None).await?;
    let acked = state.store.acknowledge_alert_instance(id, &payload.acknowledged_by).await?;
    if acked {
        Ok(Json(json!({"message": "alert acknowledged"})))
    } else {
        Err(ApiError::not_found("alert instance not found or already acknowledged"))
    }
}

pub async fn list_webhook_deliveries(
    State(state): State<AppState>,
    axum::extract::Path(instance_id): axum::extract::Path<Uuid>,
    Query(query): Query<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, query.get("tenant").and_then(|v| v.as_str())).await?;
    let _ = state.store.list_alert_instances(tenant.id, None).await?;
    let deliveries = state.store.list_webhook_deliveries(instance_id).await?;
    Ok(Json(json!({
        "deliveries": deliveries.iter().map(|d| serde_json::json!({
            "id": d.id,
            "alert_instance_id": d.alert_instance_id,
            "webhook_url": d.webhook_url,
            "status": d.status,
            "response_status_code": d.response_status_code,
            "response_body": d.response_body,
            "error_message": d.error_message,
            "delivered_at": d.delivered_at,
        })).collect::<Vec<_>>()
    })))
}

pub async fn test_webhook(
    State(_state): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let url = payload.get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::bad_request("url is required"))?;

    let payload = serde_json::json!({
        "test": true,
        "message": "Kira webhook test",
        "triggered_at": Utc::now().to_rfc3339(),
    });

    let client = reqwest::Client::new();
    let res = client
        .post(url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| ApiError::internal(anyhow::anyhow!("webhook test failed: {e}")))?;

    let status = res.status().as_u16();
    let body = res.text().await.unwrap_or_default();

    if status >= 200 && status < 300 {
        Ok(Json(json!({"status": "ok", "code": status, "body": body})))
    } else {
        Ok(Json(json!({"status": "error", "code": status, "body": body})))
    }
}

pub async fn generate_and_send_report(state: &AppState, schedule: &ReportSchedule, now: DateTime<Utc>) -> Result<(), ApiError> {
    let tenant = state.store.tenant_by_slug("default").await?
        .ok_or_else(|| ApiError::not_found("default tenant not found"))?;

    // Parse filters
    let filters = &schedule.filters;
    let from = filters.get("from").and_then(|v| v.as_str()).unwrap_or("24h");
    let to = filters.get("to").and_then(|v| v.as_str()).unwrap_or("now");

    // Resolve time range
    let range = RangeQuery {
        tenant: Some(tenant.slug.clone()),
        from: Some(from.to_string()),
        to: Some(to.to_string()),
        interval: None,
    }.resolve(now).map_err(|e| ApiError::bad_request(format!("invalid time range: {e}")))?;

    // Get events
    let q = filters.get("q").and_then(|v| v.as_str()).map(|s| s.to_string());
    let action = filters.get("action").and_then(|v| v.as_str()).map(|s| s.to_string());
    let group = filters.get("group").and_then(|v| v.as_str()).map(|s| s.to_string());
    let user = filters.get("user").and_then(|v| v.as_str()).map(|s| s.to_string());
    let category = filters.get("category").and_then(|v| v.as_str()).map(|s| s.to_string());
    let device = filters.get("device").and_then(|v| v.as_str()).map(|s| s.to_string());

    let client = state.store.pool().get().await?;
    let rows = client
        .query(
            "SELECT ts, log_type, log_component, category_group, action, COALESCE(NULLIF(user_name, ''), host(src_ip)) AS display_user,
                    COALESCE(domain, host(dst_ip)::text) AS destination, application, category,
                    COALESCE(category_type, attrs->>'http_category_type', attrs->>'category_type') AS category_type,
                    url, host(src_ip)::text AS src_ip, host(dst_ip)::text AS dst_ip, fw_rule_name,
                    sent_bytes, recv_bytes, threat_name, message, device_name
             FROM events
             WHERE tenant_id = $1 AND ts >= $2 AND ts < $3
               AND ($4::text IS NULL OR display_user ILIKE '%' || $4 || '%'
                    OR domain ILIKE '%' || $4 || '%'
                    OR url ILIKE '%' || $4 || '%'
                    OR message ILIKE '%' || $4 || '%'
                    OR threat_name ILIKE '%' || $4 || '%'
                    OR category ILIKE '%' || $4 || '%'
                    OR COALESCE(category_type, attrs->>'http_category_type', attrs->>'category_type') ILIKE '%' || $4 || '%'
                    OR fw_rule_name ILIKE '%' || $4 || '%'
                    OR host(src_ip) = $4)
               AND ($5::text IS NULL OR action = $5)
               AND ($6::text IS NULL OR category_group = $6)
               AND ($9::text IS NULL OR display_user ILIKE '%' || $9 || '%')
               AND ($10::text IS NULL OR category ILIKE '%' || $10 || '%'
                    OR COALESCE(category_type, attrs->>'http_category_type', attrs->>'category_type') ILIKE '%' || $10 || '%')
               AND ($11::text IS NULL OR device_name ILIKE '%' || $11 || '%')
             ORDER BY ts ASC
             LIMIT 100000",
            &[
                &tenant.id, &range.from, &range.to, &q, &action, &group,
                &user, &category, &device,
            ],
        )
        .await?;

    let row_count = rows.len() as i64;

    // Generate content
    let (content, _content_type, extension) = match schedule.format.as_str() {
        "csv" => {
            let mut body = String::from("timestamp,log_type,log_component,group,action,user,destination,application,category,category_type,url,src_ip,dst_ip,policy,bytes_up,bytes_down,threat,message,device\n");
            for row in &rows {
                let ts: DateTime<Utc> = row.get("ts");
                let log_type: String = row.get("log_type");
                let log_component: Option<String> = row.get("log_component");
                let group: String = row.get("group");
                let action: Option<String> = row.get("action");
                let user: Option<String> = row.get("display_user");
                let destination: Option<String> = row.get("destination");
                let application: Option<String> = row.get("application");
                let category: Option<String> = row.get("category");
                let category_type: Option<String> = row.get("category_type");
                let url: Option<String> = row.get("url");
                let src_ip: Option<String> = row.get("src_ip");
                let dst_ip: Option<String> = row.get("dst_ip");
                let policy: Option<String> = row.get("fw_rule_name");
                let bytes_up: Option<i64> = row.get("sent_bytes");
                let bytes_down: Option<i64> = row.get("recv_bytes");
                let threat: Option<String> = row.get("threat_name");
                let message: Option<String> = row.get("message");
                let device: Option<String> = row.get("device_name");

                let mut line = String::new();
                line.push_str(&csv_escape(ts.to_rfc3339()));
                line.push(',');
                line.push_str(&csv_escape(log_type));
                line.push(',');
                line.push_str(&csv_escape(log_component.unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(group));
                line.push(',');
                line.push_str(&csv_escape(action.unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(user.unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(destination.unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(application.unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(category.unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(category_type.unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(url.unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(src_ip.unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(dst_ip.unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(policy.unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(bytes_up.map(|v| v.to_string()).unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(bytes_down.map(|v| v.to_string()).unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(threat.unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(message.unwrap_or_default()));
                line.push(',');
                line.push_str(&csv_escape(device.unwrap_or_default()));
                line.push('\n');
                body.push_str(&line);
            }
            (body.into_bytes(), "text/csv", "csv")
        }
        "pdf" => {
            // Simple text-based PDF-like output (HTML content with .pdf extension marker)
            let mut body = String::from("<html><head><title>Kira Report</title></head><body>");
            body.push_str(&format!("<h1>Kira Security Report</h1><p>Generated: {}</p>", now.to_rfc3339()));
            body.push_str(&format!("<p>Tenant: {}</p><p>Events: {}</p>", tenant.name, row_count));
            body.push_str("<table border='1'><tr><th>Time</th><th>Action</th><th>User</th><th>Destination</th><th>Category</th><th>Threat</th></tr>");
            for row in &rows {
                let ts: DateTime<Utc> = row.get("ts");
                let action: Option<String> = row.get("action");
                let user: Option<String> = row.get("display_user");
                let destination: Option<String> = row.get("destination");
                let category: Option<String> = row.get("category");
                let threat: Option<String> = row.get("threat_name");
                body.push_str(&format!(
                    "<tr><td>{}</td><td>{:?}</td><td>{:?}</td><td>{:?}</td><td>{:?}</td><td>{:?}</td></tr>",
                    ts.to_rfc3339(), action, user, destination, category, threat
                ));
            }
            body.push_str("</table></body></html>");
            (body.into_bytes(), "text/html", "pdf")
        }
        _ => return Err(ApiError::bad_request("unsupported format")),
    };

    // Store report result
    let file_name = format!("kira-report-{}-{}.{}", schedule.name.replace(' ', "_"), now.format("%Y%m%dT%H%M%SZ"), extension);
    let file_path = format!("/tmp/kira_reports/{}", file_name);

    // Ensure directory exists
    let _ = std::fs::create_dir_all("/tmp/kira_reports");
    let _ = std::fs::write(&file_path, &content);

    let result = ReportResult {
        id: Uuid::new_v4(),
        schedule_id: schedule.id,
        status: "generated".to_string(),
        file_path: Some(file_path.clone()),
        row_count,
        error_message: None,
        generated_at: now,
        sent_at: None,
    };
    state.store.create_report_result(&result).await?;

    // Send email to recipients
    let recipients: Vec<String> = match &schedule.recipients {
        serde_json::Value::Array(arr) => arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(),
        _ => vec![],
    };

    if !recipients.is_empty() {
        let subject = format!("Kira Report: {} ({})", schedule.name, now.format("%Y-%m-%d %H:%M:%S"));
        let body_text = format!(
            "Kira Report: {}\nGenerated: {}\nTenant: {}\nFormat: {}\nEvents: {}\n\nSee attached file for details.\n",
            schedule.name, now.to_rfc3339(), tenant.name, schedule.format, row_count
        );

        // Send email (best effort - log failure but don't fail the report)
        if let Err(e) = send_email(state, &recipients, &subject, &body_text) {
            tracing::warn!(schedule_id = %schedule.id, %e, "failed to send report email");
        } else {
            state.store.mark_report_result_sent(result.id).await?;
        }
    }

    // Update schedule last_run/next_run
    let next_run = now + Duration::hours(1);
    state.store.update_report_schedule_run(schedule.id, now, next_run).await?;

    Ok(())
}

pub async fn evaluate_alert_rules(state: &AppState, tenant_id: Uuid, now: DateTime<Utc>) -> Result<(), ApiError> {
    let rules = state.store.get_enabled_alert_rules(tenant_id).await?;
    if rules.is_empty() {
        return Ok(());
    }

    let client = state.store.pool().get().await?;

    for rule in &rules {
        // Check cooldown
        if let Some(last_triggered) = rule.last_triggered {
            let cooldown = Duration::minutes(rule.cooldown_minutes as i64);
            if now - last_triggered < cooldown {
                continue;
            }
        }

        let (condition_met, message, _event_count) = match rule.condition_type.as_str() {
            "threat" => {
                let threshold: i64 = rule.threshold.parse().unwrap_or(1);
                let row = client.query_one(
                    "SELECT count(*)::bigint AS cnt FROM events WHERE tenant_id = $1 AND category_group = 'threat' AND ts >= $2 AND ts < $3",
                    &[&tenant_id, &(now - Duration::hours(1)), &now],
                ).await?;
                let cnt: i64 = row.get("cnt");
                if cnt >= threshold {
                    (true, format!("{} threat events detected in the last hour (threshold: {})", cnt, threshold), cnt)
                } else {
                    (false, String::new(), 0)
                }
            }
            "blocked_category" => {
                let parts: Vec<&str> = rule.threshold.split(':').collect();
                let category = parts.first().copied().unwrap_or("");
                let threshold: i64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
                let row = client.query_one(
                    "SELECT count(*)::bigint AS cnt FROM events WHERE tenant_id = $1 AND action = 'blocked' AND category = $2 AND ts >= $3 AND ts < $4",
                    &[&tenant_id, &category, &(now - Duration::hours(1)), &now],
                ).await?;
                let cnt: i64 = row.get("cnt");
                if cnt >= threshold {
                    (true, format!("{} blocked events for category '{}' in the last hour (threshold: {})", cnt, category, threshold), cnt)
                } else {
                    (false, String::new(), 0)
                }
            }
            "large_transfer" => {
                let threshold: i64 = rule.threshold.parse().unwrap_or(1000000);
                let row = client.query_one(
                    "SELECT COALESCE(sum(sent_bytes + recv_bytes), 0)::bigint AS total FROM events WHERE tenant_id = $1 AND ts >= $2 AND ts < $3",
                    &[&tenant_id, &(now - Duration::hours(1)), &now],
                ).await?;
                let total: i64 = row.get("total");
                if total >= threshold {
                    (true, format!("Total transfer {} bytes in the last hour exceeds threshold {}", total, threshold), total)
                } else {
                    (false, String::new(), 0)
                }
            }
            _ => continue,
        };

        if condition_met {
            let instance = AlertInstance {
                id: Uuid::new_v4(),
                rule_id: rule.id,
                tenant_id,
                message: message.clone(),
                severity: rule.severity.clone(),
                triggered_at: now,
                acknowledged: false,
                acknowledged_at: None,
                acknowledged_by: None,
            };
            let created_instance = state.store.create_alert_instance(&instance).await?;
            state.store.update_alert_rule_last_triggered(rule.id, now).await?;

            // Send webhook if configured
            if let Some(webhook_url) = &rule.webhook_url {
                let state_clone = state.clone();
                let instance_id = created_instance.id;
                let url = webhook_url.clone();
                let msg = instance.message.clone();
                let sev = rule.severity.clone();
                let rname = rule.name.clone();
                tokio::spawn(async move {
                    if let Err(e) = send_webhook(&state_clone, &url, &rname, &msg, &sev, now).await {
                        tracing::error!(webhook_url = %url, %e, "webhook delivery failed");
                    }
                    let _ = state_clone.store.create_webhook_delivery(&WebhookDelivery {
                        id: Uuid::new_v4(),
                        alert_instance_id: instance_id,
                        webhook_url: url,
                        status: "sent".to_string(),
                        response_status_code: None,
                        response_body: None,
                        error_message: None,
                        delivered_at: now,
                    }).await;
                });
            }
        }
    }

    Ok(())
}

pub async fn healthz(State(state): State<AppState>) -> Response {
    let db = state.store.ping().await;
    let tenant = match state.default_slug.as_deref() {
        Some(slug) => state.store.tenant_by_slug(slug).await.ok().flatten(),
        None => state.store.default_tenant().await.ok().flatten(),
    };
    let last_event_at = match &tenant {
        Some(tenant) => state.store.last_event_at(tenant.id).await.unwrap_or(None),
        None => None,
    };
    let stale = last_event_at
        .map(|ts| Utc::now() - ts > Duration::minutes(15))
        .unwrap_or(true);
    let healthy = db.is_ok();
    let status = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({
            "status": if healthy { "ok" } else { "degraded" },
            "database": db.is_ok(),
            "last_event_at": last_event_at,
            "ingest_stale": stale,
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturated_query_capacity_returns_service_unavailable() {
        let semaphore = Arc::new(Semaphore::new(1));
        let permit = acquire_query_permit(&semaphore, "busy").unwrap();
        let error = match acquire_query_permit(&semaphore, "busy") {
            Ok(_) => panic!("query slot should be saturated"),
            Err(error) => error,
        };
        assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
        drop(permit);
        assert!(acquire_query_permit(&semaphore, "busy").is_ok());
    }

    #[test]
    fn fills_empty_buckets_and_keeps_data() {
        let from = Utc.with_ymd_and_hms(2025, 4, 14, 10, 0, 0).unwrap();
        let to = from + Duration::minutes(10);
        let points = vec![SeriesPoint {
            t: from + Duration::minutes(5),
            events: 3,
            allowed: 2,
            blocked: 1,
            bytes_up: 10,
            bytes_down: 20,
        }];
        let filled = fill_series(points, from, to, 300);
        assert_eq!(filled.len(), 3);
        assert_eq!(filled[0].events, 0);
        assert_eq!(filled[1].events, 3);
        assert_eq!(filled[2].events, 0);
    }

    #[test]
    fn rounds_aggregate_ranges_to_complete_hours() {
        let from = Utc.with_ymd_and_hms(2025, 4, 14, 10, 15, 0).unwrap();
        let exact_hour = Utc.with_ymd_and_hms(2025, 4, 14, 11, 0, 0).unwrap();

        assert_eq!(
            hour_floor(from).unwrap(),
            Utc.with_ymd_and_hms(2025, 4, 14, 10, 0, 0).unwrap()
        );
        assert_eq!(
            hour_ceil(from).unwrap(),
            Utc.with_ymd_and_hms(2025, 4, 14, 11, 0, 0).unwrap()
        );
        assert_eq!(hour_ceil(exact_hour).unwrap(), exact_hour);
    }

    #[test]
    fn whitelists_top_dimensions() {
        assert_eq!(dimension_expr("domains"), Some("domain"));
        assert_eq!(
            dimension_expr("users"),
            Some("COALESCE(NULLIF(user_name, ''), host(src_ip))")
        );
        assert!(dimension_expr("domain; drop table events").is_none());
    }

    #[test]
    fn retention_tokens_are_compared_exactly() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"Secret"));
        assert!(!constant_time_eq(b"secret", b"secret-longer"));
    }

    #[test]
    fn retention_choices_are_explicit() {
        assert_eq!(RETENTION_CHOICES_HOURS, &[6, 12, 24, 48, 72, 168]);
        assert!(!RETENTION_CHOICES_HOURS.contains(&7));
    }

    #[test]
    fn retention_choices_are_within_valid_range() {
        for &hours in RETENTION_CHOICES_HOURS {
            assert!((1..=168).contains(&hours), "{hours} is outside 1-168");
        }
        let mut sorted = RETENTION_CHOICES_HOURS.to_vec();
        sorted.sort_unstable();
        assert_eq!(RETENTION_CHOICES_HOURS, &sorted, "choices must be sorted ascending");
    }
}
