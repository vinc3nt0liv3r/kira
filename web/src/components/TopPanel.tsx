import { fmtBytes, fmtNumber } from "../format";
import type { TopRow } from "../types";
import { Card, CardHeader, Empty, Skeleton } from "./ui";

const accentBar: Record<string, string> = {
  emerald: "bg-emerald-400/70",
  sky: "bg-sky-400/70",
  violet: "bg-violet-400/70",
  rose: "bg-rose-400/70",
  amber: "bg-amber-400/70",
  cyan: "bg-cyan-400/70",
};

export function TopPanel({
  title,
  hint,
  rows,
  accent = "emerald",
  loading,
  emptyHint,
  showBlocked = true,
}: {
  title: string;
  hint?: string;
  rows: TopRow[];
  accent?: keyof typeof accentBar;
  loading?: boolean;
  emptyHint?: string;
  showBlocked?: boolean;
}) {
  const max = rows[0]?.events ?? 1;
  return (
    <Card className="flex h-full flex-col">
      <CardHeader title={title} hint={hint} />
      <div className="flex-1 px-5 pb-4 pt-3">
        {rows.length === 0 ? (
          loading ? (
            <div className="space-y-3">
              {Array.from({ length: 5 }).map((_, i) => (
                <div key={i} className="flex items-center gap-3">
                  <Skeleton className="h-2.5 w-2.5 rounded-full" />
                  <Skeleton className="h-2.5 flex-1" />
                  <Skeleton className="h-2.5 w-10" />
                </div>
              ))}
            </div>
          ) : (
            <Empty title="Nothing to show" hint={emptyHint ?? "No activity in this range."} />
          )
        ) : (
          <ul className="space-y-3">
            {rows.map((row) => (
              <li key={row.key} className="group">
                <div className="flex items-baseline justify-between gap-3">
                  <span
                    className="truncate text-[13px] text-slate-300 group-hover:text-slate-100"
                    title={row.key}
                  >
                    {row.key}
                  </span>
                  <span className="tabular shrink-0 text-xs text-slate-400">
                    {fmtNumber(row.events)}
                  </span>
                </div>
                <div className="mt-1.5 flex items-center gap-2">
                  <div className="h-1 flex-1 overflow-hidden rounded-full bg-white/[0.05]">
                    <div
                      className={`h-full rounded-full ${accentBar[accent] ?? accentBar.emerald}`}
                      style={{ width: `${Math.max(2, (row.events / max) * 100)}%` }}
                    />
                  </div>
                  <span className="tabular w-[4.5rem] shrink-0 text-right text-[10px] text-slate-600">
                    {fmtBytes(row.bytes)}
                  </span>
                </div>
                {showBlocked && row.blocked > 0 ? (
                  <p className="mt-0.5 text-[10px] text-rose-400/80">{fmtNumber(row.blocked)} blocked</p>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </div>
    </Card>
  );
}
