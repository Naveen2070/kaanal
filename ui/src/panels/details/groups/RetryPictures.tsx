// The two small pictures inside "When a call fails": the wait before each retry as bars, and a table
// of what differs from the default (shown above Reset, so the person sees what Reset will undo).
// Built from the tokens already in the app; the numbers come from `retryPreview.ts`.

import { jitterOf } from "../resilience";
import { type DiffRow, retryWaits } from "./retryPreview";
import { humanMs } from "./summaries";

type Obj = Record<string, unknown>;

/** Bars for the wait before each retry: the first try (no wait) then one bar per retry. */
export function BackoffTimeline({ r }: { r: Obj }) {
  const waits = retryWaits(r);
  const tallest = Math.max(...waits);
  const jittered = jitterOf(r) !== "none";
  return (
    <figure className="m-0 mb-2" aria-label="Wait before each retry">
      <figcaption className="text-xs text-muted">
        {jittered
          ? "Longest wait before each retry (jitter shortens it)"
          : "Wait before each retry"}
      </figcaption>
      <div className="mt-1 flex h-12 items-end gap-1.5" aria-hidden>
        <div className="h-[12%] flex-1 rounded-t-sm bg-muted opacity-70" />
        {waits.map((ms, i) => (
          <div
            key={i}
            style={{ height: `${Math.max(12, (ms / tallest) * 100)}%` }}
            className="flex-1 rounded-t-sm bg-accent opacity-80"
          />
        ))}
      </div>
      <div className="mt-0.5 flex gap-1.5 text-[11px] text-muted">
        <span className="flex-1 text-center">try 1</span>
        {waits.map((ms, i) => (
          <span key={i} className="flex-1 text-center">
            +{humanMs(ms)}
          </span>
        ))}
      </div>
    </figure>
  );
}

/** The settings that differ from the default, old next to new; nothing when none differ. */
export function DiffTable({ rows }: { rows: DiffRow[] }) {
  if (!rows.length) return null;
  return (
    <table className="mb-2 w-full border-collapse text-xs">
      <caption className="sr-only">What differs from the default</caption>
      <thead>
        <tr className="text-left text-muted">
          <th scope="col" className="font-normal">
            <span className="sr-only">Setting</span>
          </th>
          <th scope="col" className="w-24 font-normal">
            Default
          </th>
          <th scope="col" className="w-24 font-normal">
            Now
          </th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.label}>
            <th scope="row" className="py-px pr-2 text-left font-normal">
              {r.label}
            </th>
            <td className="text-muted">{r.was}</td>
            <td className="font-semibold">{r.now}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
