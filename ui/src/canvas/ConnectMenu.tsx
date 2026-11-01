// "Connect to…" (WCAG 2.5.7, a way to draw an edge without dragging): the components this one can
// call, as buttons. Only valid targets are listed: not itself, not one it already calls. A refusal
// from the draft is shown beside the list with its reason.

import { useState } from "react";
import type { EdgeDoc } from "../api/types";
import { useDraft } from "../store/draft";

/** The names `from` could still call. */
export function targetsFor(from: string, names: string[], edges: EdgeDoc[]): string[] {
  const called = new Set(edges.filter((e) => String(e.from) === from).map((e) => String(e.to)));
  return names.filter((n) => n !== from && !called.has(n));
}

export function ConnectMenu({ from }: { from: string }) {
  const doc = useDraft((s) => s.view?.doc);
  const connect = useDraft((s) => s.connect);
  const [note, setNote] = useState<string | null>(null);
  const targets = targetsFor(from, Object.keys(doc?.nodes ?? {}), doc?.edges ?? []);
  return (
    <div>
      {targets.length === 0 ? (
        <p className="text-xs text-muted">
          Nothing to connect to: a component cannot call itself, and this one already calls every
          other.
        </p>
      ) : (
        <ul aria-label={`Connect ${from} to`} className="flex flex-wrap gap-1">
          {targets.map((to) => (
            <li key={to}>
              <button
                type="button"
                onClick={() =>
                  void connect(from, to).then((r) =>
                    setNote(r.ok ? (r.warning ?? null) : r.message),
                  )
                }
                className="rounded-full border border-control px-2 py-0.5 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
              >
                {to}
              </button>
            </li>
          ))}
        </ul>
      )}
      {note ? (
        <p role="alert" className="mt-1 text-xs text-negative">
          {note}
        </p>
      ) : null}
    </div>
  );
}
