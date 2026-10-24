import { useCallback, useEffect, useState } from "react";
import { Input } from "./ui";

interface ReportFilterBuilderProps {
  filters: Record<string, unknown>;
  onChange: (filters: Record<string, unknown>) => void;
  className?: string;
}

const TIME_PRESETS = [
  { label: "Last hour", value: "1h" },
  { label: "Last 24 hours", value: "24h" },
  { label: "Last 7 days", value: "7d" },
  { label: "Last 30 days", value: "30d" },
  { label: "Custom range", value: "custom" },
] as const;

const ACTIONS = [
  { value: "allowed", label: "Allowed" },
  { value: "blocked", label: "Blocked" },
  { value: "warned", label: "Warned" },
] as const;

const GROUPS = [
  { value: "web", label: "Web" },
  { value: "threat", label: "Threat" },
  { value: "firewall", label: "Firewall" },
  { value: "vpn", label: "VPN" },
  { value: "auth", label: "Auth" },
  { value: "system", label: "System" },
  { value: "other", label: "Other" },
] as const;

export function ReportFilterBuilder({
  filters,
  onChange,
  className,
}: ReportFilterBuilderProps) {
  const [timePreset, setTimePreset] = useState<
    (typeof TIME_PRESETS)[number]["value"]
  >("24h");
  const [customFrom, setCustomFrom] = useState("");
  const [customTo, setCustomTo] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedActions, setSelectedActions] = useState<string[]>([]);
  const [selectedGroups, setSelectedGroups] = useState<string[]>([]);
  const [categoryFilter, setCategoryFilter] = useState("");
  const [deviceFilter, setDeviceFilter] = useState("");
  const [userFilter, setUserFilter] = useState("");

  // Initialize from existing filters
  const initializeFromFilters = useCallback(() => {
    if (filters.from) setTimePreset("custom");
    if (filters.q) setSearchQuery(String(filters.q));
    if (filters.action && Array.isArray(filters.action))
      setSelectedActions(filters.action);
    else if (filters.action) setSelectedActions([String(filters.action)]);
    if (filters.group && Array.isArray(filters.group))
      setSelectedGroups(filters.group);
    else if (filters.group) setSelectedGroups([String(filters.group)]);
    if (filters.category) setCategoryFilter(String(filters.category));
    if (filters.device) setDeviceFilter(String(filters.device));
    if (filters.user) setUserFilter(String(filters.user));
  }, [filters]);

useEffect(initializeFromFilters, [filters]);

  const handleChange = useCallback(() => {
    const next: Record<string, unknown> = {};

    // Time range
    if (timePreset === "custom") {
      if (customFrom) next.from = customFrom;
      if (customTo) next.to = customTo;
    } else {
      next.from = timePreset;
    }

    // Search
    if (searchQuery) next.q = searchQuery;

    // Action
    if (selectedActions.length > 0) {
      next.action = selectedActions.length === 1 ? selectedActions[0] : selectedActions;
    }

    // Group
    if (selectedGroups.length > 0) {
      next.group = selectedGroups.length === 1 ? selectedGroups[0] : selectedGroups;
    }

    // Category
    if (categoryFilter) next.category = categoryFilter;

    // Device
    if (deviceFilter) next.device = deviceFilter;

    // User
    if (userFilter) next.user = userFilter;

    onChange(next);
  }, [
    timePreset, customFrom, customTo, searchQuery, selectedActions,
    selectedGroups, categoryFilter, deviceFilter, userFilter, onChange,
  ]);

  const toggleAction = (value: string) => {
    setSelectedActions((prev) =>
      prev.includes(value) ? prev.filter((v) => v !== value) : [...prev, value]
    );
  };

  const toggleGroup = (value: string) => {
    setSelectedGroups((prev) =>
      prev.includes(value) ? prev.filter((v) => v !== value) : [...prev, value]
    );
  };

  const isCustom = timePreset === "custom";

  return (
    <div className={className ?? ""}>
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4 mb-4">
        {/* Time range */}
        <div className="col-span-1 sm:col-span-2 lg:col-span-1">
          <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1">
            Time range
          </label>
          <div className="flex flex-wrap gap-2">
            {TIME_PRESETS.map((p) => (
              <button
                key={p.value}
                type="button"
                onClick={() => {
                  setTimePreset(p.value);
                  if (p.value !== "custom") {
                    setCustomFrom("");
                    setCustomTo("");
                  }
                }}
                className={`rounded-lg border px-2 py-1.5 text-xs transition-colors ${
                  timePreset === p.value
                    ? "border-emerald-400/50 bg-emerald-500/10 text-emerald-300"
                    : "border-white/10 bg-white/[0.03] text-slate-300 hover:text-slate-100"
                }`}
              >
                {p.label}
              </button>
            ))}
          </div>
          {isCustom && (
            <div className="mt-2 grid grid-cols-2 gap-2">
              <Input
                type="datetime-local"
                value={customFrom}
                onChange={(e) => setCustomFrom(e.target.value)}
                placeholder="From"
              />
              <Input
                type="datetime-local"
                value={customTo}
                onChange={(e) => setCustomTo(e.target.value)}
                placeholder="To"
              />
            </div>
          )}
        </div>

        {/* Search */}
        <div>
          <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1">
            Search
          </label>
          <Input
            placeholder="User, domain, URL, IP, rule…"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
        </div>

        {/* Category */}
        <div>
          <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1">
            Category
          </label>
          <Input
            placeholder="e.g. web, threat, malware"
            value={categoryFilter}
            onChange={(e) => setCategoryFilter(e.target.value)}
          />
        </div>

        {/* Device */}
        <div>
          <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1">
            Device
          </label>
          <Input
            placeholder="Device name"
            value={deviceFilter}
            onChange={(e) => setDeviceFilter(e.target.value)}
          />
        </div>

        {/* User */}
        <div>
          <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1">
            User
          </label>
          <Input
            placeholder="Username"
            value={userFilter}
            onChange={(e) => setUserFilter(e.target.value)}
          />
        </div>
      </div>

      <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 mb-4">
        {/* Actions */}
        <div>
          <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1">
            Action
          </label>
          <div className="flex flex-wrap gap-1">
            {ACTIONS.map((a) => (
              <button
                key={a.value}
                type="button"
                onClick={() => toggleAction(a.value)}
                className={`rounded-lg border px-2 py-1 text-xs transition-colors ${
                  selectedActions.includes(a.value)
                    ? "border-emerald-400/50 bg-emerald-500/10 text-emerald-300"
                    : "border-white/10 bg-white/[0.03] text-slate-300 hover:text-slate-100"
                }`}
              >
                {a.label}
              </button>
            ))}
          </div>
        </div>

        {/* Groups */}
        <div className="col-span-1 sm:col-span-2">
          <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1">
            Log type
          </label>
          <div className="flex flex-wrap gap-1">
            {GROUPS.map((g) => (
              <button
                key={g.value}
                type="button"
                onClick={() => toggleGroup(g.value)}
                className={`rounded-lg border px-2 py-1 text-xs transition-colors ${
                  selectedGroups.includes(g.value)
                    ? "border-emerald-400/50 bg-emerald-500/10 text-emerald-300"
                    : "border-white/10 bg-white/[0.03] text-slate-300 hover:text-slate-100"
                }`}
              >
                {g.label}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="flex items-end justify-between">
        <button
          type="button"
          onClick={handleChange}
          className="rounded-lg border border-white/10 bg-white/[0.03] px-3 py-1.5 text-xs text-slate-300 hover:text-slate-100"
        >
          Apply filters
        </button>
      </div>
    </div>
  );
}
