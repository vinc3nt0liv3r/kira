import {
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { fmtBytes, fmtClock, fmtNumber, fmtRangeTick } from "../format";
import type { SeriesPoint } from "../types";
import { Card, CardHeader, Empty } from "./ui";

export type TrafficMode = "requests" | "volume";

interface TooltipEntry {
  dataKey?: string | number;
  name?: string;
  value?: number | string;
  color?: string;
}

function ChartTooltip({
  active,
  payload,
  label,
  mode,
}: {
  active?: boolean;
  payload?: TooltipEntry[];
  label?: string | number;
  mode: TrafficMode;
}) {
  if (!active || !payload || payload.length === 0) return null;
  return (
    <div className="rounded-lg border border-white/10 bg-[#0b1220]/95 px-3 py-2 shadow-xl backdrop-blur">
      <p className="mb-1 text-[11px] text-slate-400">{fmtClock(String(label))}</p>
      {payload.map((entry) => (
        <p key={String(entry.dataKey)} className="flex items-center gap-2 text-xs text-slate-200">
          <span
            className="inline-block h-2 w-2 rounded-sm"
            style={{ backgroundColor: entry.color ?? "#94a3b8" }}
          />
          <span className="text-slate-400">{entry.name}</span>
          <span className="tabular ml-auto font-medium">
            {mode === "volume" ? fmtBytes(Number(entry.value)) : fmtNumber(Number(entry.value))}
          </span>
        </p>
      ))}
    </div>
  );
}

export function TrafficChart({
  series,
  mode,
  onMode,
  spanHours,
}: {
  series: SeriesPoint[];
  mode: TrafficMode;
  onMode: (mode: TrafficMode) => void;
  spanHours: number;
}) {
  const volume = mode === "volume";

  return (
    <Card className="flex h-full flex-col">
      <CardHeader
        title="Traffic over time"
        hint={
          volume
            ? "Bytes transferred per time bucket (volume, not throughput)"
            : "Logged requests by decision"
        }
        right={
          <div className="flex rounded-lg border border-white/10 bg-black/20 p-0.5 text-[11px]">
            {(["requests", "volume"] as const).map((option) => (
              <button
                key={option}
                type="button"
                onClick={() => onMode(option)}
                className={`rounded-md px-2 py-1 capitalize transition ${
                  mode === option
                    ? "bg-white/10 text-slate-100"
                    : "text-slate-500 hover:text-slate-300"
                }`}
              >
                {option === "volume" ? "data volume" : option}
              </button>
            ))}
          </div>
        }
      />

      {series.length === 0 ? (
        <Empty title="No traffic in this range" hint="Choose a wider time range or wait for logs." />
      ) : (
        <div className="mt-3 h-[268px] w-full pr-3">
          <ResponsiveContainer width="100%" height="100%">
            <LineChart data={series} margin={{ top: 6, right: 6, left: 0, bottom: 0 }}>
              <CartesianGrid stroke="#1e293b" strokeDasharray="3 3" vertical={false} />
              <XAxis
                dataKey="t"
                tickFormatter={(value: string) => fmtRangeTick(value, spanHours)}
                tick={{ fill: "#64748b", fontSize: 11 }}
                tickLine={false}
                axisLine={{ stroke: "#1e293b" }}
                minTickGap={32}
                interval="preserveStartEnd"
              />
              <YAxis
                tickFormatter={(value: number) => (volume ? fmtBytes(value) : fmtNumber(value))}
                tick={{ fill: "#64748b", fontSize: 11 }}
                tickLine={false}
                axisLine={false}
                width={58}
              />
              <Tooltip
                content={<ChartTooltip mode={mode} />}
                cursor={{ stroke: "#334155", strokeDasharray: "3 3" }}
              />
              {volume ? (
                <>
                  <Line
                    type="linear"
                    dataKey="bytes_down"
                    name="Downloaded"
                    stroke="#22d3ee"
                    strokeWidth={1.6}
                    isAnimationActive={false}
                    dot={false}
                  />
                  <Line
                    type="linear"
                    dataKey="bytes_up"
                    name="Uploaded"
                    stroke="#a78bfa"
                    strokeWidth={1.6}
                    isAnimationActive={false}
                    dot={false}
                  />
                </>
              ) : (
                <>
                  <Line
                    type="linear"
                    dataKey="allowed"
                    name="Allowed"
                    stroke="#34d399"
                    strokeWidth={1.6}
                    isAnimationActive={false}
                    dot={false}
                  />
                  <Line
                    type="linear"
                    dataKey="blocked"
                    name="Blocked"
                    stroke="#fb7185"
                    strokeWidth={1.6}
                    isAnimationActive={false}
                    dot={false}
                  />
                </>
              )}
            </LineChart>
          </ResponsiveContainer>
        </div>
      )}
    </Card>
  );
}
