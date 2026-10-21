import { fmtBytes, fmtNumber, fmtPercent } from "../format";
import type { Totals } from "../types";
import { Card, Skeleton } from "./ui";

interface Kpi {
  label: string;
  value: string;
  hint: string;
  accent: string;
}

export function KpiCards({ totals, loading }: { totals: Totals | null; loading: boolean }) {
  if (!totals) {
    return (
      <div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
        {Array.from({ length: 6 }).map((_, i) => (
          <Card key={i} className="p-4">
            <Skeleton className="h-3 w-20" />
            <Skeleton className="mt-3 h-7 w-16" />
            <Skeleton className="mt-2 h-2.5 w-24" />
          </Card>
        ))}
      </div>
    );
  }

  const identified = totals.allowed + totals.blocked;
  const kpis: Kpi[] = [
    {
      label: "Unique identities",
      value: fmtNumber(totals.users),
      hint: totals.users > 0 ? "usernames or fallback source IPs" : "no usernames or source IPs logged",
      accent: "bg-sky-400",
    },
    {
      label: "Web requests",
      value: fmtNumber(totals.web_requests),
      hint: `${fmtNumber(totals.domains)} destinations`,
      accent: "bg-emerald-400",
    },
    {
      label: "Blocked",
      value: fmtNumber(totals.blocked),
      hint: `${fmtPercent(totals.blocked, identified)} of decisions`,
      accent: "bg-rose-400",
    },
    {
      label: "Downloaded",
      value: fmtBytes(totals.bytes_down),
      hint: "recv_bytes from logs",
      accent: "bg-cyan-400",
    },
    {
      label: "Uploaded",
      value: fmtBytes(totals.bytes_up),
      hint: "sent_bytes from logs",
      accent: "bg-violet-400",
    },
    {
      label: "Threats",
      value: fmtNumber(totals.threats),
      hint: "IPS, AV, ATP, WAF",
      accent: "bg-amber-400",
    },
  ];

  return (
    <div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
      {kpis.map((kpi) => (
        <Card key={kpi.label} className={`relative overflow-hidden p-4 ${loading ? "opacity-70" : ""}`}>
          <span className={`absolute left-0 top-0 h-full w-[2px] ${kpi.accent} opacity-60`} />
          <p className="text-[11px] font-medium uppercase tracking-wider text-slate-500">{kpi.label}</p>
          <p className="tabular mt-1.5 text-2xl font-semibold tracking-tight text-slate-50">{kpi.value}</p>
          <p className="mt-1 truncate text-[11px] text-slate-500">{kpi.hint}</p>
        </Card>
      ))}
    </div>
  );
}
