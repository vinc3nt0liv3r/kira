use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod, Runtime};
use futures_util::SinkExt;
use sfos_core::Event;
use tokio_postgres::NoTls;
use uuid::Uuid;

const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../migrations/0001_init.sql")),
    (
        "0002_con_event",
        include_str!("../migrations/0002_con_event.sql"),
    ),
    (
        "0003_device_serial",
        include_str!("../migrations/0003_device_serial.sql"),
    ),
    (
        "0004_event_rollups",
        include_str!("../migrations/0004_event_rollups.sql"),
    ),
    (
        "0005_empty_rollups_ready",
        include_str!("../migrations/0005_empty_rollups_ready.sql"),
    ),
    (
        "0006_http_category_type",
        include_str!("../migrations/0006_http_category_type.sql"),
    ),
    (
        "0007_device_activity",
        include_str!("../migrations/0007_device_activity.sql"),
    ),
    (
        "0008_global_retention",
        include_str!("../migrations/0008_global_retention.sql"),
    ),
    (
        "0009_report_schedules",
        include_str!("../migrations/0009_report_schedules.sql"),
    ),
];
const MIGRATION_LOCK: i64 = 0x5346_4f53; // "SFOS"

#[derive(Debug, Clone)]
pub struct Tenant {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub is_default: bool,
    pub retention_days: i32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReportSchedule {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub cron_schedule: String,
    pub format: String,
    pub recipients: serde_json::Value,
    pub filters: serde_json::Value,
    pub enabled: bool,
    pub last_run: Option<chrono::DateTime<Utc>>,
    pub next_run: Option<chrono::DateTime<Utc>>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReportResult {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub status: String,
    pub file_path: Option<String>,
    pub row_count: i64,
    pub error_message: Option<String>,
    pub generated_at: chrono::DateTime<Utc>,
    pub sent_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AlertRule {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub condition_type: String,
    pub threshold: String,
    pub severity: String,
    pub cooldown_minutes: i32,
    pub enabled: bool,
    pub last_triggered: Option<chrono::DateTime<Utc>>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AlertInstance {
    pub id: Uuid,
    pub rule_id: Uuid,
    pub tenant_id: Uuid,
    pub message: String,
    pub severity: String,
    pub triggered_at: chrono::DateTime<Utc>,
    pub acknowledged: bool,
    pub acknowledged_at: Option<chrono::DateTime<Utc>>,
    pub acknowledged_by: Option<String>,
}

#[derive(Clone)]
pub struct Store {
    pool: Pool,
}

impl Store {
    pub fn connect(database_url: &str, max_size: usize) -> Result<Self> {
        Self::connect_with_password(database_url, None, max_size)
    }

    pub fn connect_with_password(
        database_url: &str,
        password: Option<&str>,
        max_size: usize,
    ) -> Result<Self> {
        Self::connect_with_password_and_options(database_url, password, max_size, None)
    }

    pub fn connect_with_password_and_options(
        database_url: &str,
        password: Option<&str>,
        max_size: usize,
        options: Option<&str>,
    ) -> Result<Self> {
        let mut pg_config = connection_config(database_url, password)?;
        if let Some(options) = options {
            pg_config.options(options);
        }
        let manager = Manager::from_config(
            pg_config,
            NoTls,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let pool = Pool::builder(manager)
            .max_size(max_size.max(1))
            .runtime(Runtime::Tokio1)
            .build()
            .context("failed to build postgres pool")?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    pub async fn migrate(&self) -> Result<()> {
        let mut client = self.pool.get().await.context("postgres unavailable")?;
        // Serialize migration across every replica *before* any DDL: concurrent
        // `CREATE TABLE IF NOT EXISTS` still races in the system catalogs.
        client
            .execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK])
            .await?;
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS schema_migrations (
                    version    text PRIMARY KEY,
                    applied_at timestamptz NOT NULL DEFAULT now()
                )",
            )
            .await?;

        let result = async {
            for (version, sql) in MIGRATIONS {
                let applied = client
                    .query_one(
                        "SELECT EXISTS (SELECT 1 FROM schema_migrations WHERE version = $1)",
                        &[&version],
                    )
                    .await?
                    .get::<_, bool>(0);
                if applied {
                    continue;
                }
                tracing::info!(version, "applying migration");
                let tx = client.transaction().await?;
                tx.batch_execute(sql).await?;
                tx.execute(
                    "INSERT INTO schema_migrations (version) VALUES ($1)",
                    &[&version],
                )
                .await?;
                tx.commit().await?;
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;

        let _ = client
            .execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK])
            .await;
        result
    }

    pub async fn initialize_global_retention(&self, retention_hours: i64) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "INSERT INTO global_retention (singleton, retention_hours)
                 VALUES (true, $1)
                 ON CONFLICT (singleton) DO NOTHING",
                &[&(retention_hours.clamp(1, 168) as i32)],
            )
            .await?;
        Ok(())
    }

    pub async fn global_retention_hours(&self) -> Result<i64> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                "SELECT retention_hours FROM global_retention WHERE singleton",
                &[],
            )
            .await?;
        Ok(i64::from(row.get::<_, i32>(0)))
    }

    pub async fn set_global_retention_hours(&self, retention_hours: i64) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE global_retention
                 SET retention_hours = $1, updated_at = now()
                 WHERE singleton",
                &[&(retention_hours as i32)],
            )
            .await?;
        Ok(())
    }

    pub async fn retention_storage_estimate(&self) -> Result<(i64, i64, Option<i64>)> {
        let client = self.pool.get().await?;
        let database_bytes: i64 = client
            .query_one("SELECT pg_database_size(current_database())::bigint", &[])
            .await?
            .get(0);
        let now = Utc::now();
        let rows = client
            .query(
                "SELECT c.relname, c.reltuples, pg_total_relation_size(c.oid)::bigint
                 FROM pg_inherits i
                 JOIN pg_class p ON p.oid = i.inhparent
                 JOIN pg_class c ON c.oid = i.inhrelid
                 WHERE p.relname = 'events'
                   AND p.relnamespace = 'public'::regnamespace
                   AND c.relname ~ '^events_[0-9]{10}$'",
                &[],
            )
            .await?;
        let mut current_raw_bytes = 0i64;
        let mut recent_bytes = 0i64;
        let mut first_observed_hour = None;
        let estimate_cutoff = now - chrono::Duration::hours(168);
        for row in rows {
            let name: String = row.get(0);
            let Some(start) = hourly_partition_start(&name) else {
                continue;
            };
            let end = start + chrono::Duration::hours(1);
            let partition_bytes: i64 = row.get(2);
            if start <= now {
                current_raw_bytes += partition_bytes;
            }
            if end <= estimate_cutoff || start > now {
                continue;
            }
            let partition_rows = f64::from(row.get::<_, f32>(1)).max(0.0);
            if partition_rows > 0.0 {
                recent_bytes += partition_bytes;
                first_observed_hour = Some(
                    first_observed_hour.map_or(start, |first: DateTime<Utc>| first.min(start)),
                );
            }
        }
        let bytes_per_hour = first_observed_hour.map(|first| {
            let observed_hours = ((now - first).num_hours() + 1).clamp(1, 168) as f64;
            (recent_bytes as f64 / observed_hours)
                .max(0.0)
                .round()
                .min(i64::MAX as f64) as i64
        });
        Ok((database_bytes, current_raw_bytes, bytes_per_hour))
    }

    pub async fn list_tenants(&self) -> Result<Vec<Tenant>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, slug, name, is_default, retention_days FROM tenants ORDER BY created_at",
                &[],
            )
            .await?;
        Ok(rows.iter().map(row_to_tenant).collect())
    }

    pub async fn tenant_by_slug(&self, slug: &str) -> Result<Option<Tenant>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "SELECT id, slug, name, is_default, retention_days FROM tenants WHERE slug = $1",
                &[&slug],
            )
            .await?;
        Ok(row.as_ref().map(row_to_tenant))
    }

    pub async fn default_tenant(&self) -> Result<Option<Tenant>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "SELECT id, slug, name, is_default, retention_days FROM tenants
                 ORDER BY is_default DESC, created_at LIMIT 1",
                &[],
            )
            .await?;
        Ok(row.as_ref().map(row_to_tenant))
    }

    pub async fn rollups_ready(&self) -> Result<bool> {
        let client = self.pool.get().await?;
        let ready = client
            .query_opt(
                "SELECT history_ready FROM event_rollup_state WHERE singleton",
                &[],
            )
            .await?
            .map(|row| row.get(0))
            .unwrap_or(false);
        Ok(ready)
    }

    /// Idempotently create a tenant and make it the default when no default
    /// exists yet.
    pub async fn ensure_tenant(&self, slug: &str, name: &str) -> Result<Tenant> {
        let client = self.pool.get().await?;
        client
            .execute(
                "INSERT INTO tenants (slug, name) VALUES ($1, $2) ON CONFLICT (slug) DO NOTHING",
                &[&slug, &name],
            )
            .await
            .with_context(|| format!("invalid tenant slug {slug:?}"))?;
        client
            .execute(
                "UPDATE tenants SET is_default = true
                 WHERE slug = $1 AND NOT EXISTS (SELECT 1 FROM tenants WHERE is_default)",
                &[&slug],
            )
            .await?;
        self.tenant_by_slug(slug)
            .await?
            .with_context(|| format!("tenant {slug} missing after insert"))
    }

    pub async fn copy_events(&self, tenant_id: Uuid, events: &[Event]) -> Result<u64> {
        if events.is_empty() {
            return Ok(0);
        }
        let mut client = self.pool.get().await?;
        let starts = std::time::Instant::now();
        let tx = client.transaction().await?;
        tx.batch_execute(&format!(
            "CREATE TEMP TABLE event_batch ON COMMIT DROP AS
             SELECT {} FROM events WITH NO DATA",
            crate::copy::COLUMNS.join(", ")
        ))
        .await?;
        let sink = tx
            .copy_in(crate::copy::copy_statement_for("event_batch").as_str())
            .await?;
        futures_util::pin_mut!(sink);

        let mut buf = String::with_capacity(events.len() * 512);
        for event in events {
            crate::copy::encode_row(tenant_id, event, &mut buf);
        }
        sink.send(bytes::Bytes::from(buf)).await?;
        let copied = sink.finish().await?;
        tx.execute(
            &format!(
                "INSERT INTO events ({})
                 SELECT {} FROM event_batch",
                crate::copy::COLUMNS.join(", "),
                crate::copy::COLUMNS.join(", ")
            ),
            &[],
        )
        .await?;
        tx.execute(Self::ROLLUP_BATCH_SQL, &[]).await?;
        tx.execute(Self::DEVICE_ACTIVITY_BATCH_SQL, &[]).await?;
        tx.commit().await?;
        tracing::debug!(
            rows = copied,
            elapsed_ms = starts.elapsed().as_millis(),
            "wrote event batch and rollups"
        );
        Ok(copied)
    }

    const ROLLUP_BATCH_SQL: &str = r#"
    WITH overview AS (
        SELECT tenant_id,
               to_timestamp(floor(extract(epoch FROM ts) / 3600) * 3600) AS bucket,
               count(*)::bigint AS events,
               count(*) FILTER (WHERE action = 'allowed'
                   AND COALESCE(con_event, '') <> 'Start')::bigint AS allowed,
               count(*) FILTER (WHERE action = 'blocked'
                   AND COALESCE(con_event, '') <> 'Start')::bigint AS blocked,
               count(*) FILTER (WHERE category_group = 'web'
                   AND COALESCE(con_event, '') <> 'Start')::bigint AS web_requests,
               count(*) FILTER (WHERE category_group = 'threat')::bigint AS threats,
               COALESCE(sum(sent_bytes), 0)::bigint AS bytes_up,
               COALESCE(sum(recv_bytes), 0)::bigint AS bytes_down
        FROM event_batch
        GROUP BY tenant_id, 2
    ), dimensions AS (
        SELECT e.tenant_id,
               to_timestamp(floor(extract(epoch FROM e.ts) / 3600) * 3600) AS bucket,
               d.dimension,
               d.key,
               count(*) FILTER (WHERE COALESCE(e.con_event, '') <> 'Start')::bigint AS events,
               count(*) FILTER (WHERE e.action = 'allowed'
                   AND COALESCE(e.con_event, '') <> 'Start')::bigint AS allowed,
               count(*) FILTER (WHERE e.action = 'blocked'
                   AND COALESCE(e.con_event, '') <> 'Start')::bigint AS blocked,
               count(*) FILTER (WHERE e.category_group = 'web'
                   AND COALESCE(e.con_event, '') <> 'Start')::bigint AS web_requests,
               count(*) FILTER (WHERE e.category_group = 'threat')::bigint AS threats,
               COALESCE(sum(e.sent_bytes) FILTER (WHERE COALESCE(e.con_event, '') <> 'Start'), 0)::bigint AS bytes_up,
               COALESCE(sum(e.recv_bytes) FILTER (WHERE COALESCE(e.con_event, '') <> 'Start'), 0)::bigint AS bytes_down
        FROM event_batch e
        CROSS JOIN LATERAL (
            VALUES
                ('users', COALESCE(NULLIF(e.user_name, ''), host(e.src_ip))),
                ('domains', NULLIF(e.domain, '')),
                ('categories', NULLIF(e.category, '')),
                ('policies', NULLIF(e.fw_rule_name, '')),
                ('threats', CASE WHEN e.category_group = 'threat'
                    THEN COALESCE(NULLIF(e.threat_name, ''), NULLIF(e.reason, '')) END),
                ('applications', NULLIF(e.application, '')),
                ('countries', NULLIF(e.dst_country, '')),
                ('devices', COALESCE(NULLIF(e.device_name, ''), NULLIF(e.device_id, ''))),
                ('destinations', 'Website: ' || NULLIF(e.domain, '')),
                ('destinations', 'Application: ' || NULLIF(e.application, ''))
        ) AS d(dimension, key)
        WHERE d.key IS NOT NULL AND d.key <> ''
        GROUP BY e.tenant_id, 2, d.dimension, d.key
    ), all_rollups AS (
        SELECT tenant_id, bucket, 'overview'::text AS dimension, ''::text AS key,
               events, allowed, blocked, web_requests, threats, bytes_up, bytes_down
        FROM overview
        UNION ALL
        SELECT tenant_id, bucket, dimension, key, events, allowed, blocked,
               web_requests, threats, bytes_up, bytes_down
        FROM dimensions
    )
    INSERT INTO event_rollups (
        tenant_id, bucket, dimension, key, events, allowed, blocked,
        web_requests, threats, bytes_up, bytes_down
    )
    SELECT tenant_id, bucket, dimension, key, events, allowed, blocked,
           web_requests, threats, bytes_up, bytes_down
    FROM all_rollups
    ON CONFLICT (tenant_id, bucket, dimension, key) DO UPDATE SET
        events = event_rollups.events + EXCLUDED.events,
        allowed = event_rollups.allowed + EXCLUDED.allowed,
        blocked = event_rollups.blocked + EXCLUDED.blocked,
        web_requests = event_rollups.web_requests + EXCLUDED.web_requests,
        threats = event_rollups.threats + EXCLUDED.threats,
        bytes_up = event_rollups.bytes_up + EXCLUDED.bytes_up,
        bytes_down = event_rollups.bytes_down + EXCLUDED.bytes_down
    "#;

    const DEVICE_ACTIVITY_BATCH_SQL: &str = r#"
    INSERT INTO device_activity (
        tenant_id, bucket, device_key, name, device_id, model, firmware, last_seen, events
    )
    SELECT tenant_id,
           date_trunc('minute', ts) AS bucket,
           COALESCE(NULLIF(device_id, ''), NULLIF(device_name, ''), 'unknown') AS device_key,
           COALESCE(NULLIF(device_name, ''), NULLIF(device_id, ''), 'unknown') AS name,
           NULLIF(device_id, ''),
           max(attrs->>'device_model'),
           max(attrs->>'firmware_version'),
           max(ts),
           count(*)::bigint
    FROM event_batch
    GROUP BY tenant_id, date_trunc('minute', ts),
             COALESCE(NULLIF(device_id, ''), NULLIF(device_name, ''), 'unknown'),
             COALESCE(NULLIF(device_name, ''), NULLIF(device_id, ''), 'unknown'),
             NULLIF(device_id, '')
    ON CONFLICT (tenant_id, bucket, device_key) DO UPDATE SET
        name = EXCLUDED.name,
        device_id = COALESCE(EXCLUDED.device_id, device_activity.device_id),
        model = COALESCE(EXCLUDED.model, device_activity.model),
        firmware = COALESCE(EXCLUDED.firmware, device_activity.firmware),
        last_seen = GREATEST(device_activity.last_seen, EXCLUDED.last_seen),
        events = device_activity.events + EXCLUDED.events
    "#;

    /// Create hourly partitions covering the previous hour through the requested horizon.
    pub async fn ensure_partitions(&self, hours_ahead: i64) -> Result<u64> {
        let client = self.pool.get().await?;
        let legacy_daily_partitions: bool = client
            .query_one(
                "SELECT EXISTS (
                    SELECT 1
                    FROM pg_inherits i
                    JOIN pg_class c ON c.oid = i.inhrelid
                    JOIN pg_class p ON p.oid = i.inhparent
                    WHERE p.oid = 'events'::regclass
                      AND c.relname ~ '^events_[0-9]{8}$'
                )",
                &[],
            )
            .await?
            .get(0);
        anyhow::ensure!(
            !legacy_daily_partitions,
            "daily event partitions are present; stop API and ingest and run scripts/migrate-hourly-retention.sh before enabling hourly retention"
        );

        let now = Utc::now();
        let current_hour: DateTime<Utc> = now
            .date_naive()
            .and_hms_opt(now.hour(), 0, 0)
            .context("failed to calculate current UTC hour")?
            .and_utc();
        let mut created = 0u64;
        for offset in -1..=hours_ahead.max(0) {
            let start = current_hour + chrono::Duration::hours(offset);
            let end = start + chrono::Duration::hours(1);
            let name = format!("events_{}", start.format("%Y%m%d%H"));
            let exists: bool = client
                .query_one("SELECT to_regclass($1) IS NOT NULL", &[&name])
                .await?
                .get(0);
            if exists {
                continue;
            }
            let sql = format!(
                "CREATE TABLE {name} PARTITION OF events FOR VALUES FROM ('{start}') TO ('{end}')",
                start = start.format("%Y-%m-%d %H:%M:%S+00"),
                end = end.format("%Y-%m-%d %H:%M:%S+00"),
            );
            client.batch_execute(&sql).await?;
            created += 1;
        }
        if created > 0 {
            tracing::info!(created, "created event partitions");
        }
        Ok(created)
    }

    /// Refresh rollups and drop hourly partitions that have fully aged out.
    /// The DEFAULT partition is never dropped.
    pub async fn drop_old_partitions(&self, retention_hours: i64) -> Result<u64> {
        let client = self.pool.get().await?;
        let cutoff = Utc::now() - chrono::Duration::hours(retention_hours.max(1));
        let rows = client
            .query(
                "SELECT c.relname
                 FROM pg_inherits i
                 JOIN pg_class c ON c.oid = i.inhrelid
                 JOIN pg_class p ON p.oid = i.inhparent
                 WHERE p.relname = 'events' AND p.relnamespace = 'public'::regnamespace",
                &[],
            )
            .await?;
        let mut dropped = 0u64;
        for row in rows {
            let name: String = row.get(0);
            let Some(suffix) = name.strip_prefix("events_") else {
                continue;
            };
            let Some(start) = hourly_partition_start(&name) else {
                if suffix.len() == 8 && suffix.bytes().all(|b| b.is_ascii_digit()) {
                    anyhow::bail!(
                        "daily event partition {name} is present; run scripts/migrate-hourly-retention.sh before enabling hourly retention"
                    );
                }
                continue;
            };
            let end = start + chrono::Duration::hours(1);
            if end > cutoff {
                continue;
            }
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            client
                .execute("SELECT sfos_refresh_rollups($1, $2)", &[&start, &end])
                .await?;
            client
                .batch_execute(&format!("DROP TABLE IF EXISTS \"{name}\""))
                .await?;
            tracing::info!(partition = %name, "dropped expired partition");
            dropped += 1;
        }
        Ok(dropped)
    }

    pub async fn drop_old_device_activity(&self) -> Result<u64> {
        let client = self.pool.get().await?;
        let deleted = client
            .execute(
                "DELETE FROM device_activity
                 WHERE bucket < date_trunc('minute', now()) - interval '10 minutes'",
                &[],
            )
            .await?;
        Ok(deleted)
    }

    // Report schedule CRUD
    pub async fn create_report_schedule(&self, schedule: &ReportSchedule) -> Result<ReportSchedule> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                "INSERT INTO report_schedules (tenant_id, name, description, cron_schedule, format, recipients, filters, enabled)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 RETURNING id, tenant_id, name, description, cron_schedule, format, recipients, filters, enabled, last_run, next_run, created_at, updated_at",
                &[
                    &schedule.tenant_id,
                    &schedule.name,
                    &schedule.description,
                    &schedule.cron_schedule,
                    &schedule.format,
                    &schedule.recipients,
                    &schedule.filters,
                    &schedule.enabled,
                ],
            )
            .await?;
        Ok(ReportSchedule::from(&row))
    }

    pub async fn list_report_schedules(&self, tenant_id: Uuid) -> Result<Vec<ReportSchedule>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, tenant_id, name, description, cron_schedule, format, recipients, filters, enabled, last_run, next_run, created_at, updated_at
                 FROM report_schedules WHERE tenant_id = $1 ORDER BY created_at DESC",
                &[&tenant_id],
            )
            .await?;
        Ok(rows.iter().map(|row| ReportSchedule::from(row)).collect())
    }

    pub async fn update_report_schedule(&self, id: Uuid, schedule: &ReportSchedule) -> Result<Option<ReportSchedule>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "UPDATE report_schedules
                 SET name = $2, description = $3, cron_schedule = $4, format = $5, recipients = $6, filters = $7, enabled = $8, updated_at = now()
                 WHERE id = $1
                 RETURNING id, tenant_id, name, description, cron_schedule, format, recipients, filters, enabled, last_run, next_run, created_at, updated_at",
                &[
                    &id,
                    &schedule.name,
                    &schedule.description,
                    &schedule.cron_schedule,
                    &schedule.format,
                    &schedule.recipients,
                    &schedule.filters,
                    &schedule.enabled,
                ],
            )
            .await?;
        Ok(row.map(|row| ReportSchedule::from(&row)))
    }

    pub async fn delete_report_schedule(&self, id: Uuid) -> Result<bool> {
        let client = self.pool.get().await?;
        let deleted = client
            .execute("DELETE FROM report_schedules WHERE id = $1", &[&id])
            .await?;
        Ok(deleted > 0)
    }

    pub async fn get_due_report_schedules(&self, now: chrono::DateTime<Utc>) -> Result<Vec<ReportSchedule>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, tenant_id, name, description, cron_schedule, format, recipients, filters, enabled, last_run, next_run, created_at, updated_at
                 FROM report_schedules
                 WHERE enabled AND (next_run IS NULL OR next_run <= $1)",
                &[&now],
            )
            .await?;
        Ok(rows.iter().map(|row| ReportSchedule::from(row)).collect())
    }

    pub async fn update_report_schedule_run(&self, id: Uuid, last_run: chrono::DateTime<Utc>, next_run: chrono::DateTime<Utc>) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE report_schedules SET last_run = $2, next_run = $3, updated_at = now() WHERE id = $1",
                &[&id, &last_run, &next_run],
            )
            .await?;
        Ok(())
    }

    // Report result CRUD
    pub async fn create_report_result(&self, result: &ReportResult) -> Result<ReportResult> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                "INSERT INTO report_results (schedule_id, status, file_path, row_count, error_message)
                 VALUES ($1, $2, $3, $4, $5)
                 RETURNING id, schedule_id, status, file_path, row_count, error_message, generated_at, sent_at",
                &[
                    &result.schedule_id,
                    &result.status,
                    &result.file_path,
                    &result.row_count,
                    &result.error_message,
                ],
            )
            .await?;
        Ok(ReportResult::from(&row))
    }

    pub async fn list_report_results(&self, schedule_id: Uuid) -> Result<Vec<ReportResult>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, schedule_id, status, file_path, row_count, error_message, generated_at, sent_at
                 FROM report_results WHERE schedule_id = $1 ORDER BY generated_at DESC",
                &[&schedule_id],
            )
            .await?;
        Ok(rows.iter().map(|row| ReportResult::from(row)).collect())
    }

    pub async fn mark_report_result_sent(&self, id: Uuid) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE report_results SET status = 'sent', sent_at = now() WHERE id = $1",
                &[&id],
            )
            .await?;
        Ok(())
    }

    // Alert rule CRUD
    pub async fn create_alert_rule(&self, rule: &AlertRule) -> Result<AlertRule> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                "INSERT INTO alert_rules (tenant_id, name, description, condition_type, threshold, severity, cooldown_minutes, enabled)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 RETURNING id, tenant_id, name, description, condition_type, threshold, severity, cooldown_minutes, enabled, last_triggered, created_at, updated_at",
                &[
                    &rule.tenant_id,
                    &rule.name,
                    &rule.description,
                    &rule.condition_type,
                    &rule.threshold,
                    &rule.severity,
                    &rule.cooldown_minutes,
                    &rule.enabled,
                ],
            )
            .await?;
        Ok(AlertRule::from(&row))
    }

    pub async fn list_alert_rules(&self, tenant_id: Uuid) -> Result<Vec<AlertRule>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, tenant_id, name, description, condition_type, threshold, severity, cooldown_minutes, enabled, last_triggered, created_at, updated_at
                 FROM alert_rules WHERE tenant_id = $1 ORDER BY created_at DESC",
                &[&tenant_id],
            )
            .await?;
        Ok(rows.iter().map(|row| AlertRule::from(row)).collect())
    }

    pub async fn update_alert_rule(&self, id: Uuid, rule: &AlertRule) -> Result<Option<AlertRule>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "UPDATE alert_rules
                 SET name = $2, description = $3, condition_type = $4, threshold = $5, severity = $6, cooldown_minutes = $7, enabled = $8, updated_at = now()
                 WHERE id = $1
                 RETURNING id, tenant_id, name, description, condition_type, threshold, severity, cooldown_minutes, enabled, last_triggered, created_at, updated_at",
                &[
                    &id,
                    &rule.name,
                    &rule.description,
                    &rule.condition_type,
                    &rule.threshold,
                    &rule.severity,
                    &rule.cooldown_minutes,
                    &rule.enabled,
                ],
            )
            .await?;
        Ok(row.map(|row| AlertRule::from(&row)))
    }

    pub async fn delete_alert_rule(&self, id: Uuid) -> Result<bool> {
        let client = self.pool.get().await?;
        let deleted = client.execute("DELETE FROM alert_rules WHERE id = $1", &[&id]).await?;
        Ok(deleted > 0)
    }

    pub async fn get_enabled_alert_rules(&self, tenant_id: Uuid) -> Result<Vec<AlertRule>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                "SELECT id, tenant_id, name, description, condition_type, threshold, severity, cooldown_minutes, enabled, last_triggered, created_at, updated_at
                 FROM alert_rules WHERE tenant_id = $1 AND enabled",
                &[&tenant_id],
            )
            .await?;
        Ok(rows.iter().map(|row| AlertRule::from(row)).collect())
    }

    pub async fn update_alert_rule_last_triggered(&self, id: Uuid, triggered_at: chrono::DateTime<Utc>) -> Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE alert_rules SET last_triggered = $2, updated_at = now() WHERE id = $1",
                &[&id, &triggered_at],
            )
            .await?;
        Ok(())
    }

    // Alert instance CRUD
    pub async fn create_alert_instance(&self, instance: &AlertInstance) -> Result<AlertInstance> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                "INSERT INTO alert_instances (rule_id, tenant_id, message, severity)
                 VALUES ($1, $2, $3, $4)
                 RETURNING id, rule_id, tenant_id, message, severity, triggered_at, acknowledged, acknowledged_at, acknowledged_by",
                &[
                    &instance.rule_id,
                    &instance.tenant_id,
                    &instance.message,
                    &instance.severity,
                ],
            )
            .await?;
        Ok(AlertInstance::from(&row))
    }

    pub async fn list_alert_instances(&self, tenant_id: Uuid, acknowledged: Option<bool>) -> Result<Vec<AlertInstance>> {
        let client = self.pool.get().await?;
        let rows = match acknowledged {
            Some(ack) => client
                .query(
                    "SELECT id, rule_id, tenant_id, message, severity, triggered_at, acknowledged, acknowledged_at, acknowledged_by
                     FROM alert_instances WHERE tenant_id = $1 AND acknowledged = $2 ORDER BY triggered_at DESC",
                    &[&tenant_id, &ack],
                )
                .await?,
            None => client
                .query(
                    "SELECT id, rule_id, tenant_id, message, severity, triggered_at, acknowledged, acknowledged_at, acknowledged_by
                     FROM alert_instances WHERE tenant_id = $1 ORDER BY triggered_at DESC",
                    &[&tenant_id],
                )
                .await?,
        };
        Ok(rows.iter().map(|row| AlertInstance::from(row)).collect())
    }

    pub async fn acknowledge_alert_instance(&self, id: Uuid, acknowledged_by: &str) -> Result<bool> {
        let client = self.pool.get().await?;
        let updated = client
            .execute(
                "UPDATE alert_instances SET acknowledged = true, acknowledged_at = now(), acknowledged_by = $2 WHERE id = $1 AND NOT acknowledged",
                &[&id, &acknowledged_by],
            )
            .await?;
        Ok(updated > 0)
    }

    pub async fn last_event_at(&self, tenant_id: Uuid) -> Result<Option<DateTime<Utc>>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "SELECT max(ts) FROM events WHERE tenant_id = $1
                 AND ts > now() - interval '7 days'",
                &[&tenant_id],
            )
            .await?;
        Ok(row.and_then(|r| r.get::<_, Option<DateTime<Utc>>>(0)))
    }

    pub async fn ping(&self) -> Result<()> {
        let client = self.pool.get().await?;
        client.simple_query("SELECT 1").await?;
        Ok(())
    }
}

