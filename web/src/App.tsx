import { useEffect, useMemo, useState } from "react";
import { api } from "./api";
import { fmtAgo, fmtNumber } from "./format";
import type { EventRow, Meta, Overview, TopRow } from "./types";
import { EventsTable } from "./components/EventsTable";
import { RetentionControl } from "./components/RetentionControl";
import { KpiCards } from "./components/KpiCards";
import { CollectorPanel, SecurityPanel } from "./components/SidePanels";
import { TopPanel } from "./components/TopPanel";
import { TrafficChart, type TrafficMode } from "./components/TrafficChart";
import AlertRulesManager from "./components/AlertRulesManager";
import ReportSchedulesManager from "./components/ReportSchedulesManager";
import CustomReportsManager from "./components/CustomReportsManager";
import { Card, Skeleton, Spinner } from "./components/ui";

const RANGES = [
  { id: "1h", label: "1h", hours: 1 },
  { id: "6h", label: "6h", hours: 6 },
  { id: "24h", label: "24h", hours: 24 },
  { id: "7d", label: "7d", hours: 168 },
  { id: "30d", label: "30d", hours: 720 },
] as const;

type Tops = {
  users: TopRow[];
  destinations: TopRow[];
  categories: TopRow[];
  threats: TopRow[];
};

const EMPTY_TOPS: Tops = { users: [], destinations: [], categories: [], threats: [] };

function useDebounced<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const id = window.setTimeout(() => setDebounced(value), delayMs);
    return () => window.clearTimeout(id);
  }, [value, delayMs]);
  return debounced;
}

