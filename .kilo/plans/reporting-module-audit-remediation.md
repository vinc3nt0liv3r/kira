# Kira Reporting Module — Technical Audit & Remediation Plan

## Plan File

`.kilo/plans/reporting-module-audit-remediation.md`

---

## Executive Summary

This plan addresses four functional failures and one UX gap in the Kira reporting module:

| # | Issue | Severity | Root Cause Summary |
|---|-------|----------|-------------------|
| 1 | "Download PDF" button unresponsive | High | Frontend `onClick` has no try/catch (unhandled rejection); backend `generate_report_on_demand` references SELECT alias `display_user` in WHERE clause (invalid SQL when `q` filter is set) |
| 2 | "Export CSV" triggers 500 Internal Server Error | High | `events_export` passes unused query params `$7`/`$8`; possible query permit exhaustion or DB error not surfaced; no frontend error handling on `window.open()` |
| 3 | Report templates fail to save | High | Migration `0011_custom_reports.sql` may not be applied; `report_templates` table missing → INSERT fails with 500 |
| 4 | Custom reports save → 500 error | High | `report_type` column on `report_schedules` may not exist; INSERT fails if migration not applied |
| 5 | Manual JSON input too complex | Medium | Both `ReportSchedulesManager` and `CustomReportsManager` use raw JSON text input for filters |

---

## Phase 1: Bug Fixes

### 1.1 Fix "Download PDF" Unresponsive

#### Root Causes

**Frontend** (`web/src/components/EventsTable.tsx:141-161`):
- `onClick` handler for Download PDF calls `api.downloadReportOnDemand(...)` with **no try/catch**. If the backend returns an error, the promise rejection is unhandled — the user sees nothing happen.

**Backend** (`crates/sfos-api/src/api.rs:1964-2189`, `generate_report_on_demand`):
- The SQL query at lines 2000-2028 references `display_user` (a SELECT output alias) in the WHERE clause at lines 2007-2015.
- PostgreSQL does **not** allow SELECT aliases in WHERE. When the `q` filter is non-null (user typed a search term), the query throws `column "display_user" does not exist` → 500 error.
- The `events_export` handler correctly wraps this in a subquery; `generate_report_on_demand` does not.

**No user feedback layer**: Errors from `downloadReportOnDemand` are thrown but never caught or displayed in the UI.

#### Proposed Fix

**Backend**:
- Rewrite `generate_report_on_demand` SQL to wrap the `events` scan in a subquery (same pattern as `events_export`), so `display_user` is a real column in the outer query's FROM scope.
- Alternatively, inline the `COALESCE(NULLIF(user_name, ''), host(src_ip))` expression directly in each WHERE predicate instead of using the alias.

**Frontend**:
- Wrap `downloadReportOnDemand` call in try/catch in `EventsTable.tsx`.
- Show an alert or inline error message when download fails.
- Optionally add a loading state to the Download PDF button.

#### Affected Files

| File | Change |
|------|--------|
| `crates/sfos-api/src/api.rs` | Fix `generate_report_on_demand` SQL: subquery-wrap `events` or inline `display_user` expression |
| `web/src/components/EventsTable.tsx` | Add try/catch around `downloadReportOnDemand` call; add error feedback |

---

### 1.2 Fix "Export CSV" 500 Internal Server Error

#### Root Causes

**Backend** (`crates/sfos-api/src/api.rs:984-1154`, `events_export`):
- Query params `$7` (`100_000i64`) and `$8` (`0i64`) are passed but **not referenced anywhere in the SQL** (lines 1054-1071 use $1-$6 and $9-$11). This is wasteful and suggests the query was recently modified without updating params.
- The query permit semaphore (`acquire_query_permit`) can reject with `ApiError::internal` if capacity is exhausted — this returns 500.
- `window.open()` in the frontend has no error handling — if the backend returns 500, the new tab shows a raw error page or blank page.

**Frontend** (`web/src/api.ts:65-81`, `exportEvents`):
- Uses `window.open()` which opens a new tab — errors are not catchable in the main page context.
- No fallback or error display.

#### Proposed Fix

**Backend**:
- Remove unused `$7`/`$8` params from the `events_export` query call.
- Investigate query permit semaphore configuration (may need to increase `API_MAX_CONCURRENT_QUERIES`).
- Ensure all DB errors return proper status codes (not 500 for client-correctable issues).

