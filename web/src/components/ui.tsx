import type { ReactNode } from "react";

export function Card({
  className = "",
  children,
}: {
  className?: string;
  children: ReactNode;
}) {
  return (
    <section
      className={`rounded-2xl border border-white/[0.06] bg-[#0b1220]/70 shadow-[inset_0_1px_0_rgba(255,255,255,0.03)] ${className}`}
    >
      {children}
    </section>
  );
}

export function CardHeader({
  title,
  hint,
  right,
}: {
  title: string;
  hint?: string;
  right?: ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-3 px-5 pt-4">
      <div>
        <h2 className="text-sm font-semibold tracking-tight text-slate-100">{title}</h2>
        {hint ? <p className="mt-0.5 text-xs text-slate-500">{hint}</p> : null}
      </div>
      {right}
    </div>
  );
}

type Tone = "emerald" | "rose" | "amber" | "sky" | "violet" | "slate" | "cyan";

const badgeTones: Record<Tone, string> = {
  emerald: "border-emerald-400/25 bg-emerald-400/10 text-emerald-300",
  rose: "border-rose-400/25 bg-rose-400/10 text-rose-300",
  amber: "border-amber-400/25 bg-amber-400/10 text-amber-300",
  sky: "border-sky-400/25 bg-sky-400/10 text-sky-300",
  violet: "border-violet-400/25 bg-violet-400/10 text-violet-300",
  slate: "border-slate-400/20 bg-slate-400/10 text-slate-300",
  cyan: "border-cyan-400/25 bg-cyan-400/10 text-cyan-300",
};

export function Badge({
  children,
  tone = "slate",
}: {
  children: ReactNode;
  tone?: Tone;
}) {
  return (
    <span
      className={`inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide ${badgeTones[tone]}`}
    >
      {children}
    </span>
  );
}

const actionTone: Record<string, Tone> = {
  allowed: "emerald",
  blocked: "rose",
  warned: "amber",
};

export function ActionBadge({ action }: { action: string | null }) {
  if (!action) return <span className="text-xs text-slate-600">—</span>;
  return <Badge tone={actionTone[action] ?? "slate"}>{action}</Badge>;
}

const groupTone: Record<string, Tone> = {
  web: "sky",
  threat: "rose",
  firewall: "violet",
  auth: "amber",
  vpn: "cyan",
  system: "slate",
  other: "slate",
};

export function GroupBadge({ group }: { group: string }) {
  return <Badge tone={groupTone[group] ?? "slate"}>{group}</Badge>;
}

export function Empty({ title, hint }: { title: string; hint?: string }) {
  return (
    <div className="flex flex-col items-center justify-center gap-1 px-6 py-10 text-center">
      <p className="text-sm text-slate-400">{title}</p>
      {hint ? <p className="max-w-sm text-xs text-slate-600">{hint}</p> : null}
    </div>
  );
}

export function Skeleton({ className = "" }: { className?: string }) {
  return <div className={`animate-pulse rounded-md bg-white/[0.05] ${className}`} />;
}

export function Spinner() {
  return (
    <svg className="h-3.5 w-3.5 animate-spin text-slate-400" viewBox="0 0 24 24" fill="none">
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeOpacity="0.25" strokeWidth="3" />
      <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
    </svg>
  );
}

export function Button({
  children,
  className = "",
  onClick,
  type = "button",
  variant = "default",
  size = "md",
  disabled,
}: {
  children?: React.ReactNode;
  className?: string;
  onClick?: (event: React.MouseEvent<HTMLButtonElement>) => void;
  type?: "button" | "submit" | "reset";
  variant?: "default" | "secondary" | "danger";
  size?: "sm" | "md";
  disabled?: boolean;
}) {
  const base = "inline-flex items-center justify-center rounded-lg border border-white/10 px-3 py-1.5 text-xs font-medium transition focus:outline-none focus:ring-2 focus:ring-emerald-400/50 disabled:opacity-50 disabled:pointer-events-none";
  const variants: Record<string, string> = {
    default: "bg-white/[0.03] text-slate-300 hover:bg-white/[0.08]",
    secondary: "bg-white/[0.03] text-slate-400 hover:text-slate-200 hover:bg-white/[0.06]",
    danger: "bg-rose-500/10 border-rose-500/30 text-rose-300 hover:bg-rose-500/20",
  };
  const sizes: Record<string, string> = {
    sm: "px-2 py-1",
    md: "px-3 py-1.5",
  };
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled}
      className={`${base} ${variants[variant]} ${sizes[size]} ${className}`}
    >
      {children}
    </button>
  );
}

export function Input({
  className = "",
  value,
  onChange,
  type = "text",
  placeholder,
}: {
  className?: string;
  value?: string;
  onChange?: (e: React.ChangeEvent<HTMLInputElement>) => void;
  type?: string;
  placeholder?: string;
}) {
  return (
    <input
      type={type}
      value={value ?? ""}
      onChange={onChange}
      placeholder={placeholder}
      className={`rounded-lg border border-white/10 bg-[#070b14] px-2.5 py-1.5 text-xs text-slate-200 focus:border-emerald-400/40 focus:outline-none ${className}`}
    />
  );
}

export function Select({
  className = "",
  value,
  onChange,
  children,
}: {
  className?: string;
  value: string;
  onChange?: (e: React.ChangeEvent<HTMLSelectElement>) => void;
  children: React.ReactNode;
}) {
  return (
    <select
      value={value}
      onChange={onChange}
      className={`rounded-lg border border-white/10 bg-[#070b14] px-2.5 py-1.5 text-xs text-slate-200 focus:border-emerald-400/40 focus:outline-none ${className}`}
    >
      {children}
    </select>
  );
}
