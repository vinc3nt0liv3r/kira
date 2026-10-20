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
  writer with retries, partition maintenance and a metrics endpoint.
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
Run `scripts/verify-samples.py --yes` against an isolated test stack to replay the anonymized
captured samples over syslog and check the rows returned by the API. It refuses to replay when
matching sample rows already exist in its fixture time window.

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

Every accepted syslog line is stored as an event row, including unmodeled fields in JSONB `attrs`.
Storage varies substantially with log mix: the historical sample used about 1.2 KB/event while
the controlled replay used about 536 bytes/event. At the historical sample size, a 14 GB event
database would represent roughly 12 million events. Size requirements depend on actual log rate
and fields; measure the deployment before choosing retention.

In the live sample, `attrs` averaged about 794 bytes in sampled rows. It contains vendor fields
that are not modeled as report columns (such as interfaces, MAC addresses, and rule/application
metadata); it is not a second copy of every normalized field. A couple of aliases were being kept
there despite also being normalized (`device_serial_id` and `http_user_agent`); new ingestion now
maps/omits those duplicates. That small correction does not shrink existing rows. A larger storage
reduction requires deciding which unmodeled attributes are needed: an allowlist or configurable
compact mode can save space, but permanently discards fields outside the chosen set. Prefer a
short raw-event window plus long-lived rollups over silently deleting fields or attempting to
compress indexes/rows blindly.

Dashboard identities now use the source IP when a log has no username, and the top-destinations
panel includes both website domains and application names. The parser also maps `http_user_agent`
to its typed field and no longer retains `device_serial_id` a second time in `attrs`.

Raw event rows are retained for `RETENTION_HOURS` (default 6, with up to one extra hour at hourly
partition boundaries). The collector refreshes each expiring hour's rollups before dropping that
raw partition. Overview and top-dimension reports use hourly rollups across the full selected
range; event detail search is limited to retained raw rows. Rollup refresh and raw-partition
removal are separate, repeatable operations so a failed refresh cannot discard its source partition.

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
scripts/verify-samples.py  replay captured events and verify persisted API results
scripts/verify-backup-restore.sh  scratch-database backup/restore verification
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
| `RETENTION_HOURS` | `6` | Raw searchable-event window; hourly partitions are rolled up before removal |
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
| `RETENTION_HOURS` | `6` | Raw event detail window shown and enforced by the API |
| `DB_POOL_SIZE` | `8` | Postgres connections |

### Security

There is **no login** in v1. Run the dashboard on a trusted management network, or put it behind a
reverse proxy that terminates TLS and enforces authentication (basic auth, OIDC, VPN). Do not
publish the dashboard port to the internet. The syslog port has no authentication either — use
`ALLOWED_SOURCES` to restrict which addresses may submit logs.

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
| `GET /api/v1/events` | Newest events with `q`, `action`, `group`, `limit`, `offset` |

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
- `events` — one row per accepted syslog message, partitioned by hour on `ts`, primary key
  `(ts, id)`. Includes normalized columns for user, domain, URL, category, application, rule,
  bytes, threat, IPs, zones, ports and `con_event`, plus `attrs jsonb` with every unmodeled SFOS
  field.
- `events_default` — catch-all partition for timestamps outside the pre-created hours.
- `event_rollups` — compact hourly overview and dimension totals for users/hosts, domains,
  destinations, categories, policies, threats, applications, countries, and devices.
- Indexes: `(tenant_id, ts)`, `(tenant_id, category_group, ts)` and `(tenant_id, user_name, ts)`.

Retention refreshes rollups and drops complete hourly partitions older than `RETENTION_HOURS`.
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
`scripts/verify-samples.py --yes` replays the anonymized captured lines in `fw1_samples.log`
through the actual TCP collector and checks that all expected log types and web categories appear
in the event API.

### Verification performed for v1

- `cargo test --workspace --locked`: 20 tests covering quoted/escaped/bare values, RFC3164 and RFC5424
  framing, classic and SFOS 21 timestamps, real captured XGS lines (fixture at
  `crates/sfos-core/tests/fixtures/sfos21.log`), timezone handling, threat/auth/VPN
  classification, COPY escaping and arity, time-range validation, bucket selection, gap filling
  and top-dimension whitelisting.
- End-to-end: 51,662 events ingested over TCP; every API total, series sum, filter and tenant
  route matched the generator's expected values; invalid tenant/dimension returned 404/400.
- Real capture replay: all 7 lines in `fw1_samples.log` parsed; Start/Stop de-duplication produced
  2 allowed / 4 blocked connections, 3 web requests, 5 users, 14,228 bytes down and 2,083 bytes
  up, with the 14-day-old timestamps landing in the DEFAULT partition as designed.
- Release 0 replay: `scripts/verify-samples.py --yes` verified all 7 anonymized captured lines
  through TCP ingestion, PostgreSQL storage, and the API, including the SFOS `http_category`
  values `Information Technology` and `Software Updates`.
- Release 0 backup verification: `scripts/verify-backup-restore.sh` restored all 1,515,841 old
  events into a scratch database and verified the row count before the old event rows were cleared.
- Controlled benchmark: 10,001 generated events; 536 bytes/event including current-day indexes;
  966 events/second across the receive window; no drops or database errors. API medians:
  overview 67 ms, top categories 20 ms, event list 5 ms.
- `cargo test --workspace --locked`: 20 Rust tests passed after the Release 0 parser correction.
- Dashboard rendered in Chromium with no console errors: search filtered rows to the matching
  user, row details expanded, bandwidth toggle worked, pause/resume worked, no horizontal overflow.

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

### Release 0 — Make the baseline trustworthy (completed 2026-10-20)

- [x] Keep anonymized, representative Sophos samples in `fw1_samples.log` and
  `crates/sfos-core/tests/fixtures/sfos21.log`.
- [x] Verify the captured sample lines from TCP ingestion through PostgreSQL and API output with
  `scripts/verify-samples.py --yes`; expected log types and web categories match.
- [x] Document backup/restore and verify a custom-format backup in a scratch database with
  `scripts/verify-backup-restore.sh`; event, tenant, and migration counts matched.
- [x] Measure storage/event, ingestion rate, and API latency on the documented 4-vCPU test host.
- [x] Clear the previous event set after the backup/restore check; the database now has zero event
  rows, with schema and default tenant retained.

**Exit gate met:** all captured events produced the expected persisted API values, and backup
restore was verified. Measurements are a reference for this test host, not a scale guarantee.

### Next releases

Ordered by what users of tools like this ask for first:

1. **Authentication + roles** (local users or OIDC) and per-tenant authorization.
2. **Scheduled reports**: branded PDF/CSV generation and email delivery to managers.
3. **Alerts**: large transfers, blocked categories, threat names, keyword and domain watchlists,
   with throttling and email/webhook delivery.
4. **Directory integration**: Entra ID / AD group → department mapping for department reports.
5. **Enrichment**: Site Clean (collapse CDN noise), search-term extraction, YouTube metadata.
6. **Rollups / continuous aggregates** for very large firewalls, plus CSV export and saved views.
7. **Sophos UTM (SG) and Cyberoam** legacy format support.
