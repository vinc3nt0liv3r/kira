use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use anyhow::Context;
use chrono::{DateTime, Duration, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sfos_store::{Store, Tenant};

use crate::query::{RangeQuery, TimeRange};

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub default_slug: Option<String>,
    pub raw_retention_hours: i64,
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
        Self::internal(err)
    }
}

impl From<tokio_postgres::Error> for ApiError {
    fn from(err: tokio_postgres::Error) -> Self {
        Self::internal(err.into())
    }
}

impl From<deadpool_postgres::PoolError> for ApiError {
    fn from(err: deadpool_postgres::PoolError) -> Self {
        Self::internal(err.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
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

    let client = state.store.pool().get().await?;
    let devices = client
        .query(
            "SELECT COALESCE(NULLIF(device_name, ''), NULLIF(device_id, ''), 'unknown') AS name,
                    COALESCE(device_id, attrs->>'device_serial_id') AS device_id,
                    max(attrs->>'device_model') AS model,
                    max(attrs->>'firmware_version') AS firmware,
                    max(ts) AS last_seen,
                    count(*)::bigint AS events
             FROM events
             WHERE tenant_id = $1 AND ts > now() - interval '5 minutes'
             GROUP BY 1, 2
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
        "raw_retention_hours": state.raw_retention_hours,
        "rollups_ready": rollups_ready,
    })))
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

pub async fn top(
    State(state): State<AppState>,
    Query(query): Query<TopQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
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
    let (sql, params): (String, Vec<&(dyn tokio_postgres::types::ToSql + Sync)>) =
        if rollups_ready {
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
                vec![
                    &tenant.id,
                    &dimension,
                    &rollup_from,
                    &rollup_to,
                    &limit,
                ],
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
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant = resolve_tenant(&state, query.tenant.as_deref()).await?;
    let range = resolve_range(&RangeQuery {
        tenant: query.tenant.clone(),
        from: query.from.clone(),
        to: query.to.clone(),
        interval: None,
    })?;
    let cutoff = raw_cutover(Utc::now(), state.raw_retention_hours);
    if range.to <= cutoff {
        return Err(ApiError::gone(format!(
            "raw event details are retained for {} hours",
            state.raw_retention_hours
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

    let client = state.store.pool().get().await?;
    let rows = client
        .query(
            "SELECT id, ts, category_group, action, log_type, log_component, device_name,
                    host(src_ip) AS src_ip, host(dst_ip) AS dst_ip, src_port, dst_port,
                    COALESCE(NULLIF(user_name, ''), host(src_ip)) AS display_user,
                    domain, url, http_method, http_status, category, application,
                    fw_rule_name, sent_bytes, recv_bytes, threat_name, message
             FROM events
             WHERE tenant_id = $1 AND ts >= $2 AND ts < $3
               AND ($4::text IS NULL OR user_name ILIKE '%' || $4 || '%'
                    OR domain ILIKE '%' || $4 || '%'
                    OR url ILIKE '%' || $4 || '%'
                    OR message ILIKE '%' || $4 || '%'
                    OR threat_name ILIKE '%' || $4 || '%'
                    OR fw_rule_name ILIKE '%' || $4 || '%'
                    OR host(src_ip) = $4)
               AND ($5::text IS NULL OR action = $5)
               AND ($6::text IS NULL OR category_group = $6)
             ORDER BY ts DESC, id DESC
             LIMIT $7 OFFSET $8",
            &[&tenant.id, &raw_from, &range.to, &q, &action, &group, &limit, &offset],
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
        "raw_window_hours": state.raw_retention_hours,
        "limit": limit,
        "offset": offset,
        "rows": rows,
    })))
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
}
