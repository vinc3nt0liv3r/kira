# Kira Release 5.5 — Reporting Gap Analysis & Custom Reports Module

## Plan File

`.kilo/plans/1792559583334-kira-reporting-gap-analysis.md`

---

## Executive Summary

This plan addresses three issues identified during the Release 5.5 review:

1. **422 errors when saving alert rules and report schedules** — frontend/backend contract mismatch on optional JSON fields (`filters`, `recipients`, `description`, `webhook_url`)
2. **Report download appears missing/broken** — download endpoint exists but is hardcoded with `schedule_id=0` in the frontend fetch URL; no tenant-ownership verification in the download handler
3. **New "Custom Reports" module** — Fastvue-inspired dedicated report configuration and management section, separate from scheduled reports

---

## Phase 1: Feature Planning and Architecture

### 1. Bug Fix Analysis: Save Failures (422 Errors)

#### Root Cause

The backend `CreateReportSchedule` and `CreateAlertRule` structs declare `serde_json::Value` fields as **required** (not `Option`):

```rust
// api.rs — CreateReportSchedule
pub struct CreateReportSchedule {
    pub name: String,
    pub description: Option<String>,
    pub cron_schedule: String,
    pub format: String,
    pub recipients: serde_json::Value,   // required, not Option
    pub filters: serde_json::Value,       // required, not Option
    pub enabled: Option<bool>,
}
```

When axum's `Json` extractor receives a JSON object missing these fields (or with `null`), deserialization fails and axum returns **422 Unprocessable Entity**.

The frontend initializes `filters: {}` and `recipients: []`, so new schedules should work. However, edge cases:
- User clears the Filters JSON field entirely → input becomes `""` → `JSON.parse("" || "{}")` = `{}` (OK)
- But if any code path serializes `undefined` or `null` for these fields, axum rejects with 422
- `UpdateReportSchedule` correctly uses `Option<serde_json::Value>` — inconsistent with create

#### Proposed Fix

**Backend** (`crates/sfos-api/src/api.rs`):

1. Change `CreateReportSchedule.recipients` and `CreateReportSchedule.filters` from `serde_json::Value` to `Option<serde_json::Value>`, defaulting to `'[]'::jsonb` and `'{}'::jsonb` respectively when `None`.
2. Change `CreateAlertRule.webhook_url` to `Option<String>` (already `Option<String>` — verify consistency).
3. In the handler, coalesce `None` to `serde_json::json!([])` / `serde_json::json!({})` before constructing the `ReportSchedule`/`AlertRule` model.

**Frontend** (`web/src/api.ts`, `web/src/components/ReportSchedulesManager.tsx`):

1. Ensure `filters` and `recipients` are always included in the POST payload (never `undefined`).
2. Add form validation: `name` required, `cron_schedule` required and non-empty, `filters` must be valid JSON.
3. Show field-level error messages instead of generic `alert()`.

#### Affected Files

| File | Change |
|------|--------|
| `crates/sfos-api/src/api.rs` | `CreateReportSchedule` struct: `recipients`/`filters` → `Option<serde_json::Value>`; handler defaults |
| `crates/sfos-api/src/api.rs` | `CreateAlertRule` struct: verify `webhook_url` is `Option<String>` |
| `web/src/api.ts` | Ensure `filters`/`recipients` always serialized in payload |
| `web/src/components/ReportSchedulesManager.tsx` | Add client-side validation and error display |

---

### 2. UI/UX Remediation: Report Download Functionality

#### Current State

Download is partially implemented:
- **Backend**: `GET /api/v1/reports/schedules/{schedule_id}/results/{result_id}/download` → `download_report_result` handler serves the file from disk with `Content-Disposition: attachment`
- **Frontend**: `api.downloadReportResult(resultId)` fetches the file and triggers browser download via blob + anchor click
- **UI**: `ReportSchedulesManager` has a "Download" button per report result row

#### Bugs

1. **Hardcoded schedule_id=0** in `api.ts` line 297:
   ```typescript
   const res = await fetch(`${BASE}/reports/schedules/0/results/${resultId}/download`, ...)
   ```
   The backend handler ignores `schedule_id` in the path (only extracts `result_id`), so this works accidentally but is fragile and incorrect.

2. **No tenant-ownership verification** — `download_report_result` looks up result by ID only, doesn't check the requesting tenant owns the schedule. Any authenticated user could download any report result by guessing UUIDs.

3. **No "Download PDF directly from browser on-demand"** — the user wants a standalone PDF download that doesn't require creating a schedule first. Currently PDF generation only happens via the scheduler (every 60s) or the "Generate Now" button on an existing schedule.

#### Proposed Fix

