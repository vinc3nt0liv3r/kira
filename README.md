# Kira - The Sophos Reporter

Self-hosted, live reporting for **Sophos Firewall (SFOS)** — a Rust syslog collector, Postgres
storage and a React dashboard. Think of the internet-usage and network-visibility side of FastVue
Reporter, rebuilt as a small service you run inside your own network.

Log data never leaves your server. There is no SaaS, no per-seat fee and no cloud dependency.

---

## What works today (v1)

- **Syslog collection** over UDP and TCP on port `5514`, with optional source allow-listing.
- **SFOS parsing** for firewall, content filtering/web, IPS, anti-virus, ATP, authentication, VPN
  and system log types. Handles both the classic `device="SFW" date=... time=...` envelope and
  the SFOS 21 `<pri>device_name=... timestamp="...+0500"` shape, quoted/escaped values, bare
  values containing spaces, RFC3164/RFC5424 headers, `bytes_sent`/`bytes_received` and
  `app_name`/`app_category` field variants.
- **Connection-aware counts**: SFOS writes firewall connections twice (`con_event=Start` and
  `Stop`); dashboards count each connection once while the event explorer still shows both.
- **Normalized event store** in Postgres: typed columns for users, domains, categories, policies,
  bytes, threats and IPs, plus a JSONB bag for every other vendor field.
- **Live dashboards** (polled every 60 s, one request at a time): identified users, web requests, blocked share,
  upload/download volume, threats, request/bandwidth area charts, top users/destinations/
  categories, security summary and collector health.
- **Event explorer** with search (user, domain, URL, IP, rule, message, threat), decision and log
  group filters, and expandable row details.
- **Tenant-ready**: every row carries a `tenant_id`; source CIDRs can be routed to different sites
  and the dashboard switches between them.
- **Hourly raw retention** (default 6 hours) with long-lived compact rollups.
- **Health & metrics** endpoints for both services.