**Frontend**:
- Replace `window.open()` with a fetch-based download (same pattern as `downloadReportOnDemand`) so errors can be caught and displayed.
- Add error handling with user-visible feedback.

#### Affected Files

| File | Change |
|------|--------|
| `crates/sfos-api/src/api.rs` | Remove unused `$7`/`$8` params from `events_export` query |
| `web/src/api.ts` | Replace `window.open()` in `exportEvents` with fetch+blob download pattern |
| `web/src/components/EventsTable.tsx` | Add try/catch around `exportEvents` call |

---

### 1.3 Fix Report Templates Save Failure

#### Root Cause

- Migration `0011_custom_reports.sql` creates `report_templates` table. If this migration has not been applied to the running database, `create_report_template` INSERT fails with `relation "report_templates" does not exist` → 500.
- The `create_report_template` handler (api.rs:1487) calls `state.store.create_report_template(&template)` which assumes the table exists.

#### Proposed Fix

**Verification**:
- Confirm migration `0011_custom_reports.sql` is applied in the target database.
- If not applied, run `cargo build` + migration (the `Store::migrate()` call in `main.rs:83` applies pending migrations on startup).

**Defensive coding**:
- Add `IF NOT EXISTS` guards (already in migration) — verify migration is idempotent.

#### Affected Files

| File | Change |
|------|--------|
| `crates/sfos-store/migrations/0011_custom_reports.sql` | Verify idempotency (already uses `IF NOT EXISTS`) |
| (No code change — operational: ensure migration runs on startup) |

---

### 1.4 Fix Custom Reports Save → 500 Error

#### Root Cause

- `create_custom_report` (api.rs:1558) creates a `ReportSchedule` with `report_type: "custom"` and calls `state.store.create_report_schedule(&report)`.
- The `report_type` column is added by migration `0011_custom_reports.sql`:
  ```sql
  ALTER TABLE report_schedules ADD COLUMN IF NOT EXISTS report_type text NOT NULL DEFAULT 'scheduled' CHECK (report_type IN ('scheduled', 'custom'));
  ```
- If migration not applied: INSERT fails with `column "report_type" does not exist` → 500.
- Existing rows without `report_type` are backfilled with `'scheduled'` by the `NOT NULL DEFAULT` clause, so existing scheduled reports continue to work.

#### Proposed Fix

