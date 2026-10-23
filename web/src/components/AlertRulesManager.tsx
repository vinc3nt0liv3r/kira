import { useEffect, useState } from "react";
import { api } from "../api";
import type { AlertInstance, AlertRuleResponse, CreateAlertRule } from "../types";
import { Card, Button, Input, Select, Skeleton } from "./ui";

const CONDITION_TYPES = [
  { value: "threat", label: "Threat detected" },
  { value: "blocked_category", label: "Blocked web category" },
  { value: "large_transfer", label: "Large data transfer" },
] as const;

const SEVERITIES = ["low", "medium", "high", "critical"] as const;

export default function AlertRulesManager() {
  const [rules, setRules] = useState<AlertRuleResponse[]>([]);
  const [instances, setInstances] = useState<AlertInstance[]>([]);
  const [loading, setLoading] = useState(true);
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<AlertRuleResponse | null>(null);
  const [form, setForm] = useState<CreateAlertRule & { id?: string }>({
    name: "",
    description: "",
    condition_type: "threat",
    threshold: "",
    severity: "medium",
    cooldown_minutes: 60,
    enabled: true,
    webhook_url: "",
  });

  useEffect(() => {
    load();
  }, []);

  const load = async () => {
    try {
      const [rulesData, instancesData] = await Promise.all([
        api.listAlertRules(),
        api.listAlertInstances(),
      ]);
      setRules(rulesData.rules);
      setInstances(instancesData.instances);
    } catch {
      // Silent fail - data loads on demand
    } finally {
      setLoading(false);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const payload: CreateAlertRule = {
      name: form.name,
      description: form.description || null,
      condition_type: form.condition_type,
      threshold: form.threshold,
      severity: form.severity,
      cooldown_minutes: form.cooldown_minutes,
      enabled: form.enabled,
      webhook_url: form.webhook_url || null,
    };
    try {
      if (editing && form.id) {
        await api.updateAlertRule(form.id, {
          name: form.name,
          description: form.description || null,
          condition_type: form.condition_type,
          threshold: form.threshold,
          severity: form.severity,
          cooldown_minutes: form.cooldown_minutes,
          enabled: form.enabled,
          webhook_url: form.webhook_url || null,
        });
      } else {
        await api.createAlertRule(payload);
      }
      setOpen(false);
      setEditing(null);
      setForm({
        name: "",
        description: "",
        condition_type: "threat",
        threshold: "",
        severity: "medium",
        cooldown_minutes: 60,
        enabled: true,
        webhook_url: "",
      });
      await load();
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to save rule");
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm("Delete this alert rule?")) return;
    try {
      await api.deleteAlertRule(id);
      await load();
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to delete rule");
    }
  };

  const handleAcknowledge = async (id: string) => {
    try {
      await api.acknowledgeAlertInstance(id, "dashboard");
      await load();
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to acknowledge");
    }
  };

  if (loading) return <Skeleton className="h-64 w-full" />;

  return (
    <Card className="p-6">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-lg font-semibold text-slate-100">Alert Rules</h2>
        <Button onClick={() => { setEditing(null); setForm({ name: "", description: "", condition_type: "threat", threshold: "", severity: "medium", cooldown_minutes: 60, enabled: true, webhook_url: "" }); setOpen(true); }}>
          + Add Rule
        </Button>
      </div>

      <div className="overflow-x-auto mb-4">
        <table className="w-full text-sm border-collapse">
          <thead>
            <tr className="border-b border-white/10 text-[10px] uppercase text-slate-500">
              <th className="text-left pb-2">Name</th>
              <th className="text-left pb-2">Condition</th>
              <th className="text-left pb-2">Threshold</th>
              <th className="text-left pb-2">Severity</th>
              <th className="text-left pb-2">Webhook</th>
              <th className="text-left pb-2">Enabled</th>
              <th className="text-right pb-2">Last Triggered</th>
              <th className="text-right pb-2 w-24">Actions</th>
            </tr>
          </thead>
          <tbody>
            {rules.map((rule) => (
              <tr key={rule.id} className="border-b border-white/5 hover:bg-white/[0.02]">
                <td className="py-2 text-slate-200">{rule.name}</td>
                <td className="py-2 text-slate-300">{rule.condition_type}</td>
                <td className="py-2 text-slate-300">{rule.threshold}</td>
                <td className="py-2">
                  <span className={`px-2 py-0.5 rounded text-[10px] ${
                    rule.severity === "critical" ? "bg-rose-500/20 text-rose-400" :
                    rule.severity === "high" ? "bg-orange-500/20 text-orange-400" :
                    rule.severity === "medium" ? "bg-amber-500/20 text-amber-400" :
                    "bg-emerald-500/20 text-emerald-400"
                  }`}>{rule.severity}</span>
                </td>
                <td className="py-2 text-slate-300">{rule.webhook_url || "—"}</td>
                <td className="py-2 text-slate-300">{rule.enabled ? "Yes" : "No"}</td>
                <td className="py-2 text-right text-slate-500 text-[11px]">
                  {rule.last_triggered ? new Date(rule.last_triggered).toLocaleString() : "—"}
                </td>
                <td className="py-2 text-right">
                  <Button size="sm" onClick={() => handleDelete(rule.id)} variant="danger">Delete</Button>
                  <Button size="sm" onClick={() => { setEditing(rule); setForm({ id: rule.id, name: rule.name, description: rule.description || "", condition_type: rule.condition_type, threshold: rule.threshold, severity: rule.severity, cooldown_minutes: rule.cooldown_minutes, enabled: rule.enabled, webhook_url: rule.webhook_url || "" }); setOpen(true); }} variant="secondary">Edit</Button>
                </td>
              </tr>
            ))}
            {rules.length === 0 && (
              <tr><td colSpan={8} className="py-4 text-center text-slate-500 text-[11px]">No alert rules configured.</td></tr>
            )}
          </tbody>
        </table>
      </div>

      <div className="mt-6">
        <h3 className="text-sm font-semibold text-slate-100 mb-3">Recent Alert Instances</h3>
        <div className="overflow-x-auto">
          <table className="w-full text-sm border-collapse">
            <thead>
              <tr className="border-b border-white/10 text-[10px] uppercase text-slate-500">
                <th className="text-left pb-2">Rule</th>
                <th className="text-left pb-2">Message</th>
                <th className="text-left pb-2">Severity</th>
                <th className="text-left pb-2">Triggered</th>
                <th className="text-left pb-2">Status</th>
                <th className="text-right pb-2 w-20">Actions</th>
              </tr>
            </thead>
            <tbody>
              {instances.slice(0, 10).map((inst) => (
                <tr key={inst.id} className="border-b border-white/5">
                  <td className="py-2 text-slate-300 text-[11px]">{inst.rule_id}</td>
                  <td className="py-2 text-slate-200 text-[11px]">{inst.message}</td>
                  <td className="py-2 text-slate-300 text-[11px]">{inst.severity}</td>
                  <td className="py-2 text-slate-500 text-[11px]">{new Date(inst.triggered_at).toLocaleString()}</td>
                  <td className="py-2 text-slate-300 text-[11px]">{inst.acknowledged ? "Acknowledged" : "Active"}</td>
                  <td className="py-2 text-right">
                    {!inst.acknowledged && (
                      <Button size="sm" onClick={() => handleAcknowledge(inst.id)}>Acknowledge</Button>
                    )}
                  </td>
                </tr>
              ))}
              {instances.length === 0 && (
                <tr><td colSpan={6} className="py-4 text-center text-slate-500 text-[11px]">No alert instances.</td></tr>
              )}
            </tbody>
          </table>
        </div>
      </div>

      {open && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div className="bg-[#0b1220] border border-white/10 rounded-xl p-6 w-full max-w-md mx-4 shadow-2xl">
            <h3 className="text-lg font-semibold text-slate-100 mb-4">
              {editing ? "Edit Alert Rule" : "Add Alert Rule"}
            </h3>
            <form onSubmit={handleSubmit} className="space-y-4">
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Name</label>
                <Input value={form.name} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setForm({ ...form, name: e.target.value })} />
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Description</label>
                <Input value={form.description || ""} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setForm({ ...form, description: e.target.value })} />
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Condition Type</label>
                <Select value={form.condition_type} onChange={(e: React.ChangeEvent<HTMLSelectElement>) => setForm({ ...form, condition_type: e.target.value as "threat" | "blocked_category" | "large_transfer" })}>
                  {CONDITION_TYPES.map((c) => (
                    <option key={c.value} value={c.value}>{c.label}</option>
                  ))}
                </Select>
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Threshold</label>
                <Input type="number" value={form.threshold} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setForm({ ...form, threshold: e.target.value })} />
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Severity</label>
                <Select value={form.severity ?? "medium"} onChange={(e: React.ChangeEvent<HTMLSelectElement>) => setForm({ ...form, severity: e.target.value as "low" | "medium" | "high" | "critical" })}>
                  {SEVERITIES.map((s) => (
                    <option key={s} value={s}>{s}</option>
                  ))}
                </Select>
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Cooldown (minutes)</label>
                <Input type="number" value={String(form.cooldown_minutes ?? 60)} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setForm({ ...form, cooldown_minutes: Number(e.target.value) || 60 })} />
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Enabled</label>
                <select value={String(form.enabled)} onChange={(e: React.ChangeEvent<HTMLSelectElement>) => setForm({ ...form, enabled: e.target.value === "true" })} className="rounded-lg border border-white/10 bg-[#070b14] px-2 py-1.5 text-xs text-slate-200 focus:border-emerald-400/40 focus:outline-none">
                  <option value="true">Yes</option>
                  <option value="false">No</option>
                </select>
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Webhook URL</label>
                <Input placeholder="https://hooks.example.com/alerts" value={form.webhook_url ?? ""} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setForm({ ...form, webhook_url: e.target.value })} />
              </div>
              <div className="flex items-center justify-end gap-2">
                <Button type="button" onClick={() => { setOpen(false); setEditing(null); }} variant="secondary">Cancel</Button>
                <Button type="submit">Save Rule</Button>
              </div>
            </form>
          </div>
        </div>
      )}
    </Card>
  );
}