import { useState } from "react";
import { createPortal } from "react-dom";
import { api } from "../api";
import { fmtBytes } from "../format";
import type { RetentionPreview } from "../types";
import { Spinner } from "./ui";

export function RetentionControl() {
  const [open, setOpen] = useState(false);
  const [preview, setPreview] = useState<RetentionPreview | null>(null);
  const [selectedHours, setSelectedHours] = useState(6);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const loadPreview = async () => {
    setBusy(true);
    setError(null);
    try {
      const result = await api.retention();
      setPreview(result);
      if (result.choices_hours.includes(result.current_hours)) {
        setSelectedHours(result.current_hours);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const show = () => {
    setOpen(true);
    setMessage(null);
    void loadPreview();
  };

  const close = () => {
    setOpen(false);
    setToken("");
    setError(null);
  };

  const apply = async () => {
    if (!token || !preview?.admin_configured) return;
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const result = await api.updateRetention(selectedHours, token);
      setMessage(result.message);
      setToken("");
      await loadPreview();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <button
        type="button"
        onClick={show}
        className="inline-flex items-center gap-1.5 rounded-lg border border-white/10 bg-white/[0.03] px-2.5 py-1.5 text-xs text-slate-400 transition hover:text-slate-200"
      >
        Retention
      </button>
      {open
        ? createPortal(
        <div className="fixed inset-0 z-[100] flex items-center justify-center overflow-y-auto bg-black/75 p-4">
          <section
            role="dialog"
            aria-modal="true"
            aria-labelledby="retention-title"
            className="my-auto max-h-[90vh] w-full max-w-xl overflow-y-auto rounded-2xl border border-white/10 bg-[#0b1220] p-5 shadow-2xl"
          >
            <div className="flex items-start justify-between gap-4">
              <div>
                <h2 id="retention-title" className="text-base font-semibold text-slate-100">
                  Global event-detail retention
                </h2>
                <p className="mt-1 text-xs text-slate-400">
                  Applies to all sites; tenant-specific retention is not implemented.
                </p>
              </div>
              <button
                type="button"
                onClick={close}
                className="rounded-md px-2 py-1 text-sm text-slate-400 hover:bg-white/5 hover:text-slate-100"
                aria-label="Close retention settings"
              >
                Close
              </button>
            </div>

            {busy && !preview ? (
              <div className="flex items-center gap-2 py-8 text-sm text-slate-400">
                <Spinner /> Loading storage estimate…
              </div>
            ) : preview ? (
              <div className="mt-5 space-y-4">
                <div className="grid grid-cols-2 gap-3 text-xs">
                  <div className="rounded-lg border border-white/[0.06] bg-black/20 p-3">
                    <p className="text-slate-500">Current retention</p>
                    <p className="mt-1 text-sm font-semibold text-slate-100">{preview.current_hours} hours</p>
                  </div>
                  <div className="rounded-lg border border-white/[0.06] bg-black/20 p-3">
                    <p className="text-slate-500">Current database size</p>
                    <p className="mt-1 text-sm font-semibold text-slate-100">{fmtBytes(preview.database_bytes)}</p>
                  </div>
                  <div className="rounded-lg border border-white/[0.06] bg-black/20 p-3">
                    <p className="text-slate-500">Current hourly event partitions</p>
                    <p className="mt-1 text-sm font-semibold text-slate-100">{fmtBytes(preview.current_raw_partition_bytes)}</p>
                  </div>
                  <div className="rounded-lg border border-white/[0.06] bg-black/20 p-3">
                    <p className="text-slate-500">Estimated event-partition growth</p>
                    <p className="mt-1 text-sm font-semibold text-slate-100">
                      {preview.estimated_raw_bytes_per_hour === null
                        ? "Unavailable"
                        : `${fmtBytes(preview.estimated_raw_bytes_per_hour)} / hour`}
                    </p>
                  </div>
                </div>
                <div className="overflow-hidden rounded-lg border border-white/[0.06]">
                  <table className="w-full text-left text-xs">
                    <thead className="bg-white/[0.03] text-slate-500">
                      <tr>
                        <th className="px-3 py-2 font-medium">Detail retention</th>
                        <th className="px-3 py-2 font-medium">Estimated database size</th>
                      </tr>
                    </thead>
                    <tbody>
                      {preview.options.map((option) => (
                        <tr key={option.hours} className="border-t border-white/[0.05] text-slate-300">
                          <td className="px-3 py-2">{option.hours} hours</td>
                          <td className="px-3 py-2">{fmtBytes(option.estimated_database_bytes)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
                <p className="text-[11px] leading-relaxed text-slate-500">
                  {preview.estimate_note} Database size includes rollups and other data; only hourly
                  event partitions are removed. Detailed events that expire cannot be recovered from
                  rollups.
                </p>
                {preview.admin_configured ? (
                  <>
                    <label className="block text-xs text-slate-400">
                      New global detail-retention period
                      <select
                        value={selectedHours}
                        onChange={(event) => setSelectedHours(Number(event.target.value))}
                        className="mt-1.5 block w-full rounded-lg border border-white/10 bg-[#070b14] px-3 py-2 text-sm text-slate-200 focus:border-emerald-400/40 focus:outline-none"
                      >
                        {preview.choices_hours.map((hours) => (
                          <option key={hours} value={hours}>{hours} hours</option>
                        ))}
                      </select>
                    </label>
                    <label className="block text-xs text-slate-400">
                      Server-configured bearer token
                      <input
                        type="password"
                        autoComplete="off"
                        value={token}
                        onChange={(event) => setToken(event.target.value)}
                        className="mt-1.5 block w-full rounded-lg border border-white/10 bg-[#070b14] px-3 py-2 text-sm text-slate-200 focus:border-emerald-400/40 focus:outline-none"
                      />
                    </label>
                    <p className="text-[11px] text-amber-200/80">
                      Shorter retention permanently removes expired event details. Hourly rollups
                      remain available for historical summaries.
                    </p>
                  </>
                ) : (
                  <p className="rounded-lg border border-amber-400/20 bg-amber-400/[0.06] p-3 text-xs text-amber-200">
                    Changes are disabled. Configure RETENTION_ADMIN_TOKEN on the API service and
                    restart it to enable applying a setting.
                  </p>
                )}
                {message ? <p className="text-xs text-emerald-300">{message}</p> : null}
                {error ? <p className="text-xs text-rose-300">{error}</p> : null}
                <div className="flex justify-end gap-2">
                  <button
                    type="button"
                    onClick={() => void loadPreview()}
                    disabled={busy}
                    className="rounded-lg border border-white/10 px-3 py-2 text-xs text-slate-300 disabled:opacity-50"
                  >
                    Refresh estimate
                  </button>
                  {preview.admin_configured ? (
                    <button
                      type="button"
                      onClick={() => void apply()}
                      disabled={busy || !token || selectedHours === preview.current_hours}
                      className="rounded-lg bg-emerald-500 px-3 py-2 text-xs font-medium text-emerald-950 disabled:opacity-40"
                    >
                      {busy ? "Applying…" : "Apply retention"}
                    </button>
                  ) : null}
                </div>
              </div>
            ) : error ? (
              <p className="mt-5 text-sm text-rose-300">{error}</p>
            ) : null}
          </section>
        </div>,
        document.body,
      )
        : null}
    </>
  );
}