- Ensure migration `0011_custom_reports.sql` is applied (same as issue #3 — both share the same migration).
- No code change needed if migration is applied; `report_type` column with CHECK constraint is already in the model.

#### Affected Files

| File | Change |
|------|--------|
| `crates/sfos-store/migrations/0011_custom_reports.sql` | Verify migration applies cleanly |
| (Operational: ensure migration runs on startup via `Store::migrate()`) |

---

## Phase 2: Visual Report Builder (JSON → Visual Configuration)

### Requirements Analysis

**Current state (JSON-based)**:
- `ReportSchedulesManager.tsx:237-239`: "Filters (JSON)" — raw `JSON.parse()` text input
- `CustomReportsManager.tsx:284-286`: Template "Default Filters (JSON)" — raw JSON input
- `CustomReportsManager.tsx:320-323`: Report "Filters (JSON)" — raw JSON input
- Users must know JSON syntax and field names (`from`, `to`, `q`, `action`, `category`, `device`, `user`, `group`)

**Target state (visual builder)**:
- Time range presets: Last hour, Last 24h, Last 7 days, Last 30 days, Custom
- Multi-select dropdowns: Users, Categories, Devices, Actions, Groups
- Free-text search field
- Output format: PDF / CSV dropdown
- Email recipients: comma-separated text input
- All configuration produces a JSON `filters` object under the hood

### Design Decisions (to be confirmed)

1. **Builder scope**: The visual builder generates the same `filters` JSON object that the backend already accepts. No backend changes needed — only frontend.
2. **Placement**: Replace the JSON text input in both `ReportSchedulesManager` and `CustomReportsManager` with a `ReportFilterBuilder` component.
3. **Template integration**: When a user selects a template, its `default_filters` populate the builder fields.

### Implementation Roadmap

#### Step 1: Fix All Bugs (Phase 1)
1. Fix `generate_report_on_demand` SQL (subquery-wrap `events`)
2. Add try/catch + error feedback to Download PDF button
3. Fix `events_export` unused params + replace `window.open()` with fetch download
4. Add try/catch to Export CSV button
5. Verify migration `0011_custom_reports.sql` applies on startup
6. Validate: `cargo test`, `cargo build`, `npx tsc`, `npm run build`, `docker compose up --build`

#### Step 2: Build `ReportFilterBuilder` Component
New file: `web/src/components/ReportFilterBuilder.tsx`

Props:
- `filters: Record<string, unknown>` (current values)
- `onChange: (filters: Record<string, unknown>) => void`
- Optional: `users`, `categories`, `devices`, `actions` (dropdown options, fetched from API or hardcoded)

Fields:
- Time range: preset buttons (1h, 24h, 7d, 30d) + custom from/to date pickers
- Search: free-text input (maps to `q`)
- Action: multi-select (allowed/blocked/warned)
- Category: multi-select input
- Device: multi-select input
- User: multi-select input
- Group: multi-select (web/threat/firewall/vpn/auth/system/other)

Output: serializes to the same `filters` JSON shape the backend expects:
```json
{
  "from": "24h",
  "q": "...",
  "action": "blocked",
  "category": "...",
  "device": "...",
  "user": "...",
  "group": "threat"
}
```

#### Step 3: Integrate Builder into Report Schedules Manager
- Replace "Filters (JSON)" text input in `ReportSchedulesManager.tsx` with `ReportFilterBuilder`
- Keep JSON as internal representation (serialize/deserialize)

#### Step 4: Integrate Builder into Custom Reports Manager
- Replace "Filters (JSON)" text input in `CustomReportsManager.tsx` (both template and report forms) with `ReportFilterBuilder`

#### Step 5: Validate
- `npx tsc --noEmit`
- `npm run build`
- `cargo test --workspace --locked`
- `cargo build --workspace --locked`
- `docker compose up --build`
- Manual: save report schedule with visual builder → no 422, no 500
- Manual: save custom report with visual builder → no 500
- Manual: download PDF from events table → file downloads
- Manual: export CSV from events table → file downloads

---

## Open Questions for User Approval

Before implementation, the following decisions need resolution:

1. **Dropdown options source**: Should filter dropdown options (users, categories, devices) be hardcoded, or fetched from API endpoints? (Recommended: hardcode common options + allow free-text input for flexibility)
2. **Backend changes**: Are any backend changes needed beyond the SQL fix in `generate_report_on_demand`? (Recommended: no — the builder produces the same JSON the backend already accepts)
3. **Template filtering**: Should templates pre-populate the builder, or should users always start from empty? (Recommended: pre-populate from `default_filters`)
4. **Plan finalization**: Should I finalize this plan now for implementation, or are there additional design questions? (Recommended: finalize now)

---

## Validation Checklist

- [ ] `cargo test --workspace --locked` passes (all tests)
- [ ] `cargo build --workspace --locked` passes
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` passes
- [ ] `docker compose up --build` passes (all containers start)
- [ ] Manual: Download PDF from EventsTable → file downloads, no error
- [ ] Manual: Export CSV from EventsTable → file downloads, no 500
- [ ] Manual: Save report template → persists in database
- [ ] Manual: Save custom report → persists with `report_type = 'custom'`
- [ ] Manual: Visual builder produces valid JSON filters, no manual JSON editing needed

---

## Rollback Plan

- All database changes are already applied (migration `0011_custom_reports.sql`). No rollback needed for bug fixes — they are pure code fixes.
- Frontend changes are component-local. If the visual builder has issues, the JSON input can be restored by reverting the component.
- API changes are additive (fixes to existing handlers, no endpoint removal).

---

## Decision Summary

| Decision | Resolution |
|----------|-----------|
| `generate_report_on_demand` SQL fix | Subquery-wrap `events` to make `display_user` a real outer-query column |
| Export CSV 500 | Remove unused params; switch frontend to fetch+blob download |
| Templates/custom reports 500 | Ensure migration `0011_custom_reports.sql` applies on startup |
| Visual builder | Frontend-only; produces same JSON backend already accepts; replace JSON text inputs |
| Backend changes for builder | None needed |

---

**Plan location**: `.kilo/plans/reporting-module-audit-remediation.md`
