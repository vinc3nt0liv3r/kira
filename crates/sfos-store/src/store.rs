use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Timelike, Utc};
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
        let pg_config = connection_config(database_url, password)?;
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
            let Ok(start) = NaiveDateTime::parse_from_str(suffix, "%Y%m%d%H") else {
                if suffix.len() == 8 && suffix.bytes().all(|b| b.is_ascii_digit()) {
                    anyhow::bail!(
                        "daily event partition {name} is present; run scripts/migrate-hourly-retention.sh before enabling hourly retention"
                    );
                }
                continue;
            };
            let start = start.and_utc();
            let end = start + chrono::Duration::hours(1);
            if end > cutoff {
                continue;
            }
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            client
                .execute(
                    "SELECT sfos_refresh_rollups($1, $2)",
                    &[&start, &end],
                )
                .await?;
            client
                .batch_execute(&format!("DROP TABLE IF EXISTS \"{name}\""))
                .await?;
            tracing::info!(partition = %name, "dropped expired partition");
            dropped += 1;
        }
        Ok(dropped)
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

    #[test]
    fn connection_password_override_accepts_url_reserved_characters() {
        let actual = connection_config(
            "postgres://sfos@db:5432/sfos",
            Some("contains@reserved:characters"),
        )
        .unwrap();
        let mut expected: tokio_postgres::Config =
            "postgres://sfos@db:5432/sfos".parse().unwrap();
        expected.password("contains@reserved:characters");
        assert_eq!(actual, expected);
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
