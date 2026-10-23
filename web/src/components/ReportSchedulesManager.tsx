import { useEffect, useState } from "react";
import { api } from "../api";
import type { ReportResult, ReportScheduleResponse, CreateReportSchedule } from "../types";
import { Card, Button, Input, Select, Skeleton } from "./ui";

export default function ReportSchedulesManager() {
  const [schedules, setSchedules] = useState<ReportScheduleResponse[]>([]);
  const [results, setResults] = useState<ReportResult[]>([]);
  const [loading, setLoading] = useState(true);
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<ReportScheduleResponse | null>(null);
  const [activeScheduleId, setActiveScheduleId] = useState<string | null>(null);
  const [form, setForm] = useState<CreateReportSchedule & { id?: string }>({
    name: "",
    description: "",
    cron_expression: "0 * * * *",
    format: "pdf",
    filters: {},
    recipients: [],
    enabled: true,
  });

  useEffect(() => {
    load();
  }, []);

  const load = async () => {
    try {
      const data = await api.listReportSchedules();
      setSchedules(data.schedules);
    } catch {
      // Silent fail
    } finally {
      setLoading(false);
    }
  };

  const loadResults = async (scheduleId: string) => {
    setActiveScheduleId(scheduleId);
    try {
      const data = await api.listReportResults(scheduleId);
      setResults(data.results);
    } catch {
      setResults([]);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const payload: CreateReportSchedule = {
      name: form.name,
      description: form.description || null,
      cron_expression: form.cron_expression,
      format: form.format,
      filters: form.filters,
      recipients: form.recipients,
      enabled: form.enabled,
    };
    try {
      if (editing && form.id) {
        await api.updateReportSchedule(form.id, {
          name: form.name,
          description: form.description || null,
          cron_expression: form.cron_expression,
          format: form.format,
          filters: form.filters,
          recipients: form.recipients,
          enabled: form.enabled,
        });
      } else {
        await api.createReportSchedule(payload);
      }
      setOpen(false);
      setEditing(null);
      setForm({ name: "", description: "", cron_expression: "0 * * * *", format: "pdf", filters: {}, recipients: [], enabled: true });
      await load();
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to save schedule");
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm("Delete this report schedule?")) return;
    try {
      await api.deleteReportSchedule(id);
      await load();
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to delete schedule");
    }
  };

  if (loading) return <Skeleton className="h-64 w-full" />;

  return (
    <Card className="p-6">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-lg font-semibold text-slate-100">Report Schedules</h2>
        <Button onClick={() => { setEditing(null); setForm({ name: "", description: "", cron_expression: "0 * * * *", format: "pdf", filters: {}, recipients: [], enabled: true }); setOpen(true); }}>
          + Add Schedule
        </Button>
      </div>

      <div className="overflow-x-auto mb-4">
        <table className="w-full text-sm border-collapse">
          <thead>
            <tr className="border-b border-white/10 text-[10px] uppercase text-slate-500">
              <th className="text-left pb-2">Name</th>
              <th className="text-left pb-2">Schedule</th>
              <th className="text-left pb-2">Format</th>
              <th className="text-left pb-2">Enabled</th>
              <th className="text-left pb-2">Last Run</th>
              <th className="text-right pb-2 w-32">Actions</th>
            </tr>
          </thead>
          <tbody>
            {schedules.map((s) => (
              <tr key={s.id} className="border-b border-white/5 hover:bg-white/[0.02] cursor-pointer" onClick={() => loadResults(s.id)}>
                <td className="py-2 text-slate-200">{s.name}</td>
                <td className="py-2 text-slate-300">{s.cron_expression}</td>
                <td className="py-2 text-slate-300">{s.format}</td>
                <td className="py-2 text-slate-300">{s.enabled ? "Yes" : "No"}</td>
                <td className="py-2 text-slate-500 text-[11px]">{s.last_run ? new Date(s.last_run).toLocaleString() : "—"}</td>
                <td className="py-2 text-right">
                  <Button size="sm" onClick={(e) => { e.stopPropagation(); handleDelete(s.id); }} variant="danger">Delete</Button>
                  <Button size="sm" onClick={(e) => { e.stopPropagation(); setEditing(s); setForm({ id: s.id, name: s.name, description: s.description || "", cron_expression: s.cron_expression, format: s.format, filters: s.filters, recipients: s.recipients, enabled: s.enabled }); setOpen(true); }} variant="secondary">Edit</Button>
                </td>
              </tr>
            ))}
            {schedules.length === 0 && (
              <tr><td colSpan={6} className="py-4 text-center text-slate-500 text-[11px]">No report schedules configured.</td></tr>
            )}
          </tbody>
        </table>
      </div>

      {activeScheduleId && (
        <div className="mt-6">
          <div className="flex items-center justify-between mb-3">
            <h3 className="text-sm font-semibold text-slate-100">Report Results</h3>
            <Button size="sm" onClick={() => { setActiveScheduleId(null); setResults([]); }}>Back</Button>
          </div>
          <div className="overflow-x-auto">
            <table className="w-full text-sm border-collapse">
              <thead>
                <tr className="border-b border-white/10 text-[10px] uppercase text-slate-500">
                  <th className="text-left pb-2">Status</th>
                  <th className="text-left pb-2">File</th>
                  <th className="text-right pb-2">Rows</th>
                  <th className="text-left pb-2">Generated</th>
                  <th className="text-right pb-2 w-20">Error</th>
                </tr>
              </thead>
              <tbody>
                {results.map((r) => (
                  <tr key={r.id} className="border-b border-white/5">
                    <td className="py-2 text-slate-300 text-[11px]">{r.status}</td>
                    <td className="py-2 text-slate-200 text-[11px]">{r.file_path || "—"}</td>
                    <td className="py-2 text-right text-slate-300 text-[11px]">{r.row_count ?? "—"}</td>
                    <td className="py-2 text-slate-500 text-[11px]">{new Date(r.generated_at).toLocaleString()}</td>
                    <td className="py-2 text-right">
                      {r.error_message ? <span className="text-rose-400 text-[10px]">{r.error_message}</span> : <span className="text-emerald-400 text-[10px]">OK</span>}
                    </td>
                  </tr>
                ))}
                {results.length === 0 && (
                  <tr><td colSpan={5} className="py-4 text-center text-slate-500 text-[11px]">No report results.</td></tr>
                )}
              </tbody>
            </table>
          </div>
        </div>
      )}
      {open && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div className="bg-[#0b1220] border border-white/10 rounded-xl p-6 w-full max-w-md mx-4 shadow-2xl">
            <h3 className="text-lg font-semibold text-slate-100 mb-4">
              {editing ? "Edit Report Schedule" : "Add Report Schedule"}
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
                <label className="block text-[11px] text-slate-500 mb-1">Cron Expression</label>
                <Input placeholder="0 * * * *" value={form.cron_expression} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setForm({ ...form, cron_expression: e.target.value })} />
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Format</label>
                <Select value={form.format} onChange={(e: React.ChangeEvent<HTMLSelectElement>) => setForm({ ...form, format: e.target.value as "pdf" | "csv" })}>
                  <option value="pdf">PDF</option>
                  <option value="csv">CSV</option>
                </Select>
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Recipients (comma-separated)</label>
                <Input placeholder="admin@example.com" value={(form.recipients ?? []).join(",")} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setForm({ ...form, recipients: e.target.value.split(",").map((s: string) => s.trim()).filter(Boolean) })} />
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Filters (JSON)</label>
                <Input placeholder='{"from": "24h", "to": "now"}' value={JSON.stringify(form.filters)} onChange={(e: React.ChangeEvent<HTMLInputElement>) => { try { setForm({ ...form, filters: JSON.parse(e.target.value || "{}" ) }); } catch {} }} />
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Enabled</label>
                <select value={String(form.enabled)} onChange={(e: React.ChangeEvent<HTMLSelectElement>) => setForm({ ...form, enabled: e.target.value === "true" })} className="rounded-lg border border-white/10 bg-[#070b14] px-2 py-1.5 text-xs text-slate-200 focus:border-emerald-400/40 focus:outline-none">
                  <option value="true">Yes</option>
                  <option value="false">No</option>
                </select>
              </div>
              <div className="flex items-center justify-end gap-2">
                <Button type="button" onClick={() => { setOpen(false); setEditing(null); }} variant="secondary">Cancel</Button>
                <Button type="submit">Save Schedule</Button>
              </div>
            </form>
          </div>
        </div>
      )}
    </Card>
  );
}