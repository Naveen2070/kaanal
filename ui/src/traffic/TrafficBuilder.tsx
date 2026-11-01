// The traffic builder (developer, 2026-11-01): a dialog that edits the active traffic profile as a
// table, one row per endpoint: method, path, weight in the mix, request and response size, and the
// services behind the gateway (several are called in parallel; "all services" picks every one).
// A service the gateway is not yet connected to is connected when the rows are applied. Applying is
// one edit, so one undo. See model.ts for what is written.

import { Plus, Trash2 } from "lucide-react";
import { useId, useLayoutEffect, useRef, useState } from "react";
import { useDraft } from "../store/draft";
import { notify } from "../toast/toasts";
import {
  activeProfile,
  blankRow,
  buildOps,
  gatewayOf,
  METHODS,
  newConnections,
  problems,
  rowsFrom,
  serviceNames,
  type Method,
  type Row,
} from "./model";
import { ServicePicker } from "./ServicePicker";
import { closeTrafficBuilder } from "./store";

const field =
  "h-8 rounded-md border border-control bg-sheet px-2 text-sm text-text focus-visible:outline-2 focus-visible:outline-accent";

function summary(r: Row, all: string[]): string {
  if (r.targets === null) return "Custom chain, kept as written";
  if (r.targets.length === 0) return "The gateway answers";
  if (all.length > 0 && r.targets.length === all.length) return `All services (${all.length})`;
  return r.targets.join(", ");
}

