// "Connect to…" (WCAG 2.5.7, a way to draw an edge without dragging): the components this one can
// call, as buttons. Only valid targets are listed: not itself, not one it already calls. A refusal
// from the draft is shown beside the list with its reason. Below it, the wires that leave this
// component, each with a button to delete it (the other way to delete a wire; Delete on a selected
// wire and its own button are the others).

import { Unplug } from "lucide-react";
import type { EdgeDoc } from "../api/types";
import { useDraft } from "../store/draft";
import { notify } from "../toast/toasts";
import { mayCall } from "./rules";
import { removeWires } from "./wires";

/** The names `from` could still call: not itself, not one it calls, and only kinds it may call. */
export function targetsFor(
  from: string,
  nodes: Record<string, { type?: string }>,
  edges: EdgeDoc[],
): string[] {
  const called = new Set(edges.filter((e) => String(e.from) === from).map((e) => String(e.to)));
  return Object.keys(nodes).filter(
    (n) => n !== from && !called.has(n) && mayCall(nodes[from]?.type, nodes[n]?.type),
  );
}

export function ConnectMenu({ from }: { from: string }) {
  const doc = useDraft((s) => s.view?.doc);
  const connect = useDraft((s) => s.connect);
  const targets = targetsFor(from, doc?.nodes ?? {}, doc?.edges ?? []);
  /** The wires leaving this component, with their place in `edges` (which is what a delete needs). */
  const wires = (doc?.edges ?? [])
    .map((e, i) => ({ to: String(e.to), from: String(e.from), i }))
    .filter((w) => w.from === from);
  return (
    <div>
      {targets.length === 0 ? (
        <p className="text-xs text-muted">
          Nothing to connect to: a component cannot call itself or one it already calls, and some
          kinds cannot call others (a service cannot call a gateway).
        </p>
      ) : (
        <ul aria-label={`Connect ${from} to`} className="flex flex-wrap gap-1">
          {targets.map((to) => (
            <li key={to}>
              <button
                type="button"
                onClick={() =>
                  void connect(from, to).then((r) => {
                    notify.clear("connect");
                    if (!r.ok) notify.error(r.message, "connect");
                    else if (r.warning) notify.warn(r.warning, "connect");
                  })
                }
                className="inline-flex min-h-7 min-w-6 items-center justify-center rounded-full border border-control px-2.5 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent"
              >
                {to}
              </button>
            </li>
          ))}
        </ul>
      )}
      {wires.length > 0 ? (
        <ul aria-label={`Wires from ${from}`} className="mt-2 flex flex-col gap-1">
          {wires.map((w) => (
            <li key={w.i} className="flex items-center gap-2 text-xs">
              <span className="min-w-0 flex-1 truncate">
                {from} to {w.to}
              </span>
              <button
                type="button"
                aria-label={`Delete the wire from ${from} to ${w.to}`}
                onClick={() => void removeWires([w.i])}
                className="flex size-6 items-center justify-center rounded-md text-muted hover:bg-raised hover:text-negative focus-visible:outline-2 focus-visible:outline-accent"
              >
                <Unplug size={14} aria-hidden />
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