**Backend**:
1. Fix `download_report_result` to extract both `schedule_id` and `result_id` from path, verify tenant ownership of the schedule.
2. Add new endpoint: `POST /api/v1/reports/generate` — on-demand report generation with inline filters (no schedule required), returns file directly as download.

**Frontend**:
1. Fix `downloadReportResult` to pass the actual `schedule_id` from the result's parent schedule.
2. Add "Download PDF" and "Download CSV" buttons to the main events/dashboard area that trigger on-demand generation and immediate download.
3. Add download button to the report results table (already present, fix the schedule_id).

#### Affected Files

| File | Change |
|------|--------|
| `crates/sfos-api/src/api.rs` | Fix `download_report_result` path extraction + tenant check; add `generate_report_on_demand` handler |
| `crates/sfos-api/src/main.rs` | Add route for on-demand generation |
| `web/src/api.ts` | Fix `downloadReportResult` schedule_id; add `downloadReportOnDemand` |
| `web/src/components/ReportSchedulesManager.tsx` | Pass correct schedule_id to download |
| `web/src/components/EventsTable.tsx` (or new component) | Add on-demand PDF/CSV download buttons |

---

### 3. New Feature Specification: "Custom Reports" Module

#### Requirements (Modeled After Fastvue)

Fastvue provides a dedicated report builder with:
- Report templates (pre-defined report types)
- Custom filter builder (time range, user, category, action, device, etc.)
- Output format selection (PDF, CSV)
- Report scheduling (cron) + manual run
- Report history with download
- Email recipients

#### Design: Kira "Custom Reports" Module

Since Kira already has `report_schedules` and `report_results` tables, the Custom Reports module builds on this foundation but adds:

1. **Report Templates** — predefined filter sets for common report types:
   - "Top Blocked Sites"
   - "Top Threat Events"
   - "User Activity Summary"
   - "Bandwidth by Category"
   - "Firewall Rule Hits"

2. **Report Builder UI** — form-based filter construction (no raw JSON required):
   - Time range picker (presets: Last hour, Last 24h, Last 7d, Last 30d, Custom)
   - Multi-select for users, categories, devices, actions
   - Free-text search
   - Output format (PDF/CSV)
   - Email recipients (optional)

3. **Report Library** — list of saved custom reports with:
   - Last run status
   - Download links for each result
   - Edit/Delete
   - Run now

4. **Separation from Scheduled Reports** — Custom Reports has its own section in the navigation, distinct from the existing "Report Schedules" card. Scheduled reports remain for cron-based automation; Custom Reports is for ad-hoc and template-based reports.

#### Data Model Changes

**New migration** (`0011_custom_reports.sql`):

```sql
-- Report templates (pre-defined filter sets)
CREATE TABLE IF NOT EXISTS report_templates (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    name text NOT NULL,
    description text,
    category text NOT NULL, -- 'security', 'bandwidth', 'user_activity', 'web_filter'
    default_filters jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Custom reports (saved report configurations, separate from schedules)
CREATE TABLE IF NOT EXISTS custom_reports (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id uuid NOT NULL REFERENCES tenants (id) ON DELETE CASCADE,
    template_id uuid REFERENCES report_templates (id) ON DELETE SET NULL,
    name text NOT NULL,
    description text,
    filters jsonb NOT NULL DEFAULT '{}'::jsonb,
    format text NOT NULL CHECK (format IN ('csv', 'pdf')) DEFAULT 'pdf',
    recipients jsonb NOT NULL DEFAULT '[]'::jsonb,
    last_run timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS custom_reports_tenant_id_idx ON custom_reports (tenant_id);
CREATE INDEX IF NOT EXISTS custom_reports_template_id_idx ON custom_reports (template_id);

-- Seed default templates
INSERT INTO report_templates (tenant_id, name, description, category, default_filters)
SELECT t.id, 'Top Blocked Sites', 'Most frequently blocked web destinations', 'web_filter',
       '{"action": "blocked", "group": "destination", "limit": 50}'::jsonb
FROM tenants t WHERE t.slug = 'default'
ON CONFLICT DO NOTHING;

-- ... additional templates
```

**Note**: Instead of a separate `custom_reports` table, we can reuse `report_schedules` with a `report_type` column (`'scheduled'` vs `'custom'`). This avoids schema duplication. Decision needed during plan approval.