Not in v1: authentication for the dashboard, scheduled PDF/email reports, alerts, Active Directory
group mapping, Site Clean/YouTube/search-term enrichment. See [Roadmap](#roadmap).

---

## Quick start

Requirements: Docker with Compose v2, and a host reachable by your firewall.

```bash
cp .env.example .env          # set POSTGRES_PASSWORD and TENANT_NAME
docker compose up -d --build
```

Open <http://localhost:8082> (or the port set in `API_PORT`). The dashboard waits for the first
firewall logs; the collector is already listening on `5514/udp` and `5514/tcp`.

Generate demo traffic if you do not have a firewall handy — it is **synthetic** and lands in the
same database as real logs, so remove it afterwards:

```bash
python3 scripts/seed-demo.py --yes --host 127.0.0.1 --port 5514
scripts/reset-data.sh --demo-only            # or --docker when using Compose
```

Demo events use the device name `DEMO-XGS2100`, which makes them easy to spot and purge.

Verify the whole pipeline end to end:

```bash
python3 scripts/smoke-test.py --base-url http://127.0.0.1:8082 --host 127.0.0.1 --port 5514
```

### Without Docker (bundled Postgres)

```bash
scripts/dev.sh start      # bundled Postgres + collector + API, state in .runtime/
scripts/dev.sh status
scripts/dev.sh logs
scripts/dev.sh stop --all
```

That script builds release binaries and the dashboard, keeps the database in `.runtime/pgdata`
(so logs survive restarts) and binds syslog to `0.0.0.0:5514`. To use your own Postgres instead:

```bash
export DATABASE_URL=postgres://user:pass@127.0.0.1:5432/sfos
cargo build --release
./target/release/sfos-api     &   # HTTP API + dashboard on :8080
./target/release/sfos-ingest  &   # syslog on :5514, health on :8081
```

Both binaries run migrations on start, so a fresh database needs nothing else. Build the dashboard
first with `npm --prefix web install && npm --prefix web run build` (the API serves `web/dist`, or
set `WEB_DIR`).

---

## Configure Sophos Firewall

1. **System → Logging → Syslog Servers → Add**
   - Server address: the host running the collector.
   - Port: `5514`, protocol **UDP** (or **TCP** if you prefer delivery confirmation).
   - Format: **Device standard format** (the `key=value` payload this parser expects).
   - Facility: any; it is ignored.
2. **System → Logging → Log Settings** — enable at least *Firewall*, *Content Filtering*,
   *Intrusion Prevention*, *Anti-Virus*, *ATP*, *Authentication* and *VPN*.
3. **User names** — `user_name` is only present when Sophos can attribute a connection to a person.
   Enable authentication (AD/LDAP, STAS/SSO or captive portal) and make sure the relevant firewall
   rules and web-filter policies log it. Without it the user panels show *unidentified*.
4. Confirm data is arriving:

   ```bash
   curl -s http://<reporter>:8081/metrics
   # {"datagrams":812,"lines":812,"parsed":812,"rejected":0,...,"written":812}
   ```

`rejected` should stay at zero. A rising counter usually means another device is sending to the
same port, or the firewall is set to *Legacy format*.

### Time zones

SFOS writes a `timezone="..."` abbreviation per message. Common abbreviations are mapped; if a
device uses one that is not recognised, set `DEFAULT_TZ_OFFSET_MINUTES` to that device's offset
(e.g. `330` for IST) so events land on the right minute.

---

## Architecture

```
 Sophos Firewall ──syslog UDP/TCP :5514──► sfos-ingest ──COPY──► Postgres
                                               │                  │
                                               ├─ parses/normalizes│ hourly partitions,
                                               ├─ batches 5000/2000ms│ hourly rollups/retention
                                               └─ health :8081    │
                                                                  ▼
                                       browser ◄── sfos-api (Axum) ── aggregates on demand
                                                   serves React build
```

- **`crates/sfos-core`** — SFOS key/value tokenizer, timestamp/timezone handling (including
  `timestamp="2026-10-03T15:45:28+0500"`), event normalization, coarse category groups (`web`,
  `threat`, `firewall`, `auth`, `vpn`, `system`, `other`) and allow/block derivation.
- **`crates/sfos-store`** — embedded SQL migrations, tenant helpers, partition management and the
  batched `COPY` writer.
- **`crates/sfos-ingest`** — UDP/TCP listeners, per-source tenant routing, bounded queue, batch
  writer with retries, partition maintenance and a metrics endpoint. `/metrics` reports current
  queue depth/capacity/utilization, high-water depth, queued/written totals, database write errors,
  rejected lines and dropped events.
- **`crates/sfos-api`** — Axum JSON API, static dashboard hosting, gzip, health endpoint.
- **`web/`** — React 19 + TypeScript + Tailwind 4 + Recharts.

Ingest writes are batched (`BATCH_SIZE` events or `BATCH_MS` milliseconds) through Postgres `COPY`
inside a transaction that also updates compact hourly dimension rollups. Events are grouped by
tenant so a single batch can serve several sites.
The default batch window is 2 seconds (or 5,000 events, whichever comes first), limiting commit/WAL
sync frequency while keeping normal ingestion latency low.

### Capacity planning and diagnostics

This Release 0 baseline was measured on a host reporting a 4-vCPU Intel Xeon Silver 4210R,
7.8 GiB RAM, and a 150 GB virtual disk. It is a reproducible reference environment, not a
minimum hardware recommendation.

Run `scripts/diagnose-storage.sh` while the Compose database is running to report database
footprint, retained event count, approximate ingestion rate, storage per event, projected daily
event storage and largest daily partitions. The event count requires a sequential scan and may
take time on a large database; the script is read-only and has a 120-second statement timeout.
Use `docker system df -v` to see Docker volume, image and build-cache usage separately.
Run `scripts/verify-release0.sh` for an isolated clean-install test. It starts a separate Compose
project bound to loopback-only test ports, replays the seven anonymized captured samples plus two
clearly labeled synthetic threat/authentication events, checks raw API rows and hourly dashboard
aggregates, measures a deterministic load, restores a backup into a scratch database, and removes
the test volume. The replay rebases sample timestamps to the current time so the configured
six-hour raw window does not hide them. Use `--keep` to retain the isolated stack for browser
inspection; remove it with the command printed at completion.

API database statements have a 10-second timeout by default (`API_STATEMENT_TIMEOUT_MS`, configurable
from 1 to 120000 ms). Overview/top/event-search requests are limited to at most two concurrent
analytical requests, leaving spare pool capacity for health and metadata calls under the default
pool size. A busy API returns HTTP 503 rather than queuing unlimited expensive work. Query timeouts
also return HTTP 503. Fresh empty databases mark
their rollups ready automatically; populated databases are not marked ready without a verified
backfill, and startup logs a warning when dashboard aggregates may still use raw events. The
`/events?q=` filter remains substring search; timeouts and concurrency limits bound its impact but
do not replace a measured search index.

The repeatable benchmark sends 10,001 synthetic events: 7,000 web, 2,000 firewall, 500 threat,
300 authentication, 200 VPN, and one relayed firewall event. It uses the default 5,000-event /
2-second batching configuration and reports persistence throughput plus partition storage growth
including indexes. On the documented 4-vCPU, 7.8-GiB host, one isolated run persisted all 10,001
events in 3.317 seconds (3,015 events/second), with 5,373,952 bytes of partition growth including
indexes (537.3 bytes/event), a 5,001-event queue high-water mark against 20,000 capacity, and no
drops or database write errors. This is a single controlled run, not a sustained-capacity guarantee.
Record the emitted JSON together with the hardware and actual Compose configuration when refreshing
capacity measurements.

On 2026-10-20, an observed database snapshot contained 1,515,841 events received over 43 minutes.
The database occupied 1.74 GB and its Docker volume 2.65 GB, including 688 MB of WAL. The largest
daily partition occupied 1.73 GB including indexes, or about 1.2 KB per event. The measured
average was about 583 events/second, with peak minutes above 75,000 events. If sustained, that
workload implies roughly 2.5 GB/hour or 56 GiB/day (about 60 GB/day) for event partitions alone;
WAL, backups, and filesystem overhead add to this. Treat this as a measured high-volume sample, not a universal
Sophos rate. This inspection did not find a 14 GB current Compose database; verify the exact
directory or volume before removing anything.

During a later overload, the database held 1,407,431 events (about 1.6 GB including indexes)
received in under an hour. `EXPLAIN` showed dashboard totals and top-category queries using parallel
sequential scans over the active daily partition. The dashboard previously launched six database-
backed requests together every five seconds, repeatedly aggregating those same raw rows. It now
runs dashboard requests sequentially and waits 60 seconds after each completed refresh; the manual
Refresh control still works immediately. This reduces overlapping query pressure but does not
replace the planned compact rollups: raw-event aggregation cost still grows with the selected
time range and event volume. The 1,407,431 accumulated event rows were then truncated while
retaining the schema and default tenant. After a PostgreSQL checkpoint, the database measured
about 8.5 MB and event partitions about 240 KB; the volume still held about 640 MB of reusable WAL.
No backup was made for this cleanup; older Release 0 backup/restore verification covered the
earlier dataset, not these subsequently received events. Never delete files from `pg_wal` manually.

A live check later on 2026-10-20 observed about 1.21 million events in the active partition
(roughly 1.4 GB including indexes), with 50,000–58,000 events in each of several recent full
minutes. That is around 900 events/second and about 1.1 KB/event, or close to 90 GB/day of event
partition growth if sustained. PostgreSQL CPU briefly reached about 285% in one sample, then later
samples were 6–16%; the host had about 5 GB RAM available and 79 GB disk free. The collector
reported zero database errors and drops at the later sample. Its cumulative queued/written counters
temporarily differed by about 13,000 events (the queue capacity is 20,000), so monitor backlog as
well as drop/error counters. This ingest rate, not refresh frequency alone, is the immediate
storage risk; current daily partitions cannot protect a disk from filling within a day.

The same live investigation found a more direct dashboard overload source: `/api/v1/meta` was
grouping the full 24-hour event table by computed device name/ID and sorting the groups. The
frontend polled metadata every 60 seconds without waiting for the previous request to finish.
PostgreSQL showed multiple copies of this query, several minutes old and writing temporary sort
files; cumulative `pg_stat_database.temp_bytes` reached about 71 GB. That is historical temp-file
I/O, not 71 GB of retained database data. The stale queries were cancelled. Metadata now aggregates
only the last five minutes, and the browser schedules the next metadata refresh five minutes after
the previous one completes. This is a protective bound; a small device registry updated during
ingestion is the durable replacement for querying raw events.

The initial overview and top-dimension queries also scanned raw events for the selected range.
They now read only the compact hourly rollups, which are updated in the same transaction as each
ingest batch; event-detail searches continue to use the six-hour raw window. This avoids rescanning
millions of raw rows on dashboard refresh. Aggregate charts use whole-hour buckets; the current
hour reflects events received so far, and partial range edges include their full hour bucket.
Event details retain event-level timestamps.

After clearing that historical event set, a controlled 10,001-event TCP replay on the same host
stored all events without collector database errors or drops. The current-day partition grew by
about 5.1 MiB including indexes (about **536 bytes/event** for this sample mix). Receive timestamps
spanned 10.36 seconds, or roughly **966 events/second**. Seven-request API checks over the resulting
dataset had median response times of 67 ms for overview, 20 ms for top categories, and 5 ms for the
event list. The fixture smoke test also passed all 26 assertions. These figures are specific to
the test mix and one small dataset; repeat this benchmark on the actual deployment before setting
capacity limits.

The pre-clean backup was restored to a scratch database and its event count matched exactly:
**1,515,841 rows**. The compressed archive was 89 MB. The scratch database and temporary backup
were removed after verification. The old event rows were then truncated; the database now has
zero event rows. PostgreSQL reduced relation storage to about 30 MB but retained roughly 1 GB of
reusable WAL segments near its configured 1 GB `max_wal_size`. WAL is not retained event data and
is reused by PostgreSQL; never delete files from `pg_wal` manually.

Every accepted syslog line is stored as an event row. By default, unmodeled fields in JSONB `attrs`
are discarded except `device_model` and `firmware_version`, which feed the device summary. Set
`KEEP_EXTENDED_ATTRIBUTES=true` to retain every unmodeled Sophos field; this increases storage and
may retain vendor data not used by reports. Storage varies substantially with log mix: the historical sample used about 1.2 KB/event while
the controlled replay used about 536 bytes/event. At the historical sample size, a 14 GB event
database would represent roughly 12 million events. Size requirements depend on actual log rate
and fields; measure the deployment before choosing retention.

In the live sample, `attrs` averaged about 794 bytes in sampled rows. It contains vendor fields
that are not modeled as report columns (such as interfaces, MAC addresses, and rule/application
metadata); it is not a second copy of every normalized field. New compact ingestion reduces new
row size but does not rewrite existing rows.

Dashboard identities now use the source IP when a log has no username, and the top-destinations
panel includes both website domains and application names. The parser also maps `http_user_agent`
to its typed field and no longer retains `device_serial_id` a second time in `attrs`.

Raw event rows use a global retention window (default 6 hours, with up to one extra hour at hourly
partition boundaries). It is editable in the dashboard using a server-configured bearer token; it
is not a per-site setting. Before applying a choice, the dashboard shows the current database size
and approximate projected sizes based on recent PostgreSQL partition statistics. The estimate is
not a capacity guarantee. The collector applies a saved change during maintenance within about five
minutes. The collector refreshes each expiring hour's rollups before dropping that raw partition.
Overview and top-dimension reports use hourly rollups across the full selected range; event detail
search is limited to retained raw rows. Rollup refresh and raw-partition removal are separate,
repeatable operations so a failed refresh cannot discard its source partition.

Rollups are retained independently from raw-event details, so trend summaries survive detail
expiry. The catch-all `events_default` partition is not dropped by hourly retention and should be
monitored if logs can arrive with timestamps outside the pre-created partition range. Optional
compressed event-level exports are deferred: they require additional archive storage and a separate
restore workflow; database backup and restore procedures remain the supported recovery path.

### Upgrade an existing daily-partition database

Existing installs still have the original daily partition layout and must be converted before
starting the new collector. Keep the API and ingest services stopped, leave PostgreSQL running,
ensure sufficient free disk for a logical backup and migration work, then run:

```sh
bash scripts/migrate-hourly-retention.sh /absolute/path/to/sfos-before-hourly.dump
```

The script refuses to overwrite a backup, creates a custom-format `pg_dump`, migrates historical
events into hourly rollups, copies the selected recent raw window into new hourly partitions,
checks summarized and retained row counts, and only then replaces the daily raw table. The backup
is intentionally retained. This operation takes an exclusive table lock and can be I/O intensive;
do not run it while the collector or API is active. The cutover was run against the live database
on 2026-10-20 after a 413 MB custom-format backup was created and validated. The transaction
verified the retained raw count and total summarized event count before swapping tables.

Keep these storage sources distinct when investigating disk use:

- PostgreSQL database and WAL in the Compose volume.
- Native PostgreSQL data under `.runtime/pgdata` when using `scripts/dev.sh`.
- Rust build artifacts under `target/`, Docker images, and Docker build cache.

They are separate copies or build artifacts; measuring the database alone does not account for
all project disk usage. At the same inspection, this workspace also had a separate 4.1 GB native
`.runtime` database directory and 1.9 GB of Rust build artifacts. These should not be mistaken for
the Compose database, and should not be deleted until their contents are confirmed disposable.

For a backup, use a non-TTY stream so custom-format dumps remain intact:

```bash
docker compose exec -T db pg_dump -U sfos -Fc sfos > backup.dump
```

Test restoration into a separate scratch database, after confirming there is enough free space
for both the backup file and restored data. The script below creates a temporary custom-format
backup, restores it into a scratch database, compares event counts, then removes only its scratch
database and temporary archive:

```bash
scripts/verify-backup-restore.sh
```

Do not restore over the production database as a test. The scratch restore temporarily duplicates
database storage; on a small disk, perform it on a separate machine or volume.

---

## Repository layout

```
Cargo.toml               Rust workspace
crates/sfos-core/        parser + event model (+ tests)
crates/sfos-store/       schema, migrations, COPY writer (+ tests)
crates/sfos-ingest/      syslog collector binary
crates/sfos-api/         API + dashboard host binary
web/                     React dashboard
migrations live in crates/sfos-store/migrations/
scripts/dev.sh           run the stack locally without Docker
scripts/reset-data.sh    delete all events, or only synthetic DEMO-* events
scripts/seed-demo.py     realistic demo syslog generator (opt-in, tagged DEMO-)
scripts/smoke-test.py    end-to-end API/dashboard smoke test
scripts/diagnose-storage.sh  read-only database size and ingestion-rate report
scripts/verify-samples.py  replay Release 0 samples and verify API/rollup/metrics results
scripts/benchmark-release0.py  deterministic storage and ingestion benchmark
scripts/verify-backup-restore.sh  scratch-database raw/rollup backup-restore verification
scripts/verify-release0.sh  isolated clean-install Release 0 verification
docker-compose.yml       Postgres + API + collector
Dockerfile               multi-stage build
```

---

## Configuration

### Docker Compose host ports

| Variable | Default | Purpose |
| --- | --- | --- |
| `HOST_BIND_IP` | `0.0.0.0` | Host interface for published ports; set to `127.0.0.1` for localhost-only access |
| `API_PORT` | `8082` | Dashboard host port |
| `SYSLOG_PORT` | `5514` | Syslog host port, UDP and TCP |
| `INGEST_HEALTH_PORT` | `8083` | Collector health/metrics host port |

### Collector (`sfos-ingest`)

| Variable | Default | Purpose |
| --- | --- | --- |
| `DATABASE_URL` | *required* | Postgres connection string |
| `DATABASE_PASSWORD` | *(unset)* | Optional password override; useful when the password contains URL-reserved characters |
| `SYSLOG_UDP_ADDR` | `0.0.0.0:5514` | UDP listen address |
| `SYSLOG_TCP_ADDR` | `0.0.0.0:5514` | TCP listen address |
| `HEALTH_ADDR` | `0.0.0.0:8081` | `/healthz` and `/metrics` |
| `TENANT_SLUG` | `default` | Site that receives unmatched sources |
| `TENANT_NAME` | `Default site` | Display name for that site |
| `TENANT_MAP` | *(empty)* | `CIDR=slug,...` routing, longest prefix wins |
| `ALLOWED_SOURCES` | *(empty)* | Only accept logs from these IPs/CIDRs |
| `DEFAULT_TZ_OFFSET_MINUTES` | `0` | Fallback for unknown timezone abbreviations |
| `RETENTION_HOURS` | `6` | Initial global raw retention for a new database; saved setting wins afterward |
| `KEEP_EXTENDED_ATTRIBUTES` | `false` | Keep all unmodeled event attributes instead of only device model/firmware |
| `BATCH_SIZE` | `5000` | Rows per COPY batch |
| `BATCH_MS` | `2000` | Max time before flushing a partial batch |
| `DB_POOL_SIZE` | `8` | Postgres connections |
| `PARTITIONS_AHEAD_HOURS` | `48` | How far ahead hourly partitions are created |
| `RUST_LOG` | `info` | `debug` logs per-batch timings and rejections |

### API (`sfos-api`)

| Variable | Default | Purpose |
| --- | --- | --- |
| `DATABASE_URL` | *required* | Postgres connection string |
| `DATABASE_PASSWORD` | *(unset)* | Optional password override; useful when the password contains URL-reserved characters |
| `API_ADDR` | `0.0.0.0:8080` | Listen address |
| `WEB_DIR` | `web/dist` | Built dashboard assets |
| `TENANT_SLUG` | `default` | Default site for API requests |
| `TENANT_NAME` | `Default site` | Created on first boot if missing |
| `RETENTION_HOURS` | `6` | Initial global raw retention for a new database; saved setting wins afterward |
| `RETENTION_ADMIN_TOKEN` | *(unset)* | Optional bearer token (minimum 32 characters) enabling dashboard retention changes |
| `DB_POOL_SIZE` | `8` | Postgres connections |
| `API_STATEMENT_TIMEOUT_MS` | `10000` | PostgreSQL statement timeout for API connections (prevents long-running queries from blocking the pool) |
| `API_MAX_CONCURRENT_QUERIES` | `4` | Semaphore limiting concurrent expensive dashboard queries to prevent overload. |

### Security

There is **no general dashboard login** in v1. Run the dashboard on a trusted management network,
or put it behind a reverse proxy that terminates TLS and enforces authentication (basic auth, OIDC,
VPN). Do not publish the dashboard port to the internet. The optional retention control uses the
server-side `RETENTION_ADMIN_TOKEN`; the bearer secret is entered only when applying a change and
is not saved in browser storage. Without the token, preview is read-only. The syslog port has no
authentication either — use `ALLOWED_SOURCES` to restrict which addresses may submit logs.

---

## API

All endpoints accept `tenant=<slug>`; when omitted the default site is used. Time bounds accept
RFC3339 (`2026-04-14T00:00:00Z`), epoch seconds, or relative windows (`1h`, `24h`, `7d`).

| Endpoint | Description |
| --- | --- |
| `GET /healthz` | DB check, last event, ingest freshness |
| `GET /api/v1/meta` | Tenants, active site, devices seen in 24 h, last event |
| `GET /api/v1/overview` | Totals + gap-filled timeseries |
| `GET /api/v1/top?dimension=users` | Ranked top-N: `users`, `domains`, `categories`, `policies`, `threats`, `applications`, `countries`, `devices`, `log_types` |
| `GET /api/v1/events` | Newest events with `q` (including category), `action`, `group`, `limit`, `offset` |
| `GET /api/v1/retention` | Global detail-retention setting and approximate storage preview |
| `PUT /api/v1/retention` | Change global detail retention; requires `Authorization: Bearer <RETENTION_ADMIN_TOKEN>` |
| `GET /api/v1/retention` | Global detail-retention setting and approximate storage preview |
| `PUT /api/v1/retention` | Change global detail retention; requires `Authorization: Bearer <RETENTION_ADMIN_TOKEN>` |

```bash
curl -s 'http://localhost:8080/api/v1/overview?from=24h' | jq .totals
curl -s 'http://localhost:8080/api/v1/top?from=7d&dimension=domains&limit=10' | jq '.rows[]'
curl -s 'http://localhost:8080/api/v1/events?from=1h&action=blocked&q=youtube&limit=20' | jq '.rows[0]'
```

`overview` returns one point per bucket, including empty buckets, so charts do not need to fill
gaps. Bucket width is chosen from the range (60 s for 1 h, 900 s for 24 h, 1 d for 30 d) and can be
overridden with `interval=<seconds>`.

Totals and top-N counts exclude `con_event=Start` records, so a connection that Sophos logs twice
is counted once. `events` still returns every raw log line, including the start record.

### Data model

- `tenants` — slug, name, default flag, `retention_days` (reserved for per-tenant retention).
- `global_retention` — persisted global raw-event detail retention setting.
- `events` — one row per accepted syslog message, partitioned by hour on `ts`, primary key
  `(ts, id)`. Includes normalized columns for user, domain, URL, HTTP category and category type,
  application, rule, bytes, threat, IPs, zones, ports and `con_event`, plus `attrs jsonb` with
  unmodeled SFOS fields when `KEEP_EXTENDED_ATTRIBUTES=true`; otherwise only `device_model` and
  `firmware_version` are kept in `attrs`. `http_category` / `category` and
  `http_category_type` / `category_type` are normalized to the same respective columns. The API
  reads legacy category-type values from `attrs` when older rows have not yet been normalized;
  startup does not run a large backfill.
- `events_default` — catch-all partition for timestamps outside the pre-created hours.
- `event_rollups` — compact hourly overview and dimension totals for users/hosts, domains,
  destinations, categories, policies, threats, applications, countries, and devices.
- `device_activity` — short-lived per-minute device counts and last-seen/model metadata used by
  the dashboard's recent-firewall list without aggregating raw events on each refresh.
- Indexes: `(tenant_id, ts)`, `(tenant_id, category_group, ts)` and `(tenant_id, user_name, ts)`.

Retention refreshes rollups and drops complete hourly partitions older than the global retention setting.
Hourly granularity means raw data can remain for up to one extra hour beyond the configured window.

---

## Multi-tenant / MSP use

A single collector can receive from several firewalls and separate their data:

```env
TENANT_MAP=192.0.2.0/24=acme,198.51.100.0/24=branch
TENANT_SLUG=default
```

- Longest prefix wins; unmatched sources fall through to `TENANT_SLUG`.
- Tenants are created automatically the first time a mapped source sends a log.
- Query any site with `?tenant=acme`; the dashboard shows a site selector when more than one
  tenant exists.

This is routing, not isolation: all tenants share one database and one API keyless surface. Add
per-tenant API credentials and row-level authorization before exposing it to customers.

---

## Development

```bash
cargo test --workspace          # parser, COPY encoding, ranges, dimensions
cargo build --release

npm --prefix web install
npm --prefix web run typecheck
npm --prefix web run build
npm --prefix web run dev        # Vite on :5173, proxies /api to :8080
```

Run a local Postgres, then:

```bash
scripts/dev.sh start
python3 scripts/seed-demo.py --yes --host 127.0.0.1 --port 5514 --web 2000
scripts/reset-data.sh --demo-only
```

The seed generator emits deterministic, realistic SFOS lines and prints the totals the dashboard
should show. `scripts/smoke-test.py` turns that into assertions against a running stack.
`scripts/verify-release0.sh` runs the sample replay, benchmark, and scratch restore against a
disposable Compose project rather than the running production database. `scripts/verify-samples.py`
rebases the fixture timestamps and verifies persisted event groups and fields, overview totals, top
categories/applications/threats, dashboard bundle delivery, and queue/write-loss metrics.

### Verification performed for v1

- `cargo test --workspace --locked`: all 23 unit and integration tests passed, covering API range and
  bucket behavior, parser and ingestion metrics, quoted/escaped/bare values, RFC3164 and RFC5424
  framing, classic and SFOS 21 timestamps, real captured XGS lines (fixture at
  `crates/sfos-core/tests/fixtures/sfos21.log`), timezone handling, threat/auth/VPN
  classification, COPY escaping and arity, time-range validation, bucket selection, gap filling
  and top-dimension whitelisting.
- End-to-end: 51,662 events ingested over TCP; every API total, series sum, filter and tenant
  route matched the generator's expected values; invalid tenant/dimension returned 404/400.
- Real capture replay: all 7 lines in `fw1_samples.log` parsed; Start/Stop de-duplication produced
  2 allowed / 4 blocked connections, 3 web requests, 5 users, 14,228 bytes down and 2,083 bytes
  up, with the 14-day-old timestamps landing in the DEFAULT partition as designed.
- Release 0 replay: `scripts/verify-samples.py --yes` replayed the seven anonymized captured
  examples plus synthetic threat and authentication cases through TCP ingestion, PostgreSQL,
  rollups and API checks. All expected log types/groups, categories, application and threat
  rankings, authentication identity, and queue/write-loss metrics matched.
- Release 0 backup verification: the isolated run restored all 10,010 rows into a scratch
  database and verified raw rows, tenants, migrations, rollup rows, summarized event totals, and
  the readiness marker. The compressed backup was 596 KiB.
- Controlled benchmark: 10,001 generated events; 537.3 bytes/event including indexes; 3,015
  events/second; queue high-water 5,001/20,000; no drops or database write errors.
- Dashboard rendered in Chromium with the replay data visible, including the Windows Update
  application, EICAR threat and authentication row; no page errors or horizontal overflow.

---

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Dashboard empty, no errors | `curl :8081/metrics`; `parsed` must increase. Confirm the firewall targets this host's LAN IP on `5514` and *Device standard format*. The collector must listen on `0.0.0.0` (default; check with `ss -lunp | grep 5514`), and the host firewall must allow UDP 5514. |
| Dashboard shows demo values | Synthetic seed data was loaded. Run `scripts/reset-data.sh` (or `--demo-only`) and let real logs refill. |
| `rejected` counter rising | Another syslog source shares the port, or the payload is Sophos UTM/Cyberoam legacy format (not supported yet). `RUST_LOG=debug` samples rejected lines. |
| Users show “unidentified” | Authentication/SSO must be active and the rule logging usernames; web-filter logs include `user_name` only then. |
| Times are offset | Set `DEFAULT_TZ_OFFSET_MINUTES` for abbreviations not in the built-in table. |
| Health says `ingest_stale: true` | No event in the last 15 minutes — firewall stopped logging or delivery is blocked. |
| Postgres disk growth | Run `scripts/diagnose-storage.sh`. Confirm the hourly migration completed and `RETENTION_HOURS` is set as intended. |

For real deployments, restrict `ALLOWED_SOURCES` to the firewall or relay IPs that should be
sending logs. The collector accepts any source by default when the allow-list is empty.

---

## Roadmap

### Release 0 — Make the baseline trustworthy (completed 2026-10-21)

- [x] Keep seven anonymized captured examples covering firewall, web, and application events plus
  synthetic threat/authentication examples; parser tests exercise every sample group.
- [x] Replay the samples with current timestamps through TCP ingestion and verify raw API rows,
  hourly totals, categories, application/threat rankings, authentication identity, and dashboard
  assets on a clean isolated install.
- [x] Verify the restored database matches raw-event counts, tenant/migration counts, rollup row
  counts, summarized event totals, and the rollup readiness marker.
- [x] Measure storage/event and persistence throughput with a fixed 10,001-event workload,
  documented hardware, batch configuration, and zero dropped events/database write errors.
- [x] Expose live queue depth, capacity, utilization, and high-water mark alongside cumulative
  queued/written/error/drop counters.

**Exit gate met:** the isolated clean install and replay produce the expected API values and visible
dashboard entries, and the upgraded-schema backup restores with raw and rollup state matching.
Capacity figures are specific to the documented test hardware and workload, not a scale guarantee.

### Release 1 — Correct Sophos data (completed 2026-10-21)

- [x] Normalize `http_category` into `category`, and preserve `http_category_type` in
  `category_type`; expose the type in event API responses and expanded dashboard rows.
- [x] Verify known HTTP user-agent aliases normalize to `user_agent` without duplicating them in
  `attrs`; retain only report/search-relevant fields as normalized columns.
- [x] Test allowed and denied category captures, including category values, category type, action,
  and preservation of the normalized user-agent.
- [x] Verify categories, actions, category types, and category summary results through the API;
  verify category and type are visible in the dashboard event explorer.

**Exit gate:** the captured allowed and denied samples show their expected category and action in
the event explorer, and both appear in the category summary. The isolated replay confirmed
Information Technology / allowed and Software Updates / blocked, each with category type
Acceptable, in the API response, category summary, and rendered dashboard.

### Release 2 — Storage controls and retention (completed 2026-10-22)

- [x] Default to compact event attributes; retain only device model and firmware outside typed
  report fields. Set `KEEP_EXTENDED_ATTRIBUTES=true` to opt back into retaining every extra field.
- [x] Persist one **global** retention period shared by API and collector; initialize it from
  `RETENTION_HOURS` only when no saved value exists.
- [x] Add the bearer-token protected retention update, explicit period choices, database-size
  preview, and approximate partition-statistics-based storage projections in the dashboard.
- [x] Continue refreshing hourly rollups before raw partition removal; these rollups outlive raw
  details. Document that catch-all partition data is not dropped and that compressed exports are
  deferred due to extra storage and restore complexity.
- [x] Pass the isolated retention API/auth/storage-preview test and verify that saved settings
  survive API and collector restart.
- [x] Wire device activity pruning into the maintenance loop.

**Exit gate met:** the chosen detail-retention period is visible and editable via the dashboard (with bearer token auth), expired partitions are removed by collector maintenance within ~5 minutes after rollups refresh, and summary dashboards remain useful for older periods via hourly rollups.

### Release 3 — Useful web and user reporting (completed 2026-10-22)

- [x] Complete web-category, domain, user, policy, and application summaries via top panels and rollups.
- [x] Add practical filters for time period, user, category, action, and device in the event explorer.
- [x] Make missing fields legible: distinguish "no category in the firewall log" from "category data not supported" (shows "—" with clear empty states).
- [x] Implement comprehensive filtering system across API endpoints with proper query optimization; **resolved previous API timeout issues** through rollup-first queries, per-query statement timeouts, and query concurrency controls (semaphore limiting concurrent expensive queries).
- [x] Add CSV export for filtered results from the event explorer.

**Exit gate met:** an operator can answer "who accessed what, when, and was it allowed?" from the UI using filters for user, category, action, device, and time period, and export the matching results to CSV. The filtering system is performant with proper timeout handling and rollup-based aggregation.

### Future Improvements

- **Compressed exports**: Optional compressed exports for older event-level records are deferred due to additional storage requirements and restore complexity.
- **Authentication + roles**: Local users/OIDC with per-tenant authorization.
- **Scheduled reports**: Branded PDF/CSV generation and email delivery.
- **Alerts**: Threshold-based alerts with watchlists, throttling, and webhook/email delivery.