export function TrafficBuilder() {
  const box = useRef<HTMLDialogElement>(null);
  const doc = useDraft((s) => s.view?.doc ?? null);
  const patch = useDraft((s) => s.patch);
  const profile = activeProfile(doc);
  const services = serviceNames(doc);
  const gateway = gatewayOf(doc);
  const [rows, setRows] = useState<Row[]>(() => {
    const r = rowsFrom(doc, profile);
    return r.length > 0 ? r : [blankRow()];
  });
  /** The row whose "sends to" list is open. */
  const [open, setOpen] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const titleId = useId();
  const rps = (doc?.traffic as Record<string, { rps?: number }> | undefined)?.[profile]?.rps;

  useLayoutEffect(() => {
    const d = box.current;
    if (!d) return;
    // when the dialog goes, focus goes back to the button that opened it
    const opener = document.activeElement as HTMLElement | null;
    if (!d.open) {
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
    }
    return () => opener?.focus();
  }, []);

  const set = (i: number, over: Partial<Row>) =>
    setRows((rs) => rs.map((r, k) => (k === i ? { ...r, ...over } : r)));
  const wrong = problems(rows, doc);
  const fresh = newConnections(doc, rows);

  const apply = async () => {
    if (!doc || wrong.length > 0 || busy) return;
    setBusy(true);
    const ok = await patch(buildOps(doc, profile, rows));
    setBusy(false);
    if (!ok) return; // the refusal is already a notice, and the rows stay for another try
    notify.success(
      `Traffic updated: ${rows.length} ${rows.length === 1 ? "endpoint" : "endpoints"}${
        fresh.length > 0 ? `, ${fresh.length} newly connected to ${gateway}` : ""
      }.`,
      "traffic",
    );
    closeTrafficBuilder();
  };

  return (
    <dialog
      ref={box}
      aria-labelledby={titleId}
      aria-modal="true"
      onCancel={(e) => {
        e.preventDefault();
        closeTrafficBuilder();
      }}
      className="max-h-[88vh] w-[min(64rem,calc(100vw-2rem))] overflow-y-auto rounded-2xl border border-border bg-sheet p-0 text-sm text-text shadow-float backdrop:bg-black/40"
    >
      <header className="border-b border-border px-5 py-4">
        <h2 id={titleId} className="m-0 text-base font-semibold">
          Traffic builder
        </h2>
        <p className="m-0 mt-1 text-xs text-muted">
          Profile “{profile}”
          {typeof rps === "number"
            ? ` · ${rps.toLocaleString("en-US")} requests a second in all`
            : ""}
          . The rate is the Offered load slider; this table is what each request is.
        </p>
      </header>

      <div className="px-5 py-4">
        <div
          aria-hidden
          className="mb-1 hidden grid-cols-[6rem_minmax(8rem,1fr)_5rem_6rem_6rem_minmax(10rem,14rem)_2rem] gap-2 px-0.5 text-xs text-muted md:grid"
        >
          <span>Method</span>
          <span>Path</span>
          <span>Weight</span>
          <span>Request KB</span>
          <span>Response KB</span>
          <span>Sends to</span>
          <span />
        </div>
        <ol className="m-0 flex list-none flex-col gap-2 p-0">
          {rows.map((r, i) => {
            const n = i + 1;
            return (
              <li key={i} className="rounded-xl border border-border bg-card p-2">
                <div className="grid grid-cols-2 items-center gap-2 md:grid-cols-[6rem_minmax(8rem,1fr)_5rem_6rem_6rem_minmax(10rem,14rem)_2rem]">
                  <select
                    aria-label={`Method of endpoint ${n}`}
                    value={r.method}
                    onChange={(e) => set(i, { method: e.currentTarget.value as Method })}
                    className={field}
                  >
                    {METHODS.map((m) => (
                      <option key={m}>{m}</option>
                    ))}
                  </select>
                  <input
                    aria-label={`Path of endpoint ${n}`}
                    value={r.path}
                    spellCheck={false}
                    onChange={(e) => set(i, { path: e.currentTarget.value })}
                    className={`${field} font-mono`}
                  />
                  <input
                    aria-label={`Weight of endpoint ${n}`}
                    type="number"
                    min={0}
                    step="any"
                    value={r.weight}
                    onChange={(e) => set(i, { weight: e.currentTarget.valueAsNumber })}
                    className={`${field} text-right tabular-nums`}
                  />
                  <input
                    aria-label={`Request size in KB of endpoint ${n}`}
                    type="number"
                    min={0}
                    step="any"
                    value={r.reqKb}
                    onChange={(e) => set(i, { reqKb: e.currentTarget.valueAsNumber })}
                    className={`${field} text-right tabular-nums`}
                  />
                  <input
                    aria-label={`Response size in KB of endpoint ${n}`}
                    type="number"
                    min={0}
                    step="any"
                    value={r.respKb}
                    onChange={(e) => set(i, { respKb: e.currentTarget.valueAsNumber })}
                    className={`${field} text-right tabular-nums`}
                  />
                  <button
                    type="button"
                    aria-expanded={open === i}
                    aria-label={`Services behind endpoint ${n}: ${summary(r, services)}`}
                    onClick={() => setOpen(open === i ? null : i)}
                    className={`${field} truncate text-left`}
                  >
                    {summary(r, services)}
                  </button>
                  <button
                    type="button"
                    aria-label={`Remove endpoint ${n}`}
                    onClick={() => {
                      setRows((rs) => rs.filter((_, k) => k !== i));
                      setOpen(null);
                    }}
                    className="flex size-8 items-center justify-center rounded-md text-muted hover:bg-raised hover:text-negative focus-visible:outline-2 focus-visible:outline-accent"
                  >
                    <Trash2 size={15} aria-hidden />
                  </button>
                </div>
                {open === i ? (
                  <fieldset className="m-0 mt-2 min-w-0 rounded-lg border border-border p-3">
                    <legend className="px-1 text-xs text-muted">
                      Services behind {gateway ?? "the gateway"} for endpoint {n}
                    </legend>
                    {r.targets === null ? (
                      <p className="m-0 flex flex-wrap items-center gap-2 text-xs text-muted">
                        This endpoint has a chain written by hand. It is kept as it is.
                        <button
                          type="button"
                          onClick={() => set(i, { targets: [] })}
                          className="rounded-md border border-control px-2 py-1 text-text hover:bg-raised"
                        >
                          Replace it with a choice of services
                        </button>
                      </p>
                    ) : (
                      <ServicePicker
                        services={services}
                        gateway={gateway}
                        edges={doc?.edges ?? []}
                        value={r.targets}
                        onChange={(targets) => set(i, { targets })}
                      />
                    )}
                  </fieldset>
                ) : null}
              </li>
            );
          })}
        </ol>
        <button
          type="button"
          onClick={() => setRows((rs) => [...rs, blankRow()])}
          className="mt-3 flex h-8 items-center gap-1.5 rounded-lg border border-control px-3 font-medium hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
        >
          <Plus size={14} aria-hidden />
          Add endpoint
        </button>
      </div>

      <footer className="flex flex-wrap items-center gap-3 border-t border-border px-5 py-3">
        <div aria-live="polite" className="min-w-0 flex-1 text-xs">
          {wrong.length > 0 ? (
            <ul className="m-0 list-none p-0 text-negative">
              {wrong.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          ) : fresh.length > 0 ? (
            <p className="m-0 text-muted">
              {gateway} will be connected to {fresh.join(", ")}.
            </p>
          ) : null}
        </div>
        <button
          type="button"
          onClick={closeTrafficBuilder}
          className="h-8 rounded-lg border border-control px-3 font-medium hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
        >
          Cancel
        </button>
        <button
          type="button"
          disabled={wrong.length > 0 || busy || doc === null}
          onClick={() => void apply()}
          className="h-8 rounded-lg bg-primary px-3.5 font-medium text-on-primary hover:opacity-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
        >
          Apply to the draft
        </button>
      </footer>
    </dialog>
  );
}