#### API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/reports/templates` | List report templates |
| `POST` | `/api/v1/reports/templates` | Create template (admin) |
| `GET` | `/api/v1/reports/custom` | List custom reports |
| `POST` | `/api/v1/reports/custom` | Create custom report |
| `GET` | `/api/v1/reports/custom/{id}` | Get custom report detail |
| `PUT` | `/api/v1/reports/custom/{id}` | Update custom report |
| `DELETE` | `/api/v1/reports/custom/{id}` | Delete custom report |
| `POST` | `/api/v1/reports/custom/{id}/run` | Run custom report on-demand |
| `GET` | `/api/v1/reports/custom/{id}/results` | List results for a custom report |
| `GET` | `/api/v1/reports/custom/{schedule_id}/results/{result_id}/download` | Download result |

#### Frontend Components

| Component | Purpose |
|-----------|---------|
| `CustomReportsManager` | Main container, tabbed view (Templates / Custom Reports / Results) |
| `ReportTemplateList` | Grid of available templates with "Use Template" action |
| `ReportBuilderForm` | Filter builder form (no raw JSON), format selector, recipients |
| `CustomReportList` | Table of saved custom reports with run status |
| `CustomReportResults` | Results table with download buttons |
| `ReportRunDialog` | Modal for configuring and running a report on-demand |

#### UI Placement

Add a new section below Report Schedules in `App.tsx`, or create a separate navigation tab. Decision needed: tabbed navigation vs. stacked cards.

---

## Phase 2: Execution Roadmap

### Step 1: Fix Save Bugs (422 Errors)

1. **Backend**: Change `CreateReportSchedule.recipients` and `filters` to `Option<serde_json::Value>` with defaults.
2. **Backend**: Verify `CreateAlertRule.webhook_url` is `Option<String>`.
3. **Frontend**: Ensure `filters`/`recipients` always serialized in POST payload.
4. **Frontend**: Add client-side validation for required fields.
5. **Validate**: `cargo test --workspace --locked`, `npx tsc --noEmit`, `npm run build`.
6. **Validate**: `docker compose up --build`.

### Step 2: Fix Report Download

1. **Backend**: Fix `download_report_result` to extract `schedule_id` from path and verify tenant ownership.
2. **Backend**: Add `POST /api/v1/reports/generate` on-demand generation endpoint.
3. **Frontend**: Fix `downloadReportResult` to pass correct `schedule_id`.
4. **Frontend**: Add `downloadReportOnDemand` API method.
5. **Frontend**: Add "Download PDF"/"Download CSV" buttons to events/dashboard area.
6. **Validate**: same as Step 1.

### Step 3: Custom Reports Module — Database

1. **Migration**: Create `0011_custom_reports.sql` with templates + custom reports tables (or decide on `report_type` column approach).
2. **Store layer**: Add CRUD methods for templates and custom reports.
3. **Validate**: `cargo test --workspace --locked`.

### Step 4: Custom Reports Module — API

1. **Handlers**: Implement template list/create, custom report CRUD, run-on-demand, results, download.
2. **Routes**: Register all new routes in `main.rs`.
3. **Validate**: `cargo test --workspace --locked`, `docker compose up --build`.

### Step 5: Custom Reports Module — Frontend

1. **Types**: Add TypeScript interfaces for templates, custom reports, custom report results.
2. **API client**: Add methods for all new endpoints.
3. **Components**: Build `CustomReportsManager`, `ReportTemplateList`, `ReportBuilderForm`, `CustomReportList`, `CustomReportResults`, `ReportRunDialog`.
4. **Integration**: Add to `App.tsx` layout.
5. **Validate**: `npx tsc --noEmit`, `npm run build`, `docker compose up --build`.

---

## Open Questions for User Approval

Before implementation begins, the following decisions need to be resolved:

1. **Custom Reports data model**: Separate `custom_reports` table vs. `report_type` column on `report_schedules`?
2. **UI placement**: Tabbed navigation vs. stacked cards below Report Schedules?
3. **On-demand download endpoint**: Should it return the file directly (stream) or generate + return a result ID + separate download?
4. **Report templates**: Seed with default templates for the `default` tenant only, or allow per-tenant templates?
5. **Authentication**: Are report endpoints protected by tenant-scoped auth, or is the single-tenant model sufficient?

---

## Validation Checklist

- [ ] `cargo test --workspace --locked` passes (all tests)
- [ ] `cargo build --workspace --locked` passes
- [ ] `npx tsc --noEmit` passes
- [ ] `npm run build` passes
- [ ] `docker compose up --build` passes (all containers start)
- [ ] Manual test: save report schedule with empty filters → no 422
- [ ] Manual test: download report result from browser → file downloads
- [ ] Manual test: on-demand PDF generation and download works

---

## Rollback Plan

- All database changes are additive (new tables/columns), reversible via migration rollback SQL if needed.
- Frontend changes are component-local; existing Report Schedules UI remains intact.
- API changes add new endpoints; existing endpoints are not removed, only fixed.