fn hourly_partition_start(name: &str) -> Option<DateTime<Utc>> {
    let suffix = name.strip_prefix("events_")?;
    if suffix.len() != 10 || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let date = NaiveDate::parse_from_str(&suffix[..8], "%Y%m%d").ok()?;
    let hour = suffix[8..].parse::<u32>().ok()?;
    date.and_hms_opt(hour, 0, 0).map(|start| start.and_utc())
}

fn connection_config(database_url: &str, password: Option<&str>) -> Result<tokio_postgres::Config> {
    let mut config: tokio_postgres::Config =
        database_url.parse().context("invalid DATABASE_URL")?;
    if let Some(password) = password {
        config.password(password);
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn connection_password_override_accepts_url_reserved_characters() {
        let actual = connection_config(
            "postgres://sfos@db:5432/sfos",
            Some("contains@reserved:characters"),
        )
        .unwrap();
        let mut expected: tokio_postgres::Config = "postgres://sfos@db:5432/sfos".parse().unwrap();
        expected.password("contains@reserved:characters");
        assert_eq!(actual, expected);
    }

    #[test]
    fn parses_hourly_partition_names_into_utc_boundaries() {
        let expected = Utc.with_ymd_and_hms(2026, 10, 21, 7, 0, 0).unwrap();
        assert_eq!(hourly_partition_start("events_2026102107"), Some(expected));
        assert_eq!(hourly_partition_start("events_default"), None);
        assert_eq!(hourly_partition_start("events_20261021"), None);
    }
}

fn row_to_tenant(row: &tokio_postgres::Row) -> Tenant {
    Tenant {
        id: row.get("id"),
        slug: row.get("slug"),
        name: row.get("name"),
        is_default: row.get("is_default"),
        retention_days: row.get("retention_days"),
    }
}

impl From<&tokio_postgres::Row> for ReportSchedule {
    fn from(row: &tokio_postgres::Row) -> ReportSchedule {
        ReportSchedule {
            id: row.get("id"),
            tenant_id: row.get("tenant_id"),
            name: row.get("name"),
            description: row.get("description"),
            cron_schedule: row.get("cron_schedule"),
            format: row.get("format"),
            recipients: row.get("recipients"),
            filters: row.get("filters"),
            enabled: row.get("enabled"),
            last_run: row.get("last_run"),
            next_run: row.get("next_run"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }
    }
}

impl From<&tokio_postgres::Row> for ReportResult {
    fn from(row: &tokio_postgres::Row) -> ReportResult {
        ReportResult {
            id: row.get("id"),
            schedule_id: row.get("schedule_id"),
            status: row.get("status"),
            file_path: row.get("file_path"),
            row_count: row.get("row_count"),
            error_message: row.get("error_message"),
            generated_at: row.get("generated_at"),
            sent_at: row.get("sent_at"),
        }
    }
}

impl From<&tokio_postgres::Row> for AlertRule {
    fn from(row: &tokio_postgres::Row) -> AlertRule {
        AlertRule {
            id: row.get("id"),
            tenant_id: row.get("tenant_id"),
            name: row.get("name"),
            description: row.get("description"),
            condition_type: row.get("condition_type"),
            threshold: row.get("threshold"),
            severity: row.get("severity"),
            cooldown_minutes: row.get("cooldown_minutes"),
            enabled: row.get("enabled"),
            last_triggered: row.get("last_triggered"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }
    }
}

impl From<&tokio_postgres::Row> for AlertInstance {
    fn from(row: &tokio_postgres::Row) -> AlertInstance {
        AlertInstance {
            id: row.get("id"),
            rule_id: row.get("rule_id"),
            tenant_id: row.get("tenant_id"),
            message: row.get("message"),
            severity: row.get("severity"),
            triggered_at: row.get("triggered_at"),
            acknowledged: row.get("acknowledged"),
            acknowledged_at: row.get("acknowledged_at"),
            acknowledged_by: row.get("acknowledged_by"),
        }
    }
}
