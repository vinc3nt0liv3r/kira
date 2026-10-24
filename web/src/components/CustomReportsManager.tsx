import { useEffect, useState } from "react";
import { api } from "../api";
import type {
  CreateCustomReport,
  CreateReportTemplate,
  CustomReportResponse,
  ReportTemplateResponse,
} from "../types";
import { Card, Button, Input, Select, Skeleton } from "./ui";

const CATEGORIES = [
  { value: "security", label: "Security" },
  { value: "web_filter", label: "Web Filter" },
  { value: "user_activity", label: "User Activity" },
  { value: "bandwidth", label: "Bandwidth" },
  { value: "custom", label: "Custom" },
] as const;

const FORMATS = [
  { value: "pdf", label: "PDF" },
  { value: "csv", label: "CSV" },
] as const;

export default function CustomReportsManager() {
  const [templates, setTemplates] = useState<ReportTemplateResponse[]>([]);
  const [customReports, setCustomReports] = useState<CustomReportResponse[]>([]);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<"templates" | "reports" | "results">("templates");
  const [openTemplate, setOpenTemplate] = useState(false);
  const [openReport, setOpenReport] = useState(false);
  const [editingTemplate, setEditingTemplate] = useState<ReportTemplateResponse | null>(null);
  const [editingReport, setEditingReport] = useState<CustomReportResponse | null>(null);
  const [templateForm, setTemplateForm] = useState<CreateReportTemplate & { id?: string }>({
    name: "",
    description: "",
    category: "custom",
    default_filters: {},
  });
  const [reportForm, setReportForm] = useState<CreateCustomReport & { id?: string }>({
    name: "",
    template_id: "",
    description: "",
    filters: {},
    format: "pdf",
    recipients: [],
  });
  const [selectedReport, setSelectedReport] = useState<CustomReportResponse | null>(null);
  const [runResult, setRunResult] = useState<{ result_id: string; status: string; file_path: string | null; generated_at: string } | null>(null);

  useEffect(() => {
    load();
  }, []);

  const load = async () => {
    try {
      const [tplData, repData] = await Promise.all([
        api.listReportTemplates(),
        api.listCustomReports(),
      ]);
      setTemplates(tplData.templates);
      setCustomReports(repData.reports);
    } catch {
      // Silent fail
    } finally {
      setLoading(false);
    }
  };

  const handleTemplateSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const payload: CreateReportTemplate = {
      name: templateForm.name,
      description: templateForm.description || null,
      category: templateForm.category,
      default_filters: templateForm.default_filters,
    };
    try {
      if (editingTemplate && templateForm.id) {
        // Templates don't have an update endpoint yet; delete + recreate
        await api.deleteReportTemplate(templateForm.id);
      }
      await api.createReportTemplate(payload);
      setOpenTemplate(false);
      setEditingTemplate(null);
      setTemplateForm({ name: "", description: "", category: "custom", default_filters: {} });
      await load();
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to save template");
    }
  };

  const handleReportSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const payload: CreateCustomReport = {
      name: reportForm.name,
      template_id: reportForm.template_id || undefined,
      description: reportForm.description || null,
      filters: reportForm.filters,
      format: reportForm.format,
      recipients: reportForm.recipients,
    };
    try {
      if (editingReport && reportForm.id) {
        await api.updateCustomReport(reportForm.id, payload);
      } else {
        await api.createCustomReport(payload);
      }
      setOpenReport(false);
      setEditingReport(null);
      setReportForm({ name: "", template_id: "", description: "", filters: {}, format: "pdf", recipients: [] });
      await load();
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to save report");
    }
  };

  const handleDeleteTemplate = async (id: string) => {
    if (!confirm("Delete this template?")) return;
    try {
      await api.deleteReportTemplate(id);
      await load();
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to delete template");
    }
  };

  const handleDeleteReport = async (id: string) => {
    if (!confirm("Delete this custom report?")) return;
    try {
      await api.deleteCustomReport(id);
      await load();
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to delete report");
    }
  };

  const handleRunReport = async (id: string) => {
    try {
      const result = await api.runCustomReport(id);
      setRunResult(result);
      setSelectedReport(customReports.find(r => r.id === id) || null);
      await load();
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to run report");
    }
  };

  const handleDownloadResult = async (scheduleId: string, resultId: string) => {
    try {
      await api.downloadReportResult(scheduleId, resultId);
    } catch (err) {
      alert(err instanceof Error ? err.message : "Failed to download report");
    }
  };

  if (loading) return <Skeleton className="h-64 w-full" />;

  return (
    <Card className="p-6">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-lg font-semibold text-slate-100">Custom Reports</h2>
        <div className="flex gap-2">
          <Button
            onClick={() => { setActiveTab("templates"); setEditingTemplate(null); setTemplateForm({ name: "", description: "", category: "custom", default_filters: {} }); setOpenTemplate(true); }}
            size="sm"
          >
            + Template
          </Button>
          <Button
            onClick={() => { setActiveTab("reports"); setEditingReport(null); setReportForm({ name: "", template_id: "", description: "", filters: {}, format: "pdf", recipients: [] }); setOpenReport(true); }}
            size="sm"
          >
            + Report
          </Button>
        </div>
      </div>

      <div className="flex gap-2 mb-4">
        <Button
          onClick={() => setActiveTab("templates")}
          variant={activeTab === "templates" ? "default" : "secondary"}
          size="sm"
        >
          Templates
        </Button>
        <Button
          onClick={() => setActiveTab("reports")}
          variant={activeTab === "reports" ? "default" : "secondary"}
          size="sm"
        >
          Custom Reports
        </Button>
      </div>

      {activeTab === "templates" && (
        <div className="overflow-x-auto mb-6">
          <table className="w-full text-sm border-collapse">
            <thead>
              <tr className="border-b border-white/10 text-[10px] uppercase text-slate-500">
                <th className="text-left pb-2">Name</th>
                <th className="text-left pb-2">Category</th>
                <th className="text-left pb-2">Description</th>
                <th className="text-right pb-2 w-24">Actions</th>
              </tr>
            </thead>
            <tbody>
              {templates.map((t) => (
                <tr key={t.id} className="border-b border-white/5 hover:bg-white/[0.02]">
                  <td className="py-2 text-slate-200">{t.name}</td>
                  <td className="py-2 text-slate-300">{t.category}</td>
                  <td className="py-2 text-slate-300 text-[11px]">{t.description || "—"}</td>
                  <td className="py-2 text-right">
                    <Button size="sm" onClick={() => handleDeleteTemplate(t.id)} variant="danger">Delete</Button>
                    <Button size="sm" onClick={() => { setEditingTemplate(t); setTemplateForm({ id: t.id, name: t.name, description: t.description || "", category: t.category, default_filters: t.default_filters }); setOpenTemplate(true); }} variant="secondary">Edit</Button>
                  </td>
                </tr>
              ))}
              {templates.length === 0 && (
                <tr><td colSpan={4} className="py-4 text-center text-slate-500 text-[11px]">No templates configured.</td></tr>
              )}
            </tbody>
          </table>
        </div>
      )}

      {activeTab === "reports" && (
        <div className="overflow-x-auto mb-6">
          <table className="w-full text-sm border-collapse">
            <thead>
              <tr className="border-b border-white/10 text-[10px] uppercase text-slate-500">
                <th className="text-left pb-2">Name</th>
                <th className="text-left pb-2">Format</th>
                <th className="text-left pb-2">Description</th>
                <th className="text-left pb-2">Last Run</th>
                <th className="text-right pb-2 w-40">Actions</th>
              </tr>
            </thead>
            <tbody>
              {customReports.map((r) => (
                <tr key={r.id} className="border-b border-white/5 hover:bg-white/[0.02]">
                  <td className="py-2 text-slate-200">{r.name}</td>
                  <td className="py-2 text-slate-300">{r.format}</td>
                  <td className="py-2 text-slate-300 text-[11px]">{r.description || "—"}</td>
                  <td className="py-2 text-slate-500 text-[11px]">{r.last_run ? new Date(r.last_run).toLocaleString() : "—"}</td>
                  <td className="py-2 text-right">
                    <Button size="sm" onClick={() => handleRunReport(r.id)}>Run</Button>
                    <Button size="sm" onClick={() => handleDeleteReport(r.id)} variant="danger">Delete</Button>
                    <Button size="sm" onClick={() => { setEditingReport(r); setReportForm({ id: r.id, name: r.name, template_id: r.template_id || "", description: r.description || "", filters: r.filters, format: r.format, recipients: r.recipients }); setOpenReport(true); }} variant="secondary">Edit</Button>
                  </td>
                </tr>
              ))}
              {customReports.length === 0 && (
                <tr><td colSpan={5} className="py-4 text-center text-slate-500 text-[11px]">No custom reports configured.</td></tr>
              )}
            </tbody>
          </table>
        </div>
      )}

      {openTemplate && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div className="bg-[#0b1220] border border-white/10 rounded-xl p-6 w-full max-w-md mx-4 shadow-2xl">
            <h3 className="text-lg font-semibold text-slate-100 mb-4">
              {editingTemplate ? "Edit Template" : "Add Template"}
            </h3>
            <form onSubmit={handleTemplateSubmit} className="space-y-4">
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Name</label>
                <Input value={templateForm.name} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setTemplateForm({ ...templateForm, name: e.target.value })} />
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Description</label>
                <Input value={templateForm.description || ""} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setTemplateForm({ ...templateForm, description: e.target.value })} />
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Category</label>
                <Select value={templateForm.category ?? "custom"} onChange={(e: React.ChangeEvent<HTMLSelectElement>) => setTemplateForm({ ...templateForm, category: e.target.value as (typeof CATEGORIES)[number]["value"] })}>
                  {CATEGORIES.map((c) => (
                    <option key={c.value} value={c.value}>{c.label}</option>
                  ))}
                </Select>
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Default Filters (JSON)</label>
                <Input placeholder='{"from": "24h", "to": "now"}' value={JSON.stringify(templateForm.default_filters)} onChange={(e: React.ChangeEvent<HTMLInputElement>) => { try { setTemplateForm({ ...templateForm, default_filters: JSON.parse(e.target.value || "{}" ) }); } catch {} }} />
              </div>
              <div className="flex items-center justify-end gap-2">
                <Button type="button" onClick={() => { setOpenTemplate(false); setEditingTemplate(null); }} variant="secondary">Cancel</Button>
                <Button type="submit">Save Template</Button>
              </div>
            </form>
          </div>
        </div>
      )}

      {openReport && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div className="bg-[#0b1220] border border-white/10 rounded-xl p-6 w-full max-w-md mx-4 shadow-2xl">
            <h3 className="text-lg font-semibold text-slate-100 mb-4">
              {editingReport ? "Edit Custom Report" : "Add Custom Report"}
            </h3>
            <form onSubmit={handleReportSubmit} className="space-y-4">
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Name</label>
                <Input value={reportForm.name} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setReportForm({ ...reportForm, name: e.target.value })} />
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Template</label>
                <Select value={reportForm.template_id ?? ""} onChange={(e: React.ChangeEvent<HTMLSelectElement>) => setReportForm({ ...reportForm, template_id: e.target.value })}>
                  <option value="">None</option>
                  {templates.map((t) => (
                    <option key={t.id} value={t.id}>{t.name} ({t.category})</option>
                  ))}
                </Select>
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Description</label>
                <Input value={reportForm.description || ""} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setReportForm({ ...reportForm, description: e.target.value })} />
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Filters (JSON)</label>
                <Input placeholder='{"from": "24h", "to": "now"}' value={JSON.stringify(reportForm.filters)} onChange={(e: React.ChangeEvent<HTMLInputElement>) => { try { setReportForm({ ...reportForm, filters: JSON.parse(e.target.value || "{}" ) }); } catch {} }} />
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Format</label>
                <Select value={reportForm.format ?? "pdf"} onChange={(e: React.ChangeEvent<HTMLSelectElement>) => setReportForm({ ...reportForm, format: e.target.value as "pdf" | "csv" })}>
                  {FORMATS.map((f) => (
                    <option key={f.value} value={f.value}>{f.label}</option>
                  ))}
                </Select>
              </div>
              <div>
                <label className="block text-[11px] text-slate-500 mb-1">Recipients (comma-separated)</label>
                <Input placeholder="admin@example.com" value={(reportForm.recipients ?? []).join(",")} onChange={(e: React.ChangeEvent<HTMLInputElement>) => setReportForm({ ...reportForm, recipients: e.target.value.split(",").map((s: string) => s.trim()).filter(Boolean) })} />
              </div>
              <div className="flex items-center justify-end gap-2">
                <Button type="button" onClick={() => { setOpenReport(false); setEditingReport(null); }} variant="secondary">Cancel</Button>
                <Button type="submit">Save Report</Button>
              </div>
            </form>
          </div>
        </div>
      )}

      {runResult && selectedReport && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div className="bg-[#0b1220] border border-white/10 rounded-xl p-6 w-full max-w-md mx-4 shadow-2xl">
            <h3 className="text-lg font-semibold text-slate-100 mb-4">Report Generated</h3>
            <div className="space-y-3 text-sm">
              <div className="flex justify-between">
                <span className="text-slate-500">Report:</span>
                <span className="text-slate-200">{selectedReport.name}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-500">Status:</span>
                <span className={`text-slate-200 ${runResult.status === "generated" ? "text-emerald-400" : "text-rose-400"}`}>{runResult.status}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-slate-500">Generated:</span>
                <span className="text-slate-200 text-[11px]">{new Date(runResult.generated_at).toLocaleString()}</span>
              </div>
            </div>
            <div className="mt-6 flex items-center justify-end gap-2">
              <Button type="button" onClick={() => { setRunResult(null); setSelectedReport(null); }} variant="secondary">Close</Button>
              {runResult.file_path && (
                <Button onClick={() => { handleDownloadResult(selectedReport.id, runResult.result_id); setRunResult(null); setSelectedReport(null); }}>Download</Button>
              )}
            </div>
          </div>
        </div>
      )}
    </Card>
  );
}