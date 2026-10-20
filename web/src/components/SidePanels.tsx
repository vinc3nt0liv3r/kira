import { fmtAgo, fmtNumber } from "../format";
import type { Meta, TopRow } from "../types";
import { Badge, Card, CardHeader, Empty, Skeleton } from "./ui";

export function SecurityPanel({
  total,
  rows,
  loading,
}: {
  total: number;
  rows: TopRow[];
  loading: boolean;
}) {
  return (
    <Card>
      <CardHeader
        title="Security"
        hint="Threats detected in range"
        right={
          <Badge tone={total > 0 ? "rose" : "emerald"}>{total > 0 ? "attention" : "clear"}</Badge>
        }
      />
      <div className="px-5 pb-4 pt-2">
        <p className="tabular text-3xl font-semibold tracking-tight text-slate-50">
          {fmtNumber(total)}
        </p>
        <div className="mt-3 space-y-1.5">
          {rows.length === 0 ? (
            loading ? (
              <Skeleton className="h-3 w-40" />
            ) : (
              <p className="text-xs text-slate-500">No IPS, AV, ATP or WAF events.</p>
            )
          ) : (
            rows.slice(0, 4).map((row) => (
              <div key={row.key} className="flex items-center justify-between gap-3 text-xs">
                <span className="truncate text-slate-300" title={row.key}>
                  {row.key}
                </span>
                <span className="tabular shrink-0 text-slate-500">{fmtNumber(row.events)}</span>
              </div>
            ))
          )}
        </div>
      </div>
    </Card>
  );
}

export function CollectorPanel({ meta }: { meta: Meta | null }) {
  return (
    <Card className="flex-1">
      <CardHeader
        title="Collector"
        hint="Firewalls seen in the last 5m"
        right={
          <span className="inline-flex items-center gap-1.5 text-[10px] uppercase tracking-wide text-slate-500">
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                meta?.last_event_at ? "bg-emerald-400 live-dot" : "bg-slate-600"
              }`}
            />
            {meta?.last_event_at ? "receiving" : "idle"}
          </span>
        }
      />
      <div className="px-5 pb-4 pt-2">
        <p className="text-xs text-slate-500">
          Last event <span className="text-slate-300">{fmtAgo(meta?.last_event_at ?? null)}</span>
          {meta ? (
            <span className="text-slate-600"> · {meta.tenant.retention_days}d retention</span>
          ) : null}
        </p>
        <div className="mt-3 space-y-2">
          {!meta ? (
            <Skeleton className="h-4 w-48" />
          ) : meta.devices.length === 0 ? (
            <Empty title="No devices yet" hint="Logs will identify the firewall automatically." />
          ) : (
            meta.devices.slice(0, 4).map((device) => {
              const identity = [device.model, device.firmware ? `SFOS ${device.firmware}` : null]
                .filter(Boolean)
                .join(" · ");
              return (
                <div
                  key={`${device.name}-${device.device_id ?? ""}`}
                  className="rounded-lg border border-white/[0.05] bg-white/[0.015] px-2.5 py-2"
                >
                  <div className="flex items-center justify-between gap-2">
                    <p className="min-w-0 truncate text-xs font-medium text-slate-200" title={device.name}>
                      {device.name}
                    </p>
                    <span className="tabular shrink-0 text-[10px] text-slate-500">
                      {fmtNumber(device.events)} ev
                    </span>
                  </div>
                  <p className="mt-1 truncate text-[10px] text-slate-500" title={device.device_id ?? undefined}>
                    {identity || "Sophos Firewall"}
                    {device.device_id && device.device_id !== device.name
                      ? ` · ${device.device_id}`
                      : null}
                  </p>
                  <p className="mt-0.5 text-[10px] text-slate-600">last event {fmtAgo(device.last_seen)}</p>
                </div>
              );
            })
          )}
        </div>
      </div>
    </Card>
  );
}