export default function App() {
  const [tenant, setTenant] = useState("");
  const [range, setRange] = useState<string>("24h");
  const [mode, setMode] = useState<TrafficMode>("requests");
  const [live, setLive] = useState(true);
  const [tick, setTick] = useState(0);

  const [meta, setMeta] = useState<Meta | null>(null);
  const [overview, setOverview] = useState<Overview | null>(null);
  const [tops, setTops] = useState<Tops>(EMPTY_TOPS);
  const [events, setEvents] = useState<EventRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null);

  const [query, setQuery] = useState("");
  const debouncedQuery = useDebounced(query, 300);
  const [action, setAction] = useState("");
  const [group, setGroup] = useState("");
  const [user, setUser] = useState("");
  const debouncedUser = useDebounced(user, 300);
  const [category, setCategory] = useState("");
  const debouncedCategory = useDebounced(category, 300);
  const [device, setDevice] = useState("");
  const debouncedDevice = useDebounced(device, 300);

  const spanHours = useMemo(
    () => RANGES.find((option) => option.id === range)?.hours ?? 24,
    [range],
  );

  // Tenant list and collector health. Refreshed slowly: it does not drive charts.
  useEffect(() => {
    const controller = new AbortController();
    let refreshTimer: number | undefined;
    const load = async () => {
      try {
        const result = await api.meta(tenant || undefined, controller.signal);
        setMeta(result);
        if (!tenant) setTenant(result.tenant.slug);
      } catch (err) {
        if (controller.signal.aborted) return;
        const message = err instanceof Error ? err.message : String(err);
        if (tenant) {
          // Saved/selected tenant disappeared; fall back to the server default.
          setTenant("");
        }
        setError(message);
      } finally {
        if (!controller.signal.aborted) {
          refreshTimer = window.setTimeout(load, 5 * 60_000);
        }
      }
    };
    void load();
    return () => {
      controller.abort();
      if (refreshTimer !== undefined) window.clearTimeout(refreshTimer);
    };
  }, [tenant]);

  useEffect(() => {
    if (!tenant) return;
    const controller = new AbortController();
    let refreshTimer: number | undefined;
    setRefreshing(true);
    const load = async () => {
      try {
        const nextOverview = await api.overview(tenant, range, controller.signal);
        const users = await api.top(tenant, range, "users", 8, controller.signal);
        const destinations = await api.top(tenant, range, "destinations", 8, controller.signal);
        const categories = await api.top(tenant, range, "categories", 8, controller.signal);
        const threats = await api.top(tenant, range, "threats", 5, controller.signal);
        const nextEvents = await api.events(
          tenant,
          range,
          {
            q: debouncedQuery || undefined,
            action: action || undefined,
            group: group || undefined,
            user: debouncedUser || undefined,
            category: debouncedCategory || undefined,
            device: debouncedDevice || undefined,
            limit: 100,
          },
          controller.signal,
        );
        if (controller.signal.aborted) return;
        setOverview(nextOverview);
        setTops({
          users: users.rows,
          destinations: destinations.rows,
          categories: categories.rows,
          threats: threats.rows,
        });
        setEvents(nextEvents.rows);
        setError(null);
        setUpdatedAt(new Date());
      } catch (err) {
        if (controller.signal.aborted) return;
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (!controller.signal.aborted) {
          setRefreshing(false);
          if (live) {
            refreshTimer = window.setTimeout(() => setTick((value) => value + 1), 60_000);
          }
        }
      }
    };
    void load();
    return () => {
      controller.abort();
      if (refreshTimer !== undefined) window.clearTimeout(refreshTimer);
    };
  }, [tenant, range, tick, debouncedQuery, action, group, debouncedUser, debouncedCategory, debouncedDevice, live]);

  const refresh = () => setTick((value) => value + 1);
  const hasData = overview !== null;
  const waitingForLogs = hasData && overview.totals.events === 0;

  return (
    <div className="min-h-screen">
      <header className="sticky top-0 z-30 border-b border-white/[0.06] bg-[#070b14]/85 backdrop-blur-md">
        <div className="mx-auto flex max-w-[1560px] flex-wrap items-center gap-x-4 gap-y-2 px-4 pb-2 pt-3 md:px-8">
          <div className="flex items-center gap-2.5">
            <svg viewBox="0 0 32 32" className="h-7 w-7" aria-hidden="true">
              <path
                d="M16 2 4 7v9c0 7.2 5.1 12.4 12 14 6.9-1.6 12-6.8 12-14V7L16 2Z"
                fill="url(#shield)"
              />
              <path d="M16 9v9m0 4v.5" stroke="#04110c" strokeWidth="2.4" strokeLinecap="round" />
              <defs>
                <linearGradient id="shield" x1="0" y1="0" x2="1" y2="1">
                  <stop offset="0%" stopColor="#6ee7b7" />
                  <stop offset="100%" stopColor="#10b981" />
                </linearGradient>
              </defs>
            </svg>
            <div className="leading-tight">
              <h1 className="text-[15px] font-semibold tracking-tight text-slate-50">
                Kira
              </h1>
              <p className="text-[10px] uppercase tracking-[0.18em] text-slate-500">
                The Sophos Reporter
              </p>
            </div>
          </div>

          <div className="ml-auto flex items-center gap-2">
            {meta && meta.tenants.length > 1 ? (
              <label className="flex items-center gap-2 text-xs text-slate-500">
                <span className="hidden sm:inline">Site</span>
                <select
                  value={tenant}
                  onChange={(event) => setTenant(event.target.value)}
                  className="rounded-lg border border-white/10 bg-[#0b1220] px-2 py-1.5 text-xs text-slate-200 focus:border-emerald-400/40 focus:outline-none"
                >
                  {meta.tenants.map((option) => (
                    <option key={option.slug} value={option.slug}>
                      {option.name}
                    </option>
                  ))}
                </select>
              </label>
            ) : meta ? (
              <span className="hidden items-center gap-1.5 text-xs text-slate-400 sm:inline-flex">
                <span className="h-1.5 w-1.5 rounded-full bg-slate-600" />
                {meta.tenant.name}
              </span>
            ) : null}

            <RetentionControl />

            <button
              type="button"
              onClick={() => setLive((value) => !value)}
              className={`inline-flex items-center gap-2 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition ${
                live
                  ? "border-emerald-400/30 bg-emerald-400/[0.08] text-emerald-300"
                  : "border-white/10 bg-white/[0.03] text-slate-400 hover:text-slate-200"
              }`}
              title={live ? "Pause automatic refresh" : "Resume automatic refresh"}
            >
              <span
                className={`h-1.5 w-1.5 rounded-full ${live ? "bg-emerald-400 live-dot" : "bg-slate-500"}`}
              />
              {live ? "Live" : "Paused"}
            </button>

            <button
              type="button"
              onClick={refresh}
              disabled={refreshing}
              className="inline-flex items-center gap-1.5 rounded-lg border border-white/10 bg-white/[0.03] px-2.5 py-1.5 text-xs text-slate-400 transition hover:text-slate-200 disabled:opacity-60"
              title="Refresh now"
            >
              {refreshing ? <Spinner /> : <RefreshIcon />}
              <span className="hidden sm:inline">Refresh</span>
            </button>
          </div>

          <div className="flex w-full items-center gap-2 overflow-x-auto pb-0.5">
            <div className="flex rounded-lg border border-white/10 bg-black/25 p-0.5">
              {RANGES.map((option) => (
                <button
                  key={option.id}
                  type="button"
                  onClick={() => setRange(option.id)}
                  className={`rounded-md px-2.5 py-1 text-[11px] font-medium transition ${
                    range === option.id
                      ? "bg-white/10 text-slate-100"
                      : "text-slate-500 hover:text-slate-300"
                  }`}
                >
                  {option.label}
                </button>
              ))}
            </div>
            <span className="ml-auto whitespace-nowrap text-[11px] text-slate-600">
              {updatedAt ? `Updated ${fmtAgo(updatedAt.toISOString())}` : "Connecting…"}
            </span>
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-[1560px] space-y-4 px-4 py-5 md:px-8">
        {error ? (
          <div className="flex items-start justify-between gap-4 rounded-xl border border-rose-500/25 bg-rose-500/[0.07] px-4 py-3">
            <p className="text-sm text-rose-200">
              <span className="font-medium">Cannot load data.</span> {error}
            </p>
            <button
              type="button"
              onClick={refresh}
              className="shrink-0 rounded-md border border-rose-400/30 px-2 py-1 text-xs text-rose-200 hover:bg-rose-400/10"
            >
              Retry
            </button>
          </div>
        ) : null}

        <KpiCards totals={overview?.totals ?? null} loading={!hasData} />

        {!hasData ? (
          <DashboardSkeleton />
        ) : waitingForLogs && !meta?.last_event_at ? (
          <WaitingForLogs />
        ) : (
          <>
            <div className="grid gap-4 lg:grid-cols-3">
              <div className="lg:col-span-2">
                <TrafficChart
                  series={overview.series}
                  mode={mode}
                  onMode={setMode}
                  spanHours={spanHours}
                />
              </div>
              <div className="flex flex-col gap-4">
                <SecurityPanel
                  total={overview.totals.threats}
                  rows={tops.threats}
                  loading={refreshing}
                />
                <CollectorPanel meta={meta} />
              </div>
            </div>

            <div className="grid gap-4 lg:grid-cols-3">
              <TopPanel
                title="Top users / hosts"
                hint="Usernames or source IPs when no username is logged"
                rows={tops.users}
                accent="sky"
                loading={refreshing}
                emptyHint="No usernames or source IP addresses were logged in this range."
              />
              <TopPanel
                title="Top destinations"
                hint="Websites and applications"
                rows={tops.destinations}
                accent="emerald"
                loading={refreshing}
                showBlocked={false}
              />
              <TopPanel
                title="Categories"
                hint="Web filter categories"
                rows={tops.categories}
                accent="violet"
                loading={refreshing}
                showBlocked={false}
              />
            </div>

            <EventsTable
              rows={events}
              loading={refreshing}
              spanHours={spanHours}
              rawRetentionHours={meta?.raw_retention_hours ?? 6}
              query={query}
              onQuery={setQuery}
              action={action}
              onAction={setAction}
              group={group}
              onGroup={setGroup}
              tenant={tenant}
              from={range}
              user={user}
              onUser={setUser}
              category={category}
              onCategory={setCategory}
              device={device}
              onDevice={setDevice}
            />
          </>
        )}

        <AlertRulesManager />
        <ReportSchedulesManager />
        <CustomReportsManager />

        <footer className="flex flex-wrap items-center justify-between gap-2 pb-4 pt-1 text-[10px] text-slate-600">
          <span>
            {meta
              ? `${meta.tenant.name} · ${meta.raw_retention_hours}h raw retention · hourly aggregates · ${fmtNumber(
                  overview?.totals.events ?? 0,
                )} events in range`
              : "Self-hosted — log data never leaves this server"}
          </span>
          <span>Sophos Firewall syslog collector · UDP/TCP 5514</span>
        </footer>
      </main>
    </div>
  );
}

