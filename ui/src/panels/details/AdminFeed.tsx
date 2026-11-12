// The request feed of Admin mode (step 8A): what the mock served a real frontend, newest first, with
// the status it got and whether a response edit answered it. Polled while this tab is open and the
// page is visible. "Edit" opens the editor on the row's route.

import { useEffect, useState } from "react";
import { startFeed, type FeedRow } from "../../data/api/logFeed";
import { LazyBoundary } from "../../kit/LazyBoundary";
import { useAdmin } from "../../data/store/admin";
import { PayloadEditor } from "./PayloadEditor.lazy";

const clock = (ms: number) => new Date(ms).toISOString().slice(11, 19); // UTC time of day, like the Log

export function AdminFeed({ onEdit }: { onEdit?: (row: FeedRow) => void }) {
  const rows = useAdmin((s) => s.rows);
  const [editing, setEditing] = useState<string | null>(null);
  // the editor opens on the row's route; a caller (a test) may take the row instead
  // a HEAD is made from its GET and answers what the GET does, so it is the GET that is edited
  const open = (r: FeedRow) =>
    onEdit ? onEdit(r) : r.endpoint !== null && setEditing(r.endpoint.replace(/^HEAD /, "GET "));
  useEffect(() => startFeed(useAdmin.getState().addRows), []);
  const newest = [...rows].reverse();
  return (
    <section aria-label="Requests from your frontend" className="flex flex-col gap-2 p-3 text-sm">
      <p className="m-0 text-xs text-muted">What the mock answered, newest first. Times are UTC.</p>
      {newest.length === 0 ? (
        <p className="m-0 text-muted">
          Point your frontend at the mock and its requests show here.
        </p>
      ) : (
        <table className="w-full border-collapse text-left">
          <thead>
            <tr className="text-xs text-muted">
              <th className="py-1 pr-2 font-normal">Time</th>
              <th className="py-1 pr-2 font-normal">Request</th>
              <th className="py-1 pr-2 font-normal">Status</th>
              <th className="py-1 pr-2 text-right font-normal">ms</th>
              <th className="py-1 font-normal">
                <span className="sr-only">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {newest.map((r) => (
              <tr key={r.id} className="border-t border-border">
                <td className="py-1 pr-2 font-mono tabular-nums">{clock(r.ts_ms)}</td>
                <td className="py-1 pr-2 font-mono">
                  {r.method} {r.path}
                </td>
                <td className="py-1 pr-2 font-mono tabular-nums">
                  {r.status}
                  {r.edited ? (
                    <span className="ml-1.5 font-sans text-xs text-muted">edited</span>
                  ) : null}
                </td>
                <td className="py-1 pr-2 text-right font-mono tabular-nums">{r.ms}</td>
                <td className="py-1 text-right">
                  <button
                    type="button"
                    disabled={r.endpoint === null || Boolean(r.rejected)}
                    aria-label={`Edit response of ${r.method} ${r.path}`}
                    onClick={() => open(r)}
                    title="Edit the response of this route"
                    className="h-7 rounded-md border border-control bg-sheet px-2 text-xs whitespace-nowrap hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50"
                  >
                    Edit
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {editing ? (
        <LazyBoundary fallback={null}>
          <PayloadEditor route={editing} onClose={() => setEditing(null)} />
        </LazyBoundary>
      ) : null}
    </section>
  );
}