function DashboardSkeleton() {
  return (
    <div className="grid gap-4 lg:grid-cols-3">
      <Card className="p-5 lg:col-span-2">
        <Skeleton className="h-4 w-36" />
        <Skeleton className="mt-6 h-[220px] w-full" />
      </Card>
      <div className="flex flex-col gap-4">
        <Card className="p-5">
          <Skeleton className="h-4 w-24" />
          <Skeleton className="mt-4 h-8 w-16" />
          <Skeleton className="mt-4 h-3 w-full" />
          <Skeleton className="mt-2 h-3 w-3/4" />
        </Card>
        <Card className="flex-1 p-5">
          <Skeleton className="h-4 w-24" />
          <Skeleton className="mt-4 h-3 w-2/3" />
          <Skeleton className="mt-3 h-3 w-1/2" />
        </Card>
      </div>
    </div>
  );
}

function WaitingForLogs() {
  return (
    <Card className="p-6 md:p-8">
      <div className="flex flex-wrap items-center gap-3">
        <span className="live-dot h-2 w-2 rounded-full bg-emerald-400" />
        <h2 className="text-base font-semibold text-slate-100">Waiting for your first firewall logs</h2>
      </div>
      <p className="mt-2 max-w-3xl text-sm text-slate-400">
        The collector and database are ready. Point Sophos Firewall at this server and events will
        appear here within seconds — no agent required.
      </p>
      <ol className="mt-6 grid gap-4 md:grid-cols-3">
        <Step
          n="1"
          title="Open syslog settings"
          body="In SFOS go to System → Logging → Syslog servers and add a server with this host's address."
        />
        <Step
          n="2"
          title="Use port 5514"
          body="Select UDP or TCP 5514, keep the default plain (key=value) format, and enable Firewall, Content Filtering, IPS, AV and Authentication log types."
        />
        <Step
          n="3"
          title="Watch traffic land"
          body="This dashboard refreshes every 60 seconds, one request at a time to reduce database load. Use Refresh for an immediate update. Usernames appear once the firewall logs them with web or firewall events."
        />
      </ol>
      <p className="mt-6 rounded-lg border border-white/[0.06] bg-black/30 px-3 py-2 font-mono text-[11px] text-slate-400">
        Syslog server: &lt;this server IP&gt; · Port: 5514 (UDP/TCP) · Format: plain
      </p>
    </Card>
  );
}

function Step({ n, title, body }: { n: string; title: string; body: string }) {
  return (
    <li className="rounded-xl border border-white/[0.06] bg-white/[0.015] p-4">
      <span className="inline-flex h-6 w-6 items-center justify-center rounded-full border border-emerald-400/25 bg-emerald-400/10 text-[11px] font-semibold text-emerald-300">
        {n}
      </span>
      <h3 className="mt-3 text-sm font-medium text-slate-200">{title}</h3>
      <p className="mt-1 text-xs leading-relaxed text-slate-500">{body}</p>
    </li>
  );
}

function RefreshIcon() {
  return (
    <svg className="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <path d="M20 11a8 8 0 1 0-2.3 5.7" strokeLinecap="round" />
      <path d="M20 4v7h-7" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}